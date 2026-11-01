import { join } from "@std/path";
import { ensurePrivateDir } from "./storage.ts";

export type LineEmoji = { start: number; end: number; key: string };
type Json = Record<string, unknown>;
const MAX_BYTES = 2 * 1024 * 1024;
const KEY = /^[a-f0-9]{24}_[0-9]{1,5}$/i;

// LINEの位置はUTF-16単位。画面にはUnicodeの文字数で渡す。
export function lineEmojis(
  text: string,
  meta: Record<string, unknown>,
): LineEmoji[] {
  try {
    if (typeof meta.REPLACE !== "string" || meta.REPLACE.length > 100_000) {
      return [];
    }
    const resources = JSON.parse(meta.REPLACE)?.sticon?.resources;
    if (!Array.isArray(resources)) return [];
    const result: LineEmoji[] = [];
    let previous = 0;
    const boundary = (offset: number) =>
      offset === 0 || offset === text.length ||
      !(text.charCodeAt(offset) >= 0xdc00 && text.charCodeAt(offset) <= 0xdfff);
    for (
      const resource of resources.slice(0, 100).sort((a, b) =>
        Number(a?.S) - Number(b?.S)
      )
    ) {
      if (!resource || typeof resource !== "object") continue;
      const start = Number(resource.S), end = Number(resource.E);
      const key = `${resource.productId}_${resource.sticonId}`;
      if (
        !Number.isSafeInteger(start) || !Number.isSafeInteger(end) ||
        start < previous ||
        end <= start || end > text.length || !boundary(start) ||
        !boundary(end) || !KEY.test(key)
      ) continue;
      result.push({
        start: [...text.slice(0, start)].length,
        end: [...text.slice(0, end)].length,
        key,
      });
      previous = end;
    }
    return result;
  } catch {
    return [];
  }
}

function validPng(bytes: Uint8Array): boolean {
  if (
    bytes.length < 24 || bytes.length > MAX_BYTES ||
    ![137, 80, 78, 71, 13, 10, 26, 10].every((byte, i) => bytes[i] === byte)
  ) return false;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return view.getUint32(12) === 0x49484452 && view.getUint32(16) > 0 &&
    view.getUint32(16) <= 512 && view.getUint32(20) > 0 &&
    view.getUint32(20) <= 512;
}

export function createEmojiService(runtime: {
  directory: string;
  emit: (key: string, path: string) => void;
  fetch?: typeof fetch;
}) {
  const queue: string[] = [];
  const queued = new Set<string>();
  const failures = new Map<string, number>();
  let active = 0;
  async function load(key: string) {
    const path = join(runtime.directory, `emoji-${key}.png`);
    try {
      try {
        const stat = await Deno.stat(path);
        if (
          stat.isFile && stat.size <= MAX_BYTES &&
          validPng(await Deno.readFile(path))
        ) {
          runtime.emit(key, path);
          return;
        }
      } catch { /* 未取得の画像はCDNから取得する。 */ }
      const [product, id] = key.split("_");
      const response = await (runtime.fetch ?? fetch)(
        `https://stickershop.line-scdn.net/sticonshop/v1/sticon/${product}/android/${id}.png`,
        { signal: AbortSignal.timeout(8000), redirect: "error" },
      );
      if (!response.ok || !response.body) {
        await response.body?.cancel();
        throw new Error("emoji_unavailable");
      }
      const chunks: Uint8Array[] = [];
      let length = 0;
      for await (const chunk of response.body) {
        length += chunk.length;
        if (length > MAX_BYTES) throw new Error("emoji_too_large");
        chunks.push(chunk);
      }
      const bytes = new Uint8Array(length);
      let offset = 0;
      for (const chunk of chunks) {
        bytes.set(chunk, offset);
        offset += chunk.length;
      }
      if (!validPng(bytes)) throw new Error("emoji_invalid_image");
      await ensurePrivateDir(runtime.directory);
      const temporary = `${path}.${Deno.pid}.tmp`;
      try {
        await Deno.writeFile(temporary, bytes, { mode: 0o600 });
        await Deno.rename(temporary, path);
      } finally {
        try {
          await Deno.remove(temporary);
        } catch { /* 移動済み。 */ }
      }
      runtime.emit(key, path);
      failures.delete(key);
    } catch {
      if (failures.size >= 500) failures.clear();
      failures.set(key, Date.now() + 60_000);
    }
  }
  function pump() {
    while (active < 4 && queue.length) {
      const key = queue.shift()!;
      active++;
      void load(key).finally(() => {
        active--;
        queued.delete(key);
        pump();
      });
    }
  }
  return {
    observe(messages: Json[]) {
      for (const message of messages) {
        if (message.decryptionFailed || !Array.isArray(message.emojis)) {
          continue;
        }
        for (const emoji of message.emojis.slice(0, 100)) {
          const key = emoji?.key;
          if (
            typeof key !== "string" || !KEY.test(key) || queued.has(key) ||
            queued.size >= 500 || (failures.get(key) ?? 0) > Date.now()
          ) continue;
          queued.add(key);
          queue.push(key);
        }
      }
      pump();
    },
  };
}

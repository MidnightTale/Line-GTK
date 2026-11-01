import { join } from "@std/path";
import { ensurePrivateDir } from "./storage.ts";
import { emojiVersion } from "./reactions.ts";

type Json = Record<string, unknown>;
export type EmojiPack = { id: string; name: string; iconPath: string | null };

const PRODUCT = /^[a-f0-9]{24}$/i;
const EMOJI_ID = /^[0-9]{1,5}$/;
const MAX_PACKS = 80;
const MAX_PER_PACK = 300;
const MAX_ICON_BYTES = 512 * 1024;
const PACKS_TTL = 10 * 60 * 1000;
const PNG = [137, 80, 78, 71, 13, 10, 26, 10];

/** Reads the owned emoji products from `getOwnedProductSummaries`. */
export function ownedPacks(response: unknown): { id: string; name: string }[] {
  const data = (response ?? {}) as Json;
  const list = (data.productList ?? data["1"] ?? []) as Json[];
  if (!Array.isArray(list)) return [];
  const seen = new Set<string>();
  const result: { id: string; name: string }[] = [];
  for (const item of list) {
    const id = String(item?.id ?? item?.["1"] ?? "");
    if (!PRODUCT.test(id) || seen.has(id)) continue;
    seen.add(id);
    result.push({ id, name: String(item?.name ?? item?.["11"] ?? "") });
    if (result.length >= MAX_PACKS) break;
  }
  return result;
}

/**
 * Latest version of each owned pack. The CDN `meta.json` carries no version,
 * but the reaction must name it or other clients cannot resolve the image.
 */
export function ownedVersions(response: unknown): Map<string, number> {
  const data = (response ?? {}) as Json;
  const list = (data.productList ?? data["1"] ?? []) as Json[];
  const versions = new Map<string, number>();
  if (!Array.isArray(list)) return versions;
  for (const item of list) {
    const id = String(item?.id ?? item?.["1"] ?? "");
    const raw = item?.latestVersion ?? item?.["21"];
    if (PRODUCT.test(id) && raw !== undefined) {
      versions.set(id, emojiVersion(raw));
    }
  }
  return versions;
}

/** Emoji ids of one pack, in the order LINE shows them. */
export function packEmojiIds(meta: unknown): string[] {
  const data = (meta ?? {}) as Json;
  const ordered = Array.isArray(data.orders) ? data.orders : [];
  const alt = data.altTexts && typeof data.altTexts === "object"
    ? Object.keys(data.altTexts as Json)
    : [];
  const ids = (ordered.length ? ordered : alt).map((id) => String(id));
  return [...new Set(ids.filter((id) => EMOJI_ID.test(id)))].slice(
    0,
    MAX_PER_PACK,
  );
}

type Runtime = {
  accountMid: () => string;
  ownedProducts: () => Promise<unknown>;
  directory: string;
  observe: (messages: Json[]) => void;
  fetch?: typeof fetch;
};

export function createReactionEmojiPacks(runtime: Runtime) {
  const get = runtime.fetch ?? fetch;
  let cached: { account: string; at: number; packs: EmojiPack[] } | null = null;
  const metas = new Map<string, { ids: string[]; version?: number }>();
  const versions = new Map<string, number>();

  async function icon(id: string): Promise<string | null> {
    const path = join(runtime.directory, `emoji-pack-${id}.png`);
    try {
      const stat = await Deno.stat(path);
      if (stat.isFile && stat.size <= MAX_ICON_BYTES) return path;
    } catch { /* 未取得のアイコンはCDNから取得する。 */ }
    try {
      const response = await get(
        `https://stickershop.line-scdn.net/sticonshop/v1/${id}/sticon/iphone/tab_on.png`,
        { signal: AbortSignal.timeout(8000), redirect: "error" },
      );
      if (!response.ok) {
        await response.body?.cancel();
        return null;
      }
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (
        bytes.length < 24 || bytes.length > MAX_ICON_BYTES ||
        !PNG.every((byte, i) => bytes[i] === byte)
      ) return null;
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
      return path;
    } catch {
      return null;
    }
  }

  async function packs(): Promise<{ packs: EmojiPack[] }> {
    const account = runtime.accountMid();
    if (
      cached && cached.account === account && Date.now() - cached.at < PACKS_TTL
    ) return { packs: cached.packs };
    try {
      const summaries = await runtime.ownedProducts();
      const owned = ownedPacks(summaries);
      for (const [id, version] of ownedVersions(summaries)) {
        versions.set(id, version);
      }
      const result: EmojiPack[] = [];
      for (let i = 0; i < owned.length; i += 6) {
        result.push(
          ...await Promise.all(
            owned.slice(i, i + 6).map(async (pack) => ({
              ...pack,
              iconPath: await icon(pack.id),
            })),
          ),
        );
      }
      if (runtime.accountMid() !== account) return { packs: [] };
      cached = { account, at: Date.now(), packs: result };
      return { packs: result };
    } catch (error) {
      console.error("[emoji packs]", error);
      if (cached?.account === account) return { packs: cached.packs };
      throw new Error("reaction_packs_unavailable");
    }
  }

  async function pack(productId: unknown): Promise<{
    emojis: { key: string; resourceType: number; version: number }[];
  }> {
    const id = String(productId ?? "");
    if (!PRODUCT.test(id)) throw new Error("reaction_invalid_pack");
    let meta = metas.get(id);
    if (!meta) {
      const response = await get(
        `https://stickershop.line-scdn.net/sticonshop/v1/product/${id}/android/meta.json`,
        { signal: AbortSignal.timeout(8000), redirect: "error" },
      );
      if (!response.ok) {
        await response.body?.cancel();
        throw new Error("reaction_packs_unavailable");
      }
      const data = await response.json();
      meta = {
        ids: packEmojiIds(data),
        version: data?.version === undefined
          ? undefined
          : emojiVersion(data.version),
      };
      if (metas.size >= 100) metas.clear();
      metas.set(id, meta);
    }
    const version = meta.version ?? versions.get(id) ?? 1;
    const emojis = meta.ids.map((emoji) => ({
      key: `${id}_${emoji}`,
      resourceType: 1,
      version,
    }));
    // The message image loader accepts at most 100 emoji per message. Packs
    // can be larger, so submit every image in bounded batches.
    const batches: Json[] = [];
    for (let i = 0; i < emojis.length; i += 100) {
      batches.push({
        emojis: emojis.slice(i, i + 100).map(({ key }) => ({ key })),
      });
    }
    runtime.observe(batches);
    return { emojis };
  }

  return { packs, pack };
}

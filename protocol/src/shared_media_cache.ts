import { join } from "@std/path";
import type { Client, TalkMessage } from "@evex/linejs";
import type {
  SharedMediaAttachment,
  SharedMediaSource,
} from "./shared_media.ts";
import { videoContainer } from "./outgoing_media.ts";

type Runtime = {
  mediaDir: string;
  accountMid: () => string;
  getClient: () => Client | null;
  resolve: (chatMid: string, source: SharedMediaSource) => Promise<TalkMessage>;
  existingFile: (path: string) => Promise<string | null>;
  existingFullImage: (id: string) => Promise<string | null>;
  isImageBytes: (bytes: Uint8Array) => boolean;
  isPreview: (path: string) => Promise<boolean>;
  mediaDest: (id: string, ext?: string) => string;
  thumbDest: (id: string) => string;
};

/** Reuse the application's media store; publish only for the captured session. */
export function createSharedAttachmentCache(runtime: Runtime) {
  async function publish(path: string, bytes: Uint8Array, check: () => void) {
    const temporary = join(
      runtime.mediaDir,
      `.shared-${crypto.randomUUID()}.tmp`,
    );
    try {
      await Deno.writeFile(temporary, bytes, { mode: 0o600 });
      check();
      // The final validity check and rename share one JS turn, including clear.
      Deno.renameSync(temporary, path);
    } finally {
      try {
        await Deno.remove(temporary);
      } catch { /* renamed or failed before creation */ }
    }
  }

  async function preview(
    path: string,
    destination: string,
    check: () => void,
  ): Promise<string | undefined> {
    const temporary = join(
      runtime.mediaDir,
      `.shared-${crypto.randomUUID()}.jpg`,
    );
    try {
      const result = await new Deno.Command("ffmpeg", {
        args: [
          "-nostdin",
          "-y",
          "-i",
          path,
          "-frames:v",
          "1",
          "-vf",
          "scale=320:320:force_original_aspect_ratio=decrease",
          temporary,
        ],
        stdout: "null",
        stderr: "null",
      }).output();
      check();
      if (!result.success) return undefined;
      const bytes = await Deno.readFile(temporary);
      check();
      if (!runtime.isImageBytes(bytes)) return undefined;
      await publish(destination, bytes, check);
      return destination;
    } catch (error) {
      check();
      if (error instanceof Deno.errors.NotFound) return undefined;
      return undefined;
    } finally {
      try {
        await Deno.remove(temporary);
      } catch { /* ffmpeg unavailable or finished */ }
    }
  }

  return async (
    chatMid: string,
    source: SharedMediaSource,
    check: () => void,
  ): Promise<SharedMediaAttachment> => {
    check();
    if (!runtime.getClient()) throw new Error("shared_media_not_logged_in");
    const message = source.message;
    const id = String(message.id ?? "");
    const type = String(message.contentType ?? "").toUpperCase();
    const account = runtime.accountMid();
    if (
      !/^[a-zA-Z0-9_-]+$/.test(id) || !["IMAGE", "VIDEO", "FILE"].includes(type)
    ) {
      throw new Error("shared_media_invalid_attachment");
    }
    const key = `shared-${account}-${chatMid}-${id}`;
    const fileName = String(
      message.fileName ||
        `${id}.${type === "VIDEO" ? "mp4" : type === "IMAGE" ? "jpg" : "bin"}`,
    );
    const safeName = Array.from(
      fileName,
      (character) =>
        character === "/" || character === "\\" || character.charCodeAt(0) < 32
          ? "_"
          : character,
    ).join("");
    let extension = type === "VIDEO"
      ? "mp4"
      : type === "IMAGE"
      ? "jpg"
      : (safeName.match(/\.([a-zA-Z0-9]{1,12})$/)?.[1].toLowerCase() || "bin");
    const candidates: Array<string | null> = [];
    if (
      typeof message.filePath === "string" &&
      message.filePath.startsWith(`${runtime.mediaDir}/`)
    ) {
      candidates.push(message.filePath);
    }
    if (type === "IMAGE") candidates.push(await runtime.existingFullImage(id));
    else candidates.push(runtime.mediaDest(id, extension));
    if (type === "FILE") {
      candidates.push(join(runtime.mediaDir, `${id}.${safeName}`));
    }
    candidates.push(runtime.mediaDest(key, extension));
    check();
    let path: string | undefined;
    for (const candidate of candidates) {
      if (!candidate || candidate.includes(".thumb.")) continue;
      const existing = await runtime.existingFile(candidate);
      check();
      if (!existing) continue;
      if (type === "IMAGE" && await runtime.isPreview(existing)) continue;
      if (type === "IMAGE" || type === "VIDEO") {
        const file = await Deno.open(existing);
        try {
          const bytes = new Uint8Array(32);
          const size = await file.read(bytes);
          check();
          if (
            !size ||
            (type === "IMAGE"
              ? !runtime.isImageBytes(bytes)
              : !videoContainer(bytes.subarray(0, size)))
          ) continue;
        } finally {
          file.close();
        }
      }
      path = existing;
      break;
    }
    if (!path) {
      const tm = await runtime.resolve(chatMid, source);
      check();
      const blob = await tm.getData(false);
      check();
      const bytes = new Uint8Array(await blob.arrayBuffer());
      check();
      if (!bytes.length) throw new Error("shared_media_original_unavailable");
      if (type === "VIDEO") {
        const container = videoContainer(bytes);
        if (!container) throw new Error("shared_media_original_unavailable");
        extension = container;
      } else if (type === "IMAGE") {
        if (!runtime.isImageBytes(bytes)) {
          throw new Error("shared_media_original_unavailable");
        }
        extension = bytes[0] === 0x89
          ? "png"
          : bytes[0] === 0x47
          ? "gif"
          : bytes[0] === 0x52
          ? "webp"
          : "jpg";
      }
      path = runtime.mediaDest(key, extension);
      await publish(path, bytes, check);
    }
    check();
    let thumbnailPath: string | undefined;
    if (type !== "FILE") {
      thumbnailPath = await runtime.existingFile(runtime.thumbDest(key)) ||
        await runtime.existingFile(runtime.thumbDest(id)) || undefined;
      check();
      thumbnailPath ??= await preview(path, runtime.thumbDest(key), check);
      if (type === "IMAGE") thumbnailPath ??= path;
    }
    const stat = await Deno.stat(path);
    check();
    return {
      path,
      fileName,
      contentType: type,
      thumbnailPath,
      bytes: stat.size,
    };
  };
}

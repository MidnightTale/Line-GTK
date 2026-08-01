import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { profileUrl } from "./contacts.ts";

export type MediaCacheRuntime = {
  getClient: () => Client | null;
  avatarDir: string;
  mediaDir: string;
  thumbMax: number;
  thumbBytes: number;
  previewMaxEdge: number;
  refetchMessageData: (
    chatMid: string,
    messageId: string,
    opts?: { allowNonImage?: boolean },
  ) => Promise<{ buf: Uint8Array; mime: string } | null>;
};

export function createMediaCache(runtime: MediaCacheRuntime) {
  const {
    avatarDir,
    mediaDir,
    thumbMax: THUMB_MAX,
    thumbBytes: THUMB_BYTES,
    previewMaxEdge: PREVIEW_MAX_EDGE,
    refetchMessageData,
  } = runtime;

  async function existingFile(path: string): Promise<string | null> {
    try {
      const st = await Deno.stat(path);
      return st.isFile && st.size > 32 ? path : null;
    } catch {
      return null;
    }
  }

  function isImageBytes(buf: Uint8Array): boolean {
    if (buf.length < 12) return false;
    // JPEG
    if (buf[0] === 0xff && buf[1] === 0xd8 && buf[2] === 0xff) return true;
    // PNG
    if (
      buf[0] === 0x89 && buf[1] === 0x50 && buf[2] === 0x4e && buf[3] === 0x47
    ) {
      return true;
    }
    // GIF
    if (buf[0] === 0x47 && buf[1] === 0x49 && buf[2] === 0x46) return true;
    // WEBP (RIFF....WEBP)
    if (
      buf[0] === 0x52 && buf[1] === 0x49 && buf[2] === 0x46 &&
      buf[3] === 0x46 &&
      buf[8] === 0x57 && buf[9] === 0x45 && buf[10] === 0x42 && buf[11] === 0x50
    ) {
      return true;
    }
    return false;
  }

  async function isImageFile(path: string): Promise<boolean> {
    try {
      const file = await Deno.open(path, { read: true });
      const buf = new Uint8Array(16);
      const n = await file.read(buf);
      file.close();
      if (!n || n < 4) return false;
      return isImageBytes(buf);
    } catch {
      return false;
    }
  }

  async function existingImage(path: string): Promise<string | null> {
    const hit = await existingFile(path);
    if (!hit) return null;
    if (await isImageFile(hit)) return hit;
    try {
      await Deno.remove(hit);
    } catch { /* ignore corrupt/html leftovers */ }
    return null;
  }

  async function writeImageFile(
    dest: string,
    buf: Uint8Array,
  ): Promise<string | null> {
    if (!isImageBytes(buf)) return null;
    const tmp = `${dest}.tmp-${Date.now()}-${
      Math.random().toString(16).slice(2)
    }`;
    try {
      await Deno.writeFile(tmp, buf);
      await Deno.rename(tmp, dest);
    } catch (e) {
      try {
        await Deno.remove(tmp);
      } catch { /* ignore */ }
      throw e;
    }
    return dest;
  }

  async function cacheUrl(url: string, dest: string): Promise<string | null> {
    const hit = await existingImage(dest);
    if (hit) return hit;
    try {
      const res = await fetch(url);
      if (!res.ok) return null;
      const ctype = (res.headers.get("content-type") || "").toLowerCase();
      if (
        ctype && !ctype.includes("image") && !ctype.includes("octet-stream")
      ) {
        return null;
      }
      const buf = new Uint8Array(await res.arrayBuffer());
      return await writeImageFile(dest, buf);
    } catch (e) {
      console.error("[cacheUrl]", e);
      return null;
    }
  }

  async function avatarPathFor(
    mid: string,
    picturePath?: string | null,
  ): Promise<string | null> {
    const dest = join(avatarDir, `${mid}.jpg`);
    const hit = await existingImage(dest);
    if (hit) return hit;
    const url = profileUrl(picturePath);
    if (!url) return null;
    return await cacheUrl(url, dest);
  }

  function mediaDest(id: string, ext = "jpg") {
    return join(mediaDir, `${id}.${ext}`);
  }

  function fullDest(id: string, ext = "jpg") {
    return join(mediaDir, `${id}.full.${ext}`);
  }

  function thumbDest(id: string) {
    return join(mediaDir, `${id}.thumb.jpg`);
  }

  async function imageDimensions(
    path: string,
  ): Promise<{ w: number; h: number } | null> {
    try {
      const p = new Deno.Command("ffprobe", {
        args: [
          "-v",
          "error",
          "-select_streams",
          "v:0",
          "-show_entries",
          "stream=width,height",
          "-of",
          "csv=s=x:p=0",
          path,
        ],
        stdout: "piped",
        stderr: "null",
      });
      const { success, stdout } = await p.output();
      if (!success) return null;
      const text = new TextDecoder().decode(stdout).trim();
      const [ws, hs] = text.split("x");
      const w = Number(ws);
      const h = Number(hs);
      if (!Number.isFinite(w) || !Number.isFinite(h) || w < 1 || h < 1) {
        return null;
      }
      return { w, h };
    } catch {
      return null;
    }
  }

  /** True when the file is a UI thumb / OBS preview, not a full original. */
  async function isPreviewOrThumbImage(path: string): Promise<boolean> {
    if (path.includes(".thumb.")) return true;
    // Explicit full originals are trusted even when the photo itself is small.
    if (path.includes(".full.")) return false;
    try {
      const st = await Deno.stat(path);
      if (st.size < 2_000) return true;
      if (st.size >= 400_000) return false;
    } catch {
      return true;
    }
    const dims = await imageDimensions(path);
    if (!dims) {
      try {
        const st = await Deno.stat(path);
        return st.size < THUMB_BYTES;
      } catch {
        return true;
      }
    }
    return Math.max(dims.w, dims.h) <= PREVIEW_MAX_EDGE;
  }

  async function existingFullImage(id: string): Promise<string | null> {
    for (const ext of ["jpg", "jpeg", "png", "webp", "gif"]) {
      const preferred = await existingImage(fullDest(id, ext));
      if (preferred) return preferred;
    }
    // Legacy slots may be polluted with OBS preview — filter those out.
    for (const ext of ["jpg", "jpeg", "png", "webp", "gif"]) {
      const p = await existingImage(mediaDest(id, ext));
      if (!p) continue;
      if (await isPreviewOrThumbImage(p)) continue;
      try {
        const dest = fullDest(id, ext === "jpeg" ? "jpg" : ext);
        await Deno.copyFile(p, dest);
        return dest;
      } catch {
        return p;
      }
    }
    const bare = await existingImage(mediaDest(id));
    if (bare && !(await isPreviewOrThumbImage(bare))) {
      try {
        const dest = fullDest(id, "jpg");
        await Deno.copyFile(bare, dest);
        return dest;
      } catch {
        return bare;
      }
    }
    return null;
  }

  async function writeFullImage(
    id: string,
    buf: Uint8Array,
    mimeOrExt?: string,
  ): Promise<string | null> {
    if (!isImageBytes(buf)) return null;
    const ext = (mimeOrExt || "").includes("png") || buf[0] === 0x89
      ? "png"
      : (mimeOrExt || "").includes("webp")
      ? "webp"
      : (mimeOrExt || "").includes("gif") ||
          (buf[0] === 0x47 && buf[1] === 0x49)
      ? "gif"
      : "jpg";
    return await writeImageFile(fullDest(id, ext), buf);
  }

  async function makeThumb(src: string, id: string): Promise<string | null> {
    const dest = thumbDest(id);
    const hit = await existingImage(dest);
    // If we already have a full original, rebuild thumb from it (OBS preview thumbs
    // are often cached first and would otherwise stick forever).
    const srcIsFull = src.includes(".full.");
    if (hit && !srcIsFull) return hit;
    if (hit && srcIsFull) {
      try {
        const hs = await Deno.stat(hit);
        const ss = await Deno.stat(src);
        if (hs.size >= ss.size || (await isPreviewOrThumbImage(hit))) {
          try {
            await Deno.remove(hit);
          } catch { /* ignore */ }
        } else {
          return hit;
        }
      } catch { /* recreate below */ }
    }
    if (!(await isImageFile(src))) return null;
    try {
      const st = await Deno.stat(src);
      if (st.size > 0 && st.size < THUMB_BYTES) {
        const dims = await imageDimensions(src);
        if (dims && Math.max(dims.w, dims.h) <= THUMB_MAX) return src;
      }
    } catch {
      return null;
    }
    try {
      const p = new Deno.Command("ffmpeg", {
        args: [
          "-y",
          "-v",
          "error",
          "-i",
          src,
          "-vf",
          `scale=${THUMB_MAX}:${THUMB_MAX}:force_original_aspect_ratio=decrease`,
          "-frames:v",
          "1",
          "-q:v",
          "4",
          dest,
        ],
        stdout: "null",
        stderr: "null",
      });
      const { success } = await p.output();
      if (success) return await existingFile(dest);
    } catch (e) {
      console.error("[makeThumb]", id, e);
    }
    return null;
  }

  /** Prefer a small thumb for UI; keep original on disk for cache. */
  async function uiMediaPath(
    id: string,
    original: string | null,
  ): Promise<string | null> {
    if (!original) return null;
    const thumb = await makeThumb(original, id);
    return thumb || original;
  }

  async function isValidAudioFile(path: string | null): Promise<boolean> {
    if (!path) return false;
    try {
      const st = await Deno.stat(path);
      // Corrupt/empty AAC shells from killed ffmpeg / failed OBS are ~44 bytes.
      if (st.size < 1024) return false;
      const buf = new Uint8Array(16);
      const f = await Deno.open(path, { read: true });
      try {
        await f.read(buf);
      } finally {
        f.close();
      }
      const head = new TextDecoder().decode(buf);
      // Reject truncated ftyp-only shells without moov.
      if (head.includes("ftyp") && st.size < 2048) {
        // Still might be tiny valid; probe with ffprobe if available.
      }
      const p = new Deno.Command("ffprobe", {
        args: [
          "-v",
          "error",
          "-show_entries",
          "format=duration",
          "-of",
          "csv=p=0",
          path,
        ],
        stdout: "piped",
        stderr: "null",
      }).spawn();
      const out = await p.output();
      if (!out.success) return false;
      const dur = Number(new TextDecoder().decode(out.stdout).trim());
      return Number.isFinite(dur) && dur > 0;
    } catch {
      return false;
    }
  }

  async function downloadAudioBytes(
    messageId: string,
    chatMid: string,
  ): Promise<
    {
      buf: Uint8Array;
      mime: string;
    } | null
  > {
    // Prefer TalkMessage.getData (E2EE-aware).
    const got = await refetchMessageData(chatMid, messageId, {
      allowNonImage: true,
    });
    if (got && got.buf.length >= 1024) return got;
    const client = runtime.getClient();
    if (!client) return null;
    try {
      const file = await client.base.obs.downloadMessageData({
        messageId,
        isPreview: false,
        isSquare: false,
      });
      const buf = new Uint8Array(await file.arrayBuffer());
      if (buf.length >= 1024) {
        return { buf, mime: file.type || "audio/mp4" };
      }
    } catch (e) {
      console.error("[audio obs]", messageId, e);
    }
    return got && got.buf.length > 0 ? got : null;
  }

  return {
    existingFile,
    isImageBytes,
    isImageFile,
    existingImage,
    writeImageFile,
    cacheUrl,
    avatarPathFor,
    mediaDest,
    fullDest,
    thumbDest,
    imageDimensions,
    isPreviewOrThumbImage,
    existingFullImage,
    writeFullImage,
    makeThumb,
    uiMediaPath,
    isValidAudioFile,
    downloadAudioBytes,
  };
}

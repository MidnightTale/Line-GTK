import { join } from "@std/path";
import { ensurePrivateDir } from "./storage.ts";
import { MAX_PNG_BYTES, validateStickerPng } from "./sticker_png.ts";

export type StickerOptimization = {
  maxDimension: number;
  fps: number;
  colors: number;
};
export const DEFAULT_OPTIMIZATION: StickerOptimization = {
  maxDimension: 0,
  fps: 15,
  colors: 128,
};
export const DEFAULT_ANIMATION_OPTIMIZATION: StickerOptimization = {
  maxDimension: 128,
  fps: 10,
  colors: 32,
};

export function optimizationOptions(
  value: unknown,
  animated = false,
  dimensions?: { width: number; height: number },
): StickerOptimization {
  const defaults = animated
    ? {
      ...DEFAULT_ANIMATION_OPTIMIZATION,
      maxDimension:
        dimensions && Math.max(dimensions.width, dimensions.height) > 128
          ? 256
          : 128,
    }
    : DEFAULT_OPTIMIZATION;
  if (value === undefined) return { ...defaults };
  if (!value || typeof value !== "object") {
    throw new Error("sticker_optimize_invalid");
  }
  const v = value as Record<string, unknown>;
  const result = { ...defaults, ...v };
  if (
    !Object.keys(v).every((key) =>
      ["maxDimension", "fps", "colors"].includes(key)
    ) ||
    ![0, 96, 128, 256, 384, 512, 1024].includes(result.maxDimension) ||
    ![5, 8, 10, 15, 20, 30].includes(result.fps) ||
    ![32, 64, 128, 256].includes(result.colors)
  ) {
    throw new Error("sticker_optimize_invalid");
  }
  return result;
}

let busy = false;
const waiters: (() => void)[] = [];
async function acquire(signal: AbortSignal) {
  while (busy) {
    await new Promise<void>((resolve, reject) => {
      const remove = () => {
        const i = waiters.indexOf(wake);
        if (i >= 0) waiters.splice(i, 1);
      };
      const abort = () => {
        remove();
        reject(new Error("sticker_import_cancelled"));
      };
      const wake = () => {
        signal.removeEventListener("abort", abort);
        resolve();
      };
      if (signal.aborted) {
        abort();
        return;
      }
      waiters.push(wake);
      signal.addEventListener("abort", abort, { once: true });
    });
  }
  signal.throwIfAborted();
  busy = true;
  return () => {
    busy = false;
    waiters.shift()?.();
  };
}

async function ffmpeg(bytes: Uint8Array, args: string[], signal: AbortSignal) {
  signal.throwIfAborted();
  let child: Deno.ChildProcess;
  try {
    child = new Deno.Command("ffmpeg", {
      args: [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-threads",
        "1",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
        ...args,
      ],
      stdin: "piped",
      stdout: "piped",
      stderr: "piped",
      signal,
    }).spawn();
  } catch (e) {
    if (
      e instanceof Deno.errors.NotFound ||
      e instanceof Deno.errors.PermissionDenied
    ) throw new Error("sticker_ffmpeg_missing");
    throw e;
  }
  const writing = (async () => {
    const writer = child.stdin.getWriter();
    try {
      await writer.write(bytes);
    } finally {
      await writer.close().catch(() => {});
      writer.releaseLock();
    }
  })();
  const reading = (async () => {
    const reader = child.stdout.getReader();
    const parts: Uint8Array[] = [];
    let length = 0;
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        length += value.length;
        if (length > MAX_PNG_BYTES) {
          try {
            child.kill("SIGKILL");
          } catch { /* exited */ }
          throw new Error("sticker_optimize_too_large");
        }
        parts.push(value);
      }
    } finally {
      reader.releaseLock();
    }
    const result = new Uint8Array(length);
    let offset = 0;
    for (const part of parts) {
      result.set(part, offset);
      offset += part.length;
    }
    return result;
  })();
  const readError = (async () => {
    const reader = child.stderr.getReader();
    const parts: Uint8Array[] = [];
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        parts.push(value);
      }
    } finally {
      reader.releaseLock();
    }
    return new TextDecoder().decode(
      parts.length === 1
        ? parts[0]
        : new Uint8Array(parts.flatMap((p) => [...p])),
    ).trim();
  })();
  const outcomes = await Promise.allSettled([
    writing,
    reading,
    child.status,
    readError,
  ]);
  signal.throwIfAborted();
  const read = outcomes[1], status = outcomes[2];
  if (read.status === "rejected") throw read.reason;
  if (
    outcomes[0].status === "rejected" || status.status !== "fulfilled" ||
    !status.value.success
  ) {
    const diagnostic = outcomes[3].status === "fulfilled"
      ? outcomes[3].value.slice(-1000)
      : "";
    throw new Error(
      diagnostic
        ? `sticker_optimize_failed: ${diagnostic}`
        : "sticker_optimize_failed",
    );
  }
  return read.value;
}

/** Keep originals intact. Use a global palette and preserve binary GIF alpha. */
export async function optimizeStickerImage(
  bytes: Uint8Array,
  options: StickerOptimization,
  workDir: string,
  cancellation: AbortSignal,
) {
  const info = validateStickerPng(bytes, true);
  const signal = AbortSignal.any([cancellation, AbortSignal.timeout(45_000)]);
  const release = await acquire(signal);
  let temporary: string | undefined;
  try {
    const size = options.maxDimension || Math.max(info.width, info.height);
    const scale =
      `scale=w='min(iw,${size})':h='min(ih,${size})':force_original_aspect_ratio=decrease:flags=lanczos`;
    const input = [
      "-f",
      info.animated ? "apng" : "png_pipe",
      ...(info.animated ? ["-ignore_loop", "1"] : []),
      "-i",
      "pipe:0",
    ];
    let output: Uint8Array;
    if (info.animated) {
      await ensurePrivateDir(workDir);
      temporary = await Deno.makeTempDir({
        dir: workDir,
        prefix: ".sticker-optimize-",
      });
      // Older APNG demuxers require seeking, which stdin cannot provide.
      const sourcePath = join(temporary, "source.png");
      await Deno.writeFile(sourcePath, bytes, { mode: 0o600 });
      input[input.length - 1] = sourcePath;
      const palette = await ffmpeg(new Uint8Array(), [
        ...input,
        "-vf",
        `fps=${options.fps},${scale},palettegen=max_colors=${options.colors}:reserve_transparent=1`,
        "-frames:v",
        "1",
        "-c:v",
        "png",
        "-f",
        "image2pipe",
        "pipe:1",
      ], signal);
      const palettePath = join(temporary, "palette.png");
      await Deno.writeFile(palettePath, palette, { mode: 0o600 });
      output = await ffmpeg(new Uint8Array(), [
        ...input,
        "-i",
        palettePath,
        "-filter_complex",
        `[0:v]fps=${options.fps},${scale}[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=3:alpha_threshold=128:diff_mode=rectangle`,
        "-loop",
        "0",
        "-f",
        "gif",
        "pipe:1",
      ], signal);
      if (
        new TextDecoder().decode(output.subarray(0, 6)) !== "GIF89a" ||
        output.length < 32 || output.at(-1) !== 0x3b
      ) throw new Error("sticker_optimize_failed");
    } else {
      output = await ffmpeg(bytes, [
        ...input,
        "-vf",
        scale,
        "-frames:v",
        "1",
        "-compression_level",
        "9",
        "-c:v",
        "png",
        "-f",
        "image2pipe",
        "pipe:1",
      ], signal);
      const resized = validateStickerPng(output);
      if (
        output.length > bytes.length && resized.width === info.width &&
        resized.height === info.height
      ) output = bytes;
    }
    return {
      bytes: output,
      extension: info.animated ? "gif" : "png",
      animated: info.animated,
      originalBytes: bytes.length,
    };
  } catch (e) {
    if (cancellation.aborted) throw new Error("sticker_import_cancelled");
    if (signal.aborted) throw new Error("sticker_optimize_timeout");
    throw e;
  } finally {
    if (temporary) {
      await Deno.remove(temporary, { recursive: true }).catch(() => {});
    }
    release();
  }
}

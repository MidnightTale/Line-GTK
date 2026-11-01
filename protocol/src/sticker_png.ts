import { inflateSync } from "node:zlib";

export const MAX_PNG_BYTES = 10 * 1024 * 1024;
export const MAX_PNG_DIMENSION = 2048;

const signature = [137, 80, 78, 71, 13, 10, 26, 10];
const crcTable = Uint32Array.from({ length: 256 }, (_, n) => {
  for (let k = 0; k < 8; k++) n = n & 1 ? 0xedb88320 ^ (n >>> 1) : n >>> 1;
  return n >>> 0;
});

function crc(bytes: Uint8Array): number {
  let n = 0xffffffff;
  for (const byte of bytes) n = crcTable[(n ^ byte) & 255] ^ (n >>> 8);
  return (n ^ 0xffffffff) >>> 0;
}

export type StickerPngInfo = {
  width: number;
  height: number;
  animated: boolean;
  frames: number;
  durationMs: number;
};
const MAX_FRAMES = 240;
const MAX_DECODED_PIXELS = 32 * 1024 * 1024;

/** Validate PNG/APNG structure and every bounded frame stream before decoding. */
export function validateStickerPng(
  bytes: Uint8Array,
  allowAnimation = false,
): StickerPngInfo {
  if (bytes.length > MAX_PNG_BYTES) throw new Error("sticker_png_too_large");
  const invalid = () => {
    throw new Error("sticker_png_invalid");
  };
  if (!signature.every((b, i) => bytes[i] === b)) invalid();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let width = 0, height = 0, depth = 0, channels = 0, interlace = 0;
  let ended = false, hasPalette = false, dataEnded = false;
  let declaredFrames = 0, sequence = 0, durationMs = 0, decodedPixels = 0;
  type Frame = {
    width: number;
    height: number;
    parts: Uint8Array[];
    usesDefault: boolean;
  };
  const frames: Frame[] = [];
  const compressed: Uint8Array[] = [];
  let current: Frame | undefined;
  for (let offset = 8; offset + 12 <= bytes.length;) {
    const size = view.getUint32(offset);
    if (size > bytes.length - offset - 12) invalid();
    const type = String.fromCharCode(...bytes.subarray(offset + 4, offset + 8));
    const data = bytes.subarray(offset + 8, offset + 8 + size);
    if (
      crc(bytes.subarray(offset + 4, offset + 8 + size)) !==
        view.getUint32(offset + 8 + size)
    ) invalid();
    if (offset === 8 && type !== "IHDR") invalid();
    if (type === "IHDR") {
      if (width || size !== 13) invalid();
      width = view.getUint32(offset + 8);
      height = view.getUint32(offset + 12);
      depth = data[8];
      const color = data[9];
      channels =
        ({ 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 } as Record<number, number>)[color] ??
          0;
      const depths = color === 0
        ? [1, 2, 4, 8, 16]
        : color === 3
        ? [1, 2, 4, 8]
        : [8, 16];
      if (
        !width || !height || !channels || !depths.includes(depth) || data[10] ||
        data[11] || data[12] > 1
      ) invalid();
      if (width > MAX_PNG_DIMENSION || height > MAX_PNG_DIMENSION) {
        throw new Error("sticker_png_dimensions");
      }
      interlace = data[12];
    } else if (type === "acTL") {
      if (!allowAnimation) throw new Error("sticker_png_animated");
      if (size !== 8 || declaredFrames || compressed.length) invalid();
      declaredFrames = view.getUint32(offset + 8);
      if (!declaredFrames || declaredFrames > MAX_FRAMES) {
        throw new Error("sticker_animation_limits");
      }
    } else if (type === "fcTL") {
      if (!allowAnimation) throw new Error("sticker_png_animated");
      if (
        !declaredFrames || size !== 26 ||
        view.getUint32(offset + 8) !== sequence++
      ) invalid();
      if (current && !current.parts.length) invalid();
      const w = view.getUint32(offset + 12), h = view.getUint32(offset + 16);
      const x = view.getUint32(offset + 20), y = view.getUint32(offset + 24);
      if (
        !w || !h || x + w > width || y + h > height || data[24] > 2 ||
        data[25] > 1
      ) invalid();
      const usesDefault = !compressed.length;
      if (usesDefault && (x || y || w !== width || h !== height)) invalid();
      durationMs += Math.max(
        10,
        view.getUint16(offset + 28) * 1000 /
          (view.getUint16(offset + 30) || 100),
      );
      decodedPixels += width * height;
      if (
        durationMs > 30_000 || decodedPixels > MAX_DECODED_PIXELS ||
        frames.length >= MAX_FRAMES
      ) throw new Error("sticker_animation_limits");
      current = { width: w, height: h, parts: [], usesDefault };
      frames.push(current);
      if (compressed.length) dataEnded = true;
    } else if (type === "fdAT") {
      if (!allowAnimation) throw new Error("sticker_png_animated");
      if (
        !current || current.usesDefault || size <= 4 ||
        view.getUint32(offset + 8) !== sequence++
      ) invalid();
      current!.parts.push(data.subarray(4));
    } else if (type === "PLTE") {
      if (compressed.length || !size || size % 3 || size > 768) invalid();
      hasPalette = true;
    } else if (type === "IDAT") {
      if (
        dataEnded || (bytes[25] === 3 && !hasPalette) ||
        (current && !current.usesDefault)
      ) invalid();
      compressed.push(data);
      if (current?.usesDefault) current.parts.push(data);
    } else if (type === "IEND") {
      if (size || !compressed.length || offset + 12 !== bytes.length) invalid();
      ended = true;
      break;
    } else {
      if (compressed.length) dataEnded = true;
      if ((bytes[offset + 4] & 32) === 0) invalid();
    }
    offset += size + 12;
  }
  if (
    !ended ||
    (declaredFrames &&
      (frames.length !== declaredFrames || frames.some((f) => !f.parts.length)))
  ) invalid();
  const streams = frames[0]?.usesDefault
    ? frames
    : [{ width, height, parts: compressed, usesDefault: true }, ...frames];
  if (
    decodedPixels + (frames[0]?.usesDefault ? 0 : width * height) >
      MAX_DECODED_PIXELS
  ) throw new Error("sticker_animation_limits");
  for (const frame of streams) {
    const passes = interlace
      ? [[0, 0, 8, 8], [4, 0, 8, 8], [0, 4, 4, 8], [2, 0, 4, 4], [0, 2, 2, 4], [
        1,
        0,
        2,
        2,
      ], [0, 1, 1, 2]]
      : [[0, 0, 1, 1]];
    const rows: { bytes: number; count: number }[] = [];
    for (const [x, y, dx, dy] of passes) {
      const w = Math.max(0, Math.ceil((frame.width - x) / dx));
      const h = Math.max(0, Math.ceil((frame.height - y) / dy));
      if (w && h) {
        rows.push({ bytes: 1 + Math.ceil(w * depth * channels / 8), count: h });
      }
    }
    const expected = rows.reduce((n, r) => n + r.bytes * r.count, 0);
    const joined = new Uint8Array(
      frame.parts.reduce((n, b) => n + b.length, 0),
    );
    let cursor = 0;
    for (const part of frame.parts) {
      joined.set(part, cursor);
      cursor += part.length;
    }
    let pixels: Uint8Array;
    try {
      pixels = inflateSync(joined, { maxOutputLength: expected });
    } catch {
      invalid();
    }
    if (pixels!.length !== expected) invalid();
    cursor = 0;
    for (const row of rows) {
      for (let i = 0; i < row.count; i++, cursor += row.bytes) {
        if (pixels![cursor] > 4) invalid();
      }
    }
  }
  return {
    width,
    height,
    animated: !!declaredFrames,
    frames: declaredFrames || 1,
    durationMs,
  };
}

/** Read from one opened handle, including growth checks, rather than stat/read races. */
export async function readStickerPng(
  path: string,
  allowAnimation = false,
): Promise<Uint8Array> {
  const file = await Deno.open(path, { read: true });
  try {
    const stat = await file.stat();
    if (!stat.isFile) throw new Error("sticker_png_invalid");
    if (stat.size > MAX_PNG_BYTES) throw new Error("sticker_png_too_large");
    const bytes = new Uint8Array(stat.size + 1);
    let length = 0;
    while (length < bytes.length) {
      const n = await file.read(bytes.subarray(length));
      if (n === null) break;
      length += n;
    }
    if (length !== stat.size) throw new Error("sticker_png_changed");
    const result = bytes.subarray(0, length);
    validateStickerPng(result, allowAnimation);
    return result;
  } finally {
    file.close();
  }
}

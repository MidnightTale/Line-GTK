export function isVisualType(contentType: string): boolean {
  return contentType === "IMAGE" || contentType === "VIDEO" ||
    contentType === "STICKER";
}

export function isMediaType(contentType: string): boolean {
  return isVisualType(contentType) || contentType === "AUDIO" ||
    contentType === "FILE";
}

/** LINE sometimes omits contentMetadata; linejs otherwise crashes. */
export function normalizeRaw(
  raw: Record<string, unknown> | null | undefined,
): Record<string, unknown> {
  const normalized = { ...(raw ?? {}) };
  if (
    normalized.contentMetadata == null ||
    typeof normalized.contentMetadata !== "object"
  ) {
    normalized.contentMetadata = {};
  }
  return normalized;
}

/** Convert the thrift representations used by LINE into an integer value. */
export function coerceI64(value: unknown): bigint | number {
  if (typeof value === "bigint") return value;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && /^-?\d+$/.test(value)) {
    const number = Number(value);
    return Number.isSafeInteger(number) ? number : BigInt(value);
  }
  if (
    typeof value === "object" && value !== null &&
    typeof (value as { toString?: () => string }).toString === "function"
  ) {
    const string = String(value);
    if (/^-?\d+$/.test(string)) return BigInt(string);
  }
  throw new TypeError(
    `cannot coerce thrift i64 from ${typeof value}: ${value}`,
  );
}

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

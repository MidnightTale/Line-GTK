export function isVisualType(contentType: string): boolean {
  return contentType === "IMAGE" || contentType === "VIDEO" ||
    contentType === "STICKER";
}

export function isMediaType(contentType: string): boolean {
  return isVisualType(contentType) || contentType === "AUDIO" ||
    contentType === "FILE";
}

export type SticonResource = {
  start: number;
  end: number;
  productId: string;
  sticonId: string;
  resourceType: string;
};

/** Resolve the conversation MID for a Talk message, including group/room traffic. */
export function talkChatMid(message: {
  from?: unknown;
  to?: unknown;
  mine?: unknown;
}): string {
  const from = String(message.from ?? "");
  const to = String(message.to ?? "");
  if (to.startsWith("c") || to.startsWith("r")) return to;
  return message.mine ? to : from;
}

/** Parse LINE emoji (STICON) replacements carried by NONE text messages. */
export function sticonResources(
  metadata: Record<string, string> | null | undefined,
): SticonResource[] {
  const encoded = metadata?.REPLACE;
  if (!encoded) return [];
  try {
    const parsed = JSON.parse(encoded) as {
      sticon?: { resources?: Record<string, unknown>[] };
    };
    const rows = parsed.sticon?.resources;
    if (!Array.isArray(rows)) return [];
    return rows.flatMap((row) => {
      const productId = String(row.productId ?? "").trim();
      const sticonId = String(row.sticonId ?? "").trim();
      if (!productId || !sticonId) return [];
      const start = Number(row.S ?? 0);
      const end = Number(row.E ?? start);
      return [{
        start: Number.isFinite(start) ? start : 0,
        end: Number.isFinite(end) ? end : 0,
        productId,
        sticonId,
        resourceType: String(row.resourceType ?? "STATIC"),
      }];
    });
  } catch {
    return [];
  }
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

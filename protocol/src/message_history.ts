type Json = Record<string, unknown>;

/** Default for messages kept per chat in the on-disk history. */
export const MAX_STORED_MESSAGES = 2000;
/** Share of the limit at which the user is warned. */
export const CACHE_WARN_RATIO = 0.9;

/** Settings value → limit; 0 means unlimited, anything invalid the default. */
export function normalizeMessageLimit(value: unknown): number {
  const n = Number(value);
  if (!Number.isFinite(n) || n < 0) return MAX_STORED_MESSAGES;
  return n === 0 ? 0 : Math.max(100, Math.floor(n));
}

/** "ok" below the warning line, "near" close to the limit, "full" at it. */
export function cacheFill(
  count: number,
  limit: number,
): "ok" | "near" | "full" {
  if (limit <= 0) return "ok";
  if (count >= limit) return "full";
  return count >= limit * CACHE_WARN_RATIO ? "near" : "ok";
}

function keepNewest(messages: Json[], max: number): Json[] {
  return max > 0 ? messages.slice(-max) : messages;
}
export const HISTORY_PAGE_DEFAULT = 50;
export const HISTORY_PAGE_MAX = 100;

function time(message: Json): number {
  const value = Number(message.createdTime ?? 0);
  return Number.isFinite(value) ? value : 0;
}

function byTime(a: Json, b: Json): number {
  return time(a) - time(b);
}

export function sortMessages(messages: Json[]): Json[] {
  return messages.slice().sort(byTime);
}

/**
 * Merge a freshly fetched batch into the stored history.
 *
 * A batch that is newer than everything stored and shares no message with it
 * leaves a hole in the history, so the old part is dropped; older pages are
 * fetched again on demand.
 */
export function mergeMessages(
  existing: Json[],
  incoming: Json[],
  /** `max` 0 keeps everything. */
  opts: { contiguous?: boolean; max?: number } = {},
): Json[] {
  const max = opts.max ?? MAX_STORED_MESSAGES;
  if (!incoming.length) return keepNewest(sortMessages(existing), max);
  if (!existing.length) return keepNewest(sortMessages(incoming), max);

  const known = new Set(existing.map((m) => String(m.id)));
  const overlaps = incoming.some((m) => known.has(String(m.id)));
  const existingNewest = Math.max(...existing.map(time));
  const incomingOldest = Math.min(...incoming.map(time));
  const hole = !opts.contiguous && !overlaps && incomingOldest > existingNewest;

  const combined = new Map<string, Json>();
  if (!hole) { for (const m of existing) combined.set(String(m.id), m); }
  for (const m of incoming) combined.set(String(m.id), m);
  return keepNewest(sortMessages([...combined.values()]), max);
}

/** Index of the anchor in a time-sorted list, or the nearest position. */
function anchorIndex(messages: Json[], id: string, at: number): number {
  const exact = id ? messages.findIndex((m) => String(m.id) === id) : -1;
  if (exact >= 0) return exact;
  let idx = messages.findIndex((m) => time(m) >= at);
  if (idx < 0) idx = messages.length;
  return idx;
}

/** Up to `limit` messages directly before the anchor, oldest first. */
export function olderThan(
  sorted: Json[],
  beforeId: string,
  beforeTime: number,
  limit: number,
): Json[] {
  const idx = anchorIndex(sorted, beforeId, beforeTime);
  return sorted.slice(Math.max(0, idx - limit), idx);
}

/** Up to `limit` messages directly after the anchor, oldest first. */
export function newerThan(
  sorted: Json[],
  afterId: string,
  afterTime: number,
  limit: number,
): { messages: Json[]; hasMore: boolean } {
  let idx = sorted.findIndex((m) => String(m.id) === afterId);
  if (idx < 0) {
    let last = -1;
    sorted.forEach((m, i) => {
      if (time(m) <= afterTime) last = i;
    });
    idx = last;
  }
  const start = idx + 1;
  const messages = sorted.slice(start, start + limit);
  return { messages, hasMore: start + limit < sorted.length };
}

export function clampPageSize(value: unknown): number {
  const n = Math.floor(Number(value));
  if (!Number.isFinite(n) || n <= 0) return HISTORY_PAGE_DEFAULT;
  return Math.min(n, HISTORY_PAGE_MAX);
}

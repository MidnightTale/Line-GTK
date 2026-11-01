import { mergeMessages } from "./message_history.ts";

type Json = Record<string, unknown>;

/** Messages fetched when only the newest part of a chat is missing. */
export const DELTA_PAGE = 20;

function time(message: Json): number {
  const value = Number(message.createdTime ?? 0);
  return Number.isFinite(value) ? value : 0;
}

export function newestMessage(messages: Json[]): Json | undefined {
  let newest: Json | undefined;
  for (const message of messages) {
    if (!newest || time(message) >= time(newest)) newest = message;
  }
  return newest;
}

/** The stored history already ends at the newest message the server has. */
export function reachesCursor(
  messages: Json[],
  cursorId: unknown,
): boolean {
  if (cursorId === undefined || cursorId === null) return false;
  const newest = newestMessage(messages);
  return !!newest && String(newest.id) === String(cursorId);
}

/**
 * Join a small batch of the newest messages onto stored history.
 * Returns null when the batch does not touch the stored messages, because
 * the part in between is unknown and the caller must fetch a full page.
 */
export function mergeDelta(
  stored: Json[],
  batch: Json[],
  limit: number,
): Json[] | null {
  if (!stored.length || !batch.length) return null;
  const known = new Set(stored.map((m) => String(m.id)));
  if (!batch.some((m) => known.has(String(m.id)))) return null;
  return mergeMessages(stored, batch).slice(-limit);
}

/** Add one live or sent message to an already validated history. */
export function appendLive(stored: Json[], message: Json): Json[] {
  return mergeMessages(stored, [message], { contiguous: true });
}

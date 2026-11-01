import type { Buffer } from "node:buffer";
import { atomicWriteJson } from "./storage.ts";
import { readMessageFile } from "./message_store.ts";

type Json = Record<string, unknown>;

export type BootProfile = {
  mid: string;
  displayName: string;
  statusMessage: string;
  picturePath: string | null;
  avatarPath: string | null;
  pictureUrl: string | null;
};

export type BootCache = {
  accountMid: string;
  profile: BootProfile;
  chats: Json[];
};

const CHAT_MID = /^[A-Za-z0-9_-]{1,128}$/;

const text = (value: unknown) => typeof value === "string" ? value : "";
const optionalText = (value: unknown) =>
  typeof value === "string" && value ? value : null;

/** Remember who signed in last so the next start can show their cache first. */
export async function saveBootSession(path: string, profile: BootProfile) {
  if (!profile.mid) return;
  await atomicWriteJson(path, { at: Date.now(), ...profile });
}

/**
 * Chat list and profile stored by an earlier run. Returns null unless the
 * list was saved for the same account as the remembered session.
 */
export async function loadBootCache(
  sessionPath: string,
  chatCachePath: string,
  maxAge: number,
): Promise<BootCache | null> {
  try {
    const session = JSON.parse(await Deno.readTextFile(sessionPath));
    const accountMid = text(session?.mid);
    if (!accountMid) return null;
    const raw = JSON.parse(await Deno.readTextFile(chatCachePath));
    if (
      raw?.accountMid !== accountMid || !Array.isArray(raw.chats) ||
      !(Date.now() - Number(raw.at || 0) < maxAge)
    ) return null;
    return {
      accountMid,
      profile: {
        mid: accountMid,
        displayName: text(session.displayName),
        statusMessage: text(session.statusMessage),
        picturePath: optionalText(session.picturePath),
        avatarPath: optionalText(session.avatarPath),
        pictureUrl: optionalText(session.pictureUrl),
      },
      chats: raw.chats as Json[],
    };
  } catch {
    return null;
  }
}

/** Stored messages of one chat, only when they belong to this account. */
export async function loadBootMessages(
  directory: string,
  chatMid: string,
  accountMid: string,
  maxAge: number,
  key: Buffer | null = null,
): Promise<Json[] | null> {
  if (!accountMid || !CHAT_MID.test(chatMid)) return null;
  try {
    const raw = await readMessageFile(directory, chatMid, key);
    if (
      raw?.accountMid !== accountMid || !Array.isArray(raw.messages) ||
      Date.now() - Number(raw.at || 0) > maxAge
    ) return null;
    return raw.messages as Json[];
  } catch {
    return null;
  }
}

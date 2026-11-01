import type { Buffer } from "node:buffer";
import { join } from "@std/path";
import { openCache, sealCache } from "./cache_crypto.ts";
import { atomicWriteTextFile } from "./storage.ts";

export type StoredMessages = {
  at: number;
  accountMid: string;
  messages: Record<string, unknown>[];
};

/** Parsed per-chat message cache; throws when missing or unreadable. */
export async function readMessageFile(
  directory: string,
  chatMid: string,
  key: Buffer | null,
): Promise<Record<string, unknown>> {
  const name = `${chatMid}.json`;
  const data = await Deno.readFile(join(directory, name));
  return JSON.parse(openCache(data, key, name));
}

/** Atomically (re)write one chat's cache, encrypted when a key is present. */
export async function writeMessageFile(
  directory: string,
  chatMid: string,
  key: Buffer | null,
  value: StoredMessages,
): Promise<void> {
  const name = `${chatMid}.json`;
  await atomicWriteTextFile(
    join(directory, name),
    sealCache(JSON.stringify(value), key, name),
  );
}

import { Buffer } from "node:buffer";
import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";

/** Marks an encrypted cache file; legacy plaintext JSON starts with `{`. */
const MAGIC = Buffer.from("LGC1");
const IV_BYTES = 12;
const TAG_BYTES = 16;
export const KEY_BYTES = 32;

/** Parse the hex (or base64) key handed over by the GTK process, or null when absent. */
export function parseCacheKey(value: string | undefined | null): Buffer | null {
  if (!value) return null;
  const text = value.trim();
  const key = /^[0-9a-fA-F]{64}$/.test(text)
    ? Buffer.from(text, "hex")
    : Buffer.from(text, "base64");
  return key.length === KEY_BYTES ? key : null;
}

export function isEncrypted(data: Uint8Array): boolean {
  return data.length > MAGIC.length &&
    Buffer.from(data.subarray(0, MAGIC.length)).equals(MAGIC);
}

/**
 * AES-256-GCM. `aad` binds the ciphertext to its file name so one chat's cache
 * cannot be swapped into another's. Without a key the text is stored as-is.
 */
export function sealCache(
  text: string,
  key: Buffer | null,
  aad: string,
): Uint8Array {
  if (!key) return Buffer.from(text, "utf8");
  const iv = randomBytes(IV_BYTES);
  const cipher = createCipheriv("aes-256-gcm", key, iv);
  cipher.setAAD(Buffer.from(aad));
  const body = Buffer.concat([cipher.update(text, "utf8"), cipher.final()]);
  return Buffer.concat([MAGIC, iv, body, cipher.getAuthTag()]);
}

/** Inverse of `sealCache`; accepts legacy plaintext so old caches still load. */
export function openCache(
  data: Uint8Array,
  key: Buffer | null,
  aad: string,
): string {
  if (!isEncrypted(data)) return Buffer.from(data).toString("utf8");
  if (!key) throw new Error("cache_key_missing");
  const buffer = Buffer.from(data);
  const iv = buffer.subarray(MAGIC.length, MAGIC.length + IV_BYTES);
  const tag = buffer.subarray(buffer.length - TAG_BYTES);
  const body = buffer.subarray(
    MAGIC.length + IV_BYTES,
    buffer.length - TAG_BYTES,
  );
  const decipher = createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAAD(Buffer.from(aad));
  decipher.setAuthTag(tag);
  return Buffer.concat([decipher.update(body), decipher.final()]).toString(
    "utf8",
  );
}

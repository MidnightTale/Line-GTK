import { assert, assertEquals, assertRejects, assertThrows } from "@std/assert";
import { join } from "@std/path";
import { Buffer } from "node:buffer";
import {
  isEncrypted,
  openCache,
  parseCacheKey,
  sealCache,
} from "../src/cache_crypto.ts";
import { readMessageFile, writeMessageFile } from "../src/message_store.ts";
import {
  cacheFill,
  mergeMessages,
  normalizeMessageLimit,
} from "../src/message_history.ts";

const key = Buffer.alloc(32, 7);
const sample = {
  at: 1,
  accountMid: "u1",
  messages: [{ id: "1", text: "secret" }],
};

Deno.test("encrypted cache hides content and round-trips", async () => {
  const dir = await Deno.makeTempDir();
  try {
    await writeMessageFile(dir, "c1", key, sample);
    const raw = await Deno.readFile(join(dir, "c1.json"));
    assert(isEncrypted(raw));
    assert(!new TextDecoder().decode(raw).includes("secret"));
    assertEquals(await readMessageFile(dir, "c1", key), sample);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("legacy plaintext cache still loads and is upgraded on write", async () => {
  const dir = await Deno.makeTempDir();
  try {
    await Deno.writeTextFile(join(dir, "c1.json"), JSON.stringify(sample));
    assertEquals(await readMessageFile(dir, "c1", key), sample);
    await writeMessageFile(dir, "c1", key, sample);
    assert(isEncrypted(await Deno.readFile(join(dir, "c1.json"))));
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("wrong key, missing key and swapped files are rejected", async () => {
  const sealed = sealCache("{}", key, "c1.json");
  assertThrows(() => openCache(sealed, Buffer.alloc(32, 8), "c1.json"));
  assertThrows(
    () => openCache(sealed, null, "c1.json"),
    Error,
    "cache_key_missing",
  );
  assertThrows(() => openCache(sealed, key, "c2.json"));
  const dir = await Deno.makeTempDir();
  try {
    await writeMessageFile(dir, "c1", key, sample);
    await assertRejects(() => readMessageFile(dir, "c1", Buffer.alloc(32, 9)));
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("no key keeps plaintext; key parsing validates length", () => {
  assertEquals(new TextDecoder().decode(sealCache("{}", null, "x")), "{}");
  assertEquals(parseCacheKey(""), null);
  assertEquals(parseCacheKey("AAAA"), null);
  assertEquals(parseCacheKey(key.toString("base64"))?.length, 32);
});

Deno.test("message limit: normalize, fill levels, unlimited merge", () => {
  assertEquals(normalizeMessageLimit(undefined), 2000);
  assertEquals(normalizeMessageLimit("x"), 2000);
  assertEquals(normalizeMessageLimit(0), 0);
  assertEquals(normalizeMessageLimit(10), 100);
  assertEquals(normalizeMessageLimit(5000), 5000);
  assertEquals(cacheFill(100, 2000), "ok");
  assertEquals(cacheFill(1800, 2000), "near");
  assertEquals(cacheFill(2000, 2000), "full");
  assertEquals(cacheFill(999999, 0), "ok");
  const many = Array.from(
    { length: 3000 },
    (_, i) => ({ id: `${i}`, createdTime: i }),
  );
  assertEquals(mergeMessages([], many, { max: 0 }).length, 3000);
  assertEquals(mergeMessages([], many, { max: 2000 }).length, 2000);
});

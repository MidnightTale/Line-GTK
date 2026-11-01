import { assertEquals, assertRejects } from "@std/assert";
import { InternalError } from "@evex/linejs/base";
import {
  DECRYPTION_FAILED,
  guardedDecrypt,
  retryKeyDecryption,
} from "../src/e2ee_guard.ts";

function rawMessage(id = "1") {
  return {
    id,
    to: "c-group",
    from: "u-new-member",
    contentType: "NONE",
    contentMetadata: { e2eeVersion: "2" },
    chunks: Array.from({ length: 5 }, () => new Uint8Array(4)),
  };
}

Deno.test("incoming message waits for missing group keys and is delivered once in order", async () => {
  const delivered: string[] = [];
  const waits: number[] = [];
  const attempts: string[] = [];
  let keyReady = false;
  const decrypt = retryKeyDecryption(
    (message) => {
      attempts.push(String(message.id));
      if (!keyReady) {
        return Promise.reject(
          new InternalError("TalkException", "key pending", {
            code: "NOT_FOUND",
          }),
        );
      }
      return Promise.resolve({ ...message, text: `decrypted ${message.id}` });
    },
    () => true,
    (ms) => {
      waits.push(ms);
      keyReady = true;
      return Promise.resolve();
    },
  );
  // Match linejs's sequential receive loop: decryption completes before emit.
  for (const id of ["1", "2"]) {
    delivered.push(String((await decrypt(rawMessage(id))).text));
  }
  assertEquals(attempts, ["1", "1", "2"]);
  assertEquals(delivered, ["decrypted 1", "decrypted 2"]);
  assertEquals(waits, [250]);
});

Deno.test("exhausted key retries retain a readable failure placeholder and original ciphertext", async () => {
  let attempts = 0;
  let failures = 0;
  const waits: number[] = [];
  const raw = rawMessage();
  const decrypt = guardedDecrypt(
    retryKeyDecryption(
      () => {
        attempts++;
        return Promise.reject(
          new InternalError("NoE2EEKey", "private error fixture"),
        );
      },
      () => true,
      (ms) => {
        waits.push(ms);
        return Promise.resolve();
      },
    ),
    () => failures++,
  );
  const result = await decrypt(raw);
  assertEquals(attempts, 3);
  assertEquals(waits, [250, 750]);
  assertEquals(failures, 1);
  assertEquals(result.text, "");
  assertEquals(
    (result.contentMetadata as Record<string, string>)[DECRYPTION_FAILED],
    "1",
  );
  assertEquals(result.chunks, raw.chunks);
  assertEquals(raw, rawMessage());
});

for (
  const error of [
    new InternalError("TalkException", "logged out", {
      code: "NOT_AUTHORIZED",
    }),
    new InternalError("InvalidSpec", "unsupported encryption version"),
    new TypeError("malformed message"),
  ]
) {
  Deno.test(`${error.name} is not retried as a missing key`, async () => {
    let attempts = 0;
    const decrypt = retryKeyDecryption(
      () => {
        attempts++;
        return Promise.reject(error);
      },
      () => true,
      () => {
        throw new Error("unexpected sleep");
      },
    );
    assertEquals(await assertRejects(() => decrypt(rawMessage())), error);
    assertEquals(attempts, 1);
  });
}

Deno.test("network failures during read-only key lookup can recover", async () => {
  let attempts = 0;
  const decrypt = retryKeyDecryption(
    (raw) =>
      ++attempts === 1
        ? Promise.reject(new DOMException("offline", "NetworkError"))
        : Promise.resolve({ ...raw, text: "recovered" }),
    () => true,
    () => Promise.resolve(),
  );
  assertEquals((await decrypt(rawMessage())).text, "recovered");
  assertEquals(attempts, 2);
});

Deno.test("session change cancels retries without using the new account", async () => {
  let active = true;
  let attempts = 0;
  const decrypt = retryKeyDecryption(
    () => {
      attempts++;
      return Promise.reject(
        new InternalError("TalkException", "pending", { code: 5 }),
      );
    },
    () => active,
    () => {
      active = false;
      return Promise.resolve();
    },
  );
  await assertRejects(
    () => decrypt(rawMessage()),
    Error,
    "receiver_session_changed",
  );
  assertEquals(attempts, 1);
});

Deno.test("late decrypt success from a retired session is rejected", async () => {
  let active = true;
  const decrypt = retryKeyDecryption((raw) => {
    active = false;
    return Promise.resolve({ ...raw, text: "old account" });
  }, () => active);
  await assertRejects(
    () => decrypt(rawMessage()),
    Error,
    "receiver_session_changed",
  );
});

Deno.test("each attempt uses original metadata and removes a stale failure marker on success", async () => {
  const raw = {
    ...rawMessage(),
    decryptionFailed: true,
    contentMetadata: {
      e2eeVersion: "2",
      [DECRYPTION_FAILED]: "1",
    },
  };
  let attempts = 0;
  const decrypt = retryKeyDecryption(
    (message) => {
      const metadata = message.contentMetadata as Record<string, string>;
      assertEquals(metadata[DECRYPTION_FAILED], undefined);
      assertEquals(metadata.mutated, undefined);
      assertEquals(message.decryptionFailed, undefined);
      if (++attempts === 1) {
        metadata.mutated = "true";
        (message.chunks as unknown[]).pop();
        return Promise.reject(
          new InternalError("No E2EEKey", "pending public key"),
        );
      }
      assertEquals((message.chunks as unknown[]).length, 5);
      return Promise.resolve({ ...message, text: "recovered" });
    },
    () => true,
    () => Promise.resolve(),
  );
  const result = await decrypt(raw);
  assertEquals(result.text, "recovered");
  assertEquals(
    (result.contentMetadata as Record<string, string>)[DECRYPTION_FAILED],
    undefined,
  );
  assertEquals(raw.contentMetadata[DECRYPTION_FAILED], "1");
  assertEquals(raw.chunks.length, 5);
});

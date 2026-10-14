import { assertEquals, assertRejects } from "@std/assert";
import { Buffer } from "node:buffer";
import crypto from "node:crypto";
import nacl from "npm:tweetnacl@1.0.3";
import { BaseClient } from "@evex/linejs/base";
import { MemoryStorage } from "@evex/linejs/storage";
import { type NestedArray, Protocols, Thrift } from "@evex/linejs/thrift";
const thriftSource =
  "https://jsr.io/@evex/linejs/3.5.0/base/thrift/readwrite/write.ts";
const { writeStruct } = await import(thriftSource);
const E2EE = new BaseClient({
  device: "DESKTOPWIN",
  storage: new MemoryStorage(),
}).e2ee.constructor as new (base: never) => BaseClient["e2ee"];
import {
  DECRYPTION_FAILED,
  guardedDecrypt,
  needsDecryptionRefresh,
  restoreCachedMessages,
} from "../src/e2ee_guard.ts";

Deno.test("同じ鍵IDを持つ別の相手の公開鍵を混同せず、旧キャッシュを使わない", async () => {
  const saved = new Map<string, string>([[
    "e2eePublicKeys:1",
    Buffer.from("旧鍵").toString("base64"),
  ]]);
  const requested: string[] = [];
  const e2ee = new E2EE({
    storage: {
      get: (key: string) => Promise.resolve(saved.get(key)),
      set: (key: string, value: string) => {
        saved.set(key, value);
        return Promise.resolve();
      },
    },
    talk: {
      negotiateE2EEPublicKey: ({ mid }: { mid: string }) => {
        requested.push(mid);
        return Promise.resolve({
          specVersion: 2,
          publicKey: { keyId: 1, keyData: Buffer.from(mid) },
        });
      },
    },
    getToType: () => 0,
    log() {},
  } as never);
  assertEquals((await e2ee.getE2EELocalPublicKey("u-a", 1)).toString(), "u-a");
  assertEquals((await e2ee.getE2EELocalPublicKey("u-b", 1)).toString(), "u-b");
  assertEquals((await e2ee.getE2EELocalPublicKey("u-a", 1)).toString(), "u-a");
  assertEquals(requested, ["u-a", "u-b"]);
});

Deno.test("ログインの鍵束から指定IDの鍵を取得し、過去の世代と別に保存する", async () => {
  const saved = new Map<string, string>();
  const e2ee = new E2EE({
    thrift: new Thrift(),
    storage: {
      get: (key: string) => Promise.resolve(saved.get(key)),
      set: (key: string, value: string) => {
        saved.set(key, value);
        return Promise.resolve();
      },
    },
    talk: { getE2EEPublicKeys: () => Promise.resolve([]) },
    log() {},
    emit() {},
  } as never);
  const keys = [10, 20].map((keyId, index) => {
    const privKey = Buffer.alloc(32, index + 1);
    return {
      keyId,
      privKey,
      pubKey: Buffer.from(nacl.scalarMult.base(privKey)),
    };
  });
  const entries: NestedArray[] = keys.map((key) => [
    [8, 1, 1],
    [8, 2, key.keyId],
    [11, 4, key.pubKey],
    [11, 5, key.privKey],
  ]);
  const plain = Buffer.from(
    writeStruct([[15, 1, [12, entries]]], Protocols[4]),
  );
  const secret = Buffer.alloc(32, 99);
  const publicKey = Buffer.from(nacl.scalarMult.base(Buffer.alloc(32, 100)));
  const shared = Buffer.from(e2ee.generateSharedSecret(secret, publicKey));
  const cipher = crypto.createCipheriv(
    "aes-256-cbc",
    e2ee.getSHA256Sum(shared, "Key"),
    e2ee.xor(e2ee.getSHA256Sum(shared, "IV")),
  );
  const data = {
    keyId: 20,
    publicKey: publicKey.toString("base64"),
    encryptedKeyChain: Buffer.concat([cipher.update(plain), cipher.final()])
      .toString("base64"),
    e2eeVersion: 2,
  };
  saved.set(
    "e2eeKeys:u-self",
    JSON.stringify({
      keyId: 20,
      privKey: keys[0].privKey.toString("base64"),
      pubKey: keys[0].pubKey.toString("base64"),
    }),
  );
  assertEquals(
    (await e2ee.decodeE2EEKeyV1(data, secret))?.privKey,
    keys[1].privKey,
  );
  assertEquals(
    JSON.parse(saved.get("e2eeKeys:10")!).privKey,
    keys[0].privKey.toString("base64"),
  );
  assertEquals(
    (await e2ee.getE2EESelfKeyData("u-self")).privKey,
    keys[1].privKey.toString("base64"),
  );
  await assertRejects(
    () => e2ee.decodeE2EEKeyV1({ ...data, keyId: 999 }, secret),
    Error,
    "Requested keyId",
  );
});

Deno.test("復号失敗を明示し、旧版の空本文と失敗行は再取得する", async () => {
  let failures = 0;
  const decrypt = guardedDecrypt(
    () => Promise.reject(new Error("秘密情報を含む例外")),
    () => {
      failures++;
    },
  );
  const raw = { id: "1", contentType: "NONE", chunks: [new Uint8Array(1)] };
  const failed = await decrypt(raw);
  assertEquals(
    (failed.contentMetadata as Record<string, string>)[DECRYPTION_FAILED],
    "1",
  );
  assertEquals(failures, 1);
  assertEquals(raw, {
    id: "1",
    contentType: "NONE",
    chunks: [new Uint8Array(1)],
  });
  const cached = restoreCachedMessages([
    { id: "1", text: "", contentType: "NONE" },
    { id: "2", text: "本文", contentType: "NONE" },
    { id: "3", contentType: "IMAGE" },
  ]);
  assertEquals(cached.map(needsDecryptionRefresh), [true, false, false]);
  const repaired = guardedDecrypt(
    (message) => Promise.resolve({ ...message, text: "復号された本文" }),
    () => {
      failures++;
    },
  );
  assertEquals((await repaired(raw)).text, "復号された本文");
  assertEquals(failures, 1);
});

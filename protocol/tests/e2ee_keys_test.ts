import { assert, assertEquals, assertRejects } from "@std/assert";
import { InternalError } from "@evex/linejs/base";
import { MemoryStorage } from "@evex/linejs/storage";
import { type NestedArray, Protocols } from "@evex/linejs/thrift";
import { Buffer } from "node:buffer";
import crypto from "node:crypto";
import nacl from "npm:tweetnacl@1.0.3";
import { writeStruct } from "../vendor/deps/linejs/base/thrift/readwrite/write.ts";
import { createKeyClient } from "../src/e2ee_keys.ts";
import { patchGroupKeyRecovery } from "../src/group_key_recovery.ts";
import { retryKeyDecryption } from "../src/e2ee_guard.ts";

function fixtureKey(keyId: number, seed: number) {
  const privKey = Buffer.alloc(32, seed);
  const pubKey = Buffer.from(nacl.scalarMult.base(privKey));
  return {
    keyId,
    privKey: privKey.toString("base64"),
    pubKey: pubKey.toString("base64"),
    e2eeVersion: 2,
  };
}
const keys = [fixtureKey(10, 1), fixtureKey(20, 2)];

function loginFixture(storage = new MemoryStorage(), imported = keys) {
  const client = createKeyClient({ device: "ANDROIDSECONDARY", storage });
  const { base } = client;
  const secret = Buffer.alloc(32, 99);
  const publicKey = Buffer.from(nacl.scalarMult.base(Buffer.alloc(32, 100)));
  const shared = Buffer.from(base.e2ee.generateSharedSecret(secret, publicKey));
  const cipher = crypto.createCipheriv(
    "aes-256-cbc",
    base.e2ee.getSHA256Sum(shared, "Key"),
    base.e2ee.xor(base.e2ee.getSHA256Sum(shared, "IV")),
  );
  const entries: NestedArray[] = imported.map((key) => [
    [8, 1, 1],
    [8, 2, key.keyId],
    [11, 4, Buffer.from(key.pubKey, "base64")],
    [11, 5, Buffer.from(key.privKey, "base64")],
  ]);
  const plain = Buffer.from(
    writeStruct([[15, 1, [12, entries]]], Protocols[4]),
  );
  const metadata = {
    keyId: imported.at(-1)!.keyId,
    publicKey: publicKey.toString("base64"),
    encryptedKeyChain: Buffer.concat([cipher.update(plain), cipher.final()])
      .toString("base64"),
    e2eeVersion: 2,
  };
  base.talk.getE2EEPublicKeys = () =>
    Promise.resolve(imported.map((key) => ({
      keyId: key.keyId,
      keyData: Buffer.from(key.pubKey, "base64"),
    })) as never);
  base.talk.getProfile = () => Promise.resolve({ mid: "u-self" } as never);
  let registrations = 0;
  base.talk.registerE2EEPublicKey = () => {
    registrations++;
    return Promise.reject(new Error("must not generate identity keys"));
  };
  const login = base.loginProcess;
  login.createSession = () => Promise.resolve({ 1: "fixture-session" });
  login.createQrCodeForSecure = () =>
    Promise.resolve({
      1: "https://example.invalid/qr",
      2: 1,
      3: 1,
      4: "fixture-nonce",
    });
  base.e2ee.createSqrSecret = () => [secret, "?secret=fixture"];
  login.checkQrCodeVerified = () => Promise.resolve(true);
  login.verifyCertificate = () => Promise.resolve(true);
  const response = {
    3: { 1: "fixture-token", 2: "fixture-refresh", 3: 0, 6: 1 },
    6: { e2eeInfo: JSON.stringify(metadata) },
  };
  login.qrCodeLoginV2ForSecure = () => Promise.resolve(response);
  return {
    ...client,
    storage,
    secret,
    metadata,
    response,
    registrations: () => registrations,
    async login() {
      await login.withQrCode({});
      await login.ready();
      await client.bindAccount();
    },
  };
}

for (const encoding of ["json", "object"] as const) {
  Deno.test(`QR ${encoding} keychain imports all generations and replaces a stale account alias`, async () => {
    const f = loginFixture();
    await f.storage.set("e2eeKeys:u-self", JSON.stringify(keys[0]));
    if (encoding === "object") {
      f.base.loginProcess.qrCodeLoginV2ForSecure = () =>
        Promise.resolve({
          ...f.response,
          10: f.metadata,
        });
    }
    await f.login();
    assertEquals(f.registrations(), 0);
    assertEquals((await f.base.e2ee.getE2EESelfKeyData("u-self")).keyId, 20);
    assertEquals(
      JSON.parse(String(await f.storage.get("e2eeKeys:u-self"))).keyId,
      20,
    );
    for (const key of keys) {
      assertEquals(await f.base.e2ee.getE2EESelfKeyDataByKeyId(key.keyId), key);
      assertEquals(await f.storage.get(`e2eeKeys:${key.keyId}`), undefined);
      assert(await f.storage.get(`lineGtk:u-self:e2eeKeys:${key.keyId}`));
    }
    const resumed = createKeyClient({
      device: "ANDROIDSECONDARY",
      storage: f.storage,
    });
    resumed.base.profile = f.base.profile;
    resumed.base.talk.getE2EEPublicKeys = f.base.talk.getE2EEPublicKeys;
    await resumed.bindAccount();
    assertEquals(
      (await resumed.base.e2ee.getE2EESelfKeyData("u-self")).keyId,
      20,
    );
    assertEquals(
      await resumed.base.e2ee.getE2EESelfKeyDataByKeyId(10),
      keys[0],
    );
  });
}

for (
  const metadata of [
    undefined,
    "invalid-json",
    "{}",
    JSON.stringify({
      keyId: 20,
      encryptedKeyChain: "invalid",
      publicKey: "invalid",
    }),
  ]
) {
  Deno.test(`QR rejects ${String(metadata).slice(0, 20)} without replacing identity keys`, async () => {
    const f = loginFixture();
    await f.storage.set("e2eeKeys:u-self", JSON.stringify(keys[0]));
    f.base.loginProcess.qrCodeLoginV2ForSecure = () =>
      Promise.resolve({
        ...f.response,
        6: { e2eeInfo: metadata },
      });
    const error = await assertRejects(() => f.login(), InternalError);
    assertEquals(error.name, "NoE2EEKey");
    assert(!error.message.includes("invalid-json"));
    assertEquals(f.registrations(), 0);
    assertEquals(
      JSON.parse(String(await f.storage.get("e2eeKeys:u-self"))),
      keys[0],
    );
    assertEquals(await f.storage.get("lineGtk:u-self:e2eeKeys:20"), undefined);
  });
}

Deno.test("a valid imported keychain must match the authenticated account before persistence", async () => {
  const f = loginFixture();
  await f.base.e2ee.decodeE2EEKeyV1(f.metadata, f.secret);
  f.base.profile = { mid: "u-other" } as never;
  f.base.talk.getE2EEPublicKeys = () =>
    Promise.resolve([{
      keyId: 20,
      keyData: Buffer.from(fixtureKey(20, 3).pubKey, "base64"),
    }] as never);
  await assertRejects(() => f.bindAccount(), InternalError, "does not match");
  assertEquals(await f.storage.get("lineGtk:u-other:e2eeKeys:20"), undefined);
  assertEquals(await f.storage.get("e2eeKeys:u-other"), undefined);
});

Deno.test("raw numeric Thrift public-key fields verify a QR import and legacy migration", async () => {
  const f = loginFixture();
  f.base.talk.getE2EEPublicKeys = () =>
    Promise.resolve(keys.map((key) => ({
      2: key.keyId,
      4: Buffer.from(key.pubKey, "base64"),
    })) as never);
  await f.login();
  assertEquals((await f.base.e2ee.getE2EESelfKeyData("u-self")).keyId, 20);
  const legacy = loginFixture();
  legacy.base.talk.getE2EEPublicKeys = f.base.talk.getE2EEPublicKeys;
  await legacy.storage.set("e2eeKeys:20", JSON.stringify(keys[1]));
  legacy.base.profile = f.base.profile;
  await legacy.bindAccount();
  assertEquals((await legacy.base.e2ee.getE2EESelfKeyData("u-self")).keyId, 20);
});

Deno.test("a mislabelled legacy key cannot pass verification or overwrite the account alias", async () => {
  const f = loginFixture();
  const wrong = { ...keys[0], keyId: 20 };
  await f.storage.set("e2eeKeys:20", JSON.stringify(wrong));
  await f.storage.set("e2eeKeys:u-self", JSON.stringify(wrong));
  f.base.profile = { mid: "u-self" } as never;
  await f.bindAccount();
  await assertRejects(
    () => f.base.e2ee.getE2EESelfKeyData("u-self"),
    InternalError,
  );
  assertEquals(await f.storage.get("lineGtk:u-self:e2eeKeys:20"), undefined);
  assertEquals(
    JSON.parse(String(await f.storage.get("e2eeKeys:u-self"))),
    wrong,
  );
});

Deno.test("legacy token sessions select the newest verified local generation instead of an old alias", async () => {
  const f = loginFixture();
  await f.storage.set("e2eeKeys:u-self", JSON.stringify(keys[0]));
  for (const key of keys) {
    await f.storage.set(`e2eeKeys:${key.keyId}`, JSON.stringify(key));
  }
  f.base.profile = { mid: "u-self" } as never;
  await f.bindAccount();
  assertEquals((await f.base.e2ee.getE2EESelfKeyData("u-self")).keyId, 20);
  assertEquals(
    JSON.parse(String(await f.storage.get("e2eeKeys:u-self"))).keyId,
    20,
  );
});

Deno.test("fresh login without a key import requires usable existing account keys", async () => {
  const f = loginFixture();
  f.base.profile = { mid: "u-self" } as never;
  await assertRejects(() => f.bindAccount(true), InternalError);
  await f.storage.set("e2eeKeys:20", JSON.stringify(keys[1]));
  await f.bindAccount(true);
  assertEquals((await f.base.e2ee.getE2EESelfKeyData("u-self")).keyId, 20);
});

Deno.test("accounts with the same key ID cannot consume each other's scoped or legacy private keys", async () => {
  const storage = new MemoryStorage();
  const a = loginFixture(storage, [fixtureKey(10, 1)]);
  await a.login();
  await storage.set("e2eeKeys:10", JSON.stringify(fixtureKey(10, 1)));
  const b = loginFixture(storage, [fixtureKey(10, 2)]);
  b.base.profile = { mid: "u-other" } as never;
  await b.bindAccount();
  await assertRejects(
    () => b.base.e2ee.getE2EESelfKeyDataByKeyId(10),
    InternalError,
  );
  await assertRejects(
    () => a.base.e2ee.getE2EESelfKeyData("u-other"),
    InternalError,
  );
  const importedB = loginFixture(storage, [fixtureKey(10, 2)]);
  importedB.base.talk.getProfile = () =>
    Promise.resolve({ mid: "u-other" } as never);
  await importedB.login();
  assertEquals(
    (await a.base.e2ee.getE2EESelfKeyDataByKeyId(10)).privKey,
    fixtureKey(10, 1).privKey,
  );
  assertEquals(
    (await importedB.base.e2ee.getE2EESelfKeyDataByKeyId(10)).privKey,
    fixtureKey(10, 2).privKey,
  );
});

Deno.test("temporary public-key lookup failure preserves storage and allows another selection", async () => {
  const f = loginFixture();
  await f.storage.set("e2eeKeys:20", JSON.stringify(keys[1]));
  f.base.profile = { mid: "u-self" } as never;
  await f.bindAccount();
  const get = f.base.talk.getE2EEPublicKeys;
  f.base.talk.getE2EEPublicKeys = () =>
    Promise.reject(new DOMException("offline", "NetworkError"));
  await assertRejects(() => f.base.e2ee.getE2EESelfKeyData("u-self"));
  assert(await f.storage.get("e2eeKeys:20"));
  f.base.talk.getE2EEPublicKeys = get;
  assertEquals((await f.base.e2ee.getE2EESelfKeyData("u-self")).keyId, 20);
});

Deno.test("a rotated sender key is fetched by exact ID and cached for older group messages", async () => {
  const f = loginFixture();
  f.base.talk.negotiateE2EEPublicKey = () =>
    Promise.resolve({
      specVersion: 2,
      publicKey: { keyId: 20, keyData: Buffer.from(keys[1].pubKey, "base64") },
    } as never);
  let fetches = 0;
  f.base.talk.getE2EEPublicKey = (options) => {
    fetches++;
    assertEquals(options, { mid: "u-sender", keyId: 10, keyVersion: 1 });
    return Promise.resolve(
      { keyId: 10, keyData: Buffer.from(keys[0].pubKey, "base64") } as never,
    );
  };
  const first = await f.base.e2ee.getE2EELocalPublicKey("u-sender", 10);
  const second = await f.base.e2ee.getE2EELocalPublicKey("u-sender", 10);
  assertEquals(first, Buffer.from(keys[0].pubKey, "base64"));
  assertEquals(first, second);
  assertEquals(fetches, 1);
});

Deno.test("imported historical self public keys remain usable without server negotiation", async () => {
  const f = loginFixture();
  await f.login();
  f.base.talk.negotiateE2EEPublicKey = () =>
    Promise.reject(new Error("unexpected negotiation"));
  assertEquals(
    await f.base.e2ee.getE2EELocalPublicKey("u-self", 10),
    Buffer.from(keys[0].pubKey, "base64"),
  );
});

Deno.test("QR-imported current keys recreate a group key after membership change", async () => {
  const f = loginFixture();
  await f.login();
  const publicKey = Buffer.from(keys[1].pubKey, "base64");
  f.base.talk.getLastE2EEGroupSharedKey = () =>
    Promise.reject(new InternalError("TalkException", "rotate", { code: 99 }));
  f.base.talk.getLastE2EEPublicKeys = () =>
    Promise.resolve({
      "u-self": { keyId: 20, keyData: publicKey },
      "u-new-member": {
        keyId: 10,
        keyData: Buffer.from(keys[0].pubKey, "base64"),
      },
    } as never);
  f.base.talk.getChats = () =>
    Promise.resolve({
      chats: [{
        chatMid: "c-group",
        extra: {
          groupExtra: { memberMids: { "u-self": 0, "u-new-member": 0 } },
        },
      }],
    } as never);
  f.base.talk.negotiateE2EEPublicKey = () =>
    Promise.resolve({
      specVersion: 2,
      publicKey: { keyId: 20, keyData: publicKey },
    } as never);
  f.base.talk.registerE2EEGroupKey = (options) => {
    assertEquals(options?.members, ["u-self", "u-new-member"]);
    assertEquals(options?.keyIds, [20, 10]);
    return Promise.resolve({
      groupKeyId: 30,
      creator: "u-self",
      creatorKeyId: 20,
      receiverKeyId: 20,
      encryptedSharedKey: options!.encryptedSharedKeys![0],
    } as never);
  };
  patchGroupKeyRecovery(f.base);
  const chunks = await f.base.e2ee.encryptE2EEMessage("c-group", "after join");
  assertEquals(chunks[3].readInt32BE(), 20);
  assertEquals(chunks[4].readInt32BE(), 30);
  assertEquals(
    (await f.base.e2ee.decryptE2EETextMessage({
      to: "c-group",
      from: "u-self",
      toType: "GROUP",
      contentType: "NONE",
      contentMetadata: { e2eeVersion: "2" },
      chunks,
    } as never))[0],
    "after join",
  );
  assertEquals(f.registrations(), 0);
});

Deno.test("a pending group generation is refetched instead of decrypting with the previous generation", async () => {
  const f = loginFixture();
  await f.login();
  const self = keys[1];
  const selfPrivate = Buffer.from(self.privKey, "base64");
  const selfPublic = Buffer.from(self.pubKey, "base64");
  const groupKeys = [Buffer.alloc(32, 3), Buffer.alloc(32, 4)];
  function wrapped(generation: number) {
    const shared = Buffer.from(
      f.base.e2ee.generateSharedSecret(selfPrivate, selfPublic),
    );
    const cipher = crypto.createCipheriv(
      "aes-256-cbc",
      f.base.e2ee.getSHA256Sum(shared, "Key"),
      f.base.e2ee.xor(f.base.e2ee.getSHA256Sum(shared, "IV")),
    );
    return {
      groupKeyId: generation + 30,
      creator: "u-self",
      creatorKeyId: 20,
      receiverKeyId: 20,
      encryptedSharedKey: Buffer.concat([
        cipher.update(groupKeys[generation]),
        cipher.final(),
      ]),
    };
  }
  let reads = 0;
  let lastReads = 0;
  f.base.talk.getE2EEGroupSharedKey = (options) => {
    assertEquals(options?.groupKeyId, 31);
    if (++reads === 1) {
      return Promise.reject(
        new InternalError("TalkException", "not ready", { code: "NOT_FOUND" }),
      );
    }
    return Promise.resolve(wrapped(1) as never);
  };
  f.base.talk.getLastE2EEGroupSharedKey = () => {
    lastReads++;
    return Promise.resolve(wrapped(0) as never);
  };
  f.base.talk.negotiateE2EEPublicKey = () =>
    Promise.resolve({
      specVersion: 2,
      publicKey: { keyId: 20, keyData: selfPublic },
    } as never);
  function message(generation: number, text: string) {
    const keyData = Buffer.from(
      f.base.e2ee.generateSharedSecret(groupKeys[generation], selfPublic),
    );
    return {
      from: "u-self",
      to: "c-group",
      toType: "GROUP",
      contentType: "NONE",
      contentMetadata: { e2eeVersion: "2" },
      chunks: f.base.e2ee.encryptE2EETextMessage(
        20,
        generation + 30,
        keyData,
        2,
        text,
        "c-group",
        "u-self",
      ),
    };
  }
  const decrypt = retryKeyDecryption(
    (raw) => f.base.e2ee.decryptE2EEMessage(raw as never) as never,
    () => true,
    () => Promise.resolve(),
  );
  assertEquals((await decrypt(message(1, "after join"))).text, "after join");
  assertEquals(reads, 2);
  assertEquals(lastReads, 1);
  assertEquals((await decrypt(message(0, "before join"))).text, "before join");
  assertEquals(f.registrations(), 0);
});

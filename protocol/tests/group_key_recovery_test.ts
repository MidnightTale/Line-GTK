import { assertEquals, assertRejects } from "@std/assert";
import { BaseClient, InternalError } from "@evex/linejs/base";
import { MemoryStorage } from "@evex/linejs/storage";
import { Buffer } from "node:buffer";
import crypto from "node:crypto";
import nacl from "npm:tweetnacl@1.0.3";
import {
  patchGroupKeyRecovery,
  recreateGroupKey,
} from "../src/group_key_recovery.ts";

function rotationError(code: string | number = "E2EE_RECREATE_GROUP_KEY") {
  return new InternalError("TalkException", "group membership changed", {
    code,
  });
}

async function groupFixture() {
  const base = new BaseClient({
    device: "ANDROIDSECONDARY",
    storage: new MemoryStorage(),
  });
  const mid = "u-self";
  const chatMid = "c-group";
  base.profile = { mid } as NonNullable<typeof base.profile>;
  const privKey = Buffer.alloc(32, 1);
  const pubKey = Buffer.from(nacl.scalarMult.base(privKey));
  const self = {
    keyId: 10,
    privKey: privKey.toString("base64"),
    pubKey: pubKey.toString("base64"),
  };
  await base.e2ee.saveE2EESelfKeyData(self);
  await base.e2ee.saveE2EESelfKeyDataByKeyId(10, self);
  const otherPublicKey = Buffer.from(nacl.scalarMult.base(Buffer.alloc(32, 2)));
  const publicKeys: Record<string, { keyId: number; keyData: Buffer }> = {
    [mid]: { keyId: 10, keyData: pubKey },
    "u-new-member": { keyId: 11, keyData: otherPublicKey },
  };
  base.talk.negotiateE2EEPublicKey = (options) =>
    Promise.resolve({
      specVersion: 2,
      publicKey: publicKeys[options!.mid!],
    } as never);
  base.talk.getLastE2EEPublicKeys = () => Promise.resolve(publicKeys as never);
  let members = [mid, "u-new-member"];
  base.talk.getChats = () =>
    Promise.resolve({
      chats: [{
        chatMid,
        extra: {
          groupExtra: {
            memberMids: Object.fromEntries(members.map((m) => [m, 0])),
          },
        },
      }],
    } as never);
  const shared = base.e2ee.generateSharedSecret(privKey, pubKey);
  const cipher = crypto.createCipheriv(
    "aes-256-cbc",
    base.e2ee.getSHA256Sum(Buffer.from(shared), "Key"),
    base.e2ee.xor(base.e2ee.getSHA256Sum(Buffer.from(shared), "IV")),
  );
  let wrapped = {
    groupKeyId: 20,
    creator: mid,
    creatorKeyId: 10,
    receiverKeyId: 10,
    encryptedSharedKey: Buffer.concat([
      cipher.update(Buffer.alloc(32, 3)),
      cipher.final(),
    ]),
  };
  let rejection: Error | null = null;
  let registrations = 0;
  base.talk.getLastE2EEGroupSharedKey = () =>
    rejection ? Promise.reject(rejection) : Promise.resolve(wrapped as never);
  base.talk.registerE2EEGroupKey = (options) => {
    registrations++;
    assertEquals(options?.members, [mid, "u-new-member"]);
    assertEquals(options?.keyIds, [10, 11]);
    wrapped = {
      ...wrapped,
      groupKeyId: wrapped.groupKeyId + 1,
      encryptedSharedKey: Buffer.from(
        options!.encryptedSharedKeys![0] as Buffer,
      ),
    };
    rejection = null;
    return Promise.resolve(wrapped as never);
  };
  const encrypt = base.e2ee.encryptE2EEMessage.bind(base.e2ee);
  const envelopes: Buffer[][] = [];
  base.e2ee.encryptE2EEMessage = async (...args) => {
    const chunks = await encrypt(...args);
    envelopes.push(chunks);
    return chunks;
  };
  patchGroupKeyRecovery(base);
  return {
    base,
    mid,
    chatMid,
    self,
    envelopes,
    registrations: () => registrations,
    setMembers: (next: string[]) => {
      members = next;
    },
    // Another member's client registers a newer generation first.
    rotateElsewhere: () => {
      wrapped = { ...wrapped, groupKeyId: wrapped.groupKeyId + 1 };
      rejection = null;
    },
    rejectLookup: (error: Error) => {
      rejection = error;
    },
    decrypt: async (chunks: Buffer[]) =>
      (await base.e2ee.decryptE2EETextMessage({
        from: mid,
        to: chatMid,
        toType: "GROUP",
        contentType: "NONE",
        contentMetadata: { e2eeVersion: "2" },
        chunks,
      } as never))[0],
  };
}

for (const code of [99, "99", "E2EE_RECREATE_GROUP_KEY"]) {
  Deno.test(`membership rejection ${code} repairs encryption and keeps old history`, async () => {
    const f = await groupFixture();
    const old = await f.base.e2ee.encryptE2EEMessage(f.chatMid, "before join");
    f.rejectLookup(rotationError(code));
    const fresh = await f.base.e2ee.encryptE2EEMessage(f.chatMid, "after join");
    assertEquals(f.registrations(), 1);
    assertEquals(fresh[4].readInt32BE(), 21);
    assertEquals(await f.decrypt(fresh), "after join");
    assertEquals(await f.decrypt(old), "before join");
    assertEquals(await f.base.e2ee.getE2EESelfKeyData(f.mid), f.self);
  });
}

Deno.test("membership change during text send rebuilds ciphertext once", async () => {
  const f = await groupFixture();
  let requests = 0;
  f.base.request.request = (() => {
    if (++requests === 1) return Promise.reject(rotationError());
    return Promise.resolve({ id: "sent" });
  }) as typeof f.base.request.request;
  await f.base.talk.sendMessage({
    to: f.chatMid,
    text: "after join",
    relatedMessageId: "42",
    contentMetadata: { MENTION: "mentions" },
    e2ee: true,
  });
  assertEquals(requests, 2);
  assertEquals(f.registrations(), 1);
  assertEquals(f.envelopes.map((chunks) => chunks[4].readInt32BE()), [20, 21]);
  assertEquals(await f.decrypt(f.envelopes[1]), "after join");
});

Deno.test("repeated text rejection stops after one recreation even if patched twice", async () => {
  const f = await groupFixture();
  patchGroupKeyRecovery(f.base);
  let requests = 0;
  f.base.request.request = (() => {
    requests++;
    return Promise.reject(rotationError());
  }) as typeof f.base.request.request;
  await assertRejects(() =>
    f.base.talk.sendMessage({ to: f.chatMid, text: "text", e2ee: true })
  );
  assertEquals(requests, 2);
  assertEquals(f.registrations(), 1);
});

Deno.test("linejs implicit encryption fallback cannot repeat an inner repair", async () => {
  const f = await groupFixture();
  let requests = 0;
  f.base.request.request = (() => {
    requests++;
    return Promise.reject(rotationError());
  }) as typeof f.base.request.request;
  await assertRejects(() =>
    f.base.talk.sendMessage({ to: f.chatMid, text: "text" })
  );
  assertEquals(requests, 3); // plain, encrypted, refreshed encrypted
  assertEquals(f.registrations(), 1);
});

for (
  const error of [
    new DOMException("timeout", "TimeoutError"),
    new InternalError("TalkException", "invalid protocol", {
      code: "E2EE_INVALID_PROTOCOL",
    }),
  ]
) {
  Deno.test(`${error.message} never recreates keys or replays text`, async () => {
    const f = await groupFixture();
    let requests = 0;
    f.base.request.request = (() => {
      requests++;
      return Promise.reject(error);
    }) as typeof f.base.request.request;
    assertEquals(
      await assertRejects(() =>
        f.base.talk.sendMessage({ to: f.chatMid, text: "text", e2ee: true })
      ),
      error,
    );
    assertEquals(requests, 1);
    f.rejectLookup(error);
    await assertRejects(() =>
      f.base.e2ee.encryptE2EEMessage(f.chatMid, "text")
    );
    assertEquals(f.registrations(), 0);
  });
}

Deno.test("prebuilt media chunks are returned to their caller for rebuilding", async () => {
  const f = await groupFixture();
  f.base.request.request =
    (() => Promise.reject(rotationError())) as typeof f.base.request.request;
  await assertRejects(() =>
    f.base.talk.sendMessage({
      to: f.chatMid,
      chunks: [Buffer.from("prebuilt")],
      e2ee: true,
    })
  );
  assertEquals(f.registrations(), 0);
});

Deno.test("concurrent group repairs share registration only within the same client", async () => {
  const f = await groupFixture();
  const other = await groupFixture();
  await Promise.all([
    recreateGroupKey(f.base, f.chatMid),
    recreateGroupKey(f.base, f.chatMid),
    recreateGroupKey(other.base, f.chatMid),
  ]);
  assertEquals(f.registrations(), 1);
  assertEquals(other.registrations(), 1);
});

Deno.test("failed registration surfaces the error and allows a later repair", async () => {
  const f = await groupFixture();
  const register = f.base.e2ee.tryRegisterE2EEGroupKey.bind(f.base.e2ee);
  f.base.e2ee.tryRegisterE2EEGroupKey = () =>
    Promise.reject(new Error("registration unavailable"));
  f.rejectLookup(rotationError());
  await assertRejects(
    () => f.base.e2ee.encryptE2EEMessage(f.chatMid, "text"),
    Error,
    "registration unavailable",
  );
  f.base.e2ee.tryRegisterE2EEGroupKey = register;
  await f.base.e2ee.encryptE2EEMessage(f.chatMid, "text");
  assertEquals(f.registrations(), 1);
});

Deno.test("a key another member already registered is reused instead of registering again", async () => {
  const f = await groupFixture();
  let requests = 0;
  f.base.request.request = (() => {
    if (++requests === 1) {
      f.rotateElsewhere();
      return Promise.reject(rotationError());
    }
    return Promise.resolve({ id: "sent" });
  }) as typeof f.base.request.request;
  await f.base.talk.sendMessage({ to: f.chatMid, text: "hi", e2ee: true });
  assertEquals(requests, 2);
  assertEquals(f.registrations(), 0);
  assertEquals(f.envelopes.map((chunks) => chunks[4].readInt32BE()), [20, 21]);
});

Deno.test("a group with a member lacking an encryption key is not given a partial key", async () => {
  const f = await groupFixture();
  f.setMembers([f.mid, "u-new-member", "u-no-key"]);
  f.rejectLookup(rotationError());
  const error = await assertRejects(
    () => f.base.e2ee.encryptE2EEMessage(f.chatMid, "text"),
    InternalError,
  );
  assertEquals(error.data.code, "E2EE_MEMBER_KEYS_MISSING");
  assertEquals(f.registrations(), 0);
});

import { BaseClient, InternalError } from "@evex/linejs/base";
import {
  type BaseStorage,
  MemoryStorage,
  type Storage,
} from "@evex/linejs/storage";
import { Buffer } from "node:buffer";

type KeyData = {
  keyId: number | string;
  privKey: string;
  pubKey: string;
};

function publicKeyOf(
  raw: unknown,
): { keyId: number; keyData: Buffer } | undefined {
  if (!raw || typeof raw !== "object") return;
  const fields = raw as Record<string, unknown>;
  const id = Number(fields.keyId ?? fields[2]);
  const data = fields.keyData ?? fields[4];
  if (
    !Number.isSafeInteger(id) || id < 0 ||
    !(typeof data === "string" || data instanceof Uint8Array)
  ) return;
  const bytes = typeof data === "string"
    ? Buffer.from(data)
    : Buffer.from(data);
  if (bytes.length === 32) return { keyId: id, keyData: bytes };
}

function missingKey(message = "Import encryption keys from the phone again") {
  return new InternalError("NoE2EEKey", message);
}

// Login imports arrive before the account MID. Stage them in memory until the
// authenticated profile is known; numeric key IDs alone are not account IDs.
export function createKeyClient(
  init: ConstructorParameters<typeof BaseClient>[0],
) {
  const original = init.storage ?? new MemoryStorage();
  const staged = new Map<string, Storage["Value"]>();
  let account: string | undefined;
  let importedId: string | undefined;
  const ownKey = (key: string) => /^e2eeKeys:\d+$/.test(key);
  const scoped = (key: string) => `lineGtk:${account}:${key}`;
  const storage: BaseStorage = {
    get: (key) =>
      ownKey(key)
        ? account ? original.get(scoped(key)) : Promise.resolve(staged.get(key))
        : original.get(key),
    set: async (key, value) => {
      if (!ownKey(key)) return await original.set(key, value);
      if (account) await original.set(scoped(key), value);
      else staged.set(key, value);
    },
    delete: async (key) => {
      if (ownKey(key)) {
        staged.delete(key);
        if (account) await original.delete(scoped(key));
      } else await original.delete(key);
    },
    clear: async () => {
      staged.clear();
      importedId = undefined;
      await original.clear();
    },
    migrate: (target) => original.migrate(target),
  };
  const base = new BaseClient({ ...init, storage });
  const e2ee = base.e2ee;
  const decode = e2ee.decodeE2EEKeyV1.bind(e2ee);
  e2ee.decodeE2EEKeyV1 = async (raw, secret) => {
    try {
      const data: unknown = typeof raw === "string" ? JSON.parse(raw) : raw;
      if (!data || typeof data !== "object") throw missingKey();
      const fields = data as Record<string, unknown>;
      if (
        !/^\d+$/.test(String(fields.keyId)) ||
        typeof fields.publicKey !== "string" || !fields.publicKey ||
        typeof fields.encryptedKeyChain !== "string" ||
        !fields.encryptedKeyChain
      ) throw missingKey();
      const result = await decode(fields, secret);
      if (!result) throw missingKey();
      importedId = String(result.keyId);
      return result;
    } catch {
      // Neither malformed metadata nor an invalid keychain permits generating
      // a new identity key. Do not include metadata or keys in errors/logs.
      staged.clear();
      importedId = undefined;
      throw missingKey("The login encryption keychain is missing or invalid");
    }
  };
  e2ee.registerE2EEKeyPair = () =>
    Promise.reject(missingKey("The login did not import encryption keys"));

  const getPublicKey = e2ee.getE2EELocalPublicKey.bind(e2ee);
  e2ee.getE2EELocalPublicKey = async (mid, keyId) => {
    if (account === mid && keyId !== undefined) {
      const own = readKey(
        await storage.get(`e2eeKeys:${keyId}`),
        String(keyId),
      );
      if (own) return Buffer.from(own.pubKey, "base64");
    }
    try {
      const key = await getPublicKey(mid, keyId);
      if (
        /^[cr]/.test(mid) && keyId !== undefined &&
        (!("keyId" in key) || Number(key.keyId) !== Number(keyId))
      ) {
        // The library can fall back to the last group key when this generation
        // is not yet distributed. That key cannot decrypt the requested message.
        throw missingKey(
          "The requested group key generation is not available yet",
        );
      }
      return key;
    } catch (error) {
      if (
        !mid.startsWith("u") || keyId === undefined ||
        !(error instanceof InternalError) || error.name !== "No E2EEKey"
      ) throw error;
      // Negotiation only returns the latest key. An older message/group-key
      // creator must be resolved by its envelope's exact public-key ID.
      const id = Number(keyId);
      if (!Number.isSafeInteger(id) || id < 0) throw error;
      const key = publicKeyOf(
        await base.talk.getE2EEPublicKey({
          mid,
          keyId: id,
          // Identity keys are registered as version 1, independently of the
          // version 2 message envelope.
          keyVersion: 1,
        }),
      );
      if (!key || key.keyId !== id) throw error;
      const bytes = key.keyData;
      await storage.set(
        `e2eePublicKeys:${mid}:${id}`,
        bytes.toString("base64"),
      );
      return bytes;
    }
  };

  function readKey(raw: unknown, id: string): KeyData | undefined {
    try {
      const value = typeof raw === "string" ? JSON.parse(raw) : raw;
      if (!value || String(value.keyId) !== id) return;
      const priv = Buffer.from(value.privKey, "base64");
      const pub = Buffer.from(value.pubKey, "base64");
      if (
        priv.length === 32 && pub.length === 32 &&
        e2ee.verifyE2EEKeyPair(priv, pub)
      ) {
        return value;
      }
    } catch { /* Invalid persisted data is not a usable key. */ }
  }

  let registered:
    | Promise<NonNullable<ReturnType<typeof publicKeyOf>>[]>
    | undefined;
  function serverKeys() {
    // getE2EEPublicKeys deliberately returns raw Thrift fields (2 and 4).
    // Other wrappers return named fields. Accept both at this boundary.
    registered ??= base.talk.getE2EEPublicKeys().then((keys) =>
      keys.map(publicKeyOf).filter((key) => key !== undefined)
    ).catch((error) => {
      registered = undefined;
      throw error;
    });
    return registered;
  }

  e2ee.getE2EESelfKeyDataByKeyId = async (keyId) => {
    const id = String(keyId);
    if (!account || base.profile?.mid !== account || !/^\d+$/.test(id)) {
      throw missingKey();
    }
    const slot = `e2eeKeys:${id}`;
    const saved = readKey(await storage.get(slot), id);
    if (saved) return saved;
    // Migrate an old unscoped key only after matching this account's server
    // public key. A different account can have exactly the same numeric ID.
    const publicKey = (await serverKeys()).find((key) =>
      String(key.keyId) === id
    );
    for (const legacy of [slot, `e2eeKeys:${account}`]) {
      const candidate = readKey(await original.get(legacy), id);
      if (
        candidate && publicKey?.keyData &&
        Buffer.from(candidate.pubKey, "base64").equals(
          Buffer.from(publicKey.keyData),
        )
      ) {
        await storage.set(slot, JSON.stringify(candidate));
        return candidate;
      }
    }
    throw missingKey();
  };

  let selected: Promise<KeyData> | undefined;
  e2ee.getE2EESelfKeyData = (mid) => {
    if (!account || mid !== account || base.profile?.mid !== account) {
      return Promise.reject(missingKey());
    }
    selected ??= (async () => {
      const keys = [...await serverKeys()].sort((a, b) =>
        Number(b.keyId) - Number(a.keyId)
      );
      if (importedId) {
        keys.sort((a, b) =>
          Number(String(b.keyId) === importedId) -
          Number(String(a.keyId) === importedId)
        );
      }
      for (const publicKey of keys) {
        let candidate: KeyData;
        try {
          candidate = await e2ee.getE2EESelfKeyDataByKeyId(
            publicKey.keyId,
          ) as KeyData;
        } catch (error) {
          if (error instanceof InternalError && error.name === "NoE2EEKey") {
            continue;
          }
          throw error;
        }
        if (
          publicKey.keyData &&
          Buffer.from(candidate.pubKey, "base64").equals(
            Buffer.from(publicKey.keyData),
          )
        ) {
          await e2ee.saveE2EESelfKeyData(candidate);
          return candidate;
        }
      }
      throw missingKey();
    })().catch((error) => {
      selected = undefined;
      registered = undefined;
      throw error;
    });
    return selected;
  };

  return {
    base,
    async bindAccount(requireKeys = false) {
      const mid = base.profile?.mid;
      if (!mid || (account && account !== mid)) throw missingKey();
      if (importedId) {
        const imported = readKey(
          staged.get(`e2eeKeys:${importedId}`) ??
            await original.get(`lineGtk:${mid}:e2eeKeys:${importedId}`),
          importedId,
        );
        const publicKey = (await serverKeys()).find((key) =>
          String(key.keyId) === importedId
        );
        if (
          !imported || !publicKey?.keyData ||
          !Buffer.from(imported.pubKey, "base64").equals(
            Buffer.from(publicKey.keyData),
          )
        ) {
          throw missingKey(
            "The imported encryption key does not match this account",
          );
        }
      }
      account = mid;
      for (const [key, value] of staged) await storage.set(key, value);
      staged.clear();
      if (importedId || requireKeys) {
        // Refresh the account alias immediately after key import, before any
        // message can use an older cached generation.
        await e2ee.getE2EESelfKeyData(mid);
      }
    },
  };
}

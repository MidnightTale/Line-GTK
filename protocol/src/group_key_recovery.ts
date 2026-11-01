import { type BaseClient, InternalError } from "@evex/linejs/base";

type SharedKey = Awaited<
  ReturnType<BaseClient["e2ee"]["tryRegisterE2EEGroupKey"]>
>;
const registrations = new WeakMap<
  BaseClient,
  Map<string, Promise<SharedKey>>
>();
// The unpatched lookup, and the generation each chat last encrypted under.
// Together they tell a stale rejection from one that still needs a new key.
const lookups = new WeakMap<
  BaseClient,
  (options: { keyVersion: number; chatMid: string }) => Promise<SharedKey>
>();
const usedKeyIds = new WeakMap<BaseClient, Map<string, number>>();
const patched = new WeakSet<BaseClient>();

export function needsGroupKeyRecreation(error: unknown): boolean {
  return error instanceof InternalError &&
    ["99", "E2EE_RECREATE_GROUP_KEY"].includes(String(error.data.code));
}

// Only identifiers are logged: never key material, ciphertext or message text.
function logGroupKey(event: string, fields: Record<string, unknown>) {
  console.error(`[group-key] ${event} ${JSON.stringify(fields)}`);
}

function keyFields(key: SharedKey | undefined) {
  return key
    ? {
      groupKeyId: key.groupKeyId,
      creatorKeyId: key.creatorKeyId,
      receiverKeyId: key.receiverKeyId,
    }
    : undefined;
}

function rememberKey(base: BaseClient, chatMid: string, key: SharedKey) {
  const id = Number(key.groupKeyId);
  if (!Number.isFinite(id)) return;
  let used = usedKeyIds.get(base);
  if (!used) {
    used = new Map();
    usedKeyIds.set(base, used);
  }
  used.set(chatMid, id);
}

async function latestKey(base: BaseClient, chatMid: string) {
  const lookup = lookups.get(base) ??
    base.talk.getLastE2EEGroupSharedKey.bind(base.talk);
  try {
    return await lookup({ keyVersion: 2, chatMid });
  } catch (error) {
    // A rejected lookup means there is no usable key yet, which is the very
    // case that is repaired below. Anything else is a real failure.
    if (needsGroupKeyRecreation(error)) return undefined;
    throw error;
  }
}

// A key that skips a member leaves that member unable to read the group, so
// refuse to register one instead of creating a split the member cannot fix.
async function assertEveryMemberHasKey(base: BaseClient, chatMid: string) {
  const [chats, publicKeys] = await Promise.all([
    base.talk.getChats({
      chatMids: [chatMid],
      withMembers: true,
      withInvitees: false,
    }),
    base.talk.getLastE2EEPublicKeys({ chatMid }),
  ]);
  const chat = chats.chats?.find((row) => row.chatMid === chatMid);
  if (!chat) {
    throw new InternalError(
      "E2EEGroupKeyUnavailable",
      "The group could not be read to verify its members",
      { code: "E2EE_GROUP_UNAVAILABLE" },
    );
  }
  const members = Object.keys(chat.extra?.groupExtra?.memberMids ?? {})
    .filter((mid) => mid.startsWith("u"));
  const missing = members.filter((mid) => !publicKeys[mid]);
  const self = base.profile?.mid;
  if (missing.length || (self && !publicKeys[self])) {
    logGroupKey("members-without-key", {
      chat: chatMid.slice(-6),
      members: members.length,
      withKey: Object.keys(publicKeys).length,
      missing: missing.length,
    });
    throw new InternalError(
      "E2EEGroupKeyUnavailable",
      "Some group members have no encryption key yet, so a new group key was not created",
      { code: "E2EE_MEMBER_KEYS_MISSING" },
    );
  }
}

// Share concurrent repairs within this session only. Keep historical group keys
// and the account's own keypair: a membership change only rotates the group key.
// Another member's client may already have rotated the key, so look first and
// register only if the server still serves the generation that was rejected.
export async function recreateGroupKey(
  base: BaseClient,
  chatMid: string,
): Promise<SharedKey> {
  let pending = registrations.get(base);
  if (!pending) {
    pending = new Map();
    registrations.set(base, pending);
  }
  const existing = pending.get(chatMid);
  if (existing) return await existing;
  const operation = (async () => {
    const rejected = usedKeyIds.get(base)?.get(chatMid);
    await assertEveryMemberHasKey(base, chatMid);
    const latest = await latestKey(base, chatMid);
    if (
      latest && rejected !== undefined && Number(latest.groupKeyId) !== rejected
    ) {
      logGroupKey("reuse", {
        chat: chatMid.slice(-6),
        rejected,
        ...keyFields(latest),
      });
      rememberKey(base, chatMid, latest);
      return latest;
    }
    const registered = await base.e2ee.tryRegisterE2EEGroupKey(chatMid);
    rememberKey(base, chatMid, registered);
    const after = await latestKey(base, chatMid);
    logGroupKey("registered", {
      chat: chatMid.slice(-6),
      rejected,
      before: keyFields(latest)?.groupKeyId,
      registered: keyFields(registered),
      // A different id here means another client registered at the same time.
      latest: keyFields(after)?.groupKeyId,
    });
    return registered;
  })();
  pending.set(chatMid, operation);
  try {
    return await operation;
  } finally {
    pending.delete(chatMid);
  }
}

export function patchGroupKeyRecovery(base: BaseClient): void {
  if (patched.has(base)) return;
  patched.add(base);
  const getLastKey = base.talk.getLastE2EEGroupSharedKey.bind(base.talk);
  lookups.set(base, (options) => getLastKey(options));
  base.talk.getLastE2EEGroupSharedKey = async (options) => {
    try {
      const key = await getLastKey(options);
      if (options?.chatMid?.startsWith("c")) {
        rememberKey(base, options.chatMid, key);
      }
      return key;
    } catch (error) {
      const mid = options?.chatMid;
      if (!mid?.startsWith("c") || !needsGroupKeyRecreation(error)) throw error;
      return await recreateGroupKey(base, mid);
    }
  };

  const sendMessage = base.talk.sendMessage.bind(base.talk);
  base.talk.sendMessage = async (options) => {
    // linejs mutates options.e2ee when it recursively switches from plain text
    // to encryption. Decide here so the outer call cannot repeat an inner repair.
    const canReencrypt = options.to.startsWith("c") && options.e2ee === true &&
      !options.chunks && Boolean(options.text || options.location);
    try {
      return await sendMessage(options);
    } catch (error) {
      if (!canReencrypt || !needsGroupKeyRecreation(error)) throw error;
      // The server explicitly rejected this envelope. Register for the current
      // members and encrypt the original payload again, at most once. Media
      // callers own their prebuilt chunks and must rebuild those themselves.
      await recreateGroupKey(base, options.to);
      return await sendMessage({ ...options });
    }
  };
}

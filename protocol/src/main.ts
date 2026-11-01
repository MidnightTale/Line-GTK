import {
  loadSearchHistory,
  searchContext,
  searchMessages,
} from "./message_search.ts";
import { lineMentions } from "./mentions.ts";
/**
 * LINE protocol sidecar for line-gtk.
 * NDJSON over stdin/stdout. Heavy I/O is cached + async so GTK stays snappy.
 */

import { Client, TalkMessage } from "@evex/linejs";
import { fromFileUrl, join } from "@std/path";
import { atomicWriteJson, ensurePrivateDir } from "./storage.ts";
import { parseCacheKey } from "./cache_crypto.ts";
import { readMessageFile, writeMessageFile } from "./message_store.ts";
import { AuthStore, LineDevice, restoreSession } from "./auth.ts";
import { emailLoginError } from "./email_login.ts";
import { CachePolicy, policyFor } from "./cache_policy.ts";
import { contactKind, picturePathOf, profileUrl } from "./contacts.ts";
import { coerceI64, conversationMid, normalizeRaw } from "./messages.ts";
import {
  cacheFill,
  MAX_STORED_MESSAGES,
  mergeMessages,
  normalizeMessageLimit,
  sortMessages,
} from "./message_history.ts";
import { appendLive } from "./message_delta.ts";
import { replyToOf } from "./replies.ts";
import { groupProfile, setGroupMuted } from "./groups.ts";
import {
  DECRYPTION_FAILED,
  guardedDecrypt,
  needsDecryptionRefresh,
  restoreCachedMessages,
  retryKeyDecryption,
} from "./e2ee_guard.ts";
import * as calls from "./calls.ts";
import { createOutgoingMedia } from "./outgoing_media.ts";
import { createLinePollService, type PollClient } from "./line_polls.ts";
import { patchMediaDecryption } from "./media_crypto.ts";
import { patchGroupKeyRecovery } from "./group_key_recovery.ts";
import { createMediaCache } from "./media_cache.ts";
import { createStickerLibrary } from "./sticker_library.ts";
import { createStickerService } from "./stickers.ts";
import { createCommandService } from "./commands.ts";
import { createContactBlocks } from "./contact_blocks.ts";
import { createListener } from "./listener.ts";
import { createUnsendService } from "./unsend.ts";
import { createReactionEmojiPacks } from "./reaction_emojis.ts";
import {
  createReactionService,
  normalizeReactions,
  reactionCarriers,
} from "./reactions.ts";
import {
  createConnectionRecovery,
  retireClient,
  waitForReceiver,
} from "./connection_recovery.ts";
import {
  createLineFetch,
  failureKind,
  restoreWithRetry,
} from "./connection.ts";
import { createMessageView } from "./message_view.ts";
import { callHistoryOf } from "./call_history.ts";
import { createReadReceipts } from "./read_receipts.ts";
import { createUnreadSync } from "./unread_sync.ts";
import {
  createChatEvents,
  normalizeChatEvent,
  summarizeChatEvent,
} from "./chat_events.ts";
import { readReceiptPermission } from "./read_policy.ts";
import { createEmojiService, lineEmojis } from "./emojis.ts";
import { createChatPreviews } from "./chat_previews.ts";
import { type ChatRow, contactChatRow } from "./chat_rows.ts";
import { createAlbumService } from "./albums.ts";
import { createMessageSenders } from "./message_senders.ts";
import { createUserProfiles } from "./user_profiles.ts";
import {
  type BootProfile,
  loadBootCache,
  loadBootMessages,
  saveBootSession,
} from "./boot_cache.ts";
import {
  createSharedMediaService,
  type SharedMediaAnchor,
  type SharedMediaSource,
} from "./shared_media.ts";
import { createSharedAttachmentCache } from "./shared_media_cache.ts";

type Json = Record<string, unknown>;

const dataDir = Deno.env.get("LINE_GTK_DATA") ??
  join(Deno.env.get("HOME") ?? ".", ".local", "share", "line-gtk");
await ensurePrivateDir(dataDir);
const cacheDir = join(dataDir, "cache");
const avatarDir = join(cacheDir, "avatars");
const mediaDir = join(cacheDir, "media");
const stickerDir = join(cacheDir, "stickers");
await ensurePrivateDir(avatarDir);
await ensurePrivateDir(mediaDir);
await ensurePrivateDir(stickerDir);

const storagePath = join(dataDir, "linejs-storage.json");
const authStore = new AuthStore(dataDir);
/** Docs: reliable PLANET audio needs ANDROID / ANDROIDSECONDARY, not DESKTOPWIN. */
const LINE_DEVICE = (Deno.env.get("LINE_DEVICE")?.trim() ||
  "ANDROIDSECONDARY") as LineDevice;
const LINE_VERSION = Deno.env.get("LINE_VERSION")?.trim() || "26.6.2";
const chatCachePath = join(cacheDir, "chats.json");
const contactCachePath = join(cacheDir, "contacts.json");
const bootSessionPath = join(cacheDir, "session.json");
const msgDiskDir = join(cacheDir, "messages");
await ensurePrivateDir(msgDiskDir);

let cacheRetention = (Deno.env.get("LINE_GTK_CACHE_RETENTION") || "smart")
  .toLowerCase();
let cachePolicyState = policyFor(cacheRetention);
let cachePolicyCheckedAt = 0;
/** Key for the on-disk message cache; null falls back to plaintext. */
const cacheKey = parseCacheKey(Deno.env.get("LINE_GTK_CACHE_KEY"));
if (!cacheKey) {
  console.error("[cache] no encryption key; messages stored in plaintext");
}
/** Per-chat message cap from settings; 0 = unlimited. */
let messageLimit = MAX_STORED_MESSAGES;

async function refreshCachePolicy() {
  if (Date.now() - cachePolicyCheckedAt < 3_000) return cachePolicyState;
  cachePolicyCheckedAt = Date.now();
  try {
    const raw = JSON.parse(
      await Deno.readTextFile(join(dataDir, "settings.json")),
    );
    messageLimit = normalizeMessageLimit(
      raw.message_cache_limit ?? MAX_STORED_MESSAGES,
    );
    const next = String(raw.cache_retention || cacheRetention || "smart")
      .toLowerCase();
    if (next !== cacheRetention) {
      cacheRetention = next;
      cachePolicyState = policyFor(next);
      console.error(`[cache] retention → ${next}`);
    }
  } catch {
    /* keep current */
  }
  return cachePolicyState;
}

function cachePolicy(): CachePolicy {
  return cachePolicyState;
}
const AVATAR_CONCURRENCY = 3;
const MEDIA_CONCURRENCY = 1;
const MEDIA_FIRST_BATCH = 5;
const THUMB_MAX = 440;
const THUMB_BYTES = 220_000;
/** Treat cached images at or below this as LINE previews, not full originals. */
const PREVIEW_MAX_EDGE = 512;

const _err = console.error.bind(console);
for (const level of ["log", "info", "debug", "warn"] as const) {
  console[level] = (...args: unknown[]) => _err(`[linejs:${level}]`, ...args);
}

const textEnc = new TextEncoder();
/** False after GTK closes the sidecar pipe (logout / quit / restart). */
let stdoutAlive = true;

function emit(obj: Json) {
  if (!stdoutAlive) return;
  try {
    Deno.stdout.writeSync(textEnc.encode(`${JSON.stringify(obj)}\n`));
  } catch (e) {
    const code = (e as { code?: string })?.code;
    const msg = e instanceof Error ? e.message : String(e);
    if (code === "EPIPE" || msg.includes("Broken pipe")) {
      stdoutAlive = false;
      return;
    }
    throw e;
  }
}
function emitEvent(event: string, payload: Json = {}) {
  emit({ event, ...payload });
}
function ok(id: number | string | null, result: unknown = {}) {
  emit({ id, ok: true, result });
}
function fail(id: number | string | null, error: string) {
  emit({ id, ok: false, error });
}

// linejs starts detached polling loops. Classify their failures before Deno
// exits so a transient outage never masquerades as an expired login.
let authDead = false;
globalThis.addEventListener("unhandledrejection", (ev) => {
  const reason = (ev as PromiseRejectionEvent).reason;
  const kind = failureKind(reason);
  if (kind === "auth") {
    ev.preventDefault();
    void sessionFailure(reason);
  } else if (kind === "network" || kind === "server") {
    ev.preventDefault();
    listening = false;
    emitEvent("connection_lost", { kind });
  } else {
    // Unknown programming/protocol errors must remain visible and allow the
    // existing bounded process recovery to handle an actual sidecar crash.
    console.error("[unhandledrejection]", reason);
  }
});

async function sessionFailure(error: unknown) {
  const kind = failureKind(error);
  if (kind === "auth") {
    if (authDead) return;
    authDead = true;
    listening = false;
    if (client) retireClient(client);
    client = null;
    cancelBackgroundWork();
    await clearAuth();
    emitEvent("session_failed", { error: "line_auth_expired" });
  } else {
    // Keep the saved token and cached chats even when restore retries run out.
    emitEvent("connection_failed", { kind });
  }
}

let client: Client | null = null;
/** Account of the cache shown before the network login finishes. */
let bootAccountMid = "";
const albums = createAlbumService({
  getClient: () => client,
  sessionId: () => myMid(),
  mediaDir,
});
let listening = false;
let bootRestoreController: AbortController | null = null;

calls.configureCallRuntime({
  getClient: () => client,
  loadAuthDevice,
  emitEvent,
  ok,
  fail,
});

const mediaCache = createMediaCache({
  getClient: () => client,
  avatarDir,
  mediaDir,
  thumbMax: THUMB_MAX,
  thumbBytes: THUMB_BYTES,
  previewMaxEdge: PREVIEW_MAX_EDGE,
  refetchMessageData,
});
const {
  existingFile,
  isImageBytes,
  existingImage,
  writeImageFile,
  cacheUrl,
  avatarPathFor,
  mediaDest,
  fullDest,
  thumbDest,
  isPreviewOrThumbImage,
  existingFullImage,
  writeFullImage,
  uiMediaPath,
  isValidAudioFile,
  downloadAudioBytes,
} = mediaCache;

const messageView = createMessageView({
  normType,
  existingFullImage,
  existingImage,
  mediaDest,
  thumbDest,
  isValidAudioFile,
  uiMediaPath,
  stickerDir,
});
const { previewLang, previewLine, extractFlex } = messageView;
async function forUiMessages(messages: Json[], chatMid?: string) {
  if (chatMid) messages = await unsends.mask(chatMid, messages);
  messages = messages.map(normalizeChatEvent);
  if (chatMid) messages = await chatEvents.merge(chatMid, messages);
  if (chatMid) {
    messages = await reactions.merge(chatMid, messages);
    const carriers = reactionCarriers(chatMid, messages);
    messageSenders.observe(carriers.profiles);
    emojiService.observe(carriers.emojis);
  }
  reactions.observe(messages);
  messageSenders.observe(messages);
  emojiService.observe(messages);
  return messageView.forUiMessages(messages);
}

const stickerLibrary = createStickerLibrary({
  dataDir,
  stickerDir,
  session: sharedSession,
  emitEvent,
});
const stickers = createStickerService({
  library: stickerLibrary,
  getClient: () => client,
  dataDir,
  stickerDir,
  refreshCachePolicy,
  cachePolicy,
  cacheUrl,
  existingImage,
  sentMessagePayload,
  invalidateMessages,
  invalidateChats: () => {
    chatCache = null;
  },
  touchChatPreviewFromMessage,
  ok,
  fail,
});

const linePolls = createLinePollService(() =>
  client as unknown as PollClient | null
);
const outgoingMedia = createOutgoingMedia({
  getClient: () => client,
  mediaDir,
  mediaDest,
  fullDest,
  thumbDest,
  existingFile,
  existingImage,
  isImageBytes,
  writeImageFile,
  writeFullImage,
  refetchMessageData,
  uiMediaPath,
  isPreviewOrThumbImage,
  getBoxCursor: (chatMid) => boxCursor.get(chatMid),
  isMineFrom,
  normType,
  getCachedMessages: (chatMid) => msgCache.get(chatMid)?.messages,
  invalidateMessages,
  invalidateChats: () => {
    chatCache = null;
  },
  sentMessagePayload,
  touchChatPreviewFromMessage,
  emitEvent,
  ok,
  fail,
});

let chatCache: { at: number; chats: ChatRow[] } | null = null;
let chatCacheWrite = Promise.resolve();
function saveChatCache(): Promise<void> {
  const write = chatCacheWrite.then(async () => {
    // 起動直後に別アカウントの一覧を出さないよう、保存時のアカウントを記録する。
    if (chatCache) {
      await atomicWriteJson(chatCachePath, {
        ...chatCache,
        accountMid: myMid() || bootAccountMid,
      });
    }
  });
  // 1回の保存失敗で、後続の更新まで停止しないようにする。
  chatCacheWrite = write.catch(() => {});
  return write;
}

const msgCache = new Map<string, { at: number; messages: Json[] }>();

/** Forget a chat's stored messages and the marker that says they are current. */
function invalidateMessages(chatMid: string) {
  msgCache.delete(chatMid);
  boxCursor.delete(chatMid);
}

/** Add a live message to history that was already checked this session. */
function appendCachedMessage(chatMid: string, message: Json) {
  void sharedMedia.observe(chatMid, [{ message }]).catch(() => {});
  const cached = msgCache.get(chatMid);
  if (!cached) return;
  msgCache.set(chatMid, {
    at: cached.at,
    messages: appendLive(cached.messages, message),
  });
  void saveDiskMessages(chatMid, [message], { contiguous: true });
}
const inFlight = new Map<string, Promise<unknown>>();
const contactIndex = new Map<
  string,
  { name: string; picturePath: string | null; kind: string; muted: boolean }
>();
const mediaEpoch = new Map<string, number>();
type BoxCursor = {
  messageId: bigint | number;
  deliveredTime: bigint | number;
};
const boxCursor = new Map<string, BoxCursor>();
const messageSenders = createMessageSenders({
  getClient: () => authDead ? null : client,
  selfMid: myMid,
  knownName: (mid) => contactIndex.get(mid)?.name,
  emit: (mid, name) => emitEvent("sender_profile", { mid, name }),
});
const emojiService = createEmojiService({
  directory: mediaDir,
  emit: (key, path) => emitEvent("emoji_ready", { key, path }),
});

const chatPreviews = createChatPreviews({
  getChats: () => chatCache?.chats ?? [],
  previewLine,
  fallback: () => previewLang().tap,
  emit: (chat) => emitEvent("chat_preview", chat),
  save: async () => {
    if (chatCache) {
      try {
        await saveChatCache();
      } catch {
        console.error(
          "[chat-preview-cache] プレビューを保存できませんでした。",
        );
      }
    }
  },
});

const readReceipts = createReadReceipts({
  getClient: () => client,
  selfMid: myMid,
  emit: emitEvent,
});

const unreadSync = createUnreadSync({
  getClient: () => client,
  selfMid: myMid,
  latestMessageId: (mid) => boxCursor.get(mid)?.messageId,
  getChats: () => chatCache?.chats ?? [],
  save: saveChatCache,
  emit: emitEvent,
});

const chatEvents = createChatEvents({
  directory: msgDiskDir,
  getClient: () => authDead ? null : client,
  selfMid: myMid,
  observeProfiles: messageSenders.observe,
  emit: emitEvent,
});

const contactBlocks = createContactBlocks({
  getClient: () => authDead ? null : client,
  selfMid: myMid,
  name: (mid) => contactIndex.get(mid)?.name,
  emit: emitEvent,
});

const userProfiles = createUserProfiles({
  getClient: () => authDead ? null : client,
  selfMid: myMid,
  avatarPathFor,
  kindOf: (contact) =>
    contactKind(
      contact as Parameters<typeof contactKind>[0],
      undefined,
    ),
  emit: emitEvent,
});

const unsends = createUnsendService({
  getClient: () => authDead ? null : client,
  dataDir,
  emit: emitEvent,
  findMessage: async (chatMid, id, createdTime) => {
    const cached = msgCache.get(chatMid)?.messages ??
      await loadDiskMessages(chatMid);
    const known = cached?.find((message) => String(message.id) === id);
    if (known) return known;
    const current = client;
    if (!current) return undefined;
    const raw = await current.base.talk.getPreviousMessagesV2WithRequest({
      request: {
        messageBoxId: chatMid,
        endMessageId: {
          messageId: coerceI64(id),
          deliveredTime: coerceI64(
            Number.isSafeInteger(createdTime) && createdTime > 0
              ? createdTime
              : 0,
          ),
        },
        messagesCount: 1,
      },
    });
    const message = raw.find((message) => String(message.id) === id);
    return message
      ? { ...message, contentType: normType(message.contentType) }
      : undefined;
  },
  changed: async (chatMid, id) => {
    await sharedMedia.unsent(chatMid, id);
    const cached = msgCache.get(chatMid)?.messages ??
      await loadDiskMessages(chatMid);
    if (cached?.length) {
      const messages = await unsends.mask(chatMid, cached);
      msgCache.set(chatMid, { at: Date.now(), messages });
      await saveDiskMessages(chatMid, messages);
      await chatPreviews.updateFromMessages(chatMid, messages);
    }
    if (String(boxCursor.get(chatMid)?.messageId) === id) {
      const preview = previewLang().unsent;
      const row = chatCache?.chats.find((chat) => chat.mid === chatMid);
      if (row) row.preview = preview;
      emitEvent("chat_preview", {
        mid: chatMid,
        preview,
        lastActivity: row?.lastActivity ?? 0,
      });
      await saveChatCache();
    }
  },
});

// The owner changes even when the same account logs in again with a new client.
let sharedOwnerClient: Client | null = null;
let sharedOwnerGeneration = -1;
let sharedOwner: object = {};
function sharedSession() {
  if (!client || authDead || !myMid()) return null;
  if (sharedOwnerClient !== client || sharedOwnerGeneration !== workGen) {
    sharedOwnerClient = client;
    sharedOwnerGeneration = workGen;
    sharedOwner = {};
  }
  return { accountMid: myMid(), owner: sharedOwner };
}

async function sharedHistoryPage(
  chatMid: string,
  anchor: SharedMediaAnchor | null,
): Promise<SharedMediaSource[]> {
  const active = client;
  const session = sharedSession();
  if (!active || !session) throw new Error("shared_media_not_logged_in");
  const check = () => {
    if (sharedSession()?.owner !== session.owner) {
      throw new Error("shared_media_cancelled");
    }
  };
  const raw = anchor
    ? await active.base.talk.getPreviousMessagesV2WithRequest({
      request: {
        messageBoxId: chatMid,
        endMessageId: {
          messageId: coerceI64(anchor.messageId),
          deliveredTime: coerceI64(
            anchor.createdTime > 0 ? anchor.createdTime : 0,
          ),
        },
        messagesCount: 100,
      },
    })
    : await active.base.talk.getRecentMessagesV2({
      messageBoxId: chatMid,
      messagesCount: 100,
    });
  check();
  const result: SharedMediaSource[] = [];
  for (const message of raw) {
    // The history endpoint can include the requested boundary message.
    if (anchor && String(message.id) === anchor.messageId) continue;
    const summary = await summarizeRawMessage(message, { withMedia: false });
    check();
    const meta = message.contentMetadata ?? {};
    const size = Number(meta.FILE_SIZE);
    if (meta.FILE_SIZE && Number.isSafeInteger(size) && size >= 0) {
      summary.fileSize = size;
    }
    result.push({ message: summary, raw: message as unknown as Json });
  }
  return result;
}

async function resolveSharedTalk(chatMid: string, source: SharedMediaSource) {
  const active = client;
  const session = sharedSession();
  if (!active || !session) throw new Error("shared_media_not_logged_in");
  const check = () => {
    if (sharedSession()?.owner !== session.owner) {
      throw new Error("shared_media_cancelled");
    }
  };
  let raw = source.raw;
  if (!raw) {
    const messages = await active.base.talk.getPreviousMessagesV2WithRequest({
      request: {
        messageBoxId: chatMid,
        endMessageId: {
          messageId: coerceI64(String(source.message.id)),
          deliveredTime: coerceI64(Number(source.message.createdTime) || 0),
        },
        messagesCount: 2,
      },
    });
    check();
    raw = messages.find((message) =>
      String(message.id) === String(source.message.id)
    ) as unknown as Json;
    if (!raw) {
      // Exclusive-boundary servers need an actual newer message as the anchor.
      const targetId = BigInt(String(source.message.id));
      const cached = await sharedSeedMessages(chatMid);
      check();
      const next = cached.filter((message) => {
        try {
          return BigInt(String(message.id)) > targetId;
        } catch {
          return false;
        }
      }).sort((a, b) => {
        const aa = BigInt(String(a.id)), bb = BigInt(String(b.id));
        return aa < bb ? -1 : aa > bb ? 1 : 0;
      })[0];
      const nearby = next
        ? await active.base.talk.getPreviousMessagesV2WithRequest({
          request: {
            messageBoxId: chatMid,
            endMessageId: {
              messageId: coerceI64(String(next.id)),
              deliveredTime: coerceI64(Number(next.createdTime) || 0),
            },
            messagesCount: 100,
          },
        })
        : await active.base.talk.getRecentMessagesV2({
          messageBoxId: chatMid,
          messagesCount: 100,
        });
      check();
      raw = nearby.find((message) =>
        String(message.id) === String(source.message.id)
      ) as unknown as Json;
    }
  }
  if (!raw || String(raw.id) !== String(source.message.id)) {
    throw new Error("shared_media_original_unavailable");
  }
  const restored = normalizeRaw(raw);
  if (Array.isArray(restored.chunks)) {
    restored.chunks = restored.chunks.map((chunk: unknown) => {
      if (chunk instanceof Uint8Array) return chunk;
      const value = chunk as { type?: string; data?: number[] };
      return Array.isArray(value?.data) ? Uint8Array.from(value.data) : chunk;
    });
  }
  const tm = await TalkMessage.fromRawTalk(
    restored as unknown as Parameters<typeof TalkMessage.fromRawTalk>[0],
    active,
  );
  check();
  return tm;
}

async function sharedSeedMessages(chatMid: string): Promise<Json[]> {
  const session = sharedSession();
  if (!session) return [];
  let disk: Json[] = [];
  try {
    // Shared browsing reuses retained content independently of refresh TTL.
    const cached = await readMessageFile(msgDiskDir, chatMid, cacheKey);
    if (
      cached.accountMid === session.accountMid && Array.isArray(cached.messages)
    ) {
      disk = restoreCachedMessages(cached.messages as Json[]);
    }
  } catch { /* no existing cache */ }
  if (sharedSession()?.owner !== session.owner) {
    throw new Error("shared_media_cancelled");
  }
  // Memory entries have no durable provenance. History discovery verifies
  // legacy IDs before adopting them and reuses their existing media files.
  return disk;
}

const sharedMedia = createSharedMediaService({
  cacheDir,
  session: sharedSession,
  seed: async (chatMid) => {
    const messages = await sharedSeedMessages(chatMid);
    return messages.map((message) => ({ message }));
  },
  fetchPage: sharedHistoryPage,
  cacheAttachment: createSharedAttachmentCache({
    mediaDir,
    accountMid: myMid,
    getClient: () => client,
    resolve: resolveSharedTalk,
    existingFile,
    existingFullImage,
    isImageBytes,
    isPreview: isPreviewOrThumbImage,
    mediaDest,
    thumbDest,
  }),
  mask: (chatMid, messages) => unsends.mask(chatMid, messages),
  changed: (chatMid) => {
    const session = sharedSession();
    if (session) {
      emitEvent("shared_media_changed", {
        accountMid: session.accountMid,
        chatMid,
      });
    }
  },
});

const reactions = createReactionService({
  dataDir,
  getClient: () => authDead ? null : client,
  emit: emitEvent,
});

const reactionEmojiPacks = createReactionEmojiPacks({
  accountMid: myMid,
  directory: mediaDir,
  observe: (messages) => emojiService.observe(messages),
  ownedProducts: async () => {
    const current = client;
    if (!current) throw new Error("not_logged_in");
    const { ShopService } = await import("@linegtk/shop");
    const language = Deno.env.get("LINE_GTK_LANG") === "ja"
      ? { language: "ja", country: "JP" }
      : { language: "en", country: "TH" };
    return await new ShopService(current.base as never)
      .getOwnedProductSummaries({
        shopId: "sticonshop",
        offset: 0,
        limit: 80,
        locale: language,
      } as never);
  },
});

const listener = createListener({
  prepareMessage: async (message) => {
    const [prepared] = await unsends.mask(conversationMid(message), [message]);
    return prepared?.contentType === "UNSENT" ? null : prepared ?? null;
  },
  handleIncomingCall: calls.handleIncomingCall,
  handleCallCancel: calls.handleCallCancel,
  shouldIgnoreMessage: contactBlocks.ignoreMessage,
  getClient: () => client,
  observeMessage: (message) => {
    unreadSync.observeMessage(message);
    messageSenders.observe([message]);
    emojiService.observe([message]);
    reactions.observe([message]);
  },
  refreshMessageSender: messageSenders.refresh,
  isListening: () => listening,
  receiverStalled: () => {
    console.error("[receiver] PUSH heartbeat or event consumer stopped");
    emitEvent("connection_lost", { kind: "network" });
  },
  setListening: (next) => {
    listening = next;
  },
  appendMessage: appendCachedMessage,
  summarizeTalkMessage,
  storeBoxCursor,
  upsertChatFromMessage,
  hydrateLiveMedia,
  upsertChatFromContact,
  handleReadReceipt: (operation) =>
    unsends.handleOperation(operation) ||
    reactions.handleOperation(operation) ||
    contactBlocks.handleOperation(operation) ||
    unreadSync.handleOperation(operation) ||
    readReceipts.handleOperation(operation) ||
    chatEvents.handleOperation(operation),
  emitEvent,
});

const connectionRecovery = createConnectionRecovery({
  getClient: () => authDead ? null : client,
  restore: async (current) => {
    const token = current.authToken;
    const device = await loadAuthDevice();
    return await restoreSession(token, device, storagePath, LINE_VERSION);
  },
  activate: (next) => {
    client = next;
    listening = false;
    // 休止前の応答しない取得に、復帰後の取得を合流させない。
    inFlight.clear();
    patchE2eeGuards();
    listener.start();
  },
});

const commands = createCommandService({
  recordOwnRead: (chatMid, through) => {
    unreadSync.handleOperation(
      { type: 40, param1: chatMid, param2: through },
      true,
    );
  },
  getClient: () => client,
  setClient: (next) => {
    if (client !== next) msgCache.clear();
    client = next;
  },
  isListening: () => listening,
  setAuthDead: (dead) => {
    authDead = dead;
  },
  getChatCache: () => chatCache,
  setChatCache: (cache) => {
    chatCache = cache;
  },
  msgCache,
  contactIndex,
  boxCursor,
  avatarDir,
  chatCachePath,
  saveChatCache,
  storagePath,
  lineDevice: LINE_DEVICE,
  lineVersion: LINE_VERSION,
  refreshCachePolicy,
  cachePolicy,
  dedupe,
  storeBoxCursor,
  existingFile,
  loadDiskContacts,
  saveDiskContacts,
  refreshContactIndex,
  hydrateAvatars,
  avatarPathFor,
  hydratePreviews,
  updateChatPreview: async (chatMid, messages) =>
    await chatPreviews.updateFromMessages(
      chatMid,
      await unsends.mask(chatMid, messages),
    ),
  refreshReadReceipts: readReceipts.refresh,
  reconcileUnread: unreadSync.reconcile,
  readReceiptPermission: (chatMid) =>
    readReceiptPermission(join(dataDir, "settings.json"), myMid(), chatMid),
  bumpMediaEpoch,
  loadDiskMessages,
  saveDiskMessages,
  forUiMessages,
  hydrateMedia,
  summarizeTalkMessage,
  summarizeRawMessage: (raw, opts) =>
    summarizeRawMessage(
      raw as Parameters<typeof summarizeRawMessage>[0],
      opts,
    ),
  saveAuth,
  loadAuth,
  loadAuthDevice,
  clearAuth,
  myProfilePayload,
  patchE2eeGuards,
  startListen: listener.start,
  sentMessagePayload,
  touchChatPreviewFromMessage,
  emitEvent,
  ok,
  fail,
});

function storeBoxCursor(
  mid: string,
  last?: { messageId?: unknown; deliveredTime?: unknown } | null,
) {
  if (last?.messageId == null || last?.deliveredTime == null) return;
  try {
    boxCursor.set(mid, {
      messageId: coerceI64(last.messageId),
      deliveredTime: coerceI64(last.deliveredTime),
    });
  } catch (e) {
    console.error("[boxCursor]", mid, e);
  }
}

function bumpMediaEpoch(chatMid: string): number {
  const n = (mediaEpoch.get(chatMid) ?? 0) + 1;
  mediaEpoch.set(chatMid, n);
  return n;
}

/** Bumped to cancel hydrateMedia / hydratePreviews after logout / token kick. */
let workGen = 0;
function cancelBackgroundWork() {
  workGen++;
  for (const mid of [...mediaEpoch.keys()]) {
    bumpMediaEpoch(mid);
  }
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

function normType(t: unknown): string {
  if (typeof t === "number") {
    // Matches @evex/linejs-types ContentType enum.
    const map: Record<number, string> = {
      0: "NONE",
      1: "IMAGE",
      2: "VIDEO",
      3: "AUDIO",
      4: "HTML",
      5: "PDF",
      6: "CALL",
      7: "STICKER",
      8: "PRESENCE",
      9: "GIFT",
      10: "GROUPBOARD",
      11: "APPLINK",
      12: "LINK",
      13: "CONTACT",
      14: "FILE",
      15: "LOCATION",
      16: "POSTNOTIFICATION",
      17: "RICH",
      18: "CHATEVENT",
      19: "MUSIC",
      20: "PAYMENT",
      21: "EXTIMAGE",
      22: "FLEX",
    };
    return map[t] ?? String(t);
  }
  return String(t ?? "NONE").toUpperCase();
}

async function fromRawTalkSafe(raw: unknown): Promise<TalkMessage> {
  const normalized = normalizeRaw(raw as Record<string, unknown>);
  return await TalkMessage.fromRawTalk(
    normalized as unknown as Parameters<typeof TalkMessage.fromRawTalk>[0],
    client!,
  );
}

const guardedClients = new WeakSet<Client>();

function patchE2eeGuards() {
  if (!client) return;
  if (guardedClients.has(client)) return;
  guardedClients.add(client);
  const current = client;
  patchGroupKeyRecovery(client.base);
  patchMediaDecryption(client.base.e2ee);
  const e2ee = client.base.e2ee as unknown as {
    decryptE2EEMessage: (
      m: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>;
  };
  const orig = e2ee.decryptE2EEMessage.bind(e2ee);
  e2ee.decryptE2EEMessage = guardedDecrypt(
    retryKeyDecryption(
      orig,
      () => client === current && !authDead && Boolean(current.base.authToken),
    ),
    () => {
      console.error(
        "[e2ee] メッセージを復号できません。暗号鍵の再取得が必要です。",
      );
    },
  );
}

const e2eeFileNames = new Map<string, string>();

async function e2eeFileName(
  tm: TalkMessage,
  id: string,
): Promise<string | null> {
  const known = e2eeFileNames.get(id);
  if (known) return known;
  const raw = (tm as unknown as { raw?: { chunks?: unknown[] } }).raw;
  if (!client || !raw?.chunks?.length) return null;
  try {
    const e2ee = client.base.e2ee as unknown as {
      decryptE2EEDataMessage: (
        m: unknown,
      ) => Promise<{ fileName?: unknown }>;
    };
    const { fileName } = await e2ee.decryptE2EEDataMessage(raw);
    if (typeof fileName === "string" && fileName) {
      e2eeFileNames.set(id, fileName);
      return fileName;
    }
  } catch (e) {
    console.error("[e2ee file name]", id, e);
  }
  return null;
}

async function refetchMessageData(
  chatMid: string,
  messageId: string,
  opts: { allowNonImage?: boolean } = {},
): Promise<{ buf: Uint8Array; mime: string } | null> {
  if (!client) return null;
  let cursor = boxCursor.get(chatMid);
  if (!cursor) {
    try {
      const boxes = await client.base.talk.getMessageBoxes({
        messageBoxListRequest: {},
      });
      for (const box of boxes.messageBoxes ?? []) {
        storeBoxCursor(String(box.id ?? ""), box.lastDeliveredMessageId);
      }
      cursor = boxCursor.get(chatMid);
    } catch {
      return null;
    }
  }
  if (!cursor) return null;
  try {
    const messages = await client.base.talk.getPreviousMessagesV2WithRequest({
      request: {
        messageBoxId: chatMid,
        endMessageId: {
          messageId: cursor.messageId,
          deliveredTime: cursor.deliveredTime,
        },
        messagesCount: 100,
      },
    });
    const raw = messages.find((m) => String(m.id) === messageId);
    if (!raw) return null;
    const tm = await fromRawTalkSafe(raw);
    const blob = await tm.getData(false);
    const buf = new Uint8Array(await blob.arrayBuffer());
    if (!opts.allowNonImage && !isImageBytes(buf)) return null;
    return { buf, mime: blob.type || "application/octet-stream" };
  } catch (e) {
    console.error("[refetchMessageData]", messageId, e);
    return null;
  }
}

function dedupe<T>(key: string, fn: () => Promise<T>): Promise<T> {
  const existing = inFlight.get(key);
  if (existing) return existing as Promise<T>;
  const p = fn().finally(() => {
    if (inFlight.get(key) === p) inFlight.delete(key);
  });
  inFlight.set(key, p);
  return p;
}

async function mapPool<T, R>(
  items: T[],
  concurrency: number,
  worker: (item: T, i: number) => Promise<R>,
): Promise<R[]> {
  const out = new Array<R>(items.length);
  let next = 0;
  async function run() {
    while (next < items.length) {
      const i = next++;
      out[i] = await worker(items[i], i);
    }
  }
  await Promise.all(
    Array.from(
      { length: Math.min(concurrency, Math.max(items.length, 1)) },
      () => run(),
    ),
  );
  return out;
}

async function saveAuth(token: string, device = LINE_DEVICE) {
  await authStore.save(token, device);
}
async function loadAuth(): Promise<string | null> {
  return await authStore.loadToken();
}
async function loadAuthDevice(): Promise<LineDevice> {
  return await authStore.loadDevice();
}
async function clearAuth() {
  await authStore.clear();
}

function myMid(): string {
  try {
    return client?.base?.profile?.mid ?? "";
  } catch {
    return "";
  }
}

/** Prefer raw mid compare — TalkMessage.isMyMessage crashes if client was cleared. */
function isMineFrom(from: unknown): boolean {
  const mid = myMid();
  const f = String(from ?? "");
  return !!mid && !!f && f === mid;
}

async function myProfilePayload(profile: {
  mid: string;
  displayName?: string;
  statusMessage?: string;
}) {
  let picturePath = picturePathOf(
    profile as unknown as Record<string, unknown>,
  );
  if (!picturePath) {
    picturePath = contactIndex.get(profile.mid)?.picturePath ?? null;
  }
  const pictureUrl = profileUrl(picturePath);
  let avatarPath: string | null = null;
  try {
    avatarPath = await avatarPathFor(profile.mid, picturePath);
  } catch (e) {
    console.error("[myProfile avatar]", e);
  }
  if (avatarPath) {
    emitEvent("avatar_ready", { mid: profile.mid, avatarPath });
  }
  const payload = {
    mid: profile.mid,
    displayName: profile.displayName ?? "",
    statusMessage: profile.statusMessage ?? "",
    picturePath,
    avatarPath,
    pictureUrl,
  };
  bootAccountMid = profile.mid;
  void saveBootSession(bootSessionPath, payload as BootProfile).catch((e) =>
    console.error("[boot session]", e)
  );
  return payload;
}

async function loadDiskContacts() {
  if (contactIndex.size > 0) return;
  try {
    const raw = JSON.parse(await Deno.readTextFile(contactCachePath));
    if (!raw?.contacts) return;
    for (
      const [mid, c] of Object.entries(
        raw.contacts as Record<string, {
          name: string;
          picturePath: string | null;
          kind?: string;
          muted?: boolean;
        }>,
      )
    ) {
      contactIndex.set(mid, {
        name: c.name,
        picturePath: c.picturePath ?? null,
        kind: c.kind || "dm",
        muted: !!c.muted,
      });
    }
    const at = Number(raw.at || 0);
    if (raw.classificationVersion === 1 && Number.isFinite(at) && at > 0) {
      contactsAt = at;
    }
  } catch { /* miss */ }
}

async function saveDiskContacts() {
  const contacts: Record<
    string,
    { name: string; picturePath: string | null; kind: string; muted: boolean }
  > = {};
  for (const [mid, c] of contactIndex) contacts[mid] = c;
  try {
    await atomicWriteJson(contactCachePath, {
      at: Date.now(),
      contacts,
      classificationVersion: contactsAt > 0 ? 1 : 0,
    });
  } catch (error) {
    console.error("[contacts-cache]", error);
  }
}

async function loadDiskMessages(chatMid: string): Promise<Json[] | null> {
  const session = sharedSession();
  if (!session) return null;
  try {
    await refreshCachePolicy();
    const raw = await readMessageFile(msgDiskDir, chatMid, cacheKey);
    if (
      sharedSession()?.owner !== session.owner ||
      raw.accountMid !== session.accountMid
    ) {
      return null;
    }
    if (Date.now() - Number(raw.at || 0) > cachePolicy().diskMsg) return null;
    if (Array.isArray(raw.messages)) {
      const messages = await unsends.mask(
        chatMid,
        restoreCachedMessages(raw.messages as Json[]),
      );
      return sharedSession()?.owner === session.owner ? messages : null;
    }
  } catch { /* miss */ }
  return null;
}

/** Chats already warned about reaching the message cap, per fill level. */
const cacheWarned = new Map<string, string>();

function warnIfNearLimit(chatMid: string, count: number) {
  const fill = cacheFill(count, messageLimit);
  if (fill === "ok") {
    cacheWarned.delete(chatMid);
    return;
  }
  if (cacheWarned.get(chatMid) === fill) return;
  cacheWarned.set(chatMid, fill);
  emitEvent("message_cache_limit", {
    chatMid,
    count,
    limit: messageLimit,
    full: fill === "full",
  });
}

async function saveDiskMessages(
  chatMid: string,
  messages: Json[],
  opts: { contiguous?: boolean } = {},
) {
  const session = sharedSession();
  if (!session) return;
  try {
    let stored: Json[] = [];
    try {
      const raw = await readMessageFile(msgDiskDir, chatMid, cacheKey);
      if (
        raw.accountMid === session.accountMid && Array.isArray(raw.messages)
      ) {
        stored = raw.messages as Json[];
      }
    } catch { /* first write */ }
    const merged = mergeMessages(
      stored,
      await unsends.mask(chatMid, messages),
      { ...opts, max: messageLimit },
    );
    if (sharedSession()?.owner !== session.owner) return;
    await writeMessageFile(msgDiskDir, chatMid, cacheKey, {
      at: Date.now(),
      accountMid: session.accountMid,
      messages: merged,
    });
    warnIfNearLimit(chatMid, merged.length);
  } catch (error) {
    console.error("[messages-cache]", chatMid, error);
  }
}

/** Keep only the newest `keep` messages of one chat's cache. */
async function trimDiskMessages(chatMid: string, keep: number) {
  const session = sharedSession();
  if (!session) return 0;
  const raw = await readMessageFile(msgDiskDir, chatMid, cacheKey);
  if (
    raw.accountMid !== session.accountMid || !Array.isArray(raw.messages)
  ) return 0;
  const kept = sortMessages(raw.messages as Json[]).slice(-Math.max(0, keep));
  if (sharedSession()?.owner !== session.owner) return 0;
  await writeMessageFile(msgDiskDir, chatMid, cacheKey, {
    at: Date.now(),
    accountMid: session.accountMid,
    messages: kept,
  });
  cacheWarned.delete(chatMid);
  return (raw.messages as Json[]).length - kept.length;
}

let contactsAt = 0;

async function refreshContactIndex(force = false) {
  if (!client) return;
  if (
    !force &&
    Date.now() - contactsAt < cachePolicy().contactsRefresh &&
    contactIndex.size > 0
  ) {
    return;
  }
  const ids: string[] = await client.base.talk.getAllContactIds({});
  // Include open message boxes so bots/OA resolve even if not in the friend list.
  try {
    const boxes = await client.base.talk.getMessageBoxes({
      messageBoxListRequest: {},
    });
    for (const box of boxes.messageBoxes ?? []) {
      const mid = String(box.id ?? "");
      if (mid.startsWith("u") && !ids.includes(mid)) ids.push(mid);
    }
  } catch { /* ignore */ }

  // Chunk to avoid huge thrift payloads
  const chunk = 40;
  for (let i = 0; i < ids.length; i += chunk) {
    const slice = ids.slice(i, i + chunk);
    try {
      const contacts = await client.base.talk.getContacts({ mids: slice });
      for (const c of contacts ?? []) {
        const mid = String(c.mid ?? "");
        if (!mid) continue;
        const kind = contactKind(c, contactIndex.get(mid)?.kind);
        contactIndex.set(mid, {
          name: c.displayName || mid,
          picturePath: c.picturePath ?? null,
          kind,
          muted: !!(c as { notificationDisabled?: boolean })
            .notificationDisabled,
        });
      }
    } catch (e) {
      console.error("[contacts chunk]", e);
      for (const mid of slice) {
        try {
          const c = await client.base.talk.getContact({ mid });
          contactIndex.set(mid, {
            name: c.displayName || mid,
            picturePath: c.picturePath ?? null,
            kind: contactKind(c, contactIndex.get(mid)?.kind),
            muted: !!(c as { notificationDisabled?: boolean })
              .notificationDisabled,
          });
        } catch {
          /* bot/OA without contact entry */
        }
      }
    }
  }
  contactsAt = Date.now();
  await saveDiskContacts();
}

async function summarizeTalkMessage(
  tm: TalkMessage,
  opts: { withMedia?: boolean } = {},
): Promise<Json> {
  const event = summarizeChatEvent(tm.raw as unknown as Json);
  if (event) return event;
  const withMedia = opts.withMedia === true;
  const raw = tm.raw as {
    id?: string | number;
    from?: string;
    to?: string;
    createdTime?: number | string | bigint;
    contentType?: string | number;
    contentMetadata?: Record<string, string>;
    relatedMessageId?: unknown;
    messageRelationType?: unknown;
    relatedMessageServiceCode?: unknown;
    reactions?: unknown;
  };
  const replyTo = replyToOf(raw);
  // 操作で届いた後のリアクションと比べるため、取得した時刻を残す。
  const reactionState = {
    reactions: normalizeReactions(raw.reactions, reactions.learnLayout),
    reactionsAt: Date.now(),
  };
  const contentType = normType(raw.contentType);
  const meta = raw.contentMetadata ?? {};
  if (meta[DECRYPTION_FAILED] === "1") {
    return {
      replyTo,
      id: String(raw.id ?? ""),
      text: "",
      from: raw.from ?? "",
      to: raw.to ?? "",
      mine: isMineFrom(raw.from),
      createdTime: Number(raw.createdTime ?? 0),
      contentType,
      decryptionFailed: true,
      needsMedia: false,
    };
  }
  let text = tm.text || meta.ALT_TEXT || meta.STKTXT || "";
  const callHistory = callHistoryOf({
    contentType,
    text,
    contentMetadata: meta,
  });
  if (callHistory) {
    return {
      replyTo,
      id: String(raw.id ?? ""),
      from: raw.from ?? "",
      to: raw.to ?? "",
      mine: isMineFrom(raw.from),
      createdTime: Number(raw.createdTime ?? 0),
      contentType: "CALL",
      text: "",
      callHistory,
      durationMs: callHistory.durationMs,
      needsMedia: false,
    };
  }

  let imagePath: string | null = null;
  let imageUrl: string | null = meta.PREVIEW_URL || meta.DOWNLOAD_URL || null;
  const id = String(raw.id ?? "");
  const stkId = meta.STKID || "";
  const stkPkg = meta.STKPKGID || "";

  if (contentType === "AUDIO") {
    text = text || "Voice message";
    const duration = meta.DURATION || meta.AUDLEN || "";
    let audioPath: string | null = null;
    if (withMedia) {
      const candidates = [
        mediaDest(id, "m4a"),
        mediaDest(id, "mp3"),
        mediaDest(id, "aac"),
        mediaDest(id, "ogg"),
      ];
      for (const p of candidates) {
        if (await isValidAudioFile(p)) {
          audioPath = p;
          break;
        }
        try {
          await Deno.remove(p);
        } catch { /* ignore */ }
      }
      if (!audioPath) {
        try {
          const blob = await tm.getData(false);
          const buf = new Uint8Array(await blob.arrayBuffer());
          if (buf.length >= 1024) {
            const mime = blob.type || "";
            const ext = mime.includes("mpeg") || mime.includes("mp3")
              ? "mp3"
              : mime.includes("aac")
              ? "aac"
              : mime.includes("ogg")
              ? "ogg"
              : "m4a";
            const dest = mediaDest(id, ext);
            await Deno.writeFile(dest, buf);
            if (await isValidAudioFile(dest)) audioPath = dest;
            else {
              try {
                await Deno.remove(dest);
              } catch { /* ignore */ }
            }
          }
        } catch (e) {
          console.error("[audio]", id, e);
        }
      }
    }
    const flex = null;
    return {
      ...reactionState,
      replyTo,
      id,
      text: duration
        ? `Voice message (${Math.round(Number(duration) / 1000)}s)`
        : text,
      from: raw.from ?? "",
      to: raw.to ?? "",
      mine: isMineFrom(raw.from),
      createdTime: Number(raw.createdTime ?? 0),
      contentType,
      imagePath: null,
      imageUrl: null,
      audioPath,
      fileName: null,
      filePath: audioPath,
      stickerId: null,
      flex,
      durationMs: duration ? Number(duration) : null,
      needsMedia: !audioPath,
    };
  }

  let fileName: string | null = meta.FILE_NAME || meta.FILENAME || null;
  let filePath: string | null = null;

  if (contentType === "IMAGE") {
    if (!text) text = "[Image]";
    if (withMedia) {
      const full = await existingFullImage(id);
      if (full) {
        filePath = full;
        imagePath = (await uiMediaPath(id, full)) || full;
      } else {
        try {
          // E2EE: getData ignores preview and returns decrypted original.
          const blob = await tm.getData(false);
          const buf = new Uint8Array(await blob.arrayBuffer());
          if (isImageBytes(buf)) {
            const ext = (blob.type || "").includes("png") || buf[0] === 0x89
              ? "png"
              : "jpg";
            filePath = await writeFullImage(id, buf, blob.type || ext);
            imagePath = (await uiMediaPath(id, filePath)) || filePath;
          }
          if (!filePath && imageUrl) {
            const cached = await cacheUrl(imageUrl, fullDest(id, "jpg"));
            filePath = cached;
            imagePath = (await uiMediaPath(id, filePath)) || filePath;
          }
        } catch (e) {
          console.error("[media image]", id, e);
          if (imageUrl) {
            filePath = await cacheUrl(imageUrl, fullDest(id, "jpg"));
            imagePath = (await uiMediaPath(id, filePath)) || filePath;
          }
        }
      }
      // UI can still show a prior thumb while full download is pending.
      if (!imagePath) {
        imagePath = await existingImage(thumbDest(id));
      }
    }
  } else if (contentType === "VIDEO") {
    if (!text) text = "[Video]";
    fileName = fileName || `${id}.mp4`;
    if (withMedia) {
      const vpath = await existingFile(mediaDest(id, "mp4"));
      if (vpath) filePath = vpath;
      const cached = await existingImage(thumbDest(id)) ||
        await existingImage(mediaDest(id)) ||
        await existingImage(mediaDest(id, "png"));
      if (cached) {
        imagePath = cached;
      } else {
        try {
          const blob = await tm.getData(false);
          const buf = new Uint8Array(await blob.arrayBuffer());
          imagePath = await outgoingMedia.materializeVideoPreview(id, buf);
          filePath = await existingFile(mediaDest(id, "mp4"));
        } catch (e) {
          console.error("[media video]", id, e);
        }
      }
    }
  } else if (contentType === "FILE") {
    // E2EE chats keep the file name inside the encrypted payload.
    if (!fileName && text && text !== "[File]") fileName = text;
    if (!fileName) fileName = await e2eeFileName(tm, id);
    text = fileName || text || "[File]";
    fileName = fileName || text;
    if (withMedia) {
      try {
        for await (const ent of Deno.readDir(mediaDir)) {
          if (
            ent.isFile && ent.name.startsWith(`${id}.`) &&
            !ent.name.includes(".thumb.")
          ) {
            filePath = join(mediaDir, ent.name);
            break;
          }
        }
      } catch { /* ignore */ }
    }
  } else if (contentType === "STICKER") {
    text = text || "[Sticker]";
    if (stkId) {
      imageUrl = stickers.animationUrl(stkId);
      if (withMedia) {
        imagePath = await stickers.ensureImage(stkId);
      }
    }
  } else if (contentType === "FLEX") {
    text = text || meta.ALT_TEXT || "[Flex message]";
  } else if (!text) {
    text = contentType !== "NONE" ? `[${contentType}]` : "";
  }

  const flex = contentType === "FLEX" ? extractFlex(meta) : null;

  return {
    ...reactionState,
    replyTo,
    id,
    text,
    emojis: contentType === "NONE" ? lineEmojis(text, meta) : [],
    mentions: contentType === "NONE" ? lineMentions(text, meta) : [],
    from: raw.from ?? "",
    to: raw.to ?? "",
    mine: isMineFrom(raw.from),
    createdTime: Number(raw.createdTime ?? 0),
    contentType,
    imagePath,
    imageUrl,
    audioPath: null,
    fileName,
    filePath,
    stickerId: stkId || null,
    stickerPackageId: stkPkg || null,
    flex,
    durationMs: meta.DURATION ? Number(meta.DURATION) : null,
    needsMedia: (contentType === "IMAGE" || contentType === "VIDEO" ||
        contentType === "STICKER")
      ? !imagePath
      : contentType === "AUDIO"
      ? true
      : false,
  };
}

async function summarizeRawMessage(
  raw: Parameters<typeof TalkMessage.fromRawTalk>[0],
  opts?: { withMedia?: boolean },
) {
  const event = summarizeChatEvent(raw as unknown as Json);
  if (event) return event;
  try {
    const tm = await fromRawTalkSafe(raw);
    return await summarizeTalkMessage(tm, opts);
  } catch (e) {
    console.error("[summarizeRawMessage]", e);
    const r = normalizeRaw(raw as unknown as Record<string, unknown>);
    const meta = (r.contentMetadata ?? {}) as Record<string, string>;
    const contentType = normType(r.contentType);
    let text = String(r.text ?? meta.ALT_TEXT ?? meta.STKTXT ?? "");
    const callHistory = callHistoryOf({
      contentType,
      text,
      contentMetadata: meta,
    });
    if (!text) {
      if (contentType === "AUDIO") text = "Voice message";
      else if (contentType === "IMAGE") text = "[Image]";
      else if (contentType === "VIDEO") text = "[Video]";
      else if (contentType === "FILE") {
        text = meta.FILE_NAME || meta.FILENAME || "[File]";
      } else if (contentType === "STICKER") text = "[Sticker]";
      else if (contentType !== "NONE") text = `[${contentType}]`;
    }
    return {
      reactions: normalizeReactions(r.reactions, reactions.learnLayout),
      reactionsAt: Date.now(),
      replyTo: replyToOf(r),
      mentions: !callHistory && contentType === "NONE"
        ? lineMentions(text, meta)
        : [],
      id: String(r.id ?? ""),
      text: callHistory ? "" : text,
      emojis: !callHistory && contentType === "NONE"
        ? lineEmojis(text, meta)
        : [],
      callHistory,
      from: String(r.from ?? ""),
      to: String(r.to ?? ""),
      mine: isMineFrom(r.from),
      createdTime: Number(r.createdTime ?? 0),
      contentType: callHistory ? "CALL" : contentType,
      imagePath: null,
      imageUrl: meta.PREVIEW_URL || meta.DOWNLOAD_URL || null,
      audioPath: null,
      fileName: meta.FILE_NAME || meta.FILENAME || null,
      filePath: null,
      stickerId: meta.STKID || null,
      stickerPackageId: meta.STKPKGID || null,
      flex: contentType === "FLEX" ? extractFlex(meta) : null,
      durationMs: callHistory?.durationMs ??
        (meta.DURATION ? Number(meta.DURATION) : null),
      needsMedia: contentType === "IMAGE" || contentType === "VIDEO" ||
        contentType === "STICKER" || contentType === "AUDIO",
    };
  }
}

async function hydrateMedia(
  messages: Json[],
  chatMid: string,
  opts: { isolated?: boolean } = {},
) {
  if (!client || !stdoutAlive) return;
  messages = await unsends.mask(chatMid, messages);
  // Live listen hydrates must not bump the chat epoch — a concurrent fetch would
  // cancel the download and leave the bubble stuck until restart.
  const isolated = !!opts.isolated;
  const epoch = isolated ? -1 : bumpMediaEpoch(chatMid);
  const epochOk = () => isolated || mediaEpoch.get(chatMid) === epoch;
  const gen = workGen;
  const pending = messages
    .filter((m) => m.needsMedia && m.id)
    .slice()
    .reverse(); // newest first
  if (!pending.length) return;

  const first = pending.slice(0, MEDIA_FIRST_BATCH);
  const rest = pending.slice(MEDIA_FIRST_BATCH);

  const failOne = (id: string) => {
    if (!stdoutAlive || workGen !== gen) return;
    if (!epochOk()) return;
    emitEvent("media_failed", { chatMid, messageId: id });
  };

  const work = async (m: Json) => {
    if (!client || !stdoutAlive || workGen !== gen) return;
    if (!epochOk()) return;
    try {
      const id = String(m.id);
      const ct = normType(m.contentType);
      let path: string | null = null;

      if (ct === "STICKER") {
        const stkId = String(m.stickerId || "");
        if (!stkId) {
          failOne(id);
          return;
        }
        if (!epochOk()) return;
        path = await stickers.ensureImage(stkId);
        if (path) {
          emitEvent("media_ready", { chatMid, messageId: id, imagePath: path });
        } else {
          failOne(id);
        }
        return;
      }

      if (ct === "AUDIO") {
        let audioPath: string | null = null;
        for (const ext of ["m4a", "mp3", "aac", "ogg"]) {
          const p = mediaDest(id, ext);
          if (await isValidAudioFile(p)) {
            audioPath = p;
            break;
          }
          try {
            await Deno.remove(p);
          } catch { /* ignore */ }
        }
        if (!audioPath) {
          const got = await downloadAudioBytes(id, chatMid);
          if (got) {
            const ext = got.mime.includes("mpeg") || got.mime.includes("mp3")
              ? "mp3"
              : got.mime.includes("aac")
              ? "aac"
              : got.mime.includes("ogg")
              ? "ogg"
              : "m4a";
            const dest = mediaDest(id, ext);
            await Deno.writeFile(dest, got.buf);
            if (await isValidAudioFile(dest)) audioPath = dest;
            else {
              try {
                await Deno.remove(dest);
              } catch { /* ignore */ }
            }
          }
        }
        if (audioPath) {
          m.audioPath = audioPath;
          m.needsMedia = false;
          const cached = msgCache.get(chatMid);
          if (cached) {
            const row = cached.messages.find((x) => String(x.id) === id);
            if (row) {
              row.audioPath = audioPath;
              row.needsMedia = false;
            }
          }
          emitEvent("media_ready", {
            chatMid,
            messageId: id,
            imagePath: null,
            audioPath,
          });
        } else {
          failOne(id);
        }
        return;
      }

      if (ct === "FILE") {
        // No bubble preview; stop retry loops.
        m.needsMedia = false;
        const cached = msgCache.get(chatMid);
        if (cached) {
          const row = cached.messages.find((x) => String(x.id) === id);
          if (row) row.needsMedia = false;
        }
        return;
      }

      path = await existingFullImage(id);

      if (!path && typeof m.imageUrl === "string" && m.imageUrl) {
        path = await cacheUrl(m.imageUrl, fullDest(id, "jpg"));
        if (path && (await isPreviewOrThumbImage(path))) {
          try {
            await Deno.copyFile(path, thumbDest(id));
            await Deno.remove(path);
          } catch { /* ignore */ }
          path = null;
        }
      }

      // OBS preview is UI-only — never write it over the full media slot.
      if (!path && client) {
        try {
          const file = await client.base.obs.downloadMessageData({
            messageId: id,
            isPreview: true,
            isSquare: false,
          });
          if (!epochOk()) return;
          const buf = new Uint8Array(await file.arrayBuffer());
          if (buf.length > 32 && isImageBytes(buf)) {
            await writeImageFile(thumbDest(id), buf);
          }
        } catch (e) {
          console.error("[hydrateMedia obs preview]", id, e);
        }
      }

      // Full original: TalkMessage.getData (E2EE-aware)
      if (!path) {
        const got = await refetchMessageData(chatMid, id, {
          allowNonImage: ct === "VIDEO",
        });
        if (got) {
          if (ct === "VIDEO") {
            path = await outgoingMedia.materializeVideoPreview(id, got.buf);
          } else if (isImageBytes(got.buf)) {
            path = await writeFullImage(id, got.buf, got.mime);
          }
        }
      }

      if (path && epochOk()) {
        // If hydrate somehow still got a preview, demote it to thumb and keep downloading.
        if (ct !== "VIDEO" && (await isPreviewOrThumbImage(path))) {
          try {
            await Deno.copyFile(path, thumbDest(id));
            if (!path.includes(".thumb.") && !path.includes(".full.")) {
              await Deno.remove(path);
            }
          } catch { /* ignore */ }
          const got = await refetchMessageData(chatMid, id, {
            allowNonImage: false,
          });
          if (got && isImageBytes(got.buf)) {
            path = await writeFullImage(id, got.buf, got.mime);
          } else {
            // Keep preview thumb rather than failing the bubble.
            path = await existingImage(thumbDest(id));
          }
        }
      }

      if (path && epochOk()) {
        const uiPath = await uiMediaPath(id, path);
        m.imagePath = uiPath || path;
        m.needsMedia = false;
        // Keep full-resolution path for viewer/download (ui path may be a thumb).
        if (ct === "VIDEO") {
          const vpath = await existingFile(mediaDest(id, "mp4"));
          if (vpath) m.filePath = vpath;
        } else {
          m.filePath = path.includes(".thumb.") ? null : path;
        }
        const cached = msgCache.get(chatMid);
        if (cached) {
          const row = cached.messages.find((x) => String(x.id) === id);
          if (row) {
            row.imagePath = m.imagePath;
            row.needsMedia = false;
            if (m.filePath) row.filePath = m.filePath;
          }
        }
        emitEvent("media_ready", {
          chatMid,
          messageId: id,
          imagePath: m.imagePath,
          filePath: m.filePath ?? (ct === "VIDEO" ? null : path),
        });
        await sleep(30);
      } else if (epochOk()) {
        // Still show thumb in UI if we have one; mark failed only when nothing.
        const thumb = await existingImage(thumbDest(id));
        if (thumb) {
          m.imagePath = thumb;
          m.needsMedia = false;
          emitEvent("media_ready", {
            chatMid,
            messageId: id,
            imagePath: thumb,
            filePath: null,
          });
        } else {
          failOne(id);
        }
      }
    } catch (e) {
      console.error("[hydrateMedia worker]", e);
      try {
        failOne(String(m.id));
      } catch { /* ignore */ }
    }
  };

  await mapPool(first, MEDIA_CONCURRENCY, (m) => work(m));
  if (!epochOk()) return;
  await sleep(120);
  await mapPool(rest, MEDIA_CONCURRENCY, (m) => work(m));
}

async function hydratePreviews(chats: ChatRow[]) {
  if (!client || !stdoutAlive) return;
  const gen = workGen;
  const currentClient = client;
  const active = () =>
    client === currentClient && stdoutAlive && workGen === gen;
  await chatPreviews.hydrate(chats, async (chat) => {
    const cached = msgCache.get(chat.mid)?.messages ??
      await loadDiskMessages(chat.mid);
    if (!active()) return [];
    if (
      cached?.some((message) =>
        Number(message.createdTime ?? 0) >= chat.lastActivity
      ) && !cached.some(needsDecryptionRefresh)
    ) return await unsends.mask(chat.mid, cached);

    let cursor = boxCursor.get(chat.mid);
    if (!cursor) {
      const boxes = await currentClient.base.talk.getMessageBoxes({
        messageBoxListRequest: {},
      });
      if (!active()) return [];
      for (const box of boxes.messageBoxes ?? []) {
        storeBoxCursor(String(box.id ?? ""), box.lastDeliveredMessageId);
      }
      cursor = boxCursor.get(chat.mid);
    }
    if (!cursor) return [];
    const messages = await currentClient.base.talk
      .getPreviousMessagesV2WithRequest({
        request: {
          messageBoxId: chat.mid,
          endMessageId: cursor,
          messagesCount: 1,
        },
      });
    if (!active()) return [];
    return await unsends.mask(
      chat.mid,
      await Promise.all(
        messages.map((raw) => summarizeRawMessage(raw, { withMedia: false })),
      ),
    );
  }, active);
}

async function buildFriendsList(): Promise<ChatRow[]> {
  const friends: ChatRow[] = [];
  for (const [mid, info] of contactIndex) {
    // Users / bots / OA use u… mids; skip groups (c…) and rooms (r…).
    if (!mid.startsWith("u") || contactBlocks.isBlocked(mid)) continue;
    friends.push({
      mid,
      name: info.name || mid,
      kind: info.kind || "dm",
      picturePath: info.picturePath,
      avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
      lastActivity: 0,
      unread: 0,
      preview: "",
      muted: !!info.muted,
    });
  }
  friends.sort((a, b) =>
    a.name.localeCompare(b.name) || a.mid.localeCompare(b.mid)
  );
  return friends;
}

async function doListFriends(id: number | string | null, force = false) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  await refreshCachePolicy();
  try {
    await loadDiskContacts();
    const cached = await buildFriendsList();
    const stale = force ||
      contactIndex.size === 0 ||
      Date.now() - contactsAt >= cachePolicy().contactsRefresh;

    // Stale-while-revalidate: show disk/memory cache immediately.
    if (cached.length > 0) {
      ok(id, { friends: cached, count: cached.length, cached: true });
      hydrateAvatars(cached);
      if (stale) {
        refreshContactIndex(force).then(async () => {
          const friends = await buildFriendsList();
          emitEvent("friends_updated", { friends, count: friends.length });
          hydrateAvatars(friends);
        }).catch((e) => console.error("[friends refresh]", e));
      }
      return;
    }

    await refreshContactIndex(true);
    const friends = await buildFriendsList();
    ok(id, { friends, count: friends.length, cached: false });
    hydrateAvatars(friends);
  } catch (e) {
    fail(id, e instanceof Error ? e.message : String(e));
  }
}

async function hydrateAvatars(chats: ChatRow[]) {
  const need = chats.filter((c) => c.picturePath && !c.avatarPath);
  await mapPool(need, AVATAR_CONCURRENCY, async (c) => {
    const path = await avatarPathFor(c.mid, c.picturePath);
    if (!path) return;
    c.avatarPath = path;
    if (chatCache) {
      const row = chatCache.chats.find((x) => x.mid === c.mid);
      if (row) row.avatarPath = path;
    }
    emitEvent("avatar_ready", { mid: c.mid, avatarPath: path });
  });
  if (chatCache) {
    try {
      await saveChatCache();
    } catch (error) {
      console.error("[chat-avatar-cache]", error);
    }
  }
}

function touchChatPreviewFromMessage(message: Json) {
  void upsertChatFromMessage(message);
}

async function resolveContactInfo(mid: string): Promise<{
  name: string;
  picturePath: string | null;
  kind: string;
  muted: boolean;
}> {
  const cached = contactIndex.get(mid);
  if (cached?.name && cached.name !== mid) return cached;
  if (!client) {
    return cached ?? {
      name: mid,
      picturePath: null,
      kind: mid.startsWith("c") ? "group" : "dm",
      muted: false,
    };
  }
  try {
    if (mid.startsWith("c")) {
      const raw = await client.base.talk.getChat({
        chatMid: mid,
        withMembers: false,
        withInvitees: false,
      });
      const info = groupProfile(raw);
      if (info) {
        contactIndex.set(mid, info);
        await saveDiskContacts();
        return info;
      }
    }
    const c = await client.base.talk.getContact({ mid });
    const info = {
      name: c.displayName || mid,
      picturePath: c.picturePath ?? null,
      kind: mid.startsWith("c") ? "group" : contactKind(c, cached?.kind),
      muted: !!(c as { notificationDisabled?: boolean }).notificationDisabled,
    };
    contactIndex.set(mid, info);
    await saveDiskContacts();
    return info;
  } catch {
    return cached ?? {
      name: mid,
      picturePath: null,
      kind: mid.startsWith("c") ? "group" : "dm",
      muted: false,
    };
  }
}

async function upsertChatFromMessage(message: Json) {
  const peer = conversationMid(message);
  if (!peer) return;
  const [visible] = await unsends.mask(peer, [message]);
  if (!visible || visible.contentType === "UNSENT") return;
  message = visible;
  const preview = previewLine(message);
  const activity = Number(message.createdTime ?? 0);
  if (!chatCache) chatCache = { at: Date.now(), chats: [] };

  let row = chatCache.chats.find((x) => x.mid === peer);
  let created = false;
  if (!row) {
    const info = await resolveContactInfo(peer);
    row = {
      mid: peer,
      name: info.name,
      kind: info.kind,
      picturePath: info.picturePath,
      avatarPath: await existingFile(join(avatarDir, `${peer}.jpg`)),
      lastActivity: activity,
      unread: 0,
      preview,
    };
    created = true;
  } else {
    row.preview = preview;
    if (activity > row.lastActivity) row.lastActivity = activity;
    if (!row.name || row.name === peer) {
      const info = await resolveContactInfo(peer);
      row.name = info.name;
      row.kind = info.kind || row.kind;
      row.picturePath = info.picturePath ?? row.picturePath ?? null;
    }
  }

  row.contactOnly = false;
  chatCache.chats = [row, ...chatCache.chats.filter((c) => c.mid !== peer)];
  chatCache.at = Date.now();
  try {
    await saveChatCache();
  } catch (error) {
    console.error("[chat-message-cache]", peer, error);
  }

  emitEvent("chat_upsert", { chat: row, created });
  emitEvent("chat_preview", { mid: peer, preview, lastActivity: activity });
  if (row.picturePath && !row.avatarPath) {
    hydrateAvatars([row]);
  }
}

async function upsertChatFromContact(mid: string) {
  if (!mid || (!mid.startsWith("u") && !mid.startsWith("c"))) return;
  const info = await resolveContactInfo(mid);
  const avatarPath = await existingFile(join(avatarDir, `${mid}.jpg`));
  if (!chatCache) chatCache = { at: Date.now(), chats: [] };
  const previous = chatCache.chats.find((x) => x.mid === mid);
  const created = !previous;
  const row = contactChatRow(
    mid,
    info,
    previous,
    avatarPath,
  );
  if (previous) {
    Object.assign(previous, row);
  } else {
    chatCache.chats.push(row);
  }
  chatCache.at = Date.now();
  try {
    await saveChatCache();
  } catch (error) {
    console.error("[chat-contact-cache]", mid, error);
  }
  emitEvent("chat_upsert", { chat: row, created });
  if (row.picturePath && !row.avatarPath) hydrateAvatars([row]);
}

/**
 * Hydrate media for a just-received TalkMessage using tm.getData() directly.
 * refetchMessageData() walks recent history and often misses the brand-new id.
 */
async function hydrateLiveMedia(
  tm: TalkMessage,
  message: Json,
  chatMid: string,
) {
  if (!client || !stdoutAlive) return;
  const id = String(message.id ?? "");
  const ct = normType(message.contentType);
  if (!id) return;

  if (ct === "STICKER" || ct === "AUDIO" || ct === "FILE") {
    await hydrateMedia([message], chatMid, { isolated: true });
    return;
  }

  const fail = () => {
    if (!stdoutAlive) return;
    emitEvent("media_failed", { chatMid, messageId: id });
  };

  try {
    let path = await existingFullImage(id);

    if (!path) {
      try {
        const blob = await tm.getData(false);
        const buf = new Uint8Array(await blob.arrayBuffer());
        if (ct === "VIDEO") {
          path = await outgoingMedia.materializeVideoPreview(id, buf);
        } else if (isImageBytes(buf)) {
          path = await writeFullImage(id, buf, blob.type || "");
        } else if (buf.length > 1024) {
          // Some video payloads lack an image magic header.
          path = await outgoingMedia.materializeVideoPreview(id, buf);
        }
      } catch (e) {
        console.error("[hydrateLive getData]", id, e);
      }
    }

    if (!path && typeof message.imageUrl === "string" && message.imageUrl) {
      path = await cacheUrl(String(message.imageUrl), fullDest(id, "jpg"));
      if (path && (await isPreviewOrThumbImage(path))) {
        try {
          await Deno.copyFile(path, thumbDest(id));
          await Deno.remove(path);
        } catch { /* ignore */ }
        path = null;
      }
    }

    if (!path && client) {
      try {
        const file = await client.base.obs.downloadMessageData({
          messageId: id,
          isPreview: true,
          isSquare: false,
        });
        const buf = new Uint8Array(await file.arrayBuffer());
        if (buf.length > 32 && isImageBytes(buf)) {
          await writeImageFile(thumbDest(id), buf);
        }
      } catch (e) {
        console.error("[hydrateLive obs preview]", id, e);
      }
    }

    if (!path) {
      const got = await refetchMessageData(chatMid, id, {
        allowNonImage: ct === "VIDEO",
      });
      if (got) {
        if (ct === "VIDEO") {
          path = await outgoingMedia.materializeVideoPreview(id, got.buf);
        } else if (isImageBytes(got.buf)) {
          path = await writeFullImage(id, got.buf, got.mime);
        }
      }
    }

    if (!path) {
      path = await existingImage(thumbDest(id));
    }

    if (!path) {
      fail();
      return;
    }

    const uiPath = await uiMediaPath(id, path);
    const imagePath = uiPath || path;
    let filePath: string | null = null;
    if (ct === "VIDEO") {
      filePath = await existingFile(mediaDest(id, "mp4"));
    } else if (!path.includes(".thumb.")) {
      filePath = path;
    }

    message.imagePath = imagePath;
    message.needsMedia = false;
    if (filePath) message.filePath = filePath;

    const cached = msgCache.get(chatMid);
    if (cached) {
      const row = cached.messages.find((x) => String(x.id) === id);
      if (row) {
        row.imagePath = imagePath;
        row.needsMedia = false;
        if (filePath) row.filePath = filePath;
      }
    }

    emitEvent("media_ready", {
      chatMid,
      messageId: id,
      imagePath,
      filePath,
    });
  } catch (e) {
    console.error("[hydrateLiveMedia]", id, e);
    fail();
  }
}

function sentMessagePayload(
  sent:
    | { id?: unknown; createdTime?: unknown; contentType?: unknown }
    | null
    | undefined,
  chatMid: string,
  text: string,
): Json {
  return {
    id: String(sent?.id ?? Date.now()),
    text,
    from: myMid(),
    to: chatMid,
    mine: true,
    createdTime: Number(sent?.createdTime ?? Date.now()),
    contentType: normType(sent?.contentType ?? "NONE"),
    imagePath: null,
    imageUrl: null,
    audioPath: null,
    stickerId: null,
    stickerPackageId: null,
    flex: null,
    durationMs: null,
    needsMedia: false,
  };
}

async function handle(req: Json) {
  const id = (req.id as number | string | null) ?? null;
  const method = String(req.method ?? "");
  const params = (req.params ?? {}) as Json;

  try {
    if (
      [
        "send_message",
        "send_audio",
        "send_media",
        "send_sticker",
        "send_postback",
        "poll_create",
        "react_message",
        "call_start",
      ].includes(method) &&
      contactBlocks.isBlocked(String(
        method === "call_start"
          ? params.mid ?? params.chatMid ?? ""
          : params.chatMid ?? "",
      ))
    ) {
      throw new Error("contact_blocked_send");
    }
    switch (method) {
      case "unsend_message":
        ok(
          id,
          await unsends.unsend(
            String(params.chatMid ?? ""),
            params.messageId,
            Number(params.createdTime ?? 0),
          ),
        );
        break;
      case "react_message":
        ok(
          id,
          await reactions.react(
            String(params.chatMid ?? ""),
            params.messageId,
            params.reaction,
          ),
        );
        break;
      case "cancel_reaction":
        ok(
          id,
          await reactions.cancel(
            String(params.chatMid ?? ""),
            params.messageId,
          ),
        );
        break;
      case "reaction_emoji_packs":
        ok(id, await reactionEmojiPacks.packs());
        break;
      case "reaction_emoji_pack":
        ok(id, await reactionEmojiPacks.pack(params.productId));
        break;
      case "reaction_emojis": {
        const emojis = await reactions.recentEmojis();
        emojiService.observe([{ emojis }]);
        ok(id, { emojis });
        break;
      }
      case "get_blocked_contacts":
        ok(id, await contactBlocks.refresh());
        break;
      case "get_user_profile":
        ok(id, await userProfiles.get(params.mid, params.force === true));
        break;
      case "set_contact_blocked":
        if (typeof params.blocked !== "boolean") {
          throw new Error("contact_blocks_invalid_user");
        }
        ok(
          id,
          await contactBlocks.set(String(params.mid ?? ""), params.blocked),
        );
        break;
      case "ping":
        ok(id, { pong: true, version: "0.2.0" });
        break;
      case "set_language": {
        const language = String(params.language ?? "");
        if (!["vi", "en", "th", "ja"].includes(language)) {
          throw new Error("表示言語が不正です");
        }
        Deno.env.set("LINE_GTK_LANG", language);
        ok(id, { language });
        break;
      }
      case "login_qr":
        bootRestoreController?.abort();
        await commands.loginQr(id);
        break;
      case "login_email": {
        bootRestoreController?.abort();
        const email = typeof params.email === "string" ? params.email : "";
        const password = typeof params.password === "string"
          ? params.password
          : "";
        delete params.password;
        try {
          await commands.loginEmail(id, email, password);
        } catch (error) {
          const code = error instanceof Error ? error.message : "";
          fail(
            id,
            [
                "login_email_missing",
                "login_invalid_email",
                "login_invalid_password",
                "login_email_failed",
                "login_email_timeout",
                "login_keys_missing",
              ].includes(code)
              ? code
              : emailLoginError(error),
          );
        }
        break;
      }
      case "login_token":
        bootRestoreController?.abort();
        await commands.loginToken(id, params.token as string | undefined);
        break;
      case "search_messages":
      case "fetch_search_message_context": {
        const chatMid = String(params.chatMid ?? "");
        if (!/^[ucr][A-Za-z0-9_-]{1,128}$/.test(chatMid)) {
          throw new Error("invalid_chat");
        }
        const owner = sharedSession();
        if (!owner) throw new Error("not_logged_in");
        const messages = await loadSearchHistory({
          session: sharedSession,
          disk: async () => {
            const raw = await readMessageFile(msgDiskDir, chatMid, cacheKey);
            return {
              accountMid: raw.accountMid,
              messages: Array.isArray(raw.messages)
                ? restoreCachedMessages(raw.messages as Json[])
                : [],
            };
          },
          memory: () => msgCache.get(chatMid)?.messages ?? [],
          mask: (messages) => unsends.mask(chatMid, messages),
        });
        let result: Json;
        if (method === "search_messages") {
          const page = searchMessages(
            messages,
            String(params.query ?? ""),
            Number(params.offset ?? 0),
          );
          result = {
            ...page,
            chatMid,
            messages: await messageView.forUiMessages(page.messages),
          };
        } else {
          const target = String(params.messageId ?? "");
          result = {
            chatMid,
            targetMessageId: target,
            messages: await forUiMessages(
              searchContext(messages, target),
              chatMid,
            ),
            hasMoreOlder: true,
          };
        }
        if (sharedSession()?.owner !== owner.owner) {
          throw new Error("message_search_cancelled");
        }
        ok(id, result);
        break;
      }
      case "load_cached_messages": {
        const chatMid = String(params.chatMid ?? "");
        const limit = Math.max(1, Math.min(Number(params.limit ?? 40), 200));
        await refreshCachePolicy();
        // 通信前の表示用。接続後は通常の fetch_messages が最新へ更新する。
        const stored = await loadBootMessages(
          msgDiskDir,
          chatMid,
          myMid() || bootAccountMid,
          cachePolicy().diskMsg,
          cacheKey,
        );
        ok(id, {
          chatMid,
          messages: stored
            ? await messageView.forUiMessages(
              restoreCachedMessages(stored).slice(-limit),
            )
            : [],
        });
        break;
      }
      case "trim_message_cache": {
        const chatMid = String(params.chatMid ?? "");
        if (!/^[A-Za-z0-9_-]{1,128}$/.test(chatMid)) {
          fail(id, "invalid_chat");
          break;
        }
        try {
          const keep = Math.floor(Number(params.keep ?? 0));
          if (!Number.isFinite(keep) || keep < 0) throw new Error("bad_keep");
          ok(id, { chatMid, removed: await trimDiskMessages(chatMid, keep) });
        } catch {
          fail(id, "trim_failed");
        }
        break;
      }
      case "list_chats":
        if (params.reconnect) {
          await connectionRecovery.reconnect();
          if (!client) throw new Error("not_logged_in");
          await waitForReceiver(client);
        }
        await commands.listChats(id, !!params.force, !!params.quiet);
        break;
      case "list_shared_media":
      case "download_shared_media":
      case "stop_shared_media":
      case "fetch_shared_message_context": {
        try {
          const chatMid = String(params.chatMid ?? "");
          if (method === "list_shared_media") {
            ok(
              id,
              await sharedMedia.list(
                chatMid,
                String(params.category ?? "photos_videos"),
                String(params.cursor ?? ""),
                Number(params.limit ?? 60),
                !!params.refresh,
              ),
            );
          } else if (method === "download_shared_media") {
            ok(
              id,
              await sharedMedia.download(
                chatMid,
                String(params.messageId ?? ""),
                !!params.retry,
              ),
            );
          } else if (method === "stop_shared_media") {
            await sharedMedia.pause(chatMid);
            ok(id, { paused: true });
          } else {
            const owner = sharedSession();
            const target = String(params.messageId ?? "");
            const source = await sharedMedia.source(chatMid, target);
            if (!source) throw new Error("shared_media_unavailable");
            let preceding: SharedMediaSource[] = [];
            try {
              preceding = await sharedHistoryPage(chatMid, {
                messageId: target,
                createdTime: Number(source.message.createdTime) || 0,
              });
            } catch {
              // Cached source remains navigable while offline.
            }
            if (!owner || sharedSession()?.owner !== owner.owner) {
              throw new Error("shared_media_cancelled");
            }
            const targetMessage = { ...source.message };
            if (source.attachment) {
              targetMessage.filePath = source.attachment.path;
              targetMessage.imagePath = source.attachment.thumbnailPath ?? null;
            }
            const messages = preceding.map((item) => item.message)
              .filter((message) => String(message.id) !== target);
            messages.sort((a, b) =>
              Number(a.createdTime) - Number(b.createdTime)
            );
            const context = await forUiMessages([
              ...messages.slice(-49),
              targetMessage,
            ], chatMid);
            if (sharedSession()?.owner !== owner.owner) {
              throw new Error("shared_media_cancelled");
            }
            ok(id, {
              chatMid,
              targetMessageId: target,
              messages: context,
              hasMoreOlder: true,
            });
          }
        } catch (error) {
          const code = error instanceof Error ? error.message : "";
          fail(
            id,
            /^shared_media_[a-z_]+$/.test(code)
              ? code
              : "shared_media_request_failed",
          );
        }
        break;
      }
      case "list_albums":
      case "list_album_photos":
      case "download_album_photo": {
        try {
          const chatMid = String(params.chatMid ?? "");
          const albumId = String(params.albumId ?? "");
          const result = method === "list_albums"
            ? await albums.listAlbums(chatMid, params.cursor)
            : method === "list_album_photos"
            ? await albums.listPhotos(chatMid, albumId, params.cursor)
            : await albums.downloadPhoto(
              chatMid,
              albumId,
              String(params.photoId ?? ""),
            );
          ok(id, result);
        } catch (error) {
          const code = error instanceof Error ? error.message : "";
          fail(
            id,
            /^album_[a-z_]+$/.test(code) ? code : "album_request_failed",
          );
        }
        break;
      }
      case "fetch_older_messages":
        await commands.fetchOlderMessages(
          id,
          String(params.chatMid ?? ""),
          String(params.beforeId ?? ""),
          Number(params.beforeTime ?? 0),
          params.limit,
        );
        break;
      case "fetch_newer_messages":
        await commands.fetchNewerCachedMessages(
          id,
          String(params.chatMid ?? ""),
          String(params.afterId ?? ""),
          Number(params.afterTime ?? 0),
          params.limit,
        );
        break;
      case "fetch_messages":
        await commands.fetchMessages(
          id,
          String(params.chatMid ?? ""),
          Number(params.limit ?? 50),
          !!params.force,
          !!params.quiet,
        );
        break;
      case "get_chat_members":
        await commands.mentionMembers(id, String(params.chatMid ?? ""));
        break;
      case "send_message":
        await commands.sendMessage(
          id,
          String(params.chatMid ?? ""),
          String(params.text ?? ""),
          params.replyToMessageId,
          params.mentions,
        );
        break;
      case "send_audio":
        await outgoingMedia.sendAudio(
          id,
          String(params.chatMid ?? ""),
          String(params.filePath ?? ""),
          params.durationMs != null ? Number(params.durationMs) : undefined,
        );
        break;
      case "send_media":
        await outgoingMedia.sendMedia(
          id,
          String(params.chatMid ?? ""),
          String(params.filePath ?? ""),
          typeof params.oType === "string" ? params.oType : "auto",
          params.durationMs != null ? Number(params.durationMs) : undefined,
        );
        break;
      case "download_media":
        await outgoingMedia.downloadMedia(
          id,
          String(params.chatMid ?? ""),
          String(params.messageId ?? ""),
          String(params.contentType ?? ""),
        );
        break;
      case "send_sticker":
        await stickers.send(
          id,
          String(params.chatMid ?? ""),
          String(params.stickerId ?? ""),
          String(params.packageId ?? params.stickerPackageId ?? ""),
          typeof params.version === "string" ? params.version : undefined,
        );
        break;
      case "list_stickers":
        if (params.refreshOwned === true) stickers.resetOwnedCache();
        await stickers.list(
          id,
          typeof params.importPack === "string" ? params.importPack : undefined,
        );
        break;
      case "list_sticker_library":
        ok(id, await stickerLibrary.list());
        break;
      case "prepare_sticker_image":
        ok(
          id,
          await stickerLibrary.prepareImage(
            String(id),
            String(params.optionKey ?? ""),
            params.options,
            typeof params.inputPath === "string" ? params.inputPath : undefined,
          ),
        );
        break;
      case "set_sticker_optimization":
        ok(
          id,
          await stickerLibrary.setOptimization(
            String(params.optionKey ?? ""),
            params.options,
          ),
        );
        break;
      case "preview_sticker_pack":
        ok(id, await stickerLibrary.preview(String(params.link ?? "")));
        break;
      case "inspect_sticker_pngs":
        ok(
          id,
          await stickerLibrary.inspectPngs(
            Array.isArray(params.paths) ? params.paths.map(String) : [],
          ),
        );
        break;
      case "import_sticker_pack":
        ok(
          id,
          await stickerLibrary.importStore(
            String(id),
            String(params.link ?? ""),
          ),
        );
        break;
      case "import_sticker_pngs":
        ok(
          id,
          await stickerLibrary.importPngs(
            String(id),
            Array.isArray(params.paths) ? params.paths.map(String) : [],
            String(params.name ?? ""),
            typeof params.packId === "string" ? params.packId : undefined,
            Array.isArray(params.digests)
              ? params.digests.map(String)
              : undefined,
          ),
        );
        break;
      case "update_sticker_library":
        ok(
          id,
          await stickerLibrary.update(
            String(params.packId ?? ""),
            String(params.action ?? ""),
            String(params.value ?? ""),
          ),
        );
        break;
      case "cancel_sticker_import":
        ok(id, stickerLibrary.cancel(String(params.requestId ?? "")));
        break;
      case "adopt_sticker_library":
        ok(id, await stickerLibrary.adoptLegacy());
        break;
      case "sticker_owned_pack":
        await stickers.ownedPack(id, String(params.packageId ?? ""));
        break;
      case "mark_read":
        await commands.markRead(
          id,
          String(params.chatMid ?? ""),
          String(params.lastMessageId ?? ""),
        );
        break;
      case "mark_read_once":
        await commands.markReadOnce(
          id,
          String(params.chatMid ?? ""),
          String(params.lastMessageId ?? ""),
        );
        break;
      case "call_start":
        calls.setCallAudioDevices(
          typeof params.audioInput === "string" ? params.audioInput : undefined,
          typeof params.audioOutput === "string"
            ? params.audioOutput
            : undefined,
        );
        calls.setCallGains(params.micGain, params.spkGain);
        await calls.doCallStart(id, String(params.mid ?? params.chatMid ?? ""));
        break;
      case "call_answer":
        calls.setCallAudioDevices(
          typeof params.audioInput === "string" ? params.audioInput : undefined,
          typeof params.audioOutput === "string"
            ? params.audioOutput
            : undefined,
        );
        calls.setCallGains(params.micGain, params.spkGain);
        await calls.doCallAnswer(id);
        break;
      case "call_decline":
        await calls.doCallDecline(id);
        break;
      case "call_end":
        await calls.doCallEnd(id);
        break;
      case "call_set_audio": {
        ok(id, calls.updateCallAudio(params));
        break;
      }
      case "poll_get": {
        try {
          ok(
            id,
            await linePolls.get(
              String(params.chatMid ?? ""),
              String(params.pollId ?? ""),
            ),
          );
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "poll_create": {
        try {
          const draft = (params.poll ?? {}) as Record<string, unknown>;
          ok(
            id,
            await linePolls.create(String(params.chatMid ?? ""), {
              title: String(draft.title ?? ""),
              choices: Array.isArray(draft.choices)
                ? draft.choices.map(String)
                : [],
              multiple: draft.multiple === true,
              anonymous: draft.anonymous === true,
              allowNewChoices: draft.allowNewChoices === true,
              closeDate: typeof draft.closeDate === "number"
                ? draft.closeDate
                : null,
            }),
          );
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "poll_vote": {
        try {
          ok(
            id,
            await linePolls.vote(
              String(params.chatMid ?? ""),
              String(params.pollId ?? ""),
              Array.isArray(params.choiceIds)
                ? params.choiceIds.map(String)
                : [],
            ),
          );
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "send_postback": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const chatMid = String(params.chatMid ?? "");
        const messageId = String(params.messageId ?? "");
        const data = String(params.data ?? "");
        const uri = params.uri != null ? String(params.uri) : "";
        try {
          await client.base.talk.sendPostback({
            request: {
              messageId,
              url: uri || `line://postback?data=${encodeURIComponent(data)}`,
              chatMID: chatMid,
              originMID: myMid(),
            },
          });
          ok(id, { sent: true });
        } catch (e) {
          // Fallback: send the data/label as a normal chat message.
          try {
            const text = data || uri || "OK";
            try {
              const chat = await client!.getChat(chatMid);
              await chat.sendMessage(text);
            } catch {
              await client!.base.talk.sendMessage({
                to: chatMid,
                text,
                e2ee: true,
              });
            }
            msgCache.delete(chatMid);
            ok(id, { sent: true, fallback: "message" });
          } catch {
            fail(id, e instanceof Error ? e.message : String(e));
          }
        }
        break;
      }
      case "clear_cache": {
        const kind = String(params.kind ?? "all");
        // Settle/invalidate writers before the existing cache files are wiped.
        await sharedMedia.clear(kind);
        if (kind === "stickers" || kind === "all") {
          await stickerLibrary.clearPreviews();
        }
        const wipe = async (dir: string) => {
          try {
            for await (const e of Deno.readDir(dir)) {
              if (e.isFile) {
                try {
                  await Deno.remove(join(dir, e.name));
                } catch { /* ignore */ }
              }
            }
          } catch { /* ignore */ }
        };
        if (kind === "media" || kind === "all") await wipe(mediaDir);
        if (kind === "stickers" || kind === "all") await wipe(stickerDir);
        if (kind === "avatars" || kind === "all") await wipe(avatarDir);
        if (kind === "messages" || kind === "all") {
          await wipe(msgDiskDir);
          cacheWarned.clear();
          chatEvents.reset();
        }
        if (kind === "chats" || kind === "all") {
          try {
            await Deno.remove(chatCachePath);
          } catch { /* ignore */ }
          try {
            await Deno.remove(contactCachePath);
          } catch { /* ignore */ }
          chatCache = null;
          contactIndex.clear();
          contactsAt = 0;
        }
        if (kind === "messages" || kind === "all") msgCache.clear();
        ok(id, { cleared: kind });
        break;
      }
      case "list_friends": {
        await doListFriends(id, !!params.force);
        break;
      }
      case "add_friend": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const userid = String(params.userid ?? "").replace(/^@/, "");
        try {
          const findByUserId = client.base.talk
            .findContactByUserid as unknown as (
              args: { userid: string },
            ) => Promise<{
              mid?: string;
              displayName?: string;
              picturePath?: string;
            }>;
          const contact = await findByUserId({ userid });
          const mid = contact?.mid;
          if (!mid) {
            fail(id, "user_not_found");
            break;
          }
          try {
            const requestFriend = client.base.talk
              .tryFriendRequest as unknown as (
                args: {
                  mid: string;
                  method: string;
                  friendRequestParams: string;
                },
              ) => Promise<unknown>;
            await requestFriend({
              mid,
              method: "USERID",
              friendRequestParams: userid,
            });
          } catch {
            /* already friends / different API shape */
          }
          contactIndex.set(String(mid), {
            name: contact.displayName || userid,
            picturePath: contact.picturePath ?? null,
            kind: "dm",
            muted: false,
          });
          await saveDiskContacts();
          ok(id, {
            mid,
            displayName: contact.displayName ?? userid,
          });
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "mute_chat": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const mid = String(params.mid ?? params.chatMid ?? "");
        const muted = !!params.muted;
        if (!mid.startsWith("u") && !mid.startsWith("c")) {
          fail(id, "mute_unsupported_chat");
          break;
        }
        try {
          if (mid.startsWith("c")) {
            await setGroupMuted(
              client.base.talk,
              mid,
              muted,
              await client.base.getReqseq(),
            );
          } else {
            await client.base.talk.updateContactSetting({
              reqSeq: await client.base.getReqseq(),
              mid,
              flag: "CONTACT_SETTING_NOTIFICATION_DISABLE",
              value: muted ? "true" : "false",
            });
          }
          const cur = contactIndex.get(mid);
          if (cur) cur.muted = muted;
          else {
            contactIndex.set(mid, {
              name: mid,
              picturePath: null,
              kind: mid.startsWith("c") ? "group" : "dm",
              muted,
            });
          }
          await saveDiskContacts();
          if (chatCache) {
            const row = chatCache.chats.find((c) => c.mid === mid);
            if (row) row.muted = muted;
          }
          emitEvent("chat_mute", { mid, muted });
          ok(id, { mid, muted });
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "logout":
        bootRestoreController?.abort();
        cancelBackgroundWork();
        stickers.resetOwnedCache();
        try {
          await clearAuth();
        } catch { /* ignore */ }
        if (client) retireClient(client);
        client = null;
        listening = false;
        chatCache = null;
        msgCache.clear();
        bootAccountMid = "";
        try {
          await Deno.remove(bootSessionPath);
        } catch { /* ignore */ }
        ok(id, {});
        break;
      default:
        fail(id, `unknown_method:${method}`);
    }
  } catch (e) {
    fail(id, e instanceof Error ? e.message : String(e));
  }
}

await stickers.loadIndex();
await loadDiskContacts();

const bootAuth = Deno.env.get("LINE_GTK_SKIP_RESTORE") === "1"
  ? null
  : await loadAuth();
const bootDevice = await loadAuthDevice();

emitEvent("ready", {
  dataDir,
  sidecar: fromFileUrl(import.meta.url),
  hasAuth: !!bootAuth,
  device: bootDevice,
});

if (bootAuth) {
  await refreshCachePolicy();
  const cached = await loadBootCache(
    bootSessionPath,
    chatCachePath,
    cachePolicy().diskChat,
  );
  if (cached) {
    bootAccountMid = cached.accountMid;
    // 接続を待たず、前回の一覧とプロフィールを先に見せる。
    emitEvent("cached_boot", {
      accountMid: cached.accountMid,
      profile: { ...cached.profile, device: bootDevice },
      chats: cached.chats,
    });
  }
  const controller = new AbortController();
  bootRestoreController = controller;
  // Keep stdin responsive (including ping) while a server is unavailable.
  void (async () => {
    let auth = bootAuth;
    const device = bootDevice;
    try {
      const restored = await restoreWithRetry(async () => {
        const candidate = await restoreSession(
          auth,
          device,
          storagePath,
          LINE_VERSION,
          undefined,
          createLineFetch(25_000, globalThis.fetch, controller.signal),
        );
        controller.signal.throwIfAborted();
        // Save refreshed credentials before another request can fail.
        auth = candidate.authToken;
        await saveAuth(auth, device);
        const profile = await candidate.getMyProfile();
        controller.signal.throwIfAborted();
        return { candidate, profile };
      }, (attempt) => emitEvent("connection_retry", { attempt }));
      client = restored.candidate;
      patchE2eeGuards();
      listener.start();
      emitEvent("session", {
        ...(await myProfilePayload(restored.profile)),
        device,
      });
    } catch (error) {
      if (!controller.signal.aborted) {
        console.error("[boot restore]", error);
        await sessionFailure(error);
      }
    } finally {
      if (bootRestoreController === controller) bootRestoreController = null;
    }
  })();
}

const decoder = new TextDecoder();
let buf = "";
for await (const chunk of Deno.stdin.readable) {
  buf += decoder.decode(chunk, { stream: true });
  while (true) {
    const nl = buf.indexOf("\n");
    if (nl < 0) break;
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (!line) continue;
    try {
      handle(JSON.parse(line) as Json);
    } catch (e) {
      fail(null, `bad_json:${e}`);
    }
  }
}

/**
 * LINE protocol sidecar for line-gtk.
 * NDJSON over stdin/stdout. Heavy I/O is cached + async so GTK stays snappy.
 */

import {
  Client,
  loginWithAuthToken,
  SquareMessage,
  TalkMessage,
} from "@evex/linejs";
import { FileStorage } from "@evex/linejs/storage";
import { fromFileUrl, join } from "@std/path";
import { atomicWriteJson, ensurePrivateDir } from "./storage.ts";
import { AuthStore, LineDevice } from "./auth.ts";
import { CachePolicy, policyFor } from "./cache_policy.ts";
import { picturePathOf, profileUrl, squareObsUrl } from "./contacts.ts";
import {
  coerceI64,
  normalizeRaw,
  sticonResources,
  talkChatMid,
} from "./messages.ts";
import * as calls from "./calls.ts";
import { createOutgoingMedia } from "./outgoing_media.ts";
import { createMediaCache } from "./media_cache.ts";
import { createStickerService } from "./stickers.ts";
import { createCommandService } from "./commands.ts";
import { createListener } from "./listener.ts";
import { createMessageView } from "./message_view.ts";

type Json = Record<string, unknown>;

type ChatRow = {
  mid: string;
  name: string;
  kind: string;
  avatarPath: string | null;
  picturePath?: string | null;
  lastActivity: number;
  unread: number;
  preview: string;
  muted?: boolean;
  pinned?: boolean;
};

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
const msgDiskDir = join(cacheDir, "messages");
const MESSAGE_CACHE_SCHEMA = 3;
await ensurePrivateDir(msgDiskDir);

let cacheRetention = (Deno.env.get("LINE_GTK_CACHE_RETENTION") || "smart")
  .toLowerCase();
let cachePolicyState = policyFor(cacheRetention);
let cachePolicyCheckedAt = 0;

async function refreshCachePolicy() {
  if (Date.now() - cachePolicyCheckedAt < 3_000) return cachePolicyState;
  cachePolicyCheckedAt = Date.now();
  try {
    const raw = JSON.parse(
      await Deno.readTextFile(join(dataDir, "settings.json")),
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
const PREVIEW_CONCURRENCY = 2;
const PREVIEW_TOP = 28;
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

// linejs listen() async loops can reject without a caller catch (e.g. logged-out token).
let authDead = false;
globalThis.addEventListener("unhandledrejection", (ev) => {
  const reason = (ev as PromiseRejectionEvent).reason;
  let msg = "";
  if (reason instanceof Error) {
    msg = reason.message;
  } else if (typeof reason === "string") {
    msg = reason;
  } else {
    try {
      msg = JSON.stringify(reason ?? "");
    } catch {
      msg = String(reason ?? "unhandledrejection");
    }
  }
  const loggedOut = msg.includes("NOT_AUTHORIZED") ||
    msg.includes("LOGGED_OUT") ||
    msg.includes("AUTHENTICATION") ||
    msg.includes("V3_TOKEN_CLIENT_LOGGED_OUT") ||
    msg.includes("NOT_AUTHORIZED_DEVICE");
  if (loggedOut) {
    ev.preventDefault();
    if (authDead) return;
    authDead = true;
    listening = false;
    client = null;
    cancelBackgroundWork();
    console.error("[auth] session ended:", msg);
    void clearAuth();
    emitEvent("session_failed", { error: msg });
    return;
  }
  console.error("[unhandledrejection]", msg);
});

let client: Client | null = null;
let listening = false;

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
const { previewLang, previewLine, extractFlex, forUiMessages } = messageView;

const stickers = createStickerService({
  getClient: () => client,
  dataDir,
  stickerDir,
  refreshCachePolicy,
  cachePolicy,
  cacheUrl,
  existingImage,
  sentMessagePayload,
  invalidateMessages: markMessageCacheStale,
  invalidateChats: () => {
    chatCache = null;
  },
  touchChatPreviewFromMessage,
  emitEvent,
  ok,
  fail,
});

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
  invalidateMessages: markMessageCacheStale,
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
type MessageHistoryCursor = {
  messageId: string;
  deliveredTime: string;
};
type MessageCacheEntry = {
  at: number;
  messages: Json[];
  historyComplete?: boolean;
  oldestCursor?: MessageHistoryCursor;
  squareSyncToken?: string;
};
const msgCache = new Map<string, MessageCacheEntry>();
const inFlight = new Map<string, Promise<unknown>>();
const contactIndex = new Map<
  string,
  {
    name: string;
    picturePath: string | null;
    statusMessage: string;
    kind: string;
    muted: boolean;
  }
>();
const squareChatIndex = new Set<string>();
const squareMemberIndex = new Map<
  string,
  {
    name: string;
    picturePath: string | null;
    statusMessage: string;
  }
>();
const mediaEpoch = new Map<string, number>();
type BoxCursor = {
  messageId: bigint | number;
  deliveredTime: bigint | number;
};
const boxCursor = new Map<string, BoxCursor>();

const listener = createListener({
  getClient: () => client,
  isListening: () => listening,
  setListening: (next) => {
    listening = next;
  },
  cacheMessage: cacheLiveMessage,
  summarizeTalkMessage,
  summarizeSquareMessage,
  storeBoxCursor,
  upsertChatFromMessage,
  hydrateLiveMedia,
  upsertChatFromContact,
  emitEvent,
});

const commands = createCommandService({
  getClient: () => client,
  setClient: (next) => {
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
  squareChatIndex,
  boxCursor,
  avatarDir,
  chatCachePath,
  storagePath,
  lineDevice: LINE_DEVICE,
  lineVersion: LINE_VERSION,
  refreshCachePolicy,
  cachePolicy,
  dedupe,
  storeBoxCursor,
  existingFile,
  loadDiskContacts,
  refreshContactIndex,
  hydrateAvatars,
  hydratePreviews,
  bumpMediaEpoch,
  loadDiskMessages,
  saveDiskMessages,
  forUiMessages,
  hydrateMedia,
  summarizeTalkMessage,
  summarizeSquareMessage,
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

function sticonSticker(meta: Record<string, string>): {
  stickerId: string;
  packageId: string;
  rawSticonId: string;
} | null {
  const first = sticonResources(meta)[0];
  if (!first) return null;
  return {
    stickerId: `sticon:${first.sticonId}`,
    packageId: first.productId,
    rawSticonId: first.sticonId,
  };
}

async function fromRawTalkSafe(raw: unknown): Promise<TalkMessage> {
  const normalized = normalizeRaw(raw as Record<string, unknown>);
  return await TalkMessage.fromRawTalk(
    normalized as unknown as Parameters<typeof TalkMessage.fromRawTalk>[0],
    client!,
  );
}

let lastE2eeFallbackLog = 0;
let e2eeFallbackSuppressed = 0;

function patchE2eeGuards() {
  if (!client) return;
  const e2ee = client.base.e2ee as unknown as {
    decryptE2EEMessage: (
      m: Record<string, unknown>,
    ) => Promise<Record<string, unknown>>;
  };
  const orig = e2ee.decryptE2EEMessage.bind(e2ee);
  e2ee.decryptE2EEMessage = async (messageObj) => {
    const msg = normalizeRaw(messageObj);
    try {
      return await orig(msg);
    } catch (e) {
      e2eeFallbackSuppressed++;
      const now = Date.now();
      if (now - lastE2eeFallbackLog > 5_000) {
        console.error("[e2ee decrypt fallback]", {
          error: e instanceof Error ? e.message : String(e),
          suppressed: Math.max(0, e2eeFallbackSuppressed - 1),
        });
        lastE2eeFallbackLog = now;
        e2eeFallbackSuppressed = 0;
      }
      return msg;
    }
  };
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
  const p = fn().finally(() => inFlight.delete(key));
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

const reactionKinds = [
  "ALL",
  "UNDO",
  "NICE",
  "LOVE",
  "FUN",
  "AMAZING",
  "SAD",
  "OMG",
];

function reactionKind(value: unknown): string {
  if (typeof value === "string") {
    const upper = value.toUpperCase();
    if (reactionKinds.includes(upper)) return upper;
    if (/^\d+$/.test(value)) return reactionKinds[Number(value)] ?? "";
  }
  if (typeof value === "number") return reactionKinds[value] ?? "";
  return "";
}

function summarizeTalkReactions(value: unknown): Json[] {
  if (!Array.isArray(value)) return [];
  const counts = new Map<string, { count: number; mine: boolean }>();
  for (const row of value as Array<Record<string, unknown>>) {
    const type = reactionKind(
      (row.reactionType as Record<string, unknown> | undefined)
        ?.predefinedReactionType,
    );
    if (!type || type === "ALL" || type === "UNDO") continue;
    const previous = counts.get(type) ?? { count: 0, mine: false };
    previous.count++;
    previous.mine ||= String(row.fromUserMid ?? "") === myMid();
    counts.set(type, previous);
  }
  return [...counts].map(([kind, summary]) => ({ kind, ...summary }));
}

function summarizeSquareReactions(value: unknown): Json[] {
  const status = value as
    | {
      countByReactionType?: Record<string, unknown>;
      myReaction?: { type?: unknown };
    }
    | null
    | undefined;
  if (!status?.countByReactionType) return [];
  const mine = reactionKind(status.myReaction?.type);
  const rows: Json[] = [];
  for (
    const [rawType, rawCount] of Object.entries(status.countByReactionType)
  ) {
    const kind = reactionKind(rawType);
    const count = Number(rawCount ?? 0);
    if (!kind || kind === "ALL" || kind === "UNDO" || count <= 0) continue;
    rows.push({ kind, count, mine: mine === kind });
  }
  return rows;
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
  return {
    mid: profile.mid,
    displayName: profile.displayName ?? "",
    statusMessage: profile.statusMessage ?? "",
    picturePath,
    avatarPath,
    pictureUrl,
  };
}

function contactStatusName(status: unknown): string {
  if (typeof status === "string") return status;
  return ({
    0: "UNSPECIFIED",
    1: "FRIEND",
    2: "FRIEND_BLOCKED",
    3: "RECOMMEND",
    4: "RECOMMEND_BLOCKED",
    5: "DELETED",
    6: "DELETED_BLOCKED",
  } as Record<number, string>)[Number(status)] ?? "UNSPECIFIED";
}

async function contactRelationPayload(mid: string) {
  if (!client) throw new Error("not_logged_in");
  if (!mid.startsWith("u")) throw new Error("profile_not_line_user");

  const contact = await client.base.talk.getContact({ mid });
  const status = contactStatusName(contact.status);
  const isFriend = status === "FRIEND" || status === "FRIEND_BLOCKED";
  const blocked = status === "FRIEND_BLOCKED" ||
    status === "RECOMMEND_BLOCKED" || status === "DELETED_BLOCKED";
  const picturePath = contact.picturePath || null;
  let avatarPath: string | null = null;
  try {
    avatarPath = await avatarPathFor(mid, picturePath);
  } catch (error) {
    console.error("[profile relation avatar]", error);
  }

  contactIndex.set(mid, {
    name: contact.displayName || mid,
    picturePath,
    statusMessage: contact.statusMessage ?? "",
    kind: /BOT/i.test(String(contact.type ?? "")) ? "bot" : "dm",
    muted: !!(contact as { notificationDisabled?: boolean })
      .notificationDisabled,
  });
  await saveDiskContacts();

  return {
    mid,
    displayName: contact.displayName || mid,
    statusMessage: contact.statusMessage ?? "",
    picturePath,
    avatarPath,
    status,
    isFriend,
    blocked,
    canAdd: !isFriend && !blocked,
    canChat: isFriend,
  };
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
          statusMessage?: string;
          kind?: string;
          muted?: boolean;
        }>,
      )
    ) {
      contactIndex.set(mid, {
        name: c.name,
        picturePath: c.picturePath ?? null,
        statusMessage: c.statusMessage ?? "",
        kind: c.kind || "dm",
        muted: !!c.muted,
      });
    }
    const at = Number(raw.at || 0);
    if (Number.isFinite(at) && at > 0) contactsAt = at;
  } catch { /* miss */ }
}

async function saveDiskContacts() {
  const contacts: Record<
    string,
    {
      name: string;
      picturePath: string | null;
      statusMessage: string;
      kind: string;
      muted: boolean;
    }
  > = {};
  for (const [mid, c] of contactIndex) contacts[mid] = c;
  try {
    await atomicWriteJson(contactCachePath, { at: Date.now(), contacts });
  } catch (error) {
    console.error("[contacts-cache]", error);
  }
}

async function loadDiskMessages(
  chatMid: string,
): Promise<MessageCacheEntry | null> {
  try {
    const raw = JSON.parse(
      await Deno.readTextFile(join(msgDiskDir, `${chatMid}.json`)),
    );
    const schema = Number(raw.schema ?? 0);
    if (
      (schema === 0 || schema === 2 || schema === MESSAGE_CACHE_SCHEMA) &&
      Array.isArray(raw.messages)
    ) {
      const messages = raw.messages as Json[];
      const oldest = messages[0];
      return {
        at: Number(raw.at || 0),
        messages,
        historyComplete: schema === MESSAGE_CACHE_SCHEMA &&
          !!raw.historyComplete,
        oldestCursor: raw.oldestCursor ?? (oldest
          ? {
            messageId: String(oldest.id ?? ""),
            deliveredTime: String(oldest.createdTime ?? "0"),
          }
          : undefined),
        squareSyncToken: typeof raw.squareSyncToken === "string"
          ? raw.squareSyncToken
          : undefined,
      };
    }
  } catch { /* miss */ }
  return null;
}

const messageWriteQueues = new Map<string, Promise<void>>();
const liveMessageSaveTimers = new Map<string, ReturnType<typeof setTimeout>>();

async function saveDiskMessages(chatMid: string, entry: MessageCacheEntry) {
  const snapshot: MessageCacheEntry = {
    ...entry,
    messages: entry.messages.map((message) => ({ ...message })),
  };
  const previous = messageWriteQueues.get(chatMid) ?? Promise.resolve();
  const write = previous.catch(() => {}).then(async () => {
    await atomicWriteJson(join(msgDiskDir, `${chatMid}.json`), {
      schema: MESSAGE_CACHE_SCHEMA,
      at: snapshot.at,
      historyComplete: !!snapshot.historyComplete,
      oldestCursor: snapshot.oldestCursor ?? null,
      squareSyncToken: snapshot.squareSyncToken ?? null,
      messages: snapshot.messages,
    });
  });
  messageWriteQueues.set(chatMid, write);
  try {
    await write;
  } catch (error) {
    console.error("[messages-cache]", chatMid, error);
  } finally {
    if (messageWriteQueues.get(chatMid) === write) {
      messageWriteQueues.delete(chatMid);
    }
  }
}

async function cacheLiveMessage(chatMid: string, message: Json) {
  let entry = msgCache.get(chatMid);
  if (!entry) entry = await loadDiskMessages(chatMid) ?? undefined;
  entry ??= { at: Date.now(), messages: [], historyComplete: false };
  const id = String(message.id ?? "");
  const index = id
    ? entry.messages.findIndex((row) => String(row.id ?? "") === id)
    : -1;
  if (index >= 0) {
    entry.messages[index] = { ...entry.messages[index], ...message };
  } else {
    entry.messages.push(message);
  }
  entry.messages.sort((a, b) =>
    Number(a.createdTime ?? 0) - Number(b.createdTime ?? 0)
  );
  entry.at = Date.now();
  msgCache.set(chatMid, entry);
  const pending = liveMessageSaveTimers.get(chatMid);
  if (pending !== undefined) clearTimeout(pending);
  liveMessageSaveTimers.set(
    chatMid,
    setTimeout(() => {
      liveMessageSaveTimers.delete(chatMid);
      const latest = msgCache.get(chatMid);
      if (latest) void saveDiskMessages(chatMid, latest);
    }, 350),
  );
}

function markMessageCacheStale(chatMid: string) {
  const entry = msgCache.get(chatMid);
  if (entry) entry.at = 0;
}

async function messageCacheEntry(chatMid: string): Promise<MessageCacheEntry> {
  let entry = msgCache.get(chatMid);
  if (!entry) entry = await loadDiskMessages(chatMid) ?? undefined;
  entry ??= { at: Date.now(), messages: [], historyComplete: false };
  return entry;
}

async function publishMessageMutation(
  chatMid: string,
  entry: MessageCacheEntry,
) {
  entry.at = Date.now();
  msgCache.set(chatMid, entry);
  await saveDiskMessages(chatMid, entry);
  emitEvent("messages", {
    chatMid,
    messages: await forUiMessages(entry.messages),
    cached: false,
    historyComplete: !!entry.historyComplete,
  });
}

async function cacheOwnReaction(
  chatMid: string,
  messageId: string,
  reaction: string,
) {
  const entry = await messageCacheEntry(chatMid);
  const message = entry.messages.find((row) =>
    String(row.id ?? "") === messageId
  );
  if (!message) return;
  const rows = Array.isArray(message.reactions)
    ? (message.reactions as Json[]).map((row) => ({ ...row }))
    : [];
  for (let index = rows.length - 1; index >= 0; index--) {
    if (!rows[index]?.mine) continue;
    rows[index]!.count = Math.max(0, Number(rows[index]!.count ?? 0) - 1);
    rows[index]!.mine = false;
    if (Number(rows[index]!.count) === 0) rows.splice(index, 1);
  }
  if (reaction !== "UNDO") {
    const current = rows.find((row) => String(row.kind ?? "") === reaction);
    if (current) {
      current.count = Number(current.count ?? 0) + 1;
      current.mine = true;
    } else {
      rows.push({ kind: reaction, count: 1, mine: true });
    }
  }
  message.reactions = rows;
  await publishMessageMutation(chatMid, entry);
}

async function removeCachedMessage(chatMid: string, messageId: string) {
  const entry = await messageCacheEntry(chatMid);
  entry.messages = entry.messages.filter((row) =>
    String(row.id ?? "") !== messageId
  );
  await publishMessageMutation(chatMid, entry);
  const latest = entry.messages.at(-1);
  if (latest) touchChatPreviewFromMessage(latest);
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
        const type = String(c.type ?? "");
        const kind = /BOT/i.test(type) ? "bot" : "dm";
        contactIndex.set(mid, {
          name: c.displayName || mid,
          picturePath: c.picturePath ?? null,
          statusMessage: c.statusMessage ?? "",
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
          const type = String(c.type ?? "");
          contactIndex.set(mid, {
            name: c.displayName || mid,
            picturePath: c.picturePath ?? null,
            statusMessage: c.statusMessage ?? "",
            kind: /BOT/i.test(type) ? "bot" : "dm",
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

async function senderProfilePayload(mid: string): Promise<Json> {
  if (!mid) {
    return {
      senderName: "",
      senderAvatarPath: null,
      senderStatusMessage: "",
      senderKind: "line",
    };
  }
  const info = await resolveContactInfo(mid);
  let avatarPath: string | null = null;
  try {
    avatarPath = await avatarPathFor(mid, info.picturePath);
  } catch (e) {
    console.error("[sender avatar]", mid, e);
  }
  return {
    senderName: info.name,
    senderAvatarPath: avatarPath,
    senderStatusMessage: info.statusMessage,
    senderKind: "line",
  };
}

async function resolveSquareMemberInfo(mid: string): Promise<{
  name: string;
  picturePath: string | null;
  statusMessage: string;
}> {
  const cached = squareMemberIndex.get(mid);
  if (cached) return cached;
  const fallback = {
    name: mid,
    picturePath: null,
    statusMessage: "",
  };
  if (!client || !mid) return fallback;
  try {
    const response = await client.base.square.getSquareMember({
      squareMemberMid: mid,
    });
    const member = response.squareMember;
    const info = {
      name: member.displayName || mid,
      picturePath: squareObsUrl(member.profileImageObsHash),
      statusMessage: member.selfIntroduction || "",
    };
    squareMemberIndex.set(mid, info);
    return info;
  } catch (e) {
    console.error("[openchat member]", mid, e);
    return fallback;
  }
}

async function summarizeSquareMessage(
  sm: SquareMessage,
  _opts: { withMedia?: boolean } = {},
): Promise<Json> {
  const raw = sm.raw;
  const message = raw.message;
  const id = String(message.id ?? "");
  const from = String(message.from ?? "");
  const chatMid = String(message.to ?? "");
  let contentType = normType(message.contentType);
  const meta = (message.contentMetadata ?? {}) as Record<string, string>;
  const member = await resolveSquareMemberInfo(from);
  let avatarPath: string | null = null;
  try {
    avatarPath = await avatarPathFor(from, member.picturePath);
  } catch (e) {
    console.error("[openchat avatar]", from, e);
  }
  let mine = false;
  try {
    mine = await sm.isMyMessage();
  } catch { /* keep false */ }
  let text = sm.text || meta.ALT_TEXT || meta.STKTXT || "";
  let imagePath: string | null = null;
  let imageUrl: string | null = null;
  let stickerId = meta.STKID || "";
  let stickerPackageId = meta.STKPKGID || "";
  let rawSticonId = "";
  const sticon = contentType === "NONE" ? sticonSticker(meta) : null;
  if (sticon) {
    contentType = "STICKER";
    stickerId = sticon.stickerId;
    stickerPackageId = sticon.packageId;
    rawSticonId = sticon.rawSticonId;
    text = "[Sticker]";
  }
  if (contentType === "STICKER" && stickerId) {
    text = text || "[Sticker]";
    if (rawSticonId) {
      imageUrl = stickers.sticonUrl(stickerPackageId, rawSticonId);
      imagePath = await stickers.ensureSticon(stickerPackageId, rawSticonId);
    } else {
      imageUrl = stickers.animationUrl(stickerId);
      imagePath = await stickers.ensureImage(stickerId);
    }
  } else if (contentType === "IMAGE") {
    text = text || "[Image]";
    imageUrl = client?.base.obs.getMessageDataUrl(id, true, true) ?? null;
  } else if (contentType === "VIDEO") {
    text = text || "[Video]";
  } else if (contentType === "AUDIO") {
    text = text || "Voice message";
  } else if (!text && contentType !== "NONE") {
    text = `[${contentType}]`;
  }
  return {
    id,
    text,
    from,
    to: chatMid,
    chatMid,
    mine,
    createdTime: Number(message.createdTime ?? 0),
    contentType,
    imagePath,
    imageUrl,
    audioPath: null,
    fileName: meta.FILE_NAME || meta.FILENAME || null,
    filePath: null,
    stickerId: stickerId || null,
    stickerPackageId: stickerPackageId || null,
    flex: contentType === "FLEX" ? extractFlex(meta) : null,
    durationMs: meta.DURATION ? Number(meta.DURATION) : null,
    needsMedia: contentType === "STICKER" ? !imagePath : false,
    senderName: member.name,
    senderAvatarPath: avatarPath,
    senderStatusMessage: member.statusMessage,
    senderKind: "openchat",
    reactions: summarizeSquareReactions(
      (raw as unknown as { messageReactionStatus?: unknown })
        .messageReactionStatus,
    ),
  };
}

async function summarizeTalkMessage(
  tm: TalkMessage,
  opts: { withMedia?: boolean } = {},
): Promise<Json> {
  const withMedia = opts.withMedia === true;
  const raw = tm.raw as {
    id?: string | number;
    from?: string;
    to?: string;
    createdTime?: number | string | bigint;
    contentType?: string | number;
    contentMetadata?: Record<string, string>;
    reactions?: unknown;
  };
  let contentType = normType(raw.contentType);
  const meta = raw.contentMetadata ?? {};
  let text = tm.text || meta.ALT_TEXT || meta.STKTXT || "";
  let imagePath: string | null = null;
  let imageUrl: string | null = meta.PREVIEW_URL || meta.DOWNLOAD_URL || null;
  const id = String(raw.id ?? "");
  const reactions = summarizeTalkReactions(raw.reactions);
  let stkId = meta.STKID || "";
  let stkPkg = meta.STKPKGID || "";
  let rawSticonId = "";
  const sticon = contentType === "NONE" ? sticonSticker(meta) : null;
  if (sticon) {
    contentType = "STICKER";
    stkId = sticon.stickerId;
    stkPkg = sticon.packageId;
    rawSticonId = sticon.rawSticonId;
    text = "[Sticker]";
    imageUrl = stickers.sticonUrl(stkPkg, rawSticonId);
  }
  const sender = await senderProfilePayload(String(raw.from ?? ""));

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
      ...sender,
      id,
      text: duration
        ? `Voice message (${Math.round(Number(duration) / 1000)}s)`
        : text,
      from: raw.from ?? "",
      to: raw.to ?? "",
      mine: isMineFrom(raw.from),
      chatMid: talkChatMid({
        from: raw.from,
        to: raw.to,
        mine: isMineFrom(raw.from),
      }),
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
      reactions,
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
    text = text || fileName || "[File]";
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
      if (rawSticonId) {
        imageUrl = stickers.sticonUrl(stkPkg, rawSticonId);
        if (withMedia) {
          imagePath = await stickers.ensureSticon(stkPkg, rawSticonId);
        }
      } else {
        imageUrl = stickers.animationUrl(stkId);
        if (withMedia) {
          imagePath = await stickers.ensureImage(stkId);
        }
      }
    }
  } else if (contentType === "FLEX") {
    text = text || meta.ALT_TEXT || "[Flex message]";
  } else if (!text) {
    text = contentType !== "NONE" ? `[${contentType}]` : "";
  }

  const flex = contentType === "FLEX" ? extractFlex(meta) : null;

  return {
    ...sender,
    id,
    text,
    from: raw.from ?? "",
    to: raw.to ?? "",
    mine: isMineFrom(raw.from),
    chatMid: talkChatMid({
      from: raw.from,
      to: raw.to,
      mine: isMineFrom(raw.from),
    }),
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
    reactions,
  };
}

async function summarizeRawMessage(
  raw: Parameters<typeof TalkMessage.fromRawTalk>[0],
  opts?: { withMedia?: boolean },
) {
  try {
    const tm = await fromRawTalkSafe(raw);
    return await summarizeTalkMessage(tm, opts);
  } catch (e) {
    console.error("[summarizeRawMessage]", e);
    const r = normalizeRaw(raw as unknown as Record<string, unknown>);
    const meta = (r.contentMetadata ?? {}) as Record<string, string>;
    let contentType = normType(r.contentType);
    let stickerId = meta.STKID || "";
    let stickerPackageId = meta.STKPKGID || "";
    let imageUrl: string | null = meta.PREVIEW_URL || meta.DOWNLOAD_URL || null;
    const sticon = contentType === "NONE" ? sticonSticker(meta) : null;
    if (sticon) {
      contentType = "STICKER";
      stickerId = sticon.stickerId;
      stickerPackageId = sticon.packageId;
      imageUrl = stickers.sticonUrl(stickerPackageId, sticon.rawSticonId);
    }
    let text = String(r.text ?? meta.ALT_TEXT ?? meta.STKTXT ?? "");
    if (sticon) text = "[Sticker]";
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
      ...(await senderProfilePayload(String(r.from ?? ""))),
      id: String(r.id ?? ""),
      text,
      from: String(r.from ?? ""),
      to: String(r.to ?? ""),
      mine: isMineFrom(r.from),
      chatMid: talkChatMid({
        from: r.from,
        to: r.to,
        mine: isMineFrom(r.from),
      }),
      createdTime: Number(r.createdTime ?? 0),
      contentType,
      imagePath: null,
      imageUrl,
      audioPath: null,
      fileName: meta.FILE_NAME || meta.FILENAME || null,
      filePath: null,
      stickerId: stickerId || null,
      stickerPackageId: stickerPackageId || null,
      flex: contentType === "FLEX" ? extractFlex(meta) : null,
      durationMs: meta.DURATION ? Number(meta.DURATION) : null,
      needsMedia: contentType === "IMAGE" || contentType === "VIDEO" ||
        contentType === "STICKER" || contentType === "AUDIO",
      reactions: summarizeTalkReactions(r.reactions),
    };
  }
}

async function hydrateMedia(
  messages: Json[],
  chatMid: string,
  opts: { isolated?: boolean } = {},
) {
  if (!client || !stdoutAlive) return;
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
        if (stkId.startsWith("sticon:")) {
          const productId = String(m.stickerPackageId || "");
          path = await stickers.ensureSticon(
            productId,
            stkId.slice("sticon:".length),
          );
        } else {
          path = await stickers.ensureImage(stkId);
        }
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
  const need = chats
    .filter((c) => {
      const L = previewLang();
      return c.lastActivity > 0 &&
        (!c.preview || c.preview === "Tap to open" || c.preview === L.tap);
    })
    .slice(0, PREVIEW_TOP);
  if (!need.length) return;

  await mapPool(need, PREVIEW_CONCURRENCY, async (c) => {
    if (!client || !stdoutAlive || workGen !== gen) return;
    try {
      let cursor = boxCursor.get(c.mid);
      if (!cursor) {
        // refresh cursors cheaply only when missing
        const boxes = await client.base.talk.getMessageBoxes({
          messageBoxListRequest: {},
        });
        if (!client || workGen !== gen) return;
        for (const box of boxes.messageBoxes ?? []) {
          const mid = String(box.id ?? "");
          storeBoxCursor(mid, box.lastDeliveredMessageId);
        }
        cursor = boxCursor.get(c.mid);
      }
      if (!cursor) return;

      const messages = await client.base.talk.getPreviousMessagesV2WithRequest({
        request: {
          messageBoxId: c.mid,
          endMessageId: {
            messageId: cursor.messageId,
            deliveredTime: cursor.deliveredTime,
          },
          messagesCount: 1,
        },
      });
      if (!client || workGen !== gen || !stdoutAlive) return;
      const raw = messages?.[messages.length - 1] ?? messages?.[0];
      if (!raw) return;
      const summary = await summarizeRawMessage(raw, { withMedia: false });
      if (workGen !== gen || !stdoutAlive) return;
      const preview = previewLine(summary);
      c.preview = preview;
      if (chatCache) {
        const row = chatCache.chats.find((x) => x.mid === c.mid);
        if (row) row.preview = preview;
      }
      emitEvent("chat_preview", {
        mid: c.mid,
        preview,
        lastActivity: c.lastActivity,
      });
    } catch (e) {
      console.error("[hydratePreviews]", c.mid, e);
      if (!c.preview) {
        c.preview = previewLang().tap;
        emitEvent("chat_preview", {
          mid: c.mid,
          preview: c.preview,
          lastActivity: c.lastActivity,
        });
      }
    }
  });

  if (chatCache) {
    try {
      await atomicWriteJson(chatCachePath, chatCache);
    } catch (error) {
      console.error("[chat-preview-cache]", error);
    }
  }
}

async function buildFriendsList(): Promise<ChatRow[]> {
  const friends: ChatRow[] = [];
  for (const [mid, info] of contactIndex) {
    // Users / bots / OA use u… mids; skip groups (c…) and rooms (r…).
    if (!mid.startsWith("u")) continue;
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
      await atomicWriteJson(chatCachePath, chatCache);
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
  statusMessage: string;
  kind: string;
  muted: boolean;
}> {
  if (mid === myMid() && client?.base.profile) {
    const profile = client.base.profile;
    return {
      name: profile.displayName || mid,
      picturePath: profile.picturePath ?? null,
      statusMessage: profile.statusMessage ?? "",
      kind: "self",
      muted: false,
    };
  }
  const cached = contactIndex.get(mid);
  if (cached?.name && cached.name !== mid) return cached;
  if (!client) {
    return cached ?? {
      name: mid,
      picturePath: null,
      statusMessage: "",
      kind: mid.startsWith("c") ? "group" : "dm",
      muted: false,
    };
  }
  try {
    const c = await client.base.talk.getContact({ mid });
    const type = String(c.type ?? "");
    const info = {
      name: c.displayName || mid,
      picturePath: c.picturePath ?? null,
      statusMessage: c.statusMessage ?? "",
      kind: /BOT/i.test(type) ? "bot" : mid.startsWith("c") ? "group" : "dm",
      muted: !!(c as { notificationDisabled?: boolean }).notificationDisabled,
    };
    contactIndex.set(mid, info);
    await saveDiskContacts();
    return info;
  } catch {
    return cached ?? {
      name: mid,
      picturePath: null,
      statusMessage: "",
      kind: mid.startsWith("c") ? "group" : "dm",
      muted: false,
    };
  }
}

async function upsertChatFromMessage(message: Json) {
  const peer = String(
    message.chatMid ?? (message.mine ? message.to : message.from),
  );
  if (!peer) return;
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

  chatCache.chats = [row, ...chatCache.chats.filter((c) => c.mid !== peer)];
  chatCache.at = Date.now();
  try {
    await atomicWriteJson(chatCachePath, chatCache);
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
  if (!chatCache) chatCache = { at: Date.now(), chats: [] };
  let row = chatCache.chats.find((x) => x.mid === mid);
  const created = !row;
  if (!row) {
    row = {
      mid,
      name: info.name,
      kind: info.kind,
      picturePath: info.picturePath,
      avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
      lastActivity: Date.now(),
      unread: 0,
      preview: "",
    };
  } else {
    row.name = info.name || row.name;
    row.kind = info.kind || row.kind;
    row.picturePath = info.picturePath ?? row.picturePath ?? null;
  }
  chatCache.chats = [row, ...chatCache.chats.filter((c) => c.mid !== mid)];
  chatCache.at = Date.now();
  try {
    await atomicWriteJson(chatCachePath, chatCache);
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
    chatMid,
    senderName: client?.base.profile?.displayName ?? "",
    senderAvatarPath: null,
    senderStatusMessage: client?.base.profile?.statusMessage ?? "",
    senderKind: "line",
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
    reactions: [],
  };
}

async function handle(req: Json) {
  const id = (req.id as number | string | null) ?? null;
  const method = String(req.method ?? "");
  const params = (req.params ?? {}) as Json;

  try {
    switch (method) {
      case "ping":
        ok(id, { pong: true, version: "0.2.0" });
        break;
      case "login_qr":
        await commands.loginQr(id);
        break;
      case "login_token":
        await commands.loginToken(id, params.token as string | undefined);
        break;
      case "list_chats":
        await commands.listChats(id, !!params.force);
        break;
      case "fetch_messages":
        await commands.fetchMessages(
          id,
          String(params.chatMid ?? ""),
          Number(params.limit ?? 50),
          !!params.force,
        );
        break;
      case "send_message":
        await commands.sendMessage(
          id,
          String(params.chatMid ?? ""),
          String(params.text ?? ""),
        );
        break;
      case "react_message": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const chatMid = String(params.chatMid ?? "");
        const messageId = String(params.messageId ?? "");
        const reaction = reactionKind(params.reaction);
        if (!chatMid || !messageId || !reaction || reaction === "ALL") {
          fail(id, "invalid_reaction");
          break;
        }
        if (squareChatIndex.has(chatMid) || chatMid.startsWith("m")) {
          const react = client.base.square.reactToMessage as unknown as (
            options: {
              request: {
                reqSeq: number;
                reactionType: string;
                messageId: string;
                squareChatMid: string;
              };
            },
          ) => Promise<unknown>;
          await react.call(client.base.square, {
            request: {
              reqSeq: 0,
              reactionType: reaction,
              messageId,
              squareChatMid: chatMid,
            },
          });
        } else {
          const react = client.base.talk.react as unknown as (
            options: { id: bigint; reaction: string },
          ) => Promise<unknown>;
          await react.call(client.base.talk, {
            id: BigInt(messageId),
            reaction,
          });
        }
        await cacheOwnReaction(chatMid, messageId, reaction);
        ok(id, { reacted: true, chatMid, messageId, reaction });
        break;
      }
      case "unsend_message": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const chatMid = String(params.chatMid ?? "");
        const messageId = String(params.messageId ?? "");
        if (!chatMid || !messageId) {
          fail(id, "missing_message_id");
          break;
        }
        if (squareChatIndex.has(chatMid) || chatMid.startsWith("m")) {
          const unsend = client.base.square.unsendMessage as unknown as (
            options: { squareChatMid: string; messageId: string },
          ) => Promise<unknown>;
          await unsend.call(client.base.square, {
            squareChatMid: chatMid,
            messageId,
          });
        } else {
          await client.base.talk.unsendMessage({ messageId });
        }
        await removeCachedMessage(chatMid, messageId);
        ok(id, { unsent: true, chatMid, messageId });
        break;
      }
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
        await stickers.list(id);
        break;
      case "mark_read":
        await commands.markRead(
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
        await calls.doCallStart(
          id,
          String(params.mid ?? params.chatMid ?? ""),
          params.videoCapable === true,
        );
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
      case "call_screen_start": {
        calls.doCallScreenStart(id);
        break;
      }
      case "call_screen_stop": {
        calls.doCallScreenStop(id);
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
            markMessageCacheStale(chatMid);
            ok(id, { sent: true, fallback: "message" });
          } catch {
            fail(id, e instanceof Error ? e.message : String(e));
          }
        }
        break;
      }
      case "clear_cache": {
        const kind = String(params.kind ?? "all");
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
        if (kind === "messages" || kind === "all") await wipe(msgDiskDir);
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
            statusMessage: "",
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
      case "profile_relation": {
        try {
          ok(id, await contactRelationPayload(String(params.mid ?? "")));
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "add_friend_mid": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const mid = String(params.mid ?? "");
        if (!mid.startsWith("u")) {
          fail(id, "profile_not_line_user");
          break;
        }
        if (mid === myMid()) {
          fail(id, "cannot_add_self");
          break;
        }
        try {
          await client.base.relation.addFriendByMid({ mid });
          const relation = await contactRelationPayload(mid);
          ok(id, { ...relation, isFriend: true, canAdd: false, canChat: true });
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
        if (!mid.startsWith("u")) {
          fail(id, "mute_dm_only");
          break;
        }
        try {
          await client.base.talk.updateContactSetting({
            reqSeq: await client.base.getReqseq(),
            mid,
            flag: "CONTACT_SETTING_NOTIFICATION_DISABLE",
            value: muted ? "true" : "false",
          });
          const cur = contactIndex.get(mid);
          if (cur) cur.muted = muted;
          else {
            contactIndex.set(mid, {
              name: mid,
              picturePath: null,
              statusMessage: "",
              kind: "dm",
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
        cancelBackgroundWork();
        stickers.resetOwnedCache();
        try {
          await clearAuth();
        } catch { /* ignore */ }
        client = null;
        listening = false;
        chatCache = null;
        msgCache.clear();
        squareChatIndex.clear();
        squareMemberIndex.clear();
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

const bootAuth = await loadAuth();
const bootDevice = await loadAuthDevice();
const needsAndroidRelogin = !!bootAuth && bootDevice === "DESKTOPWIN";

emitEvent("ready", {
  dataDir,
  sidecar: fromFileUrl(import.meta.url),
  hasAuth: !!bootAuth && !needsAndroidRelogin,
  device: needsAndroidRelogin ? "" : bootDevice,
});

// Legacy DESKTOPWIN tokens cannot do reliable PLANET calls (docs require ANDROID*).
if (needsAndroidRelogin) {
  console.error(
    "[auth] DESKTOPWIN session detected — clearing auth. Re-scan QR for ANDROIDSECONDARY (required for voice calls per linejs docs).",
  );
  await clearAuth();
  emitEvent("session_failed", {
    error: "relogin_android_required",
  });
} else if (bootAuth) {
  try {
    const storage = new FileStorage(storagePath);
    const device = bootDevice === "ANDROID" || bootDevice === "ANDROIDSECONDARY"
      ? bootDevice
      : "ANDROIDSECONDARY";
    client = await loginWithAuthToken(bootAuth, {
      device,
      version: LINE_VERSION,
      storage,
    });
    await saveAuth(client.authToken, device);
    const profile = await client.getMyProfile();
    patchE2eeGuards();
    listener.start();
    emitEvent("session", await myProfilePayload(profile));
  } catch (e) {
    console.error("[boot restore]", e);
    emitEvent("session_failed", {
      error: e instanceof Error ? e.message : String(e),
    });
  }
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

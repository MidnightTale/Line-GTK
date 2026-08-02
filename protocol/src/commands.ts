import {
  type Client,
  loginWithAuthToken,
  loginWithQR,
  SquareMessage,
  type TalkMessage,
} from "@evex/linejs";
import { FileStorage } from "@evex/linejs/storage";
import { join } from "@std/path";
import { atomicWriteJson } from "./storage.ts";
import type { CachePolicy } from "./cache_policy.ts";
import { friendName, picturePathOf, squareObsUrl } from "./contacts.ts";
import type { LineDevice } from "./auth.ts";

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
type ChatCache = { at: number; chats: ChatRow[] };
type BoxCursor = { messageId: bigint | number; deliveredTime: bigint | number };
type ContactInfo = {
  name: string;
  picturePath: string | null;
  statusMessage: string;
  kind: string;
  muted: boolean;
};
type MessageHistoryCursor = { messageId: string; deliveredTime: string };
type MessageCacheEntry = {
  at: number;
  messages: Json[];
  historyComplete?: boolean;
  oldestCursor?: MessageHistoryCursor;
  squareSyncToken?: string;
};

export type CommandRuntime = {
  getClient: () => Client | null;
  setClient: (client: Client) => void;
  isListening: () => boolean;
  setAuthDead: (dead: boolean) => void;
  getChatCache: () => ChatCache | null;
  setChatCache: (cache: ChatCache | null) => void;
  msgCache: Map<string, MessageCacheEntry>;
  contactIndex: Map<string, ContactInfo>;
  squareChatIndex: Set<string>;
  boxCursor: Map<string, BoxCursor>;
  avatarDir: string;
  chatCachePath: string;
  storagePath: string;
  lineDevice: LineDevice;
  lineVersion: string;
  refreshCachePolicy: () => Promise<CachePolicy>;
  cachePolicy: () => CachePolicy;
  dedupe: <T>(key: string, fn: () => Promise<T>) => Promise<T>;
  storeBoxCursor: (
    mid: string,
    value?: { messageId?: unknown; deliveredTime?: unknown } | null,
  ) => void;
  existingFile: (path: string) => Promise<string | null>;
  loadDiskContacts: () => Promise<void>;
  refreshContactIndex: (force?: boolean) => Promise<void>;
  hydrateAvatars: (chats: ChatRow[]) => Promise<void>;
  hydratePreviews: (chats: ChatRow[]) => Promise<void>;
  bumpMediaEpoch: (chatMid: string) => number;
  loadDiskMessages: (chatMid: string) => Promise<MessageCacheEntry | null>;
  saveDiskMessages: (
    chatMid: string,
    entry: MessageCacheEntry,
  ) => Promise<void>;
  forUiMessages: (messages: Json[]) => Promise<Json[]>;
  hydrateMedia: (messages: Json[], chatMid: string) => Promise<void>;
  summarizeTalkMessage: (
    message: TalkMessage,
    opts?: { withMedia?: boolean },
  ) => Promise<Json>;
  summarizeSquareMessage: (
    message: SquareMessage,
    opts?: { withMedia?: boolean },
  ) => Promise<Json>;
  summarizeRawMessage: (
    raw: unknown,
    opts?: { withMedia?: boolean },
  ) => Promise<Json>;
  saveAuth: (token: string, device?: LineDevice) => Promise<void>;
  loadAuth: () => Promise<string | null>;
  loadAuthDevice: () => Promise<LineDevice>;
  clearAuth: () => Promise<void>;
  myProfilePayload: (profile: {
    mid: string;
    displayName?: string;
    statusMessage?: string;
  }) => Promise<Json>;
  patchE2eeGuards: () => void;
  startListen: () => void;
  sentMessagePayload: (
    sent:
      | { id?: unknown; createdTime?: unknown; contentType?: unknown }
      | null
      | undefined,
    chatMid: string,
    text: string,
  ) => Json;
  touchChatPreviewFromMessage: (message: Json) => void;
  emitEvent: (event: string, payload?: Json) => void;
  ok: (id: number | string | null, result?: unknown) => void;
  fail: (id: number | string | null, error: string) => void;
};

export function createCommandService(runtime: CommandRuntime) {
  const {
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
    summarizeRawMessage,
    saveAuth,
    loadAuth,
    loadAuthDevice,
    clearAuth,
    myProfilePayload,
    patchE2eeGuards,
    startListen,
    sentMessagePayload,
    touchChatPreviewFromMessage,
    emitEvent,
    ok,
    fail,
  } = runtime;

  async function doMarkRead(
    id: number | string | null,
    chatMid: string,
    lastMessageId: string,
  ) {
    const client = runtime.getClient();
    const chatCache = runtime.getChatCache();
    if (!client) {
      // Soft-fail: opening a chat during restore shouldn't toast loudly.
      ok(id, { marked: false, skipped: "not_logged_in" });
      return;
    }
    if (!chatMid || !lastMessageId) {
      ok(id, { marked: false, skipped: "missing_ids" });
      return;
    }
    try {
      if (squareChatIndex.has(chatMid) || chatMid.startsWith("m")) {
        await client.base.square.markAsRead({
          request: {
            squareChatMid: chatMid,
            messageId: String(lastMessageId),
          },
        });
        ok(id, {
          marked: true,
          chatMid,
          lastMessageId: String(lastMessageId),
        });
        return;
      }
      const requestSequence = client.base.getReqseq;
      const seq = typeof requestSequence === "function"
        ? await requestSequence.call(client.base)
        : Number(requestSequence);
      // thrift STRING (ftype 11) — must be string, not i64/bigint
      await client.base.talk.sendChatChecked({
        chatMid: String(chatMid),
        lastMessageId: String(lastMessageId),
        seq,
      });
      if (chatCache) {
        const row = chatCache.chats.find((x) => x.mid === chatMid);
        if (row) row.unread = 0;
      }
      ok(id, { marked: true, chatMid, lastMessageId: String(lastMessageId) });
    } catch (e) {
      console.error("[mark_read]", e);
      // Soft-fail — mark-read is best-effort.
      ok(id, {
        marked: false,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  }

  async function doLoginQr(id: number | string | null) {
    runtime.setAuthDead(false);
    const storage = new FileStorage(storagePath);
    // Docs: QR defaults to ANDROIDSECONDARY for call-capable sessions.
    const device = LINE_DEVICE === "DESKTOPWIN"
      ? "ANDROIDSECONDARY"
      : LINE_DEVICE;
    const client = await loginWithQR(
      {
        onReceiveQRUrl: (url) => emitEvent("qr", { url }),
        onPincodeRequest: (pin) => emitEvent("pin", { pin }),
      },
      { device, version: LINE_VERSION, storage },
    );
    runtime.setClient(client);
    await saveAuth(client.authToken, device);
    const profile = await client.getMyProfile();
    patchE2eeGuards();
    await startListen();
    ok(id, { ...(await myProfilePayload(profile)), device });
  }

  async function doLoginToken(id: number | string | null, token?: string) {
    runtime.setAuthDead(false);
    let client = runtime.getClient();
    // Already restored (e.g. boot auto-login) — just return the profile.
    if (client && runtime.isListening()) {
      try {
        const profile = await client.getMyProfile();
        ok(id, {
          ...(await myProfilePayload(profile)),
          device: await loadAuthDevice(),
        });
        return;
      } catch {
        /* fall through and re-auth */
      }
    }
    const auth = token?.trim() || (await loadAuth());
    if (!auth) {
      fail(id, "no_auth_token");
      return;
    }
    const storage = new FileStorage(storagePath);
    let device = await loadAuthDevice();
    // Migrate legacy DESKTOPWIN sessions — tokens are device-bound; force re-QR.
    if (device === "DESKTOPWIN") {
      await clearAuth();
      fail(id, "relogin_android_required");
      return;
    }
    if (device !== "ANDROID" && device !== "ANDROIDSECONDARY") {
      device = "ANDROIDSECONDARY";
    }
    client = await loginWithAuthToken(auth, {
      device: device as "ANDROID" | "ANDROIDSECONDARY",
      version: LINE_VERSION,
      storage,
    });
    runtime.setClient(client);
    await saveAuth(client.authToken, device);
    const profile = await client.getMyProfile();
    patchE2eeGuards();
    await startListen();
    ok(id, {
      ...(await myProfilePayload(profile)),
      device,
    });
  }

  async function loadDiskChatCache(): Promise<ChatRow[] | null> {
    try {
      await refreshCachePolicy();
      const raw = JSON.parse(await Deno.readTextFile(chatCachePath));
      if (
        Array.isArray(raw?.chats) &&
        Date.now() - Number(raw.at || 0) < cachePolicy().diskChat
      ) {
        return raw.chats as ChatRow[];
      }
    } catch { /* miss */ }
    return null;
  }

  async function doListChats(id: number | string | null, force = false) {
    const client = runtime.getClient();
    let chatCache = runtime.getChatCache();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }

    await refreshCachePolicy();
    await dedupe(`list_chats:${force}`, async () => {
      if (
        !force && chatCache && Date.now() - chatCache.at < cachePolicy().memChat
      ) {
        ok(id, {
          chats: chatCache.chats,
          count: chatCache.chats.length,
          cached: true,
        });
        return;
      }

      // Instant warm start from disk while we refresh (event, not second ok).
      if (!force) {
        const disk = await loadDiskChatCache();
        if (disk?.length) {
          emitEvent("chats", { chats: disk, count: disk.length, cached: true });
          emitEvent("progress", { scope: "chats", state: "ready" });
        }
      }

      emitEvent("progress", { scope: "chats", state: "loading" });

      const byMid = new Map<string, ChatRow>();

      // 1) Recent boxes (cheap + ordering)
      try {
        const boxes = await client!.base.talk.getMessageBoxes({
          messageBoxListRequest: {},
        });
        for (const box of boxes.messageBoxes ?? []) {
          const mid = String(box.id ?? "");
          if (!mid) continue;
          storeBoxCursor(mid, box.lastDeliveredMessageId);
          const prevPreview = chatCache?.chats.find((x) =>
            x.mid === mid
          )?.preview ?? "";
          byMid.set(mid, {
            mid,
            name: contactIndex.get(mid)?.name || mid,
            kind: contactIndex.get(mid)?.kind ||
              (mid.startsWith("c") ? "group" : "dm"),
            avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
            picturePath: contactIndex.get(mid)?.picturePath ?? null,
            lastActivity: Number(
              box.lastDeliveredMessageId?.deliveredTime ?? 0,
            ),
            unread: Number(box.unreadCount ?? 0),
            preview: prevPreview,
            muted: !!contactIndex.get(mid)?.muted,
            pinned: chatCache?.chats.find((x) => x.mid === mid)?.pinned ??
              false,
          });
        }
      } catch (e) {
        console.error("[list_chats] boxes", e);
      }

      // 2) Contacts + bots/OA (getContacts mids — fetchUsers misses bots)
      try {
        await loadDiskContacts();
        await refreshContactIndex();
        for (const [mid, info] of contactIndex) {
          const prev = byMid.get(mid);
          if (prev) {
            prev.name = info.name;
            prev.kind = info.kind;
            prev.picturePath = info.picturePath;
            prev.muted = !!info.muted;
            if (!prev.avatarPath) {
              prev.avatarPath = await existingFile(
                join(avatarDir, `${mid}.jpg`),
              );
            }
          } else {
            byMid.set(mid, {
              mid,
              name: info.name,
              kind: info.kind,
              picturePath: info.picturePath,
              avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
              lastActivity: 0,
              unread: 0,
              preview: "",
              muted: !!info.muted,
              pinned: false,
            });
          }
        }
      } catch (e) {
        console.error("[list_chats] contacts", e);
        // fallback: friends only
        try {
          const users = await client!.fetchUsers();
          for (const user of users) {
            const name = friendName(user);
            const picturePath = picturePathOf(
              user as unknown as Record<string, unknown>,
            );
            contactIndex.set(user.mid, {
              name,
              picturePath,
              statusMessage: "",
              kind: "dm",
              muted: false,
            });
            const prev = byMid.get(user.mid);
            if (prev) {
              prev.name = name;
              prev.picturePath = picturePath;
            }
          }
        } catch (e2) {
          console.error("[list_chats] users fallback", e2);
        }
      }

      // Keep Memo is LINE's notes-to-self conversation. It uses the signed-in
      // account MID as a regular Talk message box, so expose it even before the
      // account has sent its first memo and a recent box exists.
      try {
        const profile = client!.base.profile;
        if (profile?.mid) {
          const prev = byMid.get(profile.mid);
          byMid.set(profile.mid, {
            mid: profile.mid,
            name: "Keep Memo",
            kind: "keep",
            picturePath: picturePathOf(
              profile as unknown as Record<string, unknown>,
            ),
            avatarPath: prev?.avatarPath ??
              await existingFile(join(avatarDir, `${profile.mid}.jpg`)),
            lastActivity: prev?.lastActivity ?? 0,
            unread: prev?.unread ?? 0,
            preview: prev?.preview ?? "",
            muted: false,
            pinned: prev?.pinned ?? false,
          });
        }
      } catch (e) {
        console.error("[list_chats] keep memo", e);
      }

      // 3) Groups optional (often empty) — don't block naming
      try {
        const chats = await client!.fetchJoinedChats();
        for (const chat of chats) {
          const prev = byMid.get(chat.mid);
          byMid.set(chat.mid, {
            mid: chat.mid,
            name: chat.name || chat.raw.chatName || chat.mid,
            kind: "group",
            lastActivity: prev?.lastActivity ?? 0,
            unread: prev?.unread ?? 0,
            preview: prev?.preview ?? "",
            // Group chat pictures live directly on Chat.picturePath; using the
            // contact index here produced blank/default avatars.
            picturePath: chat.raw.picturePath || prev?.picturePath || null,
            avatarPath: prev?.avatarPath ?? null,
            muted: chat.raw.notificationDisabled ?? prev?.muted ?? false,
            pinned: Number(chat.raw.favoriteTimestamp ?? 0) > 0,
          });
        }
      } catch (e) {
        console.error("[list_chats] groups", e);
      }

      // 4) OpenChat (Square) rooms joined by this account.
      try {
        const openChats = await client!.fetchJoinedSquareChats();
        for (const chat of openChats) {
          const mid = chat.mid;
          if (!mid) continue;
          squareChatIndex.add(mid);
          const prev = byMid.get(mid);
          const picturePath = squareObsUrl(chat.raw.chatImageObsHash);
          byMid.set(mid, {
            mid,
            name: chat.name || mid,
            kind: "openchat",
            lastActivity: prev?.lastActivity ?? 0,
            unread: prev?.unread ?? 0,
            preview: prev?.preview ?? "",
            picturePath,
            avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
            muted: false,
            pinned: prev?.pinned ?? false,
          });
        }
      } catch (e) {
        // Accounts/device types without Square access should still load Talk.
        console.error("[list_chats] openchat", e);
      }

      const chats = [...byMid.values()].sort((a, b) => {
        if (!!a.pinned !== !!b.pinned) return a.pinned ? -1 : 1;
        if (b.lastActivity !== a.lastActivity) {
          return b.lastActivity - a.lastActivity;
        }
        return a.name.localeCompare(b.name);
      });

      chatCache = { at: Date.now(), chats };
      runtime.setChatCache(chatCache);
      try {
        await atomicWriteJson(chatCachePath, chatCache);
      } catch (error) {
        console.error("[chat-list-cache]", error);
      }

      ok(id, { chats, count: chats.length, cached: false });
      emitEvent("progress", {
        scope: "chats",
        state: chats.length ? "ready" : "empty",
      });

      // Background fills — CDN avatars + last-message previews (throttled)
      hydrateAvatars(chats);
      hydratePreviews(chats);
    });
  }

  function mergeMessageRows(current: Json[], incoming: Json[]): Json[] {
    const byId = new Map<string, Json>();
    for (const [index, message] of current.entries()) {
      const id = String(
        message.id ?? `cached:${index}:${message.createdTime ?? 0}`,
      );
      byId.set(id, message);
    }
    for (const [index, message] of incoming.entries()) {
      const id = String(
        message.id ?? `incoming:${index}:${message.createdTime ?? 0}`,
      );
      const previous = byId.get(id);
      byId.set(
        id,
        previous
          ? {
            ...previous,
            ...message,
            imagePath: message.imagePath || previous.imagePath || null,
            audioPath: message.audioPath || previous.audioPath || null,
            filePath: message.filePath || previous.filePath || null,
          }
          : message,
      );
    }
    return [...byId.values()].sort((a, b) =>
      Number(a.createdTime ?? 0) - Number(b.createdTime ?? 0)
    );
  }

  async function cacheMessageRow(chatMid: string, message: Json) {
    let entry = msgCache.get(chatMid);
    if (!entry) entry = await loadDiskMessages(chatMid) ?? undefined;
    entry ??= { at: Date.now(), messages: [], historyComplete: false };
    entry.messages = mergeMessageRows(entry.messages, [message]);
    entry.at = Date.now();
    msgCache.set(chatMid, entry);
    void saveDiskMessages(chatMid, entry);
  }

  function rawHistoryCursor(raw: unknown): MessageHistoryCursor | undefined {
    const row = raw as { id?: unknown; deliveredTime?: unknown };
    if (row?.id == null) return undefined;
    return {
      messageId: String(row.id),
      deliveredTime: String(row.deliveredTime ?? 0),
    };
  }

  async function refreshBoxCursor(
    chatMid: string,
  ): Promise<BoxCursor | undefined> {
    const client = runtime.getClient();
    if (!client) return undefined;
    let cursor = boxCursor.get(chatMid);
    if (cursor) return cursor;
    const boxes = await client.base.talk.getMessageBoxes({
      messageBoxListRequest: {},
    });
    for (const box of boxes.messageBoxes ?? []) {
      storeBoxCursor(String(box.id ?? ""), box.lastDeliveredMessageId);
    }
    cursor = boxCursor.get(chatMid);
    return cursor;
  }

  async function summarizeTalkHistoryPage(
    rawMessages: unknown[],
  ): Promise<Json[]> {
    const out: Json[] = [];
    for (const raw of rawMessages) {
      try {
        const meta = (raw as { contentMetadata?: Record<string, string> })
          .contentMetadata;
        if (meta?.BOT_CHECK || meta?.BOT_ORIGIN) {
          const from = String((raw as { from?: unknown }).from ?? "");
          const contact = contactIndex.get(from);
          if (contact) contact.kind = "bot";
        }
        out.push(await summarizeRawMessage(raw, { withMedia: false }));
      } catch (error) {
        console.error("[fetch msg skip]", error);
      }
    }
    return out;
  }

  async function summarizeSquareHistoryEvents(
    events: unknown[],
  ): Promise<Json[]> {
    const client = runtime.getClient();
    if (!client) return [];
    const out: Json[] = [];
    for (const event of events) {
      const payload = (event as { payload?: unknown }).payload as {
        sendMessage?: { squareMessage?: unknown };
        receiveMessage?: { squareMessage?: unknown };
        notificationMessage?: { squareMessage?: unknown };
      } | undefined;
      const raw = payload?.sendMessage?.squareMessage ??
        payload?.receiveMessage?.squareMessage ??
        payload?.notificationMessage?.squareMessage;
      if (!raw) continue;
      try {
        const message = SquareMessage.fromRawTalk(
          raw as Parameters<typeof SquareMessage.fromRawTalk>[0],
          client,
        );
        out.push(await summarizeSquareMessage(message, { withMedia: false }));
      } catch (error) {
        console.error("[fetch openchat msg skip]", error);
      }
    }
    return out;
  }

  async function publishMessageHistory(
    chatMid: string,
    entry: MessageCacheEntry,
    state: "syncing" | "complete" = "syncing",
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
    emitEvent("progress", {
      scope: "messages",
      chatMid,
      state,
      count: entry.messages.length,
    });
  }

  async function seedMessageHistory(
    chatMid: string,
  ): Promise<MessageCacheEntry> {
    const client = runtime.getClient();
    if (!client) throw new Error("not_logged_in");
    if (squareChatIndex.has(chatMid) || chatMid.startsWith("m")) {
      squareChatIndex.add(chatMid);
      const response = await client.base.square.fetchSquareChatEvents({
        squareChatMid: chatMid,
        limit: 100,
        direction: "BACKWARD",
      });
      return {
        at: Date.now(),
        messages: await summarizeSquareHistoryEvents(response.events ?? []),
        historyComplete: !(response.events?.length),
        squareSyncToken: response.syncToken || undefined,
      };
    }

    const cursor = await refreshBoxCursor(chatMid);
    if (!cursor) {
      return { at: Date.now(), messages: [], historyComplete: true };
    }
    const raw = await client.base.talk.getPreviousMessagesV2WithRequest({
      request: {
        messageBoxId: chatMid,
        endMessageId: cursor,
        messagesCount: 100,
      },
    });
    return {
      at: Date.now(),
      messages: await summarizeTalkHistoryPage(raw),
      historyComplete: raw.length === 0,
      oldestCursor: rawHistoryCursor(raw.at(-1)),
    };
  }

  async function syncTalkHistory(chatMid: string, initial: MessageCacheEntry) {
    const client = runtime.getClient();
    if (!client) return;
    let entry = msgCache.get(chatMid) ?? initial;
    const newest = await refreshBoxCursor(chatMid);

    // First close the gap from the newest server message back to any local row.
    if (newest && !entry.messages.length) {
      const raw = await client.base.talk.getPreviousMessagesV2WithRequest({
        request: {
          messageBoxId: chatMid,
          endMessageId: newest,
          messagesCount: 100,
        },
      });
      entry.messages = mergeMessageRows(
        entry.messages,
        await summarizeTalkHistoryPage(raw),
      );
      entry.oldestCursor = rawHistoryCursor(raw.at(-1));
      entry.historyComplete = raw.length < 100;
      msgCache.set(chatMid, entry);
    } else if (newest && entry.messages.length) {
      const known = new Set(
        entry.messages.map((message) => String(message.id ?? "")),
      );
      let end: BoxCursor = newest;
      for (let page = 0; page < 10_000; page++) {
        const raw = await client.base.talk.getPreviousMessagesV2WithRequest({
          request: {
            messageBoxId: chatMid,
            endMessageId: end,
            messagesCount: 100,
          },
        });
        if (!raw.length) break;
        const overlap = raw.some((message) =>
          known.has(String((message as { id?: unknown }).id ?? ""))
        );
        const rows = await summarizeTalkHistoryPage(raw);
        entry = msgCache.get(chatMid) ?? entry;
        entry.messages = mergeMessageRows(entry.messages, rows);
        entry.at = Date.now();
        msgCache.set(chatMid, entry);
        const oldest = rawHistoryCursor(raw.at(-1));
        if (!oldest || overlap) break;
        end = {
          messageId: BigInt(oldest.messageId),
          deliveredTime: BigInt(oldest.deliveredTime),
        };
        if (raw.length < 100) {
          entry.historyComplete = true;
          entry.oldestCursor = oldest;
          break;
        }
      }
    }

    // Continue the durable oldest cursor until LINE says there is no older page.
    if (!entry.historyComplete) {
      let oldest = entry.oldestCursor;
      if (!oldest && entry.messages.length) {
        const first = entry.messages[0]!;
        oldest = {
          messageId: String(first.id ?? ""),
          deliveredTime: String(first.createdTime ?? 0),
        };
      }
      let pagesSincePublish = 0;
      while (oldest?.messageId) {
        const raw = await client.base.talk.getPreviousMessagesV2WithRequest({
          request: {
            messageBoxId: chatMid,
            endMessageId: {
              messageId: BigInt(oldest.messageId),
              deliveredTime: BigInt(oldest.deliveredTime),
            },
            messagesCount: 100,
          },
        });
        if (!raw.length) {
          entry.historyComplete = true;
          break;
        }
        const nextOldest = rawHistoryCursor(raw.at(-1));
        if (!nextOldest) break;
        const before = entry.messages.length;
        const rows = await summarizeTalkHistoryPage(raw);
        entry = msgCache.get(chatMid) ?? entry;
        entry.messages = mergeMessageRows(entry.messages, rows);
        entry.oldestCursor = nextOldest;
        pagesSincePublish++;
        const stuck = nextOldest.messageId === oldest.messageId;
        oldest = nextOldest;
        if (raw.length < 100 || stuck || entry.messages.length === before) {
          entry.historyComplete = raw.length < 100 || stuck;
          break;
        }
        if (pagesSincePublish >= 5) {
          pagesSincePublish = 0;
          await publishMessageHistory(chatMid, entry);
        }
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    }
    await publishMessageHistory(chatMid, entry, "complete");
    void hydrateMedia(
      await forUiMessages(entry.messages.slice(-200)),
      chatMid,
    );
  }

  async function syncSquareHistory(
    chatMid: string,
    initial: MessageCacheEntry,
  ) {
    const client = runtime.getClient();
    if (!client) return;
    let entry = msgCache.get(chatMid) ?? initial;
    let token = entry.squareSyncToken;
    let pagesSincePublish = 0;
    while (!entry.historyComplete) {
      const response = await client.base.square.fetchSquareChatEvents({
        squareChatMid: chatMid,
        syncToken: token,
        limit: 100,
        direction: "BACKWARD",
      });
      const events = response.events ?? [];
      const nextToken = response.syncToken || undefined;
      if (!events.length || (token && nextToken === token)) {
        entry.historyComplete = true;
        break;
      }
      const rows = await summarizeSquareHistoryEvents(events);
      entry = msgCache.get(chatMid) ?? entry;
      const before = entry.messages.length;
      entry.messages = mergeMessageRows(entry.messages, rows);
      entry.squareSyncToken = nextToken;
      token = nextToken;
      pagesSincePublish++;
      if (entry.messages.length === before) {
        entry.historyComplete = true;
        break;
      }
      if (pagesSincePublish >= 5) {
        pagesSincePublish = 0;
        await publishMessageHistory(chatMid, entry);
      }
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    await publishMessageHistory(chatMid, entry, "complete");
    void hydrateMedia(
      await forUiMessages(entry.messages.slice(-200)),
      chatMid,
    );
  }

  async function doFetchMessages(
    id: number | string | null,
    chatMid: string,
    _limit = 50,
    force = false,
  ) {
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    if (!chatMid) {
      fail(id, "missing_chat");
      return;
    }

    bumpMediaEpoch(chatMid);
    try {
      let entry = msgCache.get(chatMid);
      if (!entry) {
        entry = await loadDiskMessages(chatMid) ?? undefined;
        if (entry) msgCache.set(chatMid, entry);
      }

      if (!entry) {
        emitEvent("progress", {
          scope: "messages",
          chatMid,
          state: "loading",
        });
        entry = await seedMessageHistory(chatMid);
        msgCache.set(chatMid, entry);
        await saveDiskMessages(chatMid, entry);
      }

      const uiMessages = await forUiMessages(entry.messages);
      ok(id, {
        messages: uiMessages,
        cached: !force,
        historyComplete: !!entry.historyComplete,
      });
      emitEvent("progress", {
        scope: "messages",
        chatMid,
        state: entry.messages.length ? "ready" : "empty",
      });
      void hydrateMedia(uiMessages.slice(-200), chatMid);

      const sync = squareChatIndex.has(chatMid) || chatMid.startsWith("m")
        ? () => syncSquareHistory(chatMid, entry!)
        : () => syncTalkHistory(chatMid, entry!);
      void dedupe(`history:${chatMid}`, sync).catch((error) => {
        console.error("[history sync]", chatMid, error);
        emitEvent("progress", {
          scope: "messages",
          chatMid,
          state: "error",
          error: error instanceof Error ? error.message : String(error),
        });
      });
    } catch (error) {
      fail(id, error instanceof Error ? error.message : String(error));
      emitEvent("progress", {
        scope: "messages",
        chatMid,
        state: "error",
        error: String(error),
      });
    }
  }

  async function doSend(
    id: number | string | null,
    chatMid: string,
    text: string,
  ) {
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    if (!text.trim()) {
      fail(id, "empty_message");
      return;
    }

    if (squareChatIndex.has(chatMid) || chatMid.startsWith("m")) {
      try {
        squareChatIndex.add(chatMid);
        const chat = await client.getSquareChat(chatMid);
        const sent = await chat.sendMessage(text);
        const squareMessage = SquareMessage.fromRawTalk(
          sent.createdSquareMessage,
          client,
        );
        const message = await summarizeSquareMessage(squareMessage, {
          withMedia: false,
        });
        await cacheMessageRow(chatMid, message);
        runtime.setChatCache(null);
        touchChatPreviewFromMessage(message);
        ok(id, { message });
      } catch (e) {
        fail(id, e instanceof Error ? e.message : String(e));
      }
      return;
    }

    let sent:
      | { id?: unknown; createdTime?: unknown; contentType?: unknown }
      | null = null;

    // Prefer plain first (avoids e2ee contentMetadata crashes on some peers), then e2ee.
    try {
      sent = await client.base.talk.sendMessage({
        to: chatMid,
        text,
        e2ee: false,
      }) as unknown as typeof sent;
    } catch (e1) {
      try {
        sent = await client.base.talk.sendMessage({
          to: chatMid,
          text,
          e2ee: true,
        }) as unknown as typeof sent;
      } catch (e2) {
        try {
          const chat = await client.getChat(chatMid);
          const m = await chat.sendMessage(text);
          const message = await summarizeTalkMessage(m, { withMedia: false })
            .catch(() =>
              sentMessagePayload(
                {
                  id: (m as { raw?: { id?: unknown } }).raw?.id,
                  createdTime: Date.now(),
                },
                chatMid,
                text,
              )
            );
          await cacheMessageRow(chatMid, message);
          runtime.setChatCache(null);
          touchChatPreviewFromMessage(message);
          ok(id, { message });
          return;
        } catch (e3) {
          fail(
            id,
            e3 instanceof Error
              ? e3.message
              : e2 instanceof Error
              ? e2.message
              : e1 instanceof Error
              ? e1.message
              : String(e1),
          );
          return;
        }
      }
    }

    runtime.setChatCache(null);
    const message = sentMessagePayload(sent, chatMid, text);
    await cacheMessageRow(chatMid, message);
    touchChatPreviewFromMessage(message);
    ok(id, { message });
  }

  return {
    markRead: doMarkRead,
    loginQr: doLoginQr,
    loginToken: doLoginToken,
    listChats: doListChats,
    fetchMessages: doFetchMessages,
    sendMessage: doSend,
  };
}

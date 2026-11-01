import { fetchMentionMembers, mentionMetadata } from "./mentions.ts";
import { type Client, type TalkMessage } from "@evex/linejs";
import { loginWithQrKeys } from "./key_login.ts";
import { join } from "@std/path";
import { createAuthStorage, restoreSession } from "./auth.ts";
import { authenticateEmail } from "./email_login.ts";
import type { CachePolicy } from "./cache_policy.ts";
import { contactKind, friendName, picturePathOf } from "./contacts.ts";
import type { LineDevice } from "./auth.ts";
import { needsDecryptionRefresh } from "./e2ee_guard.ts";
import { fetchGroupProfiles } from "./groups.ts";
import type { ReadPermission } from "./read_policy.ts";
import { replyMessageId, replySnapshot } from "./replies.ts";
import { createLineFetch } from "./connection.ts";
import { coerceI64 } from "./messages.ts";
import { type ChatRow, contactChatRow } from "./chat_rows.ts";
import {
  clampPageSize,
  mergeMessages,
  newerThan,
  olderThan,
  sortMessages,
} from "./message_history.ts";
import {
  appendLive,
  DELTA_PAGE,
  mergeDelta,
  reachesCursor,
} from "./message_delta.ts";

type Json = Record<string, unknown>;
type ChatCache = { at: number; chats: ChatRow[] };
type BoxCursor = { messageId: bigint | number; deliveredTime: bigint | number };
type ContactInfo = {
  name: string;
  picturePath: string | null;
  kind: string;
  muted: boolean;
};

export type CommandRuntime = {
  getClient: () => Client | null;
  setClient: (client: Client) => void;
  isListening: () => boolean;
  setAuthDead: (dead: boolean) => void;
  getChatCache: () => ChatCache | null;
  setChatCache: (cache: ChatCache | null) => void;
  msgCache: Map<string, { at: number; messages: Json[] }>;
  contactIndex: Map<string, ContactInfo>;
  boxCursor: Map<string, BoxCursor>;
  avatarDir: string;
  chatCachePath: string;
  saveChatCache: () => Promise<void>;
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
  saveDiskContacts: () => Promise<void>;
  refreshContactIndex: (force?: boolean) => Promise<void>;
  hydrateAvatars: (chats: ChatRow[]) => Promise<void>;
  avatarPathFor?: (
    mid: string,
    picturePath?: string | null,
  ) => Promise<string | null>;
  hydratePreviews: (chats: ChatRow[]) => Promise<void>;
  updateChatPreview: (chatMid: string, messages: Json[]) => Promise<void>;
  refreshReadReceipts: (chatMid: string) => Promise<void>;
  reconcileUnread?: (chats: ChatRow[]) => void;
  readReceiptPermission: (chatMid: string) => Promise<ReadPermission>;
  recordOwnRead?: (chatMid: string, through: string) => void;
  bumpMediaEpoch: (chatMid: string) => number;
  loadDiskMessages: (chatMid: string) => Promise<Json[] | null>;
  saveDiskMessages: (
    chatMid: string,
    messages: Json[],
    opts?: { contiguous?: boolean },
  ) => Promise<void>;
  forUiMessages: (messages: Json[], chatMid?: string) => Promise<Json[]>;
  hydrateMedia: (
    messages: Json[],
    chatMid: string,
    opts?: { isolated?: boolean },
  ) => Promise<void>;
  summarizeTalkMessage: (
    message: TalkMessage,
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
  let loginInFlight = false;
  async function withLogin(
    id: number | string | null,
    operation: () => Promise<void>,
  ) {
    if (loginInFlight) {
      runtime.fail(id, "login_in_progress");
      return;
    }
    loginInFlight = true;
    try {
      await operation();
    } finally {
      loginInFlight = false;
    }
  }
  const {
    msgCache,
    contactIndex,
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
    updateChatPreview,
    bumpMediaEpoch,
    loadDiskMessages,
    saveDiskMessages,
    forUiMessages,
    hydrateMedia,
    summarizeTalkMessage,
    summarizeRawMessage,
    saveAuth,
    loadAuth,
    loadAuthDevice,
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
    once = false,
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
      let permission = once
        ? "allowed"
        : await runtime.readReceiptPermission(chatMid);
      if (permission !== "allowed") {
        ok(id, { marked: false, skipped: permission });
        return;
      }
      const requestSequence = client.base.getReqseq;
      const seq = typeof requestSequence === "function"
        ? await requestSequence.call(client.base)
        : Number(requestSequence);
      // 自動既読は、順序番号を待つ間に停止へ切り替えられた場合も送信しない。
      permission = once
        ? "allowed"
        : await runtime.readReceiptPermission(chatMid);
      if (permission !== "allowed" || runtime.getClient() !== client) {
        ok(id, {
          marked: false,
          skipped: permission !== "allowed" ? permission : "session_changed",
        });
        return;
      }
      // thrift STRING (ftype 11) — must be string, not i64/bigint
      await client.base.talk.sendChatChecked({
        chatMid: String(chatMid),
        lastMessageId: String(lastMessageId),
        seq,
      });
      if (runtime.getClient() !== client) {
        ok(id, { marked: false, skipped: "session_changed" });
        return;
      }
      if (once) {
        // 新着が追い越した場合は、既存の未読同期でサーバーの残件数を確認する。
        runtime.recordOwnRead?.(chatMid, lastMessageId);
      } else if (chatCache) {
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

  // 通常の自動既読とは別の、ユーザーが明示的に押す一回限りの操作。
  async function doMarkReadOnce(
    id: number | string | null,
    chatMid: string,
    through = "",
  ) {
    const client = runtime.getClient();
    if (!client || !/^[ucr][a-zA-Z0-9_-]+$/.test(chatMid)) {
      ok(id, { marked: false, skipped: "not_logged_in_or_invalid_chat" });
      return;
    }
    try {
      // 開いているトークはクリック時点の位置を使い、後続の新着まで読まない。
      if (!through) {
        const boxes = await client.base.talk.getMessageBoxes({
          messageBoxListRequest: {},
        });
        const cursor = boxes.messageBoxes?.find((box) => box.id === chatMid)
          ?.lastDeliveredMessageId?.messageId;
        if (typeof cursor === "number" && !Number.isSafeInteger(cursor)) {
          ok(id, { marked: false, skipped: "missing_ids" });
          return;
        }
        through = String(cursor ?? "");
      }
      if (runtime.getClient() !== client) {
        ok(id, { marked: false, skipped: "session_changed" });
        return;
      }
      if (!/^\d+$/.test(through) || BigInt(through) <= 0n) {
        ok(id, { marked: false, skipped: "missing_ids" });
        return;
      }
      await doMarkRead(id, chatMid, through, true);
    } catch {
      ok(id, { marked: false, error: "mark_read_once_failed" });
    }
  }

  async function doLoginQr(id: number | string | null) {
    runtime.setAuthDead(false);
    const storage = await createAuthStorage(storagePath);
    // Docs: QR defaults to ANDROIDSECONDARY for call-capable sessions.
    const device = LINE_DEVICE === "DESKTOPWIN"
      ? "ANDROIDSECONDARY"
      : LINE_DEVICE;
    const client = await loginWithQrKeys(
      {
        onReceiveQRUrl: (url) => emitEvent("qr", { url }),
        onPincodeRequest: (pin) => emitEvent("pin", { pin }),
      },
      {
        device,
        version: LINE_VERSION,
        storage,
        fetch: createLineFetch(90_000),
      },
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
    const device = await loadAuthDevice();
    client = await restoreSession(auth, device, storagePath, LINE_VERSION);
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

  async function doLoginEmail(
    id: number | string | null,
    email: string,
    password: string,
  ) {
    runtime.setAuthDead(false);
    const client = await authenticateEmail(
      email,
      password,
      storagePath,
      (pin) => emitEvent("pin", { pin }),
    );
    runtime.setClient(client);
    const profile = await client.getMyProfile();
    await saveAuth(client.authToken, "DESKTOPWIN");
    patchE2eeGuards();
    await startListen();
    ok(id, { ...(await myProfilePayload(profile)), device: "DESKTOPWIN" });
  }

  async function loadDiskChatCache(): Promise<ChatRow[] | null> {
    try {
      await refreshCachePolicy();
      const raw = JSON.parse(await Deno.readTextFile(chatCachePath));
      const account = String(runtime.getClient()?.base?.profile?.mid ?? "");
      // 別アカウントで保存した一覧は使わない。
      if (raw?.accountMid && account && raw.accountMid !== account) return null;
      if (
        Array.isArray(raw?.chats) &&
        Date.now() - Number(raw.at || 0) < cachePolicy().diskChat
      ) {
        return raw.chats as ChatRow[];
      }
    } catch { /* miss */ }
    return null;
  }

  async function doListChats(
    id: number | string | null,
    force = false,
    quiet = false,
  ) {
    const emitEvent = (event: string, payload?: Json) => {
      if (!quiet || event !== "progress") runtime.emitEvent(event, payload);
    };
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
        runtime.reconcileUnread?.(chatCache.chats);
        ok(id, {
          chats: chatCache.chats,
          count: chatCache.chats.length,
          cached: true,
        });
        void hydratePreviews(chatCache.chats);
        return;
      }

      // Instant warm start from disk while we refresh (event, not second ok).
      if (!force) {
        const disk = await loadDiskChatCache();
        if (disk?.length) {
          runtime.reconcileUnread?.(disk);
          chatCache = { at: 0, chats: disk };
          runtime.setChatCache(chatCache);
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
          const previous = chatCache?.chats.find((row) => row.mid === mid);
          const activity = Number(
            box.lastDeliveredMessageId?.deliveredTime ?? 0,
          );
          // 新着がある場合、前回の本文を最新プレビューとして使い回さない。
          const prevPreview = previous && previous.lastActivity >= activity
            ? previous.preview
            : "";
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
            contactOnly: mid.startsWith("u") && activity <= 0 &&
              Number(box.lastDeliveredMessageId?.messageId ?? 0) <= 0 &&
              Number(box.unreadCount ?? 0) <= 0,
            muted: !!contactIndex.get(mid)?.muted,
          });
        }
      } catch (e) {
        console.error("[list_chats] boxes", e);
        if (force) {
          fail(id, "chat_sync_failed");
          return;
        }
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
            byMid.set(
              mid,
              contactChatRow(
                mid,
                info,
                undefined,
                await existingFile(join(avatarDir, `${mid}.jpg`)),
              ),
            );
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
            const kind = contactKind(
              (user.raw as { contact?: unknown })?.contact,
              contactIndex.get(user.mid)?.kind,
            );
            contactIndex.set(user.mid, {
              name,
              picturePath,
              kind,
              muted: false,
            });
            const prev = byMid.get(user.mid);
            if (prev) {
              prev.name = name;
              prev.picturePath = picturePath;
              prev.kind = kind;
            }
          }
        } catch (e2) {
          console.error("[list_chats] users fallback", e2);
        }
      }

      // グループは専用APIで小分けに取得し、次回起動時にも名前を表示する。
      try {
        const chats = await fetchGroupProfiles(client!.base.talk, [
          ...byMid.keys(),
        ], () => {
          console.error("[groups] 一部のグループ情報を取得できませんでした。");
        });
        for (const chat of chats) {
          const prev = byMid.get(chat.mid);
          const cached = contactIndex.get(chat.mid);
          const name = chat.name !== chat.mid
            ? chat.name
            : prev?.name || cached?.name || chat.mid;
          const picturePath = chat.picturePath ?? cached?.picturePath ??
            prev?.picturePath ?? null;
          contactIndex.set(chat.mid, { ...chat, name, picturePath });
          byMid.set(chat.mid, {
            mid: chat.mid,
            name,
            kind: "group",
            lastActivity: prev?.lastActivity ?? 0,
            unread: prev?.unread ?? 0,
            preview: prev?.preview ?? "",
            picturePath,
            avatarPath: prev?.avatarPath ?? null,
            muted: chat.muted,
          });
        }
        await runtime.saveDiskContacts();
      } catch (e) {
        console.error("[list_chats] groups", e);
      }

      const chats = [...byMid.values()].sort((a, b) => {
        if (b.lastActivity !== a.lastActivity) {
          return b.lastActivity - a.lastActivity;
        }
        return a.name.localeCompare(b.name);
      });

      runtime.reconcileUnread?.(chats);
      chatCache = { at: Date.now(), chats };
      runtime.setChatCache(chatCache);
      try {
        await runtime.saveChatCache();
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

  async function doFetchMessages(
    id: number | string | null,
    chatMid: string,
    limit = 50,
    force = false,
    quiet = false,
  ) {
    const emitEvent = (event: string, payload?: Json) => {
      if (!quiet || event !== "progress") runtime.emitEvent(event, payload);
    };
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    if (!chatMid) {
      fail(id, "missing_chat");
      return;
    }

    await refreshCachePolicy();
    // 本文の取得元にかかわらず、サーバーの既読情報を背景で同期する。
    void runtime.refreshReadReceipts(chatMid);
    await dedupe(`msgs:${chatMid}:${limit}:${force}`, async () => {
      bumpMediaEpoch(chatMid); // cancel any prior hydrate for this chat
      const cached = msgCache.get(chatMid);
      if (
        !force && cached && Date.now() - cached.at < cachePolicy().memMsg &&
        !cached.messages.some(needsDecryptionRefresh)
      ) {
        await updateChatPreview(chatMid, cached.messages);
        const ui = await forUiMessages(cached.messages, chatMid);
        ok(id, { messages: ui, cached: true });
        hydrateMedia(cached.messages, chatMid);
        return;
      }

      // Stored history the refresh below can extend instead of replacing.
      let stored: Json[] | undefined;
      let servedCached = false;

      // Warm from disk immediately
      if (!force) {
        const diskAll = await loadDiskMessages(chatMid);
        const disk = diskAll?.slice(-limit);
        if (diskAll?.length) stored = diskAll;
        if (disk?.length) {
          servedCached = true;
          msgCache.set(chatMid, { at: Date.now(), messages: disk });
          await updateChatPreview(chatMid, disk);
          emitEvent("messages", {
            chatMid,
            messages: await forUiMessages(disk, chatMid),
            cached: true,
          });
          emitEvent("progress", {
            scope: "messages",
            chatMid,
            state: "ready",
          });
          hydrateMedia(disk, chatMid);
          // still refresh below unless memory-fresh
        }
      }

      if (!force && cached?.messages?.length) {
        servedCached = true;
        stored ??= cached.messages;
        await updateChatPreview(chatMid, cached.messages);
        emitEvent("messages", {
          chatMid,
          messages: await forUiMessages(cached.messages, chatMid),
          cached: true,
        });
        emitEvent("progress", {
          scope: "messages",
          chatMid,
          state: "ready",
        });
      }

      const canExtend = !force && !!stored?.length &&
        !stored.some(needsDecryptionRefresh);
      const cursorId = boxCursor.get(chatMid)?.messageId;
      if (canExtend && reachesCursor(stored!, cursorId)) {
        // Nothing newer exists on the server, so skip the network round trip.
        const current = stored!.slice(-limit);
        msgCache.set(chatMid, { at: Date.now(), messages: current });
        ok(id, {
          messages: await forUiMessages(current, chatMid),
          cached: true,
        });
        emitEvent("progress", { scope: "messages", chatMid, state: "ready" });
        return;
      }

      // Messages already on screen are refreshed silently.
      if (!servedCached) {
        emitEvent("progress", { scope: "messages", chatMid, state: "loading" });
      }

      try {
        let out: Json[] = [];

        const cursor = boxCursor.get(chatMid);
        if (canExtend && cursor) {
          try {
            const raw = await client!.base.talk
              .getPreviousMessagesV2WithRequest({
                request: {
                  messageBoxId: chatMid,
                  endMessageId: {
                    messageId: cursor.messageId,
                    deliveredTime: cursor.deliveredTime,
                  },
                  messagesCount: Math.min(DELTA_PAGE, limit),
                },
              });
            const batch: Json[] = [];
            for (const item of raw) {
              try {
                batch.push(
                  await summarizeRawMessage(item, { withMedia: false }),
                );
              } catch (e) {
                console.error("[fetch delta skip]", e);
              }
            }
            const merged = mergeDelta(stored!, batch, limit);
            if (merged) {
              msgCache.set(chatMid, { at: Date.now(), messages: merged });
              await saveDiskMessages(chatMid, batch, { contiguous: true });
              await updateChatPreview(chatMid, merged);
              ok(id, {
                messages: await forUiMessages(merged, chatMid),
                cached: false,
              });
              emitEvent("progress", {
                scope: "messages",
                chatMid,
                state: "ready",
              });
              hydrateMedia(merged, chatMid);
              return;
            }
          } catch (e) {
            console.error("[fetch delta]", e);
          }
        }

        // Prefer message boxes for DMs/bots (fast + works). Groups try Chat helper first.
        let usedBoxes = false;
        try {
          if (chatMid.startsWith("c")) {
            const chat = await client!.getChat(chatMid);
            const messages = await chat.fetchMessages(limit);
            for (const m of messages) {
              out.push(await summarizeTalkMessage(m, { withMedia: false }));
            }
          } else {
            usedBoxes = true;
          }
        } catch {
          usedBoxes = true;
        }

        if (usedBoxes || !out.length) {
          let cursor = boxCursor.get(chatMid);
          if (force || !cursor) {
            const boxes = await client!.base.talk.getMessageBoxes({
              messageBoxListRequest: {},
            });
            for (const box of boxes.messageBoxes ?? []) {
              const mid = String(box.id ?? "");
              storeBoxCursor(mid, box.lastDeliveredMessageId);
            }
            cursor = boxCursor.get(chatMid);
          }
          if (!cursor) {
            msgCache.set(chatMid, { at: Date.now(), messages: [] });
            await updateChatPreview(chatMid, []);
            ok(id, { messages: await forUiMessages([], chatMid) });
            emitEvent("progress", {
              scope: "messages",
              chatMid,
              state: "empty",
            });
            return;
          }
          const messages = await client!.base.talk
            .getPreviousMessagesV2WithRequest({
              request: {
                messageBoxId: chatMid,
                endMessageId: {
                  messageId: cursor.messageId,
                  deliveredTime: cursor.deliveredTime,
                },
                messagesCount: limit,
              },
            });
          out = [];
          let officialChanged = false;
          for (const raw of messages) {
            try {
              const meta = (raw as { contentMetadata?: Record<string, string> })
                .contentMetadata;
              if (meta?.BOT_CHECK || meta?.BOT_ORIGIN) {
                const from = String((raw as { from?: string }).from ?? chatMid);
                const cur = contactIndex.get(from);
                if (cur?.kind !== "bot") officialChanged = true;
                if (cur) cur.kind = "bot";
                else {
                  contactIndex.set(from, {
                    name: contactIndex.get(from)?.name || from,
                    picturePath: null,
                    kind: "bot",
                    muted: false,
                  });
                }
                const row = runtime.getChatCache()?.chats.find((chat) =>
                  chat.mid === from
                );
                if (row && row.kind !== "group" && row.kind !== "bot") {
                  row.kind = "bot";
                  emitEvent("chat_upsert", { chat: { ...row } });
                  officialChanged = true;
                }
              }
              out.push(await summarizeRawMessage(raw, { withMedia: false }));
            } catch (e) {
              console.error("[fetch msg skip]", e);
            }
          }
          if (officialChanged) {
            try {
              await runtime.saveDiskContacts();
              await runtime.saveChatCache();
            } catch (error) {
              console.error(
                "[chat-category] 公式アカウントの分類を保存できませんでした。",
                error,
              );
            }
          }
        }

        out.sort((a, b) => Number(a.createdTime) - Number(b.createdTime));
        msgCache.set(chatMid, { at: Date.now(), messages: out });
        await saveDiskMessages(chatMid, out);
        await updateChatPreview(chatMid, out);
        ok(id, { messages: await forUiMessages(out, chatMid), cached: false });
        emitEvent("progress", {
          scope: "messages",
          chatMid,
          state: out.length ? "ready" : "empty",
        });

        hydrateMedia(out, chatMid);
      } catch (e) {
        fail(id, e instanceof Error ? e.message : String(e));
        emitEvent("progress", {
          scope: "messages",
          chatMid,
          state: "error",
          error: String(e),
        });
      }
    });
  }

  /** Keep the stored history and the newest-message marker in step with a send. */
  function recordSent(chatMid: string, message: Json) {
    const cached = msgCache.get(chatMid);
    if (cached) {
      msgCache.set(chatMid, {
        at: cached.at,
        messages: appendLive(cached.messages, message),
      });
      void saveDiskMessages(chatMid, [message], { contiguous: true });
    }
    // Without a usable id the old marker would hide this message.
    boxCursor.delete(chatMid);
    runtime.storeBoxCursor(chatMid, {
      messageId: message.id || null,
      deliveredTime: message.createdTime,
    });
  }

  async function doFetchOlder(
    id: number | string | null,
    chatMid: string,
    beforeId: string,
    beforeTime: number,
    rawLimit: unknown,
  ) {
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    if (!chatMid || (!beforeId && !beforeTime)) {
      fail(id, "missing_chat");
      return;
    }
    const limit = clampPageSize(rawLimit);
    await dedupe(`older:${chatMid}:${beforeId}`, async () => {
      try {
        const stored = sortMessages(
          (await loadDiskMessages(chatMid)) ?? [],
        );
        let page = olderThan(stored, beforeId, beforeTime, limit);
        let hasMore = true;
        if (page.length < limit) {
          const oldest = page[0] ??
            stored.find((m) => String(m.id) === beforeId);
          const anchorId = String(oldest?.id ?? beforeId);
          const anchorTime = Number(oldest?.createdTime ?? beforeTime);
          if (!anchorId) {
            hasMore = false;
          } else {
            const raw = await client.base.talk.getPreviousMessagesV2WithRequest(
              {
                request: {
                  messageBoxId: chatMid,
                  endMessageId: {
                    messageId: coerceI64(anchorId),
                    deliveredTime: coerceI64(anchorTime > 0 ? anchorTime : 0),
                  },
                  messagesCount: limit,
                },
              },
            );
            const have = new Set(page.map((m) => String(m.id)));
            have.add(beforeId);
            have.add(anchorId);
            const fetched: Json[] = [];
            for (const item of raw) {
              try {
                const summary = await summarizeRawMessage(item, {
                  withMedia: false,
                });
                if (!have.has(String(summary.id))) fetched.push(summary);
              } catch (e) {
                console.error("[fetch older skip]", e);
              }
            }
            // A page served partly from disk may still have older history on
            // the server; only an empty result from both means the end.
            hasMore = fetched.length > 0 || page.length > 0;
            console.error(
              `[history] server returned ${raw.length}, new ${fetched.length}, anchor=${anchorId}`,
            );
            if (fetched.length) {
              await saveDiskMessages(chatMid, fetched, { contiguous: true });
              page = mergeMessages(page, fetched, { contiguous: true })
                .slice(-limit);
            }
          }
        }
        console.error(
          `[history] older chat=${chatMid} before=${beforeId} page=${page.length} hasMore=${hasMore}`,
        );
        const cutoff = page.length
          ? Number(page[page.length - 1].createdTime)
          : 0;
        const ui = (await forUiMessages(page, chatMid)).filter((m) =>
          Number(m.createdTime) <= cutoff
        );
        ok(id, { messages: ui, hasMore, chatMid });
        hydrateMedia(page, chatMid, { isolated: true });
      } catch (e) {
        fail(id, e instanceof Error ? e.message : String(e));
      }
    });
  }

  async function doFetchNewerCached(
    id: number | string | null,
    chatMid: string,
    afterId: string,
    afterTime: number,
    rawLimit: unknown,
  ) {
    if (!chatMid || (!afterId && !afterTime)) {
      fail(id, "missing_chat");
      return;
    }
    const limit = clampPageSize(rawLimit);
    try {
      const stored = sortMessages((await loadDiskMessages(chatMid)) ?? []);
      const { messages, hasMore } = newerThan(
        stored,
        afterId,
        afterTime,
        limit,
      );
      const cutoff = hasMore && messages.length
        ? Number(messages[messages.length - 1].createdTime)
        : Infinity;
      const ui = (await forUiMessages(messages, chatMid)).filter((m) =>
        Number(m.createdTime) <= cutoff
      );
      ok(id, { messages: ui, hasMore, chatMid });
      hydrateMedia(messages, chatMid, { isolated: true });
    } catch (e) {
      fail(id, e instanceof Error ? e.message : String(e));
    }
  }

  async function doSend(
    id: number | string | null,
    chatMid: string,
    text: string,
    replyToMessageId?: unknown,
    rawMentions?: unknown,
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

    const { mentions, contentMetadata } = mentionMetadata(text, rawMentions);
    if (mentions.length && !chatMid.startsWith("c")) {
      fail(id, "invalid_group");
      return;
    }

    const relatedMessageId = replyToMessageId == null
      ? undefined
      : replyMessageId(replyToMessageId);
    if (relatedMessageId === null) {
      fail(id, "invalid_reply_target");
      return;
    }
    const original = msgCache.get(chatMid)?.messages.find((m) =>
      m.id === relatedMessageId
    );
    const replyTo = relatedMessageId
      ? (original ? replySnapshot(original) : { messageId: relatedMessageId })
      : null;

    let sent:
      | { id?: unknown; createdTime?: unknown; contentType?: unknown }
      | null = null;

    // Prefer plain first (avoids e2ee contentMetadata crashes on some peers), then e2ee.
    try {
      sent = await client.base.talk.sendMessage({
        to: chatMid,
        text,
        contentMetadata,
        relatedMessageId,
        e2ee: false,
      }) as unknown as typeof sent;
    } catch (e1) {
      try {
        sent = await client.base.talk.sendMessage({
          to: chatMid,
          text,
          contentMetadata,
          relatedMessageId,
          e2ee: true,
        }) as unknown as typeof sent;
      } catch (e2) {
        try {
          const chat = await client.getChat(chatMid);
          const m = await chat.sendMessage({
            text,
            contentMetadata,
            relatedMessageId,
          });
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
          message.replyTo = replyTo;
          message.mentions = mentions;
          recordSent(chatMid, message);
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
    const message = {
      ...sentMessagePayload(sent, chatMid, text),
      replyTo,
      mentions,
    };
    recordSent(chatMid, message);
    touchChatPreviewFromMessage(message);
    ok(id, { message });
  }

  return {
    markRead: doMarkRead,
    markReadOnce: doMarkReadOnce,
    loginQr: (id: number | string | null) => withLogin(id, () => doLoginQr(id)),
    loginEmail: (id: number | string | null, email: string, password: string) =>
      withLogin(id, () => doLoginEmail(id, email, password)),
    loginToken: (id: number | string | null, token?: string) =>
      withLogin(id, () => doLoginToken(id, token)),
    listChats: doListChats,
    fetchMessages: doFetchMessages,
    fetchOlderMessages: doFetchOlder,
    fetchNewerCachedMessages: doFetchNewerCached,
    sendMessage: doSend,
    async mentionMembers(id: number | string | null, chatMid: string) {
      const client = runtime.getClient();
      if (!client) {
        fail(id, "not_logged_in");
        return;
      }
      const members = await fetchMentionMembers(client, chatMid);
      if (runtime.getClient() !== client) {
        fail(id, "session_changed");
        return;
      }
      const withAvatars = await Promise.all(
        members.map(async ({ mid, name }) => ({
          mid,
          name,
          avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
        })),
      );
      ok(id, { chatMid, members: withAvatars });
      const missing = members.filter((m, i) =>
        m.picturePath && !withAvatars[i].avatarPath
      );
      const download = runtime.avatarPathFor;
      if (download && missing.length) {
        void (async () => {
          for (let i = 0; i < missing.length; i += 6) {
            await Promise.all(
              missing.slice(i, i + 6).map(async (m) => {
                try {
                  const path = await download(m.mid, m.picturePath);
                  if (path && runtime.getClient() === client) {
                    emitEvent("avatar_ready", { mid: m.mid, avatarPath: path });
                  }
                } catch (e) {
                  console.error("[mentionAvatar]", e);
                }
              }),
            );
          }
        })();
      }
    },
  };
}

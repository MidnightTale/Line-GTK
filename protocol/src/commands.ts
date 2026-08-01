import {
  type Client,
  loginWithAuthToken,
  loginWithQR,
  type TalkMessage,
} from "@evex/linejs";
import { FileStorage } from "@evex/linejs/storage";
import { join } from "@std/path";
import { atomicWriteJson } from "./storage.ts";
import type { CachePolicy } from "./cache_policy.ts";
import { friendName, picturePathOf } from "./contacts.ts";
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
};
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
  loadDiskMessages: (chatMid: string) => Promise<Json[] | null>;
  saveDiskMessages: (chatMid: string, messages: Json[]) => Promise<void>;
  forUiMessages: (messages: Json[]) => Promise<Json[]>;
  hydrateMedia: (messages: Json[], chatMid: string) => Promise<void>;
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
            picturePath: prev?.picturePath ?? null,
            avatarPath: prev?.avatarPath ?? null,
          });
        }
      } catch (e) {
        console.error("[list_chats] groups", e);
      }

      const chats = [...byMid.values()].sort((a, b) => {
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

  async function doFetchMessages(
    id: number | string | null,
    chatMid: string,
    limit = 50,
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

    await refreshCachePolicy();
    await dedupe(`msgs:${chatMid}:${limit}:${force}`, async () => {
      bumpMediaEpoch(chatMid); // cancel any prior hydrate for this chat
      const cached = msgCache.get(chatMid);
      if (!force && cached && Date.now() - cached.at < cachePolicy().memMsg) {
        const ui = await forUiMessages(cached.messages);
        ok(id, { messages: ui, cached: true });
        hydrateMedia(cached.messages, chatMid);
        return;
      }

      // Warm from disk immediately
      if (!force) {
        const disk = await loadDiskMessages(chatMid);
        if (disk?.length) {
          msgCache.set(chatMid, { at: Date.now(), messages: disk });
          emitEvent("messages", {
            chatMid,
            messages: await forUiMessages(disk),
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
        emitEvent("messages", {
          chatMid,
          messages: await forUiMessages(cached.messages),
          cached: true,
        });
        emitEvent("progress", {
          scope: "messages",
          chatMid,
          state: "ready",
        });
      }

      emitEvent("progress", { scope: "messages", chatMid, state: "loading" });

      try {
        let out: Json[] = [];

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
          if (!cursor) {
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
            ok(id, { messages: [] });
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
          for (const raw of messages) {
            try {
              const meta = (raw as { contentMetadata?: Record<string, string> })
                .contentMetadata;
              if (meta?.BOT_CHECK || meta?.BOT_ORIGIN) {
                const from = String((raw as { from?: string }).from ?? chatMid);
                const cur = contactIndex.get(from);
                if (cur) cur.kind = "bot";
                else {
                  contactIndex.set(from, {
                    name: contactIndex.get(from)?.name || from,
                    picturePath: null,
                    kind: "bot",
                    muted: false,
                  });
                }
              }
              out.push(await summarizeRawMessage(raw, { withMedia: false }));
            } catch (e) {
              console.error("[fetch msg skip]", e);
            }
          }
        }

        out.sort((a, b) => Number(a.createdTime) - Number(b.createdTime));
        msgCache.set(chatMid, { at: Date.now(), messages: out });
        await saveDiskMessages(chatMid, out);
        ok(id, { messages: await forUiMessages(out), cached: false });
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
          msgCache.delete(chatMid);
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

    msgCache.delete(chatMid);
    runtime.setChatCache(null);
    const message = sentMessagePayload(sent, chatMid, text);
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

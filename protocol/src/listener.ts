import type { Client, SquareMessage, TalkMessage } from "@evex/linejs";
import * as calls from "./calls.ts";
import { talkChatMid } from "./messages.ts";

type Json = Record<string, unknown>;

export type ListenerRuntime = {
  getClient: () => Client | null;
  isListening: () => boolean;
  setListening: (listening: boolean) => void;
  cacheMessage: (chatMid: string, message: Json) => Promise<void>;
  summarizeTalkMessage: (
    message: TalkMessage,
    opts?: { withMedia?: boolean },
  ) => Promise<Json>;
  summarizeSquareMessage: (
    message: SquareMessage,
    opts?: { withMedia?: boolean },
  ) => Promise<Json>;
  storeBoxCursor: (
    mid: string,
    value?: { messageId?: unknown; deliveredTime?: unknown } | null,
  ) => void;
  upsertChatFromMessage: (message: Json) => Promise<void>;
  hydrateLiveMedia: (
    message: TalkMessage,
    summary: Json,
    chatMid: string,
  ) => Promise<void>;
  upsertChatFromContact: (mid: string) => Promise<void>;
  emitEvent: (event: string, payload?: Json) => void;
};

export function createListener(runtime: ListenerRuntime) {
  const {
    summarizeTalkMessage,
    summarizeSquareMessage,
    storeBoxCursor,
    upsertChatFromMessage,
    hydrateLiveMedia,
    upsertChatFromContact,
    emitEvent,
  } = runtime;
  const seenLiveMessageIds = new Set<string>();

  function isDuplicateLiveMessage(message: Json): boolean {
    const id = String(message.id ?? "");
    if (!id) return false;
    if (seenLiveMessageIds.has(id)) return true;
    if (seenLiveMessageIds.size >= 4096) seenLiveMessageIds.clear();
    seenLiveMessageIds.add(id);
    return false;
  }

  function startListen() {
    const client = runtime.getClient();
    if (!client || runtime.isListening()) return;
    runtime.setListening(true);

    // Register BEFORE listen() so early ops are not missed.
    client.on("message", async (msg) => {
      try {
        const tm = msg as TalkMessage;
        const message = await summarizeTalkMessage(tm, { withMedia: false });
        if (isDuplicateLiveMessage(message)) return;
        const peer = talkChatMid(message);
        if (!peer) return;
        message.chatMid = peer;
        void runtime.cacheMessage(peer, message).catch((error) =>
          console.error("[cache live message]", error)
        );
        // Advance box cursor so later refetch can see this message.
        storeBoxCursor(peer, {
          messageId: message.id,
          deliveredTime: message.createdTime,
        });
        emitEvent("message", { message: { ...message, imagePath: null } });
        await upsertChatFromMessage(message);
        if (message.needsMedia) {
          // Use the live TalkMessage.getData path — history refetch often misses
          // brand-new messages and left bubbles on "Image unavailable" until restart.
          void hydrateLiveMedia(tm, message, peer);
        }
      } catch (e) {
        console.error("[listen message]", e);
      }
    });

    client.on("event", (op: {
      type?: string | number;
      param1?: string;
      param2?: string;
      param3?: string;
    }) => {
      try {
        const type = String(op?.type ?? "");
        if (type === "NOTIFIED_READ_MESSAGE") {
          emitEvent("read_receipt", {
            chatMid: String(op.param1 ?? ""),
            userMid: String(op.param2 ?? ""),
            messageId: String(op.param3 ?? ""),
          });
        } else if (
          type.includes("ADD_CONTACT") ||
          type.includes("ACCEPT_CONTACT") ||
          type.includes("FRIEND_REQUEST") ||
          type === "NOTIFIED_UPDATE_PROFILE"
        ) {
          const mid = String(op.param1 ?? op.param2 ?? "");
          if (mid) void upsertChatFromContact(mid);
        }
      } catch (e) {
        console.error("[listen event]", e);
      }
    });

    client.on("square:message", async (msg) => {
      try {
        const message = await summarizeSquareMessage(msg, {
          withMedia: false,
        });
        if (isDuplicateLiveMessage(message)) return;
        const chatMid = String(message.chatMid ?? message.to ?? "");
        if (!chatMid) return;
        void runtime.cacheMessage(chatMid, message).catch((error) =>
          console.error("[cache live openchat message]", error)
        );
        emitEvent("message", { message });
        await upsertChatFromMessage(message);
      } catch (e) {
        console.error("[listen openchat message]", e);
      }
    });

    client.on(
      "call:incoming",
      calls.handleIncomingCall,
    );
    client.on(
      "call:cancel",
      calls.handleCallCancel,
    );

    client.listen({ talk: true, square: true });
    emitEvent("listening");
  }

  return { start: startListen };
}

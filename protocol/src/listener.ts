import type { Client, TalkMessage } from "@evex/linejs";
import * as calls from "./calls.ts";

type Json = Record<string, unknown>;

export type ListenerRuntime = {
  getClient: () => Client | null;
  isListening: () => boolean;
  setListening: (listening: boolean) => void;
  invalidateMessages: (chatMid: string) => void;
  summarizeTalkMessage: (
    message: TalkMessage,
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
    storeBoxCursor,
    upsertChatFromMessage,
    hydrateLiveMedia,
    upsertChatFromContact,
    emitEvent,
  } = runtime;

  function startListen() {
    const client = runtime.getClient();
    if (!client || runtime.isListening()) return;
    runtime.setListening(true);

    // Register BEFORE listen() so early ops are not missed.
    client.on("message", async (msg) => {
      try {
        const tm = msg as TalkMessage;
        const message = await summarizeTalkMessage(tm, { withMedia: false });
        const peer = message.mine ? String(message.to) : String(message.from);
        runtime.invalidateMessages(peer);
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

    client.on(
      "call:incoming",
      calls.handleIncomingCall,
    );
    client.on(
      "call:cancel",
      calls.handleCallCancel,
    );

    client.listen({ talk: true, square: false });
    emitEvent("listening");
  }

  return { start: startListen };
}

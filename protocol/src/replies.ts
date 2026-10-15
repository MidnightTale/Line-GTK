import { conversationMid } from "./messages.ts";

type Json = Record<string, unknown>;
export type ReplyInfo = {
  messageId: string;
  text?: string;
  from?: string;
  mine?: boolean;
  contentType?: string;
};

// 64bitを超えるIDも丸めず、未送信の仮IDは返信先に使わない。
export function replyMessageId(value: unknown): string | null {
  if (typeof value === "number" && !Number.isSafeInteger(value)) return null;
  if (!["string", "number", "bigint"].includes(typeof value)) return null;
  const id = String(value);
  return /^[1-9][0-9]{0,38}$/.test(id) ? id : null;
}

export function replyToOf(raw: Json): ReplyInfo | null {
  const cached = raw.replyTo as ReplyInfo | null | undefined;
  if (cached && replyMessageId(cached.messageId)) return cached;
  if (raw.messageRelationType !== "REPLY" && raw.messageRelationType !== 3) {
    return null;
  }
  if (
    raw.relatedMessageServiceCode != null &&
    raw.relatedMessageServiceCode !== "TALK" &&
    raw.relatedMessageServiceCode !== 1
  ) return null;
  const messageId = replyMessageId(raw.relatedMessageId);
  return messageId ? { messageId } : null;
}

export function replySnapshot(message: Json): ReplyInfo {
  return {
    messageId: String(message.id),
    text: message.decryptionFailed === true
      ? ""
      : Array.from(String(message.text ?? "")).slice(0, 512).join(""),
    from: String(message.from ?? ""),
    mine: message.mine === true,
    contentType: message.decryptionFailed === true
      ? "UNAVAILABLE"
      : String(message.contentType ?? "NONE"),
  };
}

// 同じトークの履歴だけで引用文を補う。元の本文が範囲外でも返信先IDは保持する。
export function withReplyPreviews(messages: Json[]): Json[] {
  const originals = new Map(
    messages.map((m) => [`${conversationMid(m)}:${m.id}`, m]),
  );
  return messages.map((message) => {
    const replyTo = replyToOf(message);
    if (!replyTo) return { ...message, replyTo: null };
    const original = originals.get(
      `${conversationMid(message)}:${replyTo.messageId}`,
    );
    return {
      ...message,
      replyTo: original ? replySnapshot(original) : replyTo,
    };
  });
}

import { normalizeRaw } from "./messages.ts";

type RawMessage = Record<string, unknown>;
export const DECRYPTION_FAILED = "LINE_GTK_DECRYPTION_FAILED";

// 復号できない行も履歴に残す。本文や暗号鍵をログに出さない。
export function guardedDecrypt(
  decrypt: (message: RawMessage) => Promise<RawMessage>,
  onFailure: () => void,
) {
  return async (raw: RawMessage): Promise<RawMessage> => {
    const message = normalizeRaw(raw);
    try {
      return await decrypt(message);
    } catch {
      onFailure();
      return {
        ...message,
        text: "",
        contentMetadata: {
          ...(message.contentMetadata as Record<string, string>),
          [DECRYPTION_FAILED]: "1",
        },
      };
    }
  };
}

// 旧版が保存した空の本文も再取得し、誤ったキャッシュを固定しない。
export function needsDecryptionRefresh(message: RawMessage): boolean {
  return message.decryptionFailed === true ||
    (!message.text &&
      ["NONE", "TEXT"].includes(String(message.contentType ?? "NONE")));
}

export function restoreCachedMessages(messages: RawMessage[]): RawMessage[] {
  return messages.map((message) =>
    needsDecryptionRefresh(message)
      ? { ...message, decryptionFailed: true }
      : message
  );
}

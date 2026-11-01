import { normalizeRaw } from "./messages.ts";
import { InternalError } from "@evex/linejs/base";
import { failureKind } from "./connection.ts";

type RawMessage = Record<string, unknown>;
export const DECRYPTION_FAILED = "LINE_GTK_DECRYPTION_FAILED";

function keyMayArrive(error: unknown): boolean {
  if (["network", "server"].includes(failureKind(error))) return true;
  return error instanceof InternalError &&
    (["NoE2EEKey", "No E2EEKey"].includes(error.name) ||
      ["5", "NOT_FOUND", "99", "E2EE_RECREATE_GROUP_KEY"].includes(
        String(error.data.code),
      ));
}

// Hold the message while refetching missing keys, so the live receive loop
// publishes it once and in order. Only reads are retried, never message sends.
export function retryKeyDecryption(
  decrypt: (message: RawMessage) => Promise<RawMessage>,
  active: () => boolean,
  sleep = (ms: number) =>
    new Promise<void>((resolve) => setTimeout(resolve, ms)),
) {
  return async (raw: RawMessage): Promise<RawMessage> => {
    for (let attempt = 0;; attempt++) {
      if (!active()) throw new Error("receiver_session_changed");
      // A failed decryptor can mutate its argument. Each attempt must start
      // with the original ciphertext and metadata, without a failure marker.
      const message = normalizeRaw(raw);
      message.contentMetadata = {
        ...(message.contentMetadata as Record<string, string>),
      };
      const metadata = message.contentMetadata as Record<string, string>;
      delete metadata[DECRYPTION_FAILED];
      delete message.decryptionFailed;
      if (Array.isArray(raw.chunks)) message.chunks = raw.chunks.slice();
      try {
        const result = await decrypt(message);
        if (!active()) throw new Error("receiver_session_changed");
        return result;
      } catch (error) {
        if (
          !active() || attempt >= 2 || !Array.isArray(raw.chunks) ||
          raw.chunks.length !== 5 ||
          !["1", "2"].includes(String(metadata.e2eeVersion ?? "2")) ||
          !keyMayArrive(error)
        ) throw error;
        await sleep(attempt === 0 ? 250 : 750);
      }
    }
  };
}

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

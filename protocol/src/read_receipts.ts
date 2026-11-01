import type { Client } from "@evex/linejs";

type Json = Record<string, unknown>;
export type ReadOperation = {
  type?: string | number;
  param1?: string;
  param2?: string;
  param3?: string;
};
export type ReaderRanges = {
  userMid: string;
  ranges: { startMessageId: string; endMessageId: string }[];
};

function record(value: unknown): Json {
  return value && typeof value === "object" ? value as Json : {};
}

function messageId(value: unknown): string | null {
  if (
    typeof value === "number" && (!Number.isSafeInteger(value) || value < 0)
  ) return null;
  if (!["number", "bigint", "string"].includes(typeof value)) return null;
  const id = String(value);
  return /^\d+$/.test(id) ? BigInt(id).toString() : null;
}

export function normalizeReadRanges(
  raw: unknown,
  chatMid: string,
  selfMid: string,
): ReaderRanges[] {
  if (!Array.isArray(raw) || !selfMid) return [];
  const readers = new Map<string, ReaderRanges["ranges"]>();
  for (const value of raw) {
    const chat = record(value);
    if (chat.chatId !== chatMid) continue;
    for (const [userMid, input] of Object.entries(record(chat.ranges))) {
      if (
        !userMid.startsWith("u") || userMid === selfMid ||
        (chatMid.startsWith("u") && userMid !== chatMid)
      ) continue;
      // linejsではリストが数値キーのオブジェクトとして返る場合もある。
      const object = record(input);
      const entries = Array.isArray(input)
        ? input
        : messageId(object.startMessageId ?? object["1"]) != null &&
            messageId(object.endMessageId ?? object["2"]) != null
        ? [object]
        : Object.values(object);
      const ranges = readers.get(userMid) ?? [];
      for (const value of entries) {
        const entry = record(value);
        const start = messageId(entry.startMessageId ?? entry["1"]);
        const end = messageId(entry.endMessageId ?? entry["2"]);
        if (start == null || end == null || BigInt(start) > BigInt(end)) {
          continue;
        }
        ranges.push({ startMessageId: start, endMessageId: end });
      }
      if (ranges.length) readers.set(userMid, ranges);
    }
  }
  return [...readers].map(([userMid, ranges]) => ({ userMid, ranges }));
}

type ReadReceiptRuntime = {
  getClient: () => Client | null;
  selfMid: () => string;
  emit: (event: string, payload: Json) => void;
};

export function createReadReceipts(runtime: ReadReceiptRuntime) {
  const pending = new Map<string, { client: Client; task: Promise<void> }>();
  function refresh(chatMid: string): Promise<void> {
    const client = runtime.getClient();
    const selfMid = runtime.selfMid();
    if (!client || !selfMid || !/^[ucr]/.test(chatMid)) {
      return Promise.resolve();
    }
    const previous = pending.get(chatMid);
    if (previous?.client === client) return previous.task;
    const task = (async () => {
      try {
        const response = await client.base.talk.getMessageReadRange({
          chatIds: [chatMid],
          syncReason: "INTERNAL",
        });
        if (runtime.getClient() !== client || runtime.selfMid() !== selfMid) {
          return;
        }
        runtime.emit("read_ranges", {
          chatMid,
          readers: normalizeReadRanges(response, chatMid, selfMid),
        });
      } catch {
        // 既読情報が取れなくても本文を表示し、確認済みの状態は維持する。
        console.error("[read-receipts] 既読情報を取得できませんでした。");
      }
    })();
    const job = { client, task };
    pending.set(chatMid, job);
    void task.finally(() => {
      if (pending.get(chatMid) === job) pending.delete(chatMid);
    });
    return task;
  }

  function handleOperation(operation: ReadOperation): boolean {
    if (!["NOTIFIED_READ_MESSAGE", "55"].includes(String(operation.type))) {
      return false;
    }
    const { param1: chatMid = "", param2: userMid = "" } = operation;
    const id = messageId(operation.param3);
    const selfMid = runtime.selfMid();
    if (
      !selfMid || !chatMid || !userMid.startsWith("u") || userMid === selfMid ||
      id == null ||
      (chatMid.startsWith("u") && userMid !== chatMid)
    ) return true;
    runtime.emit("read_receipt", { chatMid, userMid, messageId: id });
    // グループでは読み始めた位置も確認し、参加前などの履歴を既読にしない。
    if (/^[cr]/.test(chatMid)) void refresh(chatMid);
    return true;
  }
  return { refresh, handleOperation };
}

import type { Client } from "@evex/linejs";
import type { ReadOperation } from "./read_receipts.ts";
import { conversationMid } from "./messages.ts";

type Json = Record<string, unknown>;
type Chat = { mid: string; unread: number };
type Runtime = {
  getClient: () => Client | null;
  selfMid: () => string;
  latestMessageId: (mid: string) => unknown;
  getChats: () => Chat[];
  save: () => Promise<void>;
  emit: (event: string, payload: Json) => void;
};

function numericId(value: unknown): bigint | null {
  if (typeof value === "number" && !Number.isSafeInteger(value)) return null;
  return ["string", "number", "bigint"].includes(typeof value) &&
      /^\d+$/.test(String(value))
    ? BigInt(String(value))
    : null;
}

export function createUnreadSync(runtime: Runtime) {
  let owner: Client | null = null;
  let account = "";
  const reads = new Map<string, bigint>();
  const latestSeen = new Map<string, bigint>();
  const versions = new Map<string, number>();
  const pending = new Map<string, Promise<void>>();
  function session() {
    const client = runtime.getClient();
    const mid = runtime.selfMid();
    if (owner !== client || account !== mid) {
      owner = client;
      account = mid;
      reads.clear();
      latestSeen.clear();
      versions.clear();
      pending.clear();
    }
    return { client, mid };
  }
  function advance(mid: string) {
    versions.set(mid, (versions.get(mid) ?? 0) + 1);
  }
  function latestId(mid: string): bigint | null {
    const cursor = numericId(runtime.latestMessageId(mid));
    const seen = latestSeen.get(mid);
    return seen != null && (cursor == null || seen > cursor) ? seen : cursor;
  }
  function reconcile(chats: Chat[]) {
    session();
    for (const chat of chats) {
      const read = reads.get(chat.mid);
      const latest = latestId(chat.mid);
      if (read != null && latest != null && read >= latest) chat.unread = 0;
    }
  }
  function publish(chatMid: string, through: bigint, unread: number) {
    const chat = runtime.getChats().find((chat) => chat.mid === chatMid);
    if (chat) chat.unread = unread;
    runtime.emit("chat_unread", {
      chatMid,
      accountMid: account,
      throughMessageId: String(through),
      unread,
    });
    void runtime.save().catch(() => {
      console.error("[unread-sync] 未読状態を保存できませんでした。");
    });
  }
  function refresh(chatMid: string) {
    if (pending.has(chatMid)) return;
    const { client, mid } = session();
    if (!client || !mid) return;
    const active = () =>
      runtime.getClient() === client && runtime.selfMid() === mid;
    const task = (async () => {
      // 照会中に新着や別の既読通知が来た場合は、古い件数を捨てて取り直す。
      for (let attempt = 0; attempt < 3 && active(); attempt++) {
        const version = versions.get(chatMid);
        try {
          const result = await client.base.talk.getMessageBoxes({
            messageBoxListRequest: {},
          });
          if (!active()) return;
          if (versions.get(chatMid) !== version) continue;
          const box = result.messageBoxes?.find((box) => box.id === chatMid);
          const through = numericId(box?.lastDeliveredMessageId?.messageId);
          const count = Number(box?.unreadCount);
          if (through == null || !Number.isSafeInteger(count) || count < 0) {
            return;
          }
          const read = reads.get(chatMid);
          publish(
            chatMid,
            through,
            read != null && read >= through ? 0 : count,
          );
          return;
        } catch {
          // 通信失敗では未読を消さない。次の同期や復帰時の一覧取得で再確認する。
          console.error(
            "[unread-sync] サーバーの未読状態を取得できませんでした。",
          );
          return;
        }
      }
    })();
    pending.set(chatMid, task);
    void task.finally(() => {
      if (pending.get(chatMid) === task) pending.delete(chatMid);
    });
  }
  function handleOperation(op: ReadOperation, retry = false): boolean {
    const type = String(op.type);
    const selfChecked = ["SEND_CHAT_CHECKED", "40"].includes(type);
    const selfNotified = ["NOTIFIED_READ_MESSAGE", "55"].includes(type) &&
      op.param2 === runtime.selfMid();
    if (!selfChecked && !selfNotified) return false;
    const { client, mid } = session();
    const chatMid = op.param1 ?? "";
    const id = numericId(selfChecked ? op.param2 : op.param3);
    if (!client || !mid || !/^[ucr]/.test(chatMid) || id == null) return true;
    const previous = reads.get(chatMid);
    if (previous != null && previous >= id) {
      // 手動操作をやり直した場合は、前回失敗した件数照会も再試行する。
      if (retry) {
        const latest = latestId(chatMid);
        if (latest != null && previous >= latest) publish(chatMid, previous, 0);
        else refresh(chatMid);
      }
      return true;
    }
    reads.set(chatMid, id);
    advance(chatMid);
    const latest = latestId(chatMid);
    if (latest != null && id >= latest) publish(chatMid, id, 0);
    else refresh(chatMid);
    return true;
  }
  function observeMessage(message: Json) {
    session();
    const mid = conversationMid(message);
    const id = numericId(message.id);
    const read = reads.get(mid);
    const latest = latestId(mid);
    // スマホで読んだ後の新着は、メモリ一覧にも残す。古い受信や重複は数えない。
    if (
      !message.mine && id != null && read != null && id > read &&
      (latest == null || id > latest)
    ) {
      const chat = runtime.getChats().find((chat) => chat.mid === mid);
      if (chat) chat.unread++;
    }
    if (id != null && id > (latestSeen.get(mid) ?? -1n)) {
      latestSeen.set(mid, id);
    }
    advance(mid);
  }
  return { handleOperation, observeMessage, reconcile };
}

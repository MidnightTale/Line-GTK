import { isLegacyCallPreview } from "./call_history.ts";

type Json = Record<string, unknown>;

export type PreviewChat = {
  mid: string;
  preview: string;
  lastActivity: number;
  contactOnly?: boolean;
};

type PreviewRuntime = {
  getChats: () => PreviewChat[];
  previewLine: (message: Json) => string;
  fallback: () => string;
  emit: (chat: PreviewChat) => void;
  save: () => Promise<void>;
};

// APIの返却順に依存せず、同時刻なら64ビットのメッセージIDで比較する。
function newestMessage(messages: Json[]): Json | undefined {
  return messages.reduce<Json | undefined>((newest, message) => {
    if (!newest) return message;
    const difference = Number(message.createdTime ?? 0) -
      Number(newest.createdTime ?? 0);
    if (difference !== 0) return difference > 0 ? message : newest;
    const id = String(message.id ?? "");
    const previous = String(newest.id ?? "");
    return /^\d+$/.test(id) && /^\d+$/.test(previous) &&
        BigInt(id) > BigInt(previous)
      ? message
      : newest;
  }, undefined);
}

// 言語切り替え前のキャッシュにある案内文も再取得の対象にする。
export function needsChatPreview(chat: PreviewChat): boolean {
  return chat.lastActivity > 0 &&
    (!chat.preview || isLegacyCallPreview(chat.preview) || [
      "Tap to open",
      "แตะเพื่อเปิด",
      "タップして開く",
    ].includes(chat.preview));
}

export function createChatPreviews(runtime: PreviewRuntime) {
  const inFlight = new Map<string, {
    active: () => boolean;
    task: Promise<void>;
  }>();
  async function updateFromMessages(chatMid: string, messages: Json[]) {
    const chat = runtime.getChats().find((row) => row.mid === chatMid);
    if (!chat) return;
    const newest = newestMessage(messages);
    // 古い履歴やキャッシュで、受信済みの新しいメッセージを上書きしない。
    if (newest && Number(newest.createdTime ?? 0) < chat.lastActivity) return;
    if (!newest && chat.preview && !needsChatPreview(chat)) return;
    const preview = newest ? runtime.previewLine(newest) : runtime.fallback();
    const promoted = !!newest && chat.contactOnly === true;
    if (chat.preview === preview && !promoted) return;
    if (promoted) chat.contactOnly = false;
    chat.preview = preview;
    chat.lastActivity = Math.max(
      chat.lastActivity,
      Number(newest?.createdTime ?? 0),
    );
    runtime.emit({
      mid: chat.mid,
      preview: chat.preview,
      lastActivity: chat.lastActivity,
      ...(promoted ? { contactOnly: false } : {}),
    });
    await runtime.save();
  }

  async function hydrate(
    chats: PreviewChat[],
    fetch: (chat: PreviewChat) => Promise<Json[]>,
    active: () => boolean,
  ) {
    // 件数で打ち切らず、同時通信を2件に抑えて末尾まで処理する。
    const need = chats.filter(needsChatPreview);
    let next = 0;
    async function run() {
      while (next < need.length && active()) {
        const chat = need[next++];
        const pending = inFlight.get(chat.mid);
        if (pending?.active()) {
          await pending.task;
          continue;
        }
        const activity = chat.lastActivity;
        const previous = chat.preview;
        const task = (async () => {
          let messages: Json[] = [];
          try {
            messages = await fetch(chat);
          } catch {
            // 失敗・空応答・カーソルなしでも読み込み中を終了する。
          }
          if (!active()) return;
          const current = runtime.getChats().find((row) =>
            row.mid === chat.mid
          );
          if (
            !current || current.lastActivity !== activity ||
            current.preview !== previous
          ) return;
          const newest = newestMessage(messages);
          // 最新位置より古い履歴だけ返った場合も案内文に戻して再試行可能にする。
          await updateFromMessages(
            chat.mid,
            newest && Number(newest.createdTime ?? 0) >= activity
              ? messages
              : [],
          );
        })();
        const job = { active, task };
        inFlight.set(chat.mid, job);
        try {
          await task;
        } finally {
          if (inFlight.get(chat.mid) === job) inFlight.delete(chat.mid);
        }
      }
    }
    await Promise.all(Array.from({ length: Math.min(2, need.length) }, run));
  }

  return { updateFromMessages, hydrate };
}

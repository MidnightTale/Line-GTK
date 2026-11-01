export type ReadPermission =
  | "allowed"
  | "paused"
  | "chat_paused"
  | "auto_disabled"
  | "unavailable";

// UIの判定だけに頼らず、既読送信の直前に保存済み設定を確認する。
export async function readReceiptPermission(
  path: string,
  account = "",
  chatMid = "",
): Promise<ReadPermission> {
  try {
    const settings = JSON.parse(await Deno.readTextFile(path));
    if (!settings || typeof settings !== "object" || Array.isArray(settings)) {
      return "unavailable";
    }
    if (
      (settings.pause_read_receipts !== undefined &&
        typeof settings.pause_read_receipts !== "boolean") ||
      (settings.auto_mark_read !== undefined &&
        typeof settings.auto_mark_read !== "boolean")
    ) return "unavailable";
    const chats = settings.read_receipts_paused_chats;
    if (chats !== undefined) {
      if (!chats || typeof chats !== "object" || Array.isArray(chats)) {
        return "unavailable";
      }
      for (const [owner, mids] of Object.entries(chats)) {
        if (
          !owner.trim() || !Array.isArray(mids) ||
          !mids.every((mid) => typeof mid === "string" && mid.trim())
        ) {
          return "unavailable";
        }
      }
      // 相手別の設定があるのに送信元・送信先を確認できない場合も送らない。
      if (Object.keys(chats).length && (!account || !chatMid)) {
        return "unavailable";
      }
    }
    if (settings.pause_read_receipts === true) return "paused";
    if (settings.auto_mark_read === false) return "auto_disabled";
    if (
      chats && Object.hasOwn(chats, account) && chats[account].includes(chatMid)
    ) {
      return "chat_paused";
    }
    return "allowed";
  } catch {
    // 設定を確認できない場合は、既読を送らない側へ倒す。
    return "unavailable";
  }
}

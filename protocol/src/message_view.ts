import { withReplyPreviews } from "./replies.ts";
import { isMediaType } from "./messages.ts";
import { callHistoryOf, callHistoryPreview } from "./call_history.ts";
import { chatEventOf, normalizeChatEvent } from "./chat_events.ts";
import { join } from "@std/path";

type Json = Record<string, unknown>;

export type MessageViewRuntime = {
  normType: (value: unknown) => string;
  existingFullImage: (id: string) => Promise<string | null>;
  existingImage: (path: string) => Promise<string | null>;
  mediaDest: (id: string, ext?: string) => string;
  thumbDest: (id: string) => string;
  isValidAudioFile: (path: string | null) => Promise<boolean>;
  uiMediaPath: (id: string, path: string) => Promise<string | null>;
  stickerDir: string;
};

export function createMessageView(runtime: MessageViewRuntime) {
  const {
    normType,
    existingFullImage,
    existingImage,
    mediaDest,
    thumbDest,
    isValidAudioFile,
    uiMediaPath,
    stickerDir,
  } = runtime;

  function previewBody(msg: Json): string {
    const ct = normType(msg.contentType);
    const text = String(msg.text ?? "").trim();
    const L = previewLang();
    if (ct === "UNSENT") return L.unsent;
    if (chatEventOf(msg)) return L.groupUpdated;
    if (msg.decryptionFailed === true) return L.encrypted;
    const call = callHistoryPreview(msg, L);
    if (call) return call;
    if (ct === "IMAGE") return L.photo;
    if (ct === "VIDEO") return L.video;
    if (ct === "STICKER") return L.sticker;
    if (ct === "AUDIO") return L.voice;
    if (ct === "FILE") return L.file;
    if (ct === "FLEX") return text || L.flex;
    if (!text) return ct !== "NONE" ? ct.toLowerCase() : L.message;
    return text.length > 64 ? `${text.slice(0, 63)}…` : text;
  }

  function previewLang() {
    const code = (Deno.env.get("LINE_GTK_LANG") || "vi").toLowerCase();
    if (code === "vi" || code === "vie" || code === "vietnamese") {
      return {
        you: "Bạn",
        groupUpdated: "Thông tin nhóm đã được cập nhật",
        unsent: "Tin nhắn đã được thu hồi",
        callIncoming: "Cuộc gọi đến",
        callOutgoing: "Cuộc gọi đi",
        callMissed: "Cuộc gọi nhỡ",
        callUnanswered: "Cuộc gọi không được trả lời",
        videoIncoming: "Cuộc gọi video đến",
        videoOutgoing: "Cuộc gọi video đi",
        videoMissed: "Cuộc gọi video nhỡ",
        videoUnanswered: "Cuộc gọi video không được trả lời",
        they: "Đối phương",
        photo: "Ảnh",
        video: "Video",
        sticker: "Nhãn dán",
        voice: "Tin nhắn thoại",
        file: "Tệp",
        flex: "Tin nhắn Flex",
        message: "Tin nhắn",
        encrypted: "Cần lấy lại khóa mã hóa",
        tap: "Nhấn để mở",
      };
    }
    if (code === "ja" || code === "ja-jp" || code === "ja_jp") {
      return {
        you: "自分",
        groupUpdated: "グループの情報が更新されました",
        unsent: "メッセージの送信が取り消されました",
        callIncoming: "着信がありました",
        callOutgoing: "発信しました",
        callMissed: "不在着信",
        callUnanswered: "応答なし",
        videoIncoming: "ビデオ通話の着信",
        videoOutgoing: "ビデオ通話の発信",
        videoMissed: "ビデオ通話の不在着信",
        videoUnanswered: "ビデオ通話の応答なし",
        they: "相手",
        photo: "写真",
        video: "動画",
        sticker: "スタンプ",
        voice: "ボイスメッセージ",
        file: "ファイル",
        flex: "Flexメッセージ",
        message: "メッセージ",
        encrypted: "暗号鍵の再取得が必要です",
        tap: "タップして開く",
      };
    }
    const th = code === "th" || code === "thai";
    return th
      ? {
        you: "คุณ",
        groupUpdated: "อัปเดตข้อมูลกลุ่มแล้ว",
        unsent: "ยกเลิกข้อความแล้ว",
        callIncoming: "มีสายเรียกเข้า",
        callOutgoing: "โทรออก",
        callMissed: "สายที่ไม่ได้รับ",
        callUnanswered: "ไม่มีผู้รับสาย",
        videoIncoming: "สายวิดีโอเรียกเข้า",
        videoOutgoing: "โทรวิดีโอออก",
        videoMissed: "สายวิดีโอที่ไม่ได้รับ",
        videoUnanswered: "ไม่มีผู้รับสายวิดีโอ",
        they: "เขา",
        photo: "รูปภาพ",
        video: "วิดีโอ",
        sticker: "สติกเกอร์",
        voice: "ข้อความเสียง",
        file: "ไฟล์",
        flex: "ข้อความ Flex",
        message: "ข้อความ",
        encrypted: "ต้องรับคีย์เข้ารหัสใหม่",
        tap: "แตะเพื่อเปิด",
      }
      : {
        you: "You",
        groupUpdated: "Group information updated",
        unsent: "Message unsent",
        callIncoming: "Incoming call",
        callOutgoing: "Outgoing call",
        callMissed: "Missed call",
        callUnanswered: "Unanswered call",
        videoIncoming: "Incoming video call",
        videoOutgoing: "Outgoing video call",
        videoMissed: "Missed video call",
        videoUnanswered: "Unanswered video call",
        they: "They",
        photo: "Photo",
        video: "Video",
        sticker: "Sticker",
        voice: "Voice message",
        file: "File",
        flex: "Flex message",
        message: "Message",
        encrypted: "Retrieve encryption keys again",
        tap: "Tap to open",
      };
  }

  function previewLine(msg: Json): string {
    if (chatEventOf(msg)) return previewBody(msg);
    const L = previewLang();
    const who = msg.mine ? L.you : L.they;
    return `${who}: ${previewBody(msg)}`;
  }

  function extractFlex(meta: Record<string, string>): Json | null {
    const altText = meta.ALT_TEXT || "";
    let root: unknown = null;
    if (meta.FLEX_JSON) {
      try {
        root = JSON.parse(meta.FLEX_JSON);
      } catch {
        root = null;
      }
    }
    const firstText = (node: unknown, depth = 0): string => {
      if (!node || typeof node !== "object" || depth > 12) return "";
      if (Array.isArray(node)) {
        for (const n of node) {
          const t = firstText(n, depth + 1);
          if (t) return t;
        }
        return "";
      }
      const o = node as Record<string, unknown>;
      if (typeof o.text === "string" && o.text.trim()) return o.text.trim();
      for (const v of Object.values(o)) {
        const t = firstText(v, depth + 1);
        if (t) return t;
      }
      return "";
    };
    const texts: string[] = [];
    const actions: Json[] = [];
    const seen = new Set<string>();

    const walk = (node: unknown, depth = 0) => {
      if (!node || depth > 12) return;
      if (Array.isArray(node)) {
        for (const n of node) walk(n, depth + 1);
        return;
      }
      if (typeof node !== "object") return;
      const o = node as Record<string, unknown>;
      if (typeof o.text === "string" && o.text.trim()) {
        const t = o.text.trim();
        if (texts.length < 12 && !texts.includes(t)) texts.push(t);
      }
      if (o.action && typeof o.action === "object") {
        const a = o.action as Record<string, unknown>;
        // LINE's own templates (e.g. polls) set the literal label "action";
        // show the text inside the tapped box instead.
        const explicit = typeof a.label === "string" &&
            a.label.trim().toLowerCase() !== "action"
          ? a.label.trim()
          : "";
        const label = String(
          explicit || o.text || firstText(o) || a.data || a.uri || "Action",
        );
        const kind = String(a.type || "postback").toLowerCase();
        const data = a.data != null ? String(a.data) : null;
        const uri = a.uri != null
          ? String(a.uri)
          : (a.url != null ? String(a.url) : null);
        const key = `${kind}|${label}|${data || ""}|${uri || ""}`;
        if (!seen.has(key) && actions.length < 24) {
          seen.add(key);
          actions.push({ label, kind, data, uri });
        }
      }
      for (const v of Object.values(o)) walk(v, depth + 1);
    };
    walk(root);

    if (!altText && !texts.length && !actions.length) return null;
    return {
      altText: altText || texts[0] || "Flex message",
      texts: texts.slice(0, 8),
      actions,
    };
  }

  /** Serve disk-cached stickers/thumbs immediately; only hydrate misses. */
  async function forUiMessages(messages: Json[]): Promise<Json[]> {
    const prepared = withReplyPreviews(messages.map(normalizeChatEvent));
    return await Promise.all(prepared.map(async (m) => {
      if (m.decryptionFailed === true) return { ...m, needsMedia: false };
      const callHistory = callHistoryOf(m);
      if (callHistory) {
        return { ...m, contentType: "CALL", callHistory, needsMedia: false };
      }
      const ct = normType(m.contentType);
      const id = String(m.id ?? "");
      let imagePath: string | null = (m.imagePath as string) || null;
      let audioPath: string | null = (m.audioPath as string) || null;
      let needsMedia = isMediaType(ct) && !imagePath && !audioPath;

      if (ct === "STICKER") {
        const stkId = String(m.stickerId || "");
        if (stkId) {
          const anim = await existingImage(
            join(stickerDir, `${stkId}.anim.png`),
          );
          const st = await existingImage(join(stickerDir, `${stkId}.png`));
          if (anim) {
            imagePath = anim;
            needsMedia = false;
          } else if (st) {
            // Show static now; hydrate may upgrade to APNG.
            imagePath = st;
            needsMedia = true;
          }
        }
      } else if (ct === "IMAGE" || ct === "VIDEO") {
        const cached = await existingImage(thumbDest(id)) ||
          await existingFullImage(id) ||
          await existingImage(mediaDest(id)) ||
          await existingImage(mediaDest(id, "png"));
        if (cached) {
          imagePath = (await uiMediaPath(id, cached)) || cached;
          needsMedia = false;
        }
      } else if (ct === "AUDIO") {
        audioPath = null;
        for (const ext of ["m4a", "mp3", "aac", "ogg"]) {
          const p = mediaDest(id, ext);
          if (await isValidAudioFile(p)) {
            audioPath = p;
            break;
          }
        }
        if (audioPath) needsMedia = false;
        else needsMedia = true;
      } else if (ct === "FILE") {
        // Filename-only bubble; download is on demand / background, not a preview.
        needsMedia = false;
      }

      return { ...m, imagePath, audioPath, needsMedia };
    }));
  }

  return {
    previewBody,
    previewLang,
    previewLine,
    extractFlex,
    forUiMessages,
  };
}

import { assertEquals } from "@std/assert";
import {
  callHistoryOf,
  formatCallDuration,
  isLegacyCallPreview,
} from "../src/call_history.ts";
import { createMessageView } from "../src/message_view.ts";
import { needsChatPreview } from "../src/chat_previews.ts";

function messageView() {
  return createMessageView({
    normType: (value) => String(value ?? "NONE"),
    existingFullImage: () => Promise.resolve(null),
    existingImage: () => Promise.resolve(null),
    mediaDest: () => "",
    thumbDest: () => "",
    isValidAudioFile: () => Promise.resolve(false),
    uiMediaPath: () => Promise.resolve(null),
    stickerDir: "",
  });
}

Deno.test("旧版の通話履歴から時間を取り出し、結果コードの意味は推測しない", () => {
  assertEquals(
    callHistoryOf({
      contentType: "CALL",
      text: "Call History : 136000 millisecs, Result: 16",
    }),
    { video: false, status: "completed", durationMs: 136000 },
  );
  assertEquals(
    callHistoryOf({
      contentType: "CALL",
      text: "Call History : 0 millisecs, Result: 77",
    }),
    { video: false, status: "unknown", durationMs: 0 },
  );
  assertEquals(formatCallDuration(136000), "2:16");
  assertEquals(formatCallDuration(3723000), "1:02:03");
});

Deno.test("明示された未応答とビデオ種別を保持し、通常の本文は変換しない", () => {
  assertEquals(
    callHistoryOf({
      contentType: "NONE",
      contentMetadata: {
        ORGCONTP: "CALL",
        RESULT: "CANCELED",
        TYPE: "V",
        DURATION: "0",
      },
    }),
    { video: true, status: "unanswered", durationMs: 0 },
  );
  assertEquals(
    callHistoryOf({
      contentType: "NONE",
      text: "Call History : 136000 millisecs, Result: 16",
    }),
    null,
  );
  assertEquals(
    callHistoryOf({
      contentType: 6,
      contentMetadata: { DURATION: "-1", RESULT: "未確認" },
    }),
    { video: false, status: "unknown", durationMs: null },
  );
});

Deno.test("履歴カードと一覧用に、既存のメッセージキャッシュも構造化する", async () => {
  const view = messageView();
  const [message] = await view.forUiMessages([{
    id: "fixture",
    contentType: "CALL",
    mine: false,
    text: "Call History : 136000 millisecs, Result: 16",
    needsMedia: true,
  }]);
  assertEquals(message.callHistory, {
    video: false,
    status: "completed",
    durationMs: 136000,
  });
  assertEquals(message.needsMedia, false);
});

Deno.test("日本語の一覧で着信・発信・不在着信と通話時間が分かる", () => {
  const previous = Deno.env.get("LINE_GTK_LANG");
  const view = messageView();
  try {
    Deno.env.set("LINE_GTK_LANG", "ja");
    const incoming = { contentType: "CALL", mine: false, durationMs: 136000 };
    assertEquals(view.previewLine(incoming), "相手: 着信がありました · 2:16");
    assertEquals(
      view.previewLine({ ...incoming, mine: true }),
      "自分: 発信しました · 2:16",
    );
    assertEquals(
      view.previewBody({
        contentType: "CALL",
        callHistory: { video: false, status: "unanswered", durationMs: 0 },
      }),
      "不在着信",
    );
    assertEquals(
      view.previewBody({
        contentType: "CALL",
        callHistory: { video: true, status: "unanswered", durationMs: 0 },
      }),
      "ビデオ通話の不在着信",
    );
    assertEquals(
      view.previewBody({ contentType: "CALL", durationMs: 0 }),
      "着信がありました",
    );
    Deno.env.set("LINE_GTK_LANG", "en");
    assertEquals(view.previewBody(incoming), "Incoming call · 2:16");
    Deno.env.set("LINE_GTK_LANG", "th");
    assertEquals(view.previewBody(incoming), "มีสายเรียกเข้า · 2:16");
  } finally {
    if (previous === undefined) Deno.env.delete("LINE_GTK_LANG");
    else Deno.env.set("LINE_GTK_LANG", previous);
  }
});

Deno.test("一覧に保存された旧内部表現は再取得して読みやすい表示へ移行する", () => {
  const preview = "They: Call History : 136000 millisecs, Result: 16";
  assertEquals(isLegacyCallPreview(preview), true);
  assertEquals(
    needsChatPreview({ mid: "u-fixture", preview, lastActivity: 20 }),
    true,
  );
  assertEquals(isLegacyCallPreview("Call Historyについて相談"), false);
});

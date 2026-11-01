import { assertEquals } from "@std/assert";
import { createMessageView } from "../src/message_view.ts";

Deno.test("日本語のプレビューと実行中の言語切り替え", () => {
  const previous = Deno.env.get("LINE_GTK_LANG");
  const view = createMessageView({
    normType: (value) => String(value ?? "NONE"),
    existingFullImage: () => Promise.resolve(null),
    existingImage: () => Promise.resolve(null),
    mediaDest: () => "",
    thumbDest: () => "",
    isValidAudioFile: () => Promise.resolve(false),
    uiMediaPath: () => Promise.resolve(null),
    stickerDir: "",
  });
  try {
    Deno.env.set("LINE_GTK_LANG", "ja");
    assertEquals(
      view.previewLine({ mine: true, contentType: "IMAGE" }),
      "自分: 写真",
    );
    assertEquals(
      view.previewLine({ mine: false, contentType: "STICKER" }),
      "相手: スタンプ",
    );
    assertEquals(
      view.previewBody({ contentType: "AUDIO" }),
      "ボイスメッセージ",
    );
    assertEquals(
      view.previewBody({ text: "日本語のメッセージ", contentType: "NONE" }),
      "日本語のメッセージ",
    );
    Deno.env.set("LINE_GTK_LANG", "vi");
    assertEquals(
      view.previewLine({ mine: true, contentType: "IMAGE" }),
      "Bạn: Ảnh",
    );
    assertEquals(
      view.previewBody({ contentType: "UNSENT" }),
      "Tin nhắn đã được thu hồi",
    );
    Deno.env.set("LINE_GTK_LANG", "en");
    assertEquals(
      view.previewLine({ mine: true, contentType: "IMAGE" }),
      "You: Photo",
    );
    assertEquals(
      view.previewLine({ contentType: "CHATEVENT", text: "[CHATEVENT]" }),
      "Group information updated",
    );
    Deno.env.set("LINE_GTK_LANG", "th");
    assertEquals(
      view.previewLine({ mine: true, contentType: "IMAGE" }),
      "คุณ: รูปภาพ",
    );
  } finally {
    if (previous === undefined) Deno.env.delete("LINE_GTK_LANG");
    else Deno.env.set("LINE_GTK_LANG", previous);
  }
});

Deno.test("cached history placeholders become system rows without message actions", async () => {
  const view = createMessageView({
    normType: (value) => String(value ?? "NONE"),
    existingFullImage: () => Promise.resolve(null),
    existingImage: () => Promise.resolve(null),
    mediaDest: () => "",
    thumbDest: () => "",
    isValidAudioFile: () => Promise.resolve(false),
    uiMediaPath: () => Promise.resolve(null),
    stickerDir: "",
  });
  const [notice] = await view.forUiMessages([{
    id: "123",
    contentType: "CHATEVENT",
    text: "[CHATEVENT]",
    mine: true,
    replyTo: { messageId: "12" },
    reactions: [{ kind: "NICE" }],
  }]);
  assertEquals(notice.contentType, "CHAT_EVENT");
  assertEquals(notice.chatEvent, {
    kind: "unknown",
    actorMid: "",
    targetMids: [],
  });
  assertEquals(notice.text, "");
  assertEquals(notice.mine, false);
  assertEquals(notice.replyTo, null);
  assertEquals(notice.reactions, []);
});

Deno.test("LINE poll flex card uses box text for generic 'action' labels", () => {
  const view = createMessageView({
    normType: (value) => String(value ?? "NONE"),
    existingFullImage: () => Promise.resolve(null),
    existingImage: () => Promise.resolve(null),
    mediaDest: () => "",
    thumbDest: () => "",
    isValidAudioFile: () => Promise.resolve(false),
    uiMediaPath: () => Promise.resolve(null),
    stickerDir: "",
  });
  const uri = "https://liff.line.me/1477715170-Pl2JnXpR/view/1?chatId=C1";
  const flex = view.extractFlex({
    ALT_TEXT: "[Cuộc thăm dò ý kiến] Test",
    FLEX_JSON: JSON.stringify({
      type: "bubble",
      body: {
        type: "box",
        layout: "vertical",
        contents: [
          { type: "text", text: "[Cuộc thăm dò ý kiến] Test" },
          {
            type: "box",
            layout: "vertical",
            contents: [{ type: "text", text: "Bầu chọn ngay" }],
            action: { type: "uri", label: "action", uri },
          },
          {
            type: "box",
            layout: "horizontal",
            contents: [{
              type: "box",
              layout: "horizontal",
              contents: [
                { type: "image", url: "https://x/y.png" },
                { type: "text", text: "Cuộc thăm dò ý kiến" },
              ],
            }],
            action: { type: "uri", label: "action", uri: uri + "&b" },
          },
        ],
      },
    }),
  }) as { actions: { label: string; kind: string }[] };
  assertEquals(flex.actions.map((a) => a.label), [
    "Bầu chọn ngay",
    "Cuộc thăm dò ý kiến",
  ]);
  assertEquals(flex.actions.map((a) => a.kind), ["uri", "uri"]);
});

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
    Deno.env.set("LINE_GTK_LANG", "en");
    assertEquals(
      view.previewLine({ mine: true, contentType: "IMAGE" }),
      "You: Photo",
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

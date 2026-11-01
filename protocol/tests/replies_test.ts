import { assertEquals } from "@std/assert";
import {
  replyMessageId,
  replySnapshot,
  replyToOf,
  withReplyPreviews,
} from "../src/replies.ts";

Deno.test("返信先IDを丸めず、仮ID・別サービス・リプライ以外を引用しない", () => {
  assertEquals(replyMessageId("9007199254740993"), "9007199254740993");
  assertEquals(replyMessageId(9007199254740993n), "9007199254740993");
  for (const id of ["", "pending-1", "0", -1, 9007199254740993, {}, null]) {
    assertEquals(replyMessageId(id), null);
  }
  const raw = {
    relatedMessageId: "9007199254740993",
    messageRelationType: "REPLY",
  };
  assertEquals(replyToOf(raw), { messageId: "9007199254740993" });
  assertEquals(
    replyToOf({ ...raw, messageRelationType: 3, relatedMessageServiceCode: 1 }),
    { messageId: "9007199254740993" },
  );
  assertEquals(replyToOf({ ...raw, messageRelationType: "FORWARD" }), null);
  assertEquals(
    replyToOf({ ...raw, relatedMessageServiceCode: "SQUARE" }),
    null,
  );
});

Deno.test("前後が逆の履歴とディスクキャッシュからも、同じトークの引用だけ補う", () => {
  const original = {
    id: "9007199254740993",
    from: "u-peer",
    to: "u-self",
    mine: false,
    text: "引用元",
    contentType: "NONE",
  };
  const reply = {
    id: "9007199254740994",
    from: "u-self",
    to: "u-peer",
    mine: true,
    relatedMessageId: original.id,
    messageRelationType: "REPLY",
  };
  const result = withReplyPreviews([reply, original]);
  assertEquals(result[0].replyTo, replySnapshot(original));
  assertEquals(
    withReplyPreviews(JSON.parse(JSON.stringify(result)))[0].replyTo,
    result[0].replyTo,
  );
  const elsewhere = { ...reply, to: "c-other" };
  assertEquals(withReplyPreviews([elsewhere, original])[0].replyTo, {
    messageId: original.id,
  });
});

Deno.test("引用元が範囲外でもIDを保持し、復号失敗本文や長大な本文を引用しない", () => {
  assertEquals(
    withReplyPreviews([{ id: "99", replyTo: { messageId: "42" } }])[0].replyTo,
    { messageId: "42" },
  );
  const snapshot = replySnapshot({
    id: "42",
    text: "あ🙂".repeat(400),
    mine: true,
  });
  assertEquals(Array.from(snapshot.text!).length, 512);
  assertEquals(snapshot.text!.endsWith("🙂"), true);
  assertEquals(
    replySnapshot({ id: "42", text: "暗号文", decryptionFailed: true }).text,
    "",
  );
  assertEquals(
    replySnapshot({ id: "42", text: "暗号文", decryptionFailed: true })
      .contentType,
    "UNAVAILABLE",
  );
});

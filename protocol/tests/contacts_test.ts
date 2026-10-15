import { assertEquals } from "@std/assert";
import { contactKind } from "../src/contacts.ts";

Deno.test("公式アカウントを文字列・数値の種別と能力フラグから判別する", () => {
  for (
    const contact of [
      { type: "PROMOTION_BOT" },
      { type: 8 },
      { type: "8" },
      { type: "MID", capableBuddy: true },
      { type: "USERID", attributes: 35 },
    ]
  ) assertEquals(contactKind(contact), "bot");
  for (
    const contact of [
      null,
      {},
      { type: 6, displayName: "公式アカウント" },
      { type: "MID", attributes: 3 },
    ]
  ) assertEquals(contactKind(contact), "dm");
});

Deno.test("メッセージから確認済みの公式判定を不完全な連絡先で上書きしない", () => {
  assertEquals(contactKind({ type: "MID" }, "bot"), "bot");
  assertEquals(contactKind({ type: "MID" }, "dm"), "dm");
});

import { assertEquals } from "@std/assert";
import { readReceiptPermission } from "../src/read_policy.ts";

Deno.test("既読停止と従来の自動既読設定を、毎回保存済みファイルから確認する", async () => {
  const directory = await Deno.makeTempDir();
  const path = `${directory}/settings.json`;
  try {
    assertEquals(await readReceiptPermission(path), "unavailable");
    await Deno.writeTextFile(path, JSON.stringify({ auto_mark_read: true }));
    assertEquals(await readReceiptPermission(path), "allowed");
    await Deno.writeTextFile(
      path,
      JSON.stringify({ auto_mark_read: true, pause_read_receipts: true }),
    );
    assertEquals(await readReceiptPermission(path), "paused");
    await Deno.writeTextFile(
      path,
      JSON.stringify({ auto_mark_read: false, pause_read_receipts: false }),
    );
    assertEquals(await readReceiptPermission(path), "auto_disabled");
    await Deno.writeTextFile(
      path,
      JSON.stringify({ auto_mark_read: true, pause_read_receipts: false }),
    );
    assertEquals(await readReceiptPermission(path), "allowed");
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("設定が壊れていたり型が違ったりする場合は既読を送らない", async () => {
  const directory = await Deno.makeTempDir();
  const path = `${directory}/settings.json`;
  try {
    for (
      const contents of [
        "{",
        "null",
        "[]",
        '{"pause_read_receipts":"false"}',
        '{"auto_mark_read":"true"}',
        '{"read_receipts_paused_chats":null}',
        '{"read_receipts_paused_chats":[]}',
        '{"read_receipts_paused_chats":{"u-self":"u-friend"}}',
        '{"read_receipts_paused_chats":{"u-self":[42]}}',
        '{"read_receipts_paused_chats":{"u-self":[""]}}',
      ]
    ) {
      await Deno.writeTextFile(path, contents);
      assertEquals(await readReceiptPermission(path), "unavailable");
    }
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("友だち別の停止は同じアカウントの指定した相手にだけ適用する", async () => {
  const directory = await Deno.makeTempDir();
  const path = `${directory}/settings.json`;
  try {
    const settings = {
      auto_mark_read: true,
      pause_read_receipts: false,
      read_receipts_paused_chats: { "u-self": ["u-friend"] },
    };
    await Deno.writeTextFile(path, JSON.stringify(settings));
    assertEquals(
      await readReceiptPermission(path, "u-self", "u-friend"),
      "chat_paused",
    );
    assertEquals(
      await readReceiptPermission(path, "u-self", "u-other"),
      "allowed",
    );
    assertEquals(
      await readReceiptPermission(path, "u-other-account", "u-friend"),
      "allowed",
    );
    assertEquals(
      await readReceiptPermission(path, "", "u-friend"),
      "unavailable",
    );
    assertEquals(
      await readReceiptPermission(path, "u-self", ""),
      "unavailable",
    );
    settings.pause_read_receipts = true;
    await Deno.writeTextFile(path, JSON.stringify(settings));
    assertEquals(
      await readReceiptPermission(path, "u-self", "u-other"),
      "paused",
    );
    settings.pause_read_receipts = false;
    settings.auto_mark_read = false;
    await Deno.writeTextFile(path, JSON.stringify(settings));
    assertEquals(
      await readReceiptPermission(path, "u-self", "u-other"),
      "auto_disabled",
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

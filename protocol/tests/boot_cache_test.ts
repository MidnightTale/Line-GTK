import { assertEquals } from "@std/assert";
import { join } from "@std/path";
import {
  type BootProfile,
  loadBootCache,
  loadBootMessages,
  saveBootSession,
} from "../src/boot_cache.ts";
import { atomicWriteJson } from "../src/storage.ts";

const profile: BootProfile = {
  mid: "uaaa",
  displayName: "自分",
  statusMessage: "",
  picturePath: null,
  avatarPath: "/tmp/avatar.png",
  pictureUrl: null,
};
const chats = [{ mid: "u-friend", name: "友達" }];
const DAY = 24 * 60 * 60 * 1000;

async function fixture() {
  const dir = await Deno.makeTempDir();
  return {
    dir,
    session: join(dir, "session.json"),
    chats: join(dir, "chats.json"),
    messages: join(dir, "messages"),
  };
}

Deno.test("保存した一覧とプロフィールを同じアカウントなら返す", async () => {
  const f = await fixture();
  await saveBootSession(f.session, profile);
  await atomicWriteJson(f.chats, { at: Date.now(), accountMid: "uaaa", chats });
  const cache = await loadBootCache(f.session, f.chats, DAY);
  assertEquals(cache?.accountMid, "uaaa");
  assertEquals(cache?.profile.displayName, "自分");
  assertEquals(cache?.profile.avatarPath, "/tmp/avatar.png");
  assertEquals(cache?.chats, chats);
});

Deno.test("別アカウントや旧形式の一覧は使わない", async () => {
  const f = await fixture();
  await saveBootSession(f.session, profile);
  await atomicWriteJson(f.chats, { at: Date.now(), accountMid: "ubbb", chats });
  assertEquals(await loadBootCache(f.session, f.chats, DAY), null);
  await atomicWriteJson(f.chats, { at: Date.now(), chats });
  assertEquals(await loadBootCache(f.session, f.chats, DAY), null);
});

Deno.test("期限切れや欠けた保存ファイルは使わない", async () => {
  const f = await fixture();
  assertEquals(await loadBootCache(f.session, f.chats, DAY), null);
  await saveBootSession(f.session, profile);
  await atomicWriteJson(f.chats, {
    at: Date.now() - 2 * DAY,
    accountMid: "uaaa",
    chats,
  });
  assertEquals(await loadBootCache(f.session, f.chats, DAY), null);
});

Deno.test("mid が空のプロフィールは記録しない", async () => {
  const f = await fixture();
  await saveBootSession(f.session, { ...profile, mid: "" });
  assertEquals(
    await Deno.stat(f.session).then(() => true, () => false),
    false,
  );
});

Deno.test("保存したメッセージは同じアカウントだけ読む", async () => {
  const f = await fixture();
  await Deno.mkdir(f.messages);
  const messages = [{ id: "1", text: "こんにちは" }];
  await atomicWriteJson(join(f.messages, "u-friend.json"), {
    at: Date.now(),
    accountMid: "uaaa",
    messages,
  });
  assertEquals(
    await loadBootMessages(f.messages, "u-friend", "uaaa", DAY),
    messages,
  );
  assertEquals(
    await loadBootMessages(f.messages, "u-friend", "ubbb", DAY),
    null,
  );
  assertEquals(await loadBootMessages(f.messages, "u-friend", "", DAY), null);
});

Deno.test("不正なトーク ID では保存先の外を読まない", async () => {
  const f = await fixture();
  await Deno.mkdir(f.messages);
  await atomicWriteJson(join(f.dir, "secret.json"), {
    at: Date.now(),
    accountMid: "uaaa",
    messages: [{ id: "x" }],
  });
  assertEquals(
    await loadBootMessages(f.messages, "../secret", "uaaa", DAY),
    null,
  );
});

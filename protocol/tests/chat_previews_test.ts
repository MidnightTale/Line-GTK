import { assertEquals } from "@std/assert";
import { createChatPreviews, type PreviewChat } from "../src/chat_previews.ts";

type Json = Record<string, unknown>;

export function previewFixture(chats: PreviewChat[]) {
  const events: PreviewChat[] = [];
  let saved = 0;
  const previews = createChatPreviews({
    getChats: () => chats,
    previewLine: (message) => `相手: ${message.text}`,
    fallback: () => "タップして開く",
    emit: (chat) => events.push({ ...chat }),
    save: () => {
      saved++;
      return Promise.resolve();
    },
  });
  return { previews, events, saved: () => saved };
}

Deno.test("本文を読み込むと一覧プレビューも更新し、再起動用に保存する", async () => {
  const chat = { mid: "u-fixture", preview: "", lastActivity: 20 };
  const { previews, events, saved } = previewFixture([chat]);
  const messages: Json[] = [
    { id: "2", createdTime: 20, text: "最新の本文" },
    { id: "1", createdTime: 10, text: "古い本文" },
  ];
  await previews.updateFromMessages(chat.mid, messages);
  assertEquals(chat.preview, "相手: 最新の本文");
  assertEquals(events, [{ ...chat }]);
  assertEquals(saved(), 1);
});

Deno.test("同時刻の本文は安全な整数範囲を超えるIDでも新しい順に選ぶ", async () => {
  const chat = { mid: "c-fixture", preview: "", lastActivity: 20 };
  const { previews } = previewFixture([chat]);
  await previews.updateFromMessages(chat.mid, [
    { id: "9007199254740993", createdTime: 20, text: "新しい本文" },
    { id: "9007199254740992", createdTime: 20, text: "古い本文" },
  ]);
  assertEquals(chat.preview, "相手: 新しい本文");
});

Deno.test("古い履歴と空の応答で新着プレビューを上書きしない", async () => {
  const chat = { mid: "u-fixture", preview: "相手: 新着", lastActivity: 30 };
  const { previews, events, saved } = previewFixture([chat]);
  await previews.updateFromMessages(chat.mid, [
    { createdTime: 20, text: "古い履歴" },
  ]);
  await previews.updateFromMessages(chat.mid, []);
  assertEquals(chat.preview, "相手: 新着");
  assertEquals(events, []);
  assertEquals(saved(), 0);
});

Deno.test("本文が空の場合は読み込み中を終了する", async () => {
  const chat = { mid: "u-fixture", preview: "", lastActivity: 20 };
  const { previews } = previewFixture([chat]);
  await previews.updateFromMessages(chat.mid, []);
  assertEquals(chat.preview, "タップして開く");
});

Deno.test("28件を超えるトークも同時取得を2件に抑えて最後まで更新する", async () => {
  const chats = Array.from({ length: 65 }, (_, index) => ({
    mid: `u-fixture-${index}`,
    preview: "",
    lastActivity: 20,
  }));
  const { previews } = previewFixture(chats);
  let running = 0;
  let maximum = 0;
  let fetched = 0;
  await previews.hydrate(chats, async () => {
    running++;
    maximum = Math.max(maximum, running);
    await new Promise((resolve) => setTimeout(resolve, 0));
    fetched++;
    running--;
    return [{ createdTime: 20, text: "最新の本文" }];
  }, () => true);
  assertEquals(fetched, 65);
  assertEquals(maximum, 2);
  assertEquals(
    chats.every((chat) => chat.preview === "相手: 最新の本文"),
    true,
  );
});

Deno.test("通信失敗・空応答・古い履歴でも読み込み中を残さず、再試行できる", async () => {
  const chats = ["失敗", "空", "古い履歴"].map((mid) => ({
    mid,
    preview: "",
    lastActivity: 20,
  }));
  const { previews } = previewFixture(chats);
  await previews.hydrate(chats, (chat) => {
    if (chat.mid === "失敗") throw new Error("検証用の通信失敗");
    return Promise.resolve(
      chat.mid === "空" ? [] : [{ createdTime: 10, text: "古い履歴" }],
    );
  }, () => true);
  assertEquals(
    chats.map((chat) => chat.preview),
    Array(3).fill("タップして開く"),
  );
  await previews.hydrate(
    chats,
    () => Promise.resolve([{ createdTime: 20, text: "再取得" }]),
    () => true,
  );
  assertEquals(
    chats.map((chat) => chat.preview),
    Array(3).fill("相手: 再取得"),
  );
});

Deno.test("言語切り替え前の案内文も再取得する", async () => {
  const chats = ["Tap to open", "แตะเพื่อเปิด", "タップして開く"].map((
    preview,
    index,
  ) => ({
    mid: `u-fixture-${index}`,
    preview,
    lastActivity: 20,
  }));
  const { previews } = previewFixture(chats);
  await previews.hydrate(
    chats,
    () => Promise.resolve([{ createdTime: 20, text: "再取得" }]),
    () => true,
  );
  assertEquals(chats.every((chat) => chat.preview === "相手: 再取得"), true);
});

Deno.test("取得中に新着や本文表示が更新された場合は古い応答を捨てる", async () => {
  const chat = { mid: "u-fixture", preview: "", lastActivity: 20 };
  const { previews, events } = previewFixture([chat]);
  await previews.hydrate([chat], () => {
    chat.preview = "相手: 新着";
    chat.lastActivity = 30;
    return Promise.resolve([{ createdTime: 20, text: "古い応答" }]);
  }, () => true);
  assertEquals(chat.preview, "相手: 新着");
  assertEquals(events, []);
  chat.preview = "";
  await previews.hydrate([chat], () => {
    chat.preview = "相手: 本文から取得した同時刻の新着";
    return Promise.resolve([]);
  }, () => true);
  assertEquals(chat.preview, "相手: 本文から取得した同時刻の新着");
  assertEquals(events, []);
});

Deno.test("ログアウト後の取得結果は反映も保存もしない", async () => {
  const chat = { mid: "u-fixture", preview: "", lastActivity: 20 };
  const { previews, events, saved } = previewFixture([chat]);
  let active = true;
  await previews.hydrate([chat], () => {
    active = false;
    return Promise.resolve([{ createdTime: 20, text: "前のアカウントの本文" }]);
  }, () => active);
  assertEquals(chat.preview, "");
  assertEquals(events, []);
  assertEquals(saved(), 0);
});

Deno.test("一覧を重複取得しても同じトークの背景通信を重複させない", async () => {
  const chat = { mid: "u-fixture", preview: "", lastActivity: 20 };
  const { previews } = previewFixture([chat]);
  let fetched = 0;
  const fetch = async () => {
    fetched++;
    await new Promise((resolve) => setTimeout(resolve, 0));
    return [{ createdTime: 20, text: "最新の本文" }];
  };
  await Promise.all([
    previews.hydrate([chat], fetch, () => true),
    previews.hydrate([chat], fetch, () => true),
  ]);
  assertEquals(fetched, 1);
});

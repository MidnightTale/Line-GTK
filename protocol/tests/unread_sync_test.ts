import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createUnreadSync } from "../src/unread_sync.ts";

function fixture() {
  const chats = [{ mid: "u-peer", unread: 4 }, { mid: "c-group", unread: 3 }];
  const events: { event: string; payload: Record<string, unknown> }[] = [];
  const latest = new Map<string, unknown>([["u-peer", "9007199254740993"], [
    "c-group",
    "100",
  ]]);
  const resolvers: Array<(result: unknown) => void> = [];
  const rejecters: Array<(error: Error) => void> = [];
  let saves = 0;
  const client = {
    base: {
      talk: {
        getMessageBoxes: () =>
          new Promise((resolve, reject) => {
            resolvers.push(resolve);
            rejecters.push(reject);
          }),
      },
    },
  } as unknown as Client;
  let current: Client | null = client;
  let self = "u-self";
  const service = createUnreadSync({
    getClient: () => current,
    selfMid: () => self,
    latestMessageId: (mid) => latest.get(mid),
    getChats: () => chats,
    save: () => {
      saves++;
      return Promise.resolve();
    },
    emit: (event, payload) => events.push({ event, payload }),
  });
  return {
    service,
    chats,
    events,
    latest,
    resolvers,
    rejecters,
    saves: () => saves,
    logout: () => {
      current = null;
    },
    switchAccount: () => {
      self = "u-another-self";
      current = { base: client.base } as unknown as Client;
    },
  };
}
async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

Deno.test("一回既読の件数同期が失敗しても、同じ位置の手動再実行で再照会する", async () => {
  const f = fixture();
  const op = { type: 40, param1: "u-peer", param2: "9007199254740992" };
  f.service.handleOperation(op, true);
  assertEquals(f.resolvers.length, 1);
  f.rejecters[0](new Error("模擬照会失敗"));
  await settle();
  assertEquals(f.chats[0].unread, 4);
  f.service.handleOperation(op);
  assertEquals(f.resolvers.length, 1);
  f.service.handleOperation(op, true);
  assertEquals(f.resolvers.length, 2);
  f.resolvers[1](box("9007199254740993", 1));
  await settle();
  assertEquals(f.chats[0].unread, 1);
});
function box(id: string, unread: number) {
  return {
    messageBoxes: [{
      id: "u-peer",
      unreadCount: unread,
      lastDeliveredMessageId: { messageId: id },
    }],
  };
}

Deno.test("スマホの既読は名前・数値の通知を受け、個別とグループの未読を消して保存する", () => {
  const f = fixture();
  assertEquals(
    f.service.handleOperation({
      type: "SEND_CHAT_CHECKED",
      param1: "u-peer",
      param2: "9007199254740993",
    }),
    true,
  );
  f.service.handleOperation({ type: 40, param1: "c-group", param2: "100" });
  assertEquals(f.chats.map((chat) => chat.unread), [0, 0]);
  assertEquals(f.saves(), 2);
  assertEquals(f.events[0], {
    event: "chat_unread",
    payload: {
      accountMid: "u-self",
      chatMid: "u-peer",
      throughMessageId: "9007199254740993",
      unread: 0,
    },
  });
  assertEquals(f.resolvers.length, 0);
});

Deno.test("自分を読者とする55だけを同期し、相手の既読や不正なIDでは消さない", () => {
  const f = fixture();
  assertEquals(
    f.service.handleOperation({
      type: 55,
      param1: "u-peer",
      param2: "u-peer",
      param3: "9007199254740993",
    }),
    false,
  );
  f.service.handleOperation({
    type: 40,
    param1: "u-peer",
    param2: "pending-1",
  });
  assertEquals(f.events, []);
  f.service.handleOperation({
    type: 55,
    param1: "u-peer",
    param2: "u-self",
    param3: "9007199254740993",
  });
  assertEquals(f.chats[0].unread, 0);
});

Deno.test("古い既読位置では新着を消さず、サーバーの残り未読件数を反映する", async () => {
  const f = fixture();
  f.service.handleOperation({
    type: 40,
    param1: "u-peer",
    param2: "9007199254740992",
  });
  assertEquals(f.events, []);
  f.resolvers[0](box("9007199254740993", 1));
  await settle();
  assertEquals(f.chats[0].unread, 1);
});

Deno.test("未読照会中の新着で古い応答を捨て、二重通知は照会を増やさない", async () => {
  const f = fixture();
  const op = { type: 40, param1: "u-peer", param2: "9007199254740992" };
  f.service.handleOperation(op);
  f.service.handleOperation(op);
  assertEquals(f.resolvers.length, 1);
  f.service.observeMessage({
    from: "u-peer",
    to: "u-self",
    id: "9007199254740994",
  });
  f.resolvers[0](box("9007199254740993", 0));
  await settle();
  assertEquals(f.events, []);
  assertEquals(f.resolvers.length, 2);
  f.resolvers[1](box("9007199254740994", 2));
  await settle();
  assertEquals(f.events[0].payload.unread, 2);
});

Deno.test("ログアウト・アカウント変更後の応答では未読を消さない", async () => {
  const f = fixture();
  f.service.handleOperation({ type: 40, param1: "u-peer", param2: "1" });
  f.logout();
  f.resolvers[0](box("9007199254740993", 0));
  await settle();
  assertEquals(f.events, []);
  const g = fixture();
  g.service.handleOperation({ type: 40, param1: "u-peer", param2: "1" });
  g.switchAccount();
  g.resolvers[0](box("9007199254740993", 0));
  await settle();
  assertEquals(g.events, []);
  assertEquals(g.chats[0].unread, 4);
});

Deno.test("古い一覧の未読復活を防ぎ、既読後の新着と別アカウントの一覧は維持する", () => {
  const f = fixture();
  f.service.handleOperation({
    type: 40,
    param1: "u-peer",
    param2: "9007199254740993",
  });
  const stale = [{ mid: "u-peer", unread: 4 }];
  f.service.reconcile(stale);
  assertEquals(stale[0].unread, 0);
  f.latest.set("u-peer", "9007199254740994");
  const fresh = [{ mid: "u-peer", unread: 1 }];
  f.service.reconcile(fresh);
  assertEquals(fresh[0].unread, 1);
  f.switchAccount();
  f.latest.set("u-peer", "1");
  const other = [{ mid: "u-peer", unread: 5 }];
  f.service.reconcile(other);
  assertEquals(other[0].unread, 5);
});

Deno.test("最新位置が不明なトークは確認後に消し、通信失敗では元の未読を維持する", async () => {
  const f = fixture();
  f.latest.delete("u-peer");
  f.service.handleOperation({
    type: 40,
    param1: "u-peer",
    param2: "9007199254740993",
  });
  assertEquals(f.events, []);
  f.resolvers[0](box("9007199254740993", 0));
  await settle();
  assertEquals(f.chats[0].unread, 0);
  const g = fixture();
  g.service.handleOperation({ type: 40, param1: "u-peer", param2: "1" });
  g.rejecters[0](new Error("検証用の通信失敗"));
  await settle();
  assertEquals(g.events, []);
  assertEquals(g.chats[0].unread, 4);
});

Deno.test("古い一覧のカーソルでも、受信済みの新着が既読位置より新しければ消さない", () => {
  const f = fixture();
  f.service.handleOperation({
    type: 40,
    param1: "u-peer",
    param2: "9007199254740993",
  });
  f.service.observeMessage({
    from: "u-peer",
    to: "u-self",
    id: "9007199254740994",
  });
  f.service.reconcile(f.chats);
  assertEquals(f.chats[0].unread, 1);
  f.service.observeMessage({
    from: "u-peer",
    to: "u-self",
    id: "9007199254740994",
  });
  assertEquals(f.chats[0].unread, 1);
});

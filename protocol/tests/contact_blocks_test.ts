import { assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createContactBlocks } from "../src/contact_blocks.ts";

function fixture() {
  let ids: string[] = [];
  let sequence: () => Promise<number> = () => Promise.resolve(7);
  let load: () => Promise<string[]> = () => Promise.resolve([...ids]);
  let mutate: () => Promise<void> = () => Promise.resolve();
  const calls: unknown[] = [];
  const events: Record<string, unknown>[] = [];
  const client = {
    base: {
      getReqseq: () => sequence(),
      talk: {
        getBlockedContactIds: () => load(),
        getContacts: ({ mids }: { mids: string[] }) =>
          Promise.resolve(
            mids.map((mid) => ({ mid, displayName: "取得した名前" })),
          ),
        blockContact: async (args: { id: string }) => {
          calls.push(["block", args]);
          await mutate();
          ids.push(args.id);
        },
        unblockContact: async (args: { id: string }) => {
          calls.push(["unblock", args]);
          await mutate();
          ids = ids.filter((mid) => mid !== args.id);
        },
      },
    },
  } as unknown as Client;
  let current: Client | null = client;
  let account = "u-self";
  const service = createContactBlocks({
    getClient: () => current,
    selfMid: () => account,
    name: (mid) => mid === "u-friend" ? "友だち" : undefined,
    emit: (_event, payload) => events.push(payload),
  });
  return {
    service,
    calls,
    events,
    server: (next: string[]) => {
      ids = next;
    },
    load: (fn: typeof load) => {
      load = fn;
    },
    sequence: (fn: typeof sequence) => {
      sequence = fn;
    },
    mutate: (fn: typeof mutate) => {
      mutate = fn;
    },
    switchAccount: () => {
      account = "u-another";
      current = { base: client.base } as Client;
    },
    logout: () => {
      current = null;
    },
  };
}

Deno.test("サーバーのブロック一覧を取得し、ブロックと解除を正しいAPIと順序番号で行う", async () => {
  const f = fixture();
  f.server(["u-hidden"]);
  assertEquals((await f.service.refresh()).contacts, [{
    mid: "u-hidden",
    name: "取得した名前",
  }]);
  await f.service.set("u-friend", true);
  assertEquals(f.calls, [["block", { id: "u-friend", reqSeq: 7 }]]);
  assertEquals(f.service.isBlocked("u-friend"), true);
  await f.service.set("u-friend", true);
  assertEquals(f.calls.length, 1);
  await f.service.set("u-friend", false);
  assertEquals(f.calls[1], ["unblock", {
    id: "u-friend",
    reqSeq: 7,
    reference: "",
  }]);
  assertEquals(f.service.isBlocked("u-friend"), false);
  assertEquals(f.service.isBlocked("u-hidden"), true);
});

Deno.test("不正な相手と自分は変更せず、通信失敗でも確認済み状態を維持する", async () => {
  const f = fixture();
  await f.service.refresh();
  for (const mid of ["", "c-group", "u-self", "u/bad"]) {
    await assertRejects(() => f.service.set(mid, true));
  }
  assertEquals(f.calls.length, 0);
  f.mutate(() => Promise.reject(new Error("模擬失敗")));
  await assertRejects(() => f.service.set("u-friend", true));
  assertEquals(f.service.isBlocked("u-friend"), false);
  assertEquals(f.events.length, 1);
  f.mutate(() => Promise.resolve());
  await f.service.set("u-friend", true);
  f.mutate(() => Promise.reject(new Error("解除失敗")));
  await assertRejects(() => f.service.set("u-friend", false));
  assertEquals(f.service.isBlocked("u-friend"), true);
});

Deno.test("取得中と変更中のセッション切り替えで別アカウントの状態を書き換えない", async () => {
  const f = fixture();
  await f.service.refresh();
  f.sequence(() => {
    f.switchAccount();
    return Promise.resolve(8);
  });
  await assertRejects(() => f.service.set("u-friend", true));
  assertEquals(f.calls.length, 0);
  assertEquals(f.service.isBlocked("u-friend"), false);
  const g = fixture();
  let resolve!: (ids: string[]) => void;
  g.load(() =>
    new Promise((r) => {
      resolve = r;
    })
  );
  const pending = g.service.refresh();
  g.switchAccount();
  resolve(["u-friend"]);
  await assertRejects(() => pending);
  assertEquals(g.events.length, 0);
  const h = fixture();
  await h.service.refresh();
  h.mutate(() => {
    h.logout();
    return Promise.resolve();
  });
  await assertRejects(() => h.service.set("u-friend", true));
  assertEquals(h.events.length, 1);
});

Deno.test("変更前に開始した一覧応答は、成功したブロックを古い状態へ戻さない", async () => {
  const f = fixture();
  await f.service.refresh();
  let resolve!: (ids: string[]) => void;
  f.load(() =>
    new Promise((r) => {
      resolve = r;
    })
  );
  const pending = f.service.refresh();
  await f.service.set("u-friend", true);
  resolve([]);
  await pending;
  assertEquals(f.service.isBlocked("u-friend"), true);
  assertEquals(f.events.at(-1)?.contacts, [{
    mid: "u-friend",
    name: "友だち",
  }]);
});

Deno.test("他端末のブロック通知で再取得し、ブロック相手の個別受信だけを抑制する", async () => {
  const f = fixture();
  await f.service.refresh();
  f.server(["u-friend"]);
  assertEquals(f.service.handleOperation({ type: 68 }), true);
  await new Promise((r) => setTimeout(r, 0));
  assertEquals(
    f.service.ignoreMessage({ from: "u-friend", to: "u-self", mine: false }),
    true,
  );
  assertEquals(
    f.service.ignoreMessage({ from: "u-friend", to: "c-group", mine: false }),
    false,
  );
  assertEquals(
    f.service.ignoreMessage({ from: "u-friend", to: "r-group", mine: false }),
    false,
  );
  assertEquals(
    f.service.ignoreMessage({ from: "u-self", to: "u-friend", mine: true }),
    false,
  );
  f.server([]);
  assertEquals(f.service.handleOperation({ type: "UNBLOCK_CONTACT" }), true);
  await new Promise((r) => setTimeout(r, 0));
  assertEquals(f.service.isBlocked("u-friend"), false);
  assertEquals(f.service.handleOperation({ type: 13 }), false);
});

Deno.test("ブロック一覧の通信失敗や型不正を空の一覧として採用しない", async () => {
  const f = fixture();
  f.server(["u-friend"]);
  await f.service.refresh();
  f.load(() => Promise.reject(new Error("模擬取得失敗")));
  await assertRejects(() => f.service.refresh());
  assertEquals(f.service.isBlocked("u-friend"), true);
  f.load(() => Promise.resolve(["c-group"]));
  await assertRejects(() => f.service.refresh());
  assertEquals(f.service.isBlocked("u-friend"), true);
});

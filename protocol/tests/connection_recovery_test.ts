import { assertEquals, assertRejects, assertStrictEquals } from "@std/assert";
import type { Client, TalkMessage } from "@evex/linejs";
import {
  createConnectionRecovery,
  waitForReceiver,
} from "../src/connection_recovery.ts";
import { createListener, type ListenerRuntime } from "../src/listener.ts";

function mockClient(mid = "u-self") {
  let closes = 0;
  let starts = 0;
  const handlers = new Map<string, ((value: never) => unknown)[]>();
  const client = {
    base: {
      authToken: "模擬トークン",
      profile: { mid },
      poll: { sync: { talk: { revision: 10n, globalRev: 3n } } },
      push: {
        conns: [{
          close: () => {
            closes++;
            return Promise.resolve();
          },
        }],
        opStream: { close: () => closes++ },
        sqStream: { close: () => closes++ },
      },
    },
    on: (name: string, handler: (value: never) => unknown) => {
      handlers.set(name, [...handlers.get(name) ?? [], handler]);
    },
    listen: () => starts++,
  } as unknown as Client;
  return {
    client,
    closes: () => closes,
    starts: () => starts,
    emit: async (name: string, value: unknown) => {
      for (const handler of handlers.get(name) ?? []) {
        await handler(value as never);
      }
    },
  };
}

Deno.test("受信接続を交換して古い接続を閉じ、重複要求をまとめ最新の受信位置を引き継ぐ", async () => {
  const old = mockClient();
  const next = mockClient();
  let current = old.client;
  let release!: (client: Client) => void;
  let restores = 0;
  const recovery = createConnectionRecovery({
    getClient: () => current,
    restore: () => {
      restores++;
      return new Promise((resolve) => release = resolve);
    },
    activate: (client) => {
      current = client;
    },
  });
  const pending = recovery.reconnect();
  assertStrictEquals(recovery.reconnect(), pending);
  old.client.base.poll.sync.talk.revision = 20n;
  release(next.client);
  await pending;
  assertEquals(restores, 1);
  assertStrictEquals(current, next.client);
  assertEquals(next.client.base.poll.sync.talk.revision, 20n);
  assertEquals(old.closes(), 3);
  assertEquals(old.client.base.authToken, undefined);
  assertEquals(next.closes(), 0);
  old.client.base.poll.sync.talk.revision = 30n;
  assertEquals(next.client.base.poll.sync.talk.revision, 20n);
});

Deno.test("通信失敗では既存接続を維持し、次の要求で再試行できる", async () => {
  const old = mockClient();
  const next = mockClient();
  let current = old.client;
  let count = 0;
  const recovery = createConnectionRecovery({
    getClient: () => current,
    restore: () =>
      ++count === 1
        ? Promise.reject(new Error("オフライン"))
        : Promise.resolve(next.client),
    activate: (client) => {
      current = client;
    },
  });
  await assertRejects(() => recovery.reconnect(), Error, "オフライン");
  assertStrictEquals(current, old.client);
  assertEquals(old.closes(), 0);
  await recovery.reconnect();
  assertStrictEquals(current, next.client);
});

Deno.test("受信の開始呼び出しだけで成功にせず、実際の通信路が開くまで待つ", async () => {
  const { client } = mockClient();
  const pending = waitForReceiver(client, 100);
  const connection = client.base.push.conns[0];
  connection.resStream = new ReadableStream();
  await assertRejects(
    () => waitForReceiver(client, 2),
    Error,
    "receiver_connect_timeout",
  );
  connection.reqStream = {} as typeof connection.reqStream;
  await pending;
});

Deno.test("接続開始に失敗した受信処理を時間切れにし、セッション終了も検出する", async () => {
  const { client } = mockClient();
  await assertRejects(
    () => waitForReceiver(client, 2),
    Error,
    "receiver_connect_timeout",
  );
  client.base.authToken = undefined;
  await assertRejects(
    () => waitForReceiver(client),
    Error,
    "receiver_session_changed",
  );
});

for (
  const change of ["ログアウト", "別アカウント", "復元結果が別人"] as const
) {
  Deno.test(`復帰中の${change}で古い復元結果を適用しない`, async () => {
    const old = mockClient();
    const next = mockClient(change === "復元結果が別人" ? "u-other" : "u-self");
    let current: Client | null = old.client;
    let release!: (client: Client) => void;
    let activations = 0;
    const recovery = createConnectionRecovery({
      getClient: () => current,
      restore: () => new Promise((resolve) => release = resolve),
      activate: () => {
        activations++;
      },
    });
    const pending = recovery.reconnect();
    if (change !== "復元結果が別人") {
      current = change === "ログアウト" ? null : mockClient("u-other").client;
    }
    release(next.client);
    await assertRejects(() => pending, Error, "receiver_session_changed");
    assertEquals(activations, 0);
    assertEquals(next.closes(), 3);
    assertEquals(old.closes(), 0);
  });
}

Deno.test("復元が時間切れでも再試行でき、遅い復元結果の接続は閉じる", async () => {
  const old = mockClient();
  const late = mockClient();
  const next = mockClient();
  let release!: (client: Client) => void;
  let current = old.client;
  let count = 0;
  const recovery = createConnectionRecovery({
    getClient: () => current,
    restore: () =>
      ++count === 1
        ? new Promise((resolve) => release = resolve)
        : Promise.resolve(next.client),
    activate: (client) => {
      current = client;
    },
    timeoutMs: 5,
  });
  await assertRejects(
    () => recovery.reconnect(),
    Error,
    "receiver_reconnect_timeout",
  );
  await recovery.reconnect();
  release(late.client);
  await Promise.resolve();
  assertEquals(late.closes(), 3);
  assertStrictEquals(current, next.client);
});

Deno.test("交換直前の本文を取りこぼさず、交換後の重複と古い接続のイベントを除外する", async () => {
  const old = mockClient();
  const next = mockClient();
  let current = old.client;
  let listening = false;
  let release!: (message: Record<string, unknown>) => void;
  const events: string[] = [];
  const delayedMessage = {} as TalkMessage;
  const runtime: ListenerRuntime = {
    handleIncomingCall: () => {
      events.push("着信");
    },
    handleCallCancel: () => {
      events.push("通話終了");
    },
    getClient: () => current,
    isListening: () => listening,
    setListening: (next) => {
      listening = next;
    },
    appendMessage: () => {},
    summarizeTalkMessage: (message) =>
      message === delayedMessage
        ? new Promise((resolve) => release = resolve)
        : Promise.resolve({
          id: "200",
          from: "u-friend",
          to: "u-self",
          text: "復帰後の新着",
        }),
    storeBoxCursor: () => {},
    upsertChatFromMessage: () => Promise.resolve(),
    hydrateLiveMedia: () => Promise.resolve(),
    upsertChatFromContact: () => Promise.resolve(),
    emitEvent: (event) => {
      events.push(event);
    },
    handleReadReceipt: () => {
      events.push("操作通知");
      return true;
    },
  };
  const listener = createListener(runtime);
  listener.start();
  const delayed = old.emit("message", delayedMessage);
  const recovery = createConnectionRecovery({
    getClient: () => current,
    restore: () => Promise.resolve(next.client),
    activate: (client) => {
      current = client;
      listening = false;
      listener.start();
    },
  });
  await recovery.reconnect();
  listener.start();
  release({ id: "100", from: "u-friend", to: "u-self" });
  await delayed;
  await old.emit("message", {});
  await old.emit("event", { type: "RECEIVE_MESSAGE" });
  await old.emit("call:incoming", {});
  await old.emit("call:cancel", {});
  await next.emit("message", {});
  await next.emit("message", {});
  await next.emit("event", { type: "RECEIVE_MESSAGE" });
  await next.emit("call:incoming", {});
  await next.emit("call:cancel", {});
  assertEquals(events, [
    "listening",
    "listening",
    "message",
    "message",
    "操作通知",
    "着信",
    "通話終了",
  ]);
  assertEquals(old.starts(), 1);
  assertEquals(next.starts(), 1);
});

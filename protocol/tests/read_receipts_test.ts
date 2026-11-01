import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import {
  createReadReceipts,
  normalizeReadRanges,
} from "../src/read_receipts.ts";

Deno.test("既読範囲は名前付きと数値キーの両形式を読み、64ビットIDを保持する", () => {
  const ranges = normalizeReadRanges(
    [{
      chatId: "c-fixture",
      ranges: {
        "u-self": [{ startMessageId: 0, endMessageId: 999 }],
        "u-one": { "0": { "1": 9007199254740993n, "2": 9007199254740995n } },
        "u-two": [{
          startMessageId: "9007199254740993",
          endMessageId: "9007199254740993",
        }],
        "u-invalid": [{ startMessageId: 20, endMessageId: 10 }],
        "u-rounded": [{ startMessageId: 0, endMessageId: 9007199254740993 }],
      },
    }, {
      chatId: "c-other",
      ranges: { "u-other": [{ startMessageId: 0, endMessageId: 999 }] },
    }],
    "c-fixture",
    "u-self",
  );
  assertEquals(ranges, [
    {
      userMid: "u-one",
      ranges: [{
        startMessageId: "9007199254740993",
        endMessageId: "9007199254740995",
      }],
    },
    {
      userMid: "u-two",
      ranges: [{
        startMessageId: "9007199254740993",
        endMessageId: "9007199254740993",
      }],
    },
  ]);
});

Deno.test("個別トークは相手だけを読み、自分や別の利用者の既読を使わない", () => {
  const raw = [{
    chatId: "u-peer",
    ranges: {
      "u-self": [{ startMessageId: 0, endMessageId: 100 }],
      "u-other": [{ startMessageId: 0, endMessageId: 100 }],
      "u-peer": [{ startMessageId: 0, endMessageId: 50 }],
    },
  }];
  assertEquals(normalizeReadRanges(raw, "u-peer", "u-self"), [
    {
      userMid: "u-peer",
      ranges: [{ startMessageId: "0", endMessageId: "50" }],
    },
  ]);
  assertEquals(normalizeReadRanges(raw, "u-peer", ""), []);
});

function receiptFixture(response: unknown = []) {
  let requests = 0;
  const client = {
    base: {
      talk: {
        getMessageReadRange: () => {
          requests++;
          return Promise.resolve(response);
        },
      },
    },
  } as unknown as Client;
  let current: Client | null = client;
  const events: { event: string; payload: Record<string, unknown> }[] = [];
  const service = createReadReceipts({
    getClient: () => current,
    selfMid: () => "u-self",
    emit: (event, payload) => events.push({ event, payload }),
  });
  return {
    service,
    events,
    requests: () => requests,
    replace: () => {
      current = null;
    },
  };
}

Deno.test("再表示時の既読範囲を取得し、同時の照会は1回にまとめる", async () => {
  const fixture = receiptFixture([{
    chatId: "u-peer",
    ranges: { "u-peer": [{ startMessageId: 0, endMessageId: 50 }] },
  }]);
  await Promise.all([
    fixture.service.refresh("u-peer"),
    fixture.service.refresh("u-peer"),
  ]);
  assertEquals(fixture.requests(), 1);
  assertEquals(fixture.events, [{
    event: "read_ranges",
    payload: {
      chatMid: "u-peer",
      readers: [
        {
          userMid: "u-peer",
          ranges: [{ startMessageId: "0", endMessageId: "50" }],
        },
      ],
    },
  }]);
});

Deno.test("ログアウト後の遅い応答を捨て、照会失敗では既読状態を消さない", async () => {
  const fixture = receiptFixture();
  const pending = fixture.service.refresh("u-peer");
  fixture.replace();
  await pending;
  assertEquals(fixture.events, []);
  const events: unknown[] = [];
  const service = createReadReceipts({
    getClient: () => ({
      base: {
        talk: {
          getMessageReadRange: () => {
            throw new Error("検証用の失敗");
          },
        },
      },
    } as unknown as Client),
    selfMid: () => "u-self",
    emit: (event) => events.push(event),
  });
  await service.refresh("u-peer");
  assertEquals(events, []);
});

Deno.test("ライブの既読は名前と数値のイベントを受け付け、読者を保持する", () => {
  const fixture = receiptFixture();
  assertEquals(
    fixture.service.handleOperation({
      type: 55,
      param1: "u-peer",
      param2: "u-peer",
      param3: "9007199254740993",
    }),
    true,
  );
  fixture.service.handleOperation({
    type: "NOTIFIED_READ_MESSAGE",
    param1: "u-peer",
    param2: "u-self",
    param3: "100",
  });
  fixture.service.handleOperation({
    type: "NOTIFIED_READ_MESSAGE",
    param1: "u-peer",
    param2: "u-other",
    param3: "100",
  });
  fixture.service.handleOperation({
    type: 55,
    param1: "u-peer",
    param2: "u-peer",
    param3: "pending-1",
  });
  assertEquals(
    fixture.service.handleOperation({ type: "SEND_CHAT_CHECKED" }),
    false,
  );
  assertEquals(fixture.events, [{
    event: "read_receipt",
    payload: {
      chatMid: "u-peer",
      userMid: "u-peer",
      messageId: "9007199254740993",
    },
  }]);
});

Deno.test("グループのライブ既読後に、読んだ範囲も再照会する", async () => {
  const fixture = receiptFixture();
  fixture.service.handleOperation({
    type: "NOTIFIED_READ_MESSAGE",
    param1: "c-fixture",
    param2: "u-one",
    param3: "50",
  });
  await fixture.service.refresh("c-fixture");
  assertEquals(fixture.requests(), 1);
  assertEquals(fixture.events[0], {
    event: "read_receipt",
    payload: { chatMid: "c-fixture", userMid: "u-one", messageId: "50" },
  });
  assertEquals(fixture.events[1].event, "read_ranges");
});

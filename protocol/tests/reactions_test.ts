import { assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import {
  createReactionService,
  decodeReactionType,
  DEFAULT_PAID_LAYOUT,
  emojiVersion,
  normalizeReactions,
  paidReactArgs,
  reactionError,
  reactionOperation,
} from "../src/reactions.ts";

type Json = Record<string, unknown>;
const PRODUCT = "5ac21a8c040ab15980c9b43f";
const CHAT = "c0123456789abcdef";

Deno.test("標準リアクションとLINE絵文字のリアクションを読み取る", () => {
  assertEquals(decodeReactionType({ predefinedReactionType: "LOVE" }), {
    choice: { kind: "LOVE" },
  });
  assertEquals(decodeReactionType({ predefinedReactionType: 7 }), {
    choice: { kind: "OMG" },
  });
  assertEquals(
    decodeReactionType({
      paidReactionType: {
        productId: PRODUCT,
        emojiId: "012",
        resourceType: "ANIMATION",
      },
    }),
    {
      choice: {
        kind: "EMOJI",
        emoji: `${PRODUCT}_012`,
        resourceType: 2,
        version: 1,
      },
    },
  );
  // linejsが知らない構造体は番号のまま届く。番号の並びも覚える。
  assertEquals(
    decodeReactionType({ "4": { "5": 1, "7": "003", "6": PRODUCT } }),
    {
      choice: {
        kind: "EMOJI",
        emoji: `${PRODUCT}_003`,
        resourceType: 1,
        version: 1,
      },
      layout: { outer: 4, product: 6, emoji: 7, resource: 5, version: 4 },
    },
  );
  assertEquals(decodeReactionType({ predefinedReactionType: 1 }), null);
  assertEquals(decodeReactionType({ "2": { "1": "../x", "2": "1" } }), null);
});

Deno.test("リアクション一覧は1人1件に絞り、不正な項目を捨てる", () => {
  const reactions = normalizeReactions([
    {
      fromUserMid: "u-a",
      atMillis: 10n,
      reactionType: { predefinedReactionType: "NICE" },
    },
    {
      fromUserMid: "u-a",
      atMillis: 30,
      reactionType: { predefinedReactionType: "SAD" },
    },
    { fromUserMid: "u-b", atMillis: 20, reactionType: { "2": {} } },
    { fromUserMid: "bad", atMillis: 5, reactionType: { "1": 2 } },
    { from: "u-c", at: 15, kind: "EMOJI", emoji: `${PRODUCT}_001` },
  ]);
  assertEquals(reactions, [
    {
      from: "u-c",
      at: 15,
      kind: "EMOJI",
      emoji: `${PRODUCT}_001`,
      resourceType: 1,
      version: 1,
    },
    { from: "u-a", at: 30, kind: "SAD" },
  ]);
  assertEquals(normalizeReactions(undefined), []);
});

Deno.test("自分と他人のリアクション操作、取り消しを読み取る", () => {
  const curr = JSON.stringify({
    chatMid: CHAT,
    curr: { predefinedReactionType: 3 },
  });
  assertEquals(
    reactionOperation(
      {
        type: "NOTIFIED_SEND_REACTION",
        param1: "123",
        param2: curr,
        param3: "u-p",
      },
      "u-self",
    ),
    { chatMid: CHAT, messageId: "123", from: "u-p", choice: { kind: "LOVE" } },
  );
  assertEquals(
    reactionOperation(
      {
        type: 139,
        param1: "123",
        param2: JSON.stringify({ chatMid: CHAT, curr: null }),
      },
      "u-self",
    ),
    { chatMid: CHAT, messageId: "123", from: "u-self", choice: null },
  );
  for (
    const op of [
      { type: 140, param1: "x", param2: curr, param3: "u-p" },
      { type: 140, param1: "1", param2: "{", param3: "u-p" },
      { type: 140, param1: "1", param2: '{"chatMid":"c1"}', param3: "u-p" },
      { type: 140, param1: "1", param2: curr, param3: "../u" },
      { type: 65, param1: "1", param2: curr, param3: "u-p" },
    ]
  ) assertEquals(reactionOperation(op, "u-self"), null);
});

Deno.test("LINE絵文字リアクションの送信形式と失敗理由", () => {
  assertEquals(
    paidReactArgs(
      9,
      "9007199254740993",
      { kind: "EMOJI", emoji: `${PRODUCT}_004`, resourceType: 2, version: 123 },
      DEFAULT_PAID_LAYOUT,
    ),
    [[12, 1, [
      [8, 1, 9],
      [10, 2, 9007199254740993n],
      [12, 3, [[12, 2, [[11, 1, PRODUCT], [11, 2, "004"], [8, 3, 2], [
        10,
        4,
        123n,
      ]]]]],
    ]]],
  );
  assertEquals(
    reactionError(
      new Error(
        '{"code":"MESSAGE_NOT_FOUND","reason":"Message too old for reaction"}',
      ),
    ),
    "reaction_expired",
  );
  assertEquals(
    reactionError(new Error('{"code":"USER_NOT_STICON_OWNER"}')),
    "reaction_emoji_unavailable",
  );
  assertEquals(reactionError(new Error("socket hang up")), "reaction_failed");
  assertEquals(
    reactionError(new Error("Invalid paidReactionType in reactionType")),
    "reaction_emoji_unavailable",
  );
});

Deno.test("custom reaction versions survive named and numeric decoding", () => {
  const choice = {
    kind: "EMOJI" as const,
    emoji: `${PRODUCT}_001`,
    resourceType: 2,
    version: 123,
  };
  assertEquals(
    decodeReactionType({
      paidReactionType: {
        productId: PRODUCT,
        emojiId: "001",
        resourceType: "ANIMATION",
        version: "123",
      },
    }),
    { choice },
  );
  assertEquals(
    decodeReactionType({
      "2": { "1": PRODUCT, "2": "001", "3": 2, "4": 123n },
    }),
    { choice, layout: DEFAULT_PAID_LAYOUT },
  );
  // resourceType is optional; version 1 must not be mistaken for that enum.
  assertEquals(
    decodeReactionType({ "2": { "1": PRODUCT, "2": "001", "4": 1 } }),
    {
      choice: { ...choice, resourceType: 1, version: 1 },
      layout: DEFAULT_PAID_LAYOUT,
    },
  );
  for (
    const value of [undefined, null, 0, -1, 1.5, "bad", "9007199254740993"]
  ) {
    assertEquals(emojiVersion(value), 1);
  }
});

Deno.test("recent emoji retain nondefault versions across a restart", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.service.observe([{
      reactions: [{
        kind: "EMOJI",
        emoji: `${PRODUCT}_001`,
        resourceType: 2,
        version: 7,
      }],
    }]);
    // Seeing the same emoji in ordinary text must not discard its known version.
    f.service.observe([{ emojis: [{ key: `${PRODUCT}_001` }] }]);
    const expected = [{ key: `${PRODUCT}_001`, resourceType: 2, version: 7 }];
    assertEquals(await f.service.recentEmojis(), expected);
    await f.service.flush();
    const restarted = fixture(dir);
    assertEquals(await restarted.service.recentEmojis(), expected);
    await restarted.service.flush();
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

function fixture(dataDir: string) {
  let now = 1000;
  const events: Json[] = [];
  const calls: unknown[][] = [];
  let failure: Error | null = null;
  let current: Client | null = null;
  const account = (mid: string | null) => {
    current = mid
      ? {
        base: {
          profile: { mid },
          getReqseq: () => Promise.resolve(42),
          talk: {
            protocolType: 4,
            requestPath: "/S4",
            react: (args: unknown) => {
              calls.push(["react", args]);
              return failure ? Promise.reject(failure) : Promise.resolve();
            },
            cancelReaction: (args: unknown) => {
              calls.push(["cancel", args]);
              return Promise.resolve();
            },
          },
          request: {
            request: (...args: unknown[]) => {
              calls.push(["request", ...args]);
              return Promise.resolve();
            },
          },
        },
      } as unknown as Client
      : null;
  };
  account("u-self");
  const service = createReactionService({
    dataDir,
    getClient: () => current,
    emit: (event, payload) => events.push({ event, ...payload }),
    now: () => now,
  });
  return {
    service,
    events,
    calls,
    account,
    tick: (at: number) => {
      now = at;
    },
    fail: (error: Error | null) => {
      failure = error;
    },
  };
}

const op = (from: string, curr: unknown, id = "77") => ({
  type: "NOTIFIED_SEND_REACTION",
  param1: id,
  param2: JSON.stringify({ chatMid: CHAT, curr }),
  param3: from,
});

Deno.test("取得より後に届いた操作だけを履歴へ重ね、アカウント別に保存する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    const message = {
      id: "77",
      to: CHAT,
      reactionsAt: 2000,
      reactions: [{
        fromUserMid: "u-a",
        atMillis: 1,
        reactionType: { predefinedReactionType: 2 },
      }],
    };
    f.tick(1500);
    f.service.handleOperation(op("u-b", { predefinedReactionType: 5 }));
    f.tick(2500);
    f.service.handleOperation(op("u-a", null));
    f.service.handleOperation(op("u-c", {
      paidReactionType: { productId: PRODUCT, emojiId: "002" },
    }));
    await f.service.flush();
    const [merged] = await f.service.merge(CHAT, [message]);
    // u-bの操作は取得前なので、サーバーの内容(u-bなし)を優先する。
    assertEquals(merged.reactions, [{
      kind: "EMOJI",
      emoji: `${PRODUCT}_002`,
      resourceType: 1,
      version: 1,
      from: "u-c",
      at: 2500,
    }]);
    assertEquals(
      f.events.map((e) => [e.event, e.from, e.messageId]),
      [
        ["message_reaction", "u-b", "77"],
        ["message_reaction", "u-a", "77"],
        ["message_reaction", "u-c", "77"],
      ],
    );
    // 1対1では chatMid が相手側・自分側のどちらでも、IDで照合する。
    const [direct] = await f.service.merge("u-peer", [{
      id: "77",
      reactionsAt: 2000,
    }]);
    assertEquals((direct.reactions as Json[]).length, 1);
    const [unrelated] = await f.service.merge(CHAT, [{ id: "78" }]);
    assertEquals(unrelated, { id: "78" });

    const stored = JSON.parse(
      await Deno.readTextFile(`${dir}/reactions-u-self.json`),
    );
    assertEquals(Object.keys(stored.messages), ["77"]);
    assertEquals(
      (await f.service.recentEmojis()).map((item) => item.key),
      [`${PRODUCT}_002`],
    );

    f.account("u-other");
    const [isolated] = await f.service.merge(CHAT, [{ id: "77" }]);
    assertEquals(isolated, { id: "77" });
    assertEquals(await f.service.recentEmojis(), []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("リアクションを送信・取消し、成功後だけ画面へ反映する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    assertEquals(
      await f.service.react(CHAT, "77", { kind: "FUN" }),
      { reacted: true },
    );
    assertEquals(f.calls[0], [
      "react",
      { id: 77n, reaction: "FUN", reqSeq: 42 },
    ]);
    await f.service.react(CHAT, "77", {
      kind: "EMOJI",
      emoji: `${PRODUCT}_010`,
    });
    assertEquals(f.calls[1].slice(0, 2), [
      "request",
      paidReactArgs(
        42,
        "77",
        { kind: "EMOJI", emoji: `${PRODUCT}_010`, resourceType: 1, version: 1 },
        DEFAULT_PAID_LAYOUT,
      ),
    ]);
    assertEquals(f.calls[1].slice(2), ["react", 4, true, "/S4"]);
    assertEquals(await f.service.cancel(CHAT, "77"), { reacted: false });
    assertEquals(f.calls[2], [
      "cancel",
      { cancelReactionRequest: { reqSeq: 42, messageId: 77n } },
    ]);
    assertEquals(f.events.map((e) => e.reaction && (e.reaction as Json).kind), [
      "FUN",
      "EMOJI",
      null,
    ]);

    f.fail(new Error("Message too old for reaction"));
    await assertRejects(
      () => f.service.react(CHAT, "77", { kind: "SAD" }),
      Error,
      "reaction_expired",
    );
    assertEquals(f.events.length, 3);
    await assertRejects(
      () => f.service.react(CHAT, "pending-1", { kind: "SAD" }),
      Error,
      "reaction_invalid_message",
    );
    await assertRejects(
      () => f.service.react(CHAT, "77", { kind: "UNDO" }),
      Error,
      "reaction_invalid_choice",
    );
    await f.service.flush();
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("受信したLINE絵文字リアクションの番号配置で送信する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    // 番号のままの形は履歴から来る。操作に同じ形が含まれた場合も覚える。
    f.service.handleOperation(
      op("u-a", { "5": { "1": PRODUCT, "2": "001", "3": 1 } }),
    );
    assertEquals(f.service.layout(), {
      outer: 5,
      product: 1,
      emoji: 2,
      resource: 3,
      version: 4,
    });
    normalizeReactions([{
      fromUserMid: "u-a",
      atMillis: 1,
      reactionType: { "3": { "2": PRODUCT, "1": "001" } },
    }], f.service.learnLayout);
    assertEquals(f.service.layout(), {
      outer: 3,
      product: 2,
      emoji: 1,
      resource: 3,
      version: 4,
    });
    await f.service.flush();
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

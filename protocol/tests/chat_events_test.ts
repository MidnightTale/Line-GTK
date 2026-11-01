import { assert, assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import {
  type ChatEvent,
  chatEventMessage,
  chatEventOf,
  createChatEvents,
  normalizeChatEvent,
  summarizeChatEvent,
} from "../src/chat_events.ts";

function history(key: string, args = "") {
  return {
    id: "9007199254740995",
    to: "c-group",
    from: "u-envelope",
    createdTime: 1_760_000_000_000,
    contentType: 18,
    contentMetadata: { LOC_KEY: key, LOC_ARGS: args },
  };
}

Deno.test("history notices decode rename, membership, pictures and invitation links", () => {
  const cases: Array<[string, string, ChatEvent]> = [
    ["C_PN", "u-alice\x1eNew <group>", {
      kind: "renamed",
      actorMid: "u-alice",
      targetMids: [],
      name: "New <group>",
    }],
    ["C_GI", "u-alice\x1eu-bob\x1fu-charlie\x1fu-bob", {
      kind: "invited",
      actorMid: "u-alice",
      targetMids: ["u-bob", "u-charlie"],
    }],
    ["C_MI", "u-alice\x1eu-bob", {
      kind: "invited",
      actorMid: "u-alice",
      targetMids: ["u-bob"],
    }],
    ["C_MJ", "u-bob\x1fu-charlie", {
      kind: "joined",
      actorMid: "u-bob",
      targetMids: ["u-bob", "u-charlie"],
    }],
    ["C_ML", "u-bob", {
      kind: "left",
      actorMid: "u-bob",
      targetMids: ["u-bob"],
    }],
    ["C_MR", "u-alice\x1eu-bob", {
      kind: "removed",
      actorMid: "u-alice",
      targetMids: ["u-bob"],
    }],
    ["C_MR", "u-bob", { kind: "removed", actorMid: "", targetMids: ["u-bob"] }],
    ["C_IC", "u-alice\x1eu-bob", {
      kind: "invitation_canceled",
      actorMid: "u-alice",
      targetMids: ["u-bob"],
    }],
    ["C_PI", "u-alice", {
      kind: "picture_changed",
      actorMid: "u-alice",
      targetMids: [],
    }],
    ["C_SN", "", { kind: "link_join_allowed", actorMid: "", targetMids: [] }],
    ["C_SP", "u-alice", {
      kind: "link_join_disabled",
      actorMid: "u-alice",
      targetMids: [],
    }],
  ];
  for (const [key, args, expected] of cases) {
    const raw = history(key, args);
    assertEquals(chatEventOf(raw), expected);
    const summary = summarizeChatEvent(raw)!;
    assertEquals(summary.chatEvent, expected);
    assertEquals(summary.id, raw.id);
    assertEquals(summary.createdTime, raw.createdTime);
    assertEquals(summary.contentType, "CHAT_EVENT");
    assertEquals(summary.text, "");
    assertEquals(summary.needsMedia, false);
    assertEquals(chatEventOf(summary), expected);
  }
});

Deno.test("unknown, malformed and legacy notices remain neutral system messages", () => {
  const unknown: ChatEvent = { kind: "unknown", actorMid: "", targetMids: [] };
  for (
    const raw of [
      history("__proto__"),
      history("FUTURE_EVENT", "u-alice"),
      history("C_PN", "u-alice"),
      history("C_PN", "Not a MID\x1eTitle"),
      history("C_PN", "u-alice\x1e"),
      history("C_MR", "u-bob\x1fNot a MID"),
      history("C_MI", "u-alice\x1eu-bob\x1eextra"),
      { contentType: "CHATEVENT", text: "[CHATEVENT]" },
      {
        contentType: "CHAT_EVENT",
        contentMetadata: { LOC_KEY: "C_PN", LOC_ARGS: ["bad"] },
      },
    ]
  ) {
    assertEquals(chatEventOf(raw), unknown);
    assertEquals(normalizeChatEvent(raw).text, "");
  }
  const text = { contentType: "NONE", text: "[CHATEVENT]" };
  assertEquals(chatEventOf(text), null);
  assertEquals(normalizeChatEvent(text), text);
});

Deno.test("live group changes and invitation cancellations use numeric and named operations", () => {
  for (
    const type of [11, 122, "NOTIFIED_UPDATE_GROUP", "NOTIFIED_UPDATE_CHAT"]
  ) {
    for (
      const [param3, kind] of [["1", "renamed"], ["2", "picture_changed"], [
        "4",
        "updated",
      ]] as const
    ) {
      assertEquals(
        chatEventMessage({ ...invitation, type, param3 }, "u-self")?.chatEvent,
        { kind, actorMid: "u-alice", targetMids: [] },
      );
    }
  }
  assertEquals(
    chatEventMessage({ ...invitation, type: 121, param2: "1" }, "u-self")
      ?.chatEvent,
    { kind: "renamed", actorMid: "u-self", targetMids: [] },
  );
  for (const type of [32, 126, "NOTIFIED_CANCEL_CHAT_INVITATION"]) {
    assertEquals(
      chatEventMessage({ ...invitation, type }, "u-self")?.chatEvent.kind,
      "invitation_canceled",
    );
  }
});

const invitation = {
  type: 124,
  revision: "9007199254740993",
  createdTime: 1_760_000_000_000,
  param1: "c-group",
  param2: "u-alice",
  param3: "u-bob\x1eu-charlie\x1eu-bob",
};
Deno.test("招待した人と複数の対象を分け、数値と名前付きの操作を同じイベントにする", () => {
  const message = chatEventMessage(invitation, "u-self")!;
  assertEquals(message.chatEvent, {
    kind: "invited",
    actorMid: "u-alice",
    targetMids: ["u-bob", "u-charlie"],
  });
  assertEquals(message.id, "event:9007199254740993");
  assertEquals(
    chatEventMessage(
      { ...invitation, type: "NOTIFIED_INVITE_INTO_CHAT" },
      "u-self",
    ),
    message,
  );
  const own = chatEventMessage({
    ...invitation,
    type: 123,
    param2: "u-bob\x1eu-charlie",
  }, "u-self")!;
  assertEquals(own.chatEvent.actorMid, "u-self");
  assertEquals(own.chatEvent.targetMids, ["u-bob", "u-charlie"]);
});
Deno.test("参加・退出・メンバー削除と旧グループ操作を区別する", () => {
  for (
    const [type, kind] of [
      [130, "joined"],
      [60, "joined"],
      [17, "joined"],
      [128, "left"],
      [61, "left"],
      [15, "left"],
      [133, "removed"],
      [19, "removed"],
    ] as const
  ) {
    const result = chatEventMessage({ ...invitation, type }, "u-self")!;
    assertEquals(result.chatEvent.kind, kind);
    assertEquals(result.chatEvent.actorMid, "u-alice");
    assertEquals(
      result.chatEvent.targetMids,
      kind === "removed" ? ["u-bob", "u-charlie"] : [],
    );
  }
  const seconds = chatEventMessage({
    ...invitation,
    type: 130,
    createdTime: 1_760_000_000,
  }, "u-self")!;
  assertEquals(seconds.createdTime, invitation.createdTime);
});
Deno.test("通常のメッセージ・不完全な操作・個別トーク・不正なIDはイベント化しない", () => {
  for (
    const patch of [{ type: 26 }, { param1: "u-friend" }, { param2: "" }, {
      param3: "",
    }, { param1: "c/../../other" }]
  ) assertEquals(chatEventMessage({ ...invitation, ...patch }, "u-self"), null);
  assertEquals(chatEventMessage(invitation, "u/../../other"), null);
});
function fixture(directory: string) {
  let mid = "u-self";
  let client: Client | null = {} as Client;
  const events: Record<string, unknown>[] = [];
  const profiles: Record<string, unknown>[][] = [];
  const runtime = {
    directory,
    getClient: () => client,
    selfMid: () => mid,
    observeProfiles: (messages: Record<string, unknown>[]) =>
      profiles.push(messages),
    emit: (event: string, payload: Record<string, unknown>) =>
      events.push({ event, ...payload }),
  };
  return {
    runtime,
    service: createChatEvents(runtime),
    events,
    profiles,
    switchAccount: () => {
      mid = "u-other";
      client = {} as Client;
    },
  };
}
Deno.test("重複受信は1件にし、保存・再起動・本文との時刻順統合・名前の照会に対応する", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const f = fixture(directory);
    f.service.handleOperation(invitation);
    f.service.handleOperation(invitation);
    await f.service.settled();
    assertEquals(f.events.length, 1);
    assertEquals(f.profiles[0].map((m) => m.from), [
      "u-alice",
      "u-bob",
      "u-charlie",
    ]);
    const restored = createChatEvents(f.runtime);
    const merged = await restored.merge("c-group", [{
      id: "1",
      createdTime: invitation.createdTime - 1000,
      text: "前",
    }, { id: "2", createdTime: invitation.createdTime + 1000, text: "後" }]);
    assertEquals(merged.map((m) => m.id), ["1", "event:9007199254740993", "2"]);
    const cropped = await restored.merge("c-group", [{
      id: "3",
      createdTime: invitation.createdTime + 1000,
    }]);
    assertEquals(cropped.length, 1);
    assertEquals(await restored.merge("c-other", []), []);
    assert(
      (await Deno.readTextFile(directory + "/events-u-self.json")).includes(
        "u-bob",
      ),
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});
Deno.test("別アカウントのイベントを混ぜず、ログイン変更前に届いた操作は保存しない", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const f = fixture(directory);
    f.service.handleOperation(invitation);
    f.switchAccount();
    await f.service.settled();
    assertEquals(f.events, []);
    assertEquals(await f.service.merge("c-group", []), []);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("壊れたキャッシュのイベントを無視し、通常の本文を失わない", async () => {
  const directory = await Deno.makeTempDir();
  try {
    await Deno.writeTextFile(
      directory + "/events-u-self.json",
      JSON.stringify([{
        id: "event:1",
        to: "c-group",
        contentType: "CHAT_EVENT",
        createdTime: 1000,
        chatEvent: {
          kind: "invited",
          actorMid: "u-alice",
          targetMids: "壊れた対象一覧",
        },
      }]),
    );
    const f = fixture(directory);
    const messages = [{ id: "42", text: "本文", createdTime: 500 }];
    assertEquals(await f.service.merge("c-group", messages), messages);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("history notices replace matching live events without losing nearby events", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const f = fixture(directory);
    f.service.handleOperation(invitation);
    f.service.handleOperation({ ...invitation, type: 130, revision: "3" });
    f.service.handleOperation({
      ...invitation,
      type: 122,
      param3: "1",
      revision: "2",
      createdTime: invitation.createdTime + 1,
    });
    await f.service.settled();
    const notice = summarizeChatEvent(
      history("C_GI", "u-alice\x1eu-bob\x1fu-charlie"),
    )!;
    const joined = {
      ...summarizeChatEvent(history("C_MJ", "u-alice")),
      id: "joined",
    };
    const merged = await f.service.merge("c-group", [notice, joined]);
    assertEquals(merged.map((m) => m.id), [notice.id, "joined", "event:2"]);
    const restored = createChatEvents(f.runtime);
    assertEquals(
      (await restored.merge("c-group", [notice, joined])).map((m) => m.id),
      [
        notice.id,
        "joined",
        "event:2",
      ],
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

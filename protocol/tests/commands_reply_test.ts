import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import { policyFor } from "../src/cache_policy.ts";

type Json = Record<string, unknown>;

function commandFixture(
  source: "通信" | "メモリ" | "ディスク",
  chatCachePath = "",
) {
  const message = { id: "2", createdTime: 20, text: "読み込んだ最新の本文" };
  const chat = {
    mid: "u-fixture",
    name: "検証用トーク",
    kind: "dm",
    avatarPath: null,
    lastActivity: 20,
    unread: 0,
    preview: "",
  };
  let chatCache: ReturnType<CommandRuntime["getChatCache"]> = {
    at: Date.now(),
    chats: [chat],
  };
  const events: Json[] = [];
  const okResults: unknown[] = [];
  let networkCalls = 0;
  let hydrated = 0;
  let activity = 20;
  const client = {
    base: {
      talk: {
        getMessageBoxes: () =>
          Promise.resolve({
            messageBoxes: [{
              id: chat.mid,
              lastDeliveredMessageId: { messageId: 2, deliveredTime: activity },
            }],
          }),
        getAllChatMids: () => Promise.resolve({ memberChatMids: [] }),
        getPreviousMessagesV2WithRequest: () => {
          networkCalls++;
          return Promise.resolve([message]);
        },
      },
    },
  } as unknown as Client;
  const noOp = () => Promise.resolve();
  const runtime: CommandRuntime = {
    getClient: () => client,
    setClient: () => {},
    isListening: () => false,
    setAuthDead: () => {},
    getChatCache: () => chatCache,
    setChatCache: (next) => {
      chatCache = next;
    },
    msgCache: new Map(
      source === "メモリ"
        ? [[chat.mid, { at: Date.now(), messages: [message] }]]
        : [],
    ),
    contactIndex: new Map(),
    boxCursor: new Map([[chat.mid, { messageId: 2, deliveredTime: 20 }]]),
    avatarDir: "",
    chatCachePath,
    storagePath: "",
    lineDevice: "DESKTOPWIN",
    lineVersion: "",
    refreshCachePolicy: () => Promise.resolve(policyFor("smart")),
    cachePolicy: () => policyFor("smart"),
    dedupe: (_key, operation) => operation(),
    storeBoxCursor: () => {},
    existingFile: () => Promise.resolve(null),
    loadDiskContacts: noOp,
    refreshContactIndex: noOp,
    hydrateAvatars: noOp,
    hydratePreviews: () => {
      hydrated++;
      return Promise.resolve();
    },
    bumpMediaEpoch: () => 0,
    loadDiskMessages: () =>
      Promise.resolve(source === "ディスク" ? [message] : null),
    saveDiskMessages: noOp,
    forUiMessages: (messages) => Promise.resolve(messages),
    hydrateMedia: noOp,
    summarizeTalkMessage: () => Promise.resolve(message),
    summarizeRawMessage: (raw) => Promise.resolve(raw as Json),
    saveAuth: noOp,
    loadAuth: () => Promise.resolve(null),
    loadAuthDevice: () => Promise.resolve("DESKTOPWIN"),
    clearAuth: noOp,
    myProfilePayload: () => Promise.resolve({}),
    patchE2eeGuards: () => {},
    startListen: () => {},
    sentMessagePayload: () => ({}),
    touchChatPreviewFromMessage: () => {},
    emitEvent: (event, payload) => events.push({ event, ...payload }),
    ok: (_id, result) => okResults.push(result),
    fail: (_id, error) => {
      throw new Error(error);
    },
  };
  return {
    commands: createCommandService(runtime),
    chat,
    events,
    okResults,
    networkCalls: () => networkCalls,
    hydrated: () => hydrated,
    setActivity: (next: number) => {
      activity = next;
    },
    runtime,
  };
}

for (const route of ["plain", "encrypted", "chat"] as const) {
  Deno.test(`返信先IDを${route}の送信経路でも維持する`, async () => {
    const fixture = commandFixture("メモリ");
    const calls: Record<string, unknown>[] = [];
    const client = fixture.runtime.getClient()!;
    client.base.talk.sendMessage = (options) => {
      calls.push(options as Record<string, unknown>);
      if (route === "chat" || (route === "encrypted" && !options.e2ee)) {
        return Promise.reject(new Error("経路の検証"));
      }
      return Promise.resolve({ id: "9007199254740994" } as never);
    };
    client.getChat = (() =>
      Promise.resolve({
        sendMessage: (options: Record<string, unknown>) => {
          calls.push(options);
          return Promise.resolve({ raw: { id: "9007199254740994" } });
        },
      })) as unknown as typeof client.getChat;
    const results: unknown[] = [];
    const commands = createCommandService({
      ...fixture.runtime,
      summarizeTalkMessage: () =>
        Promise.resolve({ id: "9007199254740994", text: "返信" }),
      sentMessagePayload: () => ({ id: "9007199254740994", text: "返信" }),
      ok: (_id, result) => results.push(result),
    });
    await commands.sendMessage(1, fixture.chat.mid, "返信", "9007199254740993");
    assertEquals(
      calls.length,
      route === "plain" ? 1 : route === "encrypted" ? 2 : 3,
    );
    for (const call of calls) {
      assertEquals(call.relatedMessageId, "9007199254740993");
    }
    assertEquals(
      (results[0] as { message: { replyTo: unknown } }).message.replyTo,
      { messageId: "9007199254740993" },
    );
  });
}

Deno.test("返信先が不正なら送信せず、送信失敗でも引用なしに変換しない", async () => {
  const fixture = commandFixture("メモリ");
  let sends = 0;
  const failures: string[] = [];
  const client = fixture.runtime.getClient()!;
  client.base.talk.sendMessage = () => {
    sends++;
    return Promise.reject(new Error("送信不可"));
  };
  client.getChat = (() =>
    Promise.resolve({
      sendMessage: (options: { relatedMessageId?: string }) => {
        sends++;
        assertEquals(options.relatedMessageId, "42");
        return Promise.reject(new Error("送信不可"));
      },
    })) as unknown as typeof client.getChat;
  const commands = createCommandService({
    ...fixture.runtime,
    fail: (_id, error) => failures.push(error),
  });
  await commands.sendMessage(1, fixture.chat.mid, "返信", "pending-1");
  assertEquals(sends, 0);
  assertEquals(failures, ["invalid_reply_target"]);
  await commands.sendMessage(2, fixture.chat.mid, "返信", "42");
  assertEquals(sends, 3);
  assertEquals(failures, ["invalid_reply_target", "送信不可"]);
});

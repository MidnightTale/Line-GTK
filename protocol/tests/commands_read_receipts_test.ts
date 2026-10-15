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
  let reads = 0;
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
    refreshReadReceipts: () => {
      reads++;
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
    reads: () => reads,
  };
}

for (const source of ["通信", "メモリ", "ディスク"] as const) {
  Deno.test(`${source}からトークを開く場合にも既読範囲を照会する`, async () => {
    const fixture = commandFixture(source);
    await fixture.commands.fetchMessages(1, fixture.chat.mid);
    assertEquals(fixture.reads(), 1);
    assertEquals(fixture.networkCalls(), source === "メモリ" ? 0 : 1);
    assertEquals(fixture.okResults.length, 1);
  });
}

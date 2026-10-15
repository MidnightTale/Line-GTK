import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { type CommandRuntime, createCommandService } from "../src/commands.ts";
import { policyFor } from "../src/cache_policy.ts";
import { createChatPreviews } from "../src/chat_previews.ts";
import { atomicWriteJson } from "../src/storage.ts";

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
  const previews = createChatPreviews({
    getChats: () => chatCache?.chats ?? [],
    previewLine: (message) => `相手: ${message.text}`,
    fallback: () => "タップして開く",
    emit: (payload) => events.push({ event: "chat_preview", ...payload }),
    save: () => Promise.resolve(),
  });
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
    saveChatCache: () => atomicWriteJson(chatCachePath, chatCache),
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
    updateChatPreview: previews.updateFromMessages,
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

for (const source of ["通信", "メモリ", "ディスク"] as const) {
  Deno.test(`${source}から本文を開くと一覧の読み込み中も解消する`, async () => {
    const fixture = commandFixture(source);
    await fixture.commands.fetchMessages(1, fixture.chat.mid);
    assertEquals(fixture.chat.preview, "相手: 読み込んだ最新の本文");
    assertEquals(
      fixture.events.filter((event) => event.event === "chat_preview"),
      [{
        event: "chat_preview",
        mid: fixture.chat.mid,
        preview: "相手: 読み込んだ最新の本文",
        lastActivity: 20,
      }],
    );
    assertEquals(fixture.okResults.length, 1);
    assertEquals(fixture.networkCalls(), source === "メモリ" ? 0 : 1);
  });
}

Deno.test("一覧がメモリキャッシュでも未取得のプレビューを補完する", async () => {
  const fixture = commandFixture("メモリ");
  await fixture.commands.listChats(1);
  assertEquals(fixture.hydrated(), 1);
  assertEquals(fixture.runtime.getChatCache()?.chats[0].preview, "");
});

Deno.test("再起動後の一覧更新で保存済みのプレビューを失わない", async () => {
  const directory = await Deno.makeTempDir();
  const fixture = commandFixture("通信", `${directory}/chats.json`);
  try {
    await Deno.writeTextFile(
      fixture.runtime.chatCachePath,
      JSON.stringify({
        at: Date.now(),
        chats: [{ ...fixture.chat, preview: "相手: 保存済みの本文" }],
      }),
    );
    fixture.runtime.setChatCache(null);
    await fixture.commands.listChats(1);
    assertEquals(
      fixture.runtime.getChatCache()?.chats[0].preview,
      "相手: 保存済みの本文",
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("新着のある一覧では古いプレビューを再取得の対象に戻す", async () => {
  const directory = await Deno.makeTempDir();
  const fixture = commandFixture("通信", `${directory}/chats.json`);
  fixture.chat.preview = "相手: 古い本文";
  fixture.setActivity(30);
  try {
    await fixture.commands.listChats(1, true);
    assertEquals(fixture.runtime.getChatCache()?.chats[0].preview, "");
    assertEquals(fixture.runtime.getChatCache()?.chats[0].lastActivity, 30);
    assertEquals(fixture.hydrated(), 1);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

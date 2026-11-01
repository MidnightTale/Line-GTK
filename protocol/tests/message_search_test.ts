import { assertEquals, assertRejects, assertThrows } from "@std/assert";
import {
  loadSearchHistory,
  searchContext,
  searchMessages,
} from "../src/message_search.ts";

const message = (id: string, text = "Hello", createdTime = 1) => ({
  id,
  text,
  createdTime,
  contentType: "TEXT",
});

Deno.test("search uses literal Unicode-normalized case-insensitive substrings", () => {
  const messages = [
    message("1", "ＣＡＦÉ"),
    message("2", "cafe\u0301"),
    message("3", "[.*]"),
  ];
  assertEquals(searchMessages(messages, " café ").total, 2);
  assertEquals(searchMessages(messages, "[.*]").messages[0].id, "3");
  assertEquals(searchMessages(messages, "  ").total, 0);
  assertEquals(
    searchMessages([{ ...message("4", ""), fileName: "Report.PDF" }], "pdf")
      .total,
    1,
  );
});

Deno.test("search excludes unsent, undecryptable, pending and event messages", () => {
  const messages = [
    message("1"),
    { ...message("2"), contentType: "UNSENT" },
    { ...message("3"), decryptionFailed: true },
    message("pending-4"),
    { ...message("5"), chatEvent: { kind: "join" } },
    { ...message("6"), contentType: "CHAT_EVENT" },
  ];
  assertEquals(searchMessages(messages, "hello").messages.map((m) => m.id), [
    "1",
  ]);
});

Deno.test("search is newest-first and paginated with exact large ID ordering", () => {
  const messages = Array.from(
    { length: 102 },
    (_, index) => message(String(index + 1), "hello", index),
  );
  const first = searchMessages(messages, "hello");
  assertEquals(first.total, 102);
  assertEquals(first.messages.length, 50);
  assertEquals(first.messages[0].id, "102");
  assertEquals(first.hasMore, true);
  assertEquals(
    searchMessages(messages, "hello", 100).messages.map((m) => m.id),
    ["2", "1"],
  );
  assertEquals(searchMessages(messages, "hello", 100).hasMore, false);
  assertEquals(searchMessages(messages, "hello", NaN).offset, 0);
  assertEquals(
    searchMessages(
      [message("9007199254740992"), message("9007199254740993")],
      "hello",
    ).messages[0].id,
    "9007199254740993",
  );
});

Deno.test("search context is bounded and contains the target even at history edges", () => {
  const messages = Array.from(
    { length: 500 },
    (_, index) => message(String(index + 1), "hello", index),
  );
  for (const id of ["1", "250", "500"]) {
    const context = searchContext(messages.slice().reverse(), id);
    assertEquals(context.length <= 50, true);
    assertEquals(context.some((m) => m.id === id), true);
    assertEquals(
      context.every((m, i) =>
        i === 0 || Number(m.createdTime) > Number(context[i - 1].createdTime)
      ),
      true,
    );
  }
  assertThrows(() => searchContext(messages, "501"));
  assertThrows(() =>
    searchContext([{ ...message("1"), contentType: "UNSENT" }], "1")
  );
});

Deno.test("saved search isolates accounts, prefers live updates and keeps old history", async () => {
  const owner = {};
  const runtime = {
    session: () => ({ accountMid: "alice", owner }),
    disk: () =>
      Promise.resolve({
        accountMid: "bob",
        messages: [message("1", "private")],
      }),
    memory: () => [message("2", "current")],
    mask: (messages: Record<string, unknown>[]) => Promise.resolve(messages),
  };
  assertEquals((await loadSearchHistory(runtime)).map((m) => m.id), ["2"]);
  runtime.disk = () =>
    Promise.resolve({
      accountMid: "alice",
      messages: [message("1", "old"), message("2", "stale")],
    });
  assertEquals((await loadSearchHistory(runtime)).map((m) => m.text), [
    "old",
    "current",
  ]);
  runtime.disk = () => Promise.reject(new Error("missing cache"));
  assertEquals((await loadSearchHistory(runtime)).map((m) => m.id), ["2"]);
});

Deno.test("saved search rejects account changes during disk reads and unsend masking", async () => {
  for (const stage of ["disk", "mask"]) {
    let owner = {};
    const runtime = {
      session: () => ({ accountMid: "alice", owner }),
      disk: () => {
        if (stage === "disk") owner = {};
        return Promise.resolve({
          accountMid: "alice",
          messages: [message("1")],
        });
      },
      memory: () => [],
      mask: (messages: Record<string, unknown>[]) => {
        if (stage === "mask") owner = {};
        return Promise.resolve(messages);
      },
    };
    await assertRejects(
      () => loadSearchHistory(runtime),
      Error,
      "message_search_cancelled",
    );
  }
  await assertRejects(
    () =>
      loadSearchHistory({
        session: () => null,
        disk: () => Promise.resolve({ accountMid: "alice", messages: [] }),
        memory: () => [],
        mask: (messages) => Promise.resolve(messages),
      }),
    Error,
    "not_logged_in",
  );
});

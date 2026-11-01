import { assertEquals, assertRejects, assertThrows } from "@std/assert";
import type { Client } from "@evex/linejs";
import {
  fetchMentionMembers,
  lineMentions,
  mentionMetadata,
} from "../src/mentions.ts";

Deno.test("mentions round trip Unicode scalar positions through LINE UTF-16 offsets and JSON cache", () => {
  const text = "😀 こんにちは @An 👋 @สมชาย";
  const chars = [...text];
  const first = chars.indexOf("@");
  const second = chars.lastIndexOf("@");
  const spans = [{ start: first, end: first + 3, mid: "u-an" }, {
    start: second,
    end: chars.length,
    mid: "u-thai",
  }];
  const encoded = mentionMetadata(text, spans);
  const wire = JSON.parse(encoded.contentMetadata.MENTION).MENTIONEES;
  assertEquals(wire[0].S, String(first + 1));
  assertEquals(wire[1].S, String(second + 2));
  assertEquals(lineMentions(text, encoded.contentMetadata), spans);
  assertEquals(JSON.parse(JSON.stringify({ mentions: spans })).mentions, spans);
  assertEquals(mentionMetadata(text, undefined), {
    mentions: [],
    contentMetadata: {},
  });
});

Deno.test("incoming mentions ignore malformed metadata, split surrogate pairs, overlap and invalid identities", () => {
  const text = "😀 @Me @You";
  for (const MENTION of ["{", "null", "{}", "x".repeat(100_001)]) {
    assertEquals(lineMentions(text, { MENTION }), []);
  }
  const entries = [
    { S: "1", E: "2", M: "u-broken" },
    { S: "3", E: "6", M: "u-me" },
    { S: "4", E: "7", M: "u-overlap" },
    { S: "7", E: "11", M: "<injected>" },
    { S: "7", E: "99", M: "u-outside" },
    { S: "7", E: "11", A: "1" },
    null,
  ];
  assertEquals(
    lineMentions(text, { MENTION: JSON.stringify({ MENTIONEES: entries }) }),
    [
      { start: 2, end: 5, mid: "u-me" },
      { start: 6, end: 10, mid: "", all: true },
    ],
  );
});

Deno.test("outgoing mentions reject invalid ranges, overlaps and ambiguous identities", () => {
  for (
    const input of [
      {},
      [{ start: -1, end: 3, mid: "u-me" }],
      [{ start: 0.5, end: 3, mid: "u-me" }],
      [{ start: 0, end: 99, mid: "u-me" }],
      [{ start: 3, end: 3, mid: "u-me" }],
      [{ start: 0, end: 3, mid: "c-group" }],
      [{ start: 0, end: 3, mid: "u-me", all: true }],
      [{ start: 0, end: 3, mid: "", all: "1" }],
      [{ start: 0, end: 3, mid: "", all: false }],
      [{ start: 0, end: 3, all: true }],
      [{ start: 0, end: 3, mid: "u-me" }, { start: 2, end: 4, mid: "u-other" }],
      Array(101).fill({ start: 0, end: 3, mid: "u-me" }),
    ]
  ) {
    assertThrows(
      () => mentionMetadata("@Me @You", input),
      Error,
      "invalid_mentions",
    );
  }
});

Deno.test("member lookup includes nonfriends, excludes invitees and unrelated chats, and preserves duplicate names", async () => {
  const calls: string[][] = [];
  const client = {
    base: {
      talk: {
        getChats: (options: Record<string, unknown>) => {
          assertEquals(options, {
            chatMids: ["c-group"],
            withMembers: true,
            withInvitees: false,
          });
          return Promise.resolve({
            chats: [{
              chatMid: "c-other",
              extra: { groupExtra: { memberMids: { "u-other": 1 } } },
            }, {
              chatMid: "c-group",
              extra: {
                groupExtra: {
                  memberMids: { "u-friend": 1, "u-stranger": 2 },
                  inviteeMids: { "u-invited": 3 },
                },
              },
            }],
          });
        },
        getContacts: ({ mids }: { mids: string[] }) => {
          calls.push(mids);
          return Promise.resolve(
            mids.map((mid) => ({ mid, displayName: "Same name" })),
          );
        },
      },
    },
  } as unknown as Client;
  assertEquals(await fetchMentionMembers(client, "c-group"), [{
    mid: "u-friend",
    name: "Same name",
  }, { mid: "u-stranger", name: "Same name" }]);
  assertEquals(calls, [["u-friend", "u-stranger"]]);
  await assertRejects(
    () => fetchMentionMembers(client, "u-peer"),
    Error,
    "invalid_group",
  );
});

Deno.test("all-member mentions round trip alongside individual mentions with Unicode offsets", () => {
  const text = "😀 @all @Me";
  const spans = [
    { start: 2, end: 6, mid: "", all: true },
    { start: 7, end: 10, mid: "u-me" },
  ];
  const encoded = mentionMetadata(text, spans);
  assertEquals(JSON.parse(encoded.contentMetadata.MENTION), {
    MENTIONEES: [
      { S: "3", E: "7", A: "1" },
      { S: "8", E: "11", M: "u-me" },
    ],
  });
  assertEquals(encoded.mentions, spans);
  assertEquals(lineMentions(text, encoded.contentMetadata), spans);
  assertEquals(mentionMetadata("@all", []).contentMetadata, {});
});

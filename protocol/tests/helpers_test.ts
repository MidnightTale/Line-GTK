import { assertEquals } from "@std/assert";
import { picturePathOf, profileUrl, squareObsUrl } from "../src/contacts.ts";
import {
  coerceI64,
  isMediaType,
  normalizeRaw,
  sticonResources,
  talkChatMid,
} from "../src/messages.ts";

Deno.test("message normalization supplies metadata", () => {
  assertEquals(normalizeRaw({ text: "hello" }), {
    text: "hello",
    contentMetadata: {},
  });
  assertEquals(isMediaType("IMAGE"), true);
  assertEquals(isMediaType("NONE"), false);
});

Deno.test("thrift integer coercion preserves large identifiers", () => {
  assertEquals(coerceI64("42"), 42);
  assertEquals(coerceI64("9007199254740993"), 9007199254740993n);
});

Deno.test("contact picture helpers normalize LINE paths", () => {
  assertEquals(picturePathOf({ pictureStatus: "/abc" }), "/abc");
  assertEquals(profileUrl("/abc"), "https://profile.line-scdn.net/abc");
  assertEquals(
    profileUrl("https://example.test/photo"),
    "https://example.test/photo",
  );
});

Deno.test("OpenChat picture helper uses the OBS object directly", () => {
  assertEquals(squareObsUrl("0habc"), "https://obs.line-scdn.net/0habc");
  assertEquals(
    squareObsUrl("https://example.test/square.jpg"),
    "https://example.test/square.jpg",
  );
});

Deno.test("LINE emoji replacement metadata exposes its STICON CDN identity", () => {
  assertEquals(
    sticonResources({
      REPLACE: JSON.stringify({
        sticon: {
          resources: [{
            S: 0,
            E: 14,
            productId: "5ac1bfd5040ab15980c9b435",
            sticonId: "001",
            resourceType: "STATIC",
          }],
        },
      }),
    }),
    [{
      start: 0,
      end: 14,
      productId: "5ac1bfd5040ab15980c9b435",
      sticonId: "001",
      resourceType: "STATIC",
    }],
  );
  assertEquals(sticonResources({ REPLACE: "not-json" }), []);
});

Deno.test("Talk messages route incoming group traffic to the group MID", () => {
  assertEquals(
    talkChatMid({ from: "u-sender", to: "c-group", mine: false }),
    "c-group",
  );
  assertEquals(
    talkChatMid({ from: "u-sender", to: "u-me", mine: false }),
    "u-sender",
  );
  assertEquals(
    talkChatMid({ from: "u-me", to: "u-peer", mine: true }),
    "u-peer",
  );
  assertEquals(
    talkChatMid({ from: "u-sender", to: "r-room", mine: false }),
    "r-room",
  );
});

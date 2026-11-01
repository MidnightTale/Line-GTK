import { assertEquals } from "@std/assert";
import { picturePathOf, profileUrl } from "../src/contacts.ts";
import {
  coerceI64,
  conversationMid,
  isMediaType,
  normalizeRaw,
} from "../src/messages.ts";

Deno.test("group and room notices stay in their conversation instead of the actor's DM", () => {
  for (const to of ["c-group", "r-room"]) {
    assertEquals(conversationMid({ to, from: "u-actor", mine: false }), to);
  }
  assertEquals(
    conversationMid({ to: "u-self", from: "u-friend", mine: false }),
    "u-friend",
  );
});

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

import { assertEquals } from "@std/assert";
import { contactChatRow } from "../src/chat_rows.ts";

const contact = {
  name: "Alice",
  kind: "dm",
  picturePath: null,
  muted: false,
};

Deno.test("contact events create a contact without inventing message activity", () => {
  const row = contactChatRow("u-alice", contact);
  assertEquals(row.contactOnly, true);
  assertEquals(row.lastActivity, 0);
  assertEquals(row.preview, "");
  assertEquals(row.unread, 0);
  assertEquals(
    contactChatRow("u-bot", { ...contact, kind: "bot" }).contactOnly,
    true,
  );
  assertEquals(contactChatRow("c-group", contact).contactOnly, false);
});

Deno.test("profile changes preserve conversation state and activity", () => {
  const previous = {
    ...contactChatRow("u-alice", contact),
    contactOnly: false,
    lastActivity: 123,
    unread: 2,
    preview: "Hello",
    avatarPath: "/cached-avatar.jpg",
  };
  const updated = contactChatRow(
    previous.mid,
    { ...contact, name: "Alice updated", muted: true },
    previous,
  );
  assertEquals(updated, { ...previous, name: "Alice updated", muted: true });
});

Deno.test("profile changes do not classify a legacy conversation as an empty contact", () => {
  const previous = {
    mid: "u-alice",
    name: "Alice",
    kind: "dm",
    lastActivity: 123,
    unread: 0,
    preview: "Hello",
    avatarPath: null,
  };
  const updated = contactChatRow(previous.mid, contact, previous);
  assertEquals(updated.contactOnly, undefined);
  assertEquals(updated.lastActivity, 123);
  assertEquals(updated.preview, "Hello");
});

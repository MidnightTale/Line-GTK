import { assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createUserProfiles } from "../src/user_profiles.ts";

function fixture() {
  const calls: string[] = [];
  const events: Record<string, unknown>[] = [];
  let fail = false;
  let release: (() => void) | null = null;
  let hold = false;
  const contact: Record<string, unknown> = {
    displayName: "表示名",
    displayNameOverridden: "",
    statusMessage: " こんにちは ",
    picturePath: "/pic",
  };
  const client = {
    base: {
      talk: {
        getContact: async ({ mid }: { mid: string }) => {
          calls.push(mid);
          if (hold) await new Promise<void>((r) => (release = r));
          if (fail) throw new Error("boom");
          return { mid, ...contact };
        },
      },
    },
  } as unknown as Client;
  let current: Client | null = client;
  let account = "u-self";
  let time = 0;
  const service = createUserProfiles({
    getClient: () => current,
    selfMid: () => account,
    avatarPathFor: (mid) => Promise.resolve(`/avatar/${mid}.jpg`),
    kindOf: () => "dm",
    emit: (_event, payload) => events.push(payload),
    now: () => time,
  });
  return {
    service,
    calls,
    events,
    contact,
    setFail: (v: boolean) => (fail = v),
    setHold: (v: boolean) => (hold = v),
    release: () => release?.(),
    setAccount: (v: string) => (account = v),
    setClient: (v: Client | null) => (current = v),
    advance: (ms: number) => (time += ms),
  };
}

Deno.test("user profile rejects invalid mids", async () => {
  const f = fixture();
  for (const mid of ["", "c123", "u bad", "u/x", 5, null]) {
    await assertRejects(() => f.service.get(mid), Error, "invalid_user");
  }
  assertEquals(f.calls, []);
});

Deno.test("user profile prefers override and emits avatar", async () => {
  const f = fixture();
  assertEquals((await f.service.get("u-a")).name, "表示名");
  f.contact.displayNameOverridden = "あだ名";
  const profile = await f.service.get("u-b");
  assertEquals(profile.name, "あだ名");
  assertEquals(profile.statusMessage, "こんにちは");
  assertEquals(profile.avatarPath, "/avatar/u-b.jpg");
  assertEquals(f.events.at(-1), {
    mid: "u-b",
    avatarPath: "/avatar/u-b.jpg",
  });
  assertEquals((await f.service.get("u-self")).isSelf, true);
});

Deno.test("user profile caches, dedupes and honors force and ttl", async () => {
  const f = fixture();
  f.setHold(true);
  const a = f.service.get("u-a");
  const b = f.service.get("u-a");
  f.release();
  await Promise.all([a, b]);
  assertEquals(f.calls.length, 1);
  f.setHold(false);
  await f.service.get("u-a");
  assertEquals(f.calls.length, 1);
  await f.service.get("u-a", true);
  assertEquals(f.calls.length, 2);
  f.advance(600_001);
  await f.service.get("u-a");
  assertEquals(f.calls.length, 3);
});

Deno.test("user profile failure does not poison the cache", async () => {
  const f = fixture();
  f.setFail(true);
  await assertRejects(() => f.service.get("u-a"), Error, "boom");
  f.setFail(false);
  assertEquals((await f.service.get("u-a")).name, "表示名");
  assertEquals(f.calls.length, 2);
});

Deno.test("user profile cache is isolated per account", async () => {
  const f = fixture();
  await f.service.get("u-a");
  f.setAccount("u-other");
  await f.service.get("u-a");
  assertEquals(f.calls.length, 2);
  f.setClient(null);
  await assertRejects(() => f.service.get("u-a"), Error, "not_logged_in");
});

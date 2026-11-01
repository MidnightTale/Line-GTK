import { assert, assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createMessageSenders } from "../src/message_senders.ts";

const groupMessage = (from: string) => ({
  from,
  to: "c-group",
  mine: false,
  text: "本文",
});
async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 15));
}

Deno.test("history event actors and targets resolve names even without envelope senders", async () => {
  const f = fixture();
  f.service.observe([{
    to: "c-group",
    from: "",
    contentType: "CHATEVENT",
    contentMetadata: {
      LOC_KEY: "C_GI",
      LOC_ARGS: "u-alice\x1eu-bob\x1fu-charlie",
    },
  }]);
  await settle();
  assertEquals(f.batches.flat().sort(), ["u-alice", "u-bob", "u-charlie"]);
});
function fixture() {
  let clock = 0;
  let mid = "u-self";
  const events: Array<{ mid: string; name: string }> = [];
  const batches: string[][] = [];
  const singles: string[] = [];
  const known = new Map<string, string>();
  const talk = {
    getContacts: ({ mids }: { mids: string[] }) => {
      batches.push(mids);
      return Promise.resolve(
        mids.map((mid) => ({
          mid,
          displayName: `名前:${mid}`,
          displayNameOverridden: "",
        })),
      );
    },
    getContact: ({ mid }: { mid: string }) => {
      singles.push(mid);
      return Promise.resolve({
        mid,
        displayName: `個別:${mid}`,
        displayNameOverridden: "",
      });
    },
  };
  let client: Client | null = { base: { talk } } as unknown as Client;
  return {
    service: createMessageSenders({
      getClient: () => client,
      selfMid: () => mid,
      knownName: (mid) => known.get(mid),
      emit: (mid, name) => events.push({ mid, name }),
      now: () => clock,
    }),
    talk,
    events,
    batches,
    singles,
    known,
    advance: (ms: number) => {
      clock += ms;
    },
    changeSession: () => {
      mid = "u-another-self";
      client = { base: { talk } } as unknown as Client;
    },
    logout: () => {
      client = null;
    },
  };
}

Deno.test("グループ・複数人トークの送信者と引用元だけを取得し、本文を変更しない", async () => {
  const f = fixture();
  const messages = [
    groupMessage("u-alice"),
    groupMessage("u-alice"),
    { ...groupMessage("u-bob"), to: "r-room" },
    { ...groupMessage("u-ignored"), to: "u-self" },
    {
      ...groupMessage("u-self"),
      mine: true,
      replyTo: { from: "u-quote", mine: false },
    },
  ];
  const original = structuredClone(messages);
  f.service.observe(messages);
  assertEquals(messages, original);
  assertEquals(f.events, []);
  await settle();
  assertEquals(f.batches.flat().sort(), ["u-alice", "u-bob", "u-quote"]);
  assertEquals(f.events.length, 3);
});

Deno.test("既知の名前は即座に通知し、同じ送信者の通信は重複しない", async () => {
  const f = fixture();
  f.known.set("u-known", "既知の名前");
  f.service.observe([groupMessage("u-known")]);
  assertEquals(f.events, [{ mid: "u-known", name: "既知の名前" }]);
  f.service.observe([groupMessage("u-alice")]);
  f.service.observe([groupMessage("u-alice")]);
  await settle();
  f.service.observe([groupMessage("u-alice")]);
  await settle();
  assertEquals(f.batches, [["u-alice"]]);
  assertEquals(f.events.at(-1), { mid: "u-alice", name: "名前:u-alice" });
});

Deno.test("一括取得の失敗・欠落は個別取得へ進み、1人の失敗でほかを隠さない", async () => {
  const f = fixture();
  f.talk.getContacts = () => Promise.reject(new Error("一括取得失敗"));
  f.talk.getContact = ({ mid }) =>
    mid === "u-missing"
      ? Promise.reject(new Error("取得不可"))
      : Promise.resolve({
        mid,
        displayName: "友だち未登録のメンバー",
        displayNameOverridden: "",
      });
  f.service.observe([groupMessage("u-alice"), groupMessage("u-missing")]);
  await settle();
  assertEquals(f.events, [{ mid: "u-alice", name: "友だち未登録のメンバー" }]);
  f.service.observe([groupMessage("u-missing")]);
  await settle();
  assertEquals(f.events.length, 1);
  f.advance(60_001);
  f.talk.getContacts = () => Promise.resolve([]);
  f.talk.getContact = ({ mid }) =>
    Promise.resolve({
      mid,
      displayName: "再試行で取得",
      displayNameOverridden: "",
    });
  f.service.observe([groupMessage("u-missing")]);
  await settle();
  assertEquals(f.events.at(-1), { mid: "u-missing", name: "再試行で取得" });
});

Deno.test("多数の送信者も20人ずつ、同時2件までで取得する", async () => {
  const f = fixture();
  let active = 0;
  let peak = 0;
  f.talk.getContacts = async ({ mids }) => {
    assert(mids.length <= 20);
    active++;
    peak = Math.max(peak, active);
    await settle();
    active--;
    return mids.map((mid) => ({
      mid,
      displayName: mid + "の名前",
      displayNameOverridden: "",
    }));
  };
  f.service.observe(
    Array.from({ length: 45 }, (_, n) => groupMessage(`u-${n}`)),
  );
  for (let n = 0; n < 20 && f.events.length < 45; n++) await settle();
  assertEquals(f.events.length, 45);
  assertEquals(peak, 2);
  assertEquals(active, 0);
});

Deno.test("ログアウトやセッション変更後の遅い名前は通知しない", async () => {
  const f = fixture();
  let finish!: () => void;
  f.talk.getContacts = ({ mids }) =>
    new Promise((resolve) => {
      finish = () =>
        resolve(
          mids.map((mid) => ({
            mid,
            displayName: "古いアカウントの名前",
            displayNameOverridden: "",
          })),
        );
    });
  f.service.observe([groupMessage("u-alice")]);
  await Promise.resolve();
  f.changeSession();
  f.service.observe([]);
  finish();
  await settle();
  assertEquals(f.events, []);
  f.logout();
  f.service.observe([groupMessage("u-bob")]);
  await settle();
  assertEquals(f.events, []);
});

Deno.test("名前の変更・キャッシュ期限と、個別の表示名を反映する", async () => {
  const f = fixture();
  f.service.observe([groupMessage("u-alice")]);
  await settle();
  f.talk.getContacts = ({ mids }) =>
    Promise.resolve(
      mids.map((mid) => ({
        mid,
        displayName: "元の名前",
        displayNameOverridden: "変更した名前",
      })),
    );
  f.service.refresh("u-alice");
  await settle();
  assertEquals(f.events.at(-1)?.name, "変更した名前");
  f.advance(600_001);
  f.service.observe([groupMessage("u-alice")]);
  await settle();
  assertEquals(f.events.at(-1)?.name, "変更した名前");
});

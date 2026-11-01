import { assertEquals, assertRejects } from "@std/assert";
import {
  contextChatId,
  createLinePollService,
  normalizePoll,
  type PollClient,
  pollCreateBody,
  PollError,
  quoteLongIntegers,
} from "../src/line_polls.ts";

const detail = {
  questionId: "7702851912688516885",
  title: "Test",
  questionType: "TEXT",
  voterCount: 1,
  createDate: 1793459968615,
  closeDate: 1793632800000,
  voteByMe: true,
  choiceList: [
    {
      questionId: "7702851912688516885",
      choiceId: "7702851912688516887",
      text: "Testq",
      voterCount: 1,
      voteByMe: true,
    },
    {
      questionId: "7702851912688516885",
      choiceId: "7702851912688516889",
      text: "Test",
      voterCount: 0,
      voteByMe: false,
    },
  ],
  lineProfile: { displayName: "Owner", mid: "u1" },
  editable: true,
  closeable: true,
  closed: false,
  anonymous: true,
  multiple: true,
};

function jwt(claims: Record<string, unknown>) {
  const b64 = (v: unknown) =>
    btoa(JSON.stringify(v)).replace(/\+/g, "-").replace(/\//g, "_").replace(
      /=+$/,
      "",
    );
  return `${b64({ alg: "none" })}.${b64(claims)}.sig`;
}

type Call = { url: string; init?: RequestInit };

function fakeClient(
  responses: Array<{ status?: number; body: unknown }>,
  calls: Call[],
  issued = { n: 0 },
): PollClient {
  return {
    base: {
      liff: {
        issueLiffView: () => {
          issued.n++;
          return Promise.resolve({
            accessToken: `token-${issued.n}`,
            contextToken: jwt({ groupId: "Cgroup" }),
          });
        },
        getLiffToken: () => Promise.resolve("consent-token"),
      },
      fetch: (url, init) => {
        calls.push({ url: String(url), init });
        const next = responses.shift()!;
        return Promise.resolve(
          new Response(
            typeof next.body === "string"
              ? next.body
              : JSON.stringify(next.body),
            {
              status: next.status ?? 200,
              headers: { "content-type": "application/json" },
            },
          ),
        );
      },
    },
  };
}

Deno.test("context token chat id is read without exposing other claims", () => {
  assertEquals(contextChatId(jwt({ groupId: "Cabc", userId: "u" })), "Cabc");
  assertEquals(contextChatId(jwt({ roomId: "Rabc" })), "Rabc");
  assertEquals(contextChatId("garbage"), null);
  assertEquals(contextChatId(undefined), null);
});

Deno.test("poll detail is normalised for the UI", () => {
  const poll = normalizePoll(detail);
  assertEquals(poll.id, "7702851912688516885");
  assertEquals(poll.title, "Test");
  assertEquals(poll.multiple, true);
  assertEquals(poll.anonymous, true);
  assertEquals(poll.closed, false);
  assertEquals(poll.votedByMe, true);
  assertEquals(poll.closeDate, 1793632800000);
  assertEquals(poll.ownerName, "Owner");
  assertEquals(poll.choices, [
    {
      id: "7702851912688516887",
      text: "Testq",
      voterCount: 1,
      votedByMe: true,
    },
    {
      id: "7702851912688516889",
      text: "Test",
      voterCount: 0,
      votedByMe: false,
    },
  ]);
  const empty = normalizePoll(null);
  assertEquals(empty.choices, []);
  assertEquals(empty.closeDate, null);
});

Deno.test("get sends the LIFF token and chat id like the web app", async () => {
  const calls: Call[] = [];
  const client = fakeClient(
    [{ body: { result: detail, statusCode: 200 } }],
    calls,
  );
  const service = createLinePollService(() => client);
  const poll = await service.get("cmid", "7702851912688516885");
  assertEquals(poll.title, "Test");
  assertEquals(
    calls[0].url,
    "https://w.line.me/poll/ajax/poll/question/7702851912688516885",
  );
  const headers = calls[0].init!.headers as Record<string, string>;
  assertEquals(headers["X-Liff-Token"], "Bearer token-1");
  assertEquals(headers["X-LINE-Chat-ID"], "Cgroup");
  assertEquals(calls[0].init!.method, "GET");
});

Deno.test("tokens are cached per chat until they expire", async () => {
  const calls: Call[] = [];
  const issued = { n: 0 };
  let time = 1000;
  const body = { result: detail, statusCode: 200 };
  const client = fakeClient([{ body }, { body }, { body }], calls, issued);
  const service = createLinePollService(() => client, () => time);
  await service.get("cmid", "1");
  await service.get("cmid", "1");
  assertEquals(issued.n, 1);
  time += 6 * 60 * 1000;
  await service.get("cmid", "1");
  assertEquals(issued.n, 2);
});

Deno.test("vote posts the chosen ids then returns the refreshed poll", async () => {
  const calls: Call[] = [];
  const client = fakeClient([
    { body: { result: null, statusCode: 200 } },
    { body: { result: detail, statusCode: 200 } },
  ], calls);
  const service = createLinePollService(() => client);
  const poll = await service.vote("cmid", "7702851912688516885", [
    "7702851912688516887",
  ]);
  assertEquals(poll.votedByMe, true);
  assertEquals(
    calls[0].url,
    "https://w.line.me/poll/ajax/poll/question/7702851912688516885/vote",
  );
  assertEquals(calls[0].init!.method, "POST");
  assertEquals(JSON.parse(String(calls[0].init!.body)), [
    "7702851912688516887",
  ]);
  assertEquals(calls[1].init!.method, "GET");
});

Deno.test("vote rejects empty or malformed choices without a request", async () => {
  const calls: Call[] = [];
  const client = fakeClient([], calls);
  const service = createLinePollService(() => client);
  await assertRejects(() => service.vote("cmid", "1", []), Error);
  await assertRejects(() => service.vote("cmid", "1", ["x"]), Error);
  await assertRejects(() => service.get("cmid", "../x"), Error);
  assertEquals(calls.length, 0);
});

Deno.test("service errors carry the poll status message and code", async () => {
  const calls: Call[] = [];
  const client = fakeClient([
    { body: { statusCode: 501, statusMessage: "Poll is closed" } },
  ], calls);
  const service = createLinePollService(() => client);
  const error = await assertRejects(
    () => service.vote("cmid", "1", ["2"]),
    PollError,
    "Poll is closed",
  );
  assertEquals(error.code, 501);
});

Deno.test("no client means not logged in", async () => {
  const service = createLinePollService(() => null);
  await assertRejects(() => service.get("cmid", "1"), Error, "not_logged_in");
});

Deno.test("create body follows the web app's Poll shape", () => {
  const body = pollCreateBody({
    title: "  Lunch?  ",
    choices: [" Pho ", "Bun", ""],
    multiple: true,
    anonymous: false,
    allowNewChoices: true,
    closeDate: 5_000_000,
  }, 1000);
  assertEquals(body, {
    questionType: "TEXT",
    title: "Lunch?",
    choiceList: [{ text: "Pho" }, { text: "Bun" }],
    multiple: true,
    anonymous: false,
    editable: true,
    closeDate: 5_000_000,
  });
  const open = pollCreateBody({
    title: "t",
    choices: ["a", "b"],
    multiple: false,
    anonymous: false,
    allowNewChoices: false,
  }, 0);
  assertEquals("closeDate" in open, false);
});

Deno.test("create enforces the poll web app limits locally", () => {
  const base = {
    title: "t",
    choices: ["a", "b"],
    multiple: false,
    anonymous: false,
    allowNewChoices: false,
  };
  const fails = (draft: Partial<typeof base> & { closeDate?: number }) => {
    let message = "";
    try {
      pollCreateBody({ ...base, ...draft }, 1000);
    } catch (e) {
      message = (e as Error).message;
    }
    return message;
  };
  assertEquals(fails({ title: "  " }), "poll_title_required");
  assertEquals(fails({ title: "x".repeat(71) }), "poll_title_long");
  assertEquals(fails({ choices: ["a", " "] }), "poll_choices_few");
  assertEquals(
    fails({ choices: Array.from({ length: 51 }, (_, i) => `c${i}`) }),
    "poll_choices_many",
  );
  assertEquals(fails({ choices: ["a", "x".repeat(61)] }), "poll_choice_long");
  assertEquals(fails({ choices: ["a", " a "] }), "poll_choice_duplicate");
  assertEquals(fails({ closeDate: 1500 }), "poll_close_past");
  assertEquals(fails({ title: "😀".repeat(70) }), "");
});

Deno.test("create posts the poll and returns its id", async () => {
  const calls: Call[] = [];
  const client = fakeClient([
    { body: '{"result":7702851912688516885,"statusCode":200}' },
  ], calls);
  const service = createLinePollService(() => client, () => 1000);
  const created = await service.create("cmid", {
    title: "Lunch?",
    choices: ["Pho", "Bun"],
    multiple: false,
    anonymous: true,
    allowNewChoices: false,
  });
  assertEquals(created.pollId, "7702851912688516885");
  assertEquals(
    calls[0].url,
    "https://w.line.me/poll/ajax/poll/question/create",
  );
  assertEquals(calls[0].init!.method, "POST");
  assertEquals(JSON.parse(String(calls[0].init!.body)).anonymous, true);
});

Deno.test("an invalid draft never reaches the network", async () => {
  const calls: Call[] = [];
  const client = fakeClient([], calls);
  const service = createLinePollService(() => client);
  await assertRejects(
    () =>
      service.create("cmid", {
        title: "",
        choices: ["a", "b"],
        multiple: false,
        anonymous: false,
        allowNewChoices: false,
      }),
    Error,
    "poll_title_required",
  );
  assertEquals(calls.length, 0);
});

Deno.test("long integers in responses stay exact", () => {
  assertEquals(
    quoteLongIntegers(
      '{"result":7702851912688516885,"ids":[1,7702851912688516887],"n":5}',
    ),
    '{"result":"7702851912688516885","ids":[1,"7702851912688516887"],"n":5}',
  );
  assertEquals(
    quoteLongIntegers('{"title":"id 7702851912688516885 here"}'),
    '{"title":"id 7702851912688516885 here"}',
  );
});

Deno.test("a cached token is never reused by another logged-in account", async () => {
  const calls: Call[] = [];
  const body = { result: detail, statusCode: 200 };
  const issuedA = { n: 0 };
  const issuedB = { n: 100 };
  const accountA = fakeClient([{ body }], calls, issuedA);
  const accountB = fakeClient([{ body }], calls, issuedB);
  let current = accountA;
  const service = createLinePollService(() => current);
  await service.get("shared-group", "1");
  current = accountB;
  await service.get("shared-group", "1");
  const header = (i: number) =>
    (calls[i].init!.headers as Record<string, string>)["X-Liff-Token"];
  assertEquals(header(0), "Bearer token-1");
  assertEquals(header(1), "Bearer token-101");
});

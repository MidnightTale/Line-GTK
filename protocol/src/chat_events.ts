import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { atomicWriteJson } from "./storage.ts";

type Json = Record<string, unknown>;
export type ChatOperation = {
  type?: string | number;
  revision?: unknown;
  createdTime?: unknown;
  param1?: string;
  param2?: string;
  param3?: string;
};
export type ChatEvent = {
  kind:
    | "invited"
    | "joined"
    | "left"
    | "removed"
    | "invitation_canceled"
    | "renamed"
    | "picture_changed"
    | "link_join_allowed"
    | "link_join_disabled"
    | "created"
    | "updated"
    | "unknown";
  actorMid: string;
  targetMids: string[];
  name?: string;
};
type EventMessage = {
  id: string;
  to: string;
  from: string;
  mine: false;
  text: string;
  createdTime: number;
  contentType: "CHAT_EVENT";
  chatEvent: ChatEvent;
};
const operations: Record<string, [ChatEvent["kind"], boolean]> = {};
for (
  const [kind, self, names] of [
    ["invited", false, [
      "13",
      "22",
      "124",
      "NOTIFIED_INVITE_INTO_GROUP",
      "NOTIFIED_INVITE_INTO_ROOM",
      "NOTIFIED_INVITE_INTO_CHAT",
    ]],
    ["invited", true, [
      "12",
      "21",
      "123",
      "INVITE_INTO_GROUP",
      "INVITE_INTO_ROOM",
      "INVITE_INTO_CHAT",
    ]],
    ["joined", false, [
      "17",
      "60",
      "130",
      "NOTIFIED_ACCEPT_GROUP_INVITATION",
      "NOTIFIED_JOIN_CHAT",
      "NOTIFIED_ACCEPT_CHAT_INVITATION",
    ]],
    ["joined", true, [
      "16",
      "129",
      "ACCEPT_GROUP_INVITATION",
      "ACCEPT_CHAT_INVITATION",
    ]],
    ["left", false, [
      "15",
      "24",
      "61",
      "128",
      "NOTIFIED_LEAVE_GROUP",
      "NOTIFIED_LEAVE_ROOM",
      "NOTIFIED_LEAVE_CHAT",
      "NOTIFIED_DELETE_SELF_FROM_CHAT",
    ]],
    ["left", true, [
      "14",
      "23",
      "127",
      "LEAVE_GROUP",
      "LEAVE_ROOM",
      "DELETE_SELF_FROM_CHAT",
    ]],
    ["removed", false, [
      "19",
      "133",
      "NOTIFIED_KICKOUT_FROM_GROUP",
      "NOTIFIED_DELETE_OTHER_FROM_CHAT",
    ]],
    ["removed", true, [
      "18",
      "132",
      "KICKOUT_FROM_GROUP",
      "DELETE_OTHER_FROM_CHAT",
    ]],
    ["invitation_canceled", false, [
      "32",
      "126",
      "NOTIFIED_CANCEL_INVITATION_GROUP",
      "NOTIFIED_CANCEL_CHAT_INVITATION",
    ]],
    ["invitation_canceled", true, [
      "31",
      "125",
      "CANCEL_INVITATION_GROUP",
      "CANCEL_CHAT_INVITATION",
    ]],
    ["updated", false, [
      "11",
      "122",
      "NOTIFIED_UPDATE_GROUP",
      "NOTIFIED_UPDATE_CHAT",
    ]],
    ["updated", true, ["10", "121", "UPDATE_GROUP", "UPDATE_CHAT"]],
    ["created", true, [
      "9",
      "20",
      "120",
      "CREATE_GROUP",
      "CREATE_ROOM",
      "CREATE_CHAT",
    ]],
  ] as const
) for (const name of names) operations[name] = [kind, self];

const user = (mid: unknown): mid is string =>
  typeof mid === "string" && /^u[a-zA-Z0-9_-]{1,128}$/.test(mid);
const targeted = (kind: ChatEvent["kind"]) =>
  ["invited", "removed", "invitation_canceled"].includes(kind);

const eventKinds = new Set<string>([
  "invited",
  "joined",
  "left",
  "removed",
  "invitation_canceled",
  "renamed",
  "picture_changed",
  "link_join_allowed",
  "link_join_disabled",
  "created",
  "updated",
  "unknown",
]);

function validEvent(value: unknown): value is ChatEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as ChatEvent;
  return eventKinds.has(event.kind) &&
    (event.actorMid === "" || user(event.actorMid)) &&
    Array.isArray(event.targetMids) && event.targetMids.length <= 100 &&
    event.targetMids.every(user) &&
    (event.name === undefined ||
      (typeof event.name === "string" && event.name.length <= 1000));
}

/** Decode history notices before discarding the raw LINE metadata. Unknown or
 * malformed notices remain system rows, without guessing an actor or action. */
export function chatEventOf(message: Json): ChatEvent | null {
  if (validEvent(message.chatEvent)) return message.chatEvent;
  if (
    !["18", "CHATEVENT", "CHAT_EVENT"].includes(
      String(message.contentType).toUpperCase(),
    )
  ) {
    return null;
  }
  const unknown: ChatEvent = { kind: "unknown", actorMid: "", targetMids: [] };
  const meta = message.contentMetadata as Json | null | undefined;
  if (
    typeof meta?.LOC_KEY !== "string" ||
    (meta.LOC_ARGS !== undefined && typeof meta.LOC_ARGS !== "string")
  ) return unknown;
  const args = meta.LOC_ARGS ? String(meta.LOC_ARGS).split("\x1e") : [];
  const members = (value: string | undefined): string[] => {
    const mids = value?.split("\x1f") ?? [];
    return mids.length && mids.length <= 100 && mids.every(user)
      ? [...new Set(mids)]
      : [];
  };
  const event = (
    kind: ChatEvent["kind"],
    actorMid = "",
    targetMids: string[] = [],
  ): ChatEvent => ({ kind, actorMid, targetMids });
  switch (meta.LOC_KEY) {
    case "C_MJ":
    case "C_ML": {
      const mids = members(args[0]);
      if (args.length !== 1 || !mids.length) return unknown;
      return event(meta.LOC_KEY === "C_MJ" ? "joined" : "left", mids[0], mids);
    }
    case "C_GI":
    case "C_MI":
    case "C_IC": {
      const mids = members(args[1]);
      if (args.length !== 2 || !user(args[0]) || !mids.length) return unknown;
      return event(
        meta.LOC_KEY === "C_IC" ? "invitation_canceled" : "invited",
        args[0],
        mids,
      );
    }
    case "C_MR": {
      const mids = members(args.at(-1));
      if (
        !mids.length || ![1, 2].includes(args.length) ||
        (args.length === 2 && !user(args[0]))
      ) {
        return unknown;
      }
      return event("removed", args.length === 2 ? args[0] : "", mids);
    }
    case "C_PN":
      return args.length === 2 && user(args[0]) && args[1].trim() &&
          args[1].length <= 1000
        ? { ...event("renamed", args[0]), name: args[1] }
        : unknown;
    case "C_PI":
      return args.length === 1 && user(args[0])
        ? event("picture_changed", args[0])
        : unknown;
    case "C_SN":
    case "C_SP":
      return args.length <= 1
        ? event(
          meta.LOC_KEY === "C_SN" ? "link_join_allowed" : "link_join_disabled",
          user(args[0]) ? args[0] : "",
        )
        : unknown;
    default:
      return unknown;
  }
}

export function normalizeChatEvent(message: Json): Json {
  const chatEvent = chatEventOf(message);
  return chatEvent
    ? {
      ...message,
      chatEvent,
      contentType: "CHAT_EVENT",
      text: "",
      mine: false,
      reactions: [],
      replyTo: null,
      needsMedia: false,
    }
    : message;
}

export function summarizeChatEvent(raw: Json): Json | null {
  const chatEvent = chatEventOf(raw);
  if (!chatEvent) return null;
  return normalizeChatEvent({
    id: String(raw.id ?? ""),
    to: typeof raw.to === "string" ? raw.to : "",
    from: chatEvent.actorMid,
    createdTime: Number(raw.createdTime ?? 0),
    chatEvent,
  });
}
export function chatEventMessage(
  op: ChatOperation,
  selfMid: string,
  now = Date.now(),
): EventMessage | null {
  const definition = operations[String(op.type)];
  const chatMid = op.param1 ?? "";
  if (
    !definition || !user(selfMid) || !/^[cr][a-zA-Z0-9_-]{1,128}$/.test(chatMid)
  ) return null;
  const [operationKind, own] = definition;
  const attribute = own ? op.param2 : op.param3;
  const kind = operationKind === "updated"
    ? attribute === "1"
      ? "renamed"
      : attribute === "2"
      ? "picture_changed"
      : "updated"
    : operationKind;
  const actorMid = own ? selfMid : op.param2 ?? "";
  if (!user(actorMid)) return null;
  const rawTargets = own ? op.param2 : op.param3;
  const targetMids = targeted(kind)
    ? [...new Set((rawTargets ?? "").split("\x1e").filter(user))].slice(0, 100)
    : [];
  if (targeted(kind) && !targetMids.length) {
    return null;
  }
  const revision = String(op.revision ?? "");
  const time = Number(op.createdTime);
  const createdTime = Number.isFinite(time) && time > 0
    ? (time < 1e12 ? time * 1000 : time)
    : now;
  // 履歴のメッセージIDと混ぜず、再送された同じ操作は同じIDにする。
  const identity = /^\d+$/.test(revision) && revision !== "0"
    ? revision
    : `${createdTime}:${kind}:${actorMid}:${targetMids.join(",")}`;
  return {
    id: `event:${identity}`,
    to: chatMid,
    from: actorMid,
    mine: false,
    text: "",
    createdTime,
    contentType: "CHAT_EVENT",
    chatEvent: { kind, actorMid, targetMids },
  };
}

type Runtime = {
  directory: string;
  getClient: () => Client | null;
  selfMid: () => string;
  observeProfiles: (messages: Json[]) => void;
  emit: (event: string, payload: Json) => void;
};
export function createChatEvents(runtime: Runtime) {
  let owner = "";
  let records: EventMessage[] | null = null;
  let queue = Promise.resolve();
  function reset() {
    owner = "";
    records = null;
  }
  async function load(account: string): Promise<EventMessage[]> {
    if (owner !== account) {
      owner = account;
      records = null;
    }
    if (records) return records;
    let loaded: EventMessage[] = [];
    try {
      const raw = JSON.parse(
        await Deno.readTextFile(
          join(runtime.directory, `events-${account}.json`),
        ),
      );
      if (Array.isArray(raw)) {
        loaded = raw.filter((m) =>
          m?.contentType === "CHAT_EVENT" && typeof m.id === "string" &&
          m.id.startsWith("event:") &&
          typeof m.to === "string" && /^[cr][a-zA-Z0-9_-]{1,128}$/.test(m.to) &&
          Number.isFinite(m.createdTime) && m.createdTime > 0 &&
          validEvent(m.chatEvent)
        ).slice(-5000);
      }
    } catch { /* 初回またはキャッシュが消去された場合は空の履歴から始める。 */ }
    if (runtime.selfMid() !== account) return [];
    records = loaded;
    return loaded;
  }
  function observe(message: EventMessage) {
    runtime.observeProfiles(
      [message.chatEvent.actorMid, ...message.chatEvent.targetMids].map(
        (from) => ({ from, to: message.to, mine: false }),
      ),
    );
  }
  function handleOperation(op: ChatOperation): boolean {
    const account = runtime.selfMid();
    const client = runtime.getClient();
    const message = chatEventMessage(op, account);
    if (!message || !client) return false;
    const active = () =>
      runtime.selfMid() === account && runtime.getClient() === client;
    queue = queue.then(async () => {
      if (!active()) return;
      const loaded = await load(account);
      if (
        !active() ||
        loaded.some((m) => m.id === message.id && m.to === message.to)
      ) return;
      const sameChat = loaded.filter((m) => m.to === message.to).slice(-499);
      records = [
        ...loaded.filter((m) => m.to !== message.to),
        ...sameChat,
        message,
      ].sort((a, b) => a.createdTime - b.createdTime).slice(-5000);
      observe(message);
      runtime.emit("chat_event", { accountMid: account, message });
      await atomicWriteJson(
        join(runtime.directory, `events-${account}.json`),
        records,
      );
    }).catch(() =>
      console.error("[chat-events] グループイベントを保存できませんでした。")
    );
    return true;
  }
  async function merge(chatMid: string, messages: Json[]): Promise<Json[]> {
    const account = runtime.selfMid();
    const client = runtime.getClient();
    if (!user(account) || !/^[cr]/.test(chatMid)) return messages;
    await queue;
    const loaded = await load(account);
    if (runtime.selfMid() !== account || runtime.getClient() !== client) {
      return messages;
    }
    const oldest = messages.length
      ? Math.min(...messages.map((m) => Number(m.createdTime ?? 0)))
      : 0;
    const events = loaded.filter((m) =>
      m.to === chatMid && m.createdTime >= oldest
    ).slice(-500);
    events.forEach(observe);
    const combined = new Map(messages.map((m) => [String(m.id), m]));
    const targets = (event: ChatEvent) => {
      const mids =
        ["joined", "left"].includes(event.kind) && !event.targetMids.length
          ? [event.actorMid]
          : event.targetMids;
      return mids.slice().sort().join(",");
    };
    for (const event of events) {
      // A server history notice and its live operation have different IDs.
      // Only replace an exact timestamp/action match; nearby events may be distinct.
      const inHistory = messages.some((message) => {
        const history = chatEventOf(message);
        return history && Number(message.createdTime) === event.createdTime &&
          history.kind === event.chatEvent.kind &&
          history.actorMid === event.chatEvent.actorMid &&
          targets(history) === targets(event.chatEvent);
      });
      if (!inHistory) combined.set(event.id, event);
    }
    return [...combined.values()].sort((a, b) =>
      Number(a.createdTime) - Number(b.createdTime)
    );
  }
  return { handleOperation, merge, reset, settled: () => queue };
}

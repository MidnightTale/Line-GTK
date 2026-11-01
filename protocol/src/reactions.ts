import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { atomicWriteJson } from "./storage.ts";
import { replyMessageId } from "./replies.ts";

type Json = Record<string, unknown>;

export const PREDEFINED = [
  "NICE",
  "LOVE",
  "FUN",
  "AMAZING",
  "SAD",
  "OMG",
] as const;
export type Predefined = typeof PREDEFINED[number];
export type ReactionChoice =
  | { kind: Predefined }
  | { kind: "EMOJI"; emoji: string; resourceType: number; version: number };
export type Reaction = ReactionChoice & { from: string; at: number };

/**
 * Thrift field ids of the custom (paid) LINE emoji reaction inside
 * `ReactionType`. linejs does not know this struct.
 * Defaults follow PaidReactionType in CHRLINE-Thrift;
 * unknown field ids can also be learned from received reactions.
 */
export type PaidLayout = {
  outer: number;
  product: number;
  emoji: number;
  resource: number;
  version: number;
};
export const DEFAULT_PAID_LAYOUT: PaidLayout = {
  outer: 2,
  product: 1,
  emoji: 2,
  resource: 3,
  version: 4,
};

const MAX_REACTIONS = 500;
const MAX_INDEXED = 4000;
const MAX_RECENT = 48;
const PRODUCT = /^[a-f0-9]{24}$/i;
const EMOJI_ID = /^[0-9]{1,5}$/;
const EMOJI_KEY = /^[a-f0-9]{24}_[0-9]{1,5}$/i;
const user = (mid: unknown): mid is string =>
  typeof mid === "string" && /^u[a-zA-Z0-9_-]{1,128}$/.test(mid);
const chat = (mid: unknown): mid is string =>
  typeof mid === "string" && /^[ucr][a-zA-Z0-9_-]{1,128}$/.test(mid);
const object = (value: unknown): value is Json =>
  !!value && typeof value === "object" && !Array.isArray(value);
const accountOf = (client: Client | null) =>
  String(client?.base.profile?.mid ?? "");

function predefined(value: unknown): Predefined | null {
  if (typeof value === "string" && /^[0-9]+$/.test(value)) {
    value = Number(value);
  }
  if (typeof value === "number") return PREDEFINED[value - 2] ?? null;
  return PREDEFINED.find((name) => name === value) ?? null;
}

function resourceTypeOf(value: unknown): number {
  if (value === "ANIMATION" || value === 2 || value === "2") return 2;
  return 1;
}

export function emojiVersion(value: unknown): number {
  const version = Number(value);
  return Number.isSafeInteger(version) && version > 0 ? version : 1;
}

function paid(
  productId: unknown,
  emojiId: unknown,
  resourceType: unknown,
  version?: unknown,
): ReactionChoice | null {
  const product = String(productId ?? "");
  const emoji = String(emojiId ?? "");
  if (!PRODUCT.test(product) || !EMOJI_ID.test(emoji)) return null;
  return {
    kind: "EMOJI",
    emoji: `${product}_${emoji}`,
    resourceType: resourceTypeOf(resourceType),
    version: emojiVersion(version),
  };
}

/**
 * Reads a `ReactionType` from a renamed Thrift struct, where unknown fields
 * keep their numeric ids, or from the JSON of a reaction operation.
 */
export function decodeReactionType(
  value: unknown,
): { choice: ReactionChoice; layout?: PaidLayout } | null {
  if (!object(value)) return null;
  const named = value.paidReactionType;
  if (object(named)) {
    const choice = paid(
      named.productId,
      named.emojiId,
      named.resourceType,
      named.version,
    );
    return choice ? { choice } : null;
  }
  const kind = predefined(value.predefinedReactionType ?? value["1"]);
  if (kind) return { choice: { kind } };
  for (const [fid, inner] of Object.entries(value)) {
    if (!/^[0-9]+$/.test(fid) || fid === "1" || !object(inner)) continue;
    const fields = Object.entries(inner).filter(([id]) => /^[0-9]+$/.test(id));
    const product = fields.find(([, v]) =>
      typeof v === "string" && PRODUCT.test(v)
    );
    const emoji = fields.find(([id, v]) =>
      id !== product?.[0] && typeof v === "string" && EMOJI_ID.test(v)
    );
    const resource = fields.find(([id, v]) =>
      Number(id) === DEFAULT_PAID_LAYOUT.resource && typeof v === "number"
    ) ?? fields.find(([id, v]) =>
      Number(id) !== DEFAULT_PAID_LAYOUT.version &&
      typeof v === "number" && (v === 1 || v === 2)
    );
    const version = fields.find(([id, v]) =>
      id !== resource?.[0] &&
      (typeof v === "number" || typeof v === "bigint")
    );
    if (!product || !emoji) {
      continue;
    }
    const choice = paid(product[1], emoji[1], resource?.[1], version?.[1]);
    if (!choice) {
      continue;
    }
    return {
      choice,
      layout: {
        outer: Number(fid),
        product: Number(product[0]),
        emoji: Number(emoji[0]),
        resource: Number(resource?.[0] ?? DEFAULT_PAID_LAYOUT.resource),
        version: Number(version?.[0] ?? DEFAULT_PAID_LAYOUT.version),
      },
    };
  }
  return null;
}

function time(value: unknown): number {
  try {
    const result = Number(typeof value === "bigint" ? value : value ?? 0);
    return Number.isFinite(result) && result > 0 ? result : 0;
  } catch {
    return 0;
  }
}

/** One reaction per person, oldest first. */
export function normalizeReactions(
  raw: unknown,
  onLayout?: (layout: PaidLayout) => void,
): Reaction[] {
  if (!Array.isArray(raw)) return [];
  const byUser = new Map<string, Reaction>();
  for (const item of raw.slice(0, MAX_REACTIONS * 2)) {
    if (!object(item)) continue;
    const from = item.fromUserMid ?? item.from;
    const wire = object(item.reactionType)
      ? decodeReactionType(item.reactionType)
      : null;
    if (wire?.layout) onLayout?.(wire.layout);
    const decoded = object(item.reactionType)
      ? wire?.choice
      : validChoice(item);
    if (!user(from) || !decoded) continue;
    const reaction = { ...decoded, from, at: time(item.atMillis ?? item.at) };
    const known = byUser.get(from);
    if (!known || known.at <= reaction.at) byUser.set(from, reaction);
  }
  return [...byUser.values()].sort((a, b) => a.at - b.at).slice(
    -MAX_REACTIONS,
  );
}

/** Validates a choice coming from the UI or from the local index. */
export function validChoice(value: unknown): ReactionChoice | null {
  if (!object(value)) return null;
  if (value.kind === "EMOJI") {
    const [product, emoji] = String(value.emoji ?? "").split("_");
    return EMOJI_KEY.test(String(value.emoji ?? ""))
      ? paid(product, emoji, value.resourceType, value.version)
      : null;
  }
  const kind = PREDEFINED.find((name) => name === value.kind);
  return kind ? { kind } : null;
}

/** Shapes the emoji service and profile lookup already understand. */
export function reactionCarriers(chatMid: string, messages: Json[]) {
  const emojis: { key: string }[] = [];
  const profiles: Json[] = [];
  for (const message of messages) {
    if (!Array.isArray(message.reactions)) continue;
    for (const item of message.reactions as Json[]) {
      if (item?.kind === "EMOJI" && typeof item.emoji === "string") {
        emojis.push({ key: item.emoji });
      }
      if (user(item?.from)) {
        profiles.push({ from: item.from, to: chatMid, mine: false });
      }
    }
  }
  return {
    emojis: emojis.length ? [{ emojis: emojis.slice(0, 200) }] : [],
    profiles: profiles.slice(0, 500),
  };
}

export function applyReaction(
  reactions: Reaction[],
  from: string,
  reaction: Reaction | null,
): Reaction[] {
  const others = reactions.filter((item) => item.from !== from);
  return reaction ? [...others, reaction] : others;
}

export type ReactionOperation = {
  chatMid: string;
  messageId: string;
  from: string;
  choice: ReactionChoice | null;
  layout?: PaidLayout;
};

/**
 * SEND_REACTION (139) and NOTIFIED_SEND_REACTION (140): param1 is the message
 * id, param2 is JSON `{chatMid, curr}` where `curr` is null when the reaction
 * was removed, and param3 is the person who reacted (140 only).
 */
export function reactionOperation(
  op: {
    type?: string | number;
    param1?: string;
    param2?: string;
    param3?: string;
  },
  selfMid: string,
): ReactionOperation | null {
  const type = String(op.type);
  const own = type === "139" || type === "SEND_REACTION";
  if (!own && type !== "140" && type !== "NOTIFIED_SEND_REACTION") return null;
  const messageId = replyMessageId(op.param1);
  let payload: unknown;
  try {
    payload = JSON.parse(String(op.param2 ?? ""));
  } catch {
    return null;
  }
  if (!object(payload) || !("curr" in payload)) return null;
  const chatMid = payload.chatMid;
  const from = own ? selfMid : op.param3;
  if (!messageId || !chat(chatMid) || !user(from)) return null;
  if (payload.curr === null || payload.curr === undefined) {
    return { chatMid, messageId, from, choice: null };
  }
  const decoded = decodeReactionType(payload.curr);
  return decoded ? { chatMid, messageId, from, ...decoded } : null;
}

export function paidReactArgs(
  reqSeq: number,
  messageId: string,
  choice: Extract<ReactionChoice, { kind: "EMOJI" }>,
  layout: PaidLayout,
): unknown[] {
  const [productId, emojiId] = choice.emoji.split("_");
  return [[12, 1, [
    [8, 1, reqSeq],
    [10, 2, BigInt(messageId)],
    [12, 3, [[12, layout.outer, [
      [11, layout.product, productId],
      [11, layout.emoji, emojiId],
      [8, layout.resource, choice.resourceType],
      // Required by PaidReactionType, even for static/default-version emoji:
      // https://github.com/DeachSword/CHRLINE-Thrift/blob/main/subs/TalkService/ttypes.thrift
      [10, layout.version, BigInt(choice.version)],
    ]]]],
  ]]];
}

export function reactionError(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  if (text.startsWith("reaction_")) return text;
  if (/too old/i.test(text)) return "reaction_expired";
  if (/unsent message/i.test(text)) return "reaction_unsent";
  if (
    /USER_NOT_STICON_OWNER|NOT_PREMIUM|Invalid paidReactionType|"code":\s*(121|125)\b/
      .test(text)
  ) {
    return "reaction_emoji_unavailable";
  }
  return "reaction_failed";
}

type Entry = { at: number; reaction: ReactionChoice | null };
type Indexed = { chat: string; users: Record<string, Entry> };
type Index = Record<string, Indexed>;
type Recent = { key: string; resourceType: number; version: number };
type Runtime = {
  dataDir: string;
  getClient: () => Client | null;
  emit: (event: string, payload: Json) => void;
  now?: () => number;
};

export function createReactionService(runtime: Runtime) {
  const now = runtime.now ?? Date.now;
  const loaded = new Map<string, Promise<Index>>();
  const recent = new Map<string, Recent[]>();
  const recentLoaded = new Map<string, Promise<Recent[]>>();
  let layout: PaidLayout = DEFAULT_PAID_LAYOUT;
  let writes = Promise.resolve();
  let remembering = Promise.resolve();
  let recording = Promise.resolve();
  const indexPath = (account: string) =>
    join(runtime.dataDir, `reactions-${account}.json`);
  const recentPath = (account: string) =>
    join(runtime.dataDir, `reaction-emojis-${account}.json`);

  function load(account: string): Promise<Index> {
    if (!user(account)) return Promise.reject(new Error("not_logged_in"));
    let result = loaded.get(account);
    if (!result) {
      result = (async () => {
        try {
          const raw = JSON.parse(await Deno.readTextFile(indexPath(account)));
          if (raw?.version !== 1 || !object(raw.messages)) return {};
          const index: Index = {};
          for (const [id, value] of Object.entries(raw.messages)) {
            if (!replyMessageId(id) || !object(value) || !chat(value.chat)) {
              continue;
            }
            const users: Record<string, Entry> = {};
            for (
              const [mid, entry] of Object.entries(
                object(value.users) ? value.users : {},
              )
            ) {
              if (!user(mid) || !object(entry) || !time(entry.at)) continue;
              const reaction = entry.reaction === null
                ? null
                : validChoice(entry.reaction);
              if (entry.reaction !== null && !reaction) continue;
              users[mid] = { at: time(entry.at), reaction };
            }
            index[id] = { chat: value.chat, users };
          }
          return index;
        } catch (error) {
          if (!(error instanceof Deno.errors.NotFound)) {
            console.error("[reactions] index unreadable, starting empty");
          }
          return {};
        }
      })();
      loaded.set(account, result);
    }
    return result;
  }

  function save(path: string, value: unknown) {
    writes = writes.then(() => atomicWriteJson(path, value)).catch((error) =>
      console.error("[reactions cache]", error)
    );
    return writes;
  }

  function prune(index: Index) {
    const ids = Object.keys(index);
    if (ids.length <= MAX_INDEXED) return;
    const newest = (id: string) =>
      Math.max(0, ...Object.values(index[id].users).map((entry) => entry.at));
    ids.sort((a, b) => newest(a) - newest(b));
    for (const id of ids.slice(0, ids.length - MAX_INDEXED)) delete index[id];
  }

  async function record(
    account: string,
    chatMid: string,
    messageId: string,
    from: string,
    choice: ReactionChoice | null,
    at: number,
  ) {
    const index = await load(account);
    // メッセージIDはトークをまたいで一意。1対1の chatMid は相手側か自分側か
    // 操作によって異なりうるため、IDだけで照合する。
    const entry = index[messageId] ??
      (index[messageId] = { chat: chatMid, users: {} });
    entry.users[from] = { at, reaction: choice };
    prune(index);
    const snapshot = { version: 1, messages: { ...index } };
    if (choice?.kind === "EMOJI") remember(account, [choice]);
    if (accountOf(runtime.getClient()) === account) {
      runtime.emit("message_reaction", {
        accountMid: account,
        chatMid,
        messageId,
        from,
        reaction: choice ? { ...choice, from, at } : null,
      });
    }
    await save(indexPath(account), snapshot);
  }

  function loadRecent(account: string): Promise<Recent[]> {
    let result = recentLoaded.get(account);
    if (!result) {
      result = (async () => {
        try {
          const raw = JSON.parse(await Deno.readTextFile(recentPath(account)));
          const items = Array.isArray(raw?.emojis) ? raw.emojis : [];
          return items.filter((item: Json) =>
            object(item) && EMOJI_KEY.test(String(item.key))
          ).slice(0, MAX_RECENT).map((item: Json) => ({
            key: String(item.key),
            resourceType: resourceTypeOf(item.resourceType),
            version: emojiVersion(item.version),
          }));
        } catch {
          return [];
        }
      })();
      recentLoaded.set(account, result);
    }
    return result;
  }

  function remember(
    account: string,
    choices: { emoji: string; resourceType: number; version: number }[],
  ) {
    if (!user(account) || !choices.length) return;
    remembering = remembering.then(() => loadRecent(account)).then((stored) => {
      const list = recent.get(account) ?? stored;
      let changed = false;
      for (const { emoji, resourceType, version } of choices) {
        if (!EMOJI_KEY.test(emoji)) continue;
        if (
          list[0]?.key === emoji && list[0].resourceType >= resourceType &&
          list[0].version >= version
        ) {
          continue;
        }
        const known = list.find((item) => item.key === emoji);
        const next = {
          key: emoji,
          resourceType: Math.max(resourceType, known?.resourceType ?? 1),
          version: Math.max(version, known?.version ?? 1),
        };
        list.splice(
          0,
          list.length,
          next,
          ...list.filter((i) => i.key !== emoji),
        );
        list.length = Math.min(list.length, MAX_RECENT);
        changed = true;
      }
      recent.set(account, list);
      if (changed) {
        void save(recentPath(account), { version: 1, emojis: [...list] });
      }
    }).catch((error) => console.error("[reaction emojis]", error));
  }

  /** Remembers LINE emoji seen in messages and reactions for the picker. */
  function observe(messages: Json[]) {
    const account = accountOf(runtime.getClient());
    const seen: { emoji: string; resourceType: number; version: number }[] = [];
    for (const message of messages) {
      if (Array.isArray(message.emojis)) {
        for (const item of message.emojis.slice(0, 100)) {
          if (object(item) && typeof item.key === "string") {
            seen.push({ emoji: item.key, resourceType: 1, version: 1 });
          }
        }
      }
      if (Array.isArray(message.reactions)) {
        for (const item of message.reactions as Json[]) {
          if (item?.kind === "EMOJI" && typeof item.emoji === "string") {
            seen.push({
              emoji: item.emoji,
              resourceType: resourceTypeOf(item.resourceType),
              version: emojiVersion(item.version),
            });
          }
        }
      }
    }
    remember(account, seen);
  }

  async function recentEmojis(): Promise<Recent[]> {
    const account = accountOf(runtime.getClient());
    if (!user(account)) return [];
    await remembering;
    const stored = await loadRecent(account);
    return [...(recent.get(account) ?? stored)];
  }

  /** Applies reactions learned from operations after a message was fetched. */
  async function merge(_chatMid: string, messages: Json[]): Promise<Json[]> {
    const account = accountOf(runtime.getClient());
    if (!user(account)) return messages;
    let index: Index;
    try {
      await recording;
      index = await load(account);
    } catch {
      return messages;
    }
    if (accountOf(runtime.getClient()) !== account) return messages;
    return messages.map((message) => {
      const entry = index[String(message.id)];
      const base = normalizeReactions(message.reactions);
      if (!entry) {
        return base.length || "reactions" in message
          ? { ...message, reactions: base }
          : message;
      }
      const fetched = time(message.reactionsAt);
      let reactions = base;
      for (const [from, { at, reaction }] of Object.entries(entry.users)) {
        if (at <= fetched) continue;
        reactions = applyReaction(
          reactions,
          from,
          reaction ? { ...reaction, from, at } : null,
        );
      }
      return { ...message, reactions };
    });
  }

  function handleOperation(
    op: Parameters<typeof reactionOperation>[0],
  ): boolean {
    const account = accountOf(runtime.getClient());
    const target = reactionOperation(op, account);
    const type = String(op.type);
    const matched = ["139", "140", "SEND_REACTION", "NOTIFIED_SEND_REACTION"]
      .includes(type);
    if (!target || !user(account)) return matched;
    if (target.choice?.kind === "EMOJI") {
      // Diagnostic: compare the official client's wire shape with paidReactArgs.
      console.error(
        `[reactions] received custom emoji op=${type} raw=${
          String(op.param2).slice(0, 400)
        } layout=${JSON.stringify(target.layout ?? null)}`,
      );
    }
    if (target.layout) layout = target.layout;
    const at = now();
    recording = recording.then(() =>
      record(
        account,
        target.chatMid,
        target.messageId,
        target.from,
        target.choice,
        at,
      )
    ).catch((error) => console.error("[reaction operation]", error));
    return true;
  }

  async function send(
    chatMid: string,
    messageIdValue: unknown,
    choice: ReactionChoice | null,
  ): Promise<Json> {
    const client = runtime.getClient();
    const account = accountOf(client);
    const messageId = replyMessageId(messageIdValue);
    if (!client || !user(account) || !chat(chatMid) || !messageId) {
      throw new Error("reaction_invalid_message");
    }
    const sequence = client.base.getReqseq;
    const reqSeq = typeof sequence === "function"
      ? await sequence.call(client.base)
      : await sequence;
    if (accountOf(runtime.getClient()) !== account) {
      throw new Error("reaction_session_changed");
    }
    try {
      if (!choice) {
        await client.base.talk.cancelReaction({
          cancelReactionRequest: { reqSeq, messageId: BigInt(messageId) },
        });
      } else if (choice.kind === "EMOJI") {
        const talk = client.base.talk;
        await client.base.request.request(
          paidReactArgs(reqSeq, messageId, choice, layout) as never,
          "react",
          talk.protocolType,
          true,
          talk.requestPath,
        );
      } else {
        await client.base.talk.react({
          id: BigInt(messageId),
          reaction: choice.kind,
          reqSeq,
        });
      }
    } catch (error) {
      console.error(
        `[reactions] send failed kind=${choice?.kind ?? "cancel"} layout=${
          JSON.stringify(layout)
        }`,
        error,
      );
      throw new Error(reactionError(error));
    }
    await record(account, chatMid, messageId, account, choice, now());
    return { reacted: !!choice };
  }

  function react(chatMid: string, messageId: unknown, value: unknown) {
    const choice = validChoice(value);
    if (!choice) return Promise.reject(new Error("reaction_invalid_choice"));
    return send(chatMid, messageId, choice);
  }

  function cancel(chatMid: string, messageId: unknown) {
    return send(chatMid, messageId, null);
  }

  async function flush() {
    await recording;
    await Promise.all([...loaded.values(), ...recentLoaded.values()]);
    await remembering;
    await writes;
  }

  return {
    merge,
    observe,
    handleOperation,
    react,
    cancel,
    recentEmojis,
    flush,
    layout: () => layout,
    learnLayout: (next: PaidLayout) => {
      layout = next;
    },
  };
}

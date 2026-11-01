// Native access to LINE's poll service (the REST API behind the poll LIFF app).
// The LIFF token is issued through the account's own LINE session; it is never
// logged or returned to the UI.

export const POLL_LIFF_ID = "1477715170-Pl2JnXpR";
const POLL_ORIGIN = "https://w.line.me";
const TOKEN_TTL_MS = 5 * 60 * 1000;

export type PollClient = {
  base: {
    liff: {
      issueLiffView(
        options: { chatMid?: string; liffId: string; lang?: string },
      ): Promise<{ accessToken: string; contextToken?: string }>;
      getLiffToken(
        options: { chatMid?: string; liffId: string; lang?: string },
      ): Promise<string>;
    };
    fetch(url: string, init?: RequestInit): Promise<Response>;
  };
};

export type PollChoice = {
  id: string;
  text: string;
  voterCount: number;
  votedByMe: boolean;
};

export type PollView = {
  id: string;
  type: string;
  title: string;
  voterCount: number;
  closed: boolean;
  closeDate: number | null;
  votedByMe: boolean;
  multiple: boolean;
  anonymous: boolean;
  allowNewChoices: boolean;
  ownerName: string | null;
  closeable: boolean;
  choices: PollChoice[];
};

export type PollDraft = {
  title: string;
  choices: string[];
  multiple: boolean;
  anonymous: boolean;
  allowNewChoices: boolean;
  /** Epoch milliseconds; omitted for a poll that never closes. */
  closeDate?: number | null;
};

export const POLL_LIMITS = {
  minChoices: 2,
  maxChoices: 50,
  maxTitle: 70,
  maxChoice: 60,
  minCloseLeadMs: 60_000,
};

export class PollError extends Error {
  constructor(message: string, readonly code: number) {
    super(message);
  }
}

/** Chat-related claims of a LIFF context token (never the token itself). */
export function contextChatId(contextToken: string | undefined): string | null {
  if (!contextToken) return null;
  try {
    const part = contextToken.split(".")[1] ?? "";
    const claims = JSON.parse(
      atob(part.replace(/-/g, "+").replace(/_/g, "/")),
    ) as Record<string, unknown>;
    for (const key of ["groupId", "roomId", "utouId", "squareChatId"]) {
      if (typeof claims[key] === "string" && claims[key]) {
        return claims[key] as string;
      }
    }
  } catch { /* fall back to the chat mid */ }
  return null;
}

/**
 * Poll ids are 19-digit integers; JSON.parse would round them. Quote any bare
 * integer of 16+ digits so ids stay exact strings.
 */
export function quoteLongIntegers(raw: string): string {
  return raw.replace(
    /([:[,]\s*)(-?\d{16,})(?=\s*[,}\]])/g,
    '$1"$2"',
  );
}

const text = (value: unknown): string => typeof value === "string" ? value : "";
const count = (value: unknown): number => {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
};

export function normalizePoll(raw: unknown): PollView {
  const o = (raw && typeof raw === "object" ? raw : {}) as Record<
    string,
    unknown
  >;
  const profile =
    (o.lineProfile && typeof o.lineProfile === "object"
      ? o.lineProfile
      : {}) as Record<string, unknown>;
  const list = Array.isArray(o.choiceList) ? o.choiceList : [];
  const closeDate = Number(o.closeDate);
  return {
    id: text(o.questionId),
    type: text(o.questionType) || "TEXT",
    title: text(o.title),
    voterCount: count(o.voterCount),
    closed: o.closed === true,
    closeDate: Number.isFinite(closeDate) && closeDate > 0 ? closeDate : null,
    votedByMe: o.voteByMe === true,
    multiple: o.multiple === true,
    anonymous: o.anonymous === true,
    allowNewChoices: o.editable === true,
    ownerName: text(profile.displayName) || null,
    closeable: o.closeable === true,
    choices: list.map((c) => {
      const choice = (c && typeof c === "object" ? c : {}) as Record<
        string,
        unknown
      >;
      return {
        id: text(choice.choiceId),
        text: text(choice.text),
        voterCount: count(choice.voterCount),
        votedByMe: choice.voteByMe === true,
      };
    }).filter((c) => c.id),
  };
}

const length = (value: string) => [...value].length;

/** Same limits the poll web app enforces; throws before any request is made. */
export function pollCreateBody(draft: PollDraft, now: number) {
  const title = draft.title.trim();
  if (!title) throw new Error("poll_title_required");
  if (length(title) > POLL_LIMITS.maxTitle) throw new Error("poll_title_long");
  const choices = draft.choices.map((c) => c.trim()).filter(Boolean);
  if (choices.length < POLL_LIMITS.minChoices) {
    throw new Error("poll_choices_few");
  }
  if (choices.length > POLL_LIMITS.maxChoices) {
    throw new Error("poll_choices_many");
  }
  if (choices.some((c) => length(c) > POLL_LIMITS.maxChoice)) {
    throw new Error("poll_choice_long");
  }
  if (new Set(choices).size !== choices.length) {
    throw new Error("poll_choice_duplicate");
  }
  const body: Record<string, unknown> = {
    questionType: "TEXT",
    title,
    choiceList: choices.map((text) => ({ text })),
    multiple: draft.multiple,
    anonymous: draft.anonymous,
    editable: draft.allowNewChoices,
  };
  if (draft.closeDate != null) {
    if (
      !Number.isFinite(draft.closeDate) ||
      draft.closeDate < now + POLL_LIMITS.minCloseLeadMs
    ) {
      throw new Error("poll_close_past");
    }
    body.closeDate = Math.floor(draft.closeDate);
  }
  return body;
}

export function createLinePollService(
  getClient: () => PollClient | null,
  now: () => number = Date.now,
) {
  // Tokens belong to one logged-in client. Two accounts can share a group mid,
  // so the cache is keyed by client identity and never crosses accounts.
  const tokens = new Map<
    string,
    {
      client: PollClient;
      accessToken: string;
      chatId: string;
      expires: number;
    }
  >();

  async function session(chatMid: string) {
    const client = getClient();
    if (!client) throw new Error("not_logged_in");
    const cached = tokens.get(chatMid);
    if (cached && cached.client === client && cached.expires > now()) {
      return cached;
    }
    let accessToken: string;
    let chatId = chatMid;
    try {
      const issued = await client.base.liff.issueLiffView({
        chatMid,
        liffId: POLL_LIFF_ID,
        lang: "en_US",
      });
      accessToken = issued.accessToken;
      chatId = contextChatId(issued.contextToken) ?? chatMid;
    } catch {
      // Handles the one-time LIFF consent flow.
      accessToken = await client.base.liff.getLiffToken({
        chatMid,
        liffId: POLL_LIFF_ID,
        lang: "en_US",
      });
    }
    const entry = {
      client,
      accessToken,
      chatId,
      expires: now() + TOKEN_TTL_MS,
    };
    tokens.set(chatMid, entry);
    return entry;
  }

  async function call(
    chatMid: string,
    path: string,
    init: { method?: string; body?: unknown } = {},
  ): Promise<unknown> {
    const client = getClient();
    if (!client) throw new Error("not_logged_in");
    const auth = await session(chatMid);
    const response = await client.base.fetch(`${POLL_ORIGIN}/poll${path}`, {
      method: init.method ?? "GET",
      headers: {
        Accept: "application/json, text/plain, */*",
        "Content-Type": "application/json",
        "X-Liff-Token": `Bearer ${auth.accessToken}`,
        "X-LINE-Chat-ID": auth.chatId,
        "X-LINE-Accept-Language": "en",
        Origin: POLL_ORIGIN,
        Referer: `${POLL_ORIGIN}/poll/liff/`,
      },
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    });
    if (response.status === 401 || response.status === 403) {
      tokens.delete(chatMid);
    }
    let body: Record<string, unknown>;
    try {
      body = JSON.parse(quoteLongIntegers(await response.text())) as Record<
        string,
        unknown
      >;
    } catch {
      throw new PollError(`poll_http_${response.status}`, response.status);
    }
    const code = Number(body.statusCode ?? 200);
    if (!response.ok || (code !== 200 && code !== 0)) {
      if (code === 401 || code === 500 || code === 206) tokens.delete(chatMid);
      throw new PollError(
        text(body.statusMessage).trim() || `poll_error_${code}`,
        code,
      );
    }
    return body.result;
  }

  const pollPath = (pollId: string) => {
    if (!/^\d+$/.test(pollId)) throw new Error("invalid_poll_id");
    return `/ajax/poll/question/${pollId}`;
  };

  async function get(chatMid: string, pollId: string): Promise<PollView> {
    return normalizePoll(await call(chatMid, pollPath(pollId)));
  }

  async function vote(
    chatMid: string,
    pollId: string,
    choiceIds: string[],
  ): Promise<PollView> {
    if (!choiceIds.length || choiceIds.some((id) => !/^\d+$/.test(id))) {
      throw new Error("invalid_choice");
    }
    await call(chatMid, `${pollPath(pollId)}/vote`, {
      method: "POST",
      body: choiceIds,
    });
    return await get(chatMid, pollId);
  }

  /** Creates the poll; LINE's poll service posts the card into the chat. */
  async function create(
    chatMid: string,
    draft: PollDraft,
  ): Promise<{ pollId: string }> {
    const body = pollCreateBody(draft, now());
    const result = await call(chatMid, "/ajax/poll/question/create", {
      method: "POST",
      body,
    });
    const id = typeof result === "object" && result !== null
      ? text((result as Record<string, unknown>).questionId)
      : String(result ?? "");
    if (!/^\d+$/.test(id)) throw new PollError("poll_create_no_id", 0);
    return { pollId: id };
  }

  return { get, vote, create, call };
}

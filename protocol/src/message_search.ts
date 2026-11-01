type Json = Record<string, unknown>;

/** Literal, Unicode-normalized matching; never interpret a query as a regex. */
function folded(text: string): string {
  return text.normalize("NFKC").toLowerCase();
}

export function searchable(message: Json): boolean {
  return /^\d+$/.test(String(message.id ?? "")) &&
    !["UNSENT", "CHAT_EVENT", "CHATEVENT"].includes(
      String(message.contentType).toUpperCase(),
    ) && !message.decryptionFailed &&
    !message.chatEvent;
}

/** Stable newest-first ordering, including IDs beyond JavaScript's safe integers. */
export function searchMessages(messages: Json[], query: string, offset = 0) {
  const needle = folded(query.trim());
  const matches = needle
    ? messages.filter((message) =>
      searchable(message) &&
      [message.text, message.fileName].some((value) =>
        typeof value === "string" && folded(value).includes(needle)
      )
    ).sort((a, b) => {
      const time = (Number(b.createdTime) || 0) -
        (Number(a.createdTime) || 0);
      if (time) return time;
      const aa = BigInt(String(a.id)), bb = BigInt(String(b.id));
      return aa < bb ? 1 : aa > bb ? -1 : 0;
    })
    : [];
  const start = Number.isSafeInteger(offset) && offset >= 0 ? offset : 0;
  return {
    messages: matches.slice(start, start + 50),
    total: matches.length,
    offset: start,
    hasMore: start + 50 < matches.length,
  };
}

/** A bounded saved-history window around the selected result. */
export function searchContext(messages: Json[], target: string): Json[] {
  const sorted = messages.slice().sort((a, b) => {
    const time = (Number(a.createdTime) || 0) - (Number(b.createdTime) || 0);
    if (time) return time;
    const aa = String(a.id), bb = String(b.id);
    return aa.length - bb.length || aa.localeCompare(bb);
  });
  const index = sorted.findIndex((message) =>
    String(message.id) === target && searchable(message)
  );
  if (index < 0) throw new Error("message_search_unavailable");
  const start = Math.max(0, index - 25);
  return sorted.slice(start, start + 50);
}

/** Collect account-owned history without letting an async read cross sessions. */
export async function loadSearchHistory(runtime: {
  session: () => { accountMid: string; owner: object } | null;
  disk: () => Promise<{ accountMid: unknown; messages: Json[] }>;
  memory: () => Json[];
  mask: (messages: Json[]) => Promise<Json[]>;
}): Promise<Json[]> {
  const session = runtime.session();
  if (!session) throw new Error("not_logged_in");
  const check = () => {
    if (runtime.session()?.owner !== session.owner) {
      throw new Error("message_search_cancelled");
    }
  };
  let disk: Json[] = [];
  try {
    const stored = await runtime.disk();
    if (stored.accountMid === session.accountMid) disk = stored.messages;
  } catch {
    /* Missing or unreadable saved history still permits memory search. */
  }
  check();
  const merged = new Map<string, Json>();
  for (const message of [...disk, ...runtime.memory()]) {
    merged.set(String(message.id), message);
  }
  const messages = await runtime.mask([...merged.values()]);
  check();
  return messages;
}

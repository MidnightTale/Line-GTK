import type { Client } from "@evex/linejs";

// UI offsets count Unicode scalar values; LINE offsets count UTF-16 code units.
export type Mention = {
  start: number;
  end: number;
  mid: string;
  all?: boolean;
};
const MID = /^u[a-zA-Z0-9_-]+$/;

export function lineMentions(
  text: string,
  metadata: Record<string, unknown>,
): Mention[] {
  try {
    if (
      typeof metadata.MENTION !== "string" || metadata.MENTION.length > 100_000
    ) return [];
    const entries = JSON.parse(metadata.MENTION)?.MENTIONEES;
    if (!Array.isArray(entries)) return [];
    const result: Mention[] = [];
    let previous = 0;
    const boundary = (n: number) =>
      n === text.length ||
      !(text.charCodeAt(n) >= 0xdc00 && text.charCodeAt(n) <= 0xdfff);
    for (
      const entry of entries.slice(0, 100).sort((a, b) =>
        Number(a?.S) - Number(b?.S)
      )
    ) {
      if (!entry || typeof entry !== "object") continue;
      if (
        !["string", "number"].includes(typeof entry.S) ||
        !["string", "number"].includes(typeof entry.E)
      ) continue;
      if (entry.S === "" || entry.E === "") continue;
      const start = Number(entry.S), end = Number(entry.E);
      const all = entry.A === "1";
      if (
        !Number.isSafeInteger(start) || !Number.isSafeInteger(end) ||
        start < previous || end <= start || end > text.length ||
        !boundary(start) || !boundary(end) ||
        (!all && (typeof entry.M !== "string" || !MID.test(entry.M)))
      ) continue;
      result.push({
        start: [...text.slice(0, start)].length,
        end: [...text.slice(0, end)].length,
        mid: all ? "" : entry.M,
        ...(all ? { all: true } : {}),
      });
      previous = end;
    }
    return result;
  } catch {
    return [];
  }
}

export function mentionMetadata(
  text: string,
  input: unknown,
): { mentions: Mention[]; contentMetadata: Record<string, string> } {
  if (input == null) return { mentions: [], contentMetadata: {} };
  if (!Array.isArray(input) || input.length > 100) {
    throw new Error("invalid_mentions");
  }
  const chars = [...text];
  let previous = 0;
  const mentions: Mention[] = input.map((entry) => {
    if (
      !entry || !Number.isSafeInteger(entry.start) ||
      !Number.isSafeInteger(entry.end) || entry.start < previous ||
      entry.end <= entry.start || entry.end > chars.length ||
      (entry.all !== undefined && typeof entry.all !== "boolean") ||
      (entry.all === true
        ? entry.mid !== ""
        : typeof entry.mid !== "string" || !MID.test(entry.mid))
    ) throw new Error("invalid_mentions");
    previous = entry.end;
    return {
      start: entry.start,
      end: entry.end,
      mid: entry.mid,
      ...(entry.all === true ? { all: true } : {}),
    };
  });
  return {
    mentions,
    contentMetadata: mentions.length
      ? {
        MENTION: JSON.stringify({
          MENTIONEES: mentions.map((m) => ({
            S: String(chars.slice(0, m.start).join("").length),
            E: String(chars.slice(0, m.end).join("").length),
            ...(m.all ? { A: "1" } : { M: m.mid }),
          })),
        }),
      }
      : {},
  };
}

export async function fetchMentionMembers(client: Client, chatMid: string) {
  if (!/^c[a-zA-Z0-9_-]+$/.test(chatMid)) throw new Error("invalid_group");
  const response = await client.base.talk.getChats({
    chatMids: [chatMid],
    withMembers: true,
    withInvitees: false,
  });
  const chat = response.chats?.find((chat) => chat.chatMid === chatMid);
  if (!chat) throw new Error("group_unavailable");
  const mids = Object.keys(chat.extra?.groupExtra?.memberMids ?? {}).filter((
    mid,
  ) => MID.test(mid));
  const members = new Map<
    string,
    { mid: string; name: string; picturePath?: string }
  >(mids.map((mid) => [mid, { mid, name: mid }]));
  for (let index = 0; index < mids.length; index += 100) {
    const contacts = await client.base.talk.getContacts({
      mids: mids.slice(index, index + 100),
    });
    for (const contact of contacts) {
      if (members.has(contact.mid)) {
        members.set(contact.mid, {
          mid: contact.mid,
          name: contact.displayName?.trim() || contact.mid,
          ...(contact.picturePath ? { picturePath: contact.picturePath } : {}),
        });
      }
    }
  }
  return [...members.values()].sort((a, b) => a.name.localeCompare(b.name));
}

import type { Client } from "@evex/linejs";
import { chatEventOf } from "./chat_events.ts";

type Json = Record<string, unknown>;
type Runtime = {
  getClient: () => Client | null;
  selfMid: () => string;
  knownName: (mid: string) => string | undefined;
  emit: (mid: string, name: string) => void;
  now?: () => number;
};
type Profile = { name: string | null; until: number };

function userMid(value: unknown): value is string {
  return typeof value === "string" && /^u[^\s]{1,128}$/.test(value);
}

function nameOf(value: unknown, mid: string): string | null {
  if (typeof value !== "string") return null;
  const name = value.trim();
  return name && name !== mid ? name : null;
}

export function createMessageSenders(runtime: Runtime) {
  const now = runtime.now ?? Date.now;
  let owner: Client | null = null;
  let ownerMid = "";
  let epoch = 0;
  let active = 0;
  let scheduled = false;
  const profiles = new Map<string, Profile>();
  const queue = new Map<string, boolean>();
  const pending = new Map<string, number>();

  function sync() {
    const client = runtime.getClient();
    const mid = runtime.selfMid();
    if (client !== owner || mid !== ownerMid) {
      owner = client;
      ownerMid = mid;
      epoch++;
      profiles.clear();
      queue.clear();
      pending.clear();
    }
    return client && mid ? client : null;
  }

  function publish(mid: string, name: string | null) {
    profiles.delete(mid);
    profiles.set(mid, { name, until: now() + (name ? 600_000 : 60_000) });
    while (profiles.size > 5000) profiles.delete(profiles.keys().next().value!);
    if (name) runtime.emit(mid, name);
  }

  function schedule() {
    if (scheduled) return;
    scheduled = true;
    queueMicrotask(() => {
      scheduled = false;
      pump();
    });
  }

  function pump() {
    const client = sync();
    if (!client) return;
    while (active < 2 && queue.size) {
      const entries = [...queue];
      const single = entries[0][1];
      const mids = entries.filter(([, one]) => one === single)
        .slice(0, single ? 1 : 20).map(([mid]) => mid);
      const generation = epoch;
      const sessionMid = ownerMid;
      for (const mid of mids) {
        queue.delete(mid);
        pending.set(mid, generation);
      }
      active++;
      const current = () =>
        epoch === generation && runtime.getClient() === client &&
        runtime.selfMid() === sessionMid;
      void (async () => {
        try {
          const contacts = single
            ? [await client.base.talk.getContact({ mid: mids[0] })]
            : await client.base.talk.getContacts({ mids });
          if (!current()) return;
          const found = new Map(
            contacts.map((contact) => [contact.mid, contact]),
          );
          for (const mid of mids) {
            const contact = found.get(mid);
            if (!contact && !single) {
              queue.set(mid, true);
              continue;
            }
            publish(
              mid,
              nameOf(contact?.displayNameOverridden, mid) ||
                nameOf(contact?.displayName, mid),
            );
          }
        } catch {
          if (!current()) return;
          for (const mid of mids) {
            // 1人の取得失敗で同じグループのほかの送信者まで未表示にしない。
            if (!single) queue.set(mid, true);
            else publish(mid, null);
          }
        } finally {
          for (const mid of mids) {
            if (pending.get(mid) === generation) pending.delete(mid);
          }
          active--;
          schedule();
        }
      })();
    }
  }

  function observe(messages: Json[]) {
    if (!sync()) return;
    const mids = new Set<string>();
    for (const message of messages) {
      if (typeof message.to !== "string" || !/^[cr]/.test(message.to)) continue;
      if (!message.mine && userMid(message.from)) mids.add(message.from);
      const event = chatEventOf(message);
      if (event) {
        for (const mid of [event.actorMid, ...event.targetMids]) {
          if (userMid(mid)) mids.add(mid);
        }
      }
      const reply = message.replyTo as Json | null | undefined;
      if (reply && !reply.mine && userMid(reply.from)) mids.add(reply.from);
    }
    for (const mid of mids) {
      if (mid === ownerMid) continue;
      const cached = profiles.get(mid);
      if (cached && cached.until > now()) {
        if (cached.name) runtime.emit(mid, cached.name);
        continue;
      }
      const known = !cached ? nameOf(runtime.knownName(mid), mid) : null;
      if (known) {
        publish(mid, known);
        continue;
      }
      if (!pending.has(mid)) queue.set(mid, false);
    }
    // 本文の返却・新着通知は名前の通信完了を待たない。
    schedule();
  }

  function refresh(mid: string) {
    if (!sync() || !userMid(mid) || !profiles.has(mid)) return;
    profiles.delete(mid);
    if (!pending.has(mid)) queue.set(mid, false);
    schedule();
  }

  return { observe, refresh };
}

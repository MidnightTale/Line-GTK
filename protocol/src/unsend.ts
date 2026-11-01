import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { atomicWriteJson } from "./storage.ts";
import { replyMessageId } from "./replies.ts";

type Json = Record<string, unknown>;
type Index = Record<string, string>;
type Runtime = {
  getClient: () => Client | null;
  dataDir: string;
  findMessage: (
    chatMid: string,
    id: string,
    createdTime: number,
  ) => Promise<Json | undefined>;
  changed: (chatMid: string, id: string) => Promise<void>;
  emit: (event: string, payload: Json) => void;
};
const chat = (mid: string) => /^[ucr][a-zA-Z0-9_-]{1,128}$/.test(mid);
const accountOf = (client: Client | null) =>
  String(client?.base.profile?.mid ?? "");

export function unsendOperation(
  op: { type?: string | number; param1?: string; param2?: string },
) {
  if (
    !["64", "65", "DESTROY_MESSAGE", "NOTIFIED_DESTROY_MESSAGE"].includes(
      String(op.type),
    )
  ) return null;
  const chatMid = String(op.param1 ?? "");
  const id = replyMessageId(op.param2);
  return chat(chatMid) && id ? { chatMid, id } : null;
}

export function unsentMessage(message: Json): Json {
  return {
    id: message.id,
    from: message.from,
    to: message.to,
    mine: message.mine,
    createdTime: message.createdTime,
    contentType: "UNSENT",
    text: "",
  };
}

export function createUnsendService(runtime: Runtime) {
  const loaded = new Map<string, Promise<Index>>();
  const pending = new Map<string, Promise<Json>>();
  let writes = Promise.resolve();
  const path = (account: string) =>
    join(runtime.dataDir, `unsent-${account}.json`);
  function load(account: string): Promise<Index> {
    if (!/^u[a-zA-Z0-9_-]{1,128}$/.test(account)) {
      return Promise.reject(new Error("not_logged_in"));
    }
    let result = loaded.get(account);
    if (!result) {
      result = (async () => {
        try {
          const raw = JSON.parse(await Deno.readTextFile(path(account)));
          if (
            raw.version !== 1 || !raw.messages ||
            typeof raw.messages !== "object" || Array.isArray(raw.messages)
          ) throw new Error("unsend_invalid_index");
          const entries = Object.entries(raw.messages);
          if (
            entries.some(([id, mid]) =>
              !replyMessageId(id) || typeof mid !== "string" || !chat(mid)
            )
          ) throw new Error("unsend_invalid_index");
          return Object.fromEntries(entries) as Index;
        } catch (error) {
          if (error instanceof Deno.errors.NotFound) return {};
          loaded.delete(account);
          throw error;
        }
      })();
      loaded.set(account, result);
    }
    return result;
  }

  async function record(account: string, chatMid: string, id: string) {
    const index = await load(account);
    if (index[id] === chatMid) return;
    index[id] = chatMid;
    // 元の本文を記録せず、取消済みのIDだけをアカウント別に保持する。
    const snapshot = { version: 1, messages: { ...index } };
    writes = writes.then(() => atomicWriteJson(path(account), snapshot)).catch((
      error,
    ) => console.error("[unsend cache]", error));
    if (accountOf(runtime.getClient()) === account) {
      runtime.emit("message_unsent", {
        accountMid: account,
        chatMid,
        messageId: id,
      });
      await runtime.changed(chatMid, id).catch((error) =>
        console.error("[unsend preview]", error)
      );
    }
    await writes;
  }

  async function mask(chatMid: string, messages: Json[]): Promise<Json[]> {
    const account = accountOf(runtime.getClient());
    if (!account) return messages;
    const index = await load(account);
    if (accountOf(runtime.getClient()) !== account) return [];
    return messages.map((message) => {
      if (index[String(message.id)] === chatMid) return unsentMessage(message);
      const quote = message.replyTo as Json | undefined;
      return quote && index[String(quote.messageId)] === chatMid
        ? {
          ...message,
          replyTo: { ...quote, text: "", contentType: "UNAVAILABLE" },
        }
        : message;
    });
  }

  function unsend(
    chatMid: string,
    value: unknown,
    createdTime: number,
  ): Promise<Json> {
    const client = runtime.getClient();
    const account = accountOf(client);
    const id = replyMessageId(value);
    if (!client || !account || !chat(chatMid) || !id) {
      return Promise.reject(new Error("unsend_invalid_message"));
    }
    const key = `${account}:${chatMid}:${id}`;
    if (pending.has(key)) return pending.get(key)!;
    const task = (async () => {
      const index = await load(account);
      if (index[id] === chatMid) return { unsent: true };
      const message = await runtime.findMessage(chatMid, id, createdTime);
      if (
        !message || String(message.id) !== id || message.from !== account ||
        message.to !== chatMid || message.chatEvent ||
        ["CALL", "CHAT_EVENT", "UNSENT"].includes(
          String(message.contentType).toUpperCase(),
        )
      ) throw new Error("unsend_not_own_message");
      if (accountOf(runtime.getClient()) !== account) {
        throw new Error("unsend_session_changed");
      }
      const sequence = client.base.getReqseq;
      const seq = typeof sequence === "function"
        ? await sequence.call(client.base)
        : await sequence;
      if (accountOf(runtime.getClient()) !== account) {
        throw new Error("unsend_session_changed");
      }
      await client.base.talk.unsendMessage({ seq, messageId: id });
      await record(account, chatMid, id);
      return { unsent: true };
    })().finally(() => {
      if (pending.get(key) === task) pending.delete(key);
    });
    pending.set(key, task);
    return task;
  }

  function handleOperation(op: Parameters<typeof unsendOperation>[0]) {
    const target = unsendOperation(op);
    const account = accountOf(runtime.getClient());
    if (!target || !account) return false;
    void record(account, target.chatMid, target.id).catch((error) =>
      console.error("[unsend operation]", error)
    );
    return true;
  }
  async function flush() {
    await Promise.all(loaded.values());
    await writes;
  }
  return { unsend, mask, handleOperation, flush };
}

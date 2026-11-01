import type { Client } from "@evex/linejs";

type Contact = { mid: string; name: string };
type Json = Record<string, unknown>;
type Runtime = {
  getClient: () => Client | null;
  selfMid: () => string;
  name: (mid: string) => string | undefined;
  emit: (event: string, payload: Json) => void;
};
const user = (mid: string) => /^u[a-zA-Z0-9_-]{1,128}$/.test(mid);

export function createContactBlocks(runtime: Runtime) {
  let owner: Client | null = null;
  let account = "";
  let version = 0;
  let known = false;
  let contacts: Contact[] = [];
  function session() {
    const client = runtime.getClient();
    const mid = runtime.selfMid();
    if (client !== owner || mid !== account) {
      owner = client;
      account = mid;
      version++;
      known = false;
      contacts = [];
    }
    return { client, mid };
  }
  function snapshot() {
    return { accountMid: account, contacts: [...contacts] };
  }
  function publish() {
    runtime.emit("contact_blocks", snapshot());
  }
  async function refresh() {
    const { client, mid } = session();
    if (!client || !mid) throw new Error("contact_blocks_not_logged_in");
    const revision = ++version;
    const active = () =>
      runtime.getClient() === client && runtime.selfMid() === mid;
    const ids = await client.base.talk.getBlockedContactIds({
      syncReason: "INTERNAL",
    });
    if (
      !Array.isArray(ids) ||
      !ids.every((id) => typeof id === "string" && user(id))
    ) {
      throw new Error("contact_blocks_invalid_response");
    }
    const names = new Map<string, string>();
    const missing = [...new Set(ids)].filter((id) => !runtime.name(id));
    for (let i = 0; i < missing.length && active(); i += 40) {
      try {
        for (
          const contact of await client.base.talk.getContacts({
            mids: missing.slice(i, i + 40),
          }) ?? []
        ) {
          if (contact.mid && contact.displayName) {
            names.set(contact.mid, contact.displayName);
          }
        }
      } catch { /* 名前の取得失敗で、ブロック状態まで不明に戻さない。 */ }
    }
    if (!active()) throw new Error("contact_blocks_session_changed");
    if (version === revision) {
      contacts = [...new Set(ids)].filter((id) => id !== mid).map((id) => ({
        mid: id,
        name: runtime.name(id) || names.get(id) || id,
      }));
      known = true;
      publish();
    }
    return snapshot();
  }
  async function set(mid: string, blocked: boolean) {
    const current = session();
    if (!current.client || !current.mid) {
      throw new Error("contact_blocks_not_logged_in");
    }
    if (!user(mid) || mid === current.mid) {
      throw new Error("contact_blocks_invalid_user");
    }
    const active = () =>
      runtime.getClient() === current.client &&
      runtime.selfMid() === current.mid;
    if (!known) await refresh();
    if (!active() || !known) throw new Error("contact_blocks_session_changed");
    if (contacts.some((c) => c.mid === mid) === blocked) return snapshot();
    ++version;
    const source = current.client.base.getReqseq;
    const reqSeq = typeof source === "function"
      ? await source.call(current.client.base)
      : Number(source);
    if (!active()) throw new Error("contact_blocks_session_changed");
    if (blocked) {
      await current.client.base.talk.blockContact({ reqSeq, id: mid });
    } else {
      await current.client.base.talk.unblockContact({
        reqSeq,
        id: mid,
        reference: "",
      });
    }
    if (!active()) throw new Error("contact_blocks_session_changed");
    ++version;
    contacts = contacts.filter((c) => c.mid !== mid);
    if (blocked) contacts.push({ mid, name: runtime.name(mid) || mid });
    publish();
    return snapshot();
  }
  function isBlocked(mid: string) {
    session();
    return known && contacts.some((c) => c.mid === mid);
  }
  function handleOperation(op: { type?: string | number }) {
    if (
      ![
        "6",
        "7",
        "68",
        "69",
        "BLOCK_CONTACT",
        "UNBLOCK_CONTACT",
        "NOTIFIED_BLOCK_CONTACT",
        "NOTIFIED_UNBLOCK_CONTACT",
      ].includes(String(op.type))
    ) return false;
    void refresh().catch(() =>
      console.error("[contact-blocks] ブロック状態を同期できませんでした。")
    );
    return true;
  }
  const ignoreMessage = (message: Json) =>
    !message.mine &&
    String(message.to ?? "").startsWith("u") &&
    isBlocked(String(message.from ?? ""));
  return { refresh, set, isBlocked, handleOperation, ignoreMessage };
}

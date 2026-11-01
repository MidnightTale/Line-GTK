import type { Client } from "@evex/linejs";

type RecoveryRuntime = {
  getClient: () => Client | null;
  restore: (current: Client) => Promise<Client>;
  activate: (next: Client) => void;
  timeoutMs?: number;
};

// listen()は接続前に戻るため、PUSHの通信路が開いたことも確認する。
export async function waitForReceiver(client: Client, timeoutMs = 5_000) {
  const deadline = Date.now() + timeoutMs;
  while (client.base.authToken) {
    if (
      client.base.push.conns.some((conn) => conn.resStream && conn.reqStream)
    ) return;
    if (Date.now() >= deadline) throw new Error("receiver_connect_timeout");
    await new Promise((resolve) =>
      setTimeout(resolve, Math.min(100, timeoutMs))
    );
  }
  throw new Error("receiver_session_changed");
}

// 古い受信ループを終了させ、新しい接続のストリームに干渉させない。
export function retireClient(client: Client) {
  client.base.authToken = undefined;
  for (const connection of client.base.push.conns) {
    void connection.close().catch(() => {});
  }
  for (const stream of [client.base.push.opStream, client.base.push.sqStream]) {
    try {
      stream.close();
    } catch { /* 終了済みのストリームでも他方の終了を続ける。 */ }
  }
}

export function createConnectionRecovery(runtime: RecoveryRuntime) {
  let pending: { client: Client; promise: Promise<void> } | null = null;

  function reconnect(): Promise<void> {
    const current = runtime.getClient();
    if (!current) return Promise.reject(new Error("not_logged_in"));
    if (pending?.client === current) return pending.promise;

    const entry = { client: current, promise: Promise.resolve() };
    pending = entry;
    entry.promise = (async () => {
      let expired = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      try {
        const next = await Promise.race([
          runtime.restore(current).then((next) => {
            if (expired) retireClient(next);
            return next;
          }),
          new Promise<never>((_, reject) => {
            timer = setTimeout(() => {
              expired = true;
              reject(new Error("receiver_reconnect_timeout"));
            }, runtime.timeoutMs ?? 20_000);
          }),
        ]);
        if (
          runtime.getClient() !== current ||
          !current.base.profile?.mid ||
          next.base.profile?.mid !== current.base.profile.mid
        ) {
          retireClient(next);
          throw new Error("receiver_session_changed");
        }
        // 復帰中に進んだ受信位置も引き継ぎ、休止前からの操作を再開する。
        next.base.poll.sync = structuredClone(current.base.poll.sync);
        runtime.activate(next);
        retireClient(current);
      } finally {
        clearTimeout(timer);
        if (pending === entry) pending = null;
      }
    })();
    return entry.promise;
  }

  return { reconnect };
}

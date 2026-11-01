import type { Client } from "@evex/linejs";

// PUSH requests server heartbeats every 30 seconds, even in quiet chats.
// Leave room for delayed heartbeats and the pusher's own reconnect loop.
const RECEIVER_TIMEOUT_MS = 90_000;

export function createReceiverWatchdog(
  client: Client,
  now = () => performance.now(),
) {
  let lastActivity = now();
  const observe = ({ type }: { type: string }) => {
    if (type === "[LEGY/PUSH] readByte") lastActivity = now();
  };
  client.base.on("log", observe);

  return {
    stalled: () =>
      now() - lastActivity >= RECEIVER_TIMEOUT_MS ||
      !client.base.push.opStream.stream.locked,
    stop: () => client.base.off("log", observe),
  };
}

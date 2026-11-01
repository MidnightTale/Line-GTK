import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createReceiverWatchdog } from "../src/receiver_watchdog.ts";

function receiver() {
  const handlers = new Set<(event: { type: string }) => void>();
  const stream = new ReadableStream();
  const reader = stream.getReader();
  const client = {
    base: {
      on: (_name: string, handler: (event: { type: string }) => void) =>
        handlers.add(handler),
      off: (_name: string, handler: (event: { type: string }) => void) =>
        handlers.delete(handler),
      push: { opStream: { stream } },
    },
  } as unknown as Client;
  return {
    client,
    reader,
    handlers,
    log: (type: string) => {
      for (const handler of handlers) handler({ type });
    },
  };
}

Deno.test("quiet chats stay healthy while PUSH heartbeats arrive", () => {
  const transport = receiver();
  let time = 0;
  const watchdog = createReceiverWatchdog(transport.client, () => time);
  for (let i = 0; i < 10; i++) {
    time += 30_000;
    transport.log("[LEGY/PUSH] readByte");
    assertEquals(watchdog.stalled(), false);
  }
  watchdog.stop();
  assertEquals(transport.handlers.size, 0);
  transport.reader.releaseLock();
});

Deno.test("a silent PUSH stall is detected even while other requests succeed", () => {
  const transport = receiver();
  let time = 0;
  const watchdog = createReceiverWatchdog(transport.client, () => time);
  time = 89_999;
  assertEquals(watchdog.stalled(), false);
  transport.log("request completed");
  transport.log("[LEGY/PUSH] writeByte");
  time = 90_000;
  assertEquals(watchdog.stalled(), true);
  watchdog.stop();
  transport.reader.releaseLock();
});

Deno.test("a terminated event consumer is detected even with active heartbeats", () => {
  const transport = receiver();
  const watchdog = createReceiverWatchdog(transport.client);
  assertEquals(watchdog.stalled(), false);
  transport.reader.releaseLock();
  transport.log("[LEGY/PUSH] readByte");
  assertEquals(watchdog.stalled(), true);
  watchdog.stop();
});

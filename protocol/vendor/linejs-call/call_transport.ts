import type * as LINETypes from "@evex/linejs-types";

/** Minimal transport interface (from linejs CallTransport). */
export interface CallTransport {
  connect(opts: { route: LINETypes.CallRoute }): Promise<void>;
  close(): Promise<void>;
  send(packet: Uint8Array): void | Promise<void>;
  receive(): AsyncIterable<Uint8Array>;
}

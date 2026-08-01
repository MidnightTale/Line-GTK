/**
 * Vendored @evex/linejs call media plane with local patches for 1:1 T103.
 * Upstream 3.2.1 derives DATA SRTP but never uses it in decrypt (PT98 fail).
 */
export {
  PlanetTransport,
  type PlanetTransportOpts,
} from "./planet/transport.ts";
export { opusCodecFactory } from "./opus.ts";

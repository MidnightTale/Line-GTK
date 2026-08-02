import { assertEquals } from "@std/assert";
import {
  decodeNativeSetupOffer,
  packNativeSetupOffer,
} from "../vendor/linejs-call/planet/schema.ts";

Deno.test("native PLANET offer negotiates the H.264 video RTP stream", () => {
  const offer = packNativeSetupOffer({
    mediaPubKey: new Uint8Array(33).fill(1),
    mediaKeyId: 42,
    mediaNonce: new Uint8Array(16).fill(2),
    mediaSecret: new Uint8Array(30).fill(3),
  }, {
    videoEnabled: true,
    videoBitrateKbps: 800,
    videoFps: 15,
  });
  const decoded = decodeNativeSetupOffer(offer);
  const video = decoded.media.find((media) => media.name === "V");

  assertEquals(video?.enabled, 1);
  assertEquals(video?.bitrate, 800);
  assertEquals(video?.kind, 2);
  assertEquals(video?.rtpId, 97);
  assertEquals(video?.rtpPort, 111);
  assertEquals(video?.rtcpId, 211);
});

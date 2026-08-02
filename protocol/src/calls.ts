import type { Client } from "@evex/linejs";
import type { LineDevice } from "./auth.ts";
import {
  AnnexBParser,
  H264AccessUnitAssembler,
  packetizeH264AccessUnit,
} from "./h264.ts";

type Json = Record<string, unknown>;
type CallRuntime = {
  getClient: () => Client | null;
  loadAuthDevice: () => Promise<LineDevice>;
  emitEvent: (event: string, payload?: Json) => void;
  ok: (id: number | string | null, result?: unknown) => void;
  fail: (id: number | string | null, error: string) => void;
};

let runtime: CallRuntime;

export function configureCallRuntime(next: CallRuntime) {
  runtime = next;
}

function emitEvent(event: string, payload: Json = {}) {
  runtime.emitEvent(event, payload);
}

function ok(id: number | string | null, result: unknown = {}) {
  runtime.ok(id, result);
}

function fail(id: number | string | null, error: string) {
  runtime.fail(id, error);
}

async function loadAuthDevice() {
  return await runtime.loadAuthDevice();
}

const LINE_CALL_DEVNAME = Deno.env.get("LINE_CALL_DEVNAME")?.trim() || "";
const LINE_CALL_DEVICE_INFO = Deno.env.get("LINE_CALL_DEVICE_INFO")?.trim() ||
  `ANDROID\t${
    Deno.env.get("LINE_VERSION")?.trim() || "26.6.2"
  }\tAndroid OS\t16`;
const LINE_CALL_OPUS_SIGNAL = (() => {
  const value = Deno.env.get("LINE_CALL_OPUS_SIGNAL")?.trim() || "music";
  return value === "auto" || value === "voice" || value === "music"
    ? value
    : "music";
})();

type ActiveCall = {
  id: string;
  peer: string;
  direction: "out" | "in";
  route?: unknown;
  localMid?: string;
  transport?: {
    connect: (opts: { route: unknown }) => Promise<void>;
    inviteDetailed?: (opts: { to: string }) => Promise<unknown>;
    waitForAnswerDetailed?: (opts?: {
      timeoutMs?: number;
      autoConnRsp?: boolean;
    }) => Promise<{ mediaReady?: boolean }>;
    close: () => Promise<void>;
    send: (
      payload: Uint8Array,
      opts?: { timestampStep?: number },
    ) => Promise<void>;
    videoReady?: () => boolean;
    sendVideo?: (
      payload: Uint8Array,
      opts?: { endOfFrame?: boolean; timestampStep?: number },
    ) => Promise<void>;
    receive: () => AsyncIterable<Uint8Array>;
  };
  stopAudio?: () => void;
  stopScreenShare?: () => void;
  videoCapable: boolean;
  screenSharing: boolean;
  aborted: boolean;
};

let activeCall: ActiveCall | null = null;
let incomingOffer: { callId: string; from: string; kind: string } | null = null;
let callAudioInput = Deno.env.get("LINE_GTK_AUDIO_INPUT")?.trim() || "default";
let callAudioOutput = Deno.env.get("LINE_GTK_AUDIO_OUTPUT")?.trim() ||
  "default";

export function setCallAudioDevices(input?: string, output?: string) {
  if (input && input.trim()) callAudioInput = input.trim();
  if (output && output.trim()) callAudioOutput = output.trim();
  if (!callAudioInput) callAudioInput = "default";
  if (!callAudioOutput) callAudioOutput = "default";
}

export function setCallGains(micGain?: unknown, spkGain?: unknown) {
  if (micGain !== undefined) {
    callAudioCtl.micGain = clampGain(micGain, callAudioCtl.micGain);
  }
  if (spkGain !== undefined) {
    callAudioCtl.spkGain = clampGain(spkGain, callAudioCtl.spkGain);
  }
}

/** Android PLANET defaults from https://linejs.evex.land/docs/call */
function planetDeviceInfo() {
  return LINE_CALL_DEVICE_INFO;
}

const callAudioCtl = {
  muted: false,
  deafened: false,
  micGain: 1,
  spkGain: 1,
};

function resetCallAudioCtl() {
  callAudioCtl.muted = false;
  callAudioCtl.deafened = false;
  // keep last gains across calls
}

function clampGain(v: unknown, fallback = 1): number {
  const n = typeof v === "number" ? v : Number(v);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(2.5, Math.max(0, n));
}

function applyPcmGain(samples: Int16Array, gain: number): Int16Array {
  if (gain === 1) return samples;
  const out = new Int16Array(samples.length);
  for (let i = 0; i < samples.length; i++) {
    const v = Math.round(samples[i]! * gain);
    out[i] = v < -32768 ? -32768 : v > 32767 ? 32767 : v;
  }
  return out;
}

let lastDecryptFailLog = 0;
let decryptFailCount = 0;

/** Call tracing — set LINE_CALL_DEBUG=0 to quiet noisy packet spam. */
const CALL_DEBUG_VERBOSE = Deno.env.get("LINE_CALL_DEBUG")?.trim() !== "0";

function callLog(stage: string, detail?: Record<string, unknown>) {
  if (detail) console.error(`[call] ${stage}`, detail);
  else console.error(`[call] ${stage}`);
}

function summarizeRoute(route: unknown): Record<string, unknown> {
  const r = route as Record<string, unknown>;
  return {
    fakeCall: r.fakeCall,
    fromToken: typeof r.fromToken === "string"
      ? `${r.fromToken.slice(0, 4)}…`
      : r.fromToken,
    voipAddress: r.voipAddress ?? r.cscfHost,
    voipUdpPort: r.voipUdpPort ?? r.cscfPort,
    fromZone: r.fromZone ?? r.iZone,
    toZone: r.toZone ?? r.rZone,
    callFlowType: r.callFlowType,
  };
}

const noisyCallDebugTypes = new Set([
  "recv",
  "send",
  "decrypt_ok",
  "plain_shape",
  "cc_shape",
  "mc_shape",
  "raw_plain",
  "rtp_recv",
  "planet_msg",
  "send_planet_msg",
]);
let lastNoisyCallDebug = 0;
let noisyCallDebugSuppressed = 0;

function logTransportDebug(
  ev: Record<string, unknown>,
  extras?: Record<string, unknown>,
) {
  const type = String(ev.type ?? "");
  if (noisyCallDebugTypes.has(type)) {
    if (!CALL_DEBUG_VERBOSE) return;
    const now = Date.now();
    noisyCallDebugSuppressed++;
    if (now - lastNoisyCallDebug < 1500) return;
    console.error("[call debug]", {
      ...ev,
      ...extras,
      noisySuppressed: Math.max(0, noisyCallDebugSuppressed - 1),
    });
    lastNoisyCallDebug = now;
    noisyCallDebugSuppressed = 0;
    return;
  }
  console.error("[call debug]", extras ? { ...ev, ...extras } : ev);
}

export async function doCallStart(
  id: number | string | null,
  peerMid: string,
  preferVideo = false,
) {
  const client = runtime.getClient();
  callLog("start requested", {
    peerMid,
    device: await loadAuthDevice(),
    hasClient: !!client,
    activeCall: !!activeCall,
  });
  if (!client) {
    callLog("reject not_logged_in");
    fail(id, "not_logged_in");
    return;
  }
  const device = await loadAuthDevice();
  if (device === "DESKTOPWIN") {
    callLog("reject relogin_android_required", { device });
    fail(id, "relogin_android_required");
    return;
  }
  if (!peerMid.startsWith("u")) {
    callLog("reject voice_call_dm_only", { peerMid });
    fail(id, "voice_call_dm_only");
    return;
  }
  if (activeCall) {
    callLog("reject call_already_active", {
      peer: activeCall.peer,
      id: activeCall.id,
      aborted: activeCall.aborted,
    });
    fail(id, "call_already_active");
    return;
  }

  const callId = `out-${Date.now()}`;
  if (id != null) ok(id, { callId, peer: peerMid, state: "starting" });
  emitEvent("call_state", { callId, peer: peerMid, state: "acquiring" });

  const localMid = client.base.profile?.mid;
  if (!localMid) {
    callLog("fail profile not ready");
    emitEvent("call_state", {
      callId,
      peer: peerMid,
      state: "failed",
      error: "profile not ready",
    });
    return;
  }

  activeCall = {
    id: callId,
    peer: peerMid,
    direction: "out",
    localMid,
    videoCapable: preferVideo,
    screenSharing: false,
    aborted: false,
  };
  const call = activeCall;

  try {
    callLog("import PlanetTransport…");
    const {
      PlanetTransport,
      opusCodecFactory,
    } = await import("../vendor/linejs-call/entry.ts");
    callLog("import ok");

    callLog("acquireRoute…", {
      peerMid,
      fromEnvInfo: LINE_CALL_DEVNAME || "(none)",
      deviceInfo: planetDeviceInfo(),
    });
    let route: Awaited<ReturnType<typeof client.call.acquireRoute>> | undefined;
    if (preferVideo) {
      try {
        route = await client.call.acquireRoute({
          to: peerMid,
          callType: "VIDEO",
          ...(LINE_CALL_DEVNAME
            ? { fromEnvInfo: { devname: LINE_CALL_DEVNAME } }
            : {}),
        });
      } catch (error) {
        call.videoCapable = false;
        callLog("video route unavailable; falling back to audio", {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
    route ??= await client.call.acquireRoute({
      to: peerMid,
      callType: "AUDIO",
      ...(LINE_CALL_DEVNAME
        ? { fromEnvInfo: { devname: LINE_CALL_DEVNAME } }
        : {}),
    });
    callLog("acquireRoute ok", summarizeRoute(route));
    if (call.aborted || activeCall !== call) {
      callLog("aborted during acquireRoute");
      return;
    }
    const fakeCall = !!(route as { fakeCall?: boolean }).fakeCall;
    if (fakeCall) {
      throw new Error(
        "LINE returned fakeCall=true — peer will not ring (account/device/push blocked, or try again later)",
      );
    }
    call.route = route;

    let mediaSendCount = 0;
    let mediaRecvCount = 0;
    let lastMediaSendLog = 0;
    const transport = new PlanetTransport({
      localMid,
      timeoutMs: 10_000,
      mediaKeyMode: "audio-reverse-stage",
      enableVideo: call.videoCapable,
      videoBitrateKbps: 800,
      videoFps: 15,
      deviceInfo: planetDeviceInfo(),
      debug: (ev: Record<string, unknown>) => {
        if (ev.type === "rel_req") {
          const phrase = String(ev.relPhrase ?? "");
          callLog("inbound REL_REQ", {
            relCode: ev.relCode,
            relPhrase: ev.relPhrase,
            releaser: ev.releaser,
            userRelCode: ev.userRelCode,
            mediaSendCount,
            mediaRecvCount,
          });
          // Server push failure — show as failed, not a quiet hangup.
          if (
            Number(ev.relCode) === 205 ||
            phrase.includes("406") ||
            phrase.includes("Not Acceptable") ||
            phrase.includes("PUSH")
          ) {
            emitEvent("call_state", {
              callId: call.id,
              peer: call.peer,
              state: "failed",
              error: phrase || `REL ${ev.relCode}`,
            });
          }
          queueMicrotask(() => {
            void endActiveCall({ silent: true });
          });
          return;
        }
        if (ev.type === "media_send") {
          mediaSendCount++;
          const now = Date.now();
          if (mediaSendCount === 1 || now - lastMediaSendLog > 5000) {
            logTransportDebug({
              type: "media_send",
              count: mediaSendCount,
              payloadBytes: ev.payloadBytes,
              seq: ev.seq,
            });
            lastMediaSendLog = now;
          }
          return;
        }
        if (ev.type === "media_recv") {
          mediaRecvCount++;
          if (mediaRecvCount === 1 || mediaRecvCount % 50 === 0) {
            logTransportDebug({
              type: "media_recv",
              count: mediaRecvCount,
              payloadBytes: ev.payloadBytes,
              mediaKeyMode: ev.mediaKeyMode,
              payloadType: ev.payloadType,
            });
          }
          return;
        }
        if (ev.type === "media_decrypt_fail") {
          decryptFailCount++;
          const now = Date.now();
          if (now - lastDecryptFailLog > 2500) {
            logTransportDebug({
              ...ev,
              suppressed: Math.max(0, decryptFailCount - 1),
              mediaSendCount,
              mediaRecvCount,
            });
            lastDecryptFailLog = now;
            decryptFailCount = 0;
          }
          return;
        }
        logTransportDebug(ev, { mediaSendCount, mediaRecvCount });
      },
    });
    call.transport = transport as unknown as NonNullable<
      ActiveCall["transport"]
    >;

    if (call.aborted) {
      callLog("aborted before connect");
      await transport.close().catch(() => {});
      return;
    }

    callLog("connect…");
    await transport.connect({ route });
    callLog("connect ok");
    if (call.aborted || activeCall !== call) {
      callLog("aborted after connect");
      await transport.close().catch(() => {});
      return;
    }

    emitEvent("call_state", { callId, peer: peerMid, state: "ringing" });

    if (call.aborted || activeCall !== call) {
      callLog("aborted before invite");
      await transport.close().catch(() => {});
      return;
    }

    callLog("inviteDetailed (SETUP)…");
    try {
      await transport.inviteDetailed({ to: peerMid });
      callLog("inviteDetailed ok — peer should ring");
    } catch (e) {
      if (call.aborted) {
        callLog("invite stopped after hangup", {
          err: e instanceof Error ? e.message : String(e),
        });
        return;
      }
      throw e;
    }
    if (call.aborted || activeCall !== call) {
      callLog("aborted after invite");
      await transport.close().catch(() => {});
      return;
    }

    callLog("waitForAnswer…");
    let answer: { mediaReady?: boolean };
    try {
      answer = await transport.waitForAnswerDetailed({
        timeoutMs: 60_000,
        autoConnRsp: true,
      });
    } catch (e) {
      if (call.aborted) {
        callLog("wait-for-answer stopped after hangup", {
          err: e instanceof Error ? e.message : String(e),
        });
        return;
      }
      throw e;
    }
    callLog("answer", { mediaReady: answer.mediaReady });
    if (call.aborted || activeCall !== call) {
      callLog("aborted after answer");
      await transport.close().catch(() => {});
      return;
    }
    if (!answer.mediaReady) {
      throw new Error(
        "answered without media — often LINE error 103 (unofficial client blocked)",
      );
    }

    call.videoCapable = call.videoCapable && !!transport.videoReady?.();
    emitEvent("call_state", {
      callId,
      peer: peerMid,
      state: "connected",
      videoCapable: call.videoCapable,
    });
    callLog("starting audio I/O");
    call.stopAudio = await startCallAudioIO(
      transport,
      opusCodecFactory as unknown as Parameters<typeof startCallAudioIO>[1],
    );
    callLog("audio I/O running");
  } catch (e) {
    const raw = e instanceof Error ? e.message : String(e);
    if (call.aborted) {
      callLog("aborted:", { err: raw });
      return;
    }
    const lower = raw.toLowerCase();
    const msg = lower.includes("fakecall") ||
        lower.includes("103") ||
        lower.includes("406") ||
        lower.includes("timeout") ||
        lower.includes("media") ||
        lower.includes("denied") ||
        lower.includes("reject")
      ? raw
      : raw;
    callLog("FAILED", { err: msg });
    emitEvent("call_state", {
      callId,
      peer: peerMid,
      state: "failed",
      error: msg,
    });
    await endActiveCall({ silent: true });
  }
}

/** Best-effort REL when hangup happens after acquireRoute (no re-invite — that re-rings). */
async function cancelRouteRing(
  peerMid: string,
  localMid: string,
  route: unknown,
) {
  void peerMid;
  try {
    const { PlanetTransport } = await import("../vendor/linejs-call/entry.ts");
    const transport = new PlanetTransport({
      localMid,
      timeoutMs: 8_000,
      mediaKeyMode: "audio-reverse-stage",
      deviceInfo: planetDeviceInfo(),
    });
    await transport.connect({ route: route as never });
    await transport.close();
  } catch (e) {
    console.error("[call cancel]", e);
  }
}

export async function doCallAnswer(id: number | string | null) {
  const client = runtime.getClient();
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  const offer = incomingOffer;
  if (!offer) {
    fail(id, "no_incoming_call");
    return;
  }
  if (activeCall) {
    fail(id, "call_already_active");
    return;
  }
  incomingOffer = null;
  // linejs 3.2.1 has no callee SETUP handler for 1:1 PLANET yet.
  // Answering starts a fresh outgoing audio session to the caller so media
  // can negotiate; hangup still sends REL_REQ to stop their ring.
  ok(id, { answering: true, peer: offer.from, callId: offer.callId });
  await doCallStart(null, offer.from, false);
}

export async function doCallDecline(id: number | string | null) {
  const client = runtime.getClient();
  const offer = incomingOffer;
  incomingOffer = null;
  if (offer && client?.base.profile?.mid) {
    // Try to tear down any server-side ring by acquiring+REL quickly.
    try {
      const route = await client.call.acquireRoute({
        to: offer.from,
        callType: "AUDIO",
      });
      await cancelRouteRing(offer.from, client.base.profile.mid, route);
    } catch (e) {
      console.error("[call decline]", e);
    }
    emitEvent("call_state", {
      callId: offer.callId,
      peer: offer.from,
      state: "ended",
    });
  }
  ok(id, { declined: true });
}

async function startCallAudioIO(
  transport: {
    send: (
      payload: Uint8Array,
      opts?: { timestampStep?: number },
    ) => Promise<void>;
    receive: () => AsyncIterable<Uint8Array>;
  },
  opusCodecFactory: () => Promise<{
    newEncoder: (o: Record<string, unknown>) => {
      encode: (o: {
        samples: Int16Array;
        sampleRate: number;
        channels: number;
      }) => Uint8Array | null;
      close?: () => void;
    };
    newDecoder: (o: Record<string, unknown>) => {
      decode: (packet: Uint8Array) => {
        samples: Int16Array;
        sampleRate: number;
        channels: number;
      } | null;
      close?: () => void;
    };
  }>,
): Promise<() => void> {
  const SAMPLE_RATE = 48_000;
  const frameMs = 20;
  const frameSamples = Math.floor((SAMPLE_RATE * frameMs) / 1000);
  const bytesPerFrame = frameSamples * 2;
  let stopped = false;

  const micDev = callAudioInput || "default";
  const spkDev = callAudioOutput || "default";

  const spawnMic = () =>
    new Deno.Command("ffmpeg", {
      args: [
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "pulse",
        "-i",
        micDev,
        "-ac",
        "1",
        "-ar",
        String(SAMPLE_RATE),
        "-f",
        "s16le",
        "pipe:1",
      ],
      stdout: "piped",
      stderr: "piped",
    }).spawn();

  const spawnSpeaker = () =>
    new Deno.Command("ffmpeg", {
      args: [
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "s16le",
        "-ar",
        String(SAMPLE_RATE),
        "-ac",
        "1",
        "-i",
        "pipe:0",
        "-f",
        "pulse",
        spkDev,
      ],
      stdin: "piped",
      stdout: "null",
      stderr: "piped",
    }).spawn();

  let mic = spawnMic();
  let speaker = spawnSpeaker();

  const codec = await opusCodecFactory();
  // Match example/call_on_command.ts defaults (signal=music, vbr=false, no bitrate).
  const encoder = codec.newEncoder({
    sampleRate: SAMPLE_RATE,
    channels: 1,
    frameDurationMs: frameMs,
    signal: LINE_CALL_OPUS_SIGNAL,
    vbr: false,
  });
  const decoder = codec.newDecoder({
    sampleRate: SAMPLE_RATE,
    channels: 1,
  });

  let micReader = mic.stdout.getReader();
  let spkWriter = speaker.stdin.getWriter();
  let pending = new Uint8Array(0);
  let lastMicAt = Date.now();
  let opusFailLog = 0;
  let opusFailCount = 0;
  // Wall-clock pacing like the official example (await sleep(frameMs)).
  // ffmpeg/Pulse often delivers buffered bursts; flooding RTP makes mobile hang up.
  let nextSendAt = performance.now();
  let sendLock: Promise<void> = Promise.resolve();

  const sleepMs = (ms: number) =>
    new Promise<void>((r) => setTimeout(r, Math.max(0, ms)));

  async function sendPcmFrame(samples: Int16Array) {
    let pcm = callAudioCtl.muted ? new Int16Array(frameSamples) : samples;
    pcm = applyPcmGain(pcm, callAudioCtl.micGain);
    const packet = encoder.encode({
      samples: pcm,
      sampleRate: SAMPLE_RATE,
      channels: 1,
    });
    if (!packet) return;
    const payload = new Uint8Array(1 + packet.length);
    payload[0] = 0x00;
    payload.set(packet, 1);
    const now = performance.now();
    const wait = nextSendAt - now;
    if (wait > 1) await sleepMs(wait);
    nextSendAt = Math.max(performance.now(), nextSendAt) + frameMs;
    lastMicAt = Date.now();
    await transport.send(payload, { timestampStep: frameSamples });
  }

  function enqueuePcmFrame(samples: Int16Array) {
    sendLock = sendLock
      .then(() => sendPcmFrame(samples))
      .catch((e) => {
        if (!stopped) console.error("[call send]", e);
      });
    return sendLock;
  }

  function tryDecodeOpus(raw: Uint8Array) {
    if (raw.length < 3) return null;
    const candidates: Uint8Array[] = [];
    // LINE PLANET 1:1 wrapper is usually one prefix byte (0x00).
    if (raw.length > 1 && (raw[0] === 0x00 || raw[0] === 0x10)) {
      candidates.push(raw.subarray(1));
    }
    candidates.push(raw);
    for (const packet of candidates) {
      if (packet.length < 2) continue;
      try {
        const frame = decoder.decode(packet);
        if (frame?.samples?.length) return frame;
      } catch {
        /* try next shape */
      }
    }
    return null;
  }

  // Keep RTP alive even if Pulse mic hiccups — peer hangs up if media stops.
  const keepalive = setInterval(() => {
    if (stopped) return;
    if (Date.now() - lastMicAt < frameMs * 2) return;
    void enqueuePcmFrame(new Int16Array(frameSamples));
  }, frameMs);

  // Mic → Opus → LINE (restart ffmpeg if Pulse drops)
  (async () => {
    while (!stopped) {
      try {
        while (!stopped) {
          const { value, done } = await micReader.read();
          if (done || !value) break;
          const merged = new Uint8Array(pending.length + value.length);
          merged.set(pending, 0);
          merged.set(value, pending.length);
          pending = merged;
          while (pending.length >= bytesPerFrame && !stopped) {
            const slice = pending.subarray(0, bytesPerFrame);
            pending = pending.subarray(bytesPerFrame);
            const samples = new Int16Array(
              slice.buffer,
              slice.byteOffset,
              frameSamples,
            );
            // Copy: slice views the growing buffer until paced send runs.
            await enqueuePcmFrame(new Int16Array(samples));
          }
        }
      } catch (e) {
        if (!stopped) console.error("[call mic]", e);
      }
      if (stopped) break;
      console.error("[call mic] capture ended — restarting ffmpeg");
      try {
        mic.kill("SIGTERM");
      } catch { /* ignore */ }
      await sleepMs(200);
      if (stopped) break;
      try {
        mic = spawnMic();
        micReader = mic.stdout.getReader();
        pending = new Uint8Array(0);
      } catch (e) {
        console.error("[call mic restart]", e);
        await sleepMs(500);
      }
    }
    try {
      encoder.close?.();
    } catch { /* ignore */ }
  })();

  // LINE → Opus → Speaker (pace to 20 ms — unpaced bursts sound like static)
  (async () => {
    let nextPlayAt = performance.now();
    try {
      for await (const payload of transport.receive()) {
        if (stopped) break;
        try {
          const raw = payload instanceof Uint8Array
            ? payload
            : new Uint8Array(payload as ArrayBuffer);
          // Tiny / non-Opus control payloads decode to noise — skip.
          if (raw.length < 10 || raw.length > 400) continue;
          const frame = tryDecodeOpus(raw);
          if (!frame) {
            opusFailCount++;
            const now = Date.now();
            if (now - opusFailLog > 2500) {
              console.error("[call speaker] opus skip", {
                bytes: raw.length,
                head: raw[0],
                suppressed: Math.max(0, opusFailCount - 1),
              });
              opusFailLog = now;
              opusFailCount = 0;
            }
            continue;
          }
          if (callAudioCtl.deafened) continue;
          // Drop wildly wrong frame sizes (corrupt decode → random noise).
          if (
            frame.samples.length < frameSamples / 2 ||
            frame.samples.length > frameSamples * 2
          ) {
            continue;
          }
          const now = performance.now();
          const wait = nextPlayAt - now;
          if (wait > 1) await sleepMs(wait);
          nextPlayAt = Math.max(performance.now(), nextPlayAt) + frameMs;
          const gained = applyPcmGain(frame.samples, callAudioCtl.spkGain);
          const bytes = new Uint8Array(
            gained.buffer,
            gained.byteOffset,
            gained.byteLength,
          );
          try {
            await spkWriter.write(bytes);
          } catch (e) {
            if (stopped) break;
            console.error("[call speaker] pulse write failed — restarting", e);
            try {
              speaker.kill("SIGTERM");
            } catch { /* ignore */ }
            await sleepMs(150);
            if (stopped) break;
            speaker = spawnSpeaker();
            spkWriter = speaker.stdin.getWriter();
            await spkWriter.write(bytes).catch(() => {});
          }
        } catch (e) {
          if (!stopped) console.error("[call speaker frame]", e);
        }
      }
      if (!stopped) {
        console.error("[call speaker] receive stream ended");
        emitEvent("call_state", {
          callId: activeCall?.id ?? "",
          peer: activeCall?.peer ?? "",
          state: "ended",
        });
        queueMicrotask(() => {
          void endActiveCall({ silent: true });
        });
      }
    } catch (e) {
      if (!stopped) console.error("[call speaker]", e);
    } finally {
      try {
        decoder.close?.();
      } catch { /* ignore */ }
    }
  })();

  return () => {
    stopped = true;
    clearInterval(keepalive);
    try {
      mic.kill("SIGTERM");
    } catch { /* ignore */ }
    try {
      speaker.kill("SIGTERM");
    } catch { /* ignore */ }
    try {
      micReader.cancel();
    } catch { /* ignore */ }
    try {
      spkWriter.close();
    } catch { /* ignore */ }
  };
}

type ScreenShareControl = {
  stopped: boolean;
  selector?: Deno.ChildProcess;
  recorder?: Deno.ChildProcess;
};

const SCREEN_SHARE_FPS = 15;
const SCREEN_SHARE_RTP_STEP = Math.round(90_000 / SCREEN_SHARE_FPS);

export function doCallScreenStart(id: number | string | null) {
  const call = activeCall;
  const transport = call?.transport;
  if (!call || !transport) {
    fail(id, "call_not_connected");
    return;
  }
  if (!call.videoCapable || !transport.videoReady?.() || !transport.sendVideo) {
    fail(id, "call_video_not_negotiated");
    return;
  }
  if (call.screenSharing) {
    ok(id, { state: "active" });
    return;
  }

  const control: ScreenShareControl = { stopped: false };
  call.screenSharing = true;
  call.stopScreenShare = () => {
    control.stopped = true;
    for (const child of [control.selector, control.recorder]) {
      try {
        child?.kill("SIGINT");
      } catch { /* already exited */ }
    }
  };
  ok(id, { state: "selecting" });
  emitEvent("screen_share_state", { state: "selecting" });

  void runScreenShare(call, control).catch((error) => {
    if (!control.stopped) {
      const message = error instanceof Error ? error.message : String(error);
      callLog("screen share failed", { error: message });
      emitEvent("screen_share_state", { state: "failed", error: message });
    }
  }).finally(() => {
    try {
      control.recorder?.kill("SIGINT");
    } catch { /* already exited */ }
    if (activeCall === call) {
      call.screenSharing = false;
      call.stopScreenShare = undefined;
      if (control.stopped) {
        emitEvent("screen_share_state", { state: "stopped" });
      }
    }
  });
}

export function doCallScreenStop(id: number | string | null) {
  const call = activeCall;
  if (!call?.screenSharing) {
    ok(id, { state: "stopped" });
    return;
  }
  call.stopScreenShare?.();
  ok(id, { state: "stopping" });
}

async function runScreenShare(call: ActiveCall, control: ScreenShareControl) {
  const selector = new Deno.Command("slurp", {
    args: ["-f", "%x,%y %wx%h"],
    stdin: "null",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  control.selector = selector;
  const selected = await selector.output();
  control.selector = undefined;
  if (control.stopped) return;
  const geometry = new TextDecoder().decode(selected.stdout).trim();
  if (!selected.success || !/^\d+,\d+ \d+x\d+$/.test(geometry)) {
    throw new Error("screen_share_selection_canceled");
  }

  emitEvent("screen_share_state", { state: "starting", geometry });
  const recorder = new Deno.Command("wf-recorder", {
    args: [
      "-g",
      geometry,
      "-r",
      String(SCREEN_SHARE_FPS),
      "-x",
      "yuv420p",
      "-c",
      "libx264",
      "-m",
      "h264",
      "-p",
      "preset=ultrafast",
      "-p",
      "tune=zerolatency",
      "-p",
      "profile=baseline",
      "-p",
      "crf=28",
      "-p",
      "g=30",
      "-p",
      "bf=0",
      "-p",
      "x264-params=aud=1:repeat-headers=1:slices=1:scenecut=0",
      "-f",
      "-",
    ],
    stdin: "null",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  control.recorder = recorder;
  const stderr = readProcessStderr(recorder.stderr);
  emitEvent("screen_share_state", { state: "active", geometry });

  const annexB = new AnnexBParser();
  const accessUnits = new H264AccessUnitAssembler();
  const reader = recorder.stdout.getReader();
  try {
    while (!control.stopped && activeCall === call && !call.aborted) {
      const { value, done } = await reader.read();
      if (done) break;
      if (!value?.length) continue;
      for (const nal of annexB.push(value)) {
        for (const unit of accessUnits.push(nal)) {
          await sendScreenAccessUnit(call, unit);
        }
      }
    }
    for (const nal of annexB.flush()) {
      for (const unit of accessUnits.push(nal)) {
        await sendScreenAccessUnit(call, unit);
      }
    }
    for (const unit of accessUnits.flush()) {
      await sendScreenAccessUnit(call, unit);
    }
  } finally {
    try {
      await reader.cancel();
    } catch { /* ignore */ }
  }

  const status = await recorder.status;
  const stderrTail = await stderr;
  if (!control.stopped && !status.success) {
    throw new Error(stderrTail || `wf-recorder exited ${status.code}`);
  }
}

async function sendScreenAccessUnit(call: ActiveCall, nals: Uint8Array[]) {
  const sendVideo = call.transport?.sendVideo;
  if (!sendVideo || !nals.length) return;
  for (const packet of packetizeH264AccessUnit(nals)) {
    await sendVideo(packet.payload, {
      endOfFrame: packet.endOfFrame,
      timestampStep: SCREEN_SHARE_RTP_STEP,
    });
  }
}

async function readProcessStderr(
  stream: ReadableStream<Uint8Array>,
): Promise<string> {
  const reader = stream.getReader();
  let tail = "";
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (!value) continue;
      tail = (tail + new TextDecoder().decode(value)).slice(-4096);
    }
  } catch { /* process closed */ }
  return tail.trim();
}

async function endActiveCall(opts: { silent?: boolean } = {}) {
  const cur = activeCall;
  if (!cur || cur.aborted) {
    callLog("endActiveCall skip", {
      hasCall: !!activeCall,
      aborted: activeCall?.aborted,
      silent: !!opts.silent,
    });
    return;
  }
  callLog("endActiveCall", {
    id: cur.id,
    peer: cur.peer,
    hasTransport: !!cur.transport,
    hasRoute: !!cur.route,
    silent: !!opts.silent,
  });
  // Keep the object alive so doCallStart sees aborted=true and skips invite.
  cur.aborted = true;
  resetCallAudioCtl();
  try {
    cur.stopAudio?.();
  } catch { /* ignore */ }
  try {
    cur.stopScreenShare?.();
  } catch { /* ignore */ }

  if (cur.transport) {
    try {
      callLog("hangup — closing transport (REL if SETUP was sent)");
      await cur.transport.close();
      callLog("transport closed");
    } catch (e) {
      callLog("hangup close failed", {
        err: e instanceof Error ? e.message : String(e),
      });
    }
  } else if (cur.route && cur.localMid) {
    callLog("hangup before invite — no PLANET SETUP to cancel");
  }

  if (activeCall === cur) activeCall = null;

  if (!opts.silent) {
    emitEvent("call_state", {
      callId: cur.id,
      peer: cur.peer,
      state: "ended",
    });
  }
}

export async function doCallEnd(id: number | string | null) {
  incomingOffer = null;
  await endActiveCall();
  ok(id, { ended: true });
}

export function handleIncomingCall(ev: {
  callMid?: string;
  from?: string;
  kind?: string;
}) {
  const callId = String(ev.callMid ?? "");
  const from = String(ev.from ?? "");
  const kind = String(ev.kind ?? "AUDIO");
  incomingOffer = { callId, from, kind };
  emitEvent("call_incoming", { callId, from, kind });
}

export function handleCallCancel(ev: {
  callMid?: string;
  from?: string;
  reason?: string;
}) {
  const from = String(ev.from ?? "");
  if (
    incomingOffer?.from === from ||
    incomingOffer?.callId === String(ev.callMid ?? "")
  ) {
    incomingOffer = null;
  }
  emitEvent("call_canceled", {
    callId: String(ev.callMid ?? ""),
    from,
    reason: String(ev.reason ?? ""),
  });
  if (activeCall && (activeCall.peer === from || !from)) {
    void endActiveCall();
  }
}

export function updateCallAudio(params: Json) {
  if (params.muted !== undefined) callAudioCtl.muted = !!params.muted;
  if (params.deafened !== undefined) callAudioCtl.deafened = !!params.deafened;
  if (params.micGain !== undefined) {
    callAudioCtl.micGain = clampGain(params.micGain, callAudioCtl.micGain);
  }
  if (params.spkGain !== undefined) {
    callAudioCtl.spkGain = clampGain(params.spkGain, callAudioCtl.spkGain);
  }
  return { ...callAudioCtl };
}

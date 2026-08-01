/**
 * LINE protocol sidecar for line-gtk.
 * NDJSON over stdin/stdout. Heavy I/O is cached + async so GTK stays snappy.
 */

import { Client, loginWithAuthToken, loginWithQR, TalkMessage } from "@evex/linejs";
import { FileStorage } from "@evex/linejs/storage";
import { fromFileUrl, join } from "@std/path";
import { Buffer } from "node:buffer";

type Json = Record<string, unknown>;

type ChatRow = {
  mid: string;
  name: string;
  kind: string;
  avatarPath: string | null;
  picturePath?: string | null;
  lastActivity: number;
  unread: number;
  preview: string;
  muted?: boolean;
};

const dataDir = Deno.env.get("LINE_GTK_DATA") ??
  join(Deno.env.get("HOME") ?? ".", ".local", "share", "line-gtk");
await Deno.mkdir(dataDir, { recursive: true });
const cacheDir = join(dataDir, "cache");
const avatarDir = join(cacheDir, "avatars");
const mediaDir = join(cacheDir, "media");
const stickerDir = join(cacheDir, "stickers");
await Deno.mkdir(avatarDir, { recursive: true });
await Deno.mkdir(mediaDir, { recursive: true });
await Deno.mkdir(stickerDir, { recursive: true });

const storagePath = join(dataDir, "linejs-storage.json");
const authPath = join(dataDir, "auth-token.txt");
const authDevicePath = join(dataDir, "auth-device.txt");
/** Docs: reliable PLANET audio needs ANDROID / ANDROIDSECONDARY, not DESKTOPWIN. */
const LINE_DEVICE = (Deno.env.get("LINE_DEVICE")?.trim() ||
  "ANDROIDSECONDARY") as "ANDROID" | "ANDROIDSECONDARY" | "DESKTOPWIN";
const LINE_VERSION = Deno.env.get("LINE_VERSION")?.trim() || "26.6.2";
/** Only for primary ANDROID tokens (example leaves this unset for secondary). */
const LINE_CALL_DEVNAME = Deno.env.get("LINE_CALL_DEVNAME")?.trim() || "";
const LINE_CALL_DEVICE_INFO = Deno.env.get("LINE_CALL_DEVICE_INFO")?.trim() ||
  `ANDROID\t${LINE_VERSION}\tAndroid OS\t16`;
const LINE_CALL_OPUS_SIGNAL = (() => {
  const v = Deno.env.get("LINE_CALL_OPUS_SIGNAL")?.trim() || "music";
  return v === "auto" || v === "voice" || v === "music" ? v : "music";
})();
const chatCachePath = join(cacheDir, "chats.json");
const contactCachePath = join(cacheDir, "contacts.json");
const msgDiskDir = join(cacheDir, "messages");
await Deno.mkdir(msgDiskDir, { recursive: true });

const DAY_MS = 86_400_000;
const WEEK_MS = 7 * DAY_MS;
const MONTH_MS = 30 * DAY_MS;
const FOREVER_MS = Number.MAX_SAFE_INTEGER;

type CachePolicy = {
  memChat: number;
  memMsg: number;
  diskChat: number;
  diskMsg: number;
  ownedPack: number;
  contactsRefresh: number;
  /** 0 = never expire miss markers */
  animMiss: number;
};

function policyFor(retention: string): CachePolicy {
  switch ((retention || "smart").toLowerCase()) {
    case "day":
      return {
        memChat: 30 * 60_000,
        memMsg: 15 * 60_000,
        diskChat: DAY_MS,
        diskMsg: DAY_MS,
        ownedPack: DAY_MS,
        contactsRefresh: 30 * 60_000,
        animMiss: DAY_MS,
      };
    case "week":
      return {
        memChat: 60 * 60_000,
        memMsg: 30 * 60_000,
        diskChat: WEEK_MS,
        diskMsg: WEEK_MS,
        ownedPack: WEEK_MS,
        contactsRefresh: 2 * 60 * 60_000,
        animMiss: WEEK_MS,
      };
    case "month":
      return {
        memChat: 2 * 60 * 60_000,
        memMsg: 60 * 60_000,
        diskChat: MONTH_MS,
        diskMsg: MONTH_MS,
        ownedPack: MONTH_MS,
        contactsRefresh: 6 * 60 * 60_000,
        animMiss: MONTH_MS,
      };
    case "forever":
      return {
        memChat: 6 * 60 * 60_000,
        memMsg: 2 * 60 * 60_000,
        diskChat: FOREVER_MS,
        diskMsg: FOREVER_MS,
        ownedPack: WEEK_MS,
        contactsRefresh: 6 * 60 * 60_000,
        animMiss: 0,
      };
    case "smart":
    default:
      // Stickers / avatars / media files stay on disk forever.
      // Messages ~30d, chats ~14d, warm memory for snappy reopen.
      // anim.miss expires weekly so packs that later gain animation recheck.
      return {
        memChat: 30 * 60_000,
        memMsg: 20 * 60_000,
        diskChat: 14 * DAY_MS,
        diskMsg: 30 * DAY_MS,
        ownedPack: DAY_MS,
        contactsRefresh: 30 * 60_000,
        animMiss: WEEK_MS,
      };
  }
}

let cacheRetention =
  (Deno.env.get("LINE_GTK_CACHE_RETENTION") || "smart").toLowerCase();
let cachePolicyState = policyFor(cacheRetention);
let cachePolicyCheckedAt = 0;

async function refreshCachePolicy() {
  if (Date.now() - cachePolicyCheckedAt < 3_000) return cachePolicyState;
  cachePolicyCheckedAt = Date.now();
  try {
    const raw = JSON.parse(
      await Deno.readTextFile(join(dataDir, "settings.json")),
    );
    const next = String(raw.cache_retention || cacheRetention || "smart")
      .toLowerCase();
    if (next !== cacheRetention) {
      cacheRetention = next;
      cachePolicyState = policyFor(next);
      console.error(`[cache] retention → ${next}`);
    }
  } catch {
    /* keep current */
  }
  return cachePolicyState;
}

function cachePolicy(): CachePolicy {
  return cachePolicyState;
}
const AVATAR_CONCURRENCY = 3;
const MEDIA_CONCURRENCY = 1;
const PREVIEW_CONCURRENCY = 2;
const PREVIEW_TOP = 28;
const MEDIA_FIRST_BATCH = 5;
const THUMB_MAX = 440;
const THUMB_BYTES = 220_000;
/** Treat cached images at or below this as LINE previews, not full originals. */
const PREVIEW_MAX_EDGE = 512;

const _err = console.error.bind(console);
for (const level of ["log", "info", "debug", "warn"] as const) {
  console[level] = (...args: unknown[]) => _err(`[linejs:${level}]`, ...args);
}

const textEnc = new TextEncoder();
/** False after GTK closes the sidecar pipe (logout / quit / restart). */
let stdoutAlive = true;

function emit(obj: Json) {
  if (!stdoutAlive) return;
  try {
    Deno.stdout.writeSync(textEnc.encode(`${JSON.stringify(obj)}\n`));
  } catch (e) {
    const code = (e as { code?: string })?.code;
    const msg = e instanceof Error ? e.message : String(e);
    if (code === "EPIPE" || msg.includes("Broken pipe")) {
      stdoutAlive = false;
      return;
    }
    throw e;
  }
}
function emitEvent(event: string, payload: Json = {}) {
  emit({ event, ...payload });
}
function ok(id: number | string | null, result: unknown = {}) {
  emit({ id, ok: true, result });
}
function fail(id: number | string | null, error: string) {
  emit({ id, ok: false, error });
}

// linejs listen() async loops can reject without a caller catch (e.g. logged-out token).
let authDead = false;
globalThis.addEventListener("unhandledrejection", (ev) => {
  const reason = (ev as PromiseRejectionEvent).reason;
  let msg = "";
  if (reason instanceof Error) {
    msg = reason.message;
  } else if (typeof reason === "string") {
    msg = reason;
  } else {
    try {
      msg = JSON.stringify(reason ?? "");
    } catch {
      msg = String(reason ?? "unhandledrejection");
    }
  }
  const loggedOut =
    msg.includes("NOT_AUTHORIZED") ||
    msg.includes("LOGGED_OUT") ||
    msg.includes("AUTHENTICATION") ||
    msg.includes("V3_TOKEN_CLIENT_LOGGED_OUT") ||
    msg.includes("NOT_AUTHORIZED_DEVICE");
  if (loggedOut) {
    ev.preventDefault();
    if (authDead) return;
    authDead = true;
    listening = false;
    client = null;
    cancelBackgroundWork();
    console.error("[auth] session ended:", msg);
    void clearAuth();
    emitEvent("session_failed", { error: msg });
    return;
  }
  console.error("[unhandledrejection]", msg);
});

let client: Client | null = null;
let listening = false;

let chatCache: { at: number; chats: ChatRow[] } | null = null;
const msgCache = new Map<string, { at: number; messages: Json[] }>();
const inFlight = new Map<string, Promise<unknown>>();
const contactIndex = new Map<
  string,
  { name: string; picturePath: string | null; kind: string; muted: boolean }
>();
const mediaEpoch = new Map<string, number>();
type BoxCursor = {
  messageId: bigint | number;
  deliveredTime: bigint | number;
};
const boxCursor = new Map<string, BoxCursor>();

function asI64(v: unknown): bigint | number {
  if (typeof v === "bigint") return v;
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (typeof v === "string" && /^-?\d+$/.test(v)) {
    try {
      const n = Number(v);
      if (Number.isSafeInteger(n)) return n;
      return BigInt(v);
    } catch {
      return Number(v);
    }
  }
  // last resort — thrift I64 must be numeric
  if (typeof v === "object" && v !== null && typeof (v as { toString?: () => string }).toString === "function") {
    const s = String(v);
    if (/^-?\d+$/.test(s)) {
      try {
        return BigInt(s);
      } catch {
        return Number(s);
      }
    }
  }
  throw new TypeError(`cannot coerce thrift i64 from ${typeof v}: ${v}`);
}

function storeBoxCursor(
  mid: string,
  last?: { messageId?: unknown; deliveredTime?: unknown } | null,
) {
  if (last?.messageId == null || last?.deliveredTime == null) return;
  try {
    boxCursor.set(mid, {
      messageId: asI64(last.messageId),
      deliveredTime: asI64(last.deliveredTime),
    });
  } catch (e) {
    console.error("[boxCursor]", mid, e);
  }
}

function bumpMediaEpoch(chatMid: string): number {
  const n = (mediaEpoch.get(chatMid) ?? 0) + 1;
  mediaEpoch.set(chatMid, n);
  return n;
}

/** Bumped to cancel hydrateMedia / hydratePreviews after logout / token kick. */
let workGen = 0;
function cancelBackgroundWork() {
  workGen++;
  for (const mid of [...mediaEpoch.keys()]) {
    bumpMediaEpoch(mid);
  }
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

function normType(t: unknown): string {
  if (typeof t === "number") {
    // Matches @evex/linejs-types ContentType enum.
    const map: Record<number, string> = {
      0: "NONE",
      1: "IMAGE",
      2: "VIDEO",
      3: "AUDIO",
      4: "HTML",
      5: "PDF",
      6: "CALL",
      7: "STICKER",
      8: "PRESENCE",
      9: "GIFT",
      10: "GROUPBOARD",
      11: "APPLINK",
      12: "LINK",
      13: "CONTACT",
      14: "FILE",
      15: "LOCATION",
      16: "POSTNOTIFICATION",
      17: "RICH",
      18: "CHATEVENT",
      19: "MUSIC",
      20: "PAYMENT",
      21: "EXTIMAGE",
      22: "FLEX",
    };
    return map[t] ?? String(t);
  }
  return String(t ?? "NONE").toUpperCase();
}

function isVisualType(ct: string) {
  return ct === "IMAGE" || ct === "VIDEO" || ct === "STICKER";
}

function isMediaType(ct: string) {
  return isVisualType(ct) || ct === "AUDIO" || ct === "FILE";
}

/** LINE sometimes omits contentMetadata; linejs crashes on `.e2eeVersion`. */
function normalizeRaw(raw: Record<string, unknown> | null | undefined): Record<string, unknown> {
  const r = { ...(raw ?? {}) };
  if (r.contentMetadata == null || typeof r.contentMetadata !== "object") {
    r.contentMetadata = {};
  }
  return r;
}

async function fromRawTalkSafe(raw: unknown): Promise<TalkMessage> {
  const normalized = normalizeRaw(raw as Record<string, unknown>);
  return await TalkMessage.fromRawTalk(normalized as Parameters<typeof TalkMessage.fromRawTalk>[0], client!);
}

function patchE2eeGuards() {
  if (!client) return;
  const e2ee = client.base.e2ee as {
    decryptE2EEMessage: (m: Record<string, unknown>) => Promise<Record<string, unknown>>;
  };
  const orig = e2ee.decryptE2EEMessage.bind(e2ee);
  e2ee.decryptE2EEMessage = async (messageObj) => {
    const msg = normalizeRaw(messageObj);
    try {
      return await orig(msg);
    } catch (e) {
      console.error("[e2ee decrypt fallback]", e);
      return msg;
    }
  };
}

function stickerUrls(stkId: string): string[] {
  return [
    `https://stickershop.line-scdn.net/stickershop/v1/sticker/${stkId}/android/sticker.png`,
    `https://stickershop.line-scdn.net/stickershop/v1/sticker/${stkId}/iPhone/sticker@2x.png`,
    `https://stickershop.line-scdn.net/stickershop/v1/sticker/${stkId}/ANDROID/sticker.png`,
  ];
}

/** APNG animated stickers (mobile plays these; static PNG is the fallback). */
function stickerAnimationUrls(stkId: string): string[] {
  return [
    `https://stickershop.line-scdn.net/stickershop/v1/sticker/${stkId}/android/sticker_animation.png`,
    `https://stickershop.line-scdn.net/stickershop/v1/sticker/${stkId}/ANDROID/sticker_animation.png`,
    `https://stickershop.line-scdn.net/stickershop/v1/sticker/${stkId}/iPhone/sticker_animation@2x.png`,
  ];
}

type StickerEntry = {
  stickerId: string;
  packageId: string;
  version?: string;
  at: number;
};

const stickerIndexPath = join(dataDir, "stickers-index.json");
let stickerIndex: StickerEntry[] = [];

async function loadStickerIndex() {
  try {
    const raw = JSON.parse(await Deno.readTextFile(stickerIndexPath));
    if (Array.isArray(raw)) {
      stickerIndex = raw
        .filter((x) => x && x.stickerId && x.packageId)
        .map((x) => ({
          stickerId: String(x.stickerId),
          packageId: String(x.packageId),
          version: x.version ? String(x.version) : undefined,
          at: Number(x.at ?? 0),
        }));
    }
  } catch {
    stickerIndex = [];
  }
}

async function saveStickerIndex() {
  try {
    await Deno.writeTextFile(
      stickerIndexPath,
      JSON.stringify(stickerIndex.slice(0, 200), null, 2),
    );
  } catch (e) {
    console.error("[stickers-index]", e);
  }
}

function rememberSticker(
  stickerId: string,
  packageId: string,
  version?: string,
) {
  if (!stickerId || !packageId) return;
  stickerIndex = stickerIndex.filter((s) => s.stickerId !== stickerId);
  stickerIndex.unshift({
    stickerId,
    packageId,
    version: version || undefined,
    at: Date.now(),
  });
  if (stickerIndex.length > 200) stickerIndex.length = 200;
  void saveStickerIndex();
}

function forgetSticker(stickerId: string, packageId?: string) {
  stickerIndex = stickerIndex.filter((s) => {
    if (packageId) return !(s.stickerId === stickerId && s.packageId === packageId);
    return s.stickerId !== stickerId;
  });
  void saveStickerIndex();
}

type OwnedPackage = { id: string; version: string; name: string };
let ownedPackagesCache: { at: number; packages: OwnedPackage[] } | null = null;

async function fetchOwnedPackages(): Promise<OwnedPackage[]> {
  if (!client) return [];
  await refreshCachePolicy();
  if (
    ownedPackagesCache &&
    Date.now() - ownedPackagesCache.at < cachePolicy().ownedPack
  ) {
    return ownedPackagesCache.packages;
  }
  try {
    const { ShopService } = await import(
      "https://jsr.io/@evex/linejs/3.2.1/base/service/shop/mod.ts"
    );
    const shop = new ShopService(client.base as never);
    const res = await shop.getOwnedProductSummaries({
      shopId: "stickershop",
      offset: 0,
      limit: 80,
      locale: { language: "en", country: "TH" },
    } as never) as Record<string, unknown>;
    const list = (res.productList ?? res["1"] ?? []) as Record<string, unknown>[];
    const packages = list
      .map((p) => ({
        id: String(p.id ?? p["1"] ?? ""),
        version: String(p.latestVersion ?? p["21"] ?? "1"),
        name: String(p.name ?? p["11"] ?? ""),
      }))
      .filter((p) => p.id);
    ownedPackagesCache = { at: Date.now(), packages };
    return packages;
  } catch (e) {
    console.error("[owned stickers]", e);
    return ownedPackagesCache?.packages ?? [];
  }
}

async function stickersForOwnedPackage(
  pkg: OwnedPackage,
  limit = 40,
): Promise<{
  name: string;
  version: string;
  stickers: StickerEntry[];
}> {
  try {
    const url =
      `https://stickershop.line-scdn.net/stickershop/v1/product/${pkg.id}/android/productInfo.meta`;
    const r = await fetch(url);
    if (!r.ok) {
      return { name: pkg.name || pkg.id, version: pkg.version, stickers: [] };
    }
    const meta = await r.json() as {
      stickers?: { id?: number | string }[];
      title?: Record<string, string> | string;
      version?: number | string;
    };
    const lang = (Deno.env.get("LINE_GTK_LANG") || "th").toLowerCase();
    const preferEn = lang === "en" || lang === "eng";
    let name = pkg.name || pkg.id;
    if (typeof meta.title === "string" && meta.title.trim()) {
      name = meta.title.trim();
    } else if (meta.title && typeof meta.title === "object") {
      name = String(
        (preferEn
          ? meta.title.en
          : meta.title.th || meta.title.en || meta.title.ja) ||
          meta.title.en ||
          meta.title.ja ||
          name,
      );
    }
    const version = String(meta.version ?? pkg.version ?? "1");
    const stickers = (Array.isArray(meta.stickers) ? meta.stickers : [])
      .slice(0, limit)
      .map((s) => ({
        stickerId: String(s.id ?? ""),
        packageId: pkg.id,
        version,
        at: 0,
      }))
      .filter((s) => s.stickerId);
    return { name, version, stickers };
  } catch (e) {
    console.error("[sticker pack meta]", pkg.id, e);
    return { name: pkg.name || pkg.id, version: pkg.version, stickers: [] };
  }
}

function packIconUrls(packageId: string): string[] {
  return [
    `https://stickershop.line-scdn.net/stickershop/v1/product/${packageId}/android/main.png`,
    `https://stickershop.line-scdn.net/stickershop/v1/product/${packageId}/ANDROID/main.png`,
    `https://stickershop.line-scdn.net/stickershop/v1/product/${packageId}/android/tab_on.png`,
  ];
}

async function ensurePackIcon(packageId: string): Promise<string | null> {
  const dest = join(stickerDir, `pack-${packageId}.png`);
  const existing = await existingImage(dest);
  if (existing) return existing;
  for (const url of packIconUrls(packageId)) {
    const path = await cacheUrl(url, dest);
    if (path) return path;
  }
  return null;
}

async function ensureStickerAnimation(stickerId: string): Promise<string | null> {
  await refreshCachePolicy();
  const animDest = join(stickerDir, `${stickerId}.anim.png`);
  const missDest = join(stickerDir, `${stickerId}.anim.miss`);
  const hit = await existingImage(animDest);
  if (hit) return hit;
  try {
    const st = await Deno.stat(missDest);
    const missTtl = cachePolicy().animMiss;
    const age = Date.now() - Number(st.mtime?.getTime?.() ?? 0);
    if (missTtl === 0 || age < missTtl) {
      return null;
    }
    await Deno.remove(missDest);
  } catch {
    /* not marked missing yet */
  }
  for (const url of stickerAnimationUrls(stickerId)) {
    const path = await cacheUrl(url, animDest);
    if (path) return path;
  }
  try {
    await Deno.writeTextFile(missDest, "");
  } catch {
    /* ignore */
  }
  return null;
}

async function ensureStickerStatic(stickerId: string): Promise<string | null> {
  const dest = join(stickerDir, `${stickerId}.png`);
  const existing = await existingImage(dest);
  if (existing) return existing;
  for (const url of stickerUrls(stickerId)) {
    const path = await cacheUrl(url, dest);
    if (path) return path;
  }
  return null;
}

/** Prefer APNG when the pack is animated; otherwise static PNG. */
async function ensureStickerImage(stickerId: string): Promise<string | null> {
  const anim = await ensureStickerAnimation(stickerId);
  if (anim) return anim;
  return await ensureStickerStatic(stickerId);
}

await loadStickerIndex();

function previewBody(msg: Json): string {
  const ct = normType(msg.contentType);
  const text = String(msg.text ?? "").trim();
  const L = previewLang();
  if (ct === "IMAGE") return L.photo;
  if (ct === "VIDEO") return L.video;
  if (ct === "STICKER") return L.sticker;
  if (ct === "AUDIO") return L.voice;
  if (ct === "FILE") return L.file;
  if (ct === "FLEX") return text || L.flex;
  if (!text) return ct !== "NONE" ? ct.toLowerCase() : L.message;
  return text.length > 64 ? `${text.slice(0, 63)}…` : text;
}

function previewLang() {
  const code = (Deno.env.get("LINE_GTK_LANG") || "th").toLowerCase();
  const th = code === "th" || code === "thai";
  return th
    ? {
      you: "คุณ",
      they: "เขา",
      photo: "รูปภาพ",
      video: "วิดีโอ",
      sticker: "สติกเกอร์",
      voice: "ข้อความเสียง",
      file: "ไฟล์",
      flex: "ข้อความ Flex",
      message: "ข้อความ",
      tap: "แตะเพื่อเปิด",
    }
    : {
      you: "You",
      they: "They",
      photo: "Photo",
      video: "Video",
      sticker: "Sticker",
      voice: "Voice message",
      file: "File",
      flex: "Flex message",
      message: "Message",
      tap: "Tap to open",
    };
}

function previewLine(msg: Json): string {
  const L = previewLang();
  const who = msg.mine ? L.you : L.they;
  return `${who}: ${previewBody(msg)}`;
}

function extractFlex(meta: Record<string, string>): Json | null {
  const altText = meta.ALT_TEXT || "";
  let root: unknown = null;
  if (meta.FLEX_JSON) {
    try {
      root = JSON.parse(meta.FLEX_JSON);
    } catch {
      root = null;
    }
  }
  const texts: string[] = [];
  const actions: Json[] = [];
  const seen = new Set<string>();

  const walk = (node: unknown, depth = 0) => {
    if (!node || depth > 12) return;
    if (Array.isArray(node)) {
      for (const n of node) walk(n, depth + 1);
      return;
    }
    if (typeof node !== "object") return;
    const o = node as Record<string, unknown>;
    if (typeof o.text === "string" && o.text.trim()) {
      const t = o.text.trim();
      if (texts.length < 12 && !texts.includes(t)) texts.push(t);
    }
    if (o.action && typeof o.action === "object") {
      const a = o.action as Record<string, unknown>;
      const label = String(a.label || o.text || a.data || a.uri || "Action");
      const kind = String(a.type || "postback").toLowerCase();
      const data = a.data != null ? String(a.data) : null;
      const uri = a.uri != null ? String(a.uri) : (a.url != null ? String(a.url) : null);
      const key = `${kind}|${label}|${data || ""}|${uri || ""}`;
      if (!seen.has(key) && actions.length < 24) {
        seen.add(key);
        actions.push({ label, kind, data, uri });
      }
    }
    for (const v of Object.values(o)) walk(v, depth + 1);
  };
  walk(root);

  if (!altText && !texts.length && !actions.length) return null;
  return {
    altText: altText || texts[0] || "Flex message",
    texts: texts.slice(0, 8),
    actions,
  };
}

/** Serve disk-cached stickers/thumbs immediately; only hydrate misses. */
async function forUiMessages(messages: Json[]): Promise<Json[]> {
  return await Promise.all(messages.map(async (m) => {
    const ct = normType(m.contentType);
    const id = String(m.id ?? "");
    let imagePath: string | null = (m.imagePath as string) || null;
    let audioPath: string | null = (m.audioPath as string) || null;
    let needsMedia = isMediaType(ct) && !imagePath && !audioPath;

    if (ct === "STICKER") {
      const stkId = String(m.stickerId || "");
      if (stkId) {
        const anim = await existingImage(join(stickerDir, `${stkId}.anim.png`));
        const st = await existingImage(join(stickerDir, `${stkId}.png`));
        if (anim) {
          imagePath = anim;
          needsMedia = false;
        } else if (st) {
          // Show static now; hydrate may upgrade to APNG.
          imagePath = st;
          needsMedia = true;
        }
      }
    } else if (ct === "IMAGE" || ct === "VIDEO") {
      const cached = await existingImage(thumbDest(id)) ||
        await existingImage(mediaDest(id)) ||
        await existingImage(mediaDest(id, "png"));
      if (cached) {
        imagePath = (await uiMediaPath(id, cached)) || cached;
        needsMedia = false;
      }
    } else if (ct === "AUDIO") {
      audioPath = null;
      for (const ext of ["m4a", "mp3", "aac", "ogg"]) {
        const p = mediaDest(id, ext);
        if (await isValidAudioFile(p)) {
          audioPath = p;
          break;
        }
      }
      if (audioPath) needsMedia = false;
      else needsMedia = true;
    } else if (ct === "FILE") {
      // Filename-only bubble; download is on demand / background, not a preview.
      needsMedia = false;
    }

    return { ...m, imagePath, audioPath, needsMedia };
  }));
}

async function refetchMessageData(
  chatMid: string,
  messageId: string,
  opts: { allowNonImage?: boolean } = {},
): Promise<{ buf: Uint8Array; mime: string } | null> {
  if (!client) return null;
  let cursor = boxCursor.get(chatMid);
  if (!cursor) {
    try {
      const boxes = await client.base.talk.getMessageBoxes({
        messageBoxListRequest: {},
      });
      for (const box of boxes.messageBoxes ?? []) {
        storeBoxCursor(String(box.id ?? ""), box.lastDeliveredMessageId);
      }
      cursor = boxCursor.get(chatMid);
    } catch {
      return null;
    }
  }
  if (!cursor) return null;
  try {
    const messages = await client.base.talk.getPreviousMessagesV2WithRequest({
      request: {
        messageBoxId: chatMid,
        endMessageId: {
          messageId: cursor.messageId,
          deliveredTime: cursor.deliveredTime,
        },
        messagesCount: 100,
      },
    });
    const raw = messages.find((m) => String(m.id) === messageId);
    if (!raw) return null;
    const tm = await fromRawTalkSafe(raw);
    const blob = await tm.getData(false);
    const buf = new Uint8Array(await blob.arrayBuffer());
    if (!opts.allowNonImage && !isImageBytes(buf)) return null;
    return { buf, mime: blob.type || "application/octet-stream" };
  } catch (e) {
    console.error("[refetchMessageData]", messageId, e);
    return null;
  }
}

async function dedupe<T>(key: string, fn: () => Promise<T>): Promise<T> {
  const existing = inFlight.get(key);
  if (existing) return existing as Promise<T>;
  const p = fn().finally(() => inFlight.delete(key));
  inFlight.set(key, p);
  return p;
}

async function mapPool<T, R>(
  items: T[],
  concurrency: number,
  worker: (item: T, i: number) => Promise<R>,
): Promise<R[]> {
  const out = new Array<R>(items.length);
  let next = 0;
  async function run() {
    while (next < items.length) {
      const i = next++;
      out[i] = await worker(items[i], i);
    }
  }
  await Promise.all(
    Array.from({ length: Math.min(concurrency, Math.max(items.length, 1)) }, () => run()),
  );
  return out;
}

async function saveAuth(token: string, device = LINE_DEVICE) {
  await Deno.writeTextFile(authPath, token);
  await Deno.writeTextFile(authDevicePath, device);
}
async function loadAuth(): Promise<string | null> {
  try {
    const t = (await Deno.readTextFile(authPath)).trim();
    return t.length ? t : null;
  } catch {
    return null;
  }
}
async function loadAuthDevice(): Promise<string> {
  try {
    const d = (await Deno.readTextFile(authDevicePath)).trim();
    if (d) return d;
  } catch { /* miss */ }
  return "DESKTOPWIN"; // legacy installs before Android call migration
}
async function clearAuth() {
  for (const p of [authPath, authDevicePath]) {
    try {
      await Deno.remove(p);
    } catch { /* ignore */ }
  }
}

function myMid(): string {
  try {
    return client?.base?.profile?.mid ?? "";
  } catch {
    return "";
  }
}

/** Prefer raw mid compare — TalkMessage.isMyMessage crashes if client was cleared. */
function isMineFrom(from: unknown): boolean {
  const mid = myMid();
  const f = String(from ?? "");
  return !!mid && !!f && f === mid;
}

function friendName(user: { mid: string; raw: { contact?: { displayName?: string; picturePath?: string } } }) {
  return user.raw.contact?.displayName || user.mid;
}

function profileUrl(picturePath?: string | null): string | null {
  if (!picturePath) return null;
  if (picturePath.startsWith("http")) return picturePath;
  return `https://profile.line-scdn.net${picturePath.startsWith("/") ? "" : "/"}${picturePath}`;
}

function picturePathOf(profile: Record<string, unknown> | null | undefined): string | null {
  if (!profile) return null;
  const candidates: unknown[] = [
    profile.picturePath,
    profile.pictureStatus,
    (profile as { picture?: { path?: string } }).picture?.path,
    (profile as { raw?: { contact?: { picturePath?: string } } }).raw?.contact
      ?.picturePath,
  ];
  for (const raw of candidates) {
    if (typeof raw === "string" && raw.trim()) return raw.trim();
  }
  return null;
}

async function myProfilePayload(profile: {
  mid: string;
  displayName?: string;
  statusMessage?: string;
  [k: string]: unknown;
}) {
  let picturePath = picturePathOf(profile as Record<string, unknown>);
  if (!picturePath) {
    picturePath = contactIndex.get(profile.mid)?.picturePath ?? null;
  }
  const pictureUrl = profileUrl(picturePath);
  let avatarPath: string | null = null;
  try {
    avatarPath = await avatarPathFor(profile.mid, picturePath);
  } catch (e) {
    console.error("[myProfile avatar]", e);
  }
  if (avatarPath) {
    emitEvent("avatar_ready", { mid: profile.mid, avatarPath });
  }
  return {
    mid: profile.mid,
    displayName: profile.displayName ?? "",
    statusMessage: profile.statusMessage ?? "",
    picturePath,
    avatarPath,
    pictureUrl,
  };
}

async function existingFile(path: string): Promise<string | null> {
  try {
    const st = await Deno.stat(path);
    return st.isFile && st.size > 32 ? path : null;
  } catch {
    return null;
  }
}

function isImageBytes(buf: Uint8Array): boolean {
  if (buf.length < 12) return false;
  // JPEG
  if (buf[0] === 0xff && buf[1] === 0xd8 && buf[2] === 0xff) return true;
  // PNG
  if (buf[0] === 0x89 && buf[1] === 0x50 && buf[2] === 0x4e && buf[3] === 0x47) {
    return true;
  }
  // GIF
  if (buf[0] === 0x47 && buf[1] === 0x49 && buf[2] === 0x46) return true;
  // WEBP (RIFF....WEBP)
  if (
    buf[0] === 0x52 && buf[1] === 0x49 && buf[2] === 0x46 && buf[3] === 0x46 &&
    buf[8] === 0x57 && buf[9] === 0x45 && buf[10] === 0x42 && buf[11] === 0x50
  ) {
    return true;
  }
  return false;
}

async function isImageFile(path: string): Promise<boolean> {
  try {
    const file = await Deno.open(path, { read: true });
    const buf = new Uint8Array(16);
    const n = await file.read(buf);
    file.close();
    if (!n || n < 4) return false;
    return isImageBytes(buf);
  } catch {
    return false;
  }
}

async function existingImage(path: string): Promise<string | null> {
  const hit = await existingFile(path);
  if (!hit) return null;
  if (await isImageFile(hit)) return hit;
  try {
    await Deno.remove(hit);
  } catch { /* ignore corrupt/html leftovers */ }
  return null;
}

async function writeImageFile(dest: string, buf: Uint8Array): Promise<string | null> {
  if (!isImageBytes(buf)) return null;
  await Deno.writeFile(dest, buf);
  return dest;
}

async function cacheUrl(url: string, dest: string): Promise<string | null> {
  const hit = await existingImage(dest);
  if (hit) return hit;
  try {
    const res = await fetch(url);
    if (!res.ok) return null;
    const ctype = (res.headers.get("content-type") || "").toLowerCase();
    if (ctype && !ctype.includes("image") && !ctype.includes("octet-stream")) {
      return null;
    }
    const buf = new Uint8Array(await res.arrayBuffer());
    return await writeImageFile(dest, buf);
  } catch (e) {
    console.error("[cacheUrl]", e);
    return null;
  }
}

async function avatarPathFor(mid: string, picturePath?: string | null): Promise<string | null> {
  const dest = join(avatarDir, `${mid}.jpg`);
  const hit = await existingImage(dest);
  if (hit) return hit;
  const url = profileUrl(picturePath);
  if (!url) return null;
  return await cacheUrl(url, dest);
}

function mediaDest(id: string, ext = "jpg") {
  return join(mediaDir, `${id}.${ext}`);
}

function fullDest(id: string, ext = "jpg") {
  return join(mediaDir, `${id}.full.${ext}`);
}

function thumbDest(id: string) {
  return join(mediaDir, `${id}.thumb.jpg`);
}

async function imageDimensions(
  path: string,
): Promise<{ w: number; h: number } | null> {
  try {
    const code = `
from PIL import Image
im=Image.open(${JSON.stringify(path)})
print(im.size[0], im.size[1])
`;
    const p = new Deno.Command("python3", {
      args: ["-c", code],
      stdout: "piped",
      stderr: "null",
    });
    const { success, stdout } = await p.output();
    if (!success) return null;
    const text = new TextDecoder().decode(stdout).trim();
    const [ws, hs] = text.split(/\s+/);
    const w = Number(ws);
    const h = Number(hs);
    if (!Number.isFinite(w) || !Number.isFinite(h) || w < 1 || h < 1) return null;
    return { w, h };
  } catch {
    return null;
  }
}

/** True when the file is a UI thumb / OBS preview, not a full original. */
async function isPreviewOrThumbImage(path: string): Promise<boolean> {
  if (path.includes(".thumb.")) return true;
  // Explicit full originals are trusted even when the photo itself is small.
  if (path.includes(".full.")) return false;
  try {
    const st = await Deno.stat(path);
    if (st.size < 2_000) return true;
    if (st.size >= 400_000) return false;
  } catch {
    return true;
  }
  const dims = await imageDimensions(path);
  if (!dims) {
    try {
      const st = await Deno.stat(path);
      return st.size < THUMB_BYTES;
    } catch {
      return true;
    }
  }
  return Math.max(dims.w, dims.h) <= PREVIEW_MAX_EDGE;
}

async function existingFullImage(id: string): Promise<string | null> {
  for (const ext of ["jpg", "jpeg", "png", "webp", "gif"]) {
    const preferred = await existingImage(fullDest(id, ext));
    if (preferred) return preferred;
  }
  // Legacy slots may be polluted with OBS preview — filter those out.
  for (const ext of ["jpg", "jpeg", "png", "webp", "gif"]) {
    const p = await existingImage(mediaDest(id, ext));
    if (!p) continue;
    if (await isPreviewOrThumbImage(p)) continue;
    try {
      const dest = fullDest(id, ext === "jpeg" ? "jpg" : ext);
      await Deno.copyFile(p, dest);
      return dest;
    } catch {
      return p;
    }
  }
  const bare = await existingImage(mediaDest(id));
  if (bare && !(await isPreviewOrThumbImage(bare))) {
    try {
      const dest = fullDest(id, "jpg");
      await Deno.copyFile(bare, dest);
      return dest;
    } catch {
      return bare;
    }
  }
  return null;
}

async function writeFullImage(
  id: string,
  buf: Uint8Array,
  mimeOrExt?: string,
): Promise<string | null> {
  if (!isImageBytes(buf)) return null;
  const ext = (mimeOrExt || "").includes("png") || buf[0] === 0x89
    ? "png"
    : (mimeOrExt || "").includes("webp")
    ? "webp"
    : (mimeOrExt || "").includes("gif") || (buf[0] === 0x47 && buf[1] === 0x49)
    ? "gif"
    : "jpg";
  return await writeImageFile(fullDest(id, ext), buf);
}

async function makeThumb(src: string, id: string): Promise<string | null> {
  const dest = thumbDest(id);
  const hit = await existingImage(dest);
  if (hit) return hit;
  if (!(await isImageFile(src))) return null;
  try {
    const st = await Deno.stat(src);
    if (st.size > 0 && st.size < THUMB_BYTES) {
      const dims = await imageDimensions(src);
      if (dims && Math.max(dims.w, dims.h) <= THUMB_MAX) return src;
    }
  } catch {
    return null;
  }
  try {
    const code = `
from PIL import Image
src=${JSON.stringify(src)}
dest=${JSON.stringify(dest)}
im=Image.open(src)
im.thumbnail((${THUMB_MAX},${THUMB_MAX}))
if im.mode not in ("RGB","L"):
    im=im.convert("RGB")
im.save(dest, "JPEG", quality=80, optimize=True)
`;
    const p = new Deno.Command("python3", {
      args: ["-c", code],
      stdout: "null",
      stderr: "null",
    });
    const { success } = await p.output();
    if (success) return await existingFile(dest);
  } catch (e) {
    console.error("[makeThumb]", id, e);
  }
  return null;
}

/** Prefer a small thumb for UI; keep original on disk for cache. */
async function uiMediaPath(id: string, original: string | null): Promise<string | null> {
  if (!original) return null;
  const thumb = await makeThumb(original, id);
  return thumb || original;
}

async function isValidAudioFile(path: string | null): Promise<boolean> {
  if (!path) return false;
  try {
    const st = await Deno.stat(path);
    // Corrupt/empty AAC shells from killed ffmpeg / failed OBS are ~44 bytes.
    if (st.size < 1024) return false;
    const buf = new Uint8Array(16);
    const f = await Deno.open(path, { read: true });
    try {
      await f.read(buf);
    } finally {
      f.close();
    }
    const head = new TextDecoder().decode(buf);
    // Reject truncated ftyp-only shells without moov.
    if (head.includes("ftyp") && st.size < 2048) {
      // Still might be tiny valid; probe with ffprobe if available.
    }
    const p = new Deno.Command("ffprobe", {
      args: ["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path],
      stdout: "piped",
      stderr: "null",
    }).spawn();
    const out = await p.output();
    if (!out.success) return false;
    const dur = Number(new TextDecoder().decode(out.stdout).trim());
    return Number.isFinite(dur) && dur > 0;
  } catch {
    return false;
  }
}

async function downloadAudioBytes(messageId: string, chatMid: string): Promise<{
  buf: Uint8Array;
  mime: string;
} | null> {
  // Prefer TalkMessage.getData (E2EE-aware).
  const got = await refetchMessageData(chatMid, messageId, { allowNonImage: true });
  if (got && got.buf.length >= 1024) return got;
  if (!client) return null;
  try {
    const file = await client.base.obs.downloadMessageData({
      messageId,
      isPreview: false,
      isSquare: false,
    });
    const buf = new Uint8Array(await file.arrayBuffer());
    if (buf.length >= 1024) {
      return { buf, mime: file.type || "audio/mp4" };
    }
  } catch (e) {
    console.error("[audio obs]", messageId, e);
  }
  return got && got.buf.length > 0 ? got : null;
}

async function loadDiskContacts() {
  if (contactIndex.size > 0) return;
  try {
    const raw = JSON.parse(await Deno.readTextFile(contactCachePath));
    if (!raw?.contacts) return;
    for (const [mid, c] of Object.entries(raw.contacts as Record<string, {
      name: string;
      picturePath: string | null;
      kind?: string;
      muted?: boolean;
    }>)) {
      contactIndex.set(mid, {
        name: c.name,
        picturePath: c.picturePath ?? null,
        kind: c.kind || "dm",
        muted: !!c.muted,
      });
    }
    const at = Number(raw.at || 0);
    if (Number.isFinite(at) && at > 0) contactsAt = at;
  } catch { /* miss */ }
}

async function saveDiskContacts() {
  const contacts: Record<
    string,
    { name: string; picturePath: string | null; kind: string; muted: boolean }
  > = {};
  for (const [mid, c] of contactIndex) contacts[mid] = c;
  try {
    await Deno.writeTextFile(
      contactCachePath,
      JSON.stringify({ at: Date.now(), contacts }),
    );
  } catch { /* ignore */ }
}

async function loadDiskMessages(chatMid: string): Promise<Json[] | null> {
  try {
    await refreshCachePolicy();
    const raw = JSON.parse(
      await Deno.readTextFile(join(msgDiskDir, `${chatMid}.json`)),
    );
    if (Date.now() - Number(raw.at || 0) > cachePolicy().diskMsg) return null;
    if (Array.isArray(raw.messages)) return raw.messages as Json[];
  } catch { /* miss */ }
  return null;
}

async function saveDiskMessages(chatMid: string, messages: Json[]) {
  try {
    await Deno.writeTextFile(
      join(msgDiskDir, `${chatMid}.json`),
      JSON.stringify({ at: Date.now(), messages }),
    );
  } catch { /* ignore */ }
}

let contactsAt = 0;

async function refreshContactIndex(force = false) {
  if (!client) return;
  if (
    !force &&
    Date.now() - contactsAt < cachePolicy().contactsRefresh &&
    contactIndex.size > 0
  ) {
    return;
  }
  const ids: string[] = await client.base.talk.getAllContactIds({});
  // Include open message boxes so bots/OA resolve even if not in the friend list.
  try {
    const boxes = await client.base.talk.getMessageBoxes({
      messageBoxListRequest: {},
    });
    for (const box of boxes.messageBoxes ?? []) {
      const mid = String(box.id ?? "");
      if (mid.startsWith("u") && !ids.includes(mid)) ids.push(mid);
    }
  } catch { /* ignore */ }

  // Chunk to avoid huge thrift payloads
  const chunk = 40;
  for (let i = 0; i < ids.length; i += chunk) {
    const slice = ids.slice(i, i + chunk);
    try {
      const contacts = await client.base.talk.getContacts({ mids: slice });
      for (const c of contacts ?? []) {
        const mid = String(c.mid ?? "");
        if (!mid) continue;
        const type = String(c.type ?? "");
        const kind = /BOT/i.test(type) ? "bot" : "dm";
        contactIndex.set(mid, {
          name: c.displayName || mid,
          picturePath: c.picturePath ?? null,
          kind,
          muted: !!(c as { notificationDisabled?: boolean }).notificationDisabled,
        });
      }
    } catch (e) {
      console.error("[contacts chunk]", e);
      for (const mid of slice) {
        try {
          const c = await client.base.talk.getContact({ mid });
          const type = String(c.type ?? "");
          contactIndex.set(mid, {
            name: c.displayName || mid,
            picturePath: c.picturePath ?? null,
            kind: /BOT/i.test(type) ? "bot" : "dm",
            muted: !!(c as { notificationDisabled?: boolean }).notificationDisabled,
          });
        } catch {
          /* bot/OA without contact entry */
        }
      }
    }
  }
  contactsAt = Date.now();
  await saveDiskContacts();
}

async function summarizeTalkMessage(
  tm: TalkMessage,
  opts: { withMedia?: boolean } = {},
): Promise<Json> {
  const withMedia = opts.withMedia === true;
  const raw = tm.raw as {
    id?: string | number;
    from?: string;
    to?: string;
    createdTime?: number | string | bigint;
    contentType?: string | number;
    contentMetadata?: Record<string, string>;
  };
  const contentType = normType(raw.contentType);
  const meta = raw.contentMetadata ?? {};
  let text = tm.text || meta.ALT_TEXT || meta.STKTXT || "";
  let imagePath: string | null = null;
  let imageUrl: string | null = meta.PREVIEW_URL || meta.DOWNLOAD_URL || null;
  const id = String(raw.id ?? "");
  const stkId = meta.STKID || "";
  const stkPkg = meta.STKPKGID || "";

  if (contentType === "AUDIO") {
    text = text || "Voice message";
    const duration = meta.DURATION || meta.AUDLEN || "";
    let audioPath: string | null = null;
    if (withMedia) {
      const candidates = [
        mediaDest(id, "m4a"),
        mediaDest(id, "mp3"),
        mediaDest(id, "aac"),
        mediaDest(id, "ogg"),
      ];
      for (const p of candidates) {
        if (await isValidAudioFile(p)) {
          audioPath = p;
          break;
        }
        try {
          await Deno.remove(p);
        } catch { /* ignore */ }
      }
      if (!audioPath) {
        try {
          const blob = await tm.getData(false);
          const buf = new Uint8Array(await blob.arrayBuffer());
          if (buf.length >= 1024) {
            const mime = blob.type || "";
            const ext = mime.includes("mpeg") || mime.includes("mp3")
              ? "mp3"
              : mime.includes("aac")
              ? "aac"
              : mime.includes("ogg")
              ? "ogg"
              : "m4a";
            const dest = mediaDest(id, ext);
            await Deno.writeFile(dest, buf);
            if (await isValidAudioFile(dest)) audioPath = dest;
            else {
              try {
                await Deno.remove(dest);
              } catch { /* ignore */ }
            }
          }
        } catch (e) {
          console.error("[audio]", id, e);
        }
      }
    }
    const flex = null;
    return {
      id,
      text: duration ? `Voice message (${Math.round(Number(duration) / 1000)}s)` : text,
      from: raw.from ?? "",
      to: raw.to ?? "",
      mine: isMineFrom(raw.from),
      createdTime: Number(raw.createdTime ?? 0),
      contentType,
      imagePath: null,
      imageUrl: null,
      audioPath,
      fileName: null,
      filePath: audioPath,
      stickerId: null,
      flex,
      durationMs: duration ? Number(duration) : null,
      needsMedia: !audioPath,
    };
  }

  let fileName: string | null = meta.FILE_NAME || meta.FILENAME || null;
  let filePath: string | null = null;

  if (contentType === "IMAGE") {
    if (!text) text = "[Image]";
    if (withMedia) {
      const full = await existingFullImage(id);
      if (full) {
        filePath = full;
        imagePath = (await uiMediaPath(id, full)) || full;
      } else {
        try {
          // E2EE: getData ignores preview and returns decrypted original.
          const blob = await tm.getData(false);
          const buf = new Uint8Array(await blob.arrayBuffer());
          if (isImageBytes(buf)) {
            const ext = (blob.type || "").includes("png") || buf[0] === 0x89 ? "png" : "jpg";
            filePath = await writeFullImage(id, buf, blob.type || ext);
            imagePath = (await uiMediaPath(id, filePath)) || filePath;
          }
          if (!filePath && imageUrl) {
            const cached = await cacheUrl(imageUrl, fullDest(id, "jpg"));
            filePath = cached;
            imagePath = (await uiMediaPath(id, filePath)) || filePath;
          }
        } catch (e) {
          console.error("[media image]", id, e);
          if (imageUrl) {
            filePath = await cacheUrl(imageUrl, fullDest(id, "jpg"));
            imagePath = (await uiMediaPath(id, filePath)) || filePath;
          }
        }
      }
      // UI can still show a prior thumb while full download is pending.
      if (!imagePath) {
        imagePath = await existingImage(thumbDest(id));
      }
    }
  } else if (contentType === "VIDEO") {
    if (!text) text = "[Video]";
    fileName = fileName || `${id}.mp4`;
    if (withMedia) {
      const vpath = await existingFile(mediaDest(id, "mp4"));
      if (vpath) filePath = vpath;
      const cached = await existingImage(thumbDest(id)) ||
        await existingImage(mediaDest(id)) ||
        await existingImage(mediaDest(id, "png"));
      if (cached) {
        imagePath = cached;
      } else {
        try {
          const blob = await tm.getData(false);
          const buf = new Uint8Array(await blob.arrayBuffer());
          imagePath = await materializeVideoPreview(id, buf);
          filePath = await existingFile(mediaDest(id, "mp4"));
        } catch (e) {
          console.error("[media video]", id, e);
        }
      }
    }
  } else if (contentType === "FILE") {
    text = text || fileName || "[File]";
    fileName = fileName || text;
    if (withMedia) {
      try {
        for await (const ent of Deno.readDir(mediaDir)) {
          if (
            ent.isFile && ent.name.startsWith(`${id}.`) &&
            !ent.name.includes(".thumb.")
          ) {
            filePath = join(mediaDir, ent.name);
            break;
          }
        }
      } catch { /* ignore */ }
    }
  } else if (contentType === "STICKER") {
    text = text || "[Sticker]";
    if (stkId) {
      imageUrl = stickerAnimationUrls(stkId)[0];
      if (withMedia) {
        imagePath = await ensureStickerImage(stkId);
      }
    }
  } else if (contentType === "FLEX") {
    text = text || meta.ALT_TEXT || "[Flex message]";
  } else if (!text) {
    text = contentType !== "NONE" ? `[${contentType}]` : "";
  }

  const flex = contentType === "FLEX" ? extractFlex(meta) : null;

  return {
    id,
    text,
    from: raw.from ?? "",
    to: raw.to ?? "",
    mine: isMineFrom(raw.from),
    createdTime: Number(raw.createdTime ?? 0),
    contentType,
    imagePath,
    imageUrl,
    audioPath: null,
    fileName,
    filePath,
    stickerId: stkId || null,
    stickerPackageId: stkPkg || null,
    flex,
    durationMs: meta.DURATION ? Number(meta.DURATION) : null,
    needsMedia: (contentType === "IMAGE" || contentType === "VIDEO" || contentType === "STICKER")
      ? !imagePath
      : contentType === "AUDIO"
      ? true
      : false,
  };
}

async function summarizeRawMessage(
  raw: Parameters<typeof TalkMessage.fromRawTalk>[0],
  opts?: { withMedia?: boolean },
) {
  try {
    const tm = await fromRawTalkSafe(raw);
    return await summarizeTalkMessage(tm, opts);
  } catch (e) {
    console.error("[summarizeRawMessage]", e);
    const r = normalizeRaw(raw as Record<string, unknown>);
    const meta = (r.contentMetadata ?? {}) as Record<string, string>;
    const contentType = normType(r.contentType);
    let text = String(r.text ?? meta.ALT_TEXT ?? meta.STKTXT ?? "");
    if (!text) {
      if (contentType === "AUDIO") text = "Voice message";
      else if (contentType === "IMAGE") text = "[Image]";
      else if (contentType === "VIDEO") text = "[Video]";
      else if (contentType === "FILE") text = meta.FILE_NAME || meta.FILENAME || "[File]";
      else if (contentType === "STICKER") text = "[Sticker]";
      else if (contentType !== "NONE") text = `[${contentType}]`;
    }
    return {
      id: String(r.id ?? ""),
      text,
      from: String(r.from ?? ""),
      to: String(r.to ?? ""),
      mine: isMineFrom(r.from),
      createdTime: Number(r.createdTime ?? 0),
      contentType,
      imagePath: null,
      imageUrl: meta.PREVIEW_URL || meta.DOWNLOAD_URL || null,
      audioPath: null,
      fileName: meta.FILE_NAME || meta.FILENAME || null,
      filePath: null,
      stickerId: meta.STKID || null,
      stickerPackageId: meta.STKPKGID || null,
      flex: contentType === "FLEX" ? extractFlex(meta) : null,
      durationMs: meta.DURATION ? Number(meta.DURATION) : null,
      needsMedia: contentType === "IMAGE" || contentType === "VIDEO" ||
        contentType === "STICKER" || contentType === "AUDIO",
    };
  }
}

async function hydrateMedia(messages: Json[], chatMid: string) {
  if (!client || !stdoutAlive) return;
  const epoch = bumpMediaEpoch(chatMid);
  const gen = workGen;
  const pending = messages
    .filter((m) => m.needsMedia && m.id)
    .slice()
    .reverse(); // newest first
  if (!pending.length) return;

  const first = pending.slice(0, MEDIA_FIRST_BATCH);
  const rest = pending.slice(MEDIA_FIRST_BATCH);

  const failOne = (id: string) => {
    if (!stdoutAlive || workGen !== gen) return;
    emitEvent("media_failed", { chatMid, messageId: id });
  };

  const work = async (m: Json) => {
    if (!client || !stdoutAlive || workGen !== gen) return;
    if (mediaEpoch.get(chatMid) !== epoch) return;
    try {
      const id = String(m.id);
      const ct = normType(m.contentType);
      let path: string | null = null;

      if (ct === "STICKER") {
        const stkId = String(m.stickerId || "");
        if (!stkId) {
          failOne(id);
          return;
        }
        if (mediaEpoch.get(chatMid) !== epoch) return;
        path = await ensureStickerImage(stkId);
        if (path) {
          emitEvent("media_ready", { chatMid, messageId: id, imagePath: path });
        } else {
          failOne(id);
        }
        return;
      }

      if (ct === "AUDIO") {
        let audioPath: string | null = null;
        for (const ext of ["m4a", "mp3", "aac", "ogg"]) {
          const p = mediaDest(id, ext);
          if (await isValidAudioFile(p)) {
            audioPath = p;
            break;
          }
          try {
            await Deno.remove(p);
          } catch { /* ignore */ }
        }
        if (!audioPath) {
          const got = await downloadAudioBytes(id, chatMid);
          if (got) {
            const ext = got.mime.includes("mpeg") || got.mime.includes("mp3")
              ? "mp3"
              : got.mime.includes("aac")
              ? "aac"
              : got.mime.includes("ogg")
              ? "ogg"
              : "m4a";
            const dest = mediaDest(id, ext);
            await Deno.writeFile(dest, got.buf);
            if (await isValidAudioFile(dest)) audioPath = dest;
            else {
              try {
                await Deno.remove(dest);
              } catch { /* ignore */ }
            }
          }
        }
        if (audioPath) {
          m.audioPath = audioPath;
          m.needsMedia = false;
          const cached = msgCache.get(chatMid);
          if (cached) {
            const row = cached.messages.find((x) => String(x.id) === id);
            if (row) {
              row.audioPath = audioPath;
              row.needsMedia = false;
            }
          }
          emitEvent("media_ready", {
            chatMid,
            messageId: id,
            imagePath: null,
            audioPath,
          });
        } else {
          failOne(id);
        }
        return;
      }

      if (ct === "FILE") {
        // No bubble preview; stop retry loops.
        m.needsMedia = false;
        const cached = msgCache.get(chatMid);
        if (cached) {
          const row = cached.messages.find((x) => String(x.id) === id);
          if (row) row.needsMedia = false;
        }
        return;
      }

      path = await existingFullImage(id);

      if (!path && typeof m.imageUrl === "string" && m.imageUrl) {
        path = await cacheUrl(m.imageUrl, fullDest(id, "jpg"));
        if (path && (await isPreviewOrThumbImage(path))) {
          try {
            await Deno.copyFile(path, thumbDest(id));
            await Deno.remove(path);
          } catch { /* ignore */ }
          path = null;
        }
      }

      // OBS preview is UI-only — never write it over the full media slot.
      if (!path && client) {
        try {
          const file = await client.base.obs.downloadMessageData({
            messageId: id,
            isPreview: true,
            isSquare: false,
          });
          if (mediaEpoch.get(chatMid) !== epoch) return;
          const buf = new Uint8Array(await file.arrayBuffer());
          if (buf.length > 32 && isImageBytes(buf)) {
            await writeImageFile(thumbDest(id), buf);
          }
        } catch (e) {
          console.error("[hydrateMedia obs preview]", id, e);
        }
      }

      // Full original: TalkMessage.getData (E2EE-aware)
      if (!path) {
        const got = await refetchMessageData(chatMid, id, {
          allowNonImage: ct === "VIDEO",
        });
        if (got) {
          if (ct === "VIDEO") {
            path = await materializeVideoPreview(id, got.buf);
          } else if (isImageBytes(got.buf)) {
            path = await writeFullImage(id, got.buf, got.mime);
          }
        }
      }

      if (path && mediaEpoch.get(chatMid) === epoch) {
        // If hydrate somehow still got a preview, demote it to thumb and keep downloading.
        if (ct !== "VIDEO" && (await isPreviewOrThumbImage(path))) {
          try {
            await Deno.copyFile(path, thumbDest(id));
            if (!path.includes(".thumb.") && !path.includes(".full.")) {
              await Deno.remove(path);
            }
          } catch { /* ignore */ }
          const got = await refetchMessageData(chatMid, id, { allowNonImage: false });
          if (got && isImageBytes(got.buf)) {
            path = await writeFullImage(id, got.buf, got.mime);
          } else {
            path = null;
          }
        }
      }

      if (path && mediaEpoch.get(chatMid) === epoch) {
        const uiPath = await uiMediaPath(id, path);
        m.imagePath = uiPath || path;
        m.needsMedia = false;
        // Keep full-resolution path for viewer/download (ui path may be a thumb).
        if (ct === "VIDEO") {
          const vpath = await existingFile(mediaDest(id, "mp4"));
          if (vpath) m.filePath = vpath;
        } else {
          m.filePath = path.includes(".thumb.") ? null : path;
        }
        const cached = msgCache.get(chatMid);
        if (cached) {
          const row = cached.messages.find((x) => String(x.id) === id);
          if (row) {
            row.imagePath = m.imagePath;
            row.needsMedia = false;
            if (m.filePath) row.filePath = m.filePath;
          }
        }
        emitEvent("media_ready", {
          chatMid,
          messageId: id,
          imagePath: m.imagePath,
          filePath: m.filePath ?? (ct === "VIDEO" ? null : path),
        });
        await sleep(30);
      } else if (mediaEpoch.get(chatMid) === epoch) {
        // Still show thumb in UI if we have one; mark failed only when nothing.
        const thumb = await existingImage(thumbDest(id));
        if (thumb) {
          m.imagePath = thumb;
          m.needsMedia = false;
          emitEvent("media_ready", {
            chatMid,
            messageId: id,
            imagePath: thumb,
            filePath: null,
          });
        } else {
          failOne(id);
        }
      }
    } catch (e) {
      console.error("[hydrateMedia worker]", e);
      try {
        failOne(String(m.id));
      } catch { /* ignore */ }
    }
  };

  await mapPool(first, MEDIA_CONCURRENCY, (m) => work(m));
  if (mediaEpoch.get(chatMid) !== epoch) return;
  await sleep(120);
  await mapPool(rest, MEDIA_CONCURRENCY, (m) => work(m));
}

async function hydratePreviews(chats: ChatRow[]) {
  if (!client || !stdoutAlive) return;
  const gen = workGen;
  const need = chats
    .filter((c) => {
      const L = previewLang();
      return c.lastActivity > 0 &&
        (!c.preview || c.preview === "Tap to open" || c.preview === L.tap);
    })
    .slice(0, PREVIEW_TOP);
  if (!need.length) return;

  await mapPool(need, PREVIEW_CONCURRENCY, async (c) => {
    if (!client || !stdoutAlive || workGen !== gen) return;
    try {
      let cursor = boxCursor.get(c.mid);
      if (!cursor) {
        // refresh cursors cheaply only when missing
        const boxes = await client.base.talk.getMessageBoxes({
          messageBoxListRequest: {},
        });
        if (!client || workGen !== gen) return;
        for (const box of boxes.messageBoxes ?? []) {
          const mid = String(box.id ?? "");
          storeBoxCursor(mid, box.lastDeliveredMessageId);
        }
        cursor = boxCursor.get(c.mid);
      }
      if (!cursor) return;

      const messages = await client.base.talk.getPreviousMessagesV2WithRequest({
        request: {
          messageBoxId: c.mid,
          endMessageId: {
            messageId: cursor.messageId,
            deliveredTime: cursor.deliveredTime,
          },
          messagesCount: 1,
        },
      });
      if (!client || workGen !== gen || !stdoutAlive) return;
      const raw = messages?.[messages.length - 1] ?? messages?.[0];
      if (!raw) return;
      const summary = await summarizeRawMessage(raw, { withMedia: false });
      if (workGen !== gen || !stdoutAlive) return;
      const preview = previewLine(summary);
      c.preview = preview;
      if (chatCache) {
        const row = chatCache.chats.find((x) => x.mid === c.mid);
        if (row) row.preview = preview;
      }
      emitEvent("chat_preview", {
        mid: c.mid,
        preview,
        lastActivity: c.lastActivity,
      });
    } catch (e) {
      console.error("[hydratePreviews]", c.mid, e);
      if (!c.preview) {
        c.preview = previewLang().tap;
        emitEvent("chat_preview", {
          mid: c.mid,
          preview: c.preview,
          lastActivity: c.lastActivity,
        });
      }
    }
  });

  if (chatCache) {
    try {
      await Deno.writeTextFile(chatCachePath, JSON.stringify(chatCache));
    } catch { /* ignore */ }
  }
}

async function buildFriendsList(): Promise<ChatRow[]> {
  const friends: ChatRow[] = [];
  for (const [mid, info] of contactIndex) {
    // Users / bots / OA use u… mids; skip groups (c…) and rooms (r…).
    if (!mid.startsWith("u")) continue;
    friends.push({
      mid,
      name: info.name || mid,
      kind: info.kind || "dm",
      picturePath: info.picturePath,
      avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
      lastActivity: 0,
      unread: 0,
      preview: "",
      muted: !!info.muted,
    });
  }
  friends.sort((a, b) =>
    a.name.localeCompare(b.name) || a.mid.localeCompare(b.mid)
  );
  return friends;
}

async function doListFriends(id: number | string | null, force = false) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  await refreshCachePolicy();
  try {
    await loadDiskContacts();
    const cached = await buildFriendsList();
    const stale = force ||
      contactIndex.size === 0 ||
      Date.now() - contactsAt >= cachePolicy().contactsRefresh;

    // Stale-while-revalidate: show disk/memory cache immediately.
    if (cached.length > 0) {
      ok(id, { friends: cached, count: cached.length, cached: true });
      hydrateAvatars(cached);
      if (stale) {
        refreshContactIndex(force).then(async () => {
          const friends = await buildFriendsList();
          emitEvent("friends_updated", { friends, count: friends.length });
          hydrateAvatars(friends);
        }).catch((e) => console.error("[friends refresh]", e));
      }
      return;
    }

    await refreshContactIndex(true);
    const friends = await buildFriendsList();
    ok(id, { friends, count: friends.length, cached: false });
    hydrateAvatars(friends);
  } catch (e) {
    fail(id, e instanceof Error ? e.message : String(e));
  }
}

async function hydrateAvatars(chats: ChatRow[]) {
  const need = chats.filter((c) => c.picturePath && !c.avatarPath);
  await mapPool(need, AVATAR_CONCURRENCY, async (c) => {
    const path = await avatarPathFor(c.mid, c.picturePath);
    if (!path) return;
    c.avatarPath = path;
    if (chatCache) {
      const row = chatCache.chats.find((x) => x.mid === c.mid);
      if (row) row.avatarPath = path;
    }
    emitEvent("avatar_ready", { mid: c.mid, avatarPath: path });
  });
  if (chatCache) {
    try {
      await Deno.writeTextFile(chatCachePath, JSON.stringify(chatCache));
    } catch { /* ignore */ }
  }
}

function touchChatPreviewFromMessage(message: Json) {
  void upsertChatFromMessage(message);
}

async function resolveContactInfo(mid: string): Promise<{
  name: string;
  picturePath: string | null;
  kind: string;
  muted: boolean;
}> {
  const cached = contactIndex.get(mid);
  if (cached?.name && cached.name !== mid) return cached;
  if (!client) {
    return cached ?? {
      name: mid,
      picturePath: null,
      kind: mid.startsWith("c") ? "group" : "dm",
      muted: false,
    };
  }
  try {
    const c = await client.base.talk.getContact({ mid });
    const type = String(c.type ?? "");
    const info = {
      name: c.displayName || mid,
      picturePath: c.picturePath ?? null,
      kind: /BOT/i.test(type) ? "bot" : mid.startsWith("c") ? "group" : "dm",
      muted: !!(c as { notificationDisabled?: boolean }).notificationDisabled,
    };
    contactIndex.set(mid, info);
    await saveDiskContacts();
    return info;
  } catch {
    return cached ?? {
      name: mid,
      picturePath: null,
      kind: mid.startsWith("c") ? "group" : "dm",
      muted: false,
    };
  }
}

async function upsertChatFromMessage(message: Json) {
  const peer = message.mine ? String(message.to) : String(message.from);
  if (!peer) return;
  const preview = previewLine(message);
  const activity = Number(message.createdTime ?? 0);
  if (!chatCache) chatCache = { at: Date.now(), chats: [] };

  let row = chatCache.chats.find((x) => x.mid === peer);
  let created = false;
  if (!row) {
    const info = await resolveContactInfo(peer);
    row = {
      mid: peer,
      name: info.name,
      kind: info.kind,
      picturePath: info.picturePath,
      avatarPath: await existingFile(join(avatarDir, `${peer}.jpg`)),
      lastActivity: activity,
      unread: 0,
      preview,
    };
    created = true;
  } else {
    row.preview = preview;
    if (activity > row.lastActivity) row.lastActivity = activity;
    if (!row.name || row.name === peer) {
      const info = await resolveContactInfo(peer);
      row.name = info.name;
      row.kind = info.kind || row.kind;
      row.picturePath = info.picturePath ?? row.picturePath ?? null;
    }
  }

  chatCache.chats = [row, ...chatCache.chats.filter((c) => c.mid !== peer)];
  chatCache.at = Date.now();
  try {
    await Deno.writeTextFile(chatCachePath, JSON.stringify(chatCache));
  } catch { /* ignore */ }

  emitEvent("chat_upsert", { chat: row, created });
  emitEvent("chat_preview", { mid: peer, preview, lastActivity: activity });
  if (row.picturePath && !row.avatarPath) {
    hydrateAvatars([row]);
  }
}

async function upsertChatFromContact(mid: string) {
  if (!mid || (!mid.startsWith("u") && !mid.startsWith("c"))) return;
  const info = await resolveContactInfo(mid);
  if (!chatCache) chatCache = { at: Date.now(), chats: [] };
  let row = chatCache.chats.find((x) => x.mid === mid);
  const created = !row;
  if (!row) {
    row = {
      mid,
      name: info.name,
      kind: info.kind,
      picturePath: info.picturePath,
      avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
      lastActivity: Date.now(),
      unread: 0,
      preview: "",
    };
  } else {
    row.name = info.name || row.name;
    row.kind = info.kind || row.kind;
    row.picturePath = info.picturePath ?? row.picturePath ?? null;
  }
  chatCache.chats = [row, ...chatCache.chats.filter((c) => c.mid !== mid)];
  chatCache.at = Date.now();
  try {
    await Deno.writeTextFile(chatCachePath, JSON.stringify(chatCache));
  } catch { /* ignore */ }
  emitEvent("chat_upsert", { chat: row, created });
  if (row.picturePath && !row.avatarPath) hydrateAvatars([row]);
}

async function startListen() {
  if (!client || listening) return;
  listening = true;

  // Register BEFORE listen() so early ops are not missed.
  client.on("message", async (msg) => {
    try {
      const message = await summarizeTalkMessage(msg as TalkMessage, { withMedia: false });
      const peer = message.mine ? String(message.to) : String(message.from);
      msgCache.delete(peer);
      emitEvent("message", { message: { ...message, imagePath: null } });
      await upsertChatFromMessage(message);
      if (message.needsMedia) {
        hydrateMedia([message], peer);
      }
    } catch (e) {
      console.error("[listen message]", e);
    }
  });

  client.on("event", (op: {
    type?: string | number;
    param1?: string;
    param2?: string;
    param3?: string;
  }) => {
    try {
      const type = String(op?.type ?? "");
      if (type === "NOTIFIED_READ_MESSAGE") {
        emitEvent("read_receipt", {
          chatMid: String(op.param1 ?? ""),
          userMid: String(op.param2 ?? ""),
          messageId: String(op.param3 ?? ""),
        });
      } else if (
        type.includes("ADD_CONTACT") ||
        type.includes("ACCEPT_CONTACT") ||
        type.includes("FRIEND_REQUEST") ||
        type === "NOTIFIED_UPDATE_PROFILE"
      ) {
        const mid = String(op.param1 ?? op.param2 ?? "");
        if (mid) void upsertChatFromContact(mid);
      }
    } catch (e) {
      console.error("[listen event]", e);
    }
  });

  client.on("call:incoming", (ev: { callMid?: string; from?: string; kind?: string }) => {
    const callId = String(ev.callMid ?? "");
    const from = String(ev.from ?? "");
    const kind = String(ev.kind ?? "AUDIO");
    incomingOffer = { callId, from, kind };
    emitEvent("call_incoming", { callId, from, kind });
  });
  client.on("call:cancel", (ev: { callMid?: string; from?: string; reason?: string }) => {
    const from = String(ev.from ?? "");
    if (incomingOffer?.from === from || incomingOffer?.callId === String(ev.callMid ?? "")) {
      incomingOffer = null;
    }
    emitEvent("call_canceled", {
      callId: String(ev.callMid ?? ""),
      from,
      reason: String(ev.reason ?? ""),
    });
    // Peer canceled / ended — tear down our active session too.
    if (activeCall && (activeCall.peer === from || !from)) {
      void endActiveCall();
    }
  });

  client.listen({ talk: true, square: false });
  emitEvent("listening");
}

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
    receive: () => AsyncIterable<Uint8Array>;
  };
  stopAudio?: () => void;
  aborted: boolean;
};

let activeCall: ActiveCall | null = null;
let incomingOffer: { callId: string; from: string; kind: string } | null = null;
let callAudioInput = Deno.env.get("LINE_GTK_AUDIO_INPUT")?.trim() || "default";
let callAudioOutput = Deno.env.get("LINE_GTK_AUDIO_OUTPUT")?.trim() || "default";

function setCallAudioDevices(input?: string, output?: string) {
  if (input && input.trim()) callAudioInput = input.trim();
  if (output && output.trim()) callAudioOutput = output.trim();
  if (!callAudioInput) callAudioInput = "default";
  if (!callAudioOutput) callAudioOutput = "default";
}

function setCallGains(micGain?: unknown, spkGain?: unknown) {
  if (micGain !== undefined) callAudioCtl.micGain = clampGain(micGain, callAudioCtl.micGain);
  if (spkGain !== undefined) callAudioCtl.spkGain = clampGain(spkGain, callAudioCtl.spkGain);
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

async function doCallStart(id: number | string | null, peerMid: string) {
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
    const route = await client.call.acquireRoute({
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
    call.transport = transport;

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

    emitEvent("call_state", { callId, peer: peerMid, state: "connected" });
    callLog("starting audio I/O");
    call.stopAudio = await startCallAudioIO(transport, opusCodecFactory);
    callLog("audio I/O running");
  } catch (e) {
    const raw = e instanceof Error ? e.message : String(e);
    if (call.aborted) {
      callLog("aborted:", { err: raw });
      return;
    }
    const lower = raw.toLowerCase();
    const msg =
      lower.includes("fakecall") ||
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

async function doCallAnswer(id: number | string | null) {
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
  await doCallStart(null, offer.from);
}

async function doCallDecline(id: number | string | null) {
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
    send: (payload: Uint8Array, opts?: { timestampStep?: number }) => Promise<void>;
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

async function doCallEnd(id: number | string | null) {
  incomingOffer = null;
  await endActiveCall();
  ok(id, { ended: true });
}

async function doMarkRead(
  id: number | string | null,
  chatMid: string,
  lastMessageId: string,
) {
  if (!client) {
    // Soft-fail: opening a chat during restore shouldn't toast loudly.
    ok(id, { marked: false, skipped: "not_logged_in" });
    return;
  }
  if (!chatMid || !lastMessageId) {
    ok(id, { marked: false, skipped: "missing_ids" });
    return;
  }
  try {
    const seq = typeof client.base.getReqseq === "function"
      ? await client.base.getReqseq()
      : client.base.getReqseq;
    // thrift STRING (ftype 11) — must be string, not i64/bigint
    await client.base.talk.sendChatChecked({
      chatMid: String(chatMid),
      lastMessageId: String(lastMessageId),
      seq,
    });
    if (chatCache) {
      const row = chatCache.chats.find((x) => x.mid === chatMid);
      if (row) row.unread = 0;
    }
    ok(id, { marked: true, chatMid, lastMessageId: String(lastMessageId) });
  } catch (e) {
    console.error("[mark_read]", e);
    // Soft-fail — mark-read is best-effort.
    ok(id, {
      marked: false,
      error: e instanceof Error ? e.message : String(e),
    });
  }
}

async function doLoginQr(id: number | string | null) {
  authDead = false;
  const storage = new FileStorage(storagePath);
  // Docs: QR defaults to ANDROIDSECONDARY for call-capable sessions.
  const device = LINE_DEVICE === "DESKTOPWIN" ? "ANDROIDSECONDARY" : LINE_DEVICE;
  client = await loginWithQR(
    {
      onReceiveQRUrl: (url) => emitEvent("qr", { url }),
      onPincodeRequest: (pin) => emitEvent("pin", { pin }),
    },
    { device, version: LINE_VERSION, storage },
  );
  await saveAuth(client.authToken, device);
  const profile = await client.getMyProfile();
  patchE2eeGuards();
  await startListen();
  ok(id, { ...(await myProfilePayload(profile)), device });
}

async function doLoginToken(id: number | string | null, token?: string) {
  authDead = false;
  // Already restored (e.g. boot auto-login) — just return the profile.
  if (client && listening) {
    try {
      const profile = await client.getMyProfile();
      ok(id, {
        ...(await myProfilePayload(profile)),
        device: await loadAuthDevice(),
      });
      return;
    } catch {
      /* fall through and re-auth */
    }
  }
  const auth = token?.trim() || (await loadAuth());
  if (!auth) {
    fail(id, "no_auth_token");
    return;
  }
  const storage = new FileStorage(storagePath);
  let device = await loadAuthDevice();
  // Migrate legacy DESKTOPWIN sessions — tokens are device-bound; force re-QR.
  if (device === "DESKTOPWIN") {
    await clearAuth();
    fail(id, "relogin_android_required");
    return;
  }
  if (device !== "ANDROID" && device !== "ANDROIDSECONDARY") {
    device = "ANDROIDSECONDARY";
  }
  client = await loginWithAuthToken(auth, {
    device: device as "ANDROID" | "ANDROIDSECONDARY",
    version: LINE_VERSION,
    storage,
  });
  await saveAuth(client.authToken, device);
  const profile = await client.getMyProfile();
  patchE2eeGuards();
  await startListen();
  ok(id, {
    ...(await myProfilePayload(profile)),
    device,
  });
}

async function loadDiskChatCache(): Promise<ChatRow[] | null> {
  try {
    await refreshCachePolicy();
    const raw = JSON.parse(await Deno.readTextFile(chatCachePath));
    if (
      Array.isArray(raw?.chats) &&
      Date.now() - Number(raw.at || 0) < cachePolicy().diskChat
    ) {
      return raw.chats as ChatRow[];
    }
  } catch { /* miss */ }
  return null;
}

async function doListChats(id: number | string | null, force = false) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }

  await refreshCachePolicy();
  await dedupe(`list_chats:${force}`, async () => {
    if (!force && chatCache && Date.now() - chatCache.at < cachePolicy().memChat) {
      ok(id, { chats: chatCache.chats, count: chatCache.chats.length, cached: true });
      return;
    }

    // Instant warm start from disk while we refresh (event, not second ok).
    if (!force) {
      const disk = await loadDiskChatCache();
      if (disk?.length) {
        emitEvent("chats", { chats: disk, count: disk.length, cached: true });
        emitEvent("progress", { scope: "chats", state: "ready" });
      }
    }

    emitEvent("progress", { scope: "chats", state: "loading" });

    const byMid = new Map<string, ChatRow>();

    // 1) Recent boxes (cheap + ordering)
    try {
      const boxes = await client!.base.talk.getMessageBoxes({
        messageBoxListRequest: {},
      });
      for (const box of boxes.messageBoxes ?? []) {
        const mid = String(box.id ?? "");
        if (!mid) continue;
        storeBoxCursor(mid, box.lastDeliveredMessageId);
        const prevPreview = chatCache?.chats.find((x) => x.mid === mid)?.preview ?? "";
        byMid.set(mid, {
          mid,
          name: contactIndex.get(mid)?.name || mid,
          kind: contactIndex.get(mid)?.kind ||
            (mid.startsWith("c") ? "group" : "dm"),
          avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
          picturePath: contactIndex.get(mid)?.picturePath ?? null,
          lastActivity: Number(box.lastDeliveredMessageId?.deliveredTime ?? 0),
          unread: Number(box.unreadCount ?? 0),
          preview: prevPreview,
          muted: !!contactIndex.get(mid)?.muted,
        });
      }
    } catch (e) {
      console.error("[list_chats] boxes", e);
    }

    // 2) Contacts + bots/OA (getContacts mids — fetchUsers misses bots)
    try {
      await loadDiskContacts();
      await refreshContactIndex();
      for (const [mid, info] of contactIndex) {
        const prev = byMid.get(mid);
        if (prev) {
          prev.name = info.name;
          prev.kind = info.kind;
          prev.picturePath = info.picturePath;
          prev.muted = !!info.muted;
          if (!prev.avatarPath) {
            prev.avatarPath = await existingFile(join(avatarDir, `${mid}.jpg`));
          }
        } else {
          byMid.set(mid, {
            mid,
            name: info.name,
            kind: info.kind,
            picturePath: info.picturePath,
            avatarPath: await existingFile(join(avatarDir, `${mid}.jpg`)),
            lastActivity: 0,
            unread: 0,
            preview: "",
            muted: !!info.muted,
          });
        }
      }
    } catch (e) {
      console.error("[list_chats] contacts", e);
      // fallback: friends only
      try {
        const users = await client!.fetchUsers();
        for (const user of users) {
          const name = friendName(user);
          const picturePath = user.raw.contact?.picturePath ?? null;
          contactIndex.set(user.mid, { name, picturePath, kind: "dm", muted: false });
          const prev = byMid.get(user.mid);
          if (prev) {
            prev.name = name;
            prev.picturePath = picturePath;
          }
        }
      } catch (e2) {
        console.error("[list_chats] users fallback", e2);
      }
    }

    // 3) Groups optional (often empty) — don't block naming
    try {
      const chats = await client!.fetchJoinedChats();
      for (const chat of chats) {
        const prev = byMid.get(chat.mid);
        byMid.set(chat.mid, {
          mid: chat.mid,
          name: chat.name || chat.raw.chatName || chat.mid,
          kind: "group",
          lastActivity: prev?.lastActivity ?? 0,
          unread: prev?.unread ?? 0,
          preview: prev?.preview ?? "",
          picturePath: prev?.picturePath ?? null,
          avatarPath: prev?.avatarPath ?? null,
        });
      }
    } catch (e) {
      console.error("[list_chats] groups", e);
    }

    const chats = [...byMid.values()].sort((a, b) => {
      if (b.lastActivity !== a.lastActivity) return b.lastActivity - a.lastActivity;
      return a.name.localeCompare(b.name);
    });

    chatCache = { at: Date.now(), chats };
    try {
      await Deno.writeTextFile(chatCachePath, JSON.stringify(chatCache));
    } catch { /* ignore */ }

    ok(id, { chats, count: chats.length, cached: false });
    emitEvent("progress", { scope: "chats", state: chats.length ? "ready" : "empty" });

    // Background fills — CDN avatars + last-message previews (throttled)
    hydrateAvatars(chats);
    hydratePreviews(chats);
  });
}

async function doFetchMessages(
  id: number | string | null,
  chatMid: string,
  limit = 50,
  force = false,
) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  if (!chatMid) {
    fail(id, "missing_chat");
    return;
  }

  await refreshCachePolicy();
  await dedupe(`msgs:${chatMid}:${limit}:${force}`, async () => {
    bumpMediaEpoch(chatMid); // cancel any prior hydrate for this chat
    const cached = msgCache.get(chatMid);
    if (!force && cached && Date.now() - cached.at < cachePolicy().memMsg) {
      const ui = await forUiMessages(cached.messages);
      ok(id, { messages: ui, cached: true });
      hydrateMedia(cached.messages, chatMid);
      return;
    }

    // Warm from disk immediately
    if (!force) {
      const disk = await loadDiskMessages(chatMid);
      if (disk?.length) {
        msgCache.set(chatMid, { at: Date.now(), messages: disk });
        emitEvent("messages", {
          chatMid,
          messages: await forUiMessages(disk),
          cached: true,
        });
        emitEvent("progress", {
          scope: "messages",
          chatMid,
          state: "ready",
        });
        hydrateMedia(disk, chatMid);
        // still refresh below unless memory-fresh
      }
    }

    if (!force && cached?.messages?.length) {
      emitEvent("messages", {
        chatMid,
        messages: await forUiMessages(cached.messages),
        cached: true,
      });
      emitEvent("progress", {
        scope: "messages",
        chatMid,
        state: "ready",
      });
    }

    emitEvent("progress", { scope: "messages", chatMid, state: "loading" });

    try {
      let out: Json[] = [];

      // Prefer message boxes for DMs/bots (fast + works). Groups try Chat helper first.
      let usedBoxes = false;
      try {
        if (chatMid.startsWith("c")) {
          const chat = await client!.getChat(chatMid);
          const messages = await chat.fetchMessages(limit);
          for (const m of messages) {
            out.push(await summarizeTalkMessage(m, { withMedia: false }));
          }
        } else {
          usedBoxes = true;
        }
      } catch {
        usedBoxes = true;
      }

      if (usedBoxes || !out.length) {
        let cursor = boxCursor.get(chatMid);
        if (!cursor) {
          const boxes = await client!.base.talk.getMessageBoxes({
            messageBoxListRequest: {},
          });
          for (const box of boxes.messageBoxes ?? []) {
            const mid = String(box.id ?? "");
            storeBoxCursor(mid, box.lastDeliveredMessageId);
          }
          cursor = boxCursor.get(chatMid);
        }
        if (!cursor) {
          msgCache.set(chatMid, { at: Date.now(), messages: [] });
          ok(id, { messages: [] });
          emitEvent("progress", { scope: "messages", chatMid, state: "empty" });
          return;
        }
        const messages = await client!.base.talk.getPreviousMessagesV2WithRequest({
          request: {
            messageBoxId: chatMid,
            endMessageId: {
              messageId: cursor.messageId,
              deliveredTime: cursor.deliveredTime,
            },
            messagesCount: limit,
          },
        });
        out = [];
        for (const raw of messages) {
          try {
            const meta = (raw as { contentMetadata?: Record<string, string> })
              .contentMetadata;
            if (meta?.BOT_CHECK || meta?.BOT_ORIGIN) {
              const from = String((raw as { from?: string }).from ?? chatMid);
              const cur = contactIndex.get(from);
              if (cur) cur.kind = "bot";
              else {
                contactIndex.set(from, {
                  name: contactIndex.get(from)?.name || from,
                  picturePath: null,
                  kind: "bot",
                  muted: false,
                });
              }
            }
            out.push(await summarizeRawMessage(raw, { withMedia: false }));
          } catch (e) {
            console.error("[fetch msg skip]", e);
          }
        }
      }

      out.sort((a, b) => Number(a.createdTime) - Number(b.createdTime));
      msgCache.set(chatMid, { at: Date.now(), messages: out });
      await saveDiskMessages(chatMid, out);
      ok(id, { messages: await forUiMessages(out), cached: false });
      emitEvent("progress", {
        scope: "messages",
        chatMid,
        state: out.length ? "ready" : "empty",
      });

      hydrateMedia(out, chatMid);
    } catch (e) {
      fail(id, e instanceof Error ? e.message : String(e));
      emitEvent("progress", {
        scope: "messages",
        chatMid,
        state: "error",
        error: String(e),
      });
    }
  });
}

function sentMessagePayload(
  sent: { id?: unknown; createdTime?: unknown; contentType?: unknown } | null | undefined,
  chatMid: string,
  text: string,
): Json {
  return {
    id: String(sent?.id ?? Date.now()),
    text,
    from: myMid(),
    to: chatMid,
    mine: true,
    createdTime: Number(sent?.createdTime ?? Date.now()),
    contentType: normType(sent?.contentType ?? "NONE"),
    imagePath: null,
    imageUrl: null,
    audioPath: null,
    stickerId: null,
    stickerPackageId: null,
    flex: null,
    durationMs: null,
    needsMedia: false,
  };
}

async function doSend(id: number | string | null, chatMid: string, text: string) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  if (!text.trim()) {
    fail(id, "empty_message");
    return;
  }

  let sent: { id?: unknown; createdTime?: unknown; contentType?: unknown } | null = null;

  // Prefer plain first (avoids e2ee contentMetadata crashes on some peers), then e2ee.
  try {
    sent = await client.base.talk.sendMessage({
      to: chatMid,
      text,
      e2ee: false,
    }) as typeof sent;
  } catch (e1) {
    try {
      sent = await client.base.talk.sendMessage({
        to: chatMid,
        text,
        e2ee: true,
      }) as typeof sent;
    } catch (e2) {
      try {
        const chat = await client.getChat(chatMid);
        const m = await chat.sendMessage(text);
        const message = await summarizeTalkMessage(m, { withMedia: false }).catch(() =>
          sentMessagePayload(
            { id: (m as { raw?: { id?: unknown } }).raw?.id, createdTime: Date.now() },
            chatMid,
            text,
          )
        );
        msgCache.delete(chatMid);
        chatCache = null;
        touchChatPreviewFromMessage(message);
        ok(id, { message });
        return;
      } catch (e3) {
        fail(
          id,
          e3 instanceof Error
            ? e3.message
            : e2 instanceof Error
            ? e2.message
            : e1 instanceof Error
            ? e1.message
            : String(e1),
        );
        return;
      }
    }
  }

  msgCache.delete(chatMid);
  chatCache = null;
  const message = sentMessagePayload(sent, chatMid, text);
  touchChatPreviewFromMessage(message);
  ok(id, { message });
}

async function doListStickers(id: number | string | null) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }

  const ownedPkgs = await fetchOwnedPackages();
  const ownedPkgIds = new Set(ownedPkgs.map((p) => p.id));
  const versionByPkg = new Map(ownedPkgs.map((p) => [p.id, p.version]));

  // Recents tab (owned only).
  const recentEntries: StickerEntry[] = [];
  const recentSeen = new Set<string>();
  for (const s of stickerIndex) {
    if (!ownedPkgIds.has(s.packageId)) continue;
    const key = `${s.packageId}:${s.stickerId}`;
    if (recentSeen.has(key)) continue;
    recentSeen.add(key);
    recentEntries.push({
      ...s,
      version: s.version || versionByPkg.get(s.packageId),
    });
    if (recentEntries.length >= 24) break;
  }

  const packs: Json[] = [];

  if (recentEntries.length) {
    const stickers: Json[] = [];
    for (const s of recentEntries) {
      stickers.push({
        stickerId: s.stickerId,
        packageId: s.packageId,
        version: s.version ?? null,
        imagePath: await ensureStickerStatic(s.stickerId),
      });
    }
    packs.push({
      id: "__recent__",
      name: "Recent",
      version: null,
      iconPath: stickers[0]?.imagePath ?? null,
      recent: true,
      stickers,
    });
  }

  // Owned packs with full grids + pack icons.
  for (const pkg of ownedPkgs.slice(0, 12)) {
    const detail = await stickersForOwnedPackage(pkg, 48);
    if (!detail.stickers.length) continue;
    const stickers: Json[] = [];
    for (const s of detail.stickers) {
      stickers.push({
        stickerId: s.stickerId,
        packageId: s.packageId,
        version: s.version ?? detail.version,
        imagePath: await ensureStickerStatic(s.stickerId),
      });
    }
    let iconPath = await ensurePackIcon(pkg.id);
    if (!iconPath) {
      const first = stickers[0]?.imagePath;
      iconPath = typeof first === "string" && first ? first : null;
    }
    packs.push({
      id: pkg.id,
      name: detail.name || pkg.name || pkg.id,
      version: detail.version,
      iconPath,
      recent: false,
      stickers,
    });
  }

  ok(id, {
    packs,
    // Keep flat list for older UI paths (unused once chooser migrates).
    stickers: packs.flatMap((p) =>
      ((p.stickers as Json[]) ?? []).map((s) => ({
        ...s,
        recent: p.id === "__recent__",
      }))
    ),
    ownedPackages: ownedPkgs.length,
  });
}

async function doSendSticker(
  id: number | string | null,
  chatMid: string,
  stickerId: string,
  packageId: string,
  version?: string,
) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  if (!chatMid || !stickerId || !packageId) {
    fail(id, "missing_sticker");
    return;
  }

  // Prefer version from owned-pack cache when caller omitted it.
  let stkVer = version?.trim() || "";
  if (!stkVer) {
    const owned = await fetchOwnedPackages();
    stkVer = owned.find((p) => p.id === packageId)?.version || "1";
  }

  const meta: Record<string, string> = {
    STKID: stickerId,
    STKPKGID: packageId,
    STKVER: stkVer,
  };

  let sent: { id?: unknown; createdTime?: unknown; contentType?: unknown } | null =
    null;
  try {
    // Stickers are sent as contentType+metadata (not E2EE text chunks).
    sent = await client.base.talk.sendMessage({
      to: chatMid,
      contentType: "STICKER",
      contentMetadata: meta,
      e2ee: false,
    }) as typeof sent;
  } catch (e1) {
    try {
      sent = await client.base.talk.sendMessage({
        to: chatMid,
        contentType: "STICKER",
        contentMetadata: meta,
      }) as typeof sent;
    } catch (e2) {
      const raw = e2 instanceof Error
        ? e2.message
        : e1 instanceof Error
        ? e1.message
        : String(e1);
      if (raw.includes("USER_NOT_STICKER_OWNER") || raw.includes("not owned")) {
        forgetSticker(stickerId, packageId);
        ownedPackagesCache = null;
        fail(id, "sticker_not_owned");
        return;
      }
      fail(id, raw);
      return;
    }
  }

  rememberSticker(stickerId, packageId, stkVer);
  const imagePath = await ensureStickerImage(stickerId);
  msgCache.delete(chatMid);
  chatCache = null;
  const message = {
    ...sentMessagePayload(sent, chatMid, "[Sticker]"),
    contentType: "STICKER",
    stickerId,
    stickerPackageId: packageId,
    imagePath,
    imageUrl: stickerUrls(stickerId)[0],
    needsMedia: !imagePath,
  };
  touchChatPreviewFromMessage(message);
  ok(id, { message });
}

type MediaOType = "image" | "gif" | "video" | "audio" | "file";

function guessMime(name: string, oType: MediaOType): string {
  const lower = name.toLowerCase();
  if (lower.endsWith(".png")) return "image/png";
  if (lower.endsWith(".gif")) return "image/gif";
  if (lower.endsWith(".webp")) return "image/webp";
  if (lower.endsWith(".bmp")) return "image/bmp";
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
  if (lower.endsWith(".mp4")) return "video/mp4";
  if (lower.endsWith(".webm")) return "video/webm";
  if (lower.endsWith(".mov")) return "video/quicktime";
  if (lower.endsWith(".mkv")) return "video/x-matroska";
  if (lower.endsWith(".mp3")) return "audio/mpeg";
  if (lower.endsWith(".wav")) return "audio/wav";
  if (lower.endsWith(".ogg") || lower.endsWith(".oga")) return "audio/ogg";
  if (lower.endsWith(".m4a") || lower.endsWith(".aac")) return "audio/mp4";
  if (oType === "image" || oType === "gif") return "image/jpeg";
  if (oType === "video") return "video/mp4";
  if (oType === "audio") return "audio/mp4";
  return "application/octet-stream";
}

function normalizeMediaOType(raw: string, fileName: string): MediaOType {
  const t = raw.trim().toLowerCase();
  if (t === "image" || t === "gif" || t === "video" || t === "audio" || t === "file") {
    if (t === "image" && fileName.toLowerCase().endsWith(".gif")) return "gif";
    return t;
  }
  const lower = fileName.toLowerCase();
  if (/\.(gif)$/.test(lower)) return "gif";
  if (/\.(png|jpe?g|webp|bmp|heic)$/.test(lower)) return "image";
  if (/\.(mp4|webm|mov|mkv|m4v)$/.test(lower)) return "video";
  if (/\.(m4a|mp3|wav|ogg|oga|aac|flac)$/.test(lower)) return "audio";
  return "file";
}

function mediaContentType(oType: MediaOType): string {
  if (oType === "gif") return "IMAGE";
  return oType.toUpperCase();
}

function mediaPreviewText(oType: MediaOType, fileName: string): string {
  if (oType === "audio") return "Voice message";
  if (oType === "image" || oType === "gif") return "Photo";
  if (oType === "video") return "Video";
  return fileName || "File";
}

function supportsE2EEMedia(mid: string): boolean {
  // linejs uploadMediaByE2EE only accepts user (u) and group (c) mids.
  return mid.startsWith("u") || mid.startsWith("c");
}

async function makeImagePreviewBlob(filePath: string): Promise<Blob | null> {
  try {
    const dest = join(mediaDir, `preview-${Date.now()}.jpg`);
    const code = `
from PIL import Image
src=${JSON.stringify(filePath)}
dest=${JSON.stringify(dest)}
im=Image.open(src)
im.thumbnail((640, 640))
if im.mode not in ("RGB","L"):
    im=im.convert("RGB")
im.save(dest, "JPEG", quality=80, optimize=True)
`;
    const p = new Deno.Command("python3", {
      args: ["-c", code],
      stdout: "null",
      stderr: "null",
    });
    const { success } = await p.output();
    if (!success) return null;
    const buf = await Deno.readFile(dest);
    try {
      await Deno.remove(dest);
    } catch { /* ignore */ }
    return new Blob([buf], { type: "image/jpeg" });
  } catch {
    return null;
  }
}

async function extractVideoThumb(filePath: string, messageId: string): Promise<string | null> {
  const dest = thumbDest(messageId);
  try {
    const p = new Deno.Command("ffmpeg", {
      args: [
        "-y",
        "-i",
        filePath,
        "-ss",
        "0.2",
        "-vframes",
        "1",
        "-q:v",
        "3",
        dest,
      ],
      stdout: "null",
      stderr: "null",
    });
    const { success } = await p.output();
    if (!success) return null;
    return await existingImage(dest);
  } catch {
    return null;
  }
}

/** Turn decrypted video (or preview image) bytes into a UI thumbnail path. */
async function materializeVideoPreview(
  messageId: string,
  buf: Uint8Array,
): Promise<string | null> {
  if (buf.length < 32) return null;
  if (isImageBytes(buf)) {
    return await writeImageFile(thumbDest(messageId), buf);
  }
  const vdest = mediaDest(messageId, "mp4");
  try {
    await Deno.writeFile(vdest, buf);
  } catch (e) {
    console.error("[materializeVideoPreview write]", messageId, e);
    return null;
  }
  return await extractVideoThumb(vdest, messageId);
}

async function cacheOutgoingMedia(
  messageId: string,
  filePath: string,
  oType: MediaOType,
): Promise<{ imagePath: string | null; audioPath: string | null; filePath: string | null }> {
  try {
    const data = await Deno.readFile(filePath);
    if (oType === "audio") {
      const dest = mediaDest(messageId, "m4a");
      await Deno.writeFile(dest, data);
      return { imagePath: null, audioPath: dest, filePath: dest };
    }
    if (oType === "image" || oType === "gif") {
      const lower = filePath.toLowerCase();
      const ext = lower.endsWith(".png")
        ? "png"
        : lower.endsWith(".gif")
        ? "gif"
        : lower.endsWith(".webp")
        ? "webp"
        : "jpg";
      const dest = fullDest(messageId, ext);
      await Deno.writeFile(dest, data);
      const ui = await uiMediaPath(messageId, dest);
      return { imagePath: ui || dest, audioPath: null, filePath: dest };
    }
    if (oType === "video") {
      const thumb = await extractVideoThumb(filePath, messageId);
      // Keep original video beside the thumb for later open/play.
      const vdest = mediaDest(messageId, "mp4");
      await Deno.writeFile(vdest, data);
      return { imagePath: thumb, audioPath: null, filePath: vdest };
    }
    // Generic file: keep a copy for re-open.
    const name = filePath.split("/").pop() || "file.bin";
    const dest = join(mediaDir, `${messageId}.${name}`);
    await Deno.writeFile(dest, data);
    return { imagePath: null, audioPath: null, filePath: dest };
  } catch (e) {
    console.error("[cacheOutgoingMedia]", messageId, e);
    return {
      imagePath: oType === "image" || oType === "gif" ? filePath : null,
      audioPath: oType === "audio" ? filePath : null,
      filePath,
    };
  }
}

/**
 * E2EE OBS media send aligned with linejs `obs.uploadMediaByE2EE`, plus DURATION
 * for audio/video so official LINE clients render the bubble correctly.
 */
async function sendE2EEMedia(opts: {
  to: string;
  oType: MediaOType;
  data: Blob;
  filename: string;
  durationMs?: number;
  preview?: Blob;
  onProgress?: (progress: number, label: string) => void;
}): Promise<{ id?: unknown; createdTime?: unknown; contentType?: unknown }> {
  if (!client) throw new Error("not_logged_in");
  if (!supportsE2EEMedia(opts.to)) {
    throw new Error("e2ee_media_mid_unsupported");
  }
  const report = opts.onProgress ?? (() => {});

  const oType = opts.oType;
  const typeSet: Record<MediaOType, [string, number]> = {
    image: ["emi", 1],
    video: ["emv", 2],
    audio: ["ema", 3],
    file: ["emf", 14],
    gif: ["emi", 1],
  };
  const [obsNamespace, contentTypeNum] = typeSet[oType];
  const ext = (opts.filename.split(".").pop() || "bin").toLowerCase();
  const params: Record<string, string> = { type: "file" };
  if (oType === "gif") params.cat = "original";

  const e2ee = client.base.e2ee as {
    encryptByKeyMaterial: (
      data: Buffer,
      key?: Buffer,
    ) => Promise<{ keyMaterial: string; encryptedData: Buffer }>;
    encryptE2EEMessage: (
      to: string,
      data: { keyMaterial: string; fileName: string },
      contentType: number,
    ) => Promise<string[] | Buffer[]>;
  };
  const obs = client.base.obs as {
    uploadObjectForService: (options: Record<string, unknown>) => Promise<{
      objId: string;
      objHash: string;
      headers: Headers;
    }>;
  };

  report(0.12, "Encrypting…");
  const plain = Buffer.from(await opts.data.arrayBuffer());
  const { keyMaterial, encryptedData } = await e2ee.encryptByKeyMaterial(plain);
  // deno-lint-ignore no-explicit-any
  const edata = new Blob([encryptedData as any]);
  const tempId = "reqid-" + crypto.randomUUID();
  report(0.35, "Uploading…");
  const { objId } = await obs.uploadObjectForService({
    data: edata,
    oType: "file",
    obsPath: `talk/${obsNamespace}/${tempId}`,
    params,
  });

  if (oType === "image" || oType === "gif" || oType === "video") {
    report(0.55, "Uploading preview…");
    let previewEdata: Blob;
    if (opts.preview) {
      const enc = await e2ee.encryptByKeyMaterial(
        Buffer.from(await opts.preview.arrayBuffer()),
        Buffer.from(keyMaterial, "base64"),
      );
      // deno-lint-ignore no-explicit-any
      previewEdata = new Blob([enc.encryptedData as any]);
    } else {
      previewEdata = edata;
    }
    const { objId: objId2 } = await obs.uploadObjectForService({
      data: previewEdata,
      oType: "file",
      obsPath: `talk/${obsNamespace}/${objId}__ud-preview`,
      params,
    });
    if (objId !== objId2) {
      throw new Error("obs_preview_oid_mismatch");
    }
  }

  report(0.78, "Sending…");
  const chunks = await e2ee.encryptE2EEMessage(
    opts.to,
    { keyMaterial, fileName: opts.filename || `line.${ext}` },
    contentTypeNum,
  );

  const meta: Record<string, string> = {
    SID: obsNamespace,
    OID: objId,
    FILE_SIZE: String(edata.size),
    e2eeVersion: "2",
  };
  if (
    (oType === "audio" || oType === "video") &&
    opts.durationMs != null &&
    Number.isFinite(opts.durationMs)
  ) {
    meta.DURATION = String(Math.max(1, Math.round(opts.durationMs)));
  }
  if (oType === "file") {
    meta.FILE_NAME = opts.filename;
  }
  if (oType === "image" || oType === "gif" || oType === "video") {
    meta.MEDIA_CONTENT_INFO = JSON.stringify({
      category: "original",
      fileSize: edata.size,
      extension: ext,
      animated: oType === "gif",
    });
  }

  report(0.92, "Finishing…");
  return await client.base.talk.sendMessage({
    to: opts.to,
    chunks,
    // deno-lint-ignore no-explicit-any
    contentType: contentTypeNum as any,
    contentMetadata: meta,
  }) as { id?: unknown; createdTime?: unknown; contentType?: unknown };
}

async function doSendMedia(
  id: number | string | null,
  chatMid: string,
  filePath: string,
  oTypeRaw?: string,
  durationMs?: number,
) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  if (!chatMid || !filePath) {
    fail(id, "chatMid and filePath required");
    return;
  }
  const report = (progress: number, label: string) => {
    emitEvent("upload_progress", {
      chatMid,
      progress,
      label,
      done: false,
    });
  };
  try {
    report(0.02, "Reading file…");
    const data = await Deno.readFile(filePath);
    if (data.length < 32) {
      fail(id, "file_too_small");
      emitEvent("upload_progress", { chatMid, progress: 0, label: "", done: true });
      return;
    }
    const name = filePath.split("/").pop() || "file.bin";
    const oType = normalizeMediaOType(oTypeRaw ?? "auto", name);
    if (oType === "audio" && data.length < 1024) {
      fail(id, "audio_file_too_small");
      emitEvent("upload_progress", { chatMid, progress: 0, label: "", done: true });
      return;
    }
    const mime = guessMime(name, oType);
    const blob = new Blob([data], { type: mime });
    const talkType: MediaOType = oType === "gif" ? "image" : oType;
    const dur = durationMs != null && Number.isFinite(durationMs)
      ? Math.max(1, Math.round(durationMs))
      : (oType === "audio" || oType === "video" ? 1919 : undefined);

    report(0.08, "Preparing…");
    let preview: Blob | undefined;
    if (oType === "image" || oType === "gif") {
      preview = (await makeImagePreviewBlob(filePath)) ?? undefined;
    } else if (oType === "video") {
      const thumbPath = await extractVideoThumb(filePath, `out-${Date.now()}`);
      if (thumbPath) {
        preview = new Blob([await Deno.readFile(thumbPath)], { type: "image/jpeg" });
      }
    }

    let sent: { id?: unknown; createdTime?: unknown; contentType?: unknown } | null = null;
    let lastErr: unknown = null;

    if (supportsE2EEMedia(chatMid)) {
      try {
        sent = await sendE2EEMedia({
          to: chatMid,
          oType,
          data: blob,
          filename: name,
          durationMs: dur,
          preview,
          onProgress: report,
        });
      } catch (e1) {
        lastErr = e1;
        console.error("[send_media e2ee]", e1);
        report(0.4, "Retrying upload…");
        try {
          sent = await client.base.obs.uploadMediaByE2EE({
            to: chatMid,
            oType,
            data: blob,
            filename: name,
            preview,
          }) as typeof sent;
          report(0.9, "Finishing…");
        } catch (e2) {
          lastErr = e2;
          console.error("[send_media uploadMediaByE2EE]", e2);
        }
      }
      if (!sent) {
        emitEvent("upload_progress", { chatMid, progress: 0, label: "", done: true });
        fail(
          id,
          lastErr instanceof Error ? lastErr.message : String(lastErr ?? "e2ee_media_failed"),
        );
        return;
      }
    } else {
      try {
        report(0.4, "Uploading…");
        const up = await client.base.obs.uploadObjTalk(
          chatMid,
          talkType,
          blob,
          undefined,
          name,
          dur,
        );
        report(0.85, "Finishing…");
        let realId: string | null = null;
        try {
          const recent = await client.base.talk.getPreviousMessagesV2WithRequest({
            request: {
              messageBoxId: chatMid,
              endMessageId: boxCursor.get(chatMid)
                ? {
                  messageId: boxCursor.get(chatMid)!.messageId,
                  deliveredTime: boxCursor.get(chatMid)!.deliveredTime,
                }
                : undefined,
              messagesCount: 20,
            },
          }).catch(() => [] as unknown[]);
          const mine = (recent as Array<{ id?: unknown; from?: string; contentType?: unknown }>)
            .filter((m) => isMineFrom(m.from))
            .reverse();
          const wantCt = mediaContentType(oType);
          const hit = mine.find((m) => normType(m.contentType) === wantCt);
          if (hit?.id != null) realId = String(hit.id);
        } catch { /* ignore */ }
        sent = {
          id: realId ?? (up as { objId?: string }).objId ?? Date.now(),
          createdTime: Date.now(),
          contentType: mediaContentType(oType),
        };
      } catch (e3) {
        emitEvent("upload_progress", { chatMid, progress: 0, label: "", done: true });
        fail(
          id,
          e3 instanceof Error ? e3.message : String(e3),
        );
        return;
      }
    }

    msgCache.delete(chatMid);
    chatCache = null;
    const ct = mediaContentType(oType);
    const messageId = String(sent?.id ?? Date.now());
    report(0.96, "Caching…");
    const cached = await cacheOutgoingMedia(messageId, filePath, oType);
    const message = {
      ...sentMessagePayload(sent, chatMid, mediaPreviewText(oType, name)),
      id: messageId,
      contentType: ct,
      imagePath: cached.imagePath,
      audioPath: cached.audioPath,
      fileName: name,
      filePath: cached.filePath || filePath,
      durationMs: dur ?? null,
      needsMedia: !cached.imagePath && (oType === "image" || oType === "gif" || oType === "video"),
    };
    touchChatPreviewFromMessage(message);
    emitEvent("upload_progress", {
      chatMid,
      progress: 1,
      label: "Sent",
      done: true,
    });
    ok(id, { message });
  } catch (e) {
    emitEvent("upload_progress", { chatMid, progress: 0, label: "", done: true });
    fail(id, e instanceof Error ? e.message : String(e));
  }
}

async function doSendAudio(
  id: number | string | null,
  chatMid: string,
  filePath: string,
  durationMs?: number,
) {
  await doSendMedia(id, chatMid, filePath, "audio", durationMs);
}

function sniffMediaExt(
  buf: Uint8Array,
  hintCt: string,
  fileName?: string | null,
): string {
  if (isImageBytes(buf)) {
    if (buf[0] === 0x89) return "png";
    if (buf[0] === 0x47) return "gif";
    if (
      buf.length > 11 && buf[0] === 0x52 && buf[8] === 0x57 && buf[9] === 0x45
    ) {
      return "webp";
    }
    return "jpg";
  }
  if (
    buf.length > 12 && buf[4] === 0x66 && buf[5] === 0x74 && buf[6] === 0x79 &&
    buf[7] === 0x70
  ) {
    const brand = new TextDecoder().decode(buf.slice(8, 12));
    if (hintCt === "AUDIO" || brand.startsWith("M4A") || brand.includes("mp4a")) {
      return "m4a";
    }
    return "mp4";
  }
  if (fileName && fileName.includes(".")) {
    const ext = fileName.split(".").pop()!.toLowerCase().replace(/[^a-z0-9]/g, "");
    if (ext) return ext.slice(0, 8);
  }
  if (hintCt === "AUDIO") return "m4a";
  if (hintCt === "VIDEO") return "mp4";
  if (hintCt === "IMAGE") return "jpg";
  return "bin";
}

/**
 * Download full media bytes for a message (E2EE-aware) into the local cache.
 * Used for save-as / full-view video / file open.
 */
async function doDownloadMedia(
  id: number | string | null,
  chatMid: string,
  messageId: string,
) {
  if (!client) {
    fail(id, "not_logged_in");
    return;
  }
  if (!chatMid || !messageId) {
    fail(id, "chatMid and messageId required");
    return;
  }
  try {
    let hintCt = "FILE";
    let fileName: string | null = null;
    try {
      const cached = msgCache.get(chatMid)?.messages.find((m) =>
        String(m.id) === messageId
      );
      if (cached) {
        hintCt = normType(cached.contentType);
        if (typeof cached.fileName === "string") fileName = cached.fileName;
        else if (hintCt === "FILE" && typeof cached.text === "string") {
          fileName = cached.text;
        }
      }
    } catch { /* ignore */ }

    // Reuse local full media when already cached.
    const localCandidates = [
      fullDest(messageId, "jpg"),
      fullDest(messageId, "png"),
      fullDest(messageId, "webp"),
      fullDest(messageId, "gif"),
      mediaDest(messageId, "mp4"),
      mediaDest(messageId, "m4a"),
      mediaDest(messageId, "mp3"),
      mediaDest(messageId, "jpg"),
      mediaDest(messageId, "png"),
      mediaDest(messageId, "gif"),
      mediaDest(messageId, "webp"),
      mediaDest(messageId, "bin"),
    ];
    if (fileName) {
      localCandidates.unshift(join(mediaDir, `${messageId}.${fileName}`));
    }
    for (const p of localCandidates) {
      const hit = await existingFile(p);
      if (!hit) continue;
      // Never return UI thumbnails as the download/viewer payload.
      if (hit.includes(".thumb.")) continue;
      try {
        const st = await Deno.stat(hit);
        if (hintCt === "VIDEO" && /\.(jpe?g|png|webp)$/i.test(hit) && st.size < 200_000) {
          continue;
        }
        if (hintCt === "IMAGE") {
          // Skip polluted preview files that were cached into mediaDest.
          if (await isPreviewOrThumbImage(hit)) continue;
        }
        if (st.size >= 256) {
          ok(id, {
            path: hit,
            fileName: fileName || hit.split("/").pop() || messageId,
            contentType: hintCt,
            bytes: st.size,
            cached: true,
          });
          return;
        }
      } catch { /* try next */ }
    }

    const got = await refetchMessageData(chatMid, messageId, {
      allowNonImage: true,
    });
    if (!got || got.buf.length < 32) {
      fail(id, "download_failed");
      return;
    }
    let dest: string;
    if (hintCt === "IMAGE" && isImageBytes(got.buf)) {
      const written = await writeFullImage(messageId, got.buf, got.mime);
      if (!written) {
        fail(id, "download_failed");
        return;
      }
      dest = written;
    } else {
      const ext = sniffMediaExt(got.buf, hintCt, fileName);
      dest = join(
        mediaDir,
        fileName && fileName.includes(".")
          ? `${messageId}.${fileName.replace(/[\/\\]/g, "_")}`
          : `${messageId}.${ext}`,
      );
      await Deno.writeFile(dest, got.buf);
      if (hintCt === "VIDEO" && ext === "mp4") {
        await extractVideoThumb(dest, messageId);
      }
    }
    ok(id, {
      path: dest,
      fileName: fileName || dest.split("/").pop() || messageId,
      contentType: hintCt,
      bytes: got.buf.length,
      cached: false,
    });
  } catch (e) {
    fail(id, e instanceof Error ? e.message : String(e));
  }
}

async function handle(req: Json) {
  const id = (req.id as number | string | null) ?? null;
  const method = String(req.method ?? "");
  const params = (req.params ?? {}) as Json;

  try {
    switch (method) {
      case "ping":
        ok(id, { pong: true, version: "0.2.0" });
        break;
      case "login_qr":
        await doLoginQr(id);
        break;
      case "login_token":
        await doLoginToken(id, params.token as string | undefined);
        break;
      case "list_chats":
        await doListChats(id, !!params.force);
        break;
      case "fetch_messages":
        await doFetchMessages(
          id,
          String(params.chatMid ?? ""),
          Number(params.limit ?? 50),
          !!params.force,
        );
        break;
      case "send_message":
        await doSend(id, String(params.chatMid ?? ""), String(params.text ?? ""));
        break;
      case "send_audio":
        await doSendAudio(
          id,
          String(params.chatMid ?? ""),
          String(params.filePath ?? ""),
          params.durationMs != null ? Number(params.durationMs) : undefined,
        );
        break;
      case "send_media":
        await doSendMedia(
          id,
          String(params.chatMid ?? ""),
          String(params.filePath ?? ""),
          typeof params.oType === "string" ? params.oType : "auto",
          params.durationMs != null ? Number(params.durationMs) : undefined,
        );
        break;
      case "download_media":
        await doDownloadMedia(
          id,
          String(params.chatMid ?? ""),
          String(params.messageId ?? ""),
        );
        break;
      case "send_sticker":
        await doSendSticker(
          id,
          String(params.chatMid ?? ""),
          String(params.stickerId ?? ""),
          String(params.packageId ?? params.stickerPackageId ?? ""),
          typeof params.version === "string" ? params.version : undefined,
        );
        break;
      case "list_stickers":
        await doListStickers(id);
        break;
      case "mark_read":
        await doMarkRead(
          id,
          String(params.chatMid ?? ""),
          String(params.lastMessageId ?? ""),
        );
        break;
      case "call_start":
        setCallAudioDevices(
          typeof params.audioInput === "string" ? params.audioInput : undefined,
          typeof params.audioOutput === "string" ? params.audioOutput : undefined,
        );
        setCallGains(params.micGain, params.spkGain);
        await doCallStart(id, String(params.mid ?? params.chatMid ?? ""));
        break;
      case "call_answer":
        setCallAudioDevices(
          typeof params.audioInput === "string" ? params.audioInput : undefined,
          typeof params.audioOutput === "string" ? params.audioOutput : undefined,
        );
        setCallGains(params.micGain, params.spkGain);
        await doCallAnswer(id);
        break;
      case "call_decline":
        await doCallDecline(id);
        break;
      case "call_end":
        await doCallEnd(id);
        break;
      case "call_set_audio": {
        if (params.muted !== undefined) callAudioCtl.muted = !!params.muted;
        if (params.deafened !== undefined) {
          callAudioCtl.deafened = !!params.deafened;
        }
        if (params.micGain !== undefined) {
          callAudioCtl.micGain = clampGain(params.micGain, callAudioCtl.micGain);
        }
        if (params.spkGain !== undefined) {
          callAudioCtl.spkGain = clampGain(params.spkGain, callAudioCtl.spkGain);
        }
        ok(id, {
          muted: callAudioCtl.muted,
          deafened: callAudioCtl.deafened,
          micGain: callAudioCtl.micGain,
          spkGain: callAudioCtl.spkGain,
        });
        break;
      }
      case "send_postback": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const chatMid = String(params.chatMid ?? "");
        const messageId = String(params.messageId ?? "");
        const data = String(params.data ?? "");
        const uri = params.uri != null ? String(params.uri) : "";
        try {
          await client.base.talk.sendPostback({
            request: {
              messageId,
              url: uri || `line://postback?data=${encodeURIComponent(data)}`,
              chatMID: chatMid,
              originMID: myMid(),
            },
          });
          ok(id, { sent: true });
        } catch (e) {
          // Fallback: send the data/label as a normal chat message.
          try {
            const text = data || uri || "OK";
            try {
              const chat = await client!.getChat(chatMid);
              await chat.sendMessage(text);
            } catch {
              await client!.base.talk.sendMessage({
                to: chatMid,
                text,
                e2ee: true,
              });
            }
            msgCache.delete(chatMid);
            ok(id, { sent: true, fallback: "message" });
          } catch {
            fail(id, e instanceof Error ? e.message : String(e));
          }
        }
        break;
      }
      case "clear_cache": {
        const kind = String(params.kind ?? "all");
        const wipe = async (dir: string) => {
          try {
            for await (const e of Deno.readDir(dir)) {
              if (e.isFile) {
                try {
                  await Deno.remove(join(dir, e.name));
                } catch { /* ignore */ }
              }
            }
          } catch { /* ignore */ }
        };
        if (kind === "media" || kind === "all") await wipe(mediaDir);
        if (kind === "stickers" || kind === "all") await wipe(stickerDir);
        if (kind === "avatars" || kind === "all") await wipe(avatarDir);
        if (kind === "messages" || kind === "all") await wipe(msgDiskDir);
        if (kind === "chats" || kind === "all") {
          try {
            await Deno.remove(chatCachePath);
          } catch { /* ignore */ }
          try {
            await Deno.remove(contactCachePath);
          } catch { /* ignore */ }
          chatCache = null;
          contactIndex.clear();
          contactsAt = 0;
        }
        if (kind === "messages" || kind === "all") msgCache.clear();
        ok(id, { cleared: kind });
        break;
      }
      case "list_friends": {
        await doListFriends(id, !!params.force);
        break;
      }
      case "add_friend": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const userid = String(params.userid ?? "").replace(/^@/, "");
        try {
          const contact = await client.base.talk.findContactByUserid({
            userid,
          });
          const mid = contact?.mid;
          if (!mid) {
            fail(id, "user_not_found");
            break;
          }
          try {
            await client.base.talk.tryFriendRequest({
              mid,
              method: "USERID",
              friendRequestParams: userid,
            });
          } catch {
            /* already friends / different API shape */
          }
          contactIndex.set(String(mid), {
            name: contact.displayName || userid,
            picturePath: contact.picturePath ?? null,
            kind: "dm",
            muted: false,
          });
          await saveDiskContacts();
          ok(id, {
            mid,
            displayName: contact.displayName ?? userid,
          });
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "mute_chat": {
        if (!client) {
          fail(id, "not_logged_in");
          break;
        }
        const mid = String(params.mid ?? params.chatMid ?? "");
        const muted = !!params.muted;
        if (!mid.startsWith("u")) {
          fail(id, "mute_dm_only");
          break;
        }
        try {
          await client.base.talk.updateContactSetting({
            reqSeq: await client.base.getReqseq(),
            mid,
            flag: "CONTACT_SETTING_NOTIFICATION_DISABLE",
            value: muted ? "true" : "false",
          });
          const cur = contactIndex.get(mid);
          if (cur) cur.muted = muted;
          else {
            contactIndex.set(mid, {
              name: mid,
              picturePath: null,
              kind: "dm",
              muted,
            });
          }
          await saveDiskContacts();
          if (chatCache) {
            const row = chatCache.chats.find((c) => c.mid === mid);
            if (row) row.muted = muted;
          }
          emitEvent("chat_mute", { mid, muted });
          ok(id, { mid, muted });
        } catch (e) {
          fail(id, e instanceof Error ? e.message : String(e));
        }
        break;
      }
      case "logout":
        cancelBackgroundWork();
        ownedPackagesCache = null;
        try {
          await clearAuth();
        } catch { /* ignore */ }
        client = null;
        listening = false;
        chatCache = null;
        msgCache.clear();
        ok(id, {});
        break;
      default:
        fail(id, `unknown_method:${method}`);
    }
  } catch (e) {
    fail(id, e instanceof Error ? e.message : String(e));
  }
}

await loadDiskContacts();

const bootAuth = await loadAuth();
const bootDevice = await loadAuthDevice();
const needsAndroidRelogin = !!bootAuth && bootDevice === "DESKTOPWIN";

emitEvent("ready", {
  dataDir,
  sidecar: fromFileUrl(import.meta.url),
  hasAuth: !!bootAuth && !needsAndroidRelogin,
  device: needsAndroidRelogin ? "" : bootDevice,
});

// Legacy DESKTOPWIN tokens cannot do reliable PLANET calls (docs require ANDROID*).
if (needsAndroidRelogin) {
  console.error(
    "[auth] DESKTOPWIN session detected — clearing auth. Re-scan QR for ANDROIDSECONDARY (required for voice calls per linejs docs).",
  );
  await clearAuth();
  emitEvent("session_failed", {
    error: "relogin_android_required",
  });
} else if (bootAuth) {
  try {
    const storage = new FileStorage(storagePath);
    const device =
      bootDevice === "ANDROID" || bootDevice === "ANDROIDSECONDARY"
        ? bootDevice
        : "ANDROIDSECONDARY";
    client = await loginWithAuthToken(bootAuth, {
      device,
      version: LINE_VERSION,
      storage,
    });
    await saveAuth(client.authToken, device);
    const profile = await client.getMyProfile();
    patchE2eeGuards();
    await startListen();
    emitEvent("session", await myProfilePayload(profile));
  } catch (e) {
    console.error("[boot restore]", e);
    emitEvent("session_failed", {
      error: e instanceof Error ? e.message : String(e),
    });
  }
}

const decoder = new TextDecoder();
let buf = "";
for await (const chunk of Deno.stdin.readable) {
  buf += decoder.decode(chunk, { stream: true });
  while (true) {
    const nl = buf.indexOf("\n");
    if (nl < 0) break;
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (!line) continue;
    try {
      handle(JSON.parse(line) as Json);
    } catch (e) {
      fail(null, `bad_json:${e}`);
    }
  }
}

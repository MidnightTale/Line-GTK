import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { atomicWriteTextFile } from "./storage.ts";
import type { CachePolicy } from "./cache_policy.ts";
import { stickerPackageId } from "./sticker_import.ts";
import type { createStickerLibrary } from "./sticker_library.ts";

type Json = Record<string, unknown>;

export type StickerRuntime = {
  getClient: () => Client | null;
  library: ReturnType<typeof createStickerLibrary>;
  dataDir: string;
  stickerDir: string;
  refreshCachePolicy: () => Promise<CachePolicy>;
  cachePolicy: () => CachePolicy;
  cacheUrl: (url: string, dest: string) => Promise<string | null>;
  existingImage: (path: string) => Promise<string | null>;
  sentMessagePayload: (
    sent:
      | { id?: unknown; createdTime?: unknown; contentType?: unknown }
      | null
      | undefined,
    chatMid: string,
    text: string,
  ) => Json;
  invalidateMessages: (chatMid: string) => void;
  invalidateChats: () => void;
  touchChatPreviewFromMessage: (message: Json) => void;
  ok: (id: number | string | null, result?: unknown) => void;
  fail: (id: number | string | null, error: string) => void;
};

export function createStickerService(runtime: StickerRuntime) {
  const {
    dataDir,
    stickerDir,
    refreshCachePolicy,
    cachePolicy,
    cacheUrl,
    existingImage,
    sentMessagePayload,
    invalidateMessages,
    invalidateChats,
    touchChatPreviewFromMessage,
    ok,
    fail,
  } = runtime;

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
      await atomicWriteTextFile(
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
      if (packageId) {
        return !(s.stickerId === stickerId && s.packageId === packageId);
      }
      return s.stickerId !== stickerId;
    });
    void saveStickerIndex();
  }

  type OwnedPackage = { id: string; version: string; name: string };
  let ownedPackagesCache: {
    client: Client;
    at: number;
    packages: OwnedPackage[];
  } | null = null;

  let ownedUnavailable = false;

  async function fetchOwnedPackages(): Promise<OwnedPackage[]> {
    const check = runtime.library.guard();
    const client = runtime.getClient();
    if (!client) return [];
    await refreshCachePolicy();
    check();
    if (
      ownedPackagesCache && ownedPackagesCache.client === client &&
      Date.now() - ownedPackagesCache.at < cachePolicy().ownedPack
    ) {
      return ownedPackagesCache.packages;
    }
    try {
      const { ShopService } = await import("@linegtk/shop");
      const shop = new ShopService(client.base as never);
      const packages: OwnedPackage[] = [];
      const seen = new Set<string>();
      let offset = 0;
      while (true) {
        const res = await shop.getOwnedProductSummaries({
          shopId: "stickershop",
          offset,
          limit: 80,
          locale: Deno.env.get("LINE_GTK_LANG") === "ja"
            ? { language: "ja", country: "JP" }
            : { language: "en", country: "TH" },
        } as never) as Record<string, unknown>;
        if (runtime.getClient() !== client) {
          throw new Error("sticker_library_session_changed");
        }
        check();
        const list = (res.productList ?? res["1"] ?? []) as Record<
          string,
          unknown
        >[];
        let added = 0;
        for (const p of list) {
          const id = stickerPackageId(String(p.id ?? p["1"] ?? ""));
          if (!id || seen.has(id)) continue;
          seen.add(id);
          added++;
          packages.push({
            id,
            version: String(p.latestVersion ?? p["21"] ?? "1"),
            name: String(p.name ?? p["11"] ?? id),
          });
        }
        const total = Number(res.totalSize ?? res["3"]);
        offset += list.length;
        if (
          !list.length || (Number.isFinite(total) && offset >= total) ||
          (!Number.isFinite(total) && list.length < 80)
        ) break;
        if (!added) throw new Error("sticker_owned_pagination_stalled");
      }
      ownedUnavailable = false;
      ownedPackagesCache = { client, at: Date.now(), packages };
      return packages;
    } catch (e) {
      check();
      ownedUnavailable = true;
      console.error("[owned stickers]", e);
      return ownedPackagesCache?.client === client
        ? ownedPackagesCache.packages
        : [];
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
      const preferJa = lang === "ja" || lang === "ja-jp" || lang === "ja_jp";
      let name = pkg.name || pkg.id;
      if (typeof meta.title === "string" && meta.title.trim()) {
        name = meta.title.trim();
      } else if (meta.title && typeof meta.title === "object") {
        name = String(
          (preferJa
            ? meta.title.ja
            : preferEn
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
        .filter((s) => stickerPackageId(s.stickerId) === s.stickerId);
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

  async function ensureStickerAnimation(
    stickerId: string,
  ): Promise<string | null> {
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
      await atomicWriteTextFile(missDest, "");
    } catch (error) {
      console.error("[sticker-animation-miss-cache]", error);
    }
    return null;
  }

  async function ensureStickerStatic(
    stickerId: string,
  ): Promise<string | null> {
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

  async function doListStickers(
    id: number | string | null,
    importPack?: string,
  ) {
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    const check = runtime.library.guard();
    const library = importPack !== undefined
      ? await runtime.library.importStore(String(id), importPack)
      : await runtime.library.list();
    // Most-used packs first: more distinct stickers used in the index wins,
    // then the most recently used. Unused packs keep LINE's original order
    // (Array.prototype.sort is stable).
    const packUse = new Map<string, { count: number; recent: number }>();
    stickerIndex.forEach((s, position) => {
      const use = packUse.get(s.packageId) ??
        { count: 0, recent: position };
      use.count += 1;
      use.recent = Math.min(use.recent, position);
      packUse.set(s.packageId, use);
    });
    const ownedPkgs = [...await fetchOwnedPackages()].sort((a, b) => {
      const ua = packUse.get(a.id);
      const ub = packUse.get(b.id);
      if (!ua || !ub) return ua ? -1 : ub ? 1 : 0;
      return ub.count - ua.count || ua.recent - ub.recent;
    });
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

    const packs: Json[] = [...library.packs];

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

    // Enumerate every owned collection; fetch its grid only when selected.
    const ownedIcons = await Promise.all(
      ownedPkgs.map((pkg) => ensurePackIcon(pkg.id).catch(() => null)),
    );
    ownedPkgs.forEach((pkg, index) => {
      packs.push({
        id: pkg.id,
        name: pkg.name || pkg.id,
        version: pkg.version,
        iconPath: ownedIcons[index],
        lazy: true,
        stickers: [],
      });
    });
    if (runtime.getClient() !== client) {
      throw new Error("sticker_library_session_changed");
    }

    check();
    ok(id, {
      ...library,
      packs,
      // Keep flat list for older UI paths (unused once chooser migrates).
      stickers: packs.flatMap((p) =>
        ((p.stickers as Json[]) ?? []).map((s) => ({
          ...s,
          recent: p.id === "__recent__",
        }))
      ),
      ownedPackages: ownedPkgs.length,
      ownedUnavailable,
    });
  }

  async function ownedPack(id: number | string | null, packageId: string) {
    const check = runtime.library.guard();
    const client = runtime.getClient();
    const pkg = (await fetchOwnedPackages()).find((p) => p.id === packageId);
    if (!pkg) {
      fail(id, "sticker_not_owned");
      return;
    }
    const detail = await stickersForOwnedPackage(pkg, 2000);
    if (!detail.stickers.length) {
      fail(id, "sticker_import_unavailable");
      return;
    }
    const stickers = [];
    for (let offset = 0; offset < detail.stickers.length; offset += 3) {
      stickers.push(
        ...await Promise.all(
          detail.stickers.slice(offset, offset + 3).map(async (s) => ({
            ...s,
            imagePath: await ensureStickerStatic(s.stickerId),
          })),
        ),
      );
      if (runtime.getClient() !== client) {
        throw new Error("sticker_library_session_changed");
      }
    }
    const iconPath = await ensurePackIcon(packageId);
    check();
    ok(id, {
      id: packageId,
      name: detail.name,
      stickers,
      iconPath,
    });
  }

  async function doSendSticker(
    id: number | string | null,
    chatMid: string,
    stickerId: string,
    packageId: string,
    version?: string,
  ) {
    const client = runtime.getClient();
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

    let sent:
      | { id?: unknown; createdTime?: unknown; contentType?: unknown }
      | null = null;
    try {
      // Stickers are sent as contentType+metadata (not E2EE text chunks).
      sent = await client.base.talk.sendMessage({
        to: chatMid,
        contentType: "STICKER",
        contentMetadata: meta,
        e2ee: false,
      }) as unknown as typeof sent;
    } catch (e1) {
      try {
        sent = await client.base.talk.sendMessage({
          to: chatMid,
          contentType: "STICKER",
          contentMetadata: meta,
        }) as unknown as typeof sent;
      } catch (e2) {
        const raw = e2 instanceof Error
          ? e2.message
          : e1 instanceof Error
          ? e1.message
          : String(e1);
        if (
          raw.includes("USER_NOT_STICKER_OWNER") || raw.includes("not owned")
        ) {
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
    invalidateMessages(chatMid);
    invalidateChats();
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

  return {
    loadIndex: loadStickerIndex,
    remember: rememberSticker,
    forget: forgetSticker,
    ensureAnimation: ensureStickerAnimation,
    ensureStatic: ensureStickerStatic,
    ensureImage: ensureStickerImage,
    animationUrl: (stickerId: string) => stickerAnimationUrls(stickerId)[0],
    list: doListStickers,
    ownedPack,
    send: doSendSticker,
    resetOwnedCache: () => {
      ownedPackagesCache = null;
    },
  };
}

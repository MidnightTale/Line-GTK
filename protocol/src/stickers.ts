import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { atomicWriteTextFile } from "./storage.ts";
import type { CachePolicy } from "./cache_policy.ts";

type Json = Record<string, unknown>;

export type StickerRuntime = {
  getClient: () => Client | null;
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
  emitEvent: (event: string, payload?: Json) => void;
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
    emitEvent,
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

  function sticonUrl(productId: string, sticonId: string): string {
    return `https://stickershop.line-scdn.net/sticonshop/v1/sticon/${
      encodeURIComponent(productId)
    }/android/${encodeURIComponent(sticonId)}.png`;
  }

  async function ensureSticonImage(
    productId: string,
    sticonId: string,
  ): Promise<string | null> {
    if (!productId || !sticonId) return null;
    const safeProduct = productId.replace(/[^a-zA-Z0-9._-]/g, "_");
    const safeSticon = sticonId.replace(/[^a-zA-Z0-9._-]/g, "_");
    const dest = join(
      stickerDir,
      `sticon-${safeProduct}-${safeSticon}.png`,
    );
    const existing = await existingImage(dest);
    if (existing) return existing;
    return await cacheUrl(sticonUrl(productId, sticonId), dest);
  }

  type StickerEntry = {
    stickerId: string;
    packageId: string;
    version?: string;
    at: number;
  };

  type StickerPackCatalog = {
    id: string;
    name: string;
    version: string;
    stickers: StickerEntry[];
  };

  type StickerCatalog = {
    at: number;
    packs: StickerPackCatalog[];
  };

  const stickerIndexPath = join(dataDir, "stickers-index.json");
  const stickerCatalogPath = join(dataDir, "stickers-catalog.json");
  let stickerIndex: StickerEntry[] = [];
  let stickerCatalog: StickerCatalog | null = null;
  let catalogRefresh: Promise<void> | null = null;

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
    try {
      const raw = JSON.parse(await Deno.readTextFile(stickerCatalogPath));
      if (Array.isArray(raw?.packs)) {
        stickerCatalog = {
          at: Number(raw.at ?? 0),
          packs: raw.packs.filter((pack: StickerPackCatalog) =>
            pack?.id && Array.isArray(pack.stickers)
          ),
        };
      }
    } catch {
      stickerCatalog = null;
    }
  }

  async function mapPool<T, R>(
    items: T[],
    concurrency: number,
    worker: (item: T, index: number) => Promise<R>,
  ): Promise<R[]> {
    const out = new Array<R>(items.length);
    let cursor = 0;
    const runners = Array.from(
      { length: Math.min(concurrency, items.length) },
      async () => {
        while (cursor < items.length) {
          const index = cursor++;
          out[index] = await worker(items[index]!, index);
        }
      },
    );
    await Promise.all(runners);
    return out;
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
  let ownedPackagesCache: { at: number; packages: OwnedPackage[] } | null =
    null;

  async function fetchOwnedPackages(): Promise<OwnedPackage[]> {
    const client = runtime.getClient();
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
      const list = (res.productList ?? res["1"] ?? []) as Record<
        string,
        unknown
      >[];
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

  function packIconPath(packageId: string): string {
    return join(stickerDir, `pack-${packageId}.png`);
  }

  function stickerStaticPath(stickerId: string): string {
    return join(stickerDir, `${stickerId}.png`);
  }

  async function ensurePackIcon(packageId: string): Promise<string | null> {
    const dest = packIconPath(packageId);
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
    const dest = stickerStaticPath(stickerId);
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

  async function saveStickerCatalog() {
    if (!stickerCatalog) return;
    try {
      await atomicWriteTextFile(
        stickerCatalogPath,
        JSON.stringify(stickerCatalog),
      );
    } catch (error) {
      console.error("[sticker catalog]", error);
    }
  }

  async function refreshStickerCatalog(force = false): Promise<boolean> {
    await refreshCachePolicy();
    if (
      !force && stickerCatalog &&
      Date.now() - stickerCatalog.at < cachePolicy().ownedPack
    ) return false;
    if (catalogRefresh) {
      await catalogRefresh;
      return true;
    }
    catalogRefresh = (async () => {
      const owned = await fetchOwnedPackages();
      const details = await mapPool(
        owned,
        8,
        (pkg) => stickersForOwnedPackage(pkg),
      );
      stickerCatalog = {
        at: Date.now(),
        packs: details.map((detail, index) => ({
          id: owned[index]!.id,
          name: detail.name || owned[index]!.name || owned[index]!.id,
          version: detail.version,
          stickers: detail.stickers,
        })).filter((pack) => pack.stickers.length > 0),
      };
      await saveStickerCatalog();
    })().finally(() => {
      catalogRefresh = null;
    });
    await catalogRefresh;
    return true;
  }

  async function stickerCatalogPayload(): Promise<Json> {
    const catalog = stickerCatalog ?? { at: 0, packs: [] };
    const ownedIds = new Set(catalog.packs.map((pack) => pack.id));
    const versionByPack = new Map(
      catalog.packs.map((pack) => [pack.id, pack.version]),
    );
    const recent: StickerEntry[] = [];
    const recentSeen = new Set<string>();
    for (const sticker of stickerIndex) {
      if (!ownedIds.has(sticker.packageId)) continue;
      const key = `${sticker.packageId}:${sticker.stickerId}`;
      if (recentSeen.has(key)) continue;
      recentSeen.add(key);
      recent.push({
        ...sticker,
        version: sticker.version || versionByPack.get(sticker.packageId),
      });
      if (recent.length >= 24) break;
    }

    const packs = await mapPool(catalog.packs, 12, async (pack) => {
      const stickers = await mapPool(pack.stickers, 32, async (sticker) => ({
        stickerId: sticker.stickerId,
        packageId: sticker.packageId,
        version: sticker.version ?? pack.version,
        imagePath: await existingImage(stickerStaticPath(sticker.stickerId)),
      }));
      const iconPath = await existingImage(packIconPath(pack.id)) ??
        stickers.find((sticker) => sticker.imagePath)?.imagePath ?? null;
      return {
        id: pack.id,
        name: pack.name,
        version: pack.version,
        iconPath,
        recent: false,
        stickers,
      };
    });

    if (recent.length) {
      const stickers = await mapPool(recent, 24, async (sticker) => ({
        stickerId: sticker.stickerId,
        packageId: sticker.packageId,
        version: sticker.version ?? "",
        imagePath: await existingImage(stickerStaticPath(sticker.stickerId)),
      }));
      packs.unshift({
        id: "__recent__",
        name: "Recent",
        version: "",
        iconPath: stickers[0]?.imagePath ?? null,
        recent: true,
        stickers,
      });
    }

    return {
      packs,
      stickers: packs.flatMap((pack) =>
        pack.stickers.map((sticker) => ({
          ...sticker,
          recent: pack.id === "__recent__",
        }))
      ),
      ownedPackages: catalog.packs.length,
      cached: true,
    };
  }

  let thumbnailWarm: Promise<void> | null = null;
  function warmStickerThumbnails() {
    if (!stickerCatalog || thumbnailWarm) return;
    const catalog = stickerCatalog;
    thumbnailWarm = (async () => {
      const priorityIds = [
        ...stickerIndex.slice(0, 24).map((sticker) => sticker.stickerId),
        ...(catalog.packs[0]?.stickers ?? []).map((sticker) =>
          sticker.stickerId
        ),
      ];
      const allIds = catalog.packs.flatMap((pack) =>
        pack.stickers.map((sticker) => sticker.stickerId)
      );
      const seen = new Set<string>();
      const priority = priorityIds.filter((id) =>
        id && !seen.has(id) && seen.add(id)
      );
      const rest = allIds.filter((id) => id && !seen.has(id) && seen.add(id));

      await Promise.all([
        mapPool(catalog.packs, 12, (pack) => ensurePackIcon(pack.id)),
        mapPool(priority, 24, (stickerId) => ensureStickerStatic(stickerId)),
      ]);
      emitEvent("stickers_updated", await stickerCatalogPayload());
      await mapPool(rest, 24, (stickerId) => ensureStickerStatic(stickerId));
      emitEvent("stickers_updated", await stickerCatalogPayload());
    })().catch((error) => {
      console.error("[sticker thumbnail warm]", error);
    }).finally(() => {
      thumbnailWarm = null;
    });
  }

  async function doListStickers(id: number | string | null) {
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    try {
      if (!stickerCatalog?.packs.length) {
        await refreshStickerCatalog(true);
      }
      ok(id, await stickerCatalogPayload());
      warmStickerThumbnails();
      if (stickerCatalog) {
        void refreshStickerCatalog(false).then(async (updated) => {
          if (!updated) return;
          emitEvent("stickers_updated", await stickerCatalogPayload());
          warmStickerThumbnails();
        });
      }
    } catch (error) {
      fail(id, error instanceof Error ? error.message : String(error));
    }
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
    ensureSticon: ensureSticonImage,
    sticonUrl,
    list: doListStickers,
    send: doSendSticker,
    resetOwnedCache: () => {
      ownedPackagesCache = null;
    },
  };
}

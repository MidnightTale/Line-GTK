import { createHash } from "node:crypto";
import { dirname, join } from "@std/path";
import { atomicWriteTextFile, ensurePrivateDir } from "./storage.ts";
import {
  optimizationOptions,
  optimizeStickerImage,
  type StickerOptimization,
} from "./sticker_optimize.ts";
import { stickerPackageId } from "./sticker_import.ts";
import {
  MAX_PNG_BYTES,
  readStickerPng,
  validateStickerPng,
} from "./sticker_png.ts";

type Session = { accountMid: string; owner: object };
type Json = Record<string, unknown>;
type Item = {
  id: string;
  stickerId?: string;
  digest?: string;
  error?: string;
  animated?: boolean;
  wantsAnimation?: boolean;
  optimization?: StickerOptimization;
};
type Pack = {
  id: string;
  name: string;
  source: "line_store" | "local_png";
  packageId?: string;
  version?: string;
  items: Item[];
  excludedItems?: string[];
};
type Manifest = {
  version: 1;
  accountMid: string;
  adoptedLegacy?: boolean;
  packs: Pack[];
};
type Job = { session: Session; controller: AbortController; packId?: string };
export type StickerLibraryRuntime = {
  dataDir: string;
  stickerDir: string;
  session: () => Session | null;
  emitEvent: (event: string, data: Json) => void;
  fetch?: typeof fetch;
};
const hash = (value: string | Uint8Array) =>
  createHash("sha256").update(value).digest("hex");
const validId = (value: unknown): value is string =>
  typeof value === "string" &&
  /^(?:store-[1-9]\d{0,19}|[a-f0-9-]{36})$/.test(value);
const validDigest = (value: unknown): value is string =>
  typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
const failure = (e: unknown) =>
  e instanceof Error && e.message.startsWith("sticker_")
    ? e.message
    : "sticker_import_unavailable";

/** One account-scoped owner for durable image packs; native stickers stay separate. */
export function createStickerLibrary(runtime: StickerLibraryRuntime) {
  const jobs = new Map<string, Job>();
  const previewTasks = new Map<Promise<unknown>, AbortController>();
  const queues = new Map<string, Promise<unknown>>();
  const request = runtime.fetch ??
    ((...args) => fetch(...args)) as typeof fetch;

  function session(): Session {
    const current = runtime.session();
    if (!current?.accountMid) throw new Error("sticker_library_not_logged_in");
    return current;
  }
  function check(s: Session, signal?: AbortSignal) {
    if (signal?.aborted) throw new Error("sticker_import_cancelled");
    const current = runtime.session();
    if (
      !current || current.accountMid !== s.accountMid ||
      current.owner !== s.owner
    ) {
      throw new Error("sticker_library_session_changed");
    }
  }
  const directory = (s: Session) =>
    join(runtime.dataDir, "sticker-library", hash(s.accountMid));
  const original = (s: Session, digest: string) =>
    join(directory(s), "originals", `${digest}.png`);
  const manifestPath = (s: Session) => join(directory(s), "index.json");

  async function load(s: Session): Promise<Manifest> {
    try {
      const m = JSON.parse(
        await Deno.readTextFile(manifestPath(s)),
      ) as Manifest;
      check(s);
      if (
        m.version !== 1 || m.accountMid !== s.accountMid ||
        !Array.isArray(m.packs)
      ) throw new Error();
      const seen = new Set<string>();
      for (const p of m.packs) {
        if (
          !validId(p.id) || seen.has(p.id) || typeof p.name !== "string" ||
          !Array.isArray(p.items) ||
          !["line_store", "local_png"].includes(p.source) ||
          (p.source === "line_store" &&
            (stickerPackageId(p.packageId ?? "") !== p.packageId ||
              p.id !== `store-${p.packageId}`))
        ) throw new Error();
        if (
          p.excludedItems &&
          (!Array.isArray(p.excludedItems) ||
            !p.excludedItems.every((id) => stickerPackageId(id) === id))
        ) throw new Error();
        seen.add(p.id);
        const ids = new Set<string>();
        for (const i of p.items) {
          if (
            typeof i.id !== "string" ||
            !/^(?:[1-9]\d{0,19}|[a-f0-9-]{36})$/.test(i.id) || ids.has(i.id) ||
            (i.digest !== undefined && !validDigest(i.digest)) ||
            (i.stickerId !== undefined &&
              stickerPackageId(i.stickerId) !== i.stickerId)
          ) throw new Error();
          if (i.optimization) optimizationOptions(i.optimization);
          if (
            (i.animated !== undefined && typeof i.animated !== "boolean") ||
            (i.wantsAnimation !== undefined &&
              typeof i.wantsAnimation !== "boolean")
          ) throw new Error();
          ids.add(i.id);
        }
      }
      return m;
    } catch (e) {
      if (e instanceof Deno.errors.NotFound) {
        return { version: 1, accountMid: s.accountMid, packs: [] };
      }
      check(s);
      throw new Error("sticker_library_invalid");
    }
  }
  async function save(s: Session, m: Manifest, signal?: AbortSignal) {
    check(s, signal);
    await atomicWriteTextFile(
      manifestPath(s),
      JSON.stringify(m),
      0o600,
      () => check(s, signal),
    );
    check(s, signal);
  }
  function exclusive<T>(s: Session, work: () => Promise<T>): Promise<T> {
    const previous = queues.get(s.accountMid) ?? Promise.resolve();
    const next = previous.catch(() => {}).then(() => {
      check(s);
      return work();
    });
    queues.set(s.accountMid, next);
    void next.finally(() => {
      if (queues.get(s.accountMid) === next) queues.delete(s.accountMid);
    }).catch(() => {});
    return next;
  }
  async function store(s: Session, bytes: Uint8Array, signal?: AbortSignal) {
    validateStickerPng(bytes, true);
    const digest = hash(bytes);
    check(s, signal);
    await atomicWriteTextFile(
      original(s, digest),
      bytes,
      0o600,
      () => check(s, signal),
    );
    check(s, signal);
    return digest;
  }
  async function cachePreview(
    s: Session,
    bytes: Uint8Array,
    signal: AbortSignal,
  ) {
    validateStickerPng(bytes, true);
    const path = join(
      runtime.stickerDir,
      `preview-${hash(s.accountMid)}-${hash(bytes)}.png`,
    );
    check(s, signal);
    await atomicWriteTextFile(path, bytes, 0o600, () => check(s, signal));
    check(s, signal);
    return path;
  }
  function previewTask<T>(
    work: (signal: AbortSignal) => Promise<T>,
  ): Promise<T> {
    const controller = new AbortController();
    const task = (async () => {
      try {
        return await work(controller.signal);
      } catch (error) {
        if (controller.signal.aborted) {
          throw new Error("sticker_import_cancelled");
        }
        throw error;
      }
    })();
    previewTasks.set(task, controller);
    void task.finally(() => previewTasks.delete(task)).catch(() => {});
    return task;
  }
  async function available(s: Session, item: Item) {
    if (!item.digest) return null;
    try {
      const stat = await Deno.stat(original(s, item.digest));
      return stat.isFile ? original(s, item.digest) : null;
    } catch (e) {
      if (e instanceof Deno.errors.NotFound) return null;
      throw e;
    }
  }
  async function imageOptimization(item: Item, imagePath: string | null) {
    if (item.optimization) return item.optimization;
    if (item.animated && imagePath) {
      try {
        // Originals were validated on import. Read just the PNG header so opening
        // the picker does not decode every animation, including older libraries.
        using file = await Deno.open(imagePath);
        const header = new Uint8Array(24);
        let offset = 0;
        while (offset < header.length) {
          const count = await file.read(header.subarray(offset));
          if (count === null) break;
          offset += count;
        }
        if (offset === header.length) {
          const view = new DataView(header.buffer);
          return optimizationOptions(undefined, true, {
            width: view.getUint32(16),
            height: view.getUint32(20),
          });
        }
      } catch (e) {
        if (!(e instanceof Deno.errors.NotFound)) throw e;
      }
    }
    return optimizationOptions(undefined, !!item.animated);
  }
  async function response(
    s: Session,
    m: Manifest,
    extra: {
      errors?: { index: number; error: string }[];
      added?: number;
      cancelled?: boolean;
      migratedOptions?: Record<string, string>;
    } = {},
  ) {
    const packs = [];
    for (const pack of m.packs) {
      const stickers = [];
      for (const item of pack.items) {
        const imagePath = await available(s, item);
        stickers.push({
          stickerId: item.id,
          packageId: pack.id,
          imagePath,
          optionKey: `${hash(s.accountMid)}/${pack.id}/${item.id}`,
          personal: pack.source === "local_png",
          animated: !!item.animated,
          optimized: !!item.optimization,
          optimization: await imageOptimization(item, imagePath),
          error: imagePath ? null : item.error ?? "sticker_import_unavailable",
        });
      }
      packs.push({
        id: pack.id,
        name: pack.name,
        source: pack.source,
        sourceUrl: pack.packageId
          ? `https://store.line.me/stickershop/product/${pack.packageId}/en`
          : null,
        importPack: pack.packageId,
        imageOnly: true,
        incomplete: stickers.some((i) => !i.imagePath) ||
          pack.items.some((i) => i.wantsAnimation && !i.animated),
        iconPath: stickers.find((i) => i.imagePath)?.imagePath ?? null,
        stickers,
      });
    }
    if (m.adoptedLegacy && extra.migratedOptions === undefined) {
      try {
        extra.migratedOptions = JSON.parse(
          await Deno.readTextFile(join(directory(s), "legacy-options.json")),
        );
      } catch (e) {
        if (!(e instanceof Deno.errors.NotFound)) throw e;
      }
    }
    check(s);
    return {
      accountMid: s.accountMid,
      packs,
      legacyAvailable: !m.adoptedLegacy && await legacyExists(),
      ...extra,
    };
  }
  async function legacyExists() {
    try {
      return (await Deno.stat(join(runtime.stickerDir, "image-packs.json")))
        .isFile;
    } catch (e) {
      if (e instanceof Deno.errors.NotFound) return false;
      throw e;
    }
  }
  async function network(
    url: string,
    signal: AbortSignal,
    limit: number,
  ): Promise<Uint8Array> {
    const r = await request(url, {
      signal: AbortSignal.any([signal, AbortSignal.timeout(25_000)]),
      redirect: "error",
    });
    if (!r.ok || !r.body) {
      await r.body?.cancel();
      throw new Error("sticker_import_unavailable");
    }
    const reader = r.body.getReader();
    const parts: Uint8Array[] = [];
    let size = 0;
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        size += value.length;
        if (size > limit) throw new Error("sticker_png_too_large");
        parts.push(value);
      }
    } finally {
      await reader.cancel().catch(() => {});
      reader.releaseLock();
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const part of parts) {
      bytes.set(part, offset);
      offset += part.length;
    }
    return bytes;
  }
  async function metadata(input: string, signal: AbortSignal): Promise<Pack> {
    const id = stickerPackageId(input);
    if (!id) throw new Error("sticker_import_invalid");
    const data = await network(
      `https://stickershop.line-scdn.net/stickershop/v1/product/${id}/android/productInfo.meta`,
      signal,
      1024 * 1024,
    );
    let meta;
    try {
      meta = JSON.parse(new TextDecoder().decode(data));
    } catch {
      throw new Error("sticker_import_unavailable");
    }
    if (!meta || typeof meta !== "object") {
      throw new Error("sticker_import_unavailable");
    }
    const lang = (Deno.env.get("LINE_GTK_LANG") ?? "en").slice(0, 2);
    const title = typeof meta.title === "string"
      ? meta.title
      : meta.title?.[lang] ?? meta.title?.en ?? meta.title?.ja ?? id;
    if (
      !Array.isArray(meta.stickers) || !meta.stickers.length ||
      meta.stickers.length > 2000
    ) throw new Error("sticker_import_unavailable");
    const ids = new Set<string>();
    for (const item of meta.stickers) {
      const sid = stickerPackageId(String(item?.id ?? ""));
      if (sid) ids.add(sid);
    }
    if (!ids.size) throw new Error("sticker_import_unavailable");
    return {
      id: `store-${id}`,
      name: String(title).slice(0, 200),
      source: "line_store",
      packageId: id,
      version: String(meta.version ?? "1"),
      items: [...ids].map((sid) => ({
        id: sid,
        stickerId: sid,
        wantsAnimation: meta.hasAnimation === true ||
          meta.stickerResourceType === "ANIMATION",
      })),
    };
  }
  async function download(
    s: Session,
    item: Item,
    signal: AbortSignal,
    preview = false,
  ): Promise<string | null> {
    for (
      const suffix of [
        ...(item.wantsAnimation
          ? [
            "android/sticker_animation.png",
            "ANDROID/sticker_animation.png",
            "iPhone/sticker_animation@2x.png",
          ]
          : []),
        "android/sticker.png",
        "iPhone/sticker@2x.png",
        "ANDROID/sticker.png",
      ]
    ) {
      try {
        const bytes = await network(
          `https://stickershop.line-scdn.net/stickershop/v1/sticker/${item.stickerId}/${suffix}`,
          signal,
          MAX_PNG_BYTES,
        );
        const info = validateStickerPng(bytes, true);
        if (suffix.includes("sticker_animation") && !info.animated) {
          throw new Error("sticker_png_invalid");
        }
        if (preview) return await cachePreview(s, bytes, signal);
        item.animated = info.animated;
        item.digest = await store(s, bytes, signal);
        delete item.error;
        return original(s, item.digest);
      } catch (e) {
        check(s, signal);
        item.error = failure(e);
      }
    }
    return null;
  }
  function progress(id: string, s: Session, done: number, total: number) {
    check(s);
    runtime.emitEvent("sticker_import_progress", {
      requestId: Number(id),
      accountMid: s.accountMid,
      done,
      total,
    });
  }
  async function run<T>(
    id: string,
    s: Session,
    packId: string | undefined,
    work: (signal: AbortSignal) => Promise<T>,
  ) {
    if (jobs.has(id)) throw new Error("sticker_import_busy");
    const job = { session: s, controller: new AbortController(), packId };
    jobs.set(id, job);
    try {
      return await exclusive(s, () => work(job.controller.signal));
    } finally {
      jobs.delete(id);
    }
  }
  async function preview(input: string, cancellation: AbortSignal) {
    const s = session();
    const signal = AbortSignal.any([cancellation, AbortSignal.timeout(30_000)]);
    const pack = await metadata(input, signal);
    check(s);
    const paths: string[] = [];
    for (const item of pack.items.slice(0, 4)) {
      const path = await download(s, item, signal, true);
      if (path) paths.push(path);
    }
    check(s);
    return {
      accountMid: s.accountMid,
      name: pack.name,
      packageId: pack.packageId,
      count: pack.items.length,
      previews: paths,
    };
  }
  async function importStore(id: string, input: string) {
    const s = session();
    const packageId = stickerPackageId(input);
    if (!packageId) throw new Error("sticker_import_invalid");
    return await run(id, s, `store-${packageId}`, async (signal) => {
      const incoming = await metadata(input, signal);
      const m = await load(s);
      const previous = m.packs.find((p) => p.id === incoming.id);
      incoming.excludedItems = previous?.excludedItems ?? [];
      incoming.items = incoming.items.filter((item) =>
        !incoming.excludedItems!.includes(item.id)
      );
      for (const item of incoming.items) {
        const old = previous?.items.find((i) => i.id === item.id);
        if (old && await available(s, old)) {
          item.digest = old.digest;
          item.animated = old.animated;
          item.optimization = old.optimization;
        }
      }
      // Keep working items if the provider temporarily omits them on refresh.
      incoming.items.push(
        ...(previous?.items.filter((old) =>
          !incoming.items.some((i) => i.id === old.id)
        ) ?? []),
      );
      if (previous) incoming.name = previous.name;
      m.packs = [incoming, ...m.packs.filter((p) => p.id !== incoming.id)];
      await save(s, m, signal);
      let done = 0;
      progress(id, s, done, incoming.items.length);
      try {
        for (let offset = 0; offset < incoming.items.length; offset += 3) {
          const outcomes = await Promise.allSettled(
            incoming.items.slice(offset, offset + 3).map(async (item) => {
              check(s, signal);
              if (
                !await available(s, item) ||
                (item.wantsAnimation && !item.animated)
              ) await download(s, item, signal);
              check(s, signal);
              progress(id, s, ++done, incoming.items.length);
            }),
          );
          const failed = outcomes.find((r) => r.status === "rejected");
          if (failed?.status === "rejected") throw failed.reason;
          await save(s, m, signal);
        }
      } catch (e) {
        if (signal.aborted && runtime.session()?.owner === s.owner) {
          await save(s, m);
          return await response(s, m, { cancelled: true });
        }
        throw e;
      }
      return await response(s, m);
    });
  }
  async function importPngs(
    id: string,
    paths: string[],
    name: string,
    packId?: string,
    expectedDigests?: string[],
  ) {
    const s = session();
    if (!paths.length || paths.length > 100) {
      throw new Error("sticker_png_batch");
    }
    return await run(id, s, packId, async (signal) => {
      const m = await load(s);
      let pack = packId
        ? m.packs.find((p) => p.id === packId && p.source === "local_png")
        : undefined;
      if (packId && !pack) throw new Error("sticker_pack_missing");
      if (!pack) {
        if (!name.trim() || Array.from(name.trim()).length > 100) {
          throw new Error("sticker_pack_name_invalid");
        }
        pack = {
          id: crypto.randomUUID(),
          name: name.trim(),
          source: "local_png",
          items: [],
        };
        m.packs.unshift(pack);
      }
      const errors: { index: number; error: string }[] = [];
      let added = 0;
      try {
        for (let index = 0; index < paths.length; index++) {
          check(s, signal);
          try {
            const bytes = await readStickerPng(paths[index], true);
            if (expectedDigests && hash(bytes) !== expectedDigests[index]) {
              throw new Error("sticker_png_changed");
            }
            const digest = await store(s, bytes, signal);
            if (!pack.items.some((i) => i.digest === digest)) {
              pack.items.push({
                id: crypto.randomUUID(),
                digest,
                animated: validateStickerPng(bytes, true).animated,
              });
              added++;
            }
          } catch (e) {
            check(s, signal);
            errors.push({
              index,
              error: failure(e) === "sticker_import_unavailable"
                ? "sticker_png_unreadable"
                : failure(e),
            });
          }
          if (pack.items.length) await save(s, m, signal);
          progress(id, s, index + 1, paths.length);
        }
      } catch (e) {
        if (signal.aborted && runtime.session()?.owner === s.owner) {
          return await response(s, await load(s), {
            cancelled: true,
            added,
            errors,
          });
        }
        throw e;
      }
      if (!pack.items.length) m.packs = m.packs.filter((p) => p !== pack);
      return await response(s, m, { errors, added });
    });
  }
  async function inspectPngs(paths: string[], signal: AbortSignal) {
    const s = session();
    if (!paths.length || paths.length > 100) {
      throw new Error("sticker_png_batch");
    }
    const files = [];
    for (let index = 0; index < paths.length; index++) {
      try {
        const bytes = await readStickerPng(paths[index], true);
        const digest = hash(bytes);
        const previewPath = await cachePreview(s, bytes, signal);
        files.push({
          index,
          valid: true,
          digest,
          previewPath,
          animated: validateStickerPng(bytes, true).animated,
        });
      } catch (e) {
        files.push({
          index,
          valid: false,
          error: failure(e) === "sticker_import_unavailable"
            ? "sticker_png_unreadable"
            : failure(e),
        });
      }
      check(s, signal);
    }
    return { accountMid: s.accountMid, files };
  }
  function cancel(id: string) {
    const s = session();
    const job = jobs.get(id);
    if (job?.session.owner === s.owner) job.controller.abort();
    return { cancelled: !!job && job.session.owner === s.owner };
  }
  async function update(packId: string, action: string, value: string) {
    const s = session();
    if (action === "remove_pack" || action === "remove_item") {
      for (const job of jobs.values()) {
        if (job.session.owner === s.owner && job.packId === packId) {
          job.controller.abort();
        }
      }
    }
    return await exclusive(s, async () => {
      const m = await load(s);
      const pack = m.packs.find((p) => p.id === packId);
      if (!pack) throw new Error("sticker_pack_missing");
      const removedDigests = pack.items.filter((i) =>
        action === "remove_pack" || i.id === value
      ).flatMap((i) => i.digest ? [i.digest] : []);
      if (action === "rename") {
        if (!value.trim() || Array.from(value.trim()).length > 100) {
          throw new Error("sticker_pack_name_invalid");
        }
        pack.name = value.trim();
      } else if (action === "remove_pack") {
        m.packs = m.packs.filter((p) => p !== pack);
      } else if (action === "remove_item") {
        if (
          pack.source === "line_store" && pack.items.some((i) => i.id === value)
        ) {
          pack.excludedItems = [
            ...new Set([...(pack.excludedItems ?? []), value]),
          ];
        }
        pack.items = pack.items.filter((i) => i.id !== value);
      } else throw new Error("sticker_library_invalid");
      await save(s, m);
      // GTK snapshots originals before opening an editor or queueing a send.
      // Remove only content that this committed manifest no longer references.
      if (action !== "rename") {
        const retained = new Set(
          m.packs.flatMap((p) =>
            p.items.flatMap((i) => i.digest ? [i.digest] : [])
          ),
        );
        for (const digest of removedDigests) {
          if (!retained.has(digest)) {
            check(s);
            try {
              await Deno.remove(original(s, digest));
            } catch (e) {
              if (!(e instanceof Deno.errors.NotFound)) throw e;
            }
          }
        }
      }
      return await response(s, m);
    });
  }
  async function adoptLegacy() {
    const s = session();
    return await exclusive(s, async () => {
      const m = await load(s);
      if (m.adoptedLegacy) return await response(s, m);
      const legacy = JSON.parse(
        await Deno.readTextFile(join(runtime.stickerDir, "image-packs.json")),
      );
      if (!Array.isArray(legacy)) throw new Error("sticker_library_invalid");
      const migratedOptions: Record<string, string> = {};
      for (const old of legacy) {
        const packageId = stickerPackageId(String(old?.id ?? ""));
        if (
          !packageId || !Array.isArray(old.stickers) ||
          m.packs.some((p) => p.id === `store-${packageId}`)
        ) continue;
        const pack: Pack = {
          id: `store-${packageId}`,
          name: String(old.name ?? packageId),
          source: "line_store",
          packageId,
          version: String(old.version ?? "1"),
          items: [],
        };
        const seen = new Set<string>();
        for (const entry of old.stickers) {
          const sid = stickerPackageId(String(entry?.stickerId ?? ""));
          if (!sid || seen.has(sid)) continue;
          seen.add(sid);
          const item: Item = { id: sid, stickerId: sid };
          try {
            item.digest = await store(
              s,
              await readStickerPng(join(runtime.stickerDir, `${sid}.png`)),
            );
          } catch {
            check(s);
            item.error = "sticker_import_unavailable";
          }
          pack.items.push(item);
          migratedOptions[`${hash(s.accountMid)}/${pack.id}/${sid}`] =
            `${sid}.png`;
        }
        if (pack.items.length) m.packs.push(pack);
      }
      // Persist aliases for GTK to migrate settings, without modifying its file
      // concurrently from the sidecar. Retain aliases across adoption retries.
      await ensurePrivateDir(directory(s));
      await atomicWriteTextFile(
        join(directory(s), "legacy-options.json"),
        JSON.stringify(migratedOptions),
        0o600,
        () => check(s),
      );
      m.adoptedLegacy = true;
      await save(s, m);
      return await response(s, m, { migratedOptions });
    });
  }
  function findItem(s: Session, m: Manifest, key: string) {
    const [account, packId, itemId, extra] = key.split("/");
    if (account !== hash(s.accountMid) || extra !== undefined) {
      throw new Error("sticker_pack_missing");
    }
    const pack = m.packs.find((p) => p.id === packId);
    const item = pack?.items.find((i) => i.id === itemId);
    if (!item?.digest) throw new Error("sticker_pack_missing");
    return item;
  }
  async function prepareImage(
    id: string,
    key: string,
    options?: unknown,
    inputPath?: string,
  ) {
    const s = session();
    return await previewTask(async (cacheSignal) => {
      const job = { session: s, controller: new AbortController() };
      if (jobs.has(id)) throw new Error("sticker_import_busy");
      jobs.set(id, job);
      const signal = AbortSignal.any([cacheSignal, job.controller.signal]);
      try {
        const item = findItem(s, await load(s), key);
        // Edited static images come from the GTK editor's private media cache.
        // Animation always uses the validated APNG original.
        if (
          inputPath &&
          (item.animated ||
            dirname(inputPath) !== join(runtime.dataDir, "cache", "media") ||
            !/^sticker-image-[a-zA-Z0-9-]+\.png$/.test(
              inputPath.split("/").pop() ?? "",
            ))
        ) throw new Error("sticker_optimize_invalid");
        const bytes = await readStickerPng(
          inputPath ?? original(s, item.digest!),
          !!item.animated,
        );
        check(s, signal);
        const info = validateStickerPng(bytes, true);
        const settings = optimizationOptions(
          options ?? item.optimization,
          info.animated,
          info,
        );
        const extension = info.animated ? "gif" : "png";
        const path = join(
          runtime.stickerDir,
          `optimized-v2-${hash(s.accountMid)}-${hash(bytes)}-${
            hash(JSON.stringify(settings))
          }.${extension}`,
        );
        let output: Uint8Array | undefined;
        try {
          output = await Deno.readFile(path);
        } catch (e) {
          if (!(e instanceof Deno.errors.NotFound)) throw e;
        }
        if (!output) {
          const converted = await optimizeStickerImage(
            bytes,
            settings,
            runtime.stickerDir,
            signal,
          );
          output = converted.bytes;
          await atomicWriteTextFile(
            path,
            output,
            0o600,
            () => check(s, signal),
          );
        }
        check(s, signal);
        return {
          accountMid: s.accountMid,
          path,
          animated: info.animated,
          originalBytes: bytes.length,
          optimizedBytes: output.length,
          options: settings,
        };
      } finally {
        jobs.delete(id);
      }
    });
  }
  async function setOptimization(key: string, options: unknown) {
    const s = session();
    const settings = optimizationOptions(options);
    return await exclusive(s, async () => {
      const m = await load(s);
      findItem(s, m, key).optimization = settings;
      await save(s, m);
      return await response(s, m);
    });
  }
  return {
    list: async () => {
      const s = session();
      return await response(s, await load(s));
    },
    guard: () => {
      const s = session();
      return () => check(s);
    },
    preview: (input: string) => previewTask((signal) => preview(input, signal)),
    importStore,
    importPngs,
    inspectPngs: (paths: string[]) =>
      previewTask((signal) => inspectPngs(paths, signal)),
    clearPreviews: async () => {
      const pending = [...previewTasks.entries()];
      for (const [, controller] of pending) controller.abort();
      await Promise.allSettled(pending.map(([task]) => task));
    },
    cancel,
    prepareImage,
    setOptimization,
    update,
    adoptLegacy,
  };
}

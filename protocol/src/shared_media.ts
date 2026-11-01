import { join } from "@std/path";
import { atomicWriteTextFile, ensurePrivateDir } from "./storage.ts";

type Json = Record<string, unknown>;
export type SharedMediaSource = { message: Json; raw?: Json };
export type SharedMediaAnchor = { messageId: string; createdTime: number };
export type SharedMediaAttachment = {
  path: string;
  fileName: string;
  contentType: string;
  thumbnailPath?: string;
  bytes?: number;
};
export type SharedMediaItem = {
  key: string;
  messageId: string;
  chatMid: string;
  createdTime: number;
  senderMid: string;
  kind: "image" | "video" | "file" | "link";
  fileName?: string;
  fileSize?: number;
  durationMs?: number;
  thumbnailPath?: string;
  path?: string;
  url?: string;
  cacheState: "pending" | "cached" | "failed";
};
type Session = { accountMid: string; owner: object };
export type SharedMediaRuntime = {
  cacheDir: string;
  session: () => Session | null;
  seed: (chatMid: string) => Promise<SharedMediaSource[]>;
  fetchPage: (
    chatMid: string,
    anchor: SharedMediaAnchor | null,
  ) => Promise<SharedMediaSource[]>;
  cacheAttachment: (
    chatMid: string,
    source: SharedMediaSource,
    check: () => void,
  ) => Promise<SharedMediaAttachment>;
  mask: (chatMid: string, messages: Json[]) => Promise<Json[]>;
  changed?: (chatMid: string) => void;
};
type Entry = SharedMediaSource & {
  attachment?: SharedMediaAttachment;
  failed?: boolean;
};
type Disk = {
  version: 1;
  accountMid: string;
  chatMid: string;
  records: Record<string, Entry>;
  checkpoint: SharedMediaAnchor | null;
  latest: SharedMediaAnchor | null;
  complete: boolean;
};
type State = Disk & {
  session: Session;
  epoch: number;
  paused: boolean;
  error?: string;
  job?: Promise<void>;
  writes: Promise<void>;
  needsRefresh: boolean;
};
const validChat = (mid: string) => /^[ucr][a-zA-Z0-9_-]{1,128}$/.test(mid);
const validAccount = (mid: string) => /^u[a-zA-Z0-9_-]{1,128}$/.test(mid);
const idOf = (message: Json) => String(message.id ?? "");
const timeOf = (message: Json) => {
  const n = Number(message.createdTime);
  return Number.isFinite(n) ? n : 0;
};
function compareIds(a: string, b: string): number {
  if (/^\d+$/.test(a) && /^\d+$/.test(b)) {
    const aa = BigInt(a), bb = BigInt(b);
    return aa === bb ? 0 : aa > bb ? 1 : -1;
  }
  return a.localeCompare(b);
}
function compareMessages(a: Json, b: Json): number {
  return timeOf(a) - timeOf(b) || compareIds(idOf(a), idOf(b));
}
function anchorOf(source: SharedMediaSource): SharedMediaAnchor {
  const delivered = Number(source.raw?.deliveredTime);
  return {
    messageId: idOf(source.message),
    createdTime: Number.isFinite(delivered) && delivered > 0
      ? delivered
      : timeOf(source.message),
  };
}

/** Message links only: never crawl the linked page or quoted/Flex payloads. */
export function sharedMediaUrls(text: string): string[] {
  const urls: string[] = [];
  for (const match of text.matchAll(/https?:\/\/[^\s<>"']+/gi)) {
    let value = match[0].replace(/[.,;:!?。！？，；：]+$/u, "");
    for (const [close, open] of [[")", "("], ["]", "["], ["}", "{"]]) {
      while (
        value.endsWith(close) &&
        value.split(close).length > value.split(open).length
      ) value = value.slice(0, -1);
    }
    if (Array.from(value).some((c) => c.charCodeAt(0) < 32)) continue;
    try {
      const parsed = new URL(value);
      if (parsed.hostname && !urls.includes(value)) urls.push(value);
    } catch { /* malformed text is not a link */ }
  }
  return urls;
}

export function sharedMediaItems(
  chatMid: string,
  message: Json,
): SharedMediaItem[] {
  const messageId = idOf(message);
  if (!messageId || message.decryptionFailed || message.chatEvent) return [];
  const type = String(message.contentType ?? "NONE").toUpperCase();
  const base = {
    messageId,
    chatMid,
    createdTime: timeOf(message),
    senderMid: String(message.from ?? ""),
  };
  if (["IMAGE", "VIDEO", "FILE"].includes(type)) {
    const item: SharedMediaItem = {
      ...base,
      key: messageId,
      kind: type.toLowerCase() as "image" | "video" | "file",
      cacheState: "pending",
    };
    if (typeof message.fileName === "string") item.fileName = message.fileName;
    for (const field of ["fileSize", "durationMs"] as const) {
      const n = Number(message[field]);
      if (message[field] != null && Number.isFinite(n) && n >= 0) {
        item[field] = n;
      }
    }
    if (typeof message.imagePath === "string") {
      item.thumbnailPath = message.imagePath;
    }
    return [item];
  }
  if (!["NONE", "TEXT", "0"].includes(type)) return [];
  return sharedMediaUrls(String(message.text ?? "")).map((url, i) => ({
    ...base,
    key: `${messageId}:url:${i}`,
    kind: "link",
    url,
    cacheState: "cached",
  }));
}

export function createSharedMediaService(runtime: SharedMediaRuntime) {
  const loaded = new Map<string, Promise<State>>();
  const downloads = new Map<string, Promise<SharedMediaAttachment>>();
  const ownerIds = new WeakMap<object, number>();
  let ownerSequence = 0;
  let writeSequence = 0;
  const waiting: Array<() => void> = [];
  let transfers = 0;
  let epoch = 0;
  let clearing: Promise<void> | undefined;
  const path = (account: string, chatMid: string) =>
    join(runtime.cacheDir, `shared-media-${account}-${chatMid}.json`);

  function ownerId(owner: object): number {
    let value = ownerIds.get(owner);
    if (!value) {
      value = ++ownerSequence;
      ownerIds.set(owner, value);
    }
    return value;
  }
  function current(): Session {
    const session = runtime.session();
    if (!session || !validAccount(session.accountMid)) {
      throw new Error("shared_media_not_logged_in");
    }
    return session;
  }
  function check(state: State) {
    const session = runtime.session();
    if (
      !session || session.owner !== state.session.owner ||
      session.accountMid !== state.accountMid || state.epoch !== epoch ||
      state.paused
    ) throw new Error("shared_media_cancelled");
  }
  async function save(state: State) {
    check(state);
    const snapshot: Disk = {
      version: 1,
      accountMid: state.accountMid,
      chatMid: state.chatMid,
      records: state.records,
      checkpoint: state.checkpoint,
      latest: state.latest,
      complete: state.complete,
    };
    // Thrift i64 values are restored by the origin resolver, never rounded.
    const contents = JSON.stringify(snapshot, (_key, value) => {
      if (typeof value === "bigint") return String(value);
      if (value instanceof Uint8Array) {
        return { type: "shared_media_bytes", data: Array.from(value) };
      }
      return value;
    });
    const task = state.writes.catch(() => {}).then(async () => {
      check(state);
      await ensurePrivateDir(runtime.cacheDir);
      check(state);
      const destination = path(state.accountMid, state.chatMid);
      const temporary = `${destination}.${Deno.pid}.${++writeSequence}.tmp`;
      try {
        const file = await Deno.open(temporary, {
          createNew: true,
          write: true,
          mode: 0o600,
        });
        try {
          const bytes = new TextEncoder().encode(contents);
          let offset = 0;
          while (offset < bytes.length) {
            offset += await file.write(bytes.subarray(offset));
          }
          await file.sync();
        } finally {
          file.close();
        }
        check(state);
        // No async boundary between the ownership check and publication.
        Deno.renameSync(temporary, destination);
      } finally {
        try {
          Deno.removeSync(temporary);
        } catch { /* already published */ }
      }
    });
    state.writes = task;
    await task;
  }
  async function stateFor(chatMid: string): Promise<State> {
    if (clearing) await clearing;
    if (!validChat(chatMid)) throw new Error("shared_media_invalid_chat");
    const session = current();
    const key = `${session.accountMid}:${chatMid}`;
    let pending = loaded.get(key);
    if (pending) {
      const state = await pending;
      if (state.session.owner === session.owner && state.epoch === epoch) {
        return state;
      }
      loaded.delete(key);
    }
    const capturedEpoch = epoch;
    pending = (async () => {
      let disk: Disk = {
        version: 1,
        accountMid: session.accountMid,
        chatMid,
        records: {},
        checkpoint: null,
        latest: null,
        complete: false,
      };
      try {
        const parsed = JSON.parse(
          await Deno.readTextFile(path(session.accountMid, chatMid)),
          (_key, value) =>
            value && value.type === "shared_media_bytes" &&
              Array.isArray(value.data)
              ? Uint8Array.from(value.data)
              : value,
        );
        if (
          parsed.version !== 1 || parsed.accountMid !== session.accountMid ||
          parsed.chatMid !== chatMid || !parsed.records ||
          typeof parsed.records !== "object" || Array.isArray(parsed.records)
        ) throw new Error("shared_media_invalid_cache");
        disk = parsed;
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) {
          console.error("[shared-media cache]", error);
        }
      }
      const state: State = {
        ...disk,
        session,
        epoch: capturedEpoch,
        paused: false,
        writes: Promise.resolve(),
        needsRefresh: true,
      };
      check(state);
      await merge(state, await runtime.seed(chatMid));
      check(state);
      return state;
    })();
    loaded.set(key, pending);
    try {
      return await pending;
    } catch (error) {
      if (loaded.get(key) === pending) loaded.delete(key);
      throw error;
    }
  }
  async function merge(state: State, sources: SharedMediaSource[]) {
    check(state);
    const messages = await runtime.mask(
      state.chatMid,
      sources.map((source) => source.message),
    );
    check(state);
    const byId = new Map(messages.map((message) => [idOf(message), message]));
    for (const source of sources) {
      const id = idOf(source.message);
      const message = byId.get(id);
      if (!message || !sharedMediaItems(state.chatMid, message).length) {
        if (message?.contentType === "UNSENT") delete state.records[id];
        continue;
      }
      const old = state.records[id];
      const entry: Entry = old ?? { message: {} };
      if (!entry.raw && source.raw) delete entry.failed;
      entry.message = { ...entry.message, ...message };
      entry.raw = source.raw ?? entry.raw;
      state.records[id] = entry;
    }
  }
  async function maskedEntries(state: State): Promise<Entry[]> {
    const entries = Object.values(state.records);
    const masked = await runtime.mask(
      state.chatMid,
      entries.map((entry) => entry.message),
    );
    check(state);
    const byId = new Map(masked.map((message) => [idOf(message), message]));
    let removed = false;
    for (const entry of entries) {
      const id = idOf(entry.message);
      const message = byId.get(id);
      if (!message || !sharedMediaItems(state.chatMid, message).length) {
        delete state.records[id];
        removed = true;
      }
    }
    if (removed) await save(state);
    return Object.values(state.records);
  }
  async function withSlot<T>(operation: () => Promise<T>): Promise<T> {
    if (transfers >= 3) {
      await new Promise<void>((resolve) => waiting.push(resolve));
    } else transfers++;
    try {
      return await operation();
    } finally {
      const next = waiting.shift();
      if (next) next();
      else transfers--;
    }
  }
  async function cacheEntry(state: State, entry: Entry) {
    check(state);
    const id = idOf(entry.message);
    const key = `${
      ownerId(state.session.owner)
    }:${state.epoch}:${state.accountMid}:${state.chatMid}:${id}`;
    const pending = downloads.get(key);
    if (pending) {
      const result = await pending;
      check(state);
      return result;
    }
    const operation = withSlot(async () => {
      check(state);
      if (!(await maskedEntries(state)).some((e) => idOf(e.message) === id)) {
        throw new Error("shared_media_unavailable");
      }
      if (entry.attachment) {
        try {
          if ((await Deno.stat(entry.attachment.path)).isFile) {
            check(state);
            return entry.attachment;
          }
        } catch { /* cleared cache: obtain again */ }
        delete entry.attachment;
      }
      const result = await runtime.cacheAttachment(
        state.chatMid,
        entry,
        () => check(state),
      );
      check(state);
      if (!(await maskedEntries(state)).some((e) => idOf(e.message) === id)) {
        throw new Error("shared_media_unavailable");
      }
      entry.attachment = result;
      delete entry.failed;
      await save(state);
      runtime.changed?.(state.chatMid);
      return result;
    });
    downloads.set(key, operation);
    try {
      return await operation;
    } catch (error) {
      check(state);
      if (state.records[id] === entry) {
        entry.failed = true;
        await save(state);
        runtime.changed?.(state.chatMid);
      }
      throw error;
    } finally {
      if (downloads.get(key) === operation) downloads.delete(key);
    }
  }
  async function cachePending(state: State) {
    for (;;) {
      check(state);
      const pending = (await maskedEntries(state)).filter((entry) =>
        !entry.attachment && !entry.failed &&
        sharedMediaItems(state.chatMid, entry.message)[0]?.kind !== "link"
      ).slice(0, 3);
      if (!pending.length) return;
      await Promise.allSettled(
        pending.map((entry) => cacheEntry(state, entry)),
      );
      check(state);
    }
  }
  async function page(state: State, anchor: SharedMediaAnchor | null) {
    const sources = await runtime.fetchPage(state.chatMid, anchor);
    check(state);
    const valid = sources.filter((source) => idOf(source.message));
    if (sources.length && !valid.length) {
      throw new Error("shared_media_invalid_page");
    }
    valid.sort((a, b) => compareMessages(a.message, b.message));
    await merge(state, valid);
    return valid;
  }
  async function run(state: State) {
    check(state);
    const previousLatest = state.latest;
    let refreshAnchor: SharedMediaAnchor | null = null;
    let newLatest: SharedMediaAnchor | null = null;
    // Refresh the gap since the previous successful refresh. Commit latest only
    // after overlap so cancellation cannot permanently skip intervening history.
    for (;;) {
      const sources = await page(state, refreshAnchor);
      if (!sources.length) break;
      newLatest ??= anchorOf(sources[sources.length - 1]);
      if (!previousLatest) {
        state.latest = newLatest;
        state.complete = false;
        state.checkpoint ??= anchorOf(sources[0]);
        await save(state);
        runtime.changed?.(state.chatMid);
        break;
      }
      const overlaps = sources.some((source) => {
        if (
          /^\d+$/.test(idOf(source.message)) &&
          /^\d+$/.test(previousLatest.messageId)
        ) {
          return compareIds(idOf(source.message), previousLatest.messageId) <=
            0;
        }
        return compareMessages(source.message, {
          id: previousLatest.messageId,
          createdTime: previousLatest.createdTime,
        }) <= 0;
      });
      await save(state);
      runtime.changed?.(state.chatMid);
      if (overlaps) break;
      const next = anchorOf(sources[0]);
      if (next.messageId === refreshAnchor?.messageId) {
        throw new Error("shared_media_pagination_stalled");
      }
      refreshAnchor = next;
      await cachePending(state);
      await new Promise((resolve) => setTimeout(resolve, 0));
      check(state);
    }
    if (newLatest) state.latest = newLatest;
    await save(state);
    await cachePending(state);
    while (!state.complete) {
      const sources = await page(state, state.checkpoint);
      if (!sources.length) {
        state.complete = true;
      } else {
        const next = anchorOf(sources[0]);
        if (next.messageId === state.checkpoint?.messageId) {
          throw new Error("shared_media_pagination_stalled");
        }
        state.checkpoint = next;
      }
      await save(state);
      runtime.changed?.(state.chatMid);
      await cachePending(state);
      await new Promise((resolve) => setTimeout(resolve, 0));
      check(state);
    }
  }
  function start(state: State, refresh: boolean) {
    if (state.job || (state.error && !refresh)) return;
    state.paused = false;
    state.needsRefresh = false;
    delete state.error;
    if (refresh) {
      for (const entry of Object.values(state.records)) delete entry.failed;
    }
    state.job = run(state).catch((error) => {
      if (
        error instanceof Error && error.message === "shared_media_cancelled"
      ) {
        return;
      }
      state.error = error instanceof Error ? error.message : String(error);
    }).finally(() => {
      delete state.job;
      runtime.changed?.(state.chatMid);
    });
  }

  async function list(
    chatMid: string,
    category: string,
    cursor = "",
    rawLimit = 60,
    refresh = false,
  ) {
    if (!["photos_videos", "links", "files"].includes(category)) {
      throw new Error("shared_media_invalid_category");
    }
    const state = await stateFor(chatMid);
    if (state.paused) await state.job;
    state.paused = false;
    check(state);
    const entries = await maskedEntries(state);
    const all: SharedMediaItem[] = [];
    for (const entry of entries) {
      for (const item of sharedMediaItems(chatMid, entry.message)) {
        if (entry.attachment) {
          item.path = entry.attachment.path;
          item.thumbnailPath = entry.attachment.thumbnailPath ??
            item.thumbnailPath;
          item.fileName = entry.attachment.fileName;
          item.fileSize ??= entry.attachment.bytes;
          item.cacheState = "cached";
        } else if (entry.failed) item.cacheState = "failed";
        if (
          (category === "photos_videos" &&
            ["image", "video"].includes(item.kind)) ||
          (category === "links" && item.kind === "link") ||
          (category === "files" && item.kind === "file")
        ) all.push(item);
      }
    }
    all.sort((a, b) =>
      b.createdTime - a.createdTime || compareIds(b.messageId, a.messageId) ||
      a.key.localeCompare(b.key)
    );
    let offset = 0;
    if (cursor) {
      const match = /^(.+):([0-9]+)$/.exec(cursor);
      const prefix = `${state.accountMid}/${chatMid}/${category}/${
        ownerId(state.session.owner)
      }/${epoch}`;
      if (!match || match[1] !== prefix) {
        throw new Error("shared_media_invalid_cursor");
      }
      offset = Number(match[2]);
      if (!Number.isSafeInteger(offset)) {
        throw new Error("shared_media_invalid_cursor");
      }
    }
    const limit = Number.isFinite(rawLimit)
      ? Math.max(1, Math.min(100, Math.floor(rawLimit)))
      : 60;
    const items = all.slice(offset, offset + limit);
    const nextCursor = offset + limit < all.length
      ? `${state.accountMid}/${chatMid}/${category}/${
        ownerId(state.session.owner)
      }/${epoch}:${offset + limit}`
      : "";
    // One start per open or explicit refresh. Polling must not repeat history.
    if (
      refresh || state.needsRefresh || !state.complete ||
      entries.some((entry) =>
        !entry.attachment && !entry.failed &&
        sharedMediaItems(chatMid, entry.message)[0]?.kind !== "link"
      )
    ) start(state, refresh);
    return {
      chatMid,
      category,
      items,
      nextCursor,
      scanState: state.complete ? "end" : "more",
      busy: !!state.job,
      ...(state.error ? { error: state.error } : {}),
      cachedCount: all.filter((item) => item.cacheState === "cached").length,
      pendingCount: all.filter((item) => item.cacheState === "pending").length,
      failedCount: all.filter((item) => item.cacheState === "failed").length,
    };
  }
  async function download(chatMid: string, messageId: string, retry = false) {
    const state = await stateFor(chatMid);
    if (state.paused) await state.job;
    state.paused = false;
    check(state);
    const entry = (await maskedEntries(state)).find((entry) =>
      idOf(entry.message) === messageId
    );
    if (
      !entry || sharedMediaItems(chatMid, entry.message)[0]?.kind === "link"
    ) {
      throw new Error("shared_media_unavailable");
    }
    if (retry) delete entry.failed;
    return await cacheEntry(state, entry);
  }
  async function source(chatMid: string, messageId: string) {
    const state = await stateFor(chatMid);
    if (state.paused) await state.job;
    state.paused = false;
    return (await maskedEntries(state)).find((entry) =>
      idOf(entry.message) === messageId
    );
  }
  async function pause(chatMid: string) {
    const state = await stateFor(chatMid);
    state.paused = true;
    state.needsRefresh = true;
    await state.job;
    await state.writes.catch(() => {});
  }
  async function clearCache(kind: string) {
    if (!["all", "messages", "media"].includes(kind)) return;
    epoch++;
    const states = await Promise.allSettled([...loaded.values()]);
    loaded.clear();
    for (const result of states) {
      if (result.status !== "fulfilled") continue;
      const state = result.value;
      state.paused = true;
      await state.job;
      await state.writes.catch(() => {});
    }
    await ensurePrivateDir(runtime.cacheDir);
    for await (const file of Deno.readDir(runtime.cacheDir)) {
      if (!file.isFile || !/^shared-media-.+\.json$/.test(file.name)) continue;
      const target = join(runtime.cacheDir, file.name);
      if (kind === "media") {
        try {
          const disk = JSON.parse(await Deno.readTextFile(target)) as Disk;
          for (const entry of Object.values(disk.records)) {
            delete entry.attachment;
            delete entry.failed;
            delete entry.message.imagePath;
            delete entry.message.filePath;
          }
          await atomicWriteTextFile(target, JSON.stringify(disk));
        } catch { /* invalid cache will be rebuilt */ }
      } else await Deno.remove(target);
    }
  }
  async function clear(kind: string) {
    const previous = clearing;
    const operation = (async () => {
      await previous;
      await clearCache(kind);
    })();
    clearing = operation;
    try {
      await operation;
    } finally {
      if (clearing === operation) clearing = undefined;
    }
  }
  async function idle(chatMid: string) {
    const state = await stateFor(chatMid);
    await state.job;
    await state.writes;
  }
  async function updateLoaded(
    chatMid: string,
    operation: (state: State) => Promise<void>,
  ) {
    const session = runtime.session();
    if (!session) return;
    const pending = loaded.get(`${session.accountMid}:${chatMid}`);
    if (!pending) return;
    const state = await pending;
    const wasPaused = state.paused;
    if (wasPaused) await state.job;
    state.paused = false;
    try {
      check(state);
      await operation(state);
      await save(state);
      runtime.changed?.(chatMid);
      if (!wasPaused && !state.job) start(state, false);
    } finally {
      if (wasPaused) state.paused = true;
    }
  }
  async function observe(chatMid: string, sources: SharedMediaSource[]) {
    await updateLoaded(chatMid, (state) => merge(state, sources));
  }
  async function unsent(chatMid: string, messageId: string) {
    await updateLoaded(chatMid, (state) => {
      delete state.records[messageId];
      return Promise.resolve();
    });
  }
  return { list, download, source, pause, clear, idle, observe, unsent };
}

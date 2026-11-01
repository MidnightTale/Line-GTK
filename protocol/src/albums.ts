import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { ensurePrivateDir } from "./storage.ts";

type Album = {
  id: string;
  chatMid: string;
  title: string;
  photoCount: number;
};
type Photo = {
  id: string;
  oid: string;
  kind: "image" | "video";
};
type Runtime = {
  getClient: () => Client | null;
  sessionId: () => string;
  mediaDir: string;
};

function identifier(value: unknown): string {
  if (
    typeof value !== "string" || !value.trim() || value.length > 2048 ||
    Array.from(value).some((character) => character.charCodeAt(0) < 32)
  ) throw new Error("album_invalid_request");
  return value;
}

function cursorOf(value: unknown): string {
  if (value == null || value === "") return "";
  return identifier(value);
}

function nextCursor(value: unknown, previous: string, more = true): string {
  const cursor = cursorOf(value);
  return more && cursor !== previous ? cursor : "";
}

function imageExtension(bytes: Uint8Array): string {
  if (bytes.length < 12) throw new Error("album_image_unsupported");
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "jpg";
  if ([137, 80, 78, 71, 13, 10, 26, 10].every((b, i) => bytes[i] === b)) {
    return "png";
  }
  const signature = new TextDecoder().decode(bytes.subarray(0, 12));
  if (signature.startsWith("GIF87a") || signature.startsWith("GIF89a")) {
    return "gif";
  }
  if (signature.startsWith("RIFF") && signature.slice(8) === "WEBP") {
    return "webp";
  }
  throw new Error("album_image_unsupported");
}

export function createAlbumService(runtime: Runtime) {
  let owner: Client | null = null;
  let ownerMid = "";
  const photos = new Map<string, Photo>();
  const downloads = new Map<string, Promise<{ path: string; name: string }>>();
  let activeDownloads = 0;
  const waiting: Array<() => void> = [];

  async function withDownloadSlot<T>(operation: () => Promise<T>): Promise<T> {
    if (activeDownloads >= 3) {
      await new Promise<void>((resolve) => waiting.push(resolve));
    } else activeDownloads++;
    try {
      return await operation();
    } finally {
      const next = waiting.shift();
      if (next) next();
      else activeDownloads--;
    }
  }

  function session() {
    const client = runtime.getClient();
    const mid = runtime.sessionId();
    if (!client || !mid) throw new Error("album_not_logged_in");
    if (client !== owner || mid !== ownerMid) {
      owner = client;
      ownerMid = mid;
      photos.clear();
      downloads.clear();
    }
    return {
      client,
      mid,
      check() {
        if (runtime.getClient() !== client || runtime.sessionId() !== mid) {
          throw new Error("album_session_changed");
        }
      },
    };
  }

  function photoKey(chatMid: string, albumId: string, photoId: string) {
    return JSON.stringify([chatMid, albumId, photoId]);
  }

  async function call<T>(
    active: ReturnType<typeof session>,
    operation: () => Promise<T>,
  ) {
    try {
      const result = await operation();
      active.check();
      return result;
    } catch {
      active.check();
      // 期限切れの認証情報を再試行でも使い続けない。通信自体の自動再試行はしない。
      active.client.base.moa.clearAlbumChannelToken();
      throw new Error("album_request_failed");
    }
  }

  async function listAlbums(chatMid: string, inputCursor: unknown) {
    const active = session();
    if (chatMid) identifier(chatMid);
    let cursor = cursorOf(inputCursor);
    const visited = new Set<string>();
    // トークで絞り込む場合、該当しないページを少数ずつ読み飛ばす。
    for (let page = 0; page < 4; page++) {
      visited.add(cursor);
      const response = await call(
        active,
        () => active.client.base.moa.getAlbums({ cursor }),
      );
      active.check();
      if (!Array.isArray(response.result?.albums)) {
        throw new Error("album_request_failed");
      }
      const albums: Album[] = response.result.albums
        .filter((album) => !chatMid || album.chatId === chatMid)
        .map((album) => ({
          id: identifier(String(album.albumId ?? "")),
          chatMid: identifier(album.chatId),
          title: typeof album.title === "string" ? album.title : "",
          photoCount: Number.isSafeInteger(album.photoCount)
            ? Math.max(0, album.photoCount!)
            : 0,
        }));
      const next = nextCursor(
        response.result.nextCursor ?? response.result.cursor,
        cursor,
        response.result.hasMore !== false,
      );
      cursor = visited.has(next) ? "" : next;
      if (albums.length || !cursor || !chatMid) return { albums, cursor };
    }
    return { albums: [] as Album[], cursor };
  }

  async function listPhotos(
    chatMid: string,
    albumId: string,
    inputCursor: unknown,
  ) {
    const active = session();
    identifier(chatMid);
    identifier(albumId);
    const cursor = cursorOf(inputCursor);
    const response = await call(active, () =>
      active.client.base.moa.getPhotos({
        chatId: chatMid,
        albumId,
        cursor,
        pageSize: 12,
      }));
    active.check();
    if (!Array.isArray(response.result?.photos)) {
      throw new Error("album_request_failed");
    }
    const items = response.result.photos.map((photo): Photo => {
      const oid = identifier(photo.obsResourceId?.oid || photo.oid || "");
      return {
        id: identifier(String(photo.photoId ?? photo.id ?? oid)),
        oid,
        kind: photo.obsResourceId?.sid === "v" ||
            photo.resourceType?.toUpperCase() === "VIDEO"
          ? "video"
          : "image",
      };
    });
    for (const photo of items) {
      photos.set(photoKey(chatMid, albumId, photo.id), photo);
    }
    while (photos.size > 5000) photos.delete(photos.keys().next().value!);
    return {
      photos: items.map(({ id, kind }) => ({ id, kind })),
      cursor: nextCursor(response.result.nextCursor, cursor),
    };
  }

  async function downloadPhoto(
    chatMid: string,
    albumId: string,
    photoId: string,
  ) {
    const active = session();
    const key = photoKey(
      identifier(chatMid),
      identifier(albumId),
      identifier(photoId),
    );
    const photo = photos.get(key);
    if (!photo) throw new Error("album_photo_expired");
    if (photo.kind !== "image") throw new Error("album_image_unsupported");
    const pending = downloads.get(key);
    if (pending) return await pending;
    const operation = withDownloadSlot(async () => {
      active.check();
      // 別アカウント・別トークの写真がキャッシュで衝突しないようにする。
      const digest = await crypto.subtle.digest(
        "SHA-256",
        new TextEncoder().encode(JSON.stringify([active.mid, key, photo.oid])),
      );
      const hash = Array.from(
        new Uint8Array(digest),
        (b) => b.toString(16).padStart(2, "0"),
      ).join("");
      for (const extension of ["jpg", "png", "gif", "webp"]) {
        const path = join(runtime.mediaDir, `album-${hash}.${extension}`);
        try {
          const stat = await Deno.stat(path);
          if (stat.isFile && stat.size > 0) {
            active.check();
            return { path, name: `album-${hash.slice(0, 16)}.${extension}` };
          }
        } catch (error) {
          if (!(error instanceof Deno.errors.NotFound)) throw error;
        }
      }
      const bytes = await call(
        active,
        () =>
          active.client.base.moa.downloadPhoto({
            chatId: chatMid,
            albumId,
            oid: photo.oid,
          }),
      );
      active.check();
      if (bytes.length > 50 * 1024 * 1024) {
        throw new Error("album_image_too_large");
      }
      const extension = imageExtension(bytes);
      const path = join(runtime.mediaDir, `album-${hash}.${extension}`);
      const temporary = `${path}.${crypto.randomUUID()}.tmp`;
      await ensurePrivateDir(runtime.mediaDir);
      try {
        await Deno.writeFile(temporary, bytes, {
          createNew: true,
          mode: 0o600,
        });
        active.check();
        await Deno.rename(temporary, path);
      } finally {
        await Deno.remove(temporary).catch(() => {});
      }
      return { path, name: `album-${hash.slice(0, 16)}.${extension}` };
    });
    downloads.set(key, operation);
    try {
      return await operation;
    } finally {
      if (downloads.get(key) === operation) downloads.delete(key);
    }
  }

  return { listAlbums, listPhotos, downloadPhoto };
}

import { assert, assertEquals, assertRejects } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createAlbumService } from "../src/albums.ts";

const png = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aTacAAAAASUVORK5CYII=",
  ),
  (c) => c.charCodeAt(0),
);

function fixture(mediaDir: string) {
  let mid = "u-self";
  let calls = 0;
  const cursors: string[] = [];
  const api = {
    clearAlbumChannelToken: () => {},
    getAlbums: ({ cursor }: { cursor?: string }) => {
      cursors.push(cursor ?? "");
      return Promise.resolve({
        result: {
          albums: [
            { albumId: "1", chatId: "c-group", title: "旅行", photoCount: 2 },
            {
              albumId: "2",
              chatId: "u-friend",
              title: "別のトーク",
              photoCount: 1,
            },
          ],
          nextCursor: "next",
        },
      });
    },
    getPhotos: (
      options: { chatId: string; albumId: string; pageSize: number },
    ) => {
      assertEquals(options.pageSize, 12);
      return Promise.resolve({
        result: {
          photos: [
            {
              photoId: "9223372036854775807",
              obsResourceId: { oid: "image-original", sid: "a" },
            },
            { id: "video", obsResourceId: { oid: "video-original", sid: "v" } },
          ],
          nextCursor: "photos-next",
        },
      });
    },
    downloadPhoto: (
      _options: { chatId: string; albumId: string; oid: string },
    ) => {
      calls++;
      return Promise.resolve(png);
    },
  };
  let client: Client | null = { base: { moa: api } } as unknown as Client;
  return {
    api,
    service: createAlbumService({
      getClient: () => client,
      sessionId: () => mid,
      mediaDir,
    }),
    cursors,
    calls: () => calls,
    changeSession: () => {
      mid = "u-other";
    },
    logout: () => {
      client = null;
    },
  };
}

Deno.test("アルバムをトークで絞り込み、全体一覧・次ページにも対応する", async () => {
  const f = fixture("");
  assertEquals(await f.service.listAlbums("c-group", ""), {
    albums: [{ id: "1", chatMid: "c-group", title: "旅行", photoCount: 2 }],
    cursor: "next",
  });
  assertEquals((await f.service.listAlbums("", "")).albums.length, 2);
  assertEquals((await f.service.listAlbums("c-group", "next")).cursor, "");
});

Deno.test("該当アルバムがない場合も巡回と一度の取得ページ数を制限する", async () => {
  const f = fixture("");
  let n = 0;
  f.api.getAlbums = () =>
    Promise.resolve({ result: { albums: [], nextCursor: String(++n) } });
  assertEquals(await f.service.listAlbums("c-empty", ""), {
    albums: [],
    cursor: "4",
  });
  assertEquals(n, 4);
  f.api.getAlbums = () =>
    Promise.resolve({ result: { albums: [], nextCursor: "cycle" } });
  assertEquals((await f.service.listAlbums("c-empty", "cycle")).cursor, "");
});

Deno.test("写真IDを丸めず、写真と動画を区別して12件ずつ取得する", async () => {
  const f = fixture("");
  assertEquals(await f.service.listPhotos("c-group", "1", ""), {
    photos: [{ id: "9223372036854775807", kind: "image" }, {
      id: "video",
      kind: "video",
    }],
    cursor: "photos-next",
  });
  await assertRejects(
    () => f.service.downloadPhoto("c-group", "1", "video"),
    Error,
    "album_image_unsupported",
  );
  await assertRejects(
    () => f.service.downloadPhoto("c-group", "1", "unknown"),
    Error,
    "album_photo_expired",
  );
  assertEquals(f.calls(), 0);
});

Deno.test("原本の取得をまとめ、拡張子を判別し、完全なキャッシュだけ再利用する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    await f.service.listPhotos("c-group", "1", "");
    const [a, b] = await Promise.all([
      f.service.downloadPhoto("c-group", "1", "9223372036854775807"),
      f.service.downloadPhoto("c-group", "1", "9223372036854775807"),
    ]);
    assertEquals(a, b);
    assert(a.path.endsWith(".png"));
    assertEquals(await Deno.readFile(a.path), png);
    assertEquals(f.calls(), 1);
    assertEquals(
      await f.service.downloadPhoto("c-group", "1", "9223372036854775807"),
      a,
    );
    assertEquals(f.calls(), 1);
    if (Deno.build.os !== "windows") {
      assertEquals((await Deno.stat(a.path)).mode! & 0o777, 0o600);
    }
    assertEquals(Array.from(Deno.readDirSync(dir)).length, 1);
    await Deno.remove(a.path);
    await f.service.downloadPhoto("c-group", "1", "9223372036854775807");
    assertEquals(f.calls(), 2);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("別アルバム・別アカウントの同じ写真IDが衝突しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    await f.service.listPhotos("c-group", "1", "");
    const first = await f.service.downloadPhoto(
      "c-group",
      "1",
      "9223372036854775807",
    );
    await f.service.listPhotos("c-group", "2", "");
    const second = await f.service.downloadPhoto(
      "c-group",
      "2",
      "9223372036854775807",
    );
    assert(first.path !== second.path);
    f.changeSession();
    await assertRejects(
      () => f.service.downloadPhoto("c-group", "1", "9223372036854775807"),
      Error,
      "album_photo_expired",
    );
    await f.service.listPhotos("c-group", "1", "");
    const other = await f.service.downloadPhoto(
      "c-group",
      "1",
      "9223372036854775807",
    );
    assert(first.path !== other.path);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("通信中のセッション切替・ログアウト後は古い結果を表示しない", async () => {
  const f = fixture("");
  const original = f.api.getAlbums;
  f.api.getAlbums = async (options) => {
    const result = await original(options);
    f.changeSession();
    return result;
  };
  await assertRejects(
    () => f.service.listAlbums("", ""),
    Error,
    "album_session_changed",
  );
  f.logout();
  await assertRejects(
    () => f.service.listAlbums("", ""),
    Error,
    "album_not_logged_in",
  );
});

Deno.test("画像でない応答や通信失敗を成功キャッシュとして残さない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    await f.service.listPhotos("c-group", "1", "");
    f.api.downloadPhoto = () =>
      Promise.resolve(new TextEncoder().encode("<html>access denied</html>"));
    await assertRejects(
      () => f.service.downloadPhoto("c-group", "1", "9223372036854775807"),
      Error,
      "album_image_unsupported",
    );
    f.api.downloadPhoto = () => Promise.reject(new Error("通信失敗"));
    await assertRejects(
      () => f.service.downloadPhoto("c-group", "1", "9223372036854775807"),
      Error,
      "album_request_failed",
    );
    f.api.downloadPhoto = () => Promise.resolve(png);
    await f.service.downloadPhoto("c-group", "1", "9223372036854775807");
    assertEquals(Array.from(Deno.readDirSync(dir)).length, 1);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("通信失敗後はチャンネルトークンを破棄して明示的な再試行に備える", async () => {
  const f = fixture("");
  let cleared = 0;
  f.api.clearAlbumChannelToken = () => {
    cleared++;
  };
  const getAlbums = f.api.getAlbums;
  f.api.getAlbums = () => Promise.reject(new Error("認証期限切れ"));
  await assertRejects(
    () => f.service.listAlbums("", ""),
    Error,
    "album_request_failed",
  );
  assertEquals(cleared, 1);
  f.api.getAlbums = getAlbums;
  assertEquals((await f.service.listAlbums("", "")).albums.length, 2);
});

Deno.test("画面を切り替えても原本の同時取得は3枚までで、失敗後も待機処理を進める", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    let active = 0;
    let peak = 0;
    f.api.downloadPhoto = async ({ albumId }) => {
      active++;
      peak = Math.max(peak, active);
      try {
        await new Promise((resolve) => setTimeout(resolve, 10));
        if (albumId === "0") throw new Error("通信失敗");
        return png;
      } finally {
        active--;
      }
    };
    for (let n = 0; n < 6; n++) {
      await f.service.listPhotos("c-group", String(n), "");
    }
    const results = await Promise.allSettled(
      Array.from(
        { length: 6 },
        (_, n) =>
          f.service.downloadPhoto("c-group", String(n), "9223372036854775807"),
      ),
    );
    assertEquals(peak, 3);
    assertEquals(active, 0);
    assertEquals(
      results.filter((result) => result.status === "fulfilled").length,
      5,
    );
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("原本取得中にアカウントが切り替わったら結果とファイルを残さない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    await f.service.listPhotos("c-group", "1", "");
    let finish!: () => void;
    const started = new Promise<void>((ready) => {
      f.api.downloadPhoto = () =>
        new Promise<typeof png>((resolve) => {
          finish = () => resolve(png);
          ready();
        });
    });
    const pending = f.service.downloadPhoto(
      "c-group",
      "1",
      "9223372036854775807",
    );
    await started;
    f.changeSession();
    finish();
    await assertRejects(() => pending, Error, "album_session_changed");
    assertEquals(Array.from(Deno.readDirSync(dir)).length, 0);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

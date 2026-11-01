import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { createOutgoingMedia, videoContainer } from "../src/outgoing_media.ts";

const mp4 = new Uint8Array(256);
mp4.set(new TextEncoder().encode("ftypisom"), 4);
const webm = new Uint8Array(256);
webm.set([0x1a, 0x45, 0xdf, 0xa3]);
const poster = new Uint8Array(250_000);
poster.set([0xff, 0xd8, 0xff]);

async function fixture(test: (f: ReturnType<typeof make>) => Promise<void>) {
  const dir = await Deno.makeTempDir();
  try {
    await test(make(dir));
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
}
function make(dir: string) {
  let cached: Record<string, unknown>[] | undefined = [{
    id: "1",
    contentType: "VIDEO",
  }];
  let fresh: Uint8Array = webm;
  let fetches = 0;
  const replies: unknown[] = [];
  const errors: string[] = [];
  const mediaDest = (id: string, ext = "jpg") => join(dir, `${id}.${ext}`);
  const exists = async (path: string) => {
    try {
      return (await Deno.stat(path)).isFile ? path : null;
    } catch {
      return null;
    }
  };
  const service = createOutgoingMedia({
    getClient: () => ({}) as Client,
    mediaDir: dir,
    mediaDest,
    fullDest: (id, ext = "jpg") => join(dir, `${id}.full.${ext}`),
    thumbDest: (id) => join(dir, `${id}.thumb.jpg`),
    existingFile: exists,
    existingImage: exists,
    isImageBytes: (bytes) => bytes[0] === 0xff && bytes[1] === 0xd8,
    writeImageFile: async (dest, bytes) => {
      await Deno.writeFile(dest, bytes);
      return dest;
    },
    writeFullImage: () => Promise.resolve(null),
    refetchMessageData: () => {
      fetches++;
      return Promise.resolve({ buf: fresh, mime: "video/webm" });
    },
    uiMediaPath: (_, path) => Promise.resolve(path),
    isPreviewOrThumbImage: () => Promise.resolve(false),
    getBoxCursor: () => undefined,
    isMineFrom: () => false,
    normType: String,
    getCachedMessages: () => cached,
    invalidateMessages() {},
    invalidateChats() {},
    sentMessagePayload: () => ({}),
    touchChatPreviewFromMessage() {},
    emitEvent() {},
    ok: (_, result) => replies.push(result),
    fail: (_, error) => errors.push(error),
  });
  return {
    dir,
    service,
    replies,
    errors,
    mediaDest,
    fetches: () => fetches,
    withoutMessage: () => {
      cached = undefined;
    },
    fresh: (bytes: Uint8Array) => {
      fresh = bytes;
    },
  };
}

Deno.test("動画コンテナを判別し、画像・HTML・短いデータを拒否する", () => {
  assertEquals(videoContainer(mp4), "mp4");
  assertEquals(videoContainer(webm), "webm");
  assertEquals(videoContainer(poster), null);
  assertEquals(
    videoContainer(new TextEncoder().encode("<html>".repeat(30))),
    null,
  );
  assertEquals(videoContainer(mp4.slice(0, 12)), null);
});
Deno.test("200KBを超えるプレビュー画像よりキャッシュの動画原本を選ぶ", () =>
  fixture(async (f) => {
    await Deno.writeFile(join(f.dir, "1.full.jpg"), poster);
    const path = f.mediaDest("1", "mp4");
    await Deno.writeFile(path, mp4);
    await f.service.downloadMedia(7, "u-peer", "1");
    assertEquals(f.replies, [{
      path,
      fileName: "1.mp4",
      contentType: "VIDEO",
      bytes: 256,
      cached: true,
    }]);
    assertEquals(f.fetches(), 0);
    assertEquals(f.errors, []);
  }));
Deno.test("履歴キャッシュ外でもVIDEOヒントを使い、画像や壊れたmp4を再取得する", () =>
  fixture(async (f) => {
    f.withoutMessage();
    await Deno.writeFile(join(f.dir, "1.full.jpg"), poster);
    await Deno.writeFile(f.mediaDest("1", "mp4"), new Uint8Array(256));
    await f.service.downloadMedia(7, "u-peer", "1", "VIDEO");
    assertEquals(f.fetches(), 1);
    assertEquals(await Deno.readFile(f.mediaDest("1", "webm")), webm);
    assertEquals(f.errors, []);
    await f.service.downloadMedia(8, "u-peer", "1", "VIDEO");
    assertEquals(f.fetches(), 1);
  }));
Deno.test("サーバーが画像しか返せない動画をmp4として保存しない", () =>
  fixture(async (f) => {
    f.fresh(poster);
    await f.service.downloadMedia(7, "u-peer", "1");
    assertEquals(f.replies, []);
    assertEquals(f.errors, ["video_original_unavailable"]);
    assertEquals(Array.from(Deno.readDirSync(f.dir)), []);
  }));
Deno.test("動画プレビューの展開で不明なデータを動画キャッシュにしない", () =>
  fixture(async (f) => {
    const result = await f.service.materializeVideoPreview(
      "1",
      new Uint8Array(512),
    );
    assertEquals(result, null);
    assertEquals(Array.from(Deno.readDirSync(f.dir)), []);
  }));

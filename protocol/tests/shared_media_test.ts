import { assert, assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import {
  createSharedMediaService,
  type SharedMediaAnchor,
  sharedMediaItems,
  type SharedMediaRuntime,
  type SharedMediaSource,
  sharedMediaUrls,
} from "../src/shared_media.ts";

const message = (id: string | number, contentType = "IMAGE", text = "") => ({
  id: String(id),
  contentType,
  text,
  createdTime: Number(id),
  from: "u-friend",
  to: "c-group",
});
function fixture(dir: string) {
  let accountMid = "u-self";
  let owner = {};
  let calls = 0;
  let active = 0;
  let maximum = 0;
  let pages = 0;
  let seeds: SharedMediaSource[] = [];
  const removed = new Set<string>();
  const failed = new Set<string>();
  let sources: SharedMediaSource[] = [];
  let beforeDownload: (() => Promise<void>) | undefined;
  let fetchOverride:
    | ((anchor: SharedMediaAnchor | null) => Promise<SharedMediaSource[]>)
    | undefined;
  const runtime: SharedMediaRuntime = {
    cacheDir: dir,
    session: () => ({ accountMid, owner }),
    seed: () => Promise.resolve(seeds),
    fetchPage: (_chat, anchor) => {
      pages++;
      if (fetchOverride) return fetchOverride(anchor);
      return Promise.resolve(
        sources.filter((source) =>
          !anchor || Number(source.message.id) < Number(anchor.messageId)
        ).sort((a, b) => Number(b.message.id) - Number(a.message.id)).slice(
          0,
          100,
        ),
      );
    },
    mask: (_chat, messages) =>
      Promise.resolve(
        messages.map((msg) =>
          removed.has(String(msg.id)) ? { ...msg, contentType: "UNSENT" } : msg
        ),
      ),
    cacheAttachment: async (_chat, source, check) => {
      active++;
      maximum = Math.max(maximum, active);
      try {
        calls++;
        await beforeDownload?.();
        check();
        if (failed.has(String(source.message.id))) throw new Error("expired");
        const path = join(dir, `${accountMid}-${source.message.id}.bin`);
        await Deno.writeFile(path, new Uint8Array([1, 2, 3]));
        check();
        return {
          path,
          fileName: String(
            source.message.fileName ?? `${source.message.id}.bin`,
          ),
          contentType: String(source.message.contentType),
          bytes: 3,
        };
      } finally {
        active--;
      }
    },
  };
  return {
    runtime,
    service: createSharedMediaService(runtime),
    setSources(value: SharedMediaSource[]) {
      sources = value;
    },
    setSeeds(value: SharedMediaSource[]) {
      seeds = value;
    },
    setFetch(value: typeof fetchOverride) {
      fetchOverride = value;
    },
    setBeforeDownload(value: typeof beforeDownload) {
      beforeDownload = value;
    },
    switchAccount(value: string) {
      accountMid = value;
      owner = {};
    },
    calls: () => calls,
    pages: () => pages,
    maximum: () => maximum,
    removed,
    failed,
  };
}
async function temporary(test: (dir: string) => Promise<void>) {
  const dir = await Deno.makeTempDir();
  try {
    await test(dir);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
}

Deno.test("shared media classifies only attachments and message URLs", () => {
  assertEquals(
    sharedMediaItems("c-group", message(1, "IMAGE"))[0].kind,
    "image",
  );
  assertEquals(
    sharedMediaItems("c-group", message(2, "VIDEO"))[0].kind,
    "video",
  );
  assertEquals(sharedMediaItems("c-group", message(3, "FILE"))[0].kind, "file");
  for (const ct of ["AUDIO", "STICKER", "CALL", "UNSENT", "FLEX"]) {
    assertEquals(
      sharedMediaItems("c-group", message(1, ct, "https://x.test")),
      [],
    );
  }
  assertEquals(
    sharedMediaItems("c-group", {
      ...message(1),
      decryptionFailed: true,
    }),
    [],
  );
  assertEquals(
    sharedMediaUrls(
      "(https://example.test/写真?q=一), HTTPS://EXAMPLE.test/a_(b). https://example.test/写真?q=一 https://bad",
    ),
    [
      "https://example.test/写真?q=一",
      "HTTPS://EXAMPLE.test/a_(b)",
      "https://bad",
    ],
  );
  const links = sharedMediaItems(
    "c-group",
    message(1, "NONE", "https://a.test https://b.test https://a.test"),
  );
  assertEquals(links.map((item) => item.key), ["1:url:0", "1:url:1"]);
  assertEquals(
    sharedMediaItems("c-group", { ...message(1), fileSize: 123 })[0].fileSize,
    123,
  );
});

Deno.test("shared cache seeds immediately, indexes sparse history once for all tabs, and limits transfers", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSeeds([{ message: message(999, "FILE") }]);
    f.setSources(Array.from({ length: 205 }, (_, i) => ({
      message: message(
        i + 1,
        i < 5 ? "IMAGE" : "NONE",
        i === 120 ? "https://link.test" : "",
      ),
    })));
    f.setBeforeDownload(() => new Promise((resolve) => setTimeout(resolve, 1)));
    const initial = await f.service.list("c-group", "files");
    assertEquals(initial.items.map((item) => item.messageId), ["999"]);
    await f.service.idle("c-group");
    const photos = await f.service.list("c-group", "photos_videos");
    assertEquals(photos.items.map((item) => item.messageId), [
      "5",
      "4",
      "3",
      "2",
      "1",
    ]);
    assertEquals(photos.scanState, "end");
    assertEquals((await f.service.list("c-group", "links")).items.length, 1);
    assertEquals(f.calls(), 6);
    assert(f.maximum() <= 3);
    assertEquals(f.pages(), 4);
    await f.service.pause("c-group");
  }));

Deno.test("shared metadata and raw retrieval source survive restart with exact IDs and bytes", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    const id = "9223372036854775807";
    f.setSources([{
      message: { ...message(1), id },
      raw: { id: BigInt(id), chunks: [new Uint8Array([1, 255, 0])] },
    }]);
    // This fixture's large ID has a low timestamp; use an explicit final page.
    f.setFetch((anchor) =>
      Promise.resolve(
        anchor ? [] : [{
          message: { ...message(1), id },
          raw: { id: BigInt(id), chunks: [new Uint8Array([1, 255, 0])] },
        }],
      )
    );
    await f.service.list("c-group", "photos_videos");
    await f.service.idle("c-group");
    await f.service.pause("c-group");
    const restarted = createSharedMediaService(f.runtime);
    const source = await restarted.source("c-group", id);
    assertEquals(source?.raw?.id, id);
    assertEquals(source?.raw?.chunks, [new Uint8Array([1, 255, 0])]);
    const first = await restarted.list("c-group", "photos_videos");
    assertEquals(first.items[0].messageId, id);
    assertEquals(first.items[0].cacheState, "cached");
    await restarted.idle("c-group");
    assertEquals(f.calls(), 1);
    await restarted.pause("c-group");
    const index = [...Deno.readDirSync(dir)].find((file) =>
      file.name.startsWith("shared-media-")
    );
    assert(index);
    if (Deno.build.os !== "windows") {
      assertEquals(
        (await Deno.stat(join(dir, index.name))).mode! & 0o777,
        0o600,
      );
    }
  }));

Deno.test("local cursor pagination retains overflow and sorts equal times using exact integer IDs", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSeeds(
      ["9223372036854775806", "9223372036854775807", "7"].map((id) => ({
        message: { ...message(1, "NONE", "https://a.test"), id },
      })),
    );
    const first = await f.service.list("c-group", "links", "", 2);
    assertEquals(first.items.map((item) => item.messageId), [
      "9223372036854775807",
      "9223372036854775806",
    ]);
    assert(first.nextCursor);
    const next = await f.service.list("c-group", "links", first.nextCursor, 2);
    assertEquals(next.items.map((item) => item.messageId), ["7"]);
    await assertRejects(
      () => f.service.list("c-group", "files", first.nextCursor),
      Error,
      "invalid_cursor",
    );
    await f.service.idle("c-group");
    await f.service.pause("c-group");
  }));

Deno.test("failed originals do not block discovery and retry does not repeat successful transfers", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSources([1, 2, 3].map((id) => ({ message: message(id) })));
    f.failed.add("2");
    await f.service.list("c-group", "photos_videos");
    await f.service.idle("c-group");
    const page = await f.service.list("c-group", "photos_videos");
    assertEquals(page.failedCount, 1);
    assertEquals(page.cachedCount, 2);
    const [a, b] = await Promise.all([
      f.service.download("c-group", "1"),
      f.service.download("c-group", "1"),
    ]);
    assertEquals(a, b);
    assertEquals(f.calls(), 3);
    f.failed.delete("2");
    await f.service.download("c-group", "2", true);
    assertEquals(f.calls(), 4);
    await f.service.pause("c-group");
  }));

Deno.test("unsend masking removes indexed items and rejects download without restoring them", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSources([{ message: message(1) }]);
    await f.service.list("c-group", "photos_videos");
    await f.service.idle("c-group");
    f.removed.add("1");
    assertEquals((await f.service.list("c-group", "photos_videos")).items, []);
    await assertRejects(
      () => f.service.download("c-group", "1"),
      Error,
      "unavailable",
    );
    await f.service.pause("c-group");
  }));

Deno.test("session changes reject stale downloads and separate account indexes", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSources([{ message: message(1) }]);
    let entered!: () => void;
    const started = new Promise<void>((resolve) => {
      entered = resolve;
    });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    f.setBeforeDownload(async () => {
      entered();
      await gate;
    });
    await f.service.list("c-group", "photos_videos");
    await started;
    f.switchAccount("u-other");
    release();
    // Drain the previous owner before constructing a new one.
    await new Promise((resolve) => setTimeout(resolve, 5));
    f.setBeforeDownload(undefined);
    f.setSources([]);
    const page = await f.service.list("c-group", "photos_videos");
    assertEquals(page.items, []);
    await f.service.idle("c-group");
    assert(
      ![...Deno.readDirSync(dir)].some((file) => file.name === "u-other-1.bin"),
    );
    await f.service.pause("c-group");
  }));

Deno.test("stalled history reports an error without claiming completion or retrying on every poll", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setFetch(() => Promise.resolve([{ message: message(1, "NONE") }]));
    await f.service.list("c-group", "links");
    await f.service.idle("c-group");
    const calls = f.pages();
    const page = await f.service.list("c-group", "links");
    assertEquals(page.scanState, "more");
    assertEquals(page.error, "shared_media_pagination_stalled");
    await f.service.idle("c-group");
    assertEquals(f.pages(), calls);
    await f.service.pause("c-group");
  }));

Deno.test("explicit clearing stops work, removes indexes, and media clear allows later refill", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSources([{ message: message(1) }]);
    await f.service.list("c-group", "photos_videos");
    await f.service.idle("c-group");
    await f.service.clear("media");
    // Main's existing clear handler removes the media file itself.
    await Deno.remove(join(dir, "u-self-1.bin"));
    await f.service.list("c-group", "photos_videos");
    await f.service.idle("c-group");
    assertEquals(f.calls(), 2);
    await f.service.clear("messages");
    assert(
      ![...Deno.readDirSync(dir)].some((file) =>
        file.name.startsWith("shared-media-")
      ),
    );
  }));

Deno.test("clear during an in-flight original cannot republish the index or original", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSources([{ message: message(1) }]);
    let entered!: () => void;
    const started = new Promise<void>((resolve) => {
      entered = resolve;
    });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    f.setBeforeDownload(async () => {
      entered();
      await gate;
    });
    await f.service.list("c-group", "photos_videos");
    await started;
    const clearing = f.service.clear("messages");
    release();
    await clearing;
    assertEquals([...Deno.readDirSync(dir)], []);
  }));

Deno.test("pause persists discovery progress and reopening refreshes only new history", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSources(
      [1, 2].map((id) => ({ message: message(id, "NONE", "https://x.test") })),
    );
    await f.service.list("c-group", "links");
    await f.service.idle("c-group");
    const initialPages = f.pages();
    await f.service.list("c-group", "links");
    await f.service.idle("c-group");
    assertEquals(f.pages(), initialPages);
    await f.service.pause("c-group");
    f.setSources(
      [1, 2, 3].map((id) => ({
        message: message(id, "NONE", "https://x.test"),
      })),
    );
    await f.service.list("c-group", "links");
    await f.service.idle("c-group");
    assertEquals(
      (await f.service.list("c-group", "links")).items.map((item) =>
        item.messageId
      ),
      ["3", "2", "1"],
    );
    assertEquals(f.pages(), initialPages + 1);
    await f.service.pause("c-group");
  }));

Deno.test("live messages update the existing index and unsend invalidates late originals", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    await f.service.list("c-group", "photos_videos");
    await f.service.idle("c-group");
    let entered!: () => void;
    const started = new Promise<void>((resolve) => {
      entered = resolve;
    });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    f.setBeforeDownload(async () => {
      entered();
      await gate;
    });
    await f.service.observe("c-group", [{ message: message(10) }]);
    await started;
    f.removed.add("10");
    await f.service.unsent("c-group", "10");
    release();
    await f.service.idle("c-group");
    assertEquals((await f.service.list("c-group", "photos_videos")).items, []);
    await f.service.pause("c-group");
  }));

Deno.test("new raw metadata retries a failed seed original and uses wire delivery time for history", () =>
  temporary(async (dir) => {
    const f = fixture(dir);
    f.setSeeds([{ message: message(1) }]);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const seenAnchors: Array<SharedMediaAnchor | null> = [];
    f.setFetch(async (anchor) => {
      seenAnchors.push(anchor);
      if (anchor) return [];
      await gate;
      return [{ message: message(1), raw: { id: "1", deliveredTime: "77" } }];
    });
    const cache = f.runtime.cacheAttachment;
    f.runtime.cacheAttachment = (chat, source, check) => {
      if (!source.raw) return Promise.reject(new Error("source_missing"));
      return cache(chat, source, check);
    };
    await f.service.list("c-group", "photos_videos");
    await assertRejects(
      () => f.service.download("c-group", "1"),
      Error,
      "source_missing",
    );
    release();
    await f.service.idle("c-group");
    assertEquals(
      (await f.service.list("c-group", "photos_videos")).items[0].cacheState,
      "cached",
    );
    assertEquals(seenAnchors, [null, { messageId: "1", createdTime: 77 }]);
    assertEquals(f.calls(), 1);
    await f.service.pause("c-group");
  }));

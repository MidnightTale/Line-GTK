import { assert, assertEquals, assertRejects, assertThrows } from "@std/assert";
import { join } from "@std/path";
import { createStickerLibrary } from "../src/sticker_library.ts";
import { MAX_PNG_BYTES, validateStickerPng } from "../src/sticker_png.ts";
import {
  animatedPng,
  differentPng,
  inflationBombPng,
  oversizedPng,
  transparentPng,
} from "./fixtures/sticker_png.ts";

import { libraryFixture } from "./fixtures/sticker_library.ts";

async function fixture(
  run: (f: Awaited<ReturnType<typeof libraryFixture>>) => Promise<void>,
) {
  const f = await libraryFixture();
  try {
    await run(f);
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
}

Deno.test("PNG validation rejects corrupt, animated, oversized and overinflated input", () => {
  assertEquals(validateStickerPng(transparentPng), {
    width: 1,
    height: 1,
    animated: false,
    frames: 1,
    durationMs: 0,
  });
  for (
    const [bytes, error] of [
      [animatedPng, "sticker_png_animated"],
      [oversizedPng, "sticker_png_dimensions"],
      [inflationBombPng, "sticker_png_invalid"],
      [new Uint8Array(MAX_PNG_BYTES + 1), "sticker_png_too_large"],
    ] as const
  ) {
    assertThrows(() => validateStickerPng(bytes), Error, error);
  }
  const corrupt = transparentPng.slice();
  corrupt[45] ^= 255;
  assertThrows(() => validateStickerPng(corrupt), Error, "sticker_png_invalid");
  assertThrows(
    () => validateStickerPng(new TextEncoder().encode("renamed JPEG")),
    Error,
    "sticker_png_invalid",
  );
});

Deno.test("personal PNG copies survive source deletion, cache clear, and restart", () =>
  fixture(async (f) => {
    const source = await f.source("sticker.png");
    const added = await f.library.importPngs("1", [source], "My PNGs");
    assertEquals(added.errors, []);
    assertEquals(added.added, 1);
    const pack = added.packs[0];
    const item = pack.stickers[0];
    assertEquals(item.personal, true);
    assertEquals(pack.imageOnly, true);
    await Deno.remove(source);
    await Deno.remove(join(f.dataDir, "cache"), { recursive: true });
    const restored = await createStickerLibrary(f.runtime).list();
    assertEquals(restored.packs[0].stickers[0].optionKey, item.optionKey);
    assertEquals(await Deno.readFile(item.imagePath!), transparentPng);
    assertEquals(f.requests.length, 0);
  }));

Deno.test("content deduplication preserves item identity and independent pack settings", () =>
  fixture(async (f) => {
    const a = await f.source("same.png");
    const first = await f.library.importPngs("1", [a, a], "First");
    const packId = first.packs[0].id;
    assertEquals(first.packs[0].stickers.length, 1);
    const key = first.packs[0].stickers[0].optionKey;
    const duplicate = await f.library.importPngs("2", [a], "", packId);
    assertEquals(duplicate.added, 0);
    assertEquals(duplicate.packs[0].stickers[0].optionKey, key);
    await f.source("same.png", differentPng);
    const changed = await f.library.importPngs("3", [a], "", packId);
    assertEquals(changed.packs[0].stickers.length, 2);
    const b = await f.source("other.png");
    const second = await f.library.importPngs("4", [b], "Second");
    assert(second.packs[0].stickers[0].optionKey !== key);
    assertEquals(
      second.packs[0].stickers[0].imagePath,
      first.packs[0].stickers[0].imagePath,
    );
  }));

Deno.test("PNG review and import return per-file failures and preserve valid files", () =>
  fixture(async (f) => {
    const good = await f.source("good.png");
    const bad = await f.source("bad.png", animatedPng);
    const review = await f.library.inspectPngs([
      good,
      bad,
      join(f.root, "missing.png"),
    ]);
    assertEquals(review.files.map((i) => i.valid), [true, false, false]);
    const imported = await f.library.importPngs("1", [good, bad], "Mixed");
    assertEquals(imported.added, 1);
    assertEquals(imported.errors, [{
      index: 1,
      error: "sticker_png_invalid",
    }]);
    await assertRejects(
      () => f.library.importPngs("2", [], "Empty"),
      Error,
      "sticker_png_batch",
    );
    await assertRejects(
      () => f.library.importPngs("3", Array(101).fill(good), "Too many"),
      Error,
      "sticker_png_batch",
    );
    await assertRejects(
      () => f.library.importPngs("4", [good], "  "),
      Error,
      "sticker_pack_name_invalid",
    );
  }));

Deno.test("creator downloads expose partial state and retry only missing originals", () =>
  fixture(async (f) => {
    f.unavailable.add("11");
    const first = await f.library.importStore(
      "1",
      "https://store.line.me/stickershop/product/2/en",
    );
    assertEquals(first.packs[0].incomplete, true);
    assertEquals(first.packs[0].stickers.length, 2);
    assert(first.packs[0].stickers[0].imagePath);
    const firstRequests = f.requests.filter((url) =>
      url.includes("/sticker/10/")
    ).length;
    f.unavailable.clear();
    const retry = await f.library.importStore("2", "2");
    assertEquals(retry.packs[0].incomplete, false);
    assertEquals(
      retry.packs[0].stickers[0].optionKey,
      first.packs[0].stickers[0].optionKey,
    );
    assertEquals(
      f.requests.filter((url) => url.includes("/sticker/10/")).length,
      firstRequests,
    );
    f.requests.length = 0;
    assertEquals(
      (await createStickerLibrary(f.runtime).list()).packs.length,
      1,
    );
    assertEquals(f.requests, []);
    assertEquals(f.events.at(-1)?.done, 2);
  }));

Deno.test("invalid store links and metadata failure leave existing packs intact", () =>
  fixture(async (f) => {
    await f.library.importPngs("1", [await f.source("one.png")], "Keep");
    await assertRejects(
      () => f.library.importStore("2", "https://evil.test/2"),
      Error,
      "sticker_import_invalid",
    );
    const unavailable = createStickerLibrary({
      ...f.runtime,
      fetch: (() =>
        Promise.resolve(new Response(null, { status: 404 }))) as typeof fetch,
    });
    await assertRejects(
      () => unavailable.importStore("3", "2"),
      Error,
      "sticker_import_unavailable",
    );
    assertEquals((await f.library.list()).packs[0].name, "Keep");
  }));

Deno.test("account isolation and changed session prevent late import publication", () =>
  fixture(async (f) => {
    await f.library.importPngs("1", [await f.source("one.png")], "Account one");
    f.switchAccount("u-two");
    assertEquals((await f.library.list()).packs, []);
    f.onFetch(() => {
      f.switchAccount("u-three");
    });
    await assertRejects(
      () => f.library.importStore("2", "2"),
      Error,
      "sticker_library_session_changed",
    );
    assertEquals((await f.library.list()).packs, []);
    f.switchAccount("u-one");
    assertEquals((await f.library.list()).packs[0].name, "Account one");
    f.logout();
    await assertRejects(
      () => f.library.list(),
      Error,
      "sticker_library_not_logged_in",
    );
  }));

Deno.test("concurrent local imports serialize without overwriting packs", () =>
  fixture(async (f) => {
    const source = await f.source("one.png");
    await Promise.all([
      f.library.importPngs("1", [source], "First"),
      f.library.importPngs("2", [source], "Second"),
    ]);
    assertEquals((await f.library.list()).packs.length, 2);
  }));

Deno.test("cancelled creator import is retryable and does not continue downloads", () =>
  fixture(async (f) => {
    f.onFetch(() => {
      if (f.requests.some((u) => u.includes("/sticker/"))) {
        f.library.cancel("1");
      }
    });
    const result = await f.library.importStore("1", "2");
    assertEquals(result.cancelled, true);
    assertEquals(result.packs[0].incomplete, true);
  }));

Deno.test("legacy packs require explicit adoption and migrate idempotently", () =>
  fixture(async (f) => {
    await Deno.writeFile(join(f.stickerDir, "10.png"), transparentPng);
    await Deno.writeTextFile(
      join(f.stickerDir, "image-packs.json"),
      JSON.stringify([{
        id: "2",
        name: "Old pack",
        version: "1",
        stickers: [{ stickerId: "10", packageId: "2" }, {
          stickerId: "../bad",
        }],
      }]),
    );
    const before = await f.library.list();
    assertEquals(before.packs, []);
    assertEquals(before.legacyAvailable, true);
    const adopted = await f.library.adoptLegacy();
    assertEquals(adopted.packs.length, 1);
    assertEquals(adopted.legacyAvailable, false);
    assertEquals(Object.values(adopted.migratedOptions!), ["10.png"]);
    assertEquals((await f.library.adoptLegacy()).packs.length, 1);
    assert(await Deno.stat(join(f.stickerDir, "image-packs.json")));
  }));

Deno.test("rename and removal preserve unrelated packs and source files", () =>
  fixture(async (f) => {
    const source = await f.source("one.png");
    const first = await f.library.importPngs("1", [source], "First");
    const id = first.packs[0].id;
    await f.library.importPngs("2", [source], "Other");
    await f.library.update(id, "rename", "Renamed");
    assertEquals(
      (await f.library.list()).packs.find((p) => p.id === id)?.name,
      "Renamed",
    );
    await f.library.update(
      id,
      "remove_item",
      first.packs[0].stickers[0].stickerId,
    );
    assertEquals(
      (await f.library.list()).packs.find((p) => p.id === id)?.stickers,
      [],
    );
    await f.library.update(id, "remove_pack", "");
    assertEquals((await f.library.list()).packs[0].name, "Other");
    assertEquals(await Deno.readFile(source), transparentPng);
  }));

Deno.test("reviewed PNG changes are rejected before copying into a pack", () =>
  fixture(async (f) => {
    const source = await f.source("reviewed.png");
    const review = await f.library.inspectPngs([source]);
    const digest = review.files[0].digest!;
    await Deno.writeFile(source, differentPng);
    const result = await f.library.importPngs(
      "1",
      [source],
      "Reviewed",
      undefined,
      [digest],
    );
    assertEquals(result.errors, [{ index: 0, error: "sticker_png_changed" }]);
    assertEquals(result.packs.length, 0);
  }));

Deno.test("cancelling a PNG batch retains each already committed image", () =>
  fixture(async (f) => {
    const source = await f.source("one.png");
    f.runtime.emitEvent = (_name, event) => {
      if (event.done === 1) f.library.cancel("1");
    };
    const result = await f.library.importPngs(
      "1",
      [source, source],
      "Cancelled",
    );
    assertEquals(result.cancelled, true);
    assertEquals((await f.library.list()).packs[0].stickers.length, 1);
  }));

Deno.test("removal retains shared originals until their last library reference disappears", () =>
  fixture(async (f) => {
    const source = await f.source("one.png");
    const first = await f.library.importPngs("1", [source], "First");
    const id = first.packs[0].id;
    const path = first.packs[0].stickers[0].imagePath!;
    const second = await f.library.importPngs("2", [source], "Second");
    await f.library.update(id, "remove_pack", "");
    assertEquals(await Deno.readFile(path), transparentPng);
    await f.library.update(second.packs[0].id, "remove_pack", "");
    await assertRejects(() => Deno.stat(path), Deno.errors.NotFound);
    assertEquals(await Deno.readFile(source), transparentPng);
  }));

Deno.test("a corrupt manifest is preserved instead of being overwritten by an import", () =>
  fixture(async (f) => {
    await f.library.importPngs("1", [await f.source("one.png")], "Keep");
    const accountDir =
      Array.from(Deno.readDirSync(join(f.dataDir, "sticker-library")))[0].name;
    const manifest = join(
      f.dataDir,
      "sticker-library",
      accountDir,
      "index.json",
    );
    await Deno.writeTextFile(manifest, "broken JSON");
    await assertRejects(
      () => f.library.importPngs("2", [join(f.root, "one.png")], "New"),
      Error,
      "sticker_library_invalid",
    );
    assertEquals(await Deno.readTextFile(manifest), "broken JSON");
  }));

Deno.test("preview copies are clearable cache entries, not unconfirmed library originals", () =>
  fixture(async (f) => {
    const review = await f.library.inspectPngs([await f.source("preview.png")]);
    assert(review.files[0].previewPath!.startsWith(f.stickerDir));
    const creator = await f.library.preview("2");
    assert(creator.previews.every((path) => path.startsWith(f.stickerDir)));
    assertEquals((await f.library.list()).packs, []);
    await assertRejects(
      () => Deno.stat(join(f.dataDir, "sticker-library")),
      Deno.errors.NotFound,
    );
  }));

Deno.test("clearing previews cancels and drains writers before cache deletion", () =>
  fixture(async (f) => {
    const library = createStickerLibrary({
      ...f.runtime,
      fetch: ((_url, options) =>
        new Promise((_resolve, reject) => {
          options!.signal!.addEventListener(
            "abort",
            () => reject(new DOMException("Aborted", "AbortError")),
            { once: true },
          );
        })) as typeof fetch,
    });
    const preview = library.preview("2");
    const rejected = assertRejects(
      () => preview,
      Error,
      "sticker_import_cancelled",
    );
    await library.clearPreviews();
    await rejected;
    assertEquals((await library.list()).packs, []);
  }));

Deno.test("retrying a store pack does not restore intentionally removed images", () =>
  fixture(async (f) => {
    const first = await f.library.importStore("1", "2");
    await f.library.update(first.packs[0].id, "remove_item", "10");
    const retried = await f.library.importStore("2", "2");
    assertEquals(retried.packs[0].stickers.map((i) => i.stickerId), ["11"]);
  }));

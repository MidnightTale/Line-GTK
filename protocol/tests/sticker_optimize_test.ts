import { assert, assertEquals, assertRejects, assertThrows } from "@std/assert";
import { createStickerLibrary } from "../src/sticker_library.ts";
import {
  DEFAULT_ANIMATION_OPTIMIZATION,
  DEFAULT_OPTIMIZATION,
  optimizationOptions,
  optimizeStickerImage,
} from "../src/sticker_optimize.ts";
import { validateStickerPng } from "../src/sticker_png.ts";
import { libraryFixture } from "./fixtures/sticker_library.ts";
import {
  animatedPng,
  largeStaticPng,
  transparentPng,
  validAnimatedPng,
} from "./fixtures/sticker_png.ts";

Deno.test("APNG validation checks frame data, sequence, canvas bounds and animation budget", () => {
  assertEquals(validateStickerPng(validAnimatedPng, true), {
    width: 16,
    height: 16,
    animated: true,
    frames: 2,
    durationMs: 400,
  });
  assertThrows(
    () => validateStickerPng(validAnimatedPng),
    Error,
    "sticker_png_animated",
  );
  assertThrows(
    () => validateStickerPng(animatedPng, true),
    Error,
    "sticker_png_invalid",
  );
  const corrupt = validAnimatedPng.slice();
  corrupt[corrupt.length - 20] ^= 1;
  assertThrows(
    () => validateStickerPng(corrupt, true),
    Error,
    "sticker_png_invalid",
  );
  for (
    const options of [{ fps: 0 }, { colors: 300 }, { maxDimension: 99999 }, {
      ffmpeg: "-bad",
    }]
  ) {
    assertThrows(
      () => optimizationOptions(options),
      Error,
      "sticker_optimize_invalid",
    );
  }
});

Deno.test("APNG conversion creates an animated GIF with timing and transparent pixels", async () => {
  const f = await libraryFixture();
  try {
    const source = await f.source("animation.png", validAnimatedPng);
    const imported = await f.library.importPngs("1", [source], "Animated");
    const item = imported.packs[0].stickers[0];
    assertEquals(item.animated, true);
    assertEquals(item.optimization, DEFAULT_ANIMATION_OPTIMIZATION);
    const review = await f.library.inspectPngs([source]);
    assertEquals(review.files[0].animated, true);
    const automatic = await f.library.prepareImage("auto", item.optionKey);
    assertEquals(automatic.options, DEFAULT_ANIMATION_OPTIMIZATION);
    const result = await f.library.prepareImage("2", item.optionKey, {
      maxDimension: 128,
      fps: 10,
      colors: 64,
    });
    assertEquals(result.animated, true);
    assert(result.path.endsWith(".gif"));
    const bytes = await Deno.readFile(result.path);
    assertEquals(new TextDecoder().decode(bytes.subarray(0, 6)), "GIF89a");
    const probe = await new Deno.Command("ffprobe", {
      args: [
        "-v",
        "error",
        "-count_frames",
        "-show_entries",
        "stream=width,height,nb_read_frames,duration",
        "-of",
        "json",
        result.path,
      ],
      stdout: "piped",
      stderr: "piped",
    }).output();
    assert(probe.success);
    const info = JSON.parse(new TextDecoder().decode(probe.stdout)).streams[0];
    assertEquals([info.width, info.height], [16, 16]);
    assert(Number(info.nb_read_frames) >= 2);
    assert(Math.abs(Number(info.duration) - 0.4) < 0.11);
    const decoded = await new Deno.Command("ffmpeg", {
      args: [
        "-v",
        "error",
        "-i",
        result.path,
        "-frames:v",
        "1",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "pipe:1",
      ],
      stdout: "piped",
      stderr: "piped",
    }).output();
    assert(decoded.success);
    assertEquals(decoded.stdout[3], 0);
    assertEquals(decoded.stdout[(4 * 16 + 3) * 4 + 3], 255);
    assertEquals(await Deno.readFile(item.imagePath!), validAnimatedPng);
    await f.library.setOptimization(item.optionKey, result.options);
    const restored = createStickerLibrary(f.runtime);
    assertEquals((await restored.list()).packs[0].stickers[0].optimized, true);
    const reused = await restored.prepareImage("3", item.optionKey);
    assertEquals(reused.path, result.path);
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
});

Deno.test("lightweight animation defaults shrink downloads while explicit presets survive", async () => {
  const f = await libraryFixture();
  try {
    assertEquals(optimizationOptions(undefined), DEFAULT_OPTIMIZATION);
    assertEquals(
      optimizationOptions(undefined, true),
      DEFAULT_ANIMATION_OPTIMIZATION,
    );
    for (
      const [width, height, expected] of [
        [128, 96, 128],
        [96, 128, 128],
        [129, 96, 256],
        [96, 129, 256],
        [512, 384, 256],
      ]
    ) {
      assertEquals(
        optimizationOptions(undefined, true, { width, height }),
        { maxDimension: expected, fps: 10, colors: 32 },
      );
    }
    const width = 320, height = 240, count = 24;
    const frames = new Uint8Array(width * height * count * 4);
    for (let frame = 0; frame < count; frame++) {
      for (let y = 16; y < height - 16; y++) {
        for (let x = 16; x < width - 16; x++) {
          const offset = ((frame * height + y) * width + x) * 4;
          frames.set([
            (x * 3 + frame * 11) % 256,
            (y * 5 + frame * 7) % 256,
            (x + y + frame * 13) % 256,
            255,
          ], offset);
        }
      }
    }
    const generated = `${f.root}/generated.apng`;
    const encoder = new Deno.Command("ffmpeg", {
      args: [
        "-v",
        "error",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgba",
        "-video_size",
        `${width}x${height}`,
        "-framerate",
        "24",
        "-i",
        "pipe:0",
        "-frames:v",
        `${count}`,
        "-plays",
        "0",
        "-f",
        "apng",
        generated,
      ],
      stdin: "piped",
      stdout: "null",
      stderr: "null",
    }).spawn();
    const writer = encoder.stdin.getWriter();
    await writer.write(frames);
    await writer.close();
    assert((await encoder.status).success);
    // APNG needs a seekable destination to write its final frame count.
    const bytes = await Deno.readFile(generated);
    const imported = await f.library.importPngs("import", [
      await f.source("large-animation.png", bytes),
    ], "Large animation");
    assertEquals(imported.errors, []);
    const item = imported.packs[0].stickers[0];
    const expectedDefaults = { maxDimension: 256, fps: 10, colors: 32 };
    assertEquals(item.optimization, expectedDefaults);
    assertEquals(
      (await createStickerLibrary(f.runtime).list()).packs[0].stickers[0]
        .optimization,
      expectedDefaults,
    );
    const original = await f.library.prepareImage(
      "large",
      item.optionKey,
      DEFAULT_OPTIMIZATION,
    );
    const small = await f.library.prepareImage("small", item.optionKey);
    assertEquals(small.options, expectedDefaults);
    assert(small.optimizedBytes < original.optimizedBytes / 2);
    const probe = await new Deno.Command("ffprobe", {
      args: [
        "-v",
        "error",
        "-count_frames",
        "-show_entries",
        "stream=width,height,duration,nb_read_frames",
        "-of",
        "json",
        small.path,
      ],
      stdout: "piped",
      stderr: "piped",
    }).output();
    assert(probe.success);
    const stream =
      JSON.parse(new TextDecoder().decode(probe.stdout)).streams[0];
    assertEquals([stream.width, stream.height], [256, 192]);
    assertEquals(Number(stream.nb_read_frames), 10);
    assert(Math.abs(Number(stream.duration) - 1) < 0.11);
    await f.library.setOptimization(item.optionKey, DEFAULT_OPTIMIZATION);
    const restored = createStickerLibrary(f.runtime);
    const saved = await restored.prepareImage("saved", item.optionKey);
    assertEquals(saved.options, DEFAULT_OPTIMIZATION);
    assertEquals(saved.path, original.path);
    assertEquals(await Deno.readFile(item.imagePath!), bytes);
    const still = await f.library.importPngs("still", [
      await f.source("still.png"),
    ], "Still");
    assertEquals(
      still.packs.find((p) => p.name === "Still")!.stickers[0].optimization,
      DEFAULT_OPTIMIZATION,
    );
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
});

Deno.test("static optimization resizes proportionally and preserves the original PNG", async () => {
  const f = await libraryFixture();
  try {
    const result = await optimizeStickerImage(
      largeStaticPng,
      optimizationOptions({ maxDimension: 128 }),
      f.stickerDir,
      new AbortController().signal,
    );
    const info = validateStickerPng(result.bytes);
    assertEquals([info.width, info.height], [128, 64]);
    assertEquals(result.extension, "png");
    assertEquals(result.animated, false);
    assertEquals(result.originalBytes, largeStaticPng.length);
    const small = await optimizeStickerImage(
      transparentPng,
      optimizationOptions(undefined),
      f.stickerDir,
      new AbortController().signal,
    );
    assert(small.bytes.length <= transparentPng.length);
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
});

Deno.test("creator imports prefer APNG and keep usable stills when animation is unavailable", async () => {
  const f = await libraryFixture();
  try {
    const requests: string[] = [];
    let animationAvailable = true;
    const library = createStickerLibrary({
      ...f.runtime,
      fetch: ((url) => {
        const path = String(url);
        requests.push(path);
        if (path.endsWith("productInfo.meta")) {
          return Promise.resolve(
            Response.json({
              hasAnimation: true,
              title: "Animated creator",
              stickers: [{ id: 10 }],
            }),
          );
        }
        if (path.includes("sticker_animation")) {
          return Promise.resolve(
            animationAvailable
              ? new Response(validAnimatedPng)
              : new Response(null, { status: 404 }),
          );
        }
        return Promise.resolve(new Response(transparentPng));
      }) as typeof fetch,
    });
    animationAvailable = false;
    const fallback = await library.importStore("1", "2");
    assert(fallback.packs[0].stickers[0].imagePath);
    assertEquals(fallback.packs[0].incomplete, true);
    animationAvailable = true;
    const animated = await library.importStore("2", "2");
    assertEquals(animated.packs[0].stickers[0].animated, true);
    assertEquals(animated.packs[0].incomplete, false);
    assertEquals(
      animated.packs[0].stickers[0].optionKey,
      fallback.packs[0].stickers[0].optionKey,
    );
    assert(requests.some((url) => url.includes("sticker_animation.png")));
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
});

Deno.test("optimization cancellation, account changes and invalid keys do not publish results", async () => {
  const f = await libraryFixture();
  try {
    const controller = new AbortController();
    controller.abort();
    await assertRejects(() =>
      optimizeStickerImage(
        validAnimatedPng,
        optimizationOptions(undefined),
        f.stickerDir,
        controller.signal,
      )
    );
    const imported = await f.library.importPngs("1", [
      await f.source("one.png"),
    ], "One");
    const key = imported.packs[0].stickers[0].optionKey;
    await assertRejects(
      () => f.library.prepareImage("2", "wrong/account/key"),
      Error,
      "sticker_pack_missing",
    );
    await assertRejects(
      () =>
        f.library.prepareImage("3", key, {}, "/tmp/sticker-image-other.png"),
      Error,
      "sticker_optimize_invalid",
    );
    f.switchAccount("u-two");
    await assertRejects(
      () => f.library.prepareImage("4", key),
      Error,
      "sticker_pack_missing",
    );
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
});

Deno.test("clearing cache drains an active conversion without publishing a late GIF", async () => {
  const f = await libraryFixture();
  try {
    const imported = await f.library.importPngs("1", [
      await f.source("animated.png", validAnimatedPng),
    ], "Animated");
    const preparing = f.library.prepareImage(
      "2",
      imported.packs[0].stickers[0].optionKey,
    );
    const rejected = assertRejects(
      () => preparing,
      Error,
      "sticker_import_cancelled",
    );
    await new Promise((resolve) => setTimeout(resolve, 10));
    await f.library.clearPreviews();
    await rejected;
    const files = await Array.fromAsync(Deno.readDir(f.stickerDir));
    assertEquals(
      files.filter((f) =>
        f.name.startsWith("optimized-") ||
        f.name.startsWith(".sticker-optimize-")
      ).length,
      0,
    );
    assertEquals((await f.library.list()).packs[0].stickers[0].animated, true);
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
});

import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { stickerPackageId } from "../src/sticker_import.ts";
import { createStickerService } from "../src/stickers.ts";
import { policyFor } from "../src/cache_policy.ts";
import { libraryFixture } from "./fixtures/sticker_library.ts";

Deno.test("sticker pack input accepts official links and rejects arbitrary URLs", () => {
  for (
    const input of [
      " 2 ",
      "https://store.line.me/stickershop/product/2/en?from=search",
      "https://line.me/S/sticker/2",
    ]
  ) assertEquals(stickerPackageId(input), "2");
  for (
    const input of [
      "",
      "0",
      "../2",
      "https://example.com/stickershop/product/2/en",
      "https://store.line.me.evil.test/stickershop/product/2/en",
      "https://user@store.line.me/stickershop/product/2/en",
      "http://store.line.me/stickershop/product/2/en",
      "https://store.line.me/stickershop/product/2bad/en",
    ]
  ) assertEquals(stickerPackageId(input), null, input);
});

Deno.test("all owned creator packs are enumerated without downloading their grids", async () => {
  const f = await libraryFixture();
  try {
    const results: Record<string, unknown>[] = [];
    const errors: string[] = [];
    const offsets: number[] = [];
    const client = {
      base: {
        request: {
          request: (args: unknown[][]) => {
            const offset = Number(args.find((a) => a[1] === 3)?.[2]);
            offsets.push(offset);
            return Promise.resolve({
              productList: Array.from(
                { length: Math.min(80, 100 - offset) },
                (_, i) => ({
                  id: String(offset + i + 1),
                  name: `Creator ${offset + i + 1}`,
                }),
              ),
              totalSize: 100,
            });
          },
        },
      },
    } as unknown as Client;
    const policy = policyFor("smart");
    const service = createStickerService({
      getClient: () => client,
      library: f.library,
      dataDir: f.dataDir,
      stickerDir: f.stickerDir,
      refreshCachePolicy: () => Promise.resolve(policy),
      cachePolicy: () => policy,
      existingImage: () => Promise.resolve(null),
      cacheUrl: () => {
        throw new Error("grid eagerly downloaded");
      },
      sentMessagePayload: () => ({}),
      invalidateMessages() {},
      invalidateChats() {},
      touchChatPreviewFromMessage() {},
      ok: (_id, result) => results.push(result as Record<string, unknown>),
      fail: (_id, error) => errors.push(error),
    });
    await service.list(1);
    assertEquals(errors, []);
    assertEquals(offsets, [0, 80]);
    const packs = results[0].packs as Record<string, unknown>[];
    assertEquals(packs.length, 100);
    assertEquals(packs.every((p) => p.lazy === true), true);
    assertEquals(packs[99].name, "Creator 100");
    await service.list(2);
    assertEquals(offsets, [0, 80]);
    await service.ownedPack(3, "999");
    assertEquals(errors, ["sticker_not_owned"]);
  } finally {
    await Deno.remove(f.root, { recursive: true });
  }
});

import { join } from "@std/path";
import { createStickerLibrary } from "../../src/sticker_library.ts";
import { transparentPng } from "./sticker_png.ts";

export async function libraryFixture() {
  const root = await Deno.makeTempDir();
  const dataDir = join(root, "data");
  const stickerDir = join(dataDir, "cache", "stickers");
  await Deno.mkdir(stickerDir, { recursive: true });
  let active: { accountMid: string; owner: object } | null = {
    accountMid: "u-one",
    owner: {},
  };
  const requests: string[] = [];
  const unavailable = new Set<string>();
  const events: Record<string, unknown>[] = [];
  let onFetch: (() => void) | undefined;
  const runtime = {
    dataDir,
    stickerDir,
    session: () => active,
    emitEvent: (_name: string, event: Record<string, unknown>) => {
      events.push(event);
    },
    fetch: ((url: string | URL | Request) => {
      const path = String(url);
      requests.push(path);
      onFetch?.();
      if (path.endsWith("productInfo.meta")) {
        return Promise.resolve(
          Response.json({
            title: { en: "Creator pack" },
            version: "3",
            stickers: [{ id: 10 }, { id: 11 }, { id: "../invalid" }],
          }),
        );
      }
      return Promise.resolve(
        unavailable.has(path.match(/sticker\/(\d+)\//)?.[1] ?? "")
          ? new Response(null, { status: 404 })
          : new Response(transparentPng),
      );
    }) as typeof fetch,
  };
  const library = createStickerLibrary(runtime);
  return {
    root,
    dataDir,
    stickerDir,
    runtime,
    library,
    requests,
    unavailable,
    events,
    switchAccount: (mid: string) => {
      active = { accountMid: mid, owner: {} };
    },
    logout: () => {
      active = null;
    },
    onFetch: (fn: () => void) => {
      onFetch = fn;
    },
    source: async (name: string, bytes = transparentPng) => {
      const path = join(root, name);
      await Deno.writeFile(path, bytes);
      return path;
    },
  };
}

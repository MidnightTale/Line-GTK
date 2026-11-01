import { assertEquals, assertRejects } from "@std/assert";
import { createEmojiService } from "../src/emojis.ts";
import {
  createReactionEmojiPacks,
  ownedPacks,
  packEmojiIds,
} from "../src/reaction_emojis.ts";

type Json = Record<string, unknown>;
const A = "5ac1bfd5040ab15980c9b435";
const B = "5ac21a8c040ab15980c9b43f";
const png = new Uint8Array([
  137,
  80,
  78,
  71,
  13,
  10,
  26,
  10,
  ...new Array(20).fill(0),
]);

Deno.test("opening a large pack loads every image, including emoji after the first 100", async () => {
  const directory = await Deno.makeTempDir();
  const ids = Array.from(
    { length: 211 },
    (_, i) => String(i + 1).padStart(3, "0"),
  );
  const image = Uint8Array.from(
    atob(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aK1sAAAAASUVORK5CYII=",
    ),
    (c) => c.charCodeAt(0),
  );
  const ready = new Set<string>();
  const downloads: string[] = [];
  const images = createEmojiService({
    directory,
    emit: (key) => {
      ready.add(key);
    },
    fetch: (input) => {
      downloads.push(String(input));
      return Promise.resolve(new Response(image));
    },
  });
  const packs = createReactionEmojiPacks({
    directory,
    accountMid: () => "u-self",
    ownedProducts: () => Promise.resolve({ productList: [] }),
    observe: (messages) => images.observe(messages),
    fetch: () => Promise.resolve(Response.json({ orders: ids })),
  });
  try {
    assertEquals((await packs.pack(A)).emojis.length, ids.length);
    const deadline = Date.now() + 5000;
    // Wait for the loader to drain before cleaning up its temporary directory.
    while (ready.size < ids.length && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    assertEquals([...ready].sort(), ids.map((id) => `${A}_${id}`));
    assertEquals(downloads.length, ids.length);
    // Reopening a pack should also load all images from disk without a fetch.
    ready.clear();
    await packs.pack(A);
    const cachedDeadline = Date.now() + 5000;
    while (ready.size < ids.length && Date.now() < cachedDeadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    assertEquals([...ready].sort(), ids.map((id) => `${A}_${id}`));
    assertEquals(downloads.length, ids.length);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("所有している絵文字パックを重複なく読み取り、不正なIDを捨てる", () => {
  assertEquals(
    ownedPacks({
      productList: [
        { id: A, name: "ムーン" },
        { id: A, name: "重複" },
        { id: "../x", name: "不正" },
        { "1": B, "11": "コニー" },
      ],
    }),
    [{ id: A, name: "ムーン" }, { id: B, name: "コニー" }],
  );
  assertEquals(ownedPacks(undefined), []);
  assertEquals(ownedPacks({ productList: "x" }), []);
});

Deno.test("パックの絵文字IDはLINEの並び順で読み、不正な値を捨てる", () => {
  assertEquals(packEmojiIds({ orders: ["002", "001", "x", "002", 3] }), [
    "002",
    "001",
    "3",
  ]);
  assertEquals(packEmojiIds({ altTexts: { "001": "a", "002": "b" } }), [
    "001",
    "002",
  ]);
  assertEquals(packEmojiIds(null), []);
  assertEquals(
    packEmojiIds({ orders: Array(500).fill("1").map((_, i) => String(i)) })
      .length,
    300,
  );
});

function fixture(dir: string, version?: unknown) {
  const urls: string[] = [];
  const observed: Json[][] = [];
  let account = "u-self";
  let owned: () => Promise<unknown> = () =>
    Promise.resolve({ productList: [{ id: A, name: "ムーン" }] });
  const service = createReactionEmojiPacks({
    accountMid: () => account,
    ownedProducts: () => owned(),
    directory: dir,
    observe: (messages) => observed.push(messages),
    fetch: ((input: string | URL | Request) => {
      const url = String(input);
      urls.push(url);
      if (url.endsWith("tab_on.png")) return Promise.resolve(new Response(png));
      if (url.endsWith("meta.json")) {
        return Promise.resolve(
          Response.json({ orders: ["001", "002"], version }),
        );
      }
      return Promise.resolve(new Response("", { status: 404 }));
    }) as typeof fetch,
  });
  return {
    service,
    urls,
    observed,
    account: (mid: string) => {
      account = mid;
    },
    owned: (fn: typeof owned) => {
      owned = fn;
    },
  };
}

Deno.test("所有パックの一覧とアイコンを取得し、同じアカウントでは再取得しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    const first = await f.service.packs();
    assertEquals(first.packs, [{
      id: A,
      name: "ムーン",
      iconPath: `${dir}/emoji-pack-${A}.png`,
    }]);
    assertEquals((await Deno.stat(first.packs[0].iconPath!)).size, png.length);
    const calls = f.urls.length;
    await f.service.packs();
    assertEquals(f.urls.length, calls);
    f.account("u-other");
    f.owned(() => Promise.resolve({ productList: [] }));
    assertEquals(await f.service.packs(), { packs: [] });
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("emoji pack versions reach the reaction picker", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir, "7");
    const expected = {
      emojis: [
        { key: `${A}_001`, resourceType: 1, version: 7 },
        { key: `${A}_002`, resourceType: 1, version: 7 },
      ],
    };
    assertEquals(await f.service.pack(A), expected);
    assertEquals(await f.service.pack(A), expected);
    assertEquals(f.urls.length, 1);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("pack version falls back to the owned product latestVersion", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.owned(() =>
      Promise.resolve({
        productList: [{ id: A, name: "ムーン", latestVersion: 3n }],
      })
    );
    await f.service.packs();
    assertEquals(await f.service.pack(A), {
      emojis: [
        { key: `${A}_001`, resourceType: 1, version: 3 },
        { key: `${A}_002`, resourceType: 1, version: 3 },
      ],
    });
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("取得に失敗すると明示的なエラーにする", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.owned(() => Promise.reject(new Error("boom")));
    await assertRejects(
      () => f.service.packs(),
      Error,
      "reaction_packs_unavailable",
    );
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("パックを開いた時だけ絵文字を取得し、画像の取得を依頼する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    assertEquals(await f.service.pack(A), {
      emojis: [
        { key: `${A}_001`, resourceType: 1, version: 1 },
        { key: `${A}_002`, resourceType: 1, version: 1 },
      ],
    });
    assertEquals(f.observed, [[{
      emojis: [{ key: `${A}_001` }, { key: `${A}_002` }],
    }]]);
    await f.service.pack(A);
    assertEquals(f.urls.filter((url) => url.endsWith("meta.json")).length, 1);
    await assertRejects(
      () => f.service.pack("../x"),
      Error,
      "reaction_invalid_pack",
    );
    await assertRejects(
      () => f.service.pack(B + "0"),
      Error,
      "reaction_invalid_pack",
    );
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

import { assert, assertEquals } from "@std/assert";
import { createEmojiService, lineEmojis } from "../src/emojis.ts";

const product = "5ac1bfd5040ab15980c9b435";
const key = `${product}_001`;
const image = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aK1sAAAAASUVORK5CYII=",
  ),
  (c) => c.charCodeAt(0),
);
async function waitFor(test: () => boolean) {
  const deadline = Date.now() + 2000;
  while (!test()) {
    assert(Date.now() < deadline, "絵文字の背景処理が完了しませんでした");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  await new Promise((resolve) => setTimeout(resolve, 10));
}

Deno.test("LINE絵文字のUTF-16範囲を変換し、本文・絵文字の組み合わせを維持する", () => {
  const text = "😀前(顔)後(顔)";
  const r = (S: unknown, E: unknown, sticonId = "001") => ({
    S,
    E,
    productId: product,
    sticonId,
  });
  assertEquals(
    lineEmojis(text, {
      REPLACE: JSON.stringify({ sticon: { resources: [r(8, 11), r(3, 6)] } }),
    }),
    [
      { start: 2, end: 5, key },
    ],
  );
  assertEquals(
    lineEmojis(text, {
      REPLACE: JSON.stringify({
        sticon: { resources: [r(3, 6), r(7, 10, "002")] },
      }),
    }),
    [
      { start: 2, end: 5, key },
      { start: 6, end: 9, key: `${product}_002` },
    ],
  );
});

Deno.test("壊れた・重複した・範囲外の絵文字情報を本文に適用しない", () => {
  assertEquals(lineEmojis("本文", { REPLACE: "{" }), []);
  assertEquals(lineEmojis("本文", {}), []);
  for (
    const [S, E, productId] of [[1, 2, product], [-1, 1, product], [
      0,
      999,
      product,
    ], [0, 1, "../../path"]]
  ) {
    // サロゲートの途中と任意URL・任意パスを拒否する。
    assertEquals(
      lineEmojis("😀本文", {
        REPLACE: JSON.stringify({
          sticon: { resources: [{ S, E, productId, sticonId: "001" }] },
        }),
      }),
      [],
    );
  }
});

Deno.test("絵文字画像を重複取得せず、保存済み画像を通信なしで再利用する", async () => {
  const directory = await Deno.makeTempDir();
  let downloads = 0;
  const ready: string[] = [];
  const service = createEmojiService({
    directory,
    emit: (_key, path) => ready.push(path),
    fetch: () => {
      downloads++;
      return Promise.resolve(new Response(image));
    },
  });
  const messages = [{ emojis: [{ key }, { key }] }];
  try {
    service.observe(messages);
    service.observe(messages);
    await waitFor(() => ready.length === 1);
    assertEquals(downloads, 1);
    assertEquals(await Deno.readFile(ready[0]), image);
    if (Deno.build.os !== "windows") {
      assertEquals((await Deno.stat(ready[0])).mode! & 0o777, 0o600);
    }
    service.observe(messages);
    await waitFor(() => ready.length === 2);
    assertEquals(downloads, 1);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("背景取得を4件に制限し、取得失敗の連続再試行を抑える", async () => {
  const directory = await Deno.makeTempDir();
  let active = 0, maximum = 0, completed = 0, failed = 0;
  const service = createEmojiService({
    directory,
    emit: () => {
      completed++;
    },
    fetch: async (url) => {
      if (String(url).includes("999.png")) {
        failed++;
        return new Response("error", { status: 404 });
      }
      active++;
      maximum = Math.max(maximum, active);
      await new Promise((resolve) => setTimeout(resolve, 15));
      active--;
      return new Response(image);
    },
  });
  try {
    service.observe([{
      emojis: Array.from({ length: 7 }, (_, i) => ({ key: `${product}_${i}` })),
    }]);
    await waitFor(() => completed === 7);
    assertEquals(maximum, 4);
    service.observe([{
      emojis: [{ key: `${product}_999` }, { key: "../../bad" }],
    }]);
    await waitFor(() => failed === 1);
    service.observe([{ emojis: [{ key: `${product}_999` }] }]);
    await new Promise((resolve) => setTimeout(resolve, 20));
    assertEquals(failed, 1);
    assertEquals(completed, 7);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

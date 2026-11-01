import { assertEquals } from "@std/assert";
import {
  appendLive,
  mergeDelta,
  newestMessage,
  reachesCursor,
} from "../src/message_delta.ts";

const msg = (n: number) => ({ id: String(n), createdTime: n * 1000 });
const range = (from: number, to: number) =>
  Array.from({ length: to - from + 1 }, (_, i) => msg(from + i));
const ids = (list: Record<string, unknown>[]) => list.map((m) => m.id);

Deno.test("newest message is found regardless of order", () => {
  assertEquals(newestMessage([msg(5), msg(9), msg(2)])?.id, "9");
  assertEquals(newestMessage([]), undefined);
});

Deno.test("history reaching the cursor needs no network fetch", () => {
  assertEquals(reachesCursor(range(1, 40), 40n), true);
  assertEquals(reachesCursor(range(1, 40), "40"), true);
  assertEquals(reachesCursor(range(1, 40), 41), false);
  assertEquals(reachesCursor(range(1, 40), undefined), false);
  assertEquals(reachesCursor([], 1), false);
});

Deno.test("a small batch that overlaps extends the stored history", () => {
  const merged = mergeDelta(range(1, 40), range(38, 45), 40);
  assertEquals(ids(merged!), ids(range(6, 45)));
});

Deno.test("a batch with a gap falls back to a full fetch", () => {
  assertEquals(mergeDelta(range(1, 40), range(60, 80), 40), null);
  assertEquals(mergeDelta(range(1, 40), [], 40), null);
  assertEquals(mergeDelta([], range(1, 5), 40), null);
});

Deno.test("live messages keep order and never duplicate", () => {
  const next = appendLive(range(1, 5), msg(6));
  assertEquals(ids(next), ids(range(1, 6)));
  assertEquals(ids(appendLive(next, msg(6))), ids(range(1, 6)));
  assertEquals(ids(appendLive(next, msg(3))), ids(range(1, 6)));
});

Deno.test("a live message keeps older history that has a gap before it", () => {
  assertEquals(ids(appendLive(range(1, 5), msg(20))), [
    "1",
    "2",
    "3",
    "4",
    "5",
    "20",
  ]);
});

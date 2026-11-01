import { assertEquals } from "@std/assert";
import {
  clampPageSize,
  mergeMessages,
  newerThan,
  olderThan,
} from "../src/message_history.ts";

const msg = (n: number) => ({ id: String(n), createdTime: n * 1000 });
const range = (from: number, to: number) =>
  Array.from({ length: to - from + 1 }, (_, i) => msg(from + i));
const ids = (list: Record<string, unknown>[]) => list.map((m) => m.id);

Deno.test("merge unions overlapping batches without duplicates", () => {
  const merged = mergeMessages(range(1, 40), range(30, 60));
  assertEquals(merged.length, 60);
  assertEquals(merged[0].id, "1");
  assertEquals(merged[59].id, "60");
});

Deno.test("merge drops stored messages when a newer batch leaves a hole", () => {
  const merged = mergeMessages(range(1, 40), range(100, 140));
  assertEquals(ids(merged), ids(range(100, 140)));
});

Deno.test("contiguous older pages never count as a hole", () => {
  const merged = mergeMessages(range(50, 90), range(1, 49), {
    contiguous: true,
  });
  assertEquals(merged.length, 90);
});

Deno.test("incoming copies replace stored ones with the same id", () => {
  const merged = mergeMessages(
    [{ id: "1", createdTime: 1000, text: "old" }],
    [{ id: "1", createdTime: 1000, text: "new" }],
  );
  assertEquals(merged[0].text, "new");
});

Deno.test("merge keeps only the newest messages up to the cap", () => {
  const merged = mergeMessages(range(1, 10), range(8, 20), { max: 5 });
  assertEquals(ids(merged), ["16", "17", "18", "19", "20"]);
});

Deno.test("olderThan returns the page right before the anchor", () => {
  const sorted = range(1, 100);
  assertEquals(ids(olderThan(sorted, "51", 0, 3)), ["48", "49", "50"]);
  assertEquals(ids(olderThan(sorted, "", 51000, 2)), ["49", "50"]);
  assertEquals(ids(olderThan(sorted, "2", 0, 10)), ["1"]);
  assertEquals(olderThan(sorted, "1", 0, 10), []);
});

Deno.test("newerThan pages forward and reports remaining history", () => {
  const sorted = range(1, 10);
  const first = newerThan(sorted, "3", 0, 4);
  assertEquals(ids(first.messages), ["4", "5", "6", "7"]);
  assertEquals(first.hasMore, true);
  const last = newerThan(sorted, "7", 0, 5);
  assertEquals(ids(last.messages), ["8", "9", "10"]);
  assertEquals(last.hasMore, false);
  assertEquals(newerThan(sorted, "", 5500, 2).messages[0].id, "6");
});

Deno.test("page size is clamped", () => {
  assertEquals(clampPageSize(undefined), 50);
  assertEquals(clampPageSize(-3), 50);
  assertEquals(clampPageSize(7.9), 7);
  assertEquals(clampPageSize(100000), 100);
});

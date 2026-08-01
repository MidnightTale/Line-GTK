import { assertEquals } from "@std/assert";
import { DAY_MS, FOREVER_MS, policyFor } from "../src/cache_policy.ts";

Deno.test("cache policies preserve retention guarantees", () => {
  assertEquals(policyFor("day").diskMsg, DAY_MS);
  assertEquals(policyFor("forever").diskChat, FOREVER_MS);
  assertEquals(policyFor("forever").animMiss, 0);
  assertEquals(policyFor("invalid"), policyFor("smart"));
});

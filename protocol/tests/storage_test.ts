import { assertEquals, assertRejects } from "@std/assert";
import {
  atomicWriteJson,
  atomicWriteTextFile,
  ensurePrivateDir,
} from "../src/storage.ts";

Deno.test("atomic JSON persistence leaves no temporary file", async () => {
  const directory = await Deno.makeTempDir({ prefix: "line-gtk-storage-" });
  try {
    await ensurePrivateDir(directory);
    const path = `${directory}/cache.json`;
    await atomicWriteJson(path, { generation: 1 });
    await atomicWriteJson(path, { generation: 2 });
    assertEquals(JSON.parse(await Deno.readTextFile(path)), { generation: 2 });
    assertEquals(
      (await Array.fromAsync(Deno.readDir(directory))).map((entry) =>
        entry.name
      ),
      ["cache.json"],
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("a stale writer cannot replace committed content and leaves no staging file", async () => {
  const directory = await Deno.makeTempDir();
  try {
    const path = `${directory}/index.json`;
    await atomicWriteTextFile(path, "previous");
    await assertRejects(
      () =>
        atomicWriteTextFile(path, "stale", 0o600, () => {
          throw new Error("session changed before commit");
        }),
      Error,
      "session changed before commit",
    );
    assertEquals(await Deno.readTextFile(path), "previous");
    assertEquals(
      (await Array.fromAsync(Deno.readDir(directory))).map((entry) =>
        entry.name
      ),
      ["index.json"],
    );
    await atomicWriteTextFile(path, "current", 0o600, () => {});
    assertEquals(await Deno.readTextFile(path), "current");
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

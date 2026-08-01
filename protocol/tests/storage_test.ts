import { assertEquals } from "@std/assert";
import { atomicWriteJson, ensurePrivateDir } from "../src/storage.ts";

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

import { assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import type { Client, TalkMessage } from "@evex/linejs";
import { createSharedAttachmentCache } from "../src/shared_media_cache.ts";

async function fixture() {
  const directory = await Deno.makeTempDir();
  let resolutions = 0;
  const runtime = {
    mediaDir: directory,
    accountMid: () => "uaccount",
    getClient: () => ({} as Client),
    resolve: () => {
      resolutions++;
      return Promise.resolve({
        getData: () => Promise.resolve(new Blob(["original document"])),
      } as unknown as TalkMessage);
    },
    existingFile: async (path: string) => {
      try {
        return (await Deno.stat(path)).isFile ? path : null;
      } catch {
        return null;
      }
    },
    existingFullImage: () => Promise.resolve(null),
    isImageBytes: () => false,
    isPreview: () => Promise.resolve(false),
    mediaDest: (id: string, extension = "jpg") =>
      join(directory, `${id}.${extension}`),
    thumbDest: (id: string) => join(directory, `${id}.thumb.jpg`),
  };
  return { directory, runtime, resolutions: () => resolutions };
}

Deno.test("shared attachment reuses the existing file without resolving or copying", async () => {
  const f = await fixture();
  try {
    const original = join(f.directory, "old-report.pdf");
    await Deno.writeTextFile(original, "already downloaded");
    const result = await createSharedAttachmentCache(f.runtime)("cchat", {
      message: {
        id: "9007199254740993",
        contentType: "FILE",
        fileName: "report.pdf",
        filePath: original,
      },
    }, () => {});
    assertEquals(result.path, original);
    assertEquals(result.fileName, "report.pdf");
    assertEquals(f.resolutions(), 0);
    assertEquals(Array.from(Deno.readDirSync(f.directory)).length, 1);
  } finally {
    await Deno.remove(f.directory, { recursive: true });
  }
});

Deno.test("shared attachment publishes an old original using the exact source", async () => {
  const f = await fixture();
  try {
    const result = await createSharedAttachmentCache(f.runtime)("cchat", {
      message: {
        id: "9007199254740993",
        contentType: "FILE",
        fileName: "../report.txt",
      },
    }, () => {});
    assertEquals(
      result.path,
      join(f.directory, "shared-uaccount-cchat-9007199254740993.txt"),
    );
    assertEquals(await Deno.readTextFile(result.path), "original document");
    assertEquals(result.contentType, "FILE");
    assertEquals(f.resolutions(), 1);
    if (Deno.build.os !== "windows") {
      assertEquals((await Deno.stat(result.path)).mode! & 0o777, 0o600);
    }
  } finally {
    await Deno.remove(f.directory, { recursive: true });
  }
});

Deno.test("shared attachment rejects stale download before publishing bytes", async () => {
  const f = await fixture();
  let active = true;
  try {
    f.runtime.resolve = () => {
      active = false;
      return Promise.resolve(
        {
          getData: () => Promise.resolve(new Blob(["private"])),
        } as unknown as TalkMessage,
      );
    };
    await assertRejects(
      () =>
        createSharedAttachmentCache(f.runtime)("cchat", {
          message: { id: "1", contentType: "FILE" },
        }, () => {
          if (!active) throw new Error("session changed");
        }),
      Error,
      "session changed",
    );
    assertEquals(Array.from(Deno.readDirSync(f.directory)).length, 0);
  } finally {
    await Deno.remove(f.directory, { recursive: true });
  }
});

Deno.test("shared attachment never stores a video preview as its original", async () => {
  const f = await fixture();
  try {
    await assertRejects(
      () =>
        createSharedAttachmentCache(f.runtime)("cchat", {
          message: { id: "1", contentType: "VIDEO" },
        }, () => {}),
      Error,
      "shared_media_original_unavailable",
    );
    assertEquals(Array.from(Deno.readDirSync(f.directory)).length, 0);
  } finally {
    await Deno.remove(f.directory, { recursive: true });
  }
});

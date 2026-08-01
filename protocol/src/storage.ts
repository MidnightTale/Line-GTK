import { dirname, join } from "@std/path";

let temporarySequence = 0;

/** Ensure application-owned directories are private on Unix-like systems. */
export async function ensurePrivateDir(path: string): Promise<void> {
  await Deno.mkdir(path, { recursive: true, mode: 0o700 });
  if (Deno.build.os !== "windows") {
    await Deno.chmod(path, 0o700);
  }
}

function temporaryPath(path: string): string {
  temporarySequence += 1;
  return join(
    dirname(path),
    `.${
      path.split(/[\\/]/).pop() ?? "data"
    }.${Deno.pid}.${temporarySequence}.tmp`,
  );
}

/**
 * Durably replace a text file. A crash can leave the previous complete file or
 * the new complete file, but never a partially-written JSON document.
 */
export async function atomicWriteTextFile(
  path: string,
  contents: string,
  mode = 0o600,
): Promise<void> {
  await ensurePrivateDir(dirname(path));
  const temporary = temporaryPath(path);
  try {
    const file = await Deno.open(temporary, {
      createNew: true,
      write: true,
      mode,
    });
    try {
      await file.write(new TextEncoder().encode(contents));
      await file.sync();
    } finally {
      file.close();
    }
    if (Deno.build.os !== "windows") {
      await Deno.chmod(temporary, mode);
    }
    await Deno.rename(temporary, path);
  } catch (error) {
    try {
      await Deno.remove(temporary);
    } catch { /* best effort */ }
    throw error;
  }
}

export async function atomicWriteJson(
  path: string,
  value: unknown,
): Promise<void> {
  await atomicWriteTextFile(path, JSON.stringify(value));
}

export async function writePrivateTextFile(
  path: string,
  contents: string,
): Promise<void> {
  await atomicWriteTextFile(path, contents, 0o600);
}

export async function readPrivateTextFile(path: string): Promise<string> {
  if (Deno.build.os !== "windows") {
    await Deno.chmod(path, 0o600);
  }
  return await Deno.readTextFile(path);
}

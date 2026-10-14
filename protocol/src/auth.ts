import { dirname, join } from "@std/path";
import { loginWithAuthToken } from "@evex/linejs";
import { FileStorage } from "@evex/linejs/storage";
import {
  ensurePrivateDir,
  readPrivateTextFile,
  writePrivateTextFile,
} from "./storage.ts";

export type LineDevice = "ANDROID" | "ANDROIDSECONDARY" | "DESKTOPWIN";

// セッションの機種と同じ設定で復元する。PCにはAndroidのバージョンを渡さない。
export function loginInit(device: LineDevice, androidVersion: string) {
  return device === "DESKTOPWIN"
    ? { device }
    : { device, version: androidVersion };
}

export async function createAuthStorage(path: string) {
  await ensurePrivateDir(dirname(path));
  const storage = new FileStorage(path);
  if (Deno.build.os !== "windows") await Deno.chmod(path, 0o600);
  return storage;
}

export async function restoreSession(
  token: string,
  device: LineDevice,
  storagePath: string,
  androidVersion: string,
  authenticate: typeof loginWithAuthToken = loginWithAuthToken,
) {
  return await authenticate(token, {
    ...loginInit(device, androidVersion),
    storage: await createAuthStorage(storagePath),
  });
}

export class AuthStore {
  readonly tokenPath: string;
  readonly devicePath: string;

  constructor(dataDir: string) {
    this.tokenPath = join(dataDir, "auth-token.txt");
    this.devicePath = join(dataDir, "auth-device.txt");
  }

  async save(token: string, device: LineDevice): Promise<void> {
    await writePrivateTextFile(this.tokenPath, token);
    await writePrivateTextFile(this.devicePath, device);
  }

  async loadToken(): Promise<string | null> {
    try {
      const token = (await readPrivateTextFile(this.tokenPath)).trim();
      return token || null;
    } catch {
      return null;
    }
  }

  async loadDevice(): Promise<LineDevice> {
    try {
      const device = (await readPrivateTextFile(this.devicePath)).trim();
      if (
        device === "ANDROID" || device === "ANDROIDSECONDARY" ||
        device === "DESKTOPWIN"
      ) {
        return device;
      }
    } catch { /* missing */ }
    return "DESKTOPWIN";
  }

  async clear(): Promise<void> {
    for (const path of [this.tokenPath, this.devicePath]) {
      try {
        await Deno.remove(path);
      } catch (error) {
        if (!(error instanceof Deno.errors.NotFound)) throw error;
      }
    }
  }
}

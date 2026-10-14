import { assert, assertEquals, assertRejects } from "@std/assert";
import { join } from "@std/path";
import type { Client } from "@evex/linejs";
import { AuthStore, restoreSession } from "../src/auth.ts";
import {
  authenticateEmail,
  emailLoginError,
  generateLoginPin,
  type PasswordAuthenticator,
} from "../src/email_login.ts";

Deno.test("メール認証はPC用セッションと認証番号を使い、パスワードを保存しない", async () => {
  const directory = await Deno.makeTempDir();
  const storagePath = join(directory, "storage.json");
  const password = "fixture-password-only";
  let receivedPin = "";
  try {
    const authenticate: PasswordAuthenticator = async (options, init) => {
      assertEquals(init.device, "DESKTOPWIN");
      assertEquals(init.version, undefined);
      assertEquals(options.email, "test@example.invalid");
      assertEquals(options.password, password);
      assert(/^\d{6}$/.test(options.pincode!));
      await options.onPincodeRequest(options.pincode!);
      assertEquals(receivedPin, options.pincode);
      await init.storage!.set(
        "cert:test@example.invalid",
        "fixture-certificate",
      );
      return { authToken: "fixture-token" } as Client;
    };
    const client = await authenticateEmail(
      "  test@example.invalid  ",
      password,
      storagePath,
      (pin) => {
        receivedPin = pin;
      },
      authenticate,
    );
    assertEquals(client.authToken, "fixture-token");
    const stored = await Deno.readTextFile(storagePath);
    assert(!stored.includes(password));
    assertEquals(
      JSON.parse(stored)["cert:test@example.invalid"],
      "fixture-certificate",
    );
    if (Deno.build.os !== "windows") {
      assertEquals((await Deno.stat(storagePath)).mode! & 0o777, 0o600);
    }
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("入力不足・不正なメールは通信前に拒否し、認証失敗に秘密情報を含めない", async () => {
  const directory = await Deno.makeTempDir();
  let called = false;
  const authenticate: PasswordAuthenticator = () => {
    called = true;
    throw new Error(
      "入力値 test@example.invalid fixture-password-only を含む例外",
    );
  };
  try {
    await assertRejects(
      () =>
        authenticateEmail(
          "",
          "p",
          join(directory, "storage.json"),
          () => {},
          authenticate,
        ),
      Error,
      "login_email_missing",
    );
    await assertRejects(
      () =>
        authenticateEmail(
          "not-an-email",
          "p",
          join(directory, "storage.json"),
          () => {},
          authenticate,
        ),
      Error,
      "login_invalid_email",
    );
    assertEquals(called, false);
    try {
      await authenticateEmail(
        "test@example.invalid",
        "fixture-password-only",
        join(directory, "storage.json"),
        () => {},
        authenticate,
      );
      throw new Error("認証エラーが返りませんでした");
    } catch (error) {
      assertEquals((error as Error).message, "login_email_failed");
    }
    assertEquals(
      emailLoginError(new Error("invalid password")),
      "login_invalid_password",
    );
    assertEquals(
      emailLoginError(new Error("timed out")),
      "login_email_timeout",
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("PC用トークンを削除せず同じ機種で復元し、QRのAndroidセッションも維持する", async () => {
  const directory = await Deno.makeTempDir();
  const store = new AuthStore(directory);
  try {
    for (
      const device of ["DESKTOPWIN", "ANDROIDSECONDARY", "ANDROID"] as const
    ) {
      await store.save("fixture-token", device);
      const restored = await restoreSession(
        (await store.loadToken())!,
        await store.loadDevice(),
        join(directory, "storage.json"),
        "26.6.2",
        (token, init) => {
          assertEquals(token, "fixture-token");
          assertEquals(init.device, device);
          assertEquals(
            init.version,
            device === "DESKTOPWIN" ? undefined : "26.6.2",
          );
          return Promise.resolve({ authToken: token } as Client);
        },
      );
      assertEquals(restored.authToken, "fixture-token");
      assertEquals(await store.loadDevice(), device);
      assertEquals(await store.loadToken(), "fixture-token");
    }
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

Deno.test("認証番号は毎回生成される6桁の値", () => {
  const pins = Array.from({ length: 32 }, generateLoginPin);
  assert(pins.every((pin) => /^\d{6}$/.test(pin)));
  assert(new Set(pins).size > 1);
});

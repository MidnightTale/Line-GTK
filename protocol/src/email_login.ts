import { loginWithPassword } from "@evex/linejs";
import { createAuthStorage } from "./auth.ts";

export type PasswordAuthenticator = typeof loginWithPassword;

export function generateLoginPin(): string {
  const random = new Uint32Array(1);
  do crypto.getRandomValues(random); while (random[0] >= 4_294_000_000);
  return String(random[0] % 1_000_000).padStart(6, "0");
}

export function emailLoginError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("invalid email")) return "login_invalid_email";
  if (message.includes("invalid password")) return "login_invalid_password";
  if (/timeout|timed out/i.test(message)) return "login_email_timeout";
  // ライブラリ・サーバーの例外には入力値が含まれる可能性があるため、そのまま送らない。
  return "login_email_failed";
}

export async function authenticateEmail(
  email: string,
  password: string,
  storagePath: string,
  onPin: (pin: string) => void,
  authenticate: PasswordAuthenticator = loginWithPassword,
) {
  email = email.trim();
  if (!email || !password) throw new Error("login_email_missing");
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    throw new Error("login_invalid_email");
  }
  const storage = await createAuthStorage(storagePath);
  try {
    // PCの追加デバイスとして認証し、スマホのメイン端末の登録を置き換えない。
    return await authenticate({
      email,
      password,
      pincode: generateLoginPin(),
      onPincodeRequest: onPin,
    }, { device: "DESKTOPWIN", storage });
  } catch (error) {
    throw new Error(emailLoginError(error));
  }
}

import {
  Client,
  type loginWithAuthToken,
  type loginWithPassword,
  type loginWithQR,
} from "@evex/linejs";
import { createKeyClient } from "./e2ee_keys.ts";
import { InternalError } from "@evex/linejs/base";

async function keyLogin(operation: () => Promise<Client>): Promise<Client> {
  try {
    return await operation();
  } catch (error) {
    if (
      error instanceof InternalError &&
      ["NoE2EEKey", "E2EEKeyMismatch"].includes(error.name)
    ) {
      throw new Error("login_keys_missing");
    }
    throw error;
  }
}

export const loginWithQrKeys: typeof loginWithQR = (options, init) =>
  keyLogin(async () => {
    const { base, bindAccount } = createKeyClient(init);
    base.on("qrcall", options.onReceiveQRUrl);
    base.on("pincall", options.onPincodeRequest);
    await base.loginProcess.withQrCode({});
    await base.loginProcess.ready();
    await bindAccount(true);
    return new Client(base);
  });

export const loginWithPasswordKeys: typeof loginWithPassword = (
  options,
  init,
) =>
  keyLogin(async () => {
    const { base, bindAccount } = createKeyClient(init);
    base.on("pincall", options.onPincodeRequest);
    await base.loginProcess.withPassword({
      email: options.email,
      password: options.password,
      pincode: options.pincode,
    });
    await base.loginProcess.ready();
    await bindAccount(true);
    return new Client(base);
  });

export const loginWithTokenKeys: typeof loginWithAuthToken = (
  token,
  init,
) =>
  keyLogin(async () => {
    const { base, bindAccount } = createKeyClient(init);
    await base.loginProcess.login({ authToken: token });
    await bindAccount();
    return new Client(base);
  });

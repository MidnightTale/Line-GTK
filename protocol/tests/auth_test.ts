import { assertEquals } from "@std/assert";
import { AuthStore } from "../src/auth.ts";

Deno.test("auth lifecycle uses private files", async () => {
  const directory = await Deno.makeTempDir({ prefix: "line-gtk-auth-" });
  try {
    const store = new AuthStore(directory);
    assertEquals(await store.loadToken(), null);
    await store.save("secret-token", "ANDROIDSECONDARY");
    assertEquals(await store.loadToken(), "secret-token");
    assertEquals(await store.loadDevice(), "ANDROIDSECONDARY");
    if (Deno.build.os !== "windows") {
      assertEquals((await Deno.stat(store.tokenPath)).mode! & 0o777, 0o600);
    }
    await store.clear();
    assertEquals(await store.loadToken(), null);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

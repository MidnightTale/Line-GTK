import { assertEquals, assertRejects, assertThrows } from "@std/assert";
import { Buffer } from "node:buffer";
import { createCipheriv, createHash, hkdfSync } from "node:crypto";
import vectors from "./fixtures/video-integrity.json" with { type: "json" };
import {
  decryptChunkedVideo,
  patchMediaDecryption,
} from "../src/media_crypto.ts";

type MediaCrypto = Parameters<typeof patchMediaDecryption>[0];

const key = Buffer.from(Array.from({ length: 32 }, (_, i) => i));
// HMACの正解値はPythonのhashlib/hmacとOpenSSLで生成。受信処理と計算を共有しない。
function encrypted(vector: typeof vectors[number]) {
  const plain = Buffer.from(
    Array.from({ length: vector.length }, (_, i) => (i * 17 + 3) % 256),
  );
  const keys = Buffer.from(
    hkdfSync("sha256", key, Buffer.alloc(0), "FileEncryption", 76),
  );
  const counter = Buffer.alloc(16);
  keys.copy(counter, 0, 64, 76);
  const cipher = createCipheriv("aes-256-ctr", keys.subarray(0, 32), counter);
  const ciphertext = Buffer.concat([cipher.update(plain), cipher.final()]);
  assertEquals(
    createHash("sha256").update(ciphertext).digest("hex"),
    vector.cipherSha256,
  );
  return {
    plain,
    data: Buffer.concat([ciphertext, Buffer.from(vector.tag, "hex")]),
  };
}
const integrityError = () =>
  Object.assign(
    new Error(
      "HMAC verification failed: ciphertext was tampered with or keyMaterial is wrong",
    ),
    { name: "E2EEMediaError" },
  );

Deno.test("独立ベクトル: 動画HMACを128KiBの境界前後と複数チャンクで検証して復号する", () => {
  for (const vector of vectors) {
    const { plain, data } = encrypted(vector);
    assertEquals(decryptChunkedVideo(data, key), plain);
    assertEquals(decryptChunkedVideo(data, key.toString("base64")), plain);
  }
});
Deno.test("改変した先頭・チャンク境界・末尾・認証タグと誤った鍵を拒否する", () => {
  const { data } = encrypted(vectors[5]);
  for (const offset of [0, 131071, 131072, data.length - 33, data.length - 1]) {
    const corrupt = Buffer.from(data);
    corrupt[offset] ^= 1;
    assertThrows(
      () => decryptChunkedVideo(corrupt, key),
      Error,
      "video_integrity_failed",
    );
  }
  for (const size of [0, 1, 31, 32]) {
    assertThrows(() => decryptChunkedVideo(data.subarray(0, size), key));
  }
  assertThrows(() => decryptChunkedVideo(data, Buffer.alloc(32, 9)));
  assertThrows(() => decryptChunkedVideo(data, "不正な鍵"));
});
Deno.test("従来の正常な復号は変更せず、動画方式のHMAC不一致だけを補完する", async () => {
  const { data, plain } = encrypted(vectors[5]);
  const error = integrityError();
  let calls = 0;
  const crypto: MediaCrypto = {
    decryptByKeyMaterial: () => {
      calls++;
      return Promise.reject(error);
    },
  };
  patchMediaDecryption(crypto);
  patchMediaDecryption(crypto);
  assertEquals(await crypto.decryptByKeyMaterial(data, key), plain);
  assertEquals(calls, 1);
  const standard: MediaCrypto = {
    decryptByKeyMaterial: () => Promise.resolve(Buffer.from("従来の画像")),
  };
  patchMediaDecryption(standard);
  assertEquals(
    await standard.decryptByKeyMaterial(Buffer.alloc(0), key),
    Buffer.from("従来の画像"),
  );
});
Deno.test("破損・誤鍵・通信エラーでは元のエラーを保持し平文を返さない", async () => {
  const { data } = encrypted(vectors[1]);
  const error = integrityError();
  const crypto: MediaCrypto = {
    decryptByKeyMaterial: () => Promise.reject(error),
  };
  patchMediaDecryption(crypto);
  assertEquals(
    await assertRejects(() =>
      crypto.decryptByKeyMaterial(data, Buffer.alloc(32, 9))
    ),
    error,
  );
  const corrupt = Buffer.from(data);
  corrupt[0] ^= 1;
  assertEquals(
    await assertRejects(() => crypto.decryptByKeyMaterial(corrupt, key)),
    error,
  );
  const networkError = new Error("HTTP 404");
  const network: MediaCrypto = {
    decryptByKeyMaterial: () => Promise.reject(networkError),
  };
  patchMediaDecryption(network);
  assertEquals(
    await assertRejects(() => network.decryptByKeyMaterial(data, key)),
    networkError,
  );
});

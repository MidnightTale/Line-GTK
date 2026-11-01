import { Buffer } from "node:buffer";
import {
  createDecipheriv,
  createHash,
  createHmac,
  hkdfSync,
  timingSafeEqual,
} from "node:crypto";

type MediaDecryptor = (
  data: Buffer,
  keyMaterial: Buffer | string,
) => Promise<Buffer>;
type MediaCrypto = { decryptByKeyMaterial: MediaDecryptor };
const patched = new WeakSet<MediaCrypto>();

// 動画の原本は128KiBごとのSHA-256を連結してHMACを計算する。
// 画像・音声・ファイル・動画のプレビューに使う従来の検証はそのまま優先する。
// 参考: https://github.com/beeper/line/blob/main/pkg/connector/media.go
export function decryptChunkedVideo(
  data: Buffer,
  keyMaterial: Buffer | string,
): Buffer {
  if (data.length <= 32) throw new Error("video_integrity_failed");
  const key = typeof keyMaterial === "string"
    ? Buffer.from(keyMaterial, "base64")
    : keyMaterial;
  if (key.length !== 32) throw new Error("video_integrity_failed");
  const keys = Buffer.from(
    hkdfSync("sha256", key, Buffer.alloc(0), "FileEncryption", 76),
  );
  const ciphertext = data.subarray(0, -32);
  const received = data.subarray(-32);
  const mac = createHmac("sha256", keys.subarray(32, 64));
  for (let offset = 0; offset < ciphertext.length; offset += 131072) {
    mac.update(
      createHash("sha256")
        .update(ciphertext.subarray(offset, offset + 131072)).digest(),
    );
  }
  if (!timingSafeEqual(received, mac.digest())) {
    throw new Error("video_integrity_failed");
  }
  // 正しいHMACを確認するまで平文を生成しない。
  const counter = Buffer.alloc(16);
  keys.copy(counter, 0, 64, 76);
  const decipher = createDecipheriv(
    "aes-256-ctr",
    keys.subarray(0, 32),
    counter,
  );
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

export function patchMediaDecryption(crypto: MediaCrypto): void {
  if (patched.has(crypto)) return;
  const original = crypto.decryptByKeyMaterial.bind(crypto);
  crypto.decryptByKeyMaterial = async (data, key) => {
    try {
      return await original(data, key);
    } catch (error) {
      // 通信失敗や鍵取得失敗は互換処理で隠さず、元のエラーを返す。
      if (
        !(error instanceof Error) || error.name !== "E2EEMediaError" ||
        !error.message.includes("HMAC verification failed")
      ) throw error;
      try {
        return decryptChunkedVideo(data, key);
      } catch {
        throw error;
      }
    }
  };
  patched.add(crypto);
}

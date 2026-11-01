import { assertEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { BaseClient, InternalError } from "@evex/linejs/base";
import { MemoryStorage } from "@evex/linejs/storage";
import { Buffer } from "node:buffer";
import nacl from "npm:tweetnacl@1.0.3";
import { createOutgoingMedia } from "../src/outgoing_media.ts";

Deno.test("sending a local transparent PNG uploads its original bytes as IMAGE", async () => {
  const directory = await Deno.makeTempDir();
  // A 1x1 RGBA PNG, including its alpha channel.
  const png = Uint8Array.from(
    atob(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNgAAIAAAUAAarVyFEAAAAASUVORK5CYII=",
    ),
    (c) => c.charCodeAt(0),
  );
  const source = `${directory}/sticker.png`;
  const uploads: { type: string; data: Blob; name: string }[] = [];
  const results: Record<string, unknown>[] = [];
  const errors: string[] = [];
  const client = {
    base: {
      obs: {
        uploadObjTalk: (
          _to: string,
          type: string,
          data: Blob,
          _unused: unknown,
          name: string,
        ) => {
          uploads.push({ type, data, name });
          return Promise.resolve({ objId: "sent-image" });
        },
      },
      talk: { getPreviousMessagesV2WithRequest: () => Promise.resolve([]) },
    },
  } as unknown as Client;
  const outgoing = outgoingFixture(directory, client, results, errors);
  try {
    await Deno.writeFile(source, png);
    await outgoing.sendMedia(1, "r-test-room", source, "image");
    assertEquals(errors, []);
    assertEquals(uploads.length, 1);
    assertEquals(uploads[0].type, "image");
    assertEquals(uploads[0].data.type, "image/png");
    assertEquals(new Uint8Array(await uploads[0].data.arrayBuffer()), png);
    const message = results[0].message as Record<string, unknown>;
    assertEquals(message.contentType, "IMAGE");
    assertEquals(await Deno.readFile(String(message.imagePath)), png);
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});

function outgoingFixture(
  directory: string,
  client: Client,
  results: Record<string, unknown>[],
  errors: string[],
  events: Record<string, unknown>[] = [],
) {
  return createOutgoingMedia({
    getClient: () => client,
    mediaDir: directory,
    mediaDest: (id, ext = "jpg") => `${directory}/${id}.${ext}`,
    fullDest: (id, ext = "jpg") => `${directory}/${id}.full.${ext}`,
    thumbDest: (id) => `${directory}/${id}.thumb.jpg`,
    existingFile: () => Promise.resolve(null),
    existingImage: () => Promise.resolve(null),
    isImageBytes: () => true,
    writeImageFile: async (path, bytes) => {
      await Deno.writeFile(path, bytes);
      return path;
    },
    writeFullImage: () => Promise.resolve(null),
    refetchMessageData: () => Promise.resolve(null),
    uiMediaPath: (_id, path) => Promise.resolve(path),
    isPreviewOrThumbImage: () => Promise.resolve(false),
    getBoxCursor: () => undefined,
    isMineFrom: () => true,
    normType: String,
    getCachedMessages: () => undefined,
    invalidateMessages() {},
    invalidateChats() {},
    sentMessagePayload: () => ({}),
    touchChatPreviewFromMessage() {},
    emitEvent: (_event, payload) => events.push(payload ?? {}),
    ok: (_id, result) => results.push(result as Record<string, unknown>),
    fail: (_id, error) => errors.push(error),
  });
}

type SendOptions = Parameters<BaseClient["talk"]["sendMessage"]>[0];

async function encryptedFixture(
  options: {
    gif?: boolean;
    sendError?: Error;
    rejectEverySend?: boolean;
    uploadErrorAt?: number;
  },
  check: (f: {
    sends: SendOptions[];
    uploads: { data: Blob; obsPath: string }[];
    results: Record<string, unknown>[];
    errors: string[];
    events: Record<string, unknown>[];
    groupReads: number;
    groupRegistrations: number;
    fallbackUploads: number;
    payload: (message: SendOptions) => Record<string, string>;
    decrypt: (blob: Blob, key: string) => Promise<Buffer>;
    png: Buffer;
  }) => Promise<void> | void,
) {
  const directory = await Deno.makeTempDir();
  try {
    const base = new BaseClient({
      device: "DESKTOPWIN",
      storage: new MemoryStorage(),
    });
    const mid = "u-sender";
    base.profile = { mid } as NonNullable<typeof base.profile>;
    const privateKey = Buffer.alloc(32, 1);
    const publicKey = Buffer.from(nacl.scalarMult.base(privateKey));
    await base.e2ee.saveE2EESelfKeyData({
      keyId: 10,
      privKey: privateKey.toString("base64"),
      pubKey: publicKey.toString("base64"),
    });
    const groupKeys = [Buffer.alloc(32, 2), Buffer.alloc(32, 3)];
    let groupReads = 0;
    let groupRegistrations = 0;
    base.talk.getChats = () =>
      Promise.resolve({
        chats: [{
          chatMid: "c-group",
          extra: { groupExtra: { memberMids: { [mid]: 0 } } },
        }],
      } as never);
    base.talk.getLastE2EEPublicKeys = () =>
      Promise.resolve({ [mid]: { keyId: 10, keyData: publicKey } } as never);
    base.e2ee.tryRegisterE2EEGroupKey = () => {
      groupRegistrations++;
      return Promise.resolve({} as never);
    };
    base.e2ee.getE2EELocalPublicKey = () => {
      groupReads++;
      const generation = Math.min(groupRegistrations, 1);
      return Promise.resolve({
        keyId: generation + 20,
        privKey: groupKeys[generation].toString("base64"),
      });
    };
    const sends: SendOptions[] = [];
    const uploads: { data: Blob; obsPath: string }[] = [];
    const results: Record<string, unknown>[] = [];
    const errors: string[] = [];
    const events: Record<string, unknown>[] = [];
    let fallbackUploads = 0;
    base.obs.uploadObjectForService = ({ data, obsPath }) => {
      uploads.push({ data, obsPath: obsPath ?? "" });
      if (options.uploadErrorAt === uploads.length) {
        return Promise.reject(new Error("upload_timeout"));
      }
      return Promise.resolve({
        objId: "uploaded",
        objHash: "",
        headers: new Headers(),
      });
    };
    base.obs.uploadMediaByE2EE = () => {
      fallbackUploads++;
      return Promise.reject(new Error("unexpected_fallback_upload"));
    };
    const send = base.talk.sendMessage.bind(base.talk);
    base.talk.sendMessage = (message) => {
      sends.push(message);
      return send(message);
    };
    let requests = 0;
    // Use the real TalkService so its implicit E2EE retry is exercised too.
    base.request.request = (() => {
      requests++;
      if (options.sendError && (options.rejectEverySend || requests === 1)) {
        return Promise.reject(options.sendError);
      }
      return Promise.resolve({ id: "sent-image", contentType: "IMAGE" });
    }) as typeof base.request.request;
    const png = Buffer.from(
      options.gif
        ? "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"
        : "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNgAAIAAAUAAarVyFEAAAAASUVORK5CYII=",
      "base64",
    );
    const source = `${directory}/sticker.${options.gif ? "gif" : "png"}`;
    await Deno.writeFile(source, png);
    const outgoing = outgoingFixture(
      directory,
      { base } as Client,
      results,
      errors,
      events,
    );
    await outgoing.sendMedia(
      1,
      "c-group",
      source,
      options.gif ? "gif" : "image",
    );
    await check({
      sends,
      uploads,
      results,
      errors,
      events,
      groupReads,
      groupRegistrations,
      fallbackUploads,
      png,
      payload: (message) => {
        const chunks = message.chunks as Buffer[];
        const generation = chunks[4].readInt32BE() - 20;
        return base.e2ee.decryptE2EEMessageV2(
          message.to,
          mid,
          chunks,
          groupKeys[generation],
          publicKey,
          2,
          1,
        );
      },
      decrypt: async (blob, key) =>
        await base.e2ee.decryptByKeyMaterial(
          Buffer.from(await blob.arrayBuffer()),
          key,
        ),
    });
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
}

Deno.test("group sticker PNG and preview decrypt with the key in the IMAGE envelope", () =>
  encryptedFixture({}, async (f) => {
    assertEquals(f.errors, []);
    assertEquals(f.results.length, 1);
    assertEquals(f.sends.length, 1);
    assertEquals(f.uploads.length, 2);
    assertEquals(f.sends[0].contentType, 1);
    assertEquals(f.sends[0].contentMetadata?.SID, "emi");
    assertEquals(f.sends[0].contentMetadata?.OID, "uploaded");
    const payload = f.payload(f.sends[0]);
    assertEquals(payload.fileName, "sticker.png");
    assertEquals(
      await f.decrypt(f.uploads[0].data, payload.keyMaterial),
      f.png,
    );
    const preview = await f.decrypt(f.uploads[1].data, payload.keyMaterial);
    assertEquals(preview.subarray(0, 8), f.png.subarray(0, 8));
  }));

for (const code of ["E2EE_RECREATE_GROUP_KEY", 99]) {
  Deno.test(`group key rejection ${code} reencrypts once without reuploading`, () =>
    encryptedFixture({
      sendError: new InternalError("TalkException", "rotate_group_key", {
        code,
      }),
    }, (f) => {
      assertEquals(f.errors, []);
      assertEquals(f.results.length, 1);
      assertEquals(f.sends.length, 2);
      assertEquals(f.groupReads, 2);
      assertEquals(f.groupRegistrations, 1);
      assertEquals(f.uploads.length, 2);
      assertEquals(f.fallbackUploads, 0);
      assertEquals((f.sends[0].chunks as Buffer[])[4].readInt32BE(), 20);
      assertEquals((f.sends[1].chunks as Buffer[])[4].readInt32BE(), 21);
      assertEquals(f.payload(f.sends[0]), f.payload(f.sends[1]));
    }));
}

Deno.test("repeated group key rejection stops after one refreshed envelope", () =>
  encryptedFixture({
    sendError: new InternalError("TalkException", "rotate_group_key", {
      code: "E2EE_RECREATE_GROUP_KEY",
    }),
    rejectEverySend: true,
  }, (f) => {
    assertEquals(f.results, []);
    assertEquals(f.errors, ["rotate_group_key"]);
    assertEquals(f.sends.length, 2);
    assertEquals(f.groupReads, 2);
    assertEquals(f.groupRegistrations, 1);
    assertEquals(f.fallbackUploads, 0);
  }));

for (
  const error of [
    new DOMException("send_timeout", "TimeoutError"),
    new InternalError("TalkException", "invalid_envelope", {
      code: "E2EE_INVALID_PROTOCOL",
    }),
  ]
) {
  Deno.test(`${error.message} does not resend a possibly accepted sticker`, () =>
    encryptedFixture({ sendError: error }, (f) => {
      assertEquals(f.results, []);
      assertEquals(f.errors, [error.message]);
      assertEquals(f.sends.length, 1);
      assertEquals(f.uploads.length, 2);
      assertEquals(f.fallbackUploads, 0);
      assertEquals(f.events.at(-1)?.done, true);
    }));
}

for (const uploadErrorAt of [1, 2]) {
  Deno.test(`timeout at media upload ${uploadErrorAt} never sends an incomplete sticker`, () =>
    encryptedFixture({ uploadErrorAt }, (f) => {
      assertEquals(f.results, []);
      assertEquals(f.errors, ["upload_timeout"]);
      assertEquals(f.sends, []);
      assertEquals(f.uploads.length, uploadErrorAt);
      assertEquals(f.fallbackUploads, 0);
      assertEquals(f.events.at(-1)?.done, true);
    }));
}

Deno.test("custom GIF sends its original animation bytes with IMAGE animation metadata", () =>
  encryptedFixture({ gif: true }, async (f) => {
    assertEquals(f.errors, []);
    assertEquals(f.sends.length, 1);
    const info = JSON.parse(f.sends[0].contentMetadata!.MEDIA_CONTENT_INFO);
    assertEquals(info.animated, true);
    assertEquals(info.extension, "gif");
    assertEquals(f.sends[0].contentType, 1);
    const payload = f.payload(f.sends[0]);
    assertEquals(payload.fileName, "sticker.gif");
    assertEquals(
      await f.decrypt(f.uploads[0].data, payload.keyMaterial),
      f.png,
    );
    const preview = await f.decrypt(f.uploads[1].data, payload.keyMaterial);
    // The native client shows the separately uploaded preview before fetching
    // the GIF. JPEG would permanently flatten its transparent background.
    assertEquals([...preview.subarray(0, 8)], [
      137,
      80,
      78,
      71,
      13,
      10,
      26,
      10,
    ]);
    const decoder = new Deno.Command("ffmpeg", {
      args: [
        "-v",
        "error",
        "-i",
        "pipe:0",
        "-vf",
        "scale=1:1",
        "-frames:v",
        "1",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "pipe:1",
      ],
      stdin: "piped",
      stdout: "piped",
      stderr: "piped",
    }).spawn();
    const writer = decoder.stdin.getWriter();
    await writer.write(preview);
    await writer.close();
    const decoded = await decoder.output();
    assertEquals(decoded.success, true);
    assertEquals(decoded.stdout.length, 4);
    assertEquals(decoded.stdout[3], 0);
  }));

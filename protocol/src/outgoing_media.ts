import type { Client } from "@evex/linejs";
import { join } from "@std/path";
import { Buffer } from "node:buffer";

type Json = Record<string, unknown>;

export type OutgoingMediaRuntime = {
  getClient: () => Client | null;
  mediaDir: string;
  mediaDest: (id: string, ext?: string) => string;
  fullDest: (id: string, ext?: string) => string;
  thumbDest: (id: string) => string;
  existingFile: (path: string) => Promise<string | null>;
  existingImage: (path: string) => Promise<string | null>;
  isImageBytes: (buf: Uint8Array) => boolean;
  writeImageFile: (dest: string, buf: Uint8Array) => Promise<string | null>;
  writeFullImage: (
    id: string,
    buf: Uint8Array,
    mimeOrExt?: string,
  ) => Promise<string | null>;
  refetchMessageData: (
    chatMid: string,
    messageId: string,
    opts?: { allowNonImage?: boolean },
  ) => Promise<{ buf: Uint8Array; mime: string } | null>;
  uiMediaPath: (id: string, path: string) => Promise<string | null>;
  isPreviewOrThumbImage: (path: string) => Promise<boolean>;
  getBoxCursor: (
    chatMid: string,
  ) =>
    | { messageId: bigint | number; deliveredTime: bigint | number }
    | undefined;
  isMineFrom: (from: unknown) => boolean;
  normType: (value: unknown) => string;
  getCachedMessages: (chatMid: string) => Json[] | undefined;
  invalidateMessages: (chatMid: string) => void;
  invalidateChats: () => void;
  sentMessagePayload: (
    sent:
      | { id?: unknown; createdTime?: unknown; contentType?: unknown }
      | null
      | undefined,
    chatMid: string,
    text: string,
  ) => Json;
  touchChatPreviewFromMessage: (message: Json) => void;
  emitEvent: (event: string, payload?: Json) => void;
  ok: (id: number | string | null, result?: unknown) => void;
  fail: (id: number | string | null, error: string) => void;
};

export function createOutgoingMedia(runtime: OutgoingMediaRuntime) {
  const {
    mediaDir,
    mediaDest,
    fullDest,
    thumbDest,
    existingFile,
    existingImage,
    isImageBytes,
    writeImageFile,
    writeFullImage,
    refetchMessageData,
    uiMediaPath,
    isPreviewOrThumbImage,
    getBoxCursor,
    isMineFrom,
    normType,
    getCachedMessages,
    invalidateMessages,
    invalidateChats,
    sentMessagePayload,
    touchChatPreviewFromMessage,
    emitEvent,
    ok,
    fail,
  } = runtime;

  type MediaOType = "image" | "gif" | "video" | "audio" | "file";

  function guessMime(name: string, oType: MediaOType): string {
    const lower = name.toLowerCase();
    if (lower.endsWith(".png")) return "image/png";
    if (lower.endsWith(".gif")) return "image/gif";
    if (lower.endsWith(".webp")) return "image/webp";
    if (lower.endsWith(".bmp")) return "image/bmp";
    if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
    if (lower.endsWith(".mp4")) return "video/mp4";
    if (lower.endsWith(".webm")) return "video/webm";
    if (lower.endsWith(".mov")) return "video/quicktime";
    if (lower.endsWith(".mkv")) return "video/x-matroska";
    if (lower.endsWith(".mp3")) return "audio/mpeg";
    if (lower.endsWith(".wav")) return "audio/wav";
    if (lower.endsWith(".ogg") || lower.endsWith(".oga")) return "audio/ogg";
    if (lower.endsWith(".m4a") || lower.endsWith(".aac")) return "audio/mp4";
    if (oType === "image" || oType === "gif") return "image/jpeg";
    if (oType === "video") return "video/mp4";
    if (oType === "audio") return "audio/mp4";
    return "application/octet-stream";
  }

  function normalizeMediaOType(raw: string, fileName: string): MediaOType {
    const t = raw.trim().toLowerCase();
    if (
      t === "image" || t === "gif" || t === "video" || t === "audio" ||
      t === "file"
    ) {
      if (t === "image" && fileName.toLowerCase().endsWith(".gif")) {
        return "gif";
      }
      return t;
    }
    const lower = fileName.toLowerCase();
    if (/\.(gif)$/.test(lower)) return "gif";
    if (/\.(png|jpe?g|webp|bmp|heic)$/.test(lower)) return "image";
    if (/\.(mp4|webm|mov|mkv|m4v)$/.test(lower)) return "video";
    if (/\.(m4a|mp3|wav|ogg|oga|aac|flac)$/.test(lower)) return "audio";
    return "file";
  }

  function mediaContentType(oType: MediaOType): string {
    if (oType === "gif") return "IMAGE";
    return oType.toUpperCase();
  }

  function mediaPreviewText(oType: MediaOType, fileName: string): string {
    if (oType === "audio") return "Voice message";
    if (oType === "image" || oType === "gif") return "Photo";
    if (oType === "video") return "Video";
    return fileName || "File";
  }

  function supportsE2EEMedia(mid: string): boolean {
    // linejs uploadMediaByE2EE only accepts user (u) and group (c) mids.
    return mid.startsWith("u") || mid.startsWith("c");
  }

  async function makeImagePreviewBlob(filePath: string): Promise<Blob | null> {
    try {
      const dest = join(mediaDir, `preview-${Date.now()}.jpg`);
      const p = new Deno.Command("ffmpeg", {
        args: [
          "-y",
          "-v",
          "error",
          "-i",
          filePath,
          "-vf",
          "scale=640:640:force_original_aspect_ratio=decrease",
          "-frames:v",
          "1",
          "-q:v",
          "4",
          dest,
        ],
        stdout: "null",
        stderr: "null",
      });
      const { success } = await p.output();
      if (!success) return null;
      const buf = await Deno.readFile(dest);
      try {
        await Deno.remove(dest);
      } catch { /* ignore */ }
      return new Blob([buf], { type: "image/jpeg" });
    } catch {
      return null;
    }
  }

  async function extractVideoThumb(
    filePath: string,
    messageId: string,
  ): Promise<string | null> {
    const dest = thumbDest(messageId);
    try {
      const p = new Deno.Command("ffmpeg", {
        args: [
          "-y",
          "-i",
          filePath,
          "-ss",
          "0.2",
          "-vframes",
          "1",
          "-q:v",
          "3",
          dest,
        ],
        stdout: "null",
        stderr: "null",
      });
      const { success } = await p.output();
      if (!success) return null;
      return await existingImage(dest);
    } catch {
      return null;
    }
  }

  /** Turn decrypted video (or preview image) bytes into a UI thumbnail path. */
  async function materializeVideoPreview(
    messageId: string,
    buf: Uint8Array,
  ): Promise<string | null> {
    if (buf.length < 32) return null;
    if (isImageBytes(buf)) {
      return await writeImageFile(thumbDest(messageId), buf);
    }
    const vdest = mediaDest(messageId, "mp4");
    try {
      await Deno.writeFile(vdest, buf);
    } catch (e) {
      console.error("[materializeVideoPreview write]", messageId, e);
      return null;
    }
    return await extractVideoThumb(vdest, messageId);
  }

  async function cacheOutgoingMedia(
    messageId: string,
    filePath: string,
    oType: MediaOType,
  ): Promise<
    {
      imagePath: string | null;
      audioPath: string | null;
      filePath: string | null;
    }
  > {
    try {
      const data = await Deno.readFile(filePath);
      if (oType === "audio") {
        const dest = mediaDest(messageId, "m4a");
        await Deno.writeFile(dest, data);
        return { imagePath: null, audioPath: dest, filePath: dest };
      }
      if (oType === "image" || oType === "gif") {
        const lower = filePath.toLowerCase();
        const ext = lower.endsWith(".png")
          ? "png"
          : lower.endsWith(".gif")
          ? "gif"
          : lower.endsWith(".webp")
          ? "webp"
          : "jpg";
        const dest = fullDest(messageId, ext);
        await Deno.writeFile(dest, data);
        const ui = await uiMediaPath(messageId, dest);
        return { imagePath: ui || dest, audioPath: null, filePath: dest };
      }
      if (oType === "video") {
        const thumb = await extractVideoThumb(filePath, messageId);
        // Keep original video beside the thumb for later open/play.
        const vdest = mediaDest(messageId, "mp4");
        await Deno.writeFile(vdest, data);
        return { imagePath: thumb, audioPath: null, filePath: vdest };
      }
      // Generic file: keep a copy for re-open.
      const name = filePath.split("/").pop() || "file.bin";
      const dest = join(mediaDir, `${messageId}.${name}`);
      await Deno.writeFile(dest, data);
      return { imagePath: null, audioPath: null, filePath: dest };
    } catch (e) {
      console.error("[cacheOutgoingMedia]", messageId, e);
      return {
        imagePath: oType === "image" || oType === "gif" ? filePath : null,
        audioPath: oType === "audio" ? filePath : null,
        filePath,
      };
    }
  }

  /**
   * E2EE OBS media send aligned with linejs `obs.uploadMediaByE2EE`, plus DURATION
   * for audio/video so official LINE clients render the bubble correctly.
   */
  async function sendE2EEMedia(opts: {
    to: string;
    oType: MediaOType;
    data: Blob;
    filename: string;
    durationMs?: number;
    preview?: Blob;
    onProgress?: (progress: number, label: string) => void;
  }): Promise<{ id?: unknown; createdTime?: unknown; contentType?: unknown }> {
    const client = runtime.getClient();
    if (!client) throw new Error("not_logged_in");
    if (!supportsE2EEMedia(opts.to)) {
      throw new Error("e2ee_media_mid_unsupported");
    }
    const report = opts.onProgress ?? (() => {});

    const oType = opts.oType;
    const typeSet: Record<MediaOType, [string, number]> = {
      image: ["emi", 1],
      video: ["emv", 2],
      audio: ["ema", 3],
      file: ["emf", 14],
      gif: ["emi", 1],
    };
    const [obsNamespace, contentTypeNum] = typeSet[oType];
    const ext = (opts.filename.split(".").pop() || "bin").toLowerCase();
    const params: Record<string, string> = { type: "file" };
    if (oType === "gif") params.cat = "original";

    const e2ee = client.base.e2ee as unknown as {
      encryptByKeyMaterial: (
        data: Buffer,
        key?: Buffer,
      ) => Promise<{ keyMaterial: string; encryptedData: Buffer }>;
      encryptE2EEMessage: (
        to: string,
        data: { keyMaterial: string; fileName: string },
        contentType: number,
      ) => Promise<string[] | Buffer[]>;
    };
    const obs = client.base.obs as unknown as {
      uploadObjectForService: (options: Record<string, unknown>) => Promise<{
        objId: string;
        objHash: string;
        headers: Headers;
      }>;
    };

    report(0.12, "Encrypting…");
    const plain = Buffer.from(await opts.data.arrayBuffer());
    const { keyMaterial, encryptedData } = await e2ee.encryptByKeyMaterial(
      plain,
    );
    // deno-lint-ignore no-explicit-any
    const edata = new Blob([encryptedData as any]);
    const tempId = "reqid-" + crypto.randomUUID();
    report(0.35, "Uploading…");
    const { objId } = await obs.uploadObjectForService({
      data: edata,
      oType: "file",
      obsPath: `talk/${obsNamespace}/${tempId}`,
      params,
    });

    if (oType === "image" || oType === "gif" || oType === "video") {
      report(0.55, "Uploading preview…");
      let previewEdata: Blob;
      if (opts.preview) {
        const enc = await e2ee.encryptByKeyMaterial(
          Buffer.from(await opts.preview.arrayBuffer()),
          Buffer.from(keyMaterial, "base64"),
        );
        // deno-lint-ignore no-explicit-any
        previewEdata = new Blob([enc.encryptedData as any]);
      } else {
        previewEdata = edata;
      }
      const { objId: objId2 } = await obs.uploadObjectForService({
        data: previewEdata,
        oType: "file",
        obsPath: `talk/${obsNamespace}/${objId}__ud-preview`,
        params,
      });
      if (objId !== objId2) {
        throw new Error("obs_preview_oid_mismatch");
      }
    }

    report(0.78, "Sending…");
    const chunks = await e2ee.encryptE2EEMessage(
      opts.to,
      { keyMaterial, fileName: opts.filename || `line.${ext}` },
      contentTypeNum,
    );

    const meta: Record<string, string> = {
      SID: obsNamespace,
      OID: objId,
      FILE_SIZE: String(edata.size),
      e2eeVersion: "2",
    };
    if (
      (oType === "audio" || oType === "video") &&
      opts.durationMs != null &&
      Number.isFinite(opts.durationMs)
    ) {
      meta.DURATION = String(Math.max(1, Math.round(opts.durationMs)));
    }
    if (oType === "file") {
      meta.FILE_NAME = opts.filename;
    }
    if (oType === "image" || oType === "gif" || oType === "video") {
      meta.MEDIA_CONTENT_INFO = JSON.stringify({
        category: "original",
        fileSize: edata.size,
        extension: ext,
        animated: oType === "gif",
      });
    }

    report(0.92, "Finishing…");
    return await client.base.talk.sendMessage({
      to: opts.to,
      chunks,
      // deno-lint-ignore no-explicit-any
      contentType: contentTypeNum as any,
      contentMetadata: meta,
    }) as { id?: unknown; createdTime?: unknown; contentType?: unknown };
  }

  async function doSendMedia(
    id: number | string | null,
    chatMid: string,
    filePath: string,
    oTypeRaw?: string,
    durationMs?: number,
  ) {
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    if (!chatMid || !filePath) {
      fail(id, "chatMid and filePath required");
      return;
    }
    const report = (progress: number, label: string) => {
      emitEvent("upload_progress", {
        chatMid,
        progress,
        label,
        done: false,
      });
    };
    try {
      report(0.02, "Reading file…");
      const data = await Deno.readFile(filePath);
      if (data.length < 32) {
        fail(id, "file_too_small");
        emitEvent("upload_progress", {
          chatMid,
          progress: 0,
          label: "",
          done: true,
        });
        return;
      }
      const name = filePath.split("/").pop() || "file.bin";
      const oType = normalizeMediaOType(oTypeRaw ?? "auto", name);
      if (oType === "audio" && data.length < 1024) {
        fail(id, "audio_file_too_small");
        emitEvent("upload_progress", {
          chatMid,
          progress: 0,
          label: "",
          done: true,
        });
        return;
      }
      const mime = guessMime(name, oType);
      const blob = new Blob([data], { type: mime });
      const talkType: MediaOType = oType === "gif" ? "image" : oType;
      const dur = durationMs != null && Number.isFinite(durationMs)
        ? Math.max(1, Math.round(durationMs))
        : (oType === "audio" || oType === "video" ? 1919 : undefined);

      report(0.08, "Preparing…");
      let preview: Blob | undefined;
      if (oType === "image" || oType === "gif") {
        preview = (await makeImagePreviewBlob(filePath)) ?? undefined;
      } else if (oType === "video") {
        const thumbPath = await extractVideoThumb(
          filePath,
          `out-${Date.now()}`,
        );
        if (thumbPath) {
          preview = new Blob([await Deno.readFile(thumbPath)], {
            type: "image/jpeg",
          });
        }
      }

      let sent:
        | { id?: unknown; createdTime?: unknown; contentType?: unknown }
        | null = null;
      let lastErr: unknown = null;

      if (supportsE2EEMedia(chatMid)) {
        try {
          sent = await sendE2EEMedia({
            to: chatMid,
            oType,
            data: blob,
            filename: name,
            durationMs: dur,
            preview,
            onProgress: report,
          });
        } catch (e1) {
          lastErr = e1;
          console.error("[send_media e2ee]", e1);
          report(0.4, "Retrying upload…");
          try {
            sent = await client.base.obs.uploadMediaByE2EE({
              to: chatMid,
              oType,
              data: blob,
              filename: name,
              preview,
            }) as typeof sent;
            report(0.9, "Finishing…");
          } catch (e2) {
            lastErr = e2;
            console.error("[send_media uploadMediaByE2EE]", e2);
          }
        }
        if (!sent) {
          emitEvent("upload_progress", {
            chatMid,
            progress: 0,
            label: "",
            done: true,
          });
          fail(
            id,
            lastErr instanceof Error
              ? lastErr.message
              : String(lastErr ?? "e2ee_media_failed"),
          );
          return;
        }
      } else {
        try {
          report(0.4, "Uploading…");
          const up = await client.base.obs.uploadObjTalk(
            chatMid,
            talkType,
            blob,
            undefined,
            name,
            dur,
          );
          report(0.85, "Finishing…");
          let realId: string | null = null;
          try {
            const recent = await client.base.talk
              .getPreviousMessagesV2WithRequest({
                request: {
                  messageBoxId: chatMid,
                  endMessageId: getBoxCursor(chatMid)
                    ? {
                      messageId: getBoxCursor(chatMid)!.messageId,
                      deliveredTime: getBoxCursor(chatMid)!.deliveredTime,
                    }
                    : undefined,
                  messagesCount: 20,
                },
              }).catch(() => [] as unknown[]);
            const mine = (recent as Array<
              { id?: unknown; from?: string; contentType?: unknown }
            >)
              .filter((m) => isMineFrom(m.from))
              .reverse();
            const wantCt = mediaContentType(oType);
            const hit = mine.find((m) => normType(m.contentType) === wantCt);
            if (hit?.id != null) realId = String(hit.id);
          } catch { /* ignore */ }
          sent = {
            id: realId ?? (up as { objId?: string }).objId ?? Date.now(),
            createdTime: Date.now(),
            contentType: mediaContentType(oType),
          };
        } catch (e3) {
          emitEvent("upload_progress", {
            chatMid,
            progress: 0,
            label: "",
            done: true,
          });
          fail(
            id,
            e3 instanceof Error ? e3.message : String(e3),
          );
          return;
        }
      }

      invalidateMessages(chatMid);
      invalidateChats();
      const ct = mediaContentType(oType);
      const messageId = String(sent?.id ?? Date.now());
      report(0.96, "Caching…");
      const cached = await cacheOutgoingMedia(messageId, filePath, oType);
      const message = {
        ...sentMessagePayload(sent, chatMid, mediaPreviewText(oType, name)),
        id: messageId,
        contentType: ct,
        imagePath: cached.imagePath,
        audioPath: cached.audioPath,
        fileName: name,
        filePath: cached.filePath || filePath,
        durationMs: dur ?? null,
        needsMedia: !cached.imagePath &&
          (oType === "image" || oType === "gif" || oType === "video"),
      };
      touchChatPreviewFromMessage(message);
      emitEvent("upload_progress", {
        chatMid,
        progress: 1,
        label: "Sent",
        done: true,
      });
      ok(id, { message });
    } catch (e) {
      emitEvent("upload_progress", {
        chatMid,
        progress: 0,
        label: "",
        done: true,
      });
      fail(id, e instanceof Error ? e.message : String(e));
    }
  }

  async function doSendAudio(
    id: number | string | null,
    chatMid: string,
    filePath: string,
    durationMs?: number,
  ) {
    await doSendMedia(id, chatMid, filePath, "audio", durationMs);
  }

  function sniffMediaExt(
    buf: Uint8Array,
    hintCt: string,
    fileName?: string | null,
  ): string {
    if (isImageBytes(buf)) {
      if (buf[0] === 0x89) return "png";
      if (buf[0] === 0x47) return "gif";
      if (
        buf.length > 11 && buf[0] === 0x52 && buf[8] === 0x57 && buf[9] === 0x45
      ) {
        return "webp";
      }
      return "jpg";
    }
    if (
      buf.length > 12 && buf[4] === 0x66 && buf[5] === 0x74 &&
      buf[6] === 0x79 &&
      buf[7] === 0x70
    ) {
      const brand = new TextDecoder().decode(buf.slice(8, 12));
      if (
        hintCt === "AUDIO" || brand.startsWith("M4A") || brand.includes("mp4a")
      ) {
        return "m4a";
      }
      return "mp4";
    }
    if (fileName && fileName.includes(".")) {
      const ext = fileName.split(".").pop()!.toLowerCase().replace(
        /[^a-z0-9]/g,
        "",
      );
      if (ext) return ext.slice(0, 8);
    }
    if (hintCt === "AUDIO") return "m4a";
    if (hintCt === "VIDEO") return "mp4";
    if (hintCt === "IMAGE") return "jpg";
    return "bin";
  }

  /**
   * Download full media bytes for a message (E2EE-aware) into the local cache.
   * Used for save-as / full-view video / file open.
   */
  async function doDownloadMedia(
    id: number | string | null,
    chatMid: string,
    messageId: string,
  ) {
    const client = runtime.getClient();
    if (!client) {
      fail(id, "not_logged_in");
      return;
    }
    if (!chatMid || !messageId) {
      fail(id, "chatMid and messageId required");
      return;
    }
    try {
      let hintCt = "FILE";
      let fileName: string | null = null;
      try {
        const cached = getCachedMessages(chatMid)?.find((m) =>
          String(m.id) === messageId
        );
        if (cached) {
          hintCt = normType(cached.contentType);
          if (typeof cached.fileName === "string") fileName = cached.fileName;
          else if (hintCt === "FILE" && typeof cached.text === "string") {
            fileName = cached.text;
          }
        }
      } catch { /* ignore */ }

      // Reuse local full media when already cached.
      const localCandidates = [
        fullDest(messageId, "jpg"),
        fullDest(messageId, "png"),
        fullDest(messageId, "webp"),
        fullDest(messageId, "gif"),
        mediaDest(messageId, "mp4"),
        mediaDest(messageId, "m4a"),
        mediaDest(messageId, "mp3"),
        mediaDest(messageId, "jpg"),
        mediaDest(messageId, "png"),
        mediaDest(messageId, "gif"),
        mediaDest(messageId, "webp"),
        mediaDest(messageId, "bin"),
      ];
      if (fileName) {
        localCandidates.unshift(join(mediaDir, `${messageId}.${fileName}`));
      }
      for (const p of localCandidates) {
        const hit = await existingFile(p);
        if (!hit) continue;
        // Never return UI thumbnails as the download/viewer payload.
        if (hit.includes(".thumb.")) continue;
        try {
          const st = await Deno.stat(hit);
          if (
            hintCt === "VIDEO" && /\.(jpe?g|png|webp)$/i.test(hit) &&
            st.size < 200_000
          ) {
            continue;
          }
          if (hintCt === "IMAGE") {
            // Skip polluted preview files that were cached into mediaDest.
            if (await isPreviewOrThumbImage(hit)) continue;
          }
          if (st.size >= 256) {
            ok(id, {
              path: hit,
              fileName: fileName || hit.split("/").pop() || messageId,
              contentType: hintCt,
              bytes: st.size,
              cached: true,
            });
            return;
          }
        } catch { /* try next */ }
      }

      const got = await refetchMessageData(chatMid, messageId, {
        allowNonImage: true,
      });
      if (!got || got.buf.length < 32) {
        fail(id, "download_failed");
        return;
      }
      let dest: string;
      if (hintCt === "IMAGE" && isImageBytes(got.buf)) {
        const written = await writeFullImage(messageId, got.buf, got.mime);
        if (!written) {
          fail(id, "download_failed");
          return;
        }
        dest = written;
      } else {
        const ext = sniffMediaExt(got.buf, hintCt, fileName);
        dest = join(
          mediaDir,
          fileName && fileName.includes(".")
            ? `${messageId}.${fileName.replace(/[\/\\]/g, "_")}`
            : `${messageId}.${ext}`,
        );
        await Deno.writeFile(dest, got.buf);
        if (hintCt === "VIDEO" && ext === "mp4") {
          await extractVideoThumb(dest, messageId);
        }
      }
      ok(id, {
        path: dest,
        fileName: fileName || dest.split("/").pop() || messageId,
        contentType: hintCt,
        bytes: got.buf.length,
        cached: false,
      });
    } catch (e) {
      fail(id, e instanceof Error ? e.message : String(e));
    }
  }

  return {
    sendMedia: doSendMedia,
    sendAudio: doSendAudio,
    downloadMedia: doDownloadMedia,
    materializeVideoPreview,
  };
}

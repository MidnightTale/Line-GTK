/** Incremental Annex-B parser for the raw H.264 stream produced by wf-recorder. */
export class AnnexBParser {
  #pending = new Uint8Array(0);

  push(chunk: Uint8Array): Uint8Array[] {
    if (!chunk.length) return [];
    const merged = new Uint8Array(this.#pending.length + chunk.length);
    merged.set(this.#pending, 0);
    merged.set(chunk, this.#pending.length);

    const starts = findStartCodes(merged);
    if (starts.length < 2) {
      this.#pending = merged;
      return [];
    }

    const out: Uint8Array[] = [];
    for (let i = 0; i + 1 < starts.length; i++) {
      const [start, prefix] = starts[i]!;
      const [end] = starts[i + 1]!;
      const nal = trimTrailingZeros(merged.subarray(start + prefix, end));
      if (nal.length) out.push(nal.slice());
    }
    this.#pending = merged.subarray(starts.at(-1)![0]).slice();
    return out;
  }

  flush(): Uint8Array[] {
    const starts = findStartCodes(this.#pending);
    if (!starts.length) {
      this.#pending = new Uint8Array(0);
      return [];
    }
    const [start, prefix] = starts.at(-1)!;
    const nal = trimTrailingZeros(this.#pending.subarray(start + prefix));
    this.#pending = new Uint8Array(0);
    return nal.length ? [nal.slice()] : [];
  }
}

/** Groups the encoder's NAL units into access units (video frames).
 * wf-recorder/x264 is started with AUD enabled and one slice per frame. The
 * VCL fallback also keeps the parser useful when AUD is missing.
 */
export class H264AccessUnitAssembler {
  #pending: Uint8Array[] = [];
  #hasVcl = false;

  push(nal: Uint8Array): Uint8Array[][] {
    if (!nal.length) return [];
    const out: Uint8Array[][] = [];
    const kind = nal[0]! & 0x1f;
    const isVcl = kind === 1 || kind === 5;
    const startsNextUnit = kind === 9 ||
      (this.#hasVcl && (isVcl || kind === 6 || kind === 7 || kind === 8));

    if (startsNextUnit && this.#pending.length) {
      out.push(this.#take());
    }
    this.#pending.push(nal);
    if (isVcl) this.#hasVcl = true;
    return out;
  }

  flush(): Uint8Array[][] {
    return this.#pending.length ? [this.#take()] : [];
  }

  #take(): Uint8Array[] {
    const current = this.#pending;
    this.#pending = [];
    this.#hasVcl = false;
    return current;
  }
}

export type H264RtpPayload = {
  payload: Uint8Array;
  endOfFrame: boolean;
};

/** Packetize one H.264 access unit using RFC 6184 single-NAL/FU-A payloads. */
export function packetizeH264AccessUnit(
  nals: Uint8Array[],
  maxPayload = 1100,
): H264RtpPayload[] {
  const packets = nals.flatMap((nal) => packetizeH264Nal(nal, maxPayload));
  return packets.map((payload, index) => ({
    payload,
    endOfFrame: index === packets.length - 1,
  }));
}

export function packetizeH264Nal(
  nal: Uint8Array,
  maxPayload = 1100,
): Uint8Array[] {
  if (!nal.length) return [];
  if (maxPayload < 3) throw new Error("H.264 RTP payload limit is too small");
  if (nal.length <= maxPayload) return [nal.slice()];

  const header = nal[0]!;
  const fuIndicator = (header & 0xe0) | 28;
  const nalType = header & 0x1f;
  const chunkSize = maxPayload - 2;
  const out: Uint8Array[] = [];
  for (let offset = 1; offset < nal.length; offset += chunkSize) {
    const end = Math.min(nal.length, offset + chunkSize);
    const payload = new Uint8Array(2 + end - offset);
    payload[0] = fuIndicator;
    payload[1] = nalType;
    if (offset === 1) payload[1] |= 0x80;
    if (end === nal.length) payload[1] |= 0x40;
    payload.set(nal.subarray(offset, end), 2);
    out.push(payload);
  }
  return out;
}

function findStartCodes(bytes: Uint8Array): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (let i = 0; i + 2 < bytes.length; i++) {
    if (bytes[i] !== 0 || bytes[i + 1] !== 0) continue;
    if (bytes[i + 2] === 1) {
      out.push([i, 3]);
      i += 2;
    } else if (
      i + 3 < bytes.length && bytes[i + 2] === 0 && bytes[i + 3] === 1
    ) {
      out.push([i, 4]);
      i += 3;
    }
  }
  return out;
}

function trimTrailingZeros(bytes: Uint8Array): Uint8Array {
  let end = bytes.length;
  while (end > 0 && bytes[end - 1] === 0) end--;
  return bytes.subarray(0, end);
}

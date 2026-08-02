import { assertEquals } from "@std/assert";
import {
  AnnexBParser,
  H264AccessUnitAssembler,
  packetizeH264AccessUnit,
  packetizeH264Nal,
} from "../src/h264.ts";

Deno.test("AnnexBParser handles start codes split across chunks", () => {
  const parser = new AnnexBParser();
  assertEquals(parser.push(new Uint8Array([0, 0, 0])), []);
  assertEquals(parser.push(new Uint8Array([1, 0x67, 1, 2, 0, 0])), []);
  assertEquals(parser.push(new Uint8Array([1, 0x68, 3])), [
    new Uint8Array([0x67, 1, 2]),
  ]);
  assertEquals(parser.flush(), [new Uint8Array([0x68, 3])]);
});

Deno.test("packetizeH264Nal emits RFC 6184 FU-A fragments", () => {
  const nal = new Uint8Array([0x65, 1, 2, 3, 4, 5, 6]);
  const packets = packetizeH264Nal(nal, 5);
  assertEquals(packets, [
    new Uint8Array([0x7c, 0x85, 1, 2, 3]),
    new Uint8Array([0x7c, 0x45, 4, 5, 6]),
  ]);
});

Deno.test("access units mark only their final RTP payload", () => {
  const assembler = new H264AccessUnitAssembler();
  assertEquals(assembler.push(new Uint8Array([0x09, 0x10])), []);
  assertEquals(assembler.push(new Uint8Array([0x67, 1])), []);
  assertEquals(assembler.push(new Uint8Array([0x65, 2])), []);
  const units = assembler.push(new Uint8Array([0x09, 0x10]));
  assertEquals(units.length, 1);
  assertEquals(packetizeH264AccessUnit(units[0]!, 1100), [
    { payload: new Uint8Array([0x09, 0x10]), endOfFrame: false },
    { payload: new Uint8Array([0x67, 1]), endOfFrame: false },
    { payload: new Uint8Array([0x65, 2]), endOfFrame: true },
  ]);
});

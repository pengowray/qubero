// The line saying where a byte of an unpacked stream came from, and the mark
// one tab puts on another.
//
// The map behind both is package A's, and until its decoders keep a trace every
// `mapOut` answers null and nothing is drawn. What can be pinned down now is
// what happens to an answer once there is one: the exact wording of the line,
// and the range that becomes a mark. The example asserted here is the handover's
// own, character for character, because two places show this line and they have
// to show the same one.

import { test } from "node:test";
import assert from "node:assert/strict";

import { UNPACKED, unpackedOrigin } from "../src/strings.ts";
import { markFromRange, markFromStep, type Step } from "../src/unpackedlink.ts";

/** Bit 5 of byte 0x1a3 to bit 2 of byte 0x1a4: the handover's own example. */
const MATCH: Step = {
  in_start: 0x1a3 * 8 + 5,
  in_end: 0x1a4 * 8 + 2,
  out_start: 0x40,
  out_end: 0x45,
  kind: "match",
  len: 5,
  dist: 12,
};

/** The same bits, for a step that is only its own name. */
const PLAIN = (kind: string): Step => ({
  in_start: MATCH.in_start,
  in_end: MATCH.in_end,
  out_start: MATCH.out_start,
  out_end: MATCH.out_end,
  kind,
});

const line = (s: Step, file = "hello.txt.zst"): string =>
  unpackedOrigin(file, s.in_start, s.in_end, s.kind, s.len, s.dist);

test("a match says how long it is and how far back it reaches", () => {
  assert.equal(line(MATCH), "from bits 0x1a3.5 to 0x1a4.2 of hello.txt.zst: match, 5 bytes back 12");
});

test("a bit offset is the byte in hex and the bit after a dot", () => {
  assert.equal(UNPACKED.bit(0), "0x0.0");
  assert.equal(UNPACKED.bit(0x1a3 * 8 + 5), "0x1a3.5");
  // A whole byte still says which bit, so every offset on the line reads the
  // same way and none of them has to be guessed at.
  assert.equal(UNPACKED.bit(0x1a4 * 8), "0x1a4.0");
});

test("a match of one byte is not called 1 bytes", () => {
  assert.equal(UNPACKED.step("match", 1, 3), "match, 1 byte back 3");
});

test("the steps that only name themselves", () => {
  assert.equal(line(PLAIN("literal")), "from bits 0x1a3.5 to 0x1a4.2 of hello.txt.zst: literal");
  assert.equal(line(PLAIN("stored")), "from bits 0x1a3.5 to 0x1a4.2 of hello.txt.zst: stored");
  assert.equal(UNPACKED.step("block"), "block header");
  assert.equal(UNPACKED.step("header"), "block header");
  assert.equal(UNPACKED.step("table"), "Huffman table");
});

test("a kind this build has no word for is passed through rather than dropped", () => {
  assert.equal(UNPACKED.step("rle"), "rle");
});

test("a match with no length falls back to the plain name", () => {
  // The core sends `len` and `dist` only for a match. A match without them is a
  // step half-described, and "match" beats "match, undefined bytes back".
  assert.equal(UNPACKED.step("match"), "match");
});

test("the tab is named for the stream and the file it came out of", () => {
  assert.equal(UNPACKED.tabTitle("decoded", "hello.txt.zst"), "decoded unpacked from hello.txt.zst");
});

test("the read-only line says what cannot be done and why it is not yet", () => {
  assert.equal(UNPACKED.readOnly, "Unpacked data cannot be edited yet");
});

test("a step becomes the bits to mark in the compressed tab", () => {
  // Bits, not bytes: a deflate literal is a few bits in the middle of one.
  assert.deepEqual(markFromStep(MATCH), { startBit: 0x1a3 * 8 + 5, endBit: 0x1a4 * 8 + 2 });
});

test("a range becomes the bytes to mark in the unpacked tab", () => {
  assert.deepEqual(markFromRange(MATCH), { startBit: 0x200, endBit: 0x228 });
});

test("nothing to mark where the map has no answer", () => {
  assert.equal(markFromStep(null), null);
  assert.equal(markFromRange(null), null);
  // A step that read no bits, and a range of no bytes, are not marks either: an
  // outline round nothing would say the other cursor is somewhere it is not.
  assert.equal(markFromStep({ ...MATCH, in_end: MATCH.in_start }), null);
  assert.equal(markFromRange({ ...MATCH, out_end: MATCH.out_start }), null);
});

// A deflate code's bits between the byte order they are stored in and the
// order they are read. See web/src/bitorder.ts.

import { test } from "node:test";
import assert from "node:assert/strict";

import { bitOrderLayout } from "../src/bitorder.ts";

test("a code's first bit is the low bit of its byte, the rightmost digit written out", () => {
  // Eight bits of Huffman code starting at bit 0 of byte 0.
  const l = bitOrderLayout(0, 8, [{ bits: 8, lowFirst: false, label: "code" }]);
  assert.equal(l.firstByte, 0);
  assert.equal(l.byteCount, 1);
  // Read order 0..7 comes from byte bits 0..7, which are written at 7..0.
  assert.deepEqual(l.top, [7, 6, 5, 4, 3, 2, 1, 0]);
  // A Huffman code is read high bit first: the first bit read is its first
  // digit.
  assert.deepEqual(l.bottom, [0, 1, 2, 3, 4, 5, 6, 7]);
});

test("extra bits are a number stored low bit first, so their digits keep the byte's order", () => {
  const l = bitOrderLayout(0, 4, [{ bits: 4, lowFirst: true, label: "extra" }]);
  assert.deepEqual(l.top, [7, 6, 5, 4]);
  assert.deepEqual(l.bottom, [3, 2, 1, 0]);
  // Top place and bottom place fall the same way: the band does not cross.
  for (let i = 1; i < 4; i++) {
    assert.ok((l.top[i] ?? 0) < (l.top[i - 1] ?? 0));
    assert.ok((l.bottom[i] ?? 0) < (l.bottom[i - 1] ?? 0));
  }
});

test("a code that starts inside a byte and runs into the next spans both, part by part", () => {
  // A length code of 7 bits and 2 extra bits, from bit 5 of byte 10.
  const l = bitOrderLayout(85, 9, [
    { bits: 7, lowFirst: false, label: "code" },
    { bits: 2, lowFirst: true, label: "extra" },
  ]);
  assert.equal(l.firstByte, 10);
  assert.equal(l.byteCount, 2);
  // Bits 5, 6, 7 of the first byte are written at 2, 1, 0; then the second
  // byte from its low bit, written at 15, 14, ...
  assert.deepEqual(l.top.slice(0, 4), [2, 1, 0, 15]);
  assert.deepEqual(l.partOf, [0, 0, 0, 0, 0, 0, 0, 1, 1]);
  // The code's digits are 0..6, the extra bits' 8 then 7.
  assert.deepEqual(l.bottom, [0, 1, 2, 3, 4, 5, 6, 8, 7]);
});

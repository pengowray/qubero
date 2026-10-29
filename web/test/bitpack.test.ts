// The bit-order ribbon's rows and bands, and what its hover card calls each
// bit. See web/src/viz/bitpacklayout.ts.

import { test } from "node:test";
import assert from "node:assert/strict";

import { bitOrderLayout, type BitPart } from "../src/bitorder.ts";
import { digitName, packRibbon, PART_COLORS, partStarts } from "../src/viz/bitpacklayout.ts";

const lengthMatch: BitPart[] = [
  { bits: 7, lowFirst: false, label: "Length code" },
  { bits: 1, lowFirst: true, label: "Length extra bits" },
  { bits: 5, lowFirst: false, label: "Distance code" },
  { bits: 3, lowFirst: true, label: "Distance extra bits" },
];
const matchColors = [PART_COLORS.code, PART_COLORS.extra, PART_COLORS.distanceCode, PART_COLORS.distanceExtra];

test("each part of a match code has its own colour on its bands", () => {
  const layout = bitOrderLayout(3, 16, lengthMatch);
  const rib = packRibbon({ parts: lengthMatch, colors: matchColors, digits: "0".repeat(16), bytes: new Uint8Array(3), code: true }, layout);
  const used = new Set(rib.bands.map((b) => b.color));
  assert.equal(used.size, 4);
  // Band i is bit i of the field, and its colour is its part's.
  assert.equal(rib.bands[0]?.color, PART_COLORS.code);
  assert.equal(rib.bands[7]?.color, PART_COLORS.extra);
  assert.equal(rib.bands[8]?.color, PART_COLORS.distanceCode);
  assert.equal(rib.bands[15]?.color, PART_COLORS.distanceExtra);
});

test("the top row is every bit of the bytes touched, the field's own not dimmed", () => {
  const parts: BitPart[] = [{ bits: 5, lowFirst: true, label: "hlit" }];
  // Five bits from bit 6 of byte 0: two in byte 0, three in byte 1.
  const layout = bitOrderLayout(6, 5, parts);
  const rib = packRibbon({ parts, colors: [PART_COLORS.extra], digits: "10110", bytes: Uint8Array.of(0b1100_0000, 0b0000_0101), code: false }, layout);
  assert.equal(rib.top.boxes.length, 16);
  assert.equal(rib.top.boxes.filter((b) => b.dim !== true).length, 5);
  // Bits 6 and 7 of byte 0 are written at slots 1 and 0, high bit first.
  assert.equal(rib.top.boxes[0]?.dim, false);
  assert.equal(rib.top.boxes[1]?.dim, false);
  assert.equal(rib.top.boxes[2]?.dim, true);
  // The second byte starts half a digit after the first ends.
  assert.equal(rib.top.boxes[8]?.from, 8.5);
  assert.equal(rib.bottom.boxes.length, 5);
});

test("parts along the bottom are half a digit apart", () => {
  const { starts, end } = partStarts(lengthMatch);
  assert.deepEqual(starts, [0, 7.5, 9, 14.5]);
  assert.equal(end, 17.5);
});

test("the hover card counts a number's bits from the lowest and a code's in reading order", () => {
  const layout = bitOrderLayout(0, 16, lengthMatch);
  // The first bit read of a Huffman code is its first digit.
  assert.equal(digitName(layout, lengthMatch, 0, "x"), "Length code: bit 1 of 7, in the order the code is read");
  // The one extra bit is bit 0 of its number.
  assert.equal(digitName(layout, lengthMatch, 7, "x"), "Length extra bits: bit 0 of the number (bit 0 is the lowest)");
  // Extra bits are stored low bit first: the first one read is bit 0, the
  // last is the highest.
  assert.equal(digitName(layout, lengthMatch, 13, "x"), "Distance extra bits: bit 0 of the number (bit 0 is the lowest)");
  assert.equal(digitName(layout, lengthMatch, 15, "x"), "Distance extra bits: bit 2 of the number (bit 0 is the lowest)");
});

// What one cell of the grid shows: which bits of a byte a mark covers, and
// the classes a cell ends up with.

import { test } from "node:test";
import assert from "node:assert/strict";

import { cellDraw, covers, highlightBits, markBits, selectionBits, type CellInput } from "../src/hexcell.ts";

const cell = (o: Partial<CellInput> = {}): CellInput => ({
  off: 0,
  len: 16,
  binary: false,
  complete: true,
  byte: 0x41,
  span: null,
  hl: [],
  sel: null,
  link: null,
  cursor: -1,
  pane: "hex",
  nibble: 0,
  insertMode: false,
  ...o,
});

const classes = (s: string): string[] => s.split(" ").filter((c) => c !== "");

test("a whole byte is covered when its runs reach across it", () => {
  assert.equal(covers([{ from: 0, to: 8 }], 0, 8), true);
  assert.equal(covers([{ from: 0, to: 4 }, { from: 4, to: 8 }], 0, 8), true);
  assert.equal(covers([{ from: 0, to: 3 }, { from: 4, to: 8 }], 0, 8), false);
  assert.equal(covers([], 0, 8), false);
});

test("a highlight is cut to the bits of the byte asked about", () => {
  const hl = [{ startBit: 4, endBit: 20 }];
  assert.deepEqual(highlightBits(hl, 0), [{ from: 4, to: 8 }]);
  assert.deepEqual(highlightBits(hl, 1), [{ from: 0, to: 8 }]);
  assert.deepEqual(highlightBits(hl, 2), [{ from: 0, to: 4 }]);
  assert.deepEqual(highlightBits(hl, 3), []);
});

test("a field of no length keeps its place, in the byte it starts in and no other", () => {
  const hl = [{ startBit: 8, endBit: 8 }];
  assert.deepEqual(highlightBits(hl, 0), []);
  assert.deepEqual(highlightBits(hl, 1), [{ from: 0, to: 0 }]);
});

test("runs that touch or overlap are one mark", () => {
  const hl = [{ startBit: 0, endBit: 4 }, { startBit: 2, endBit: 6 }];
  assert.deepEqual(highlightBits(hl, 0), [{ from: 0, to: 6 }]);
});

test("a selection is cut to the byte, and an empty run is not a selection", () => {
  assert.deepEqual(selectionBits({ startBit: 4, endBit: 12 }, 0), { from: 4, to: 8 });
  assert.deepEqual(selectionBits({ startBit: 4, endBit: 12 }, 2), null);
  assert.deepEqual(selectionBits({ startBit: 8, endBit: 8 }, 1), null);
});

test("a partly covered byte is marked with a gradient, and an uncovered one with nothing", () => {
  assert.match(markBits([{ from: 0, to: 4 }]), /^linear-gradient\(to right, /);
  assert.equal(markBits([]), "");
  // A run of no bits still shows.
  assert.notEqual(markBits([{ from: 3, to: 3 }]), "");
});

test("a byte shows its hex digits and its glyph, and an unprintable one is marked", () => {
  const a = cellDraw(cell({ byte: 0x41 }));
  assert.equal(a.hexText, "41");
  assert.equal(a.asciiText, "A");
  assert.deepEqual(classes(a.ascii), []);
  const b = cellDraw(cell({ byte: 0x00 }));
  assert.equal(b.asciiText, "·");
  assert.deepEqual(classes(b.ascii), ["hv-np"]);
});

test("bytes still on their way say so", () => {
  const d = cellDraw(cell({ complete: false }));
  assert.equal(d.hexText, "··");
  assert.equal(d.asciiText, " ");
  assert.deepEqual(classes(d.hex), ["hv-pending"]);
});

test("the place one past the last byte is marked as the end", () => {
  const d = cellDraw(cell({ off: 16, len: 16 }));
  assert.equal(d.hexText, "  ");
  assert.deepEqual(classes(d.hex), ["hv-end"]);
  // Further past the end there is nothing at all to say.
  assert.deepEqual(classes(cellDraw(cell({ off: 17, len: 16 })).hex), []);
});

test("a field tints its bytes and the first of them says it starts there", () => {
  const start = cellDraw(cell({ span: { kind: "int", startsHere: true } }));
  assert.equal(start.hex.includes("hv-tint"), true);
  assert.equal(start.hex.includes("hv-field-start"), true);
  const rest = cellDraw(cell({ span: { kind: "int", startsHere: false } }));
  assert.equal(rest.hex.includes("hv-field-start"), false);
});

test("a fully covered byte is marked in both columns; a partly covered one weakly in the text", () => {
  const whole = cellDraw(cell({ hl: [{ from: 0, to: 8 }] }));
  assert.equal(whole.hex.includes("hv-hl"), true);
  assert.equal(whole.bits, "");
  assert.equal(whole.ascii.includes("hv-hl"), true);
  const part = cellDraw(cell({ hl: [{ from: 0, to: 4 }] }));
  assert.equal(part.hex.includes("hv-hlbits"), true);
  assert.notEqual(part.bits, "");
  assert.equal(part.ascii.includes("hv-hl-weak"), true);
});

test("a field of no length is not marked in the text column, which cannot show it", () => {
  const d = cellDraw(cell({ hl: [{ from: 3, to: 3 }] }));
  assert.deepEqual(classes(d.ascii), []);
  assert.equal(d.hex.includes("hv-hlbits"), true);
});

test("a whole selected byte is marked fully in the pane it was dragged in, weakly in the other", () => {
  const hex = cellDraw(cell({ sel: { from: 0, to: 8 }, pane: "hex" }));
  assert.equal(hex.hex.includes("hv-sel-weak"), false);
  assert.equal(hex.ascii.includes("hv-sel-weak"), true);
  const text = cellDraw(cell({ sel: { from: 0, to: 8 }, pane: "ascii" }));
  assert.equal(text.hex.includes("hv-sel-weak"), true);
  assert.equal(text.ascii.includes("hv-sel-weak"), false);
  // Half a byte is weak in both: neither column can show half of one.
  const half = cellDraw(cell({ sel: { from: 0, to: 4 }, pane: "hex" }));
  assert.equal(half.hex.includes("hv-sel-weak"), true);
  assert.equal(half.ascii.includes("hv-sel-weak"), true);
});

test("the linked stretch is outlined by the byte, with its two ends marked", () => {
  const link = { startBit: 8, endBit: 24 };
  assert.equal(cellDraw(cell({ off: 0, link })).hex.includes("hv-linked"), false);
  const first = cellDraw(cell({ off: 1, link }));
  assert.equal(first.hex.includes("hv-linked-first"), true);
  assert.equal(first.hex.includes("hv-linked-last"), false);
  const last = cellDraw(cell({ off: 2, link }));
  assert.equal(last.hex.includes("hv-linked-last"), true);
  assert.equal(cellDraw(cell({ off: 3, link })).hex.includes("hv-linked"), false);
});

test("the cursor is bright in the pane it is in and dim in the other", () => {
  const d = cellDraw(cell({ cursor: 0, pane: "hex" }));
  assert.equal(d.hex.includes("hv-cur hv-focus"), true);
  assert.equal(d.ascii.includes("hv-cur hv-dim"), true);
  assert.equal(cellDraw(cell({ cursor: 0, pane: "hex", nibble: 1 })).hex.includes("hv-nib1"), true);
  assert.equal(cellDraw(cell({ cursor: 0, insertMode: true })).hex.includes("hv-ins"), true);
});

test("in binary the bits carry the cursor, except past the end where there are none", () => {
  const inside = cellDraw(cell({ binary: true, cursor: 0 }));
  assert.equal(inside.hexText, null);
  assert.equal(inside.hex.includes("hv-cur"), false);
  const past = cellDraw(cell({ binary: true, cursor: 16, off: 16, len: 16 }));
  assert.equal(past.hexText, "        ");
  assert.equal(past.hex.includes("hv-cur"), true);
});

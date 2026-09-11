// Which of the three layouts a run's values get, and how wide the pieces of
// each layout are.

import { test } from "node:test";
import assert from "node:assert/strict";

import { CHAR, quantRun, run } from "./runcells.ts";
import {
  alignedFits,
  alignedWidth,
  chooseLayout,
  columnsLineUp,
  rowLayout,
  strideBits,
  typeDigits,
  uniformFit,
  uniformWidth,
  VALUE_PAD,
  widestPieceBits,
  type Cell,
} from "../src/valuelayout.ts";

/** The sizes a 16-byte row of a wide column has. */
const WIDE = { bpr: 16, noteWidth: 400, hexPitch: 22, measure: CHAR };

// ----- which layout -----

test("a run whose values fit the bits they are stored in is aligned", () => {
  // Three bytes a value, a wide column: `-394928` has a third of 16 bytes.
  const cells = [run({ stride: 24, from: 0, to: 12, text: () => "-394928" })];
  assert.equal(alignedFits(cells, WIDE), true);
});

test("six-bit codes go uniform: no value fits six bits of a row", () => {
  const cells = [run({ stride: 6, from: 0, to: 40, type: "u6", text: (i) => String(i) })];
  assert.equal(alignedFits(cells, WIDE), false);
});

test("a run the core says is not contiguous goes uniform however well it fits", () => {
  const cells = [run({ stride: 32, from: 0, to: 4, text: () => "1", contiguous: false })];
  assert.equal(alignedFits(cells, WIDE), false);
});

test("a narrow column takes the aligned table down with it", () => {
  const cells = [run({ stride: 24, from: 0, to: 12, text: () => "-394928" })];
  assert.equal(alignedFits(cells, { ...WIDE, noteWidth: 90 }), false);
});

test("a cell whose type says nothing about its width is as wide as its text", () => {
  const cells = [run({ stride: 16, kind: "str", from: 0, to: 4, text: (i) => "0".repeat(i + 1) })];
  assert.equal(uniformWidth(cells, CHAR), 4 * 7 + VALUE_PAD);
});

test("a block's scale does not set the width of the weights beside it", () => {
  // A `q4_0` block is one `0.004108` and thirty-two nibbles reading `-8` to
  // `7`. Measured against the scale every nibble would take eight characters
  // where it needs two, and a sixteen-byte row would run to five lines.
  const cells = [quantRun({ blocks: 2, weights: 32, bits: 4 })];
  assert.equal(uniformWidth(cells, CHAR), 2 * 7 + VALUE_PAD);
  // Nothing a float can promise, so the scale falls back to its own text.
  assert.equal(typeDigits("scale", 16), "");
});

test("packed weights do not fit the bits they are stored in", () => {
  // Four bits at sixteen bytes a row is eleven pixels, and `-8` is nineteen.
  assert.equal(chooseLayout(quantRun({ blocks: 4, weights: 32, bits: 4 }), WIDE), "uniform");
  // A five-bit type keeps a weight's top bit elsewhere, and there is no one
  // place to draw a value whose bits are in two.
  assert.equal(chooseLayout(quantRun({ blocks: 4, weights: 32, bits: 4, contiguous: false }), WIDE), "uniform");
});

test("a number is as wide as its type can be, not as wide as it happens to be", () => {
  // A u16 is five digits whether it holds 7 or 65,535, so the layout holds
  // still while the reader scrolls into the wider values.
  const small = [run({ stride: 16, kind: "uint", from: 0, to: 4, text: () => "7" })];
  const large = [run({ stride: 16, kind: "uint", from: 0, to: 4, text: () => "65535" })];
  assert.equal(uniformWidth(small, CHAR), 5 * 7 + VALUE_PAD);
  assert.equal(uniformWidth(large, CHAR), uniformWidth(small, CHAR));
  // Signed spends a character on the sign and a digit on the smaller range.
  assert.equal(typeDigits("int", 16), "-88888");
  assert.equal(typeDigits("uint", 8), "888");
  assert.equal(typeDigits("float", 32), "");
});

// ----- chooseLayout: one answer per run -----

test("a run that fits its bits chooses aligned", () => {
  assert.equal(chooseLayout(run({ stride: 32, from: 0, to: 12, text: () => "-394928" }), WIDE), "aligned");
});

test("a stride the row does not divide chooses uniform, however well it fits", () => {
  // 24-bit samples over a 16-byte row: every value fits the bits it is stored
  // in, and the columns still come out like brickwork, a third of a byte
  // further along each row. The uniform layout lines them up instead.
  const samples = run({ stride: 24, from: 0, to: 12, text: () => "-394928" });
  assert.equal(alignedFits([samples], WIDE), true);
  assert.equal(columnsLineUp(samples, WIDE.bpr), false);
  assert.equal(chooseLayout(samples, WIDE), "uniform");
  // Three bytes a value does divide a 24-byte row, and there the columns hold.
  assert.equal(columnsLineUp(samples, 24), true);
  assert.equal(chooseLayout(samples, { ...WIDE, bpr: 24, noteWidth: 700 }), "aligned");
});

test("elements that are not a fixed width apart keep the aligned layout", () => {
  // Records that end on what they read have no columns to line up either way,
  // so they stay beside the bytes they were read from.
  const records = run({ stride: 32, from: 0, to: 4, text: () => "ok" });
  const ragged = {
    ...records,
    cells: records.cells.map((c, i) => ({ ...c, offset_bits: c.offset_bits + (i % 2) * 8 })),
  };
  assert.equal(strideBits(ragged.cells), null);
  assert.equal(columnsLineUp(ragged, WIDE.bpr), true);
});

test("a run with one element on screen has no stride to read", () => {
  assert.equal(strideBits(run({ stride: 24, from: 0, to: 1 }).cells), null);
  assert.equal(strideBits(run({ stride: 24, from: 0, to: 3 }).cells), 24);
});

test("a run that does not fit its bits chooses uniform", () => {
  assert.equal(chooseLayout(run({ stride: 6, from: 0, to: 40, type: "u6" }), WIDE), "uniform");
});

test("a run the core says is not contiguous chooses uniform", () => {
  assert.equal(
    chooseLayout(run({ stride: 32, from: 0, to: 4, text: () => "1", contiguous: false }), WIDE),
    "uniform",
  );
});

test("a decoder's symbols choose flow even where they would fit their bits", () => {
  // Nine bits a symbol and one character of text: this would pass the aligned
  // test on width alone, and flows anyway because of what it holds.
  const symbols = run({ stride: 32, from: 0, to: 4, kind: "symbol", text: () => "a" });
  assert.equal(alignedFits([symbols], WIDE), true);
  assert.equal(chooseLayout(symbols, WIDE), "flow");
});

test("a run of no cells chooses uniform, which draws nothing", () => {
  assert.equal(chooseLayout(run({ stride: 24, from: 0, to: 0 }), WIDE), "uniform");
});

// ----- rowLayout: what two runs sharing a row agree on -----

test("a row of one run takes that run's layout", () => {
  assert.equal(rowLayout(["aligned"]), "aligned");
  assert.equal(rowLayout(["flow"]), "flow");
});

test("runs that agree keep their layout and runs that do not fall back to uniform", () => {
  assert.equal(rowLayout(["aligned", "aligned"]), "aligned");
  assert.equal(rowLayout(["flow", "flow"]), "flow");
  assert.equal(rowLayout(["aligned", "flow"]), "uniform");
  assert.equal(rowLayout(["flow", "uniform"]), "uniform");
  assert.equal(rowLayout(["aligned", "uniform"]), "uniform");
});

test("a row no run reaches is uniform", () => {
  assert.equal(rowLayout([]), "uniform");
});

// ----- how many uniform cells a line holds -----

test("the last line of a capped table gives up room to the count", () => {
  const { perLine, lastLine } = uniformFit(400, 40);
  assert.equal(perLine, Math.floor(402 / 42));
  assert.equal(lastLine < perLine, true);
  // Even a column too narrow for one cell draws one rather than none.
  assert.equal(uniformFit(10, 400).perLine, 1);
  assert.equal(uniformFit(10, 400).lastLine, 1);
});

// ----- a straddling element is measured whole -----

test("the piece of a value left on a row does not decide the layout", () => {
  // Three bytes a value at byte 14: one byte of the first is on the first row.
  // Measured on that sliver every run with a straddle would go uniform.
  const cells: Cell[] = [
    { index: 0, offset_bits: 14 * 8, size_bits: 24, text: "-394928", label: "-394928", kind: "int", contiguous: true, repeat: false },
  ];
  const runs = [{ ...run({ stride: 24, from: 0, to: 0 }), cells }];
  assert.equal(alignedFits(runs, WIDE), true);
  // The larger piece is the two bytes on the second row, and the eight
  // characters an i24 can need do not fit two bytes at the hex pitch, so the
  // grid is drawn wider than the bytes: 61px over 16 bits, for 128 bits.
  assert.equal(alignedWidth(runs, { ...WIDE, noteWidth: 600 }), (61 / 16) * 128);
  // A column too narrow for that widened grid stops at the column, and the
  // piece lets its text out over the end of the table instead.
  assert.equal(alignedWidth(runs, WIDE), 400);
  assert.equal(alignedFits(runs, WIDE), true);
});

test("a run nothing cuts is drawn at the hex pitch", () => {
  const runs = [run({ stride: 16, kind: "str", from: 0, to: 8, text: () => "-1" })];
  assert.equal(alignedWidth(runs, WIDE), 16 * 22);
  assert.equal(widestPieceBits({ index: 0, offset_bits: 120, size_bits: 32, text: "", label: "", kind: "int", contiguous: true, repeat: false }, 128), 24);
});

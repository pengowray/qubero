// The table of values beside a folded run: which layout a run's cells get,
// where each cell sits on its row, and what a row of them adds to the row's
// height.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { ChipMeasure } from "../src/chipfit.ts";
import {
  alignedFits,
  planRowValues,
  typeDigits,
  uniformWidth,
  VALUE_PAD,
  type Cell,
  type RunCells,
} from "../src/valuetable.ts";

/** Seven pixels a character, near enough to the mono a cell is drawn in, so a
 *  width in this file is a count of characters at that pitch. */
const CHAR: ChipMeasure = { name: (s) => s.length * 7, value: (s) => s.length * 7 };

/** A run of fixed-stride elements starting at byte 0. */
function run(o: {
  name?: string;
  type?: string;
  kind?: string;
  stride: number;
  from: number;
  to: number;
  text?: (i: number) => string;
  contiguous?: boolean;
}): RunCells {
  const cells: Cell[] = [];
  for (let i = o.from; i < o.to; i++) {
    cells.push({
      index: i,
      offset_bits: i * o.stride,
      size_bits: o.stride,
      text: (o.text ?? ((n: number) => String(n)))(i),
      kind: o.kind ?? "int",
      contiguous: o.contiguous ?? true,
    });
  }
  return { path: [3], name: o.name ?? "body", type: o.type ?? "i24 le", symbol: false, widest: "", cells };
}

/** The plan for one row, with the sizes a 16-byte row of a wide column has. */
function row(
  runs: readonly RunCells[],
  rowStart: number,
  o: { bpr?: number; layout?: "aligned" | "uniform"; noteWidth?: number; maxLines?: number; cellWidth?: number } = {},
) {
  return planRowValues({
    runs,
    rowStart,
    bpr: o.bpr ?? 16,
    layout: o.layout ?? "aligned",
    cellWidth: o.cellWidth ?? uniformWidth(runs, CHAR),
    noteWidth: o.noteWidth ?? 400,
    maxLines: o.maxLines ?? Infinity,
    valLine: 18,
  });
}

// ----- which layout -----

test("a run whose values fit the bits they are stored in is aligned", () => {
  // Three bytes a value, a wide column: `-394928` has a third of 16 bytes.
  const cells = [run({ stride: 24, from: 0, to: 12, text: () => "-394928" })];
  assert.equal(alignedFits(cells, { bpr: 16, noteWidth: 400, hexPitch: 22, measure: CHAR }), true);
});

test("six-bit codes go uniform: no value fits six bits of a row", () => {
  const cells = [run({ stride: 6, from: 0, to: 40, type: "u6", text: (i) => String(i) })];
  assert.equal(alignedFits(cells, { bpr: 16, noteWidth: 400, hexPitch: 22, measure: CHAR }), false);
});

test("a run the core says is not contiguous goes uniform however well it fits", () => {
  const cells = [run({ stride: 32, from: 0, to: 4, text: () => "1", contiguous: false })];
  assert.equal(alignedFits(cells, { bpr: 16, noteWidth: 400, hexPitch: 22, measure: CHAR }), false);
});

test("a narrow column takes the aligned table down with it", () => {
  const cells = [run({ stride: 24, from: 0, to: 12, text: () => "-394928" })];
  assert.equal(alignedFits(cells, { bpr: 16, noteWidth: 90, hexPitch: 22, measure: CHAR }), false);
});

test("a cell whose type says nothing about its width is as wide as its text", () => {
  const cells = [run({ stride: 16, kind: "str", from: 0, to: 4, text: (i) => "0".repeat(i + 1) })];
  assert.equal(uniformWidth(cells, CHAR), 4 * 7 + VALUE_PAD);
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

// ----- aligned -----

test("24-bit samples at 16 bytes a row: the sixth of each row is split", () => {
  const cells = [run({ stride: 24, from: 0, to: 12, text: (i) => `v${i}` })];
  const first = row(cells, 0);
  assert.equal(first.layout, "aligned");
  assert.equal(first.lines, 1);
  assert.equal(first.height, 18);
  assert.deepEqual(
    first.cells.map((c) => [c.index, c.from, c.to, c.text]),
    [
      [0, 1, 25, "v0"],
      [1, 25, 49, "v1"],
      [2, 49, 73, "v2"],
      [3, 73, 97, "v3"],
      [4, 97, 121, "v4"],
      // Cut by the row edge, its text on the row it starts on.
      [5, 121, 129, "v5"],
    ],
  );
  const second = row(cells, 16);
  assert.equal(second.cells[0]?.continued, true);
  assert.deepEqual(
    second.cells.map((c) => [c.index, c.from, c.to, c.text]),
    [
      [5, 1, 17, ""],
      [6, 17, 41, "v6"],
      [7, 41, 65, "v7"],
      [8, 65, 89, "v8"],
      [9, 89, 113, "v9"],
      [10, 113, 129, "v10"],
    ],
  );
});

test("a u8 run is one cell a byte, over the byte it is", () => {
  const cells = [run({ stride: 8, from: 0, to: 32, type: "u8", text: (i) => String(i) })];
  const first = row(cells, 0);
  assert.equal(first.layout, "aligned");
  assert.equal(first.cells.length, 16);
  assert.deepEqual(
    [first.cells[0]?.from, first.cells[0]?.to, first.cells[15]?.from, first.cells[15]?.to],
    [1, 9, 121, 129],
  );
  assert.equal(first.cells.every((c) => !c.continued), true);
  // The second row is the next sixteen, not the same sixteen again.
  assert.deepEqual(row(cells, 16).cells[0]?.index, 16);
});

test("numbers are read from the right and everything else from the left", () => {
  const numbers = row([run({ stride: 16, from: 0, to: 8, kind: "uint" })], 0);
  assert.equal(numbers.cells[0]?.numeric, true);
  const text = row([run({ stride: 16, from: 0, to: 8, kind: "str" })], 0);
  assert.equal(text.cells[0]?.numeric, false);
});

test("a row no run reaches has no table and adds no height", () => {
  const cells = [run({ stride: 24, from: 0, to: 4, text: () => "v" })];
  const empty = row(cells, 16);
  assert.equal(empty.lines, 0);
  assert.equal(empty.height, 0);
  assert.equal(empty.key, "");
});

// ----- uniform -----

test("uniform wraps into as many lines as the row needs", () => {
  // 512 six-bit codes: 21 and a bit to a 16-byte row, four to a line of 40px.
  const cells = [run({ stride: 6, from: 0, to: 512, type: "u6", text: () => "-13" })];
  const plan = row(cells, 0, { layout: "uniform", noteWidth: 4 * (3 * 7 + VALUE_PAD) + 3 * 2 });
  assert.equal(plan.layout, "uniform");
  assert.equal(plan.cells.length, 22);
  assert.equal(plan.rest, 0);
  assert.equal(plan.lines, 6);
  assert.equal(plan.height, 6 * 18);
  // No grid columns in this layout, and no cell drawn twice.
  assert.equal(plan.cells[0]?.from, 0);
  assert.deepEqual(plan.cells[0]?.index, 0);
  assert.deepEqual(plan.cells[21]?.index, 21);
});

test("condensed stops at three lines and counts the rest", () => {
  const cells = [run({ stride: 6, from: 0, to: 512, type: "u6", text: () => "-13" })];
  const plan = row(cells, 0, { layout: "uniform", noteWidth: 4 * (3 * 7 + VALUE_PAD) + 3 * 2, maxLines: 3 });
  assert.equal(plan.lines, 3);
  assert.equal(plan.height, 3 * 18);
  // Two full lines and a last line with room kept for the count.
  assert.equal(plan.cells.length + plan.rest, 22);
  assert.equal(plan.rest > 0, true);
  assert.equal(plan.cells.length, 10);
  assert.equal(plan.rest, 12);
});

test("what a row says is its key, and a changed value changes it", () => {
  const a = row([run({ stride: 16, from: 0, to: 8, text: () => "1" })], 0);
  const b = row([run({ stride: 16, from: 0, to: 8, text: () => "1" })], 0);
  const c = row([run({ stride: 16, from: 0, to: 8, text: (i) => (i === 3 ? "2" : "1") })], 0);
  assert.equal(a.key, b.key);
  assert.notEqual(a.key, c.key);
});

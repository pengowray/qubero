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
  VALUE_GAP,
  VALUE_PAD,
  VALUE_REST,
  type Cell,
  type Layout,
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
  /** What the cell shows, where that is not what its tooltip says. */
  label?: (i: number) => string;
  contiguous?: boolean;
  symbol?: boolean;
  at?: number;
}): RunCells {
  const cells: Cell[] = [];
  const text = o.text ?? ((n: number) => String(n));
  for (let i = o.from; i < o.to; i++) {
    cells.push({
      index: i,
      offset_bits: (o.at ?? 0) + i * o.stride,
      size_bits: o.stride,
      text: text(i),
      label: (o.label ?? text)(i),
      kind: o.kind ?? "int",
      contiguous: o.contiguous ?? true,
    });
  }
  return {
    path: [3],
    name: o.name ?? "body",
    type: o.type ?? "i24 le",
    symbol: o.symbol ?? false,
    widest: "",
    cells,
  };
}

/** The plan for one row, with the sizes a 16-byte row of a wide column has. */
function row(
  runs: readonly RunCells[],
  rowStart: number,
  o: { bpr?: number; layout?: Layout; noteWidth?: number; maxLines?: number; cellWidth?: number } = {},
) {
  return planRowValues({
    runs,
    rowStart,
    bpr: o.bpr ?? 16,
    layout: o.layout ?? "aligned",
    measure: CHAR,
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
      // One byte of it is on this row and two on the next, so the number is
      // said down there and this piece is left empty.
      [5, 121, 129, ""],
    ],
  );
  assert.equal(first.cells[5]?.carried, "below");
  const second = row(cells, 16);
  assert.equal(second.cells[0]?.carried, null);
  assert.deepEqual(
    second.cells.map((c) => [c.index, c.from, c.to, c.text]),
    [
      [5, 1, 17, "v5"],
      [6, 17, 41, "v6"],
      [7, 41, 65, "v7"],
      [8, 65, 89, "v8"],
      [9, 89, 113, "v9"],
      [10, 113, 129, "v10"],
    ],
  );
  // The piece carrying the text is still narrower than the value needs, so it
  // is let out of its cell.
  assert.equal(second.cells[0]?.cut, true);
  assert.equal(second.cells[1]?.cut, false);
});

test("a value cut by the row edge is said on its wider piece", () => {
  // Three bytes at byte 14: two on this row, one on the next.
  const cells = [run({ stride: 24, from: 0, to: 1, at: 14 * 8, text: () => "v" })];
  const first = row(cells, 0);
  assert.equal(first.cells[0]?.text, "v");
  assert.equal(first.cells[0]?.carried, null);
  const second = row(cells, 16);
  assert.equal(second.cells[0]?.text, "");
  assert.equal(second.cells[0]?.carried, "above");
});

test("a value split evenly is said where it starts", () => {
  // Four bytes at byte 14 of a 16-byte row: two either side of the edge.
  const cells = [run({ stride: 32, from: 0, to: 1, at: 14 * 8, text: () => "v" })];
  assert.equal(row(cells, 0).cells[0]?.text, "v");
  assert.equal(row(cells, 16).cells[0]?.carried, "above");
});

test("the two pieces of a cut value say which way the other lies", () => {
  const cells = [run({ stride: 24, from: 5, to: 6, text: () => "v" })];
  const before = row(cells, 0).cells[0];
  const after = row(cells, 16).cells[0];
  // The same element, once on each row, and only one of them says the number.
  assert.deepEqual([before?.index, after?.index], [5, 5]);
  assert.deepEqual([before?.carried, after?.carried], ["below", null]);
  // What the row says changes with which way the piece looks, so a redraw
  // cannot leave the wrong tooltip on a reused cell.
  assert.notEqual(row(cells, 0).key, row(cells, 16).key);
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
  assert.equal(first.cells.every((c) => c.carried === null), true);
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

// ----- flow -----

/** A block of deflate symbols: literals of one character each and a match
 *  among them, the widths their bits were coded in. */
function symbols(): RunCells[] {
  const cells: Cell[] = [];
  let at = 0;
  for (let i = 0; i < 30; i++) {
    const match = i % 10 === 9;
    cells.push({
      index: i,
      offset_bits: at,
      size_bits: match ? 17 : 9,
      text: match ? "match 6 back 18" : `literal '${String.fromCharCode(97 + (i % 26))}'`,
      label: match ? "6 back 18" : String.fromCharCode(97 + (i % 26)),
      kind: "symbol",
      contiguous: true,
    });
    at += match ? 17 : 9;
  }
  return [{ path: [6, 1, 0], name: "symbols", type: "symbol", symbol: true, widest: "", cells }];
}

test("symbols flow, each cell as wide as the byte it decodes to", () => {
  const plan = row(symbols(), 0, { layout: "flow", noteWidth: 200 });
  assert.equal(plan.layout, "flow");
  // A literal is one character and a match is nine, so the cells are not one
  // width. Every symbol on the row is drawn, in the order of the bytes.
  assert.equal(plan.cells[0]?.text, "a");
  assert.equal(plan.cells[0]?.width, 7 + VALUE_PAD);
  assert.equal(plan.cells[9]?.text, "6 back 18");
  assert.equal(plan.cells[9]?.width, 9 * 7 + VALUE_PAD);
  // The tooltip still says the whole thing.
  assert.equal(plan.cells[0]?.tip, "literal 'a'");
  // A match and the end of a block are the copies; a literal is not.
  assert.equal(plan.cells[9]?.copy, true);
  assert.equal(plan.cells[0]?.copy, false);
});

test("a flow row takes as many lines as its own widths need", () => {
  // 16 bytes is 128 bits: thirteen literals of nine bits and one match of
  // seventeen begin on the row, and a symbol belongs to the row it begins on.
  const plan = row(symbols(), 0, { layout: "flow", noteWidth: 200 });
  assert.equal(plan.cells.length, 14);
  // Greedy, at the widths the cells are drawn: what fits a 200px line.
  let lines = 1;
  let used = 0;
  for (const c of plan.cells) {
    const next = used === 0 ? c.width : used + VALUE_GAP + c.width;
    if (used > 0 && next > 200) {
      lines++;
      used = c.width;
    } else used = next;
  }
  assert.equal(plan.lines, lines);
  assert.equal(plan.height, lines * 18);
  assert.equal(plan.rest, 0);
});

test("a narrow column wraps a flow row and the cap counts the rest", () => {
  const narrow = row(symbols(), 0, { layout: "flow", noteWidth: 40 });
  assert.equal(narrow.lines > 3, true);
  const capped = row(symbols(), 0, { layout: "flow", noteWidth: 40, maxLines: 3 });
  assert.equal(capped.lines, 3);
  assert.equal(capped.height, 3 * 18);
  assert.equal(capped.rest > 0, true);
  assert.equal(capped.cells.length + capped.rest, narrow.cells.length);
  // The last line keeps room for the count of what is left.
  assert.equal(VALUE_REST > 0, true);
});

test("a label wider than the column takes a line rather than none", () => {
  const wide = [
    {
      path: [6, 1, 0],
      name: "symbols",
      type: "symbol",
      symbol: true,
      widest: "",
      cells: [
        { index: 0, offset_bits: 0, size_bits: 9, text: "literal 'a'", label: "a", kind: "symbol", contiguous: true },
        {
          index: 1,
          offset_bits: 9,
          size_bits: 17,
          text: "match 300 back 32000",
          label: "300 back 32000",
          kind: "symbol",
          contiguous: true,
        },
      ] as Cell[],
    },
  ];
  const plan = row(wide, 0, { layout: "flow", noteWidth: 20 });
  assert.equal(plan.cells.length, 2);
  assert.equal(plan.lines, 2);
});

test("what a row says is its key, and a changed value changes it", () => {
  const a = row([run({ stride: 16, from: 0, to: 8, text: () => "1" })], 0);
  const b = row([run({ stride: 16, from: 0, to: 8, text: () => "1" })], 0);
  const c = row([run({ stride: 16, from: 0, to: 8, text: (i) => (i === 3 ? "2" : "1") })], 0);
  assert.equal(a.key, b.key);
  assert.notEqual(a.key, c.key);
});

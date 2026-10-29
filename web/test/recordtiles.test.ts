// The DOM-free halves of the record tiles and the run sparkline: colour slots
// by kind, the tile grid and its hit test, the fill, the word for a list's
// records, and the sparkline's number parsing, envelope and end labels.

import { test } from "node:test";
import assert from "node:assert/strict";

import { assignCategories, cellOf, collapseRuns, fillPx, HUE_SLOTS, OTHER_SLOT, recordWord, tileAt, tileBox, tileGrid } from "../src/viz/tilelayout.ts";
import { binOf, binRecords, envelope, findStep, parseNumber, sameRun, shortNumber, worthDrawing } from "../src/viz/sparkmath.ts";
import { figureKey, figureOpenOn, figurePath, isFigureKey } from "../src/viz/headingfigures.ts";

// ----- categories -----

test("each kind gets a slot in order of first appearance", () => {
  const { slot, legend } = assignCategories(["IHDR", "PLTE", "IDAT", "IDAT", "IEND"]);
  assert.deepEqual(slot, [0, 1, 2, 2, 3]);
  assert.deepEqual(
    legend.map((e) => [e.name, e.count, e.slot]),
    [
      ["IHDR", 1, 0],
      ["PLTE", 1, 1],
      ["IDAT", 2, 2],
      ["IEND", 1, 3],
    ],
  );
});

test("the most common kinds get the hues and the rest share grey", () => {
  const kinds = ["a", "b", "c", "d", "big", "big", "big", "e", "f", "g", "h", "h"];
  const { slot, legend } = assignCategories(kinds, 3);
  // big (3) and h (2) are the commonest; a is the first of the ties at 1.
  assert.equal(slot[kinds.indexOf("big")], 1);
  assert.equal(slot[kinds.indexOf("a")], 0);
  assert.equal(slot[kinds.indexOf("h")], 2);
  assert.equal(slot[kinds.indexOf("b")], OTHER_SLOT);
  const grey = legend[legend.length - 1];
  assert.equal(grey?.slot, OTHER_SLOT);
  assert.equal(grey?.name, null);
  assert.equal(grey?.count, 6);
  assert.deepEqual(grey?.kinds, ["b", "c", "d", "e", "f", "g"]);
});

test("one kind left over keeps its name in grey", () => {
  const kinds = Array.from({ length: HUE_SLOTS + 1 }, (_, i) => `k${i}`);
  const { legend } = assignCategories(kinds);
  assert.equal(legend.length, HUE_SLOTS + 1);
  assert.equal(legend[HUE_SLOTS]?.name, `k${HUE_SLOTS}`);
  assert.equal(legend[HUE_SLOTS]?.slot, OTHER_SLOT);
});

test("a kind keeps its colour when records of other kinds are added after it", () => {
  const before = assignCategories(["x", "y", "y"]);
  const after = assignCategories(["x", "y", "y", "z", "w"]);
  assert.equal(before.slot[0], after.slot[0]);
  assert.equal(before.slot[1], after.slot[1]);
});

// ----- grid -----

test("a short list gets the largest tiles, in one row as wide as the tiles", () => {
  const g = tileGrid(6, 800, 120);
  assert.equal(g.cell, 28);
  assert.equal(g.rows, 1);
  assert.equal(g.width, 6 * 30 - 2);
  assert.equal(g.height, 28);
});

test("a long list gets smaller tiles to stay under the height", () => {
  const g = tileGrid(4000, 800, 120);
  assert.ok(g.cell < 28);
  assert.ok(g.height <= 120, `height ${g.height}`);
  assert.ok(g.cols * g.rows >= 4000);
  assert.ok(g.cols * g.pitch - g.gap <= 800);
});

test("a list too long for any tile size takes the smallest and grows", () => {
  const g = tileGrid(100_000, 300, 40);
  assert.equal(g.cell, 2);
  assert.ok(g.height > 40);
  assert.ok(g.cols * g.rows >= 100_000);
});

test("the hit test finds the tile under the pointer and the nearest in a gap", () => {
  const g = tileGrid(10, 4 * 30 - 2, 1000); // 4 columns of 28px tiles, 3 rows
  assert.equal(g.cols, 4);
  const b = tileBox(g, 5);
  assert.equal(tileAt(g, (b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2), 5);
  // In the 2px gap right of tile 5: still 5 or 6, never null.
  const inGap = tileAt(g, b.x1 + 0.5, b.y0 + 4);
  assert.ok(inGap === 5 || inGap === 6);
  // Past the last tile on the last row: nothing.
  const last = tileBox(g, 9);
  assert.equal(tileAt(g, last.x1 + 20, last.y0 + 4), null);
  assert.equal(tileAt(g, -40, -40), null);
});

test("a tile smaller than the target is still picked near its centre", () => {
  const g = tileGrid(100_000, 300, 40); // 2px tiles
  const b = tileBox(g, 0);
  assert.equal(tileAt(g, b.x0 + 1, b.y0 + 1, 24), 0);
  assert.equal(tileAt(g, b.x0 + 1, b.y0 + 1, 0), 0);
});

test("the fill is linear and never empty for a record with bytes", () => {
  assert.equal(fillPx(780 * 8, 780 * 8, 28), 28);
  assert.equal(fillPx(390 * 8, 780 * 8, 28), 14);
  assert.equal(fillPx(12 * 8, 780 * 8, 28), 1);
  assert.equal(fillPx(0, 780 * 8, 28), 0);
});

test("a list's records are called by its unit, else its element type", () => {
  assert.equal(recordWord({ unit: "event", type: "Event[]" }), "event");
  assert.equal(recordWord({ type: "Chunk[]" }), "chunk");
  assert.equal(recordWord({ type: "offsets → Cell" }), "cell");
  assert.equal(recordWord({ type: "u16 be[]" }), "item");
  assert.equal(recordWord({ type: "IFDEntry[]" }), "item");
});

// ----- sparkline -----

test("numbers parse in decimal and hex, and anything else is not a number", () => {
  assert.equal(parseNumber("768"), 768);
  assert.equal(parseNumber("-3"), -3);
  assert.equal(parseNumber("0x1f"), 31);
  assert.equal(parseNumber("-0x10"), -16);
  assert.equal(parseNumber("1.5e3"), 1500);
  assert.equal(parseNumber(""), null);
  assert.equal(parseNumber("IHDR"), null);
});

test("a short run is one bin per record", () => {
  const e = envelope([3, 1, 2], 10);
  assert.equal(e.exact, true);
  assert.deepEqual(e.min, [3, 1, 2]);
  assert.deepEqual(e.max, [3, 1, 2]);
  assert.deepEqual(e.first, [0, 1, 2]);
});

test("a long run keeps each bin's smallest and largest, so a spike survives", () => {
  const values = new Array(1000).fill(5);
  values[517] = 900;
  const e = envelope(values, 50);
  assert.equal(e.exact, false);
  assert.equal(e.min.length, 50);
  assert.equal(Math.max(...e.max), 900);
  assert.equal(Math.min(...e.min), 5);
  const j = binOf(517, 1000, 50);
  assert.equal(e.max[j], 900);
  const r = binRecords(j, 1000, 50);
  assert.ok(r.from <= 517 && 517 < r.to);
});

test("every record lands in exactly one bin", () => {
  const n = 997;
  const bins = 64;
  const seen = new Array<number>(n).fill(0);
  for (let j = 0; j < bins; j++) {
    const { from, to } = binRecords(j, n, bins);
    for (let i = from; i < to; i++) seen[i] = (seen[i] ?? 0) + 1;
  }
  assert.ok(seen.every((c) => c === 1));
  for (let i = 0; i < n; i += 37) {
    const r = binRecords(binOf(i, n, bins), n, bins);
    assert.ok(r.from <= i && i < r.to, `record ${i}`);
  }
});

test("end labels are short", () => {
  assert.equal(shortNumber(0), "0");
  assert.equal(shortNumber(768), "768");
  assert.equal(shortNumber(12_345), "12.3k");
  assert.equal(shortNumber(4_294_967_295), "4.29G");
  assert.equal(shortNumber(-2500), "-2.5k");
  assert.equal(shortNumber(0.125), "0.125");
});

test("a run of the same value, or of fewer than three, is not drawn", () => {
  assert.equal(worthDrawing([1, 1, 1, 1]), false);
  assert.equal(worthDrawing([1, 2]), false);
  assert.equal(worthDrawing([1, 2, 1]), true);
});

// ----- figures under a heading -----

test("a figure key names its figure and the heading it opens under", () => {
  const key = figureKey("record-tiles", [1, 0]);
  assert.equal(isFigureKey(key), true);
  assert.equal(figurePath(key), "1.0");
  assert.equal(figurePath(figureKey("record-tiles", [])), "");
  assert.equal(isFigureKey("ask:struct:1.0"), false);
  assert.equal(figurePath("ask:struct:1.0"), null);
});

test("a heading is measured only while a figure is open under it", () => {
  const cards = new Set([figureKey("record-tiles", [2]), "ask:struct:3"]);
  assert.equal(figureOpenOn(cards, [2]), true);
  assert.equal(figureOpenOn(cards, [3]), false);
  // A heading whose path only starts like an open one's is not it.
  assert.equal(figureOpenOn(new Set([figureKey("record-tiles", [2, 1])]), [2]), false);
  assert.equal(figureOpenOn(new Set([figureKey("record-tiles", [12])]), [2]), false);
});

// ----- runs of the same record -----

test("records in a row with the same key are one cell with a count", () => {
  const cells = collapseRuns(["IHDR", "IDAT:8192", "IDAT:8192", "IDAT:8192", "IDAT:100", "IEND"]);
  assert.deepEqual(cells.first, [0, 1, 4, 5]);
  assert.deepEqual(cells.count, [1, 3, 1, 1]);
});

test("the same key apart, or a null key, is not a run", () => {
  const cells = collapseRuns(["a", "b", "a", null, null, "c", "c"]);
  assert.deepEqual(cells.first, [0, 1, 2, 3, 4, 5]);
  assert.deepEqual(cells.count, [1, 1, 1, 1, 1, 2]);
});

test("each record is found in the cell that stands for it", () => {
  const cells = collapseRuns(["x", "y", "y", "y", "z", "z", "w"]);
  assert.deepEqual([0, 1, 2, 3, 4, 5, 6].map((r) => cellOf(cells, r)), [0, 1, 1, 1, 2, 2, 3]);
  assert.equal(cellOf(cells, 7), null);
  assert.equal(cellOf(collapseRuns([]), 0), null);
});

// ----- the same field in records of other types -----

test("a field is found by name in records of other types", () => {
  // A ZIP's records: the body of each is a LocalFile, a DataDescriptor or a
  // CentralDirectoryFile, and each has a compressed_size somewhere else.
  const local = [{ name: "version" }, { name: "flags" }, { name: "method" }, { name: "compressed_size" }];
  const descriptor = [{ name: "crc32" }, { name: "compressed_size" }, { name: "uncompressed_size" }];
  const central = [{ name: "made_by" }, { name: "version" }, { name: "flags" }, { name: "method" }, { name: "compressed_size" }];
  const step = { name: "compressed_size", index: 3 };
  assert.equal(findStep(local, step), 3);
  assert.equal(findStep(descriptor, step), 1);
  assert.equal(findStep(central, step), 4);
  assert.equal(findStep([{ name: "signature" }], step), null);
});

test("a flat stretch of a run is found either side of a point", () => {
  const values = [1, 5, 5, 5, 2, 2, 9];
  assert.deepEqual(sameRun(values, 2), { from: 1, to: 4 });
  assert.deepEqual(sameRun(values, 0), { from: 0, to: 1 });
  assert.deepEqual(sameRun(values, 5), { from: 4, to: 6 });
});

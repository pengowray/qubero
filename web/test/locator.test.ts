// The arithmetic behind the locator pieces: how the file map cuts parts into
// cells in file order (`viz/mapcells.ts`), what a pointer on a strip is on,
// how much of a cell a lit range covers, where a crumb's field sits in the
// crumb before it (`trail.ts`), and which strips the zoom stack draws for a
// path (`viz/stacklevels.ts`).

import { test } from "node:test";
import assert from "node:assert/strict";

import { bitInCell, cellAt, cellSpans, cellsThatFit, foldCells, litSpans, mapCells, MIN_LIT_FRACTION, partAt, placeIn, type MapCell } from "../src/viz/mapcells.ts";
import { levelDomain, stackPlan, type StackNode } from "../src/viz/stacklevels.ts";
import { trailItems, type TrailNode } from "../src/trail.ts";

const part = (byteAt: number, bytes: number) => ({ offsetBits: byteAt * 8, sizeBits: bytes * 8 });
const owners = (cells: readonly MapCell[]): number[] => cells.map((c) => c.owner);

// ----- cells -----

test("parts in order end to end are one cell each", () => {
  const cells = mapCells([part(0, 76), part(76, 16), part(92, 71)]);
  assert.deepEqual(owners(cells), [0, 1, 2]);
  assert.equal(cells[2]!.end, 163 * 8);
});

test("bytes between parts are a cell of no part, and so are bytes past the last when the length is given", () => {
  const cells = mapCells([part(0, 4), part(10, 4)], 0, 20 * 8);
  assert.deepEqual(owners(cells), [0, -1, 1, -1]);
  assert.deepEqual(cells.map((c) => c.end - c.start), [32, 48, 32, 48]);
});

test("parts handed over out of order are drawn in file order", () => {
  // The listing gives a PDF's headings in reading order: the trailer before
  // the objects that come first in the file.
  const cells = mapCells([part(100, 10), part(0, 100)]);
  assert.deepEqual(owners(cells), [1, 0]);
});

test("a part with others inside it is cut round them, and the innermost wins", () => {
  // linearized.pdf: `objects` runs from byte 9 to the end with the first
  // page's cross-reference table and trailer inside it.
  const parts = [part(0, 9), part(9, 2601), part(216, 4), part(220, 125), part(345, 150)];
  const cells = mapCells(parts);
  assert.deepEqual(owners(cells), [0, 1, 2, 3, 4, 1]);
  assert.equal(cells[5]!.start, 495 * 8);
  assert.equal(cells[5]!.end, 2610 * 8);
});

test("a part of no size has nothing to draw", () => {
  assert.deepEqual(owners(mapCells([part(0, 4), part(4, 0), part(4, 4)])), [0, 2]);
  assert.deepEqual(mapCells([]), []);
});

test("many cells fold into as many as the strip can draw, keeping every part", () => {
  const parts = Array.from({ length: 1000 }, (_, i) => part(i * 10, 10));
  const max = cellsThatFit(96);
  assert.equal(max, 32);
  const folded = foldCells(mapCells(parts), max);
  assert.ok(folded.length <= max);
  assert.equal(folded.reduce((n, c) => n + c.owners.length, 0), 1000);
  assert.equal(folded[0]!.start, 0);
  assert.equal(folded[folded.length - 1]!.end, 10_000 * 8);
});

test("a folded cell takes the colour of the part with most bytes in it", () => {
  const folded = foldCells(mapCells([part(0, 1), part(1, 50), part(51, 1), part(52, 1)]), 2);
  assert.equal(folded[0]!.owner, 1);
});

// ----- the pointer -----

test("a two-pixel cell is found from beside it, and the bit under the pointer names the part in a folded cell", () => {
  const spans = cellSpans([40, 2, 52]);
  assert.equal(spans[1]!.start, 41);
  assert.equal(cellAt(spans, 42), 1);
  // Inside the big neighbour, its own cell wins.
  assert.equal(cellAt(spans, 30), 0);
  const parts = [part(0, 10), part(10, 30)];
  const cell: MapCell = { start: 0, end: 40 * 8, owner: 1, owners: [0, 1] };
  const bit = bitInCell(cell, { start: 0, end: 40 }, 5);
  assert.equal(bit, 40);
  assert.equal(partAt(parts, cell, bit), 0);
  assert.equal(partAt(parts, cell, bitInCell(cell, { start: 0, end: 40 }, 39)), 1);
});

// ----- lit ranges -----

test("lit ranges in one cell merge, and a range too thin to see is widened inside the cell", () => {
  const cell = { start: 0, end: 1000 };
  assert.deepEqual(litSpans(cell, [{ offsetBits: 0, sizeBits: 500 }, { offsetBits: 400, sizeBits: 200 }]), [[0, 0.6]]);
  const thin = litSpans(cell, [{ offsetBits: 999, sizeBits: 1 }]);
  assert.equal(thin.length, 1);
  assert.ok(Math.abs(thin[0]![1] - thin[0]![0] - MIN_LIT_FRACTION) < 1e-9);
  assert.ok(thin[0]![1] <= 1);
  assert.deepEqual(litSpans(cell, [{ offsetBits: 2000, sizeBits: 8 }]), []);
});

test("a range of no size still marks where it is", () => {
  assert.equal(litSpans({ start: 0, end: 100 }, [{ offsetBits: 50, sizeBits: 0 }]).length, 1);
});

// ----- crumbs -----

test("a crumb's field is placed in its parent, and not across address spaces", () => {
  assert.deepEqual(placeIn({ offsetBits: 0, sizeBits: 50 }, { offsetBits: 0, sizeBits: 100 }, 0.05), [0, 0.5]);
  assert.equal(placeIn({ offsetBits: 0, sizeBits: 8, space: 1 }, { offsetBits: 368, sizeBits: 240 }, 0.05), null);
  assert.equal(placeIn({ offsetBits: 0, sizeBits: 8 }, { offsetBits: 0, sizeBits: 0 }, 0.05), null);
});

test("the trail places each crumb in the one before it", () => {
  const n = (name: string, offset: number, size: number, composite: boolean, list = false, space = 0): TrailNode => ({
    name, type: name, composite, list, size_bits: size * 8, child_count: composite ? 2 : 0, inline: false, offset_bits: offset * 8, space,
  });
  const nodes: Record<string, TrailNode> = {
    "": n("file", 0, 318, true),
    "0": n("records", 0, 318, true, true),
    "0/2": n("[2] notes.txt", 92, 71, true),
    "0/2/0": n("signature", 92, 4, false),
  };
  const items = trailItems([0, 2, 0], (p) => nodes[p.join("/")] ?? null);
  assert.deepEqual(items.map((i) => i.label), ["file", "records[2]", "signature"]);
  assert.equal(items[0]!.place, null);
  const rec = items[1]!.place!;
  assert.ok(Math.abs(rec.at[0] - 92 / 318) < 1e-9);
  assert.equal(rec.parent, "file");
  assert.equal(rec.parentBits, 318 * 8);
  assert.equal(items[2]!.place!.at[0], 0);
});

// ----- the zoom stack -----

const node = (path: number[], name: string, at: number, size: number, children: number, space = 0): StackNode => ({
  path, name, offset_bits: at * 8, size_bits: size * 8, space, composite: children > 0, child_count: children,
});

test("a wrapper with one child gets no strip, and the field's structure is the last", () => {
  // streamed-data-descriptors.zip: file > records > [0] > body > crc32.
  const chain = [node([], "file", 0, 318, 1), node([0], "records", 0, 318, 7), node([0, 0], "[0]", 0, 76, 2), node([0, 0, 1], "body", 4, 72, 15), node([0, 0, 1, 5], "crc32", 14, 4, 0)];
  const { levels, skipped } = stackPlan(chain);
  assert.equal(skipped, 0);
  assert.deepEqual(levels.map((l) => l.parent.name), ["records", "[0]", "body"]);
  assert.deepEqual(levels.map((l) => l.lit?.name), ["[0]", "body", "crc32"]);
  assert.equal(levels[0]!.source, null);
  assert.deepEqual(levels[2]!.source, { offsetBits: 32, sizeBits: 576 });
});

test("a field in a stream hangs from the compressed run, marked unpacked", () => {
  const data = node([0, 0, 1, 14], "data", 46, 30, 2);
  const chain = [
    node([], "file", 0, 318, 1),
    node([0], "records", 0, 318, 7),
    node([0, 0], "[0]", 0, 76, 2),
    node([0, 0, 1], "body", 4, 72, 15),
    data,
    node([0, 0, 1, 14, 0], "data", 0, 31, 1, 1),
    node([0, 0, 1, 14, 0, 0], "text", 0, 31, 0, 1),
  ];
  const { levels } = stackPlan(chain);
  assert.deepEqual(levels.map((l) => l.parent.name), ["records", "[0]", "body", "data"]);
  const stream = levels[3]!;
  assert.equal(stream.parent.space, 1);
  assert.equal(stream.unpackedFrom, data);
  // The lens starts at the compressed run's bytes in the strip above.
  assert.deepEqual(stream.source, { offsetBits: 46 * 8, sizeBits: 30 * 8 });
  assert.equal(levels[2]!.unpackedFrom, null);
});

test("a deep path keeps the first strip and the deepest, and counts the rest", () => {
  const chain = Array.from({ length: 12 }, (_, i) => node(Array.from({ length: i }, () => 0), `n${i}`, 0, 100 - i, i === 11 ? 0 : 3));
  const { levels, skipped } = stackPlan(chain, 4);
  assert.equal(levels.length, 4);
  assert.equal(skipped, 7);
  assert.deepEqual(levels.map((l) => l.parent.name), ["n0", "n8", "n9", "n10"]);
  assert.deepEqual(levels[1]!.source, { offsetBits: 0, sizeBits: 92 * 8 });
});

test("a strip's axis takes in a child placed outside its structure", () => {
  const p = node([1], "p", 10, 10, 2);
  assert.deepEqual(levelDomain(p, [node([1, 0], "a", 10, 2, 0), node([1, 1], "b", 40, 4, 0)]), [80, 352]);
  // A child in another space is not on this axis.
  assert.deepEqual(levelDomain(p, [node([1, 0], "a", 0, 400, 0, 1)]), [80, 160]);
});

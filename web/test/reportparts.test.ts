// The report's rules for dividing a file into parts and ordering them. See
// web/src/report/partrules.ts and docs/DESIGN-report-view.md.

import { test } from "node:test";
import assert from "node:assert/strict";

import { gapsOf, readingOrder, runPosition, runsOf, stripIndex, variantText } from "../src/report/partrules.ts";

test("a list element's name loses its place in the list and keeps the rest", () => {
  assert.equal(stripIndex("[3] dqt, quantisation tables"), "dqt, quantisation tables");
  assert.equal(stripIndex("[12] word/document.xml"), "word/document.xml");
  assert.equal(stripIndex("[0]"), "");
  assert.equal(stripIndex("header"), "header");
});

test("a variant is the case's name, without the number an enum adds", () => {
  assert.equal(variantText("local file (0x4034b50)"), "local file");
  assert.equal(variantText("central directory file"), "central directory file");
  assert.equal(variantText("progbits (1)"), "progbits");
});

test("a variant that is only a number names nothing", () => {
  assert.equal(variantText("1130461"), null);
  assert.equal(variantText("0x1f"), null);
  assert.equal(variantText(""), null);
});

test("plain fields next to each other are one run, and a structure breaks it", () => {
  const kids = ["a", "b", "S", "c", "T", "U"];
  const runs = runsOf(kids, (k) => k === k.toLowerCase());
  assert.deepEqual(runs, [["a", "b"], ["S"], ["c"], ["T"], ["U"]]);
});

test("a field placed away from the one before it starts a run of its own", () => {
  const kids = [
    { at: 0, len: 4 },
    { at: 4, len: 4 },
    { at: 100, len: 8 },
  ];
  const runs = runsOf(kids, () => true, (a, b) => a.at + a.len === b.at);
  assert.equal(runs.length, 2);
  assert.equal(runs[0]?.length, 2);
  assert.equal(runs[1]?.[0]?.at, 100);
});

test("a run is a header at the start, a trailer at the end, and fields between", () => {
  assert.equal(runPosition({ offsetBits: 0, sizeBits: 32 }, 800), "start");
  assert.equal(runPosition({ offsetBits: 768, sizeBits: 32 }, 800), "end");
  assert.equal(runPosition({ offsetBits: 64, sizeBits: 32 }, 800), "middle");
});

test("gaps are what no extent covers, including before the first and after the last", () => {
  const gaps = gapsOf(
    [
      { offsetBits: 16, sizeBits: 16 },
      { offsetBits: 48, sizeBits: 16 },
      // Overlapping the one before: counts once.
      { offsetBits: 56, sizeBits: 16 },
    ],
    0,
    96,
  );
  assert.deepEqual(gaps, [
    { offsetBits: 0, sizeBits: 16 },
    { offsetBits: 32, sizeBits: 16 },
    { offsetBits: 72, sizeBits: 24 },
  ]);
});

test("nothing missing is no gaps", () => {
  assert.deepEqual(gapsOf([{ offsetBits: 0, sizeBits: 64 }], 0, 64), []);
});

test("parts with nothing between them go largest first", () => {
  assert.deepEqual(readingOrder([10, 500, 40], [[], [], []]), [1, 2, 0]);
});

test("a header that sizes the chunks comes after them, however small the chunks are", () => {
  // 0: a 12-byte header that sizes 1 and 2; 1 and 2: chunks.
  assert.deepEqual(readingOrder([12, 24, 300000], [[1, 2], [], []]), [2, 1, 0]);
});

test("a directory comes after every table it places, and a loop does not stop the order", () => {
  // 0 places 1, 1 places 0: a loop. Both are still listed, largest first.
  const order = readingOrder([5, 9, 1], [[1], [0], []]);
  assert.equal(order.length, 3);
  assert.deepEqual([...order].sort(), [0, 1, 2]);
  assert.equal(order[0], 2);
});

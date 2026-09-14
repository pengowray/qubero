// What the Diagram view says about the file's counts: the badges, their hover,
// the toolbar's line and button, when the drawing is laid out again, and where
// a double click goes.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { CensusState, DiagramCensus } from "../src/doc.ts";
import {
  AUTO_COUNT,
  boxBadge,
  caseCountTitle,
  countLimit,
  countStatus,
  countTitle,
  goTarget,
  isUnused,
  joinTitle,
  NO_LIMIT,
  onlyUsedTitle,
  REDRAW_MS,
  redrawIn,
  rowBadge,
} from "../src/diagramcounts.ts";

function census(state: CensusState, walked = 100): DiagramCensus {
  return { boxes: [], rows: [], walked, state };
}

test("a finished count badges a box of more than one, and fades a box of none", () => {
  assert.equal(boxBadge(12, "done"), "×12");
  assert.equal(boxBadge(1, "done"), null);
  assert.equal(boxBadge(0, "done"), null);
  assert.equal(isUnused(0, "done"), true);
  assert.equal(isUnused(1, "done"), false);
});

test("an unfinished count badges every box as a floor and fades none", () => {
  for (const state of ["working", "waiting", "capped"] as const) {
    assert.equal(boxBadge(12, state), "×12+");
    assert.equal(boxBadge(1, state), "×1+");
    assert.equal(boxBadge(0, state), "×0+");
    assert.equal(isUnused(0, state), false, `${state} faded a box nothing has been found for yet`);
  }
});

test("a row is badged only where its count differs from its box's", () => {
  // Every field of a journal object is there once per object: no badge.
  assert.equal(rowBadge(156, 156, "done"), null);
  // An optional field, or a case taken by some.
  assert.equal(rowBadge(40, 156, "done"), "×40");
  assert.equal(rowBadge(1, 3, "done"), "×1");
  // None, once finished, is the faded row and not a badge.
  assert.equal(rowBadge(0, 3, "done"), null);
});

test("while counting, a row equal to its box so far is still unbadged, and one that differs is a floor", () => {
  assert.equal(rowBadge(300, 300, "working"), null);
  assert.equal(rowBadge(20, 300, "working"), "×20+");
  assert.equal(rowBadge(0, 300, "capped"), "×0+");
});

test("the hover says at least, and never none, until the count is finished", () => {
  assert.equal(countTitle(12, "Chunk", census("done")), "This file has 12 of these: Chunk");
  assert.equal(countTitle(1, "IHDR", census("done")), "This file has 1 IHDR");
  assert.equal(countTitle(0, "IEND", census("done")), "This file has none of these");
  assert.equal(countTitle(12, "Chunk", census("working")), "This file has at least 12 of these: Chunk. Still counting.");
  assert.equal(countTitle(0, "IEND", census("waiting")), "None of these found yet. Still counting.");
  assert.equal(
    countTitle(12, "Chunk", census("capped", 200000)),
    `This file has at least 12 of these: Chunk. Only the first ${(200000).toLocaleString()} fields are counted.`,
  );
  assert.equal(
    countTitle(0, "IEND", census("capped", 200000)),
    `None of these in the first ${(200000).toLocaleString()} fields. The rest of this file is not counted.`,
  );
});

test("a switch's row is a case, and its hover says how often it was taken", () => {
  assert.equal(caseCountTitle("1", 1, census("done")), "Case 1 taken once in this file");
  assert.equal(caseCountTitle("'IHDR'", 7, census("done")), "Case 'IHDR' taken 7 times in this file");
  assert.equal(caseCountTitle("1", 7, census("working")), "Case 1 taken at least 7 times in this file. Still counting.");
});

test("the toolbar says counting, stopped with a button, or nothing", () => {
  assert.deepEqual(countStatus(null, false), { text: "", keep: false });
  assert.deepEqual(countStatus(null, true), { text: "Counting…", keep: false });
  assert.deepEqual(countStatus(census("working", 5000), true), { text: `Counting… ${(5000).toLocaleString()} fields so far`, keep: false });
  assert.deepEqual(countStatus(census("waiting", 5000), true), { text: `Counting… ${(5000).toLocaleString()} fields so far`, keep: false });
  assert.deepEqual(countStatus(census("capped", 200000), false), {
    text: `Counted the first ${(200000).toLocaleString()} fields of this file.`,
    keep: true,
  });
  assert.deepEqual(countStatus(census("done"), false), { text: "", keep: false });
});

test("the toggle's hover owns up to hiding what has not been found yet", () => {
  assert.equal(onlyUsedTitle(census("done")), "Leave out the types not found in this file, and lay the rest out again.");
  assert.equal(
    onlyUsedTitle(census("working")),
    "Leave out the types not found in this file, and lay the rest out again. Types not found yet are left out too.",
  );
});

test("a small file is counted whole and a large one stops until asked", () => {
  assert.equal(countLimit(6_000, false), NO_LIMIT);
  assert.equal(countLimit(AUTO_COUNT.wholeFileUnderBytes - 1, false), NO_LIMIT);
  assert.equal(countLimit(AUTO_COUNT.wholeFileUnderBytes, false), AUTO_COUNT.fieldsBeforeAsking);
  assert.equal(countLimit(AUTO_COUNT.wholeFileUnderBytes * 100, true), NO_LIMIT);
});

test("a finished or stopped count is drawn at once", () => {
  assert.equal(redrawIn(null, census("done"), 50, 0, 0), 0);
  assert.equal(redrawIn(census("working", 10), census("capped", 20), 50, 40, 0), 0);
});

test("the first drawing of a running count waits a whole interval from when counting began", () => {
  // A small file that finishes inside the interval never draws its `×0+`.
  assert.equal(redrawIn(null, census("working", 10), 300, 0, 100), REDRAW_MS - 200);
  assert.equal(redrawIn(null, census("working", 10), 100 + REDRAW_MS + 5, 0, 100), 0);
});

test("a running count redraws at most once an interval", () => {
  assert.equal(redrawIn(census("working", 10), census("working", 20), 1500, 1000, 0), REDRAW_MS - 500);
  assert.equal(redrawIn(census("working", 10), census("working", 20), 2500, 1000, 0), 0);
});

test("nothing is laid out again when nothing a reader sees has changed", () => {
  assert.equal(redrawIn(census("done", 117), census("done", 117), 5000, 0, 0), null);
  // Waiting for bytes is still counting.
  assert.equal(redrawIn(census("working", 10), census("waiting", 10), 5000, 0, 0), null);
  assert.equal(redrawIn(null, null, 5000, 0, 0), null);
  assert.equal(redrawIn(census("done"), null, 5000, 0, 0), 0);
});

test("a count taken again after an edit is drawn even when it walked as far", () => {
  const box = (count: number) => ({ key: "chunk", count, first_path: [0], space: 0 });
  const row = (count: number) => ({ key: "chunk", row: 1, count, first_path: [0, 1], space: 0 });
  const before: DiagramCensus = { boxes: [box(10)], rows: [row(2)], walked: 117, state: "done" };
  assert.equal(redrawIn(before, { ...before, boxes: [box(10)], rows: [row(2)] }, 5000, 0, 0), null);
  assert.equal(redrawIn(before, { ...before, rows: [row(3)] }, 5000, 0, 0), 0);
  assert.equal(redrawIn(before, { ...before, boxes: [box(9)] }, 5000, 0, 0), 0);
  assert.equal(redrawIn(before, { ...before, boxes: [{ ...box(10), first_path: [2] }] }, 5000, 0, 0), 0);
});

test("a double click goes to the field itself, the first one counted, or nowhere", () => {
  assert.deepEqual(goTarget(true, { first_path: [1, 2], space: 0 }), { kind: "pick" });
  assert.deepEqual(goTarget(true, undefined), { kind: "pick" });
  assert.deepEqual(goTarget(false, { first_path: [1, 2], space: 0 }), { kind: "path", path: [1, 2] });
  assert.deepEqual(goTarget(false, { first_path: [0], space: 3 }), { kind: "stream" });
  assert.equal(goTarget(false, undefined), null);
});

test("hover sentences are joined with one full stop between them", () => {
  assert.equal(joinTitle("This file has 3 of these: x", "Double-click to go"), "This file has 3 of these: x. Double-click to go");
  assert.equal(joinTitle("None of these found yet. Still counting.", "Double-click to go"), "None of these found yet. Still counting. Double-click to go");
  assert.equal(joinTitle("", "Double-click to go"), "Double-click to go");
  assert.equal(joinTitle("Field: x", null, undefined), "Field: x");
});

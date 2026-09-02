// What the line index knows, what it only guesses, and what an edit takes
// away from it.

import { test } from "node:test";
import assert from "node:assert/strict";

import { GUESS_LINE, LineIndex, before } from "../src/lineindex.ts";

const none = { lf: 0, cr: 0, crlf: 0 };

/** An index over a file of fixed-length lines, scanned as far as `to`. */
function fixed(lineLen: number, lines: number, scanned = lines): LineIndex {
  const idx = new LineIndex(lineLen * lines);
  const starts = new Float64Array(scanned);
  for (let i = 0; i < scanned; i++) starts[i] = i * lineLen;
  idx.add(starts, scanned * lineLen, { lf: scanned, cr: 0, crlf: 0 });
  return idx;
}

test("a scan from the front gives every line it covered a number", () => {
  const idx = fixed(10, 100);
  assert.equal(idx.knownLines, 100);
  assert.equal(idx.lineAt(0), 0);
  assert.equal(idx.lineAt(9), 0);
  assert.equal(idx.lineAt(10), 1);
  assert.equal(idx.lineAt(995), 99);
  assert.equal(idx.byteOfLine(50), 500);
  assert.equal(idx.totalLines, 100);
  assert.equal(idx.exact, true);
});

test("past the scan a byte has no line number and a guess instead", () => {
  const idx = fixed(10, 100, 40);
  assert.equal(idx.indexedTo, 400);
  assert.equal(idx.complete, false);
  assert.equal(idx.exact, false);
  assert.equal(idx.lineAt(500), null);
  assert.equal(idx.byteOfLine(60), null);
  // Ten bytes a line over the scanned part, so the rest is sixty lines more.
  assert.equal(idx.totalLines, 100);
  assert.equal(idx.guessLineAt(500), 50);
  assert.equal(idx.guessByteOfLine(50), 500);
});

test("nothing scanned is a whole file of estimate", () => {
  const idx = new LineIndex(GUESS_LINE * 12);
  assert.equal(idx.knownLines, 0);
  assert.equal(idx.totalLines, 12);
  assert.equal(idx.gap, 0);
});

test("a scan carries on where the last one stopped", () => {
  const idx = new LineIndex(300);
  idx.add(new Float64Array([0, 10, 20]), 30, { lf: 3, cr: 0, crlf: 0 });
  assert.equal(idx.gap, 30);
  idx.add(new Float64Array([30, 40]), 50, { lf: 2, cr: 0, crlf: 0 });
  assert.equal(idx.knownLines, 5);
  assert.equal(idx.lineAt(45), 4);
  assert.deepEqual(idx.endings, { lf: 5, cr: 0, crlf: 0 });
  // Two segments, never welded: an edit is what they are kept apart for.
  assert.equal(idx.segments.length, 2);
});

test("a segment away from the front knows its lines but not their numbers", () => {
  const idx = new LineIndex(1000);
  idx.add(new Float64Array([500, 520, 540]), 560, { lf: 3, cr: 0, crlf: 0 });
  assert.equal(idx.lineAt(530), null);
  const p = idx.place(530);
  assert.equal(p?.at, 520);
  assert.equal(p?.index, 1);
  assert.equal(p?.line, null);
  // It still counts towards what the file's usual ending is.
  assert.deepEqual(idx.endings, { lf: 3, cr: 0, crlf: 0 });
  assert.equal(idx.indexedTo, 0);
});

test("the chain from the front picks up a detached segment it runs into", () => {
  const idx = new LineIndex(1000);
  idx.add(new Float64Array([500, 520, 540]), 560, { lf: 3, cr: 0, crlf: 0 });
  const head = new Float64Array(50);
  for (let i = 0; i < 50; i++) head[i] = i * 10;
  idx.add(head, 500, { lf: 50, cr: 0, crlf: 0 });
  assert.equal(idx.indexedTo, 560);
  assert.equal(idx.lineAt(530), 51);
  assert.equal(idx.byteOfLine(52), 540);
});

test("a detached segment the chain does not line up with is dropped", () => {
  const idx = new LineIndex(1000);
  // Landed mid-line, as a scan started in the middle of a line too long to
  // have an ending in it can.
  idx.add(new Float64Array([505, 525]), 545, { lf: 2, cr: 0, crlf: 0 });
  const head = new Float64Array(51);
  for (let i = 0; i < 51; i++) head[i] = i * 10;
  idx.add(head, 510, { lf: 51, cr: 0, crlf: 0 });
  assert.equal(idx.indexedTo, 510);
  assert.equal(idx.segments.length, 1);
  assert.deepEqual(idx.endings, { lf: 51, cr: 0, crlf: 0 });
});

test("a scan over ground already covered from nearer the front is ignored", () => {
  const idx = fixed(10, 100, 40);
  idx.add(new Float64Array([200, 213]), 226, { lf: 2, cr: 0, crlf: 0 });
  assert.equal(idx.byteOfLine(21), 210);
  assert.equal(idx.segments.length, 1);
});

test("an edit takes back the line it landed on and everything after it", () => {
  const idx = new LineIndex(300);
  idx.add(new Float64Array([0, 10, 20]), 30, { lf: 3, cr: 0, crlf: 0 });
  idx.add(new Float64Array([30, 40, 50]), 60, { lf: 3, cr: 0, crlf: 0 });
  idx.dropFrom(35);
  assert.equal(idx.indexedTo, 30);
  assert.equal(idx.knownLines, 3);
  assert.deepEqual(idx.endings, { lf: 3, cr: 0, crlf: 0 });
});

test("typing inside a line moves the lines after it rather than forgetting them", () => {
  const idx = fixed(10, 10);
  idx.shiftFrom(23, 1);
  assert.equal(idx.lengthBytes, 101);
  assert.equal(idx.byteOfLine(2), 20);
  assert.equal(idx.byteOfLine(3), 31);
  assert.equal(idx.knownLines, 10);
  assert.equal(idx.lineAt(31), 3);
});

test("a change of encoding is a different file's lines", () => {
  const idx = fixed(10, 100);
  idx.clear();
  assert.equal(idx.knownLines, 0);
  assert.equal(idx.gap, 0);
  assert.deepEqual(idx.endings, none);
});

test("the search lands on the last value at or before the one wanted", () => {
  const a = new Float64Array([0, 10, 20, 30]);
  assert.equal(before(a, 0), 0);
  assert.equal(before(a, 9), 0);
  assert.equal(before(a, 10), 1);
  assert.equal(before(a, 999), 3);
});

// A PNG's passes and filters as the report draws them. See
// web/src/report/pngpasses.ts and web/src/report/pngscanlines.ts.

import { test } from "node:test";
import assert from "node:assert/strict";

import { adam7Passes, filterCounts, passOf, pictureRow, type Scanline } from "../src/report/pngpasses.ts";
import { RV } from "../src/report/text.ts";

test("a 32 by 32 picture's seven passes are the sizes the notes on its report give", () => {
  const shapes = adam7Passes(32, 32).map((p) => [p.cols, p.rows]);
  assert.deepEqual(shapes, [[4, 4], [4, 4], [8, 4], [8, 8], [16, 8], [16, 16], [32, 16]]);
});

test("a small picture leaves passes with no pixels", () => {
  const shapes = adam7Passes(3, 2).map((p) => [p.cols, p.rows]);
  assert.deepEqual(shapes, [[1, 1], [0, 1], [1, 0], [1, 1], [2, 0], [1, 1], [3, 1]]);
});

test("every pixel of a tile is in the pass whose grid it is on", () => {
  const passes = adam7Passes(8, 8);
  for (let y = 0; y < 8; y++) {
    for (let x = 0; x < 8; x++) {
      const p = passes[passOf(x, y) - 1];
      assert.ok(p !== undefined);
      assert.equal((x - p.x0) % p.dx, 0, `${x},${y}`);
      assert.equal((y - p.y0) % p.dy, 0, `${x},${y}`);
    }
  }
  // Counted over the tile, each pass holds as many pixels as its shape says.
  const count = [0, 0, 0, 0, 0, 0, 0];
  for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) count[passOf(x, y) - 1]++;
  assert.deepEqual(count, passes.map((p) => p.cols * p.rows));
});

test("a scanline of a pass is a row of the picture a step apart", () => {
  const line = (pass: number | null, row: number): Scanline => ({ pass, row, filter: 4, bytes: 257, path: [] });
  assert.equal(pictureRow(line(7, 4)), 9);
  assert.equal(pictureRow(line(3, 1)), 12);
  assert.equal(pictureRow(line(null, 12)), 12);
});

test("the heading and the counts say how many use which filter, most first", () => {
  const lines = [4, 4, 4, 1, 0].map((filter, row): Scanline => ({ pass: 1, row, filter, bytes: 9, path: [] }));
  const counts = filterCounts(lines);
  assert.deepEqual(counts, [1, 1, 0, 0, 3]);
  assert.equal(RV.pngScanlinesHeading(5, true, "Paeth", 3, true), "5 scanlines in seven passes, 3 of them filtered with Paeth");
  assert.equal(RV.pngScanlinesHeading(32, false, "None", 32, true), "32 scanlines, all filtered with None");
  assert.equal(RV.pngScanlinesHeading(4000, false, "Up", 2000, false), "4,000 scanlines");
  assert.equal(
    RV.pngFilterCounts([["Paeth", 36], ["Sub", 11], ["Up", 8], ["None", 4], ["Average", 1]], null),
    "This picture uses Paeth on 36 scanlines, Sub on 11, Up on 8, None on 4, and Average on 1.",
  );
  assert.equal(RV.pngFilterCounts([["None", 32]], null), "Every scanline uses None.");
});

// The map's geometry: every strip has the same segments in the same places, so
// the widths are worked out from the file's parts alone and have to hold for
// the awkward shapes real files come in.

import { test } from "node:test";
import assert from "node:assert/strict";

import { segmentWidths } from "../src/filemap.ts";
import type { MapSegment } from "../src/filemap.ts";

const seg = (bytes: number): MapSegment => ({ offsetBits: 0, sizeBits: bytes * 8, color: "#000" });

/** Total drawn width, gaps included, which must come to the strip's own. */
function spans(widths: readonly number[], width = 96): number {
  return widths.reduce((a, b) => a + b, 0) + (widths.length - 1);
}

test("parts share the width in proportion to their bytes", () => {
  const w = segmentWidths([seg(100), seg(100), seg(200)]);
  assert.ok(Math.abs(w[0]! - w[1]!) < 0.01);
  assert.ok(Math.abs(w[2]! - w[0]! * 2) < 0.01);
  assert.ok(Math.abs(spans(w) - 96) < 0.01);
});

test("a part too small to see is still drawn", () => {
  // notes.sqlite: a hundred bytes of header and three four-kilobyte pages.
  const w = segmentWidths([seg(100), seg(4096), seg(4096), seg(4096)]);
  assert.ok(w[0]! >= 2, `header drawn ${w[0]}px`);
  assert.ok(Math.abs(spans(w) - 96) < 0.01);
  // The pages keep their own proportions to each other.
  assert.ok(Math.abs(w[1]! - w[2]!) < 0.01);
});

test("the widening comes out of the parts that can afford it", () => {
  const w = segmentWidths([seg(1), seg(1), seg(1_000_000)]);
  assert.ok(w[0]! >= 2 && w[1]! >= 2);
  assert.ok(w[2]! > 80, `the big part keeps the rest: ${w[2]}px`);
  assert.ok(Math.abs(spans(w) - 96) < 0.01);
});

test("more parts than pixels share what there is", () => {
  const many = Array.from({ length: 60 }, () => seg(1));
  const w = segmentWidths(many);
  assert.equal(w.length, 60);
  assert.ok(w.every((x) => x > 0));
  assert.ok(Math.abs(spans(w) - 96) < 0.5, `drawn ${spans(w)}px`);
});

test("a file of no bytes still divides evenly", () => {
  const w = segmentWidths([seg(0), seg(0)]);
  assert.ok(w.every((x) => x > 0));
  assert.equal(segmentWidths([]).length, 0);
});

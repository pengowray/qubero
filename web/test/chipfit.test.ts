// How a row's chips lay out in the field column: they wrap onto further lines
// of the row before any of them is counted rather than shown.

import { test } from "node:test";
import assert from "node:assert/strict";

import { chipLayout, chipWidth } from "../src/chipfit.ts";

test("chips that fit on one line stay on one line", () => {
  assert.deepEqual(chipLayout([100, 100, 100], 320), { shown: 3, lines: 1 });
});

test("a chip that does not fit beside the last one starts the next line", () => {
  assert.deepEqual(chipLayout([200, 200, 200], 320), { shown: 3, lines: 3 });
});

test("past the last line the rest is counted, with room kept for the count", () => {
  // Seven chips of 150 in a 320 column: two per line, so six fit in three
  // lines; the sixth would leave no room for the count and gives way to it.
  assert.deepEqual(chipLayout([150, 150, 150, 150, 150, 150, 150], 320), { shown: 5, lines: 3 });
});

test("the last chip needs no room kept after it", () => {
  assert.deepEqual(chipLayout([150, 150, 150, 150, 150, 150], 320), { shown: 6, lines: 3 });
});

test("the first chip on a line is drawn even when it is too wide", () => {
  assert.deepEqual(chipLayout([500], 320), { shown: 1, lines: 1 });
  assert.deepEqual(chipLayout([500, 500], 320), { shown: 2, lines: 2 });
});

test("no chips take no lines", () => {
  assert.deepEqual(chipLayout([], 320), { shown: 0, lines: 0 });
});

test("a line limit of one is the old behaviour", () => {
  assert.deepEqual(chipLayout([200, 200, 200], 320, 1), { shown: 1, lines: 1 });
});

test("a chip is never measured wider than it is drawn", () => {
  assert.equal(chipWidth("a".repeat(80), ""), chipWidth("a".repeat(26), ""));
});

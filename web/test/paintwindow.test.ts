import { test } from "node:test";
import assert from "node:assert/strict";

import { moveRows, needsPaint, paintWindow } from "../src/paintwindow.ts";

test("compressed scrolling moves by full-height rows and keeps the pixel remainder", () => {
  assert.deepEqual(moveRows(100, 0, 55, 20, 1000), { line: 102, offset: 15 });
  assert.deepEqual(moveRows(102, 15, -18, 20, 1000), { line: 101, offset: 17 });
});

test("compressed scrolling stops cleanly at both file ends", () => {
  assert.deepEqual(moveRows(2, 0, -100, 20, 1000), { line: 0, offset: 0 });
  assert.deepEqual(moveRows(998, 10, 100, 20, 1000), { line: 1000, offset: 0 });
});

test("the DOM keeps eight full viewports painted on either side", () => {
  assert.deepEqual(paintWindow(200, 20), { first: 40, count: 340, runway: 160 });
  // At the front, unused upper runway remains available below the viewport.
  assert.deepEqual(paintWindow(10, 20), { first: 0, count: 340, runway: 160 });
});

test("a half-page and a whole page scroll use rows already painted", () => {
  const w = paintWindow(200, 20);
  assert.equal(needsPaint(210, 20, w.first, w.count, w.runway, false, false), false);
  assert.equal(needsPaint(220, 20, w.first, w.count, w.runway, false, false), false);
});

test("the painted window refills with a viewport still in reserve", () => {
  const w = paintWindow(200, 20);
  assert.equal(needsPaint(281, 20, w.first, w.count, w.runway, false, false), true);
  assert.equal(needsPaint(119, 20, w.first, w.count, w.runway, false, false), true);
});

test("a real file edge does not cause repeated refills", () => {
  const w = paintWindow(200, 20);
  const front = paintWindow(0, 20);
  assert.equal(needsPaint(359, 20, w.first, w.count, w.runway, false, true), false);
  assert.equal(needsPaint(0, 20, front.first, front.count, front.runway, true, false), false);
});

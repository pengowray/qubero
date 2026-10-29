// The board's `hold` limits (`holdInView` in viz/boardmath.ts): a picture
// panned or zoomed as far as it goes still has some of itself on screen, at
// every zoom, and a picture narrower than the view is left where it is drawn.

import { test } from "node:test";
import assert from "node:assert/strict";

import { holdEdge, holdInView } from "../src/viz/boardmath.ts";

test("a picture narrower than the view stays where it is drawn", () => {
  // 186 pixels of tiles at the left of a 1249 pixel figure, at no zoom.
  assert.equal(holdEdge(0, 186, 1249, 1249 / 2), 0);
  // Dragged left, it stops at its own place; dragged right, it stops in the
  // middle of the view.
  assert.equal(holdEdge(-500, 186, 1249, 1249 / 2), 0);
  assert.equal(holdEdge(2000, 186, 1249, 1249 / 2), 1249 / 2);
});

test("a picture wider than the view may be pulled until its edge reaches the slack", () => {
  // 1060 layout pixels of tiles zoomed 5 times: 5300 on screen in a 1249 view.
  const slack = 1249 / 2;
  assert.equal(holdEdge(slack + 100, 5300, 1249, slack), slack);
  assert.equal(holdEdge(-99999, 5300, 1249, slack), 1249 - slack - 5300);
  assert.equal(holdEdge(-1000, 5300, 1249, slack), -1000);
});

test("no zoom and pan leaves the view empty", () => {
  const content = { x0: 0, y0: 0, x1: 186, y1: 34 };
  const width = 1249;
  const height = 34;
  for (const k of [1, 2, 4, 5.7, 8]) {
    for (const x of [-1e6, -5000, -500, 0, 500, 5000, 1e6]) {
      const v = holdInView({ k, x, y: 0 }, content, width, height, 0.5, true);
      const left = content.x0 * v.k + v.x;
      const right = content.x1 * v.k + v.x;
      assert.ok(right > 0 && left < width, `k=${k} x=${x}: tiles from ${left} to ${right}`);
      assert.equal(v.y, 0);
    }
  }
});

test("both ways, each axis is held on its own", () => {
  const content = { x0: 0, y0: 0, x1: 1200, y1: 112 };
  const v = holdInView({ k: 8, x: 1e6, y: -1e6 }, content, 1249, 118, 0.5, false);
  assert.equal(v.x, 1249 / 2);
  assert.equal(v.y, 118 - 59 - 112 * 8);
});

test("a fitted picture, centred, is inside the limits", () => {
  // Centred with room either side is a position the limits allow, so a Fit is
  // not moved again by them.
  const size = 400;
  const centred = (1249 - size) / 2;
  assert.equal(holdEdge(centred, size, 1249, 1249 / 2), centred);
});

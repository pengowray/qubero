// The ribbon's geometry: rows scaled to the width, boxes never too thin to
// point at, and a band's outline from one extent to the other. See
// web/src/report/ribbon.ts.

import { test } from "node:test";
import assert from "node:assert/strict";

import { bandPath, boxSpans, scaleOf } from "../src/report/ribbon.ts";

test("a row's stretch is drawn across the whole width", () => {
  const x = scaleOf(100, 200, 0, 1000);
  assert.equal(x(100), 0);
  assert.equal(x(150), 500);
  assert.equal(x(200), 1000);
});

test("an empty stretch draws at the left edge rather than dividing by zero", () => {
  const x = scaleOf(5, 5, 10, 20);
  assert.equal(x(5), 10);
  assert.equal(x(99), 10);
});

test("each row scales on its own, so a band can be narrow at one end and wide at the other", () => {
  // Eight bits of code at the top write 40 bytes at the bottom: in a window of
  // 80 bits and 80 bytes, the top box is a tenth of the width and the bottom
  // one half of it.
  const top = boxSpans({ from: 0, to: 80, caption: "", boxes: [{ from: 0, to: 8 }] }, 1000);
  const bottom = boxSpans({ from: 0, to: 80, caption: "", boxes: [{ from: 0, to: 40 }] }, 1000);
  assert.equal(top[0]?.w, 100);
  assert.equal(bottom[0]?.w, 500);
});

test("a box too thin to see is drawn wide enough to point at", () => {
  const spans = boxSpans({ from: 0, to: 100000, caption: "", boxes: [{ from: 10, to: 11 }] }, 100);
  assert.ok((spans[0]?.w ?? 0) >= 1.5);
});

test("a band's outline runs down one edge, along the bottom, and up the other", () => {
  const d = bandPath(10, 20, 0, 100, 300, 50);
  assert.ok(d.startsWith("M10 0"), d);
  assert.ok(d.includes("L300 50"), d);
  assert.ok(d.endsWith("20 0 Z"), d);
  // Both curves leave their row straight down: the first control point of
  // each edge is right under or over its end.
  assert.ok(d.includes("C10 25, 100 25, 100 50"), d);
  assert.ok(d.includes("C300 25, 20 25, 20 0"), d);
});

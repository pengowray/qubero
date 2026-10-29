// The stride raster's arithmetic (`viz/stridemath.ts`) and the periodicity
// line behind it (`viz/periodicity.ts`): the two zooms, what a pixel shows,
// the row width a template suggests, the FFT against a direct sum, and the
// peak that comes out on record-shaped bytes.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  byteKind,
  cellWidth,
  clampStride,
  clampTopRow,
  columnTicks,
  followZoom,
  greyLevel,
  MAX_CELL_PX,
  MAX_STRIDE,
  maxZoom,
  OTHER,
  pixelColumns,
  pixelValue,
  rangeOutline,
  rasterGeometry,
  rasterHeight,
  rowAt,
  rowHeight,
  ROW_MAX_PX,
  strideRows,
  templateStride,
  TEXT,
  ZERO,
} from "../src/viz/stridemath.ts";
import { autocorrelation, commonStep, fft, findPeaks, noiseFloor, periodicity } from "../src/viz/periodicity.ts";

// ----- rows and the two zooms -----

test("rows round up, and the last row may be short", () => {
  assert.equal(strideRows(51744, 2352), 22);
  assert.equal(strideRows(10, 4), 3);
  assert.equal(strideRows(0, 16), 0);
});

test("a stride is a whole number of bytes within the limits", () => {
  assert.equal(clampStride(0), 1);
  assert.equal(clampStride(2351.6), 2352);
  assert.equal(clampStride(1e9), MAX_STRIDE);
  assert.equal(clampStride(Number.NaN), 1);
});

test("a CD image's rows fill the view and keep their height until the cells are square", () => {
  const g = rasterGeometry(51744, 2352, 175, 352);
  assert.equal(g.rows, 22);
  assert.equal(g.rowBase, ROW_MAX_PX);
  assert.equal(rowHeight(g, 1), ROW_MAX_PX);
  // Ten times wider cells are still under a pixel: the rows stay put.
  assert.equal(rowHeight(g, 10), ROW_MAX_PX);
  // Past 16 pixels a cell, rows follow the cells.
  const k = 20 / g.fitX;
  assert.ok(Math.abs(rowHeight(g, k) - 20) < 1e-9);
  assert.ok(Math.abs(cellWidth(g, maxZoom(g)) - MAX_CELL_PX) < 1e-9);
});

test("a long file's rows are squeezed to fit, and meet the cells at square", () => {
  const g = rasterGeometry(100 * 1024 * 1024, 16, 175, 400);
  assert.ok(g.rowBase < 0.001);
  assert.ok(Math.abs(rowHeight(g, 1) - g.rowBase) < 1e-12);
  let last = 0;
  for (let k = 1; k <= maxZoom(g); k *= 1.1) {
    const h = rowHeight(g, k);
    assert.ok(h >= last, `row height falls at zoom ${k}`);
    assert.ok(h <= cellWidth(g, k) + 1e-9, "a row is never taller than a cell is wide here");
    last = h;
  }
  const kSq = Math.max(2, ROW_MAX_PX / g.fitX);
  assert.ok(Math.abs(rowHeight(g, kSq) - cellWidth(g, kSq)) < 1e-9);
  assert.ok(maxZoom(g) >= kSq);
});

test("the raster asks for every row at full height, within its limits", () => {
  assert.equal(rasterHeight(22, 120, 400), 352);
  assert.equal(rasterHeight(2, 120, 400), 120);
  assert.equal(rasterHeight(5000, 120, 400), 400);
});

test("a zoom keeps the row under the pointer under the pointer", () => {
  const g = rasterGeometry(100 * 1024 * 1024, 16, 175, 400);
  const prev = { k: 2, x: -100, y: -50 };
  // d3-zoom zooming by 1.5 about (60, 120): (p - t) / k is kept.
  const k = 3;
  const next = { k, x: 60 - ((60 - prev.x) / prev.k) * k, y: 120 - ((120 - prev.y) / prev.k) * k };
  const top = 1000;
  const before = top + 120 / rowHeight(g, prev.k);
  const after = followZoom(g, top, prev, next);
  assert.ok(Math.abs(after + 120 / rowHeight(g, next.k) - before) < 1e-6);
});

test("a pan moves the rows by the pixels dragged, and stops at the ends", () => {
  const g = rasterGeometry(100 * 1024 * 1024, 16, 175, 400);
  const v = { k: 3, x: 0, y: 0 };
  const h = rowHeight(g, 3);
  assert.ok(Math.abs(followZoom(g, 100, v, { ...v, y: -40 }) - (100 + 40 / h)) < 1e-9);
  assert.equal(followZoom(g, 0, v, { ...v, y: 500 }), 0);
  assert.equal(clampTopRow(g, h, 1e12), g.rows - g.height / h);
  // Every row fits: nothing can be above the view.
  const small = rasterGeometry(51744, 2352, 175, 352);
  assert.equal(clampTopRow(small, ROW_MAX_PX, 5), 0);
});

test("the row under a point, and nothing below the last", () => {
  const g = rasterGeometry(51744, 2352, 175, 400);
  assert.equal(rowAt(g, 16, 0, 0), 0);
  assert.equal(rowAt(g, 16, 0, 17), 1);
  assert.equal(rowAt(g, 16, 2, 0), 2);
  assert.equal(rowAt(g, 16, 0, 16 * 22 + 1), -1);
});

// ----- what a pixel shows -----

test("a pixel covers the bytes under it, and neighbouring pixels share a wide byte", () => {
  const g = rasterGeometry(1000, 100, 50, 100);
  const view = { k: 1, x: 0, y: 0 };
  const { from, to } = pixelColumns(g, view, 50, 1);
  assert.equal(from[0], 0);
  assert.equal(to[0], 2);
  assert.equal(from[49], 98);
  assert.equal(to[49], 100);
  const zoomed = pixelColumns(g, { k: 4, x: 0, y: 0 }, 4, 1);
  assert.deepEqual([...zoomed.from], [0, 0, 1, 1]);
  assert.deepEqual([...zoomed.to], [1, 1, 2, 2]);
  // Off the end of the row: nothing.
  const off = pixelColumns(g, { k: 1, x: -40, y: 0 }, 50, 1);
  assert.equal(off.from[49], off.to[49]);
});

test("a pixel shows its largest byte and its most notable kind", () => {
  // The CD sync pattern's first eight bytes, sharing one pixel.
  const bytes = Uint8Array.from([0x00, 0xff, 0xff, 0x41, 0, 0, 0, 0]);
  assert.deepEqual(pixelValue(bytes, 0, 8), { max: 0xff, kind: OTHER });
  assert.deepEqual(pixelValue(bytes, 4, 8), { max: 0, kind: ZERO });
  assert.deepEqual(pixelValue(Uint8Array.from([0x41, 0]), 0, 2), { max: 0x41, kind: TEXT });
  assert.equal(byteKind(10), TEXT);
  assert.equal(byteKind(0x7f), OTHER);
});

test("zero is the background grey in either theme, and 0xff is far from it", () => {
  assert.equal(greyLevel(0, false, 255), 255);
  assert.equal(greyLevel(255, false, 255), 20);
  assert.equal(greyLevel(0, true, 23), 23);
  assert.equal(greyLevel(255, true, 23), 240);
});

test("a range in one row is a box, and one across rows is one outline", () => {
  assert.deepEqual(rangeOutline(16, 18, 20), [
    { col: 2, row: 1 },
    { col: 4, row: 1 },
    { col: 4, row: 2 },
    { col: 2, row: 2 },
  ]);
  const across = rangeOutline(16, 10, 40);
  assert.equal(across.length, 8);
  assert.deepEqual(across[0], { col: 10, row: 0 });
  assert.deepEqual(across[3], { col: 8, row: 2 });
  assert.deepEqual(rangeOutline(16, 5, 5), []);
});

// ----- the row width a template suggests -----

test("the widest run of equal records gives the row width", () => {
  assert.equal(templateStride([{ sizes: [2352 * 8, 2352 * 8, 2352 * 8], bits: 22 * 2352 * 8 }]), 2352);
  assert.equal(
    templateStride([
      { sizes: [512 * 8, 512 * 8], bits: 1024 * 8 },
      { sizes: [64 * 8, 64 * 8, 64 * 8], bits: 640 * 8 },
    ]),
    512,
  );
  // Unequal records, one record, sub-byte or tiny records: nothing.
  assert.equal(templateStride([{ sizes: [512 * 8, 1024 * 8], bits: 1536 * 8 }]), null);
  assert.equal(templateStride([{ sizes: [512 * 8], bits: 512 * 8 }]), null);
  assert.equal(templateStride([{ sizes: [12, 12], bits: 24 }]), null);
  assert.equal(templateStride([{ sizes: [16, 16, 16], bits: 48 }]), null);
});

test("column ticks keep inside the row and clear of each other", () => {
  const ticks = columnTicks([0, 500, 1000, 1500, 2000], (c) => (c / 2352) * 175, 175, () => 24);
  assert.equal(ticks[0]?.anchor, "start");
  for (let i = 1; i < ticks.length; i++) {
    const a = ticks[i - 1];
    const b = ticks[i];
    assert.ok(a !== undefined && b !== undefined);
    assert.ok(b.value > a.value);
  }
  // 37 pixels apart, labels 24 wide with a 6 pixel gap: every other one goes.
  assert.ok(ticks.length < 5);
  assert.ok(ticks.length >= 2);
});

// ----- periodicity -----

function directAutocorrelation(bytes: Uint8Array, lag: number): number {
  const n = bytes.length;
  let mean = 0;
  for (const b of bytes) mean += b;
  mean /= n;
  let zero = 0;
  for (const b of bytes) zero += (b - mean) ** 2;
  zero /= n;
  let s = 0;
  for (let i = 0; i + lag < n; i++) s += ((bytes[i] as number) - mean) * ((bytes[i + lag] as number) - mean);
  return s / (n - lag) / zero;
}

test("the FFT turns one way and back again", () => {
  const re = Float64Array.from([1, 2, 3, 4, 0, -1, 5, 2]);
  const im = new Float64Array(8);
  const copy = Float64Array.from(re);
  fft(re, im);
  fft(re, im, true);
  for (let i = 0; i < 8; i++) assert.ok(Math.abs((re[i] as number) / 8 - (copy[i] as number)) < 1e-9);
});

test("the FFT autocorrelation matches the direct sum", () => {
  const bytes = new Uint8Array(300);
  let x = 12345;
  for (let i = 0; i < bytes.length; i++) {
    x = (x * 1103515245 + 12345) >>> 0;
    bytes[i] = (x >>> 16) & 0xff;
  }
  const r = autocorrelation(bytes, 50);
  assert.equal(r.length, 51);
  assert.ok(Math.abs((r[0] as number) - 1) < 1e-6);
  for (const lag of [1, 7, 33, 50]) assert.ok(Math.abs((r[lag] as number) - directAutocorrelation(bytes, lag)) < 1e-4, `lag ${lag}`);
});

test("bytes that never change have no autocorrelation and no peaks", () => {
  const r = autocorrelation(new Uint8Array(100).fill(7), 20);
  assert.ok(r.every((v) => v === 0));
  assert.deepEqual(findPeaks(r), []);
});

test("records of 7 bytes give a peak at 7, not at 14", () => {
  const bytes = new Uint8Array(7 * 400);
  let x = 99;
  for (let i = 0; i < bytes.length; i++) {
    x = (x * 1103515245 + 12345) >>> 0;
    // A fixed marker byte and noise around it.
    bytes[i] = i % 7 === 0 ? 0xff : (x >>> 16) & 0x3f;
  }
  const { peaks } = periodicity({ id: 1, bytes, maxLag: 100 });
  assert.equal(peaks[0]?.lag, 7);
  // 14, 21 and the rest are multiples and are left out.
  assert.ok(peaks.every((p) => p.lag === 7 || p.lag % 7 !== 0));
});

test("sector-shaped bytes give the sector size", () => {
  // A small CD image: a 12-byte sync, a 4-byte header, and mostly zeros.
  const sector = 2352;
  const bytes = new Uint8Array(sector * 22);
  for (let s = 0; s < 22; s++) {
    const at = s * sector;
    for (let i = 1; i <= 10; i++) bytes[at + i] = 0xff;
    bytes[at + 13] = 2;
    bytes[at + 14] = s;
    bytes[at + 15] = 1;
    for (let i = 0; i < 40; i++) bytes[at + 16 + i] = 0x41 + ((s + i) % 26);
  }
  const r = autocorrelation(bytes, 8192);
  const peaks = findPeaks(r, { floor: noiseFloor(bytes.length) });
  assert.equal(peaks[0]?.lag, sector);
});

test("blocks whose headers come every two or three blocks give the block size", () => {
  // A tar: 512-byte blocks, a header (a name, and "ustar" at 257) then one
  // or two data blocks of text padded with zeros, so headers are 1,024 or
  // 1,536 bytes apart and never 512.
  const block = 512;
  const plan = [2, 3, 2, 2, 3, 3, 2, 3, 2, 2, 3, 2, 3, 3, 2, 2, 3, 2, 3, 2];
  const total = plan.reduce((a, b) => a + b, 0);
  const bytes = new Uint8Array(total * block);
  let x = 7;
  let at = 0;
  for (const blocks of plan) {
    for (let i = 0; i < 100; i++) bytes[at + i] = 0x61 + (i % 26);
    bytes.set([0x75, 0x73, 0x74, 0x61, 0x72], at + 257);
    x = (x * 1103515245 + 12345) >>> 0;
    const text = 200 + ((x >>> 16) % ((blocks - 1) * block - 200));
    for (let i = 0; i < text; i++) {
      x = (x * 1103515245 + 12345) >>> 0;
      bytes[at + block + i] = 0x20 + ((x >>> 16) % 95);
    }
    at += blocks * block;
  }
  const { peaks } = periodicity({ id: 1, bytes, maxLag: 8192 });
  assert.equal(peaks[0]?.lag, block);
});

test("the common step is the shortest lag, or a fraction of it that fits", () => {
  assert.equal(commonStep([2352, 4704, 7056]), 2352);
  assert.equal(commonStep([1024, 1536, 2560, 3584]), 512);
  // A peak a byte off among many is still on the step.
  assert.equal(commonStep([8, 16, 24, 32, 41]), 8);
  // 8 does not split into steps of 3, and steps under 16 are not offered.
  assert.equal(commonStep([8, 12]), null);
  assert.equal(commonStep([]), null);
});

test("the noise floor rises for a short sample", () => {
  assert.equal(noiseFloor(1 << 20), 0.03);
  assert.ok(noiseFloor(1000) > 0.1);
});

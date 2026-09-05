// What the height ledger promises: a running total that never falls, a row
// found at a pixel that agrees with adding the rows up one at a time, and a
// measured row that is dropped when the map is full rather than growing it.

import { test } from "node:test";
import assert from "node:assert/strict";

import { RowHeights } from "../src/rowheights.ts";

test("sparse measured rows beyond the signed 32-bit range retain their addresses", () => {
  const h = new RowHeights();
  h.setRows(6_000_000_000);
  h.measure(4_000_000_000, 100);
  assert.equal(h.heightBefore(4_000_000_001), 4_000_000_001 * 20 + 80);
  assert.deepEqual(h.rowAtY(4_000_000_000 * 20 + 75), { row: 4_000_000_000, offsetPx: 75 });
});

/** A ledger of `rows` rows, base 20. */
function ledger(rows: number): RowHeights {
  const h = new RowHeights();
  h.setBase(20);
  h.setRows(rows);
  return h;
}

/** Where a row starts, added up the slow way. */
function scanBefore(h: RowHeights, row: number): number {
  let y = 0;
  for (let i = 0; i < row; i++) y += h.heightOf(i);
  return y;
}

test("rows of one height are the base height each", () => {
  const h = ledger(10);
  assert.equal(h.heightOf(3), 20);
  assert.equal(h.heightBefore(4), 80);
  assert.equal(h.totalHeight(), 200);
});

test("a structural extra lifts the rows after it and not the ones before", () => {
  const h = ledger(10);
  h.setStructural([{ row: 4, extra: 26 }]);
  assert.equal(h.heightOf(4), 46);
  assert.equal(h.heightBefore(4), 80);
  assert.equal(h.heightBefore(5), 126);
  assert.equal(h.totalHeight(), 226);
});

test("two extras on one row are one row that much taller", () => {
  const h = ledger(4);
  h.setStructural([
    { row: 1, extra: 26 },
    { row: 1, extra: 20 },
  ]);
  assert.equal(h.heightOf(1), 66);
  assert.equal(h.totalHeight(), 80 + 46);
});

test("structural extras given out of order still land on their rows", () => {
  const h = ledger(6);
  h.setStructural([
    { row: 5, extra: 10 },
    { row: 0, extra: 4 },
    { row: 3, extra: 7 },
  ]);
  assert.equal(h.heightOf(0), 24);
  assert.equal(h.heightOf(3), 27);
  assert.equal(h.heightOf(5), 30);
  assert.equal(h.totalHeight(), 120 + 21);
});

test("a measurement is kept as what the row had over the height it was reckoned", () => {
  const h = ledger(10);
  h.setStructural([{ row: 2, extra: 26 }]);
  // Drawn at 68: base 20, heading 26, and 22 more of chips.
  h.measure(2, 68);
  assert.equal(h.heightOf(2), 68);
  assert.equal(h.totalHeight(), 200 + 26 + 22);
});

test("a row drawn shorter than the base does not shorten the ledger", () => {
  const h = ledger(5);
  h.measure(1, 4);
  assert.equal(h.heightOf(1), 20);
  assert.equal(h.totalHeight(), 100);
});

test("the running total never falls", () => {
  const h = ledger(50);
  h.setStructural(Array.from({ length: 9 }, (_, i) => ({ row: i * 5 + 1, extra: 26 })));
  for (let i = 0; i < 50; i += 3) h.measure(i, 20 + (i % 7) * 11);
  let last = -1;
  for (let r = 0; r <= 50; r++) {
    const y = h.heightBefore(r);
    assert.ok(y >= last, `row ${r} went backwards`);
    last = y;
  }
});

test("heightBefore agrees with adding the rows up", () => {
  const h = ledger(40);
  h.setStructural([
    { row: 0, extra: 26 },
    { row: 7, extra: 46 },
    { row: 8, extra: 20 },
    { row: 39, extra: 26 },
  ]);
  for (const r of [1, 3, 7, 8, 9, 20, 39]) h.measure(r, 20 + r);
  for (let r = 0; r <= 40; r++) assert.equal(h.heightBefore(r), scanBefore(h, r), `row ${r}`);
});

test("the row at a pixel is the one a linear scan finds", () => {
  const h = ledger(30);
  h.setStructural([
    { row: 2, extra: 26 },
    { row: 3, extra: 26 },
    { row: 17, extra: 46 },
  ]);
  for (const r of [0, 3, 4, 17, 29]) h.measure(r, 20 + r * 2);
  const total = h.totalHeight();
  for (let y = 0; y < total; y++) {
    const got = h.rowAtY(y);
    let row = 0;
    let at = 0;
    while (row < 29 && at + h.heightOf(row) <= y) {
      at += h.heightOf(row);
      row++;
    }
    assert.equal(got.row, row, `y=${y}`);
    assert.equal(got.offsetPx, y - at, `y=${y} offset`);
  }
});

test("a pixel before the file or past it lands on the first or last row", () => {
  const h = ledger(8);
  h.setStructural([{ row: 7, extra: 26 }]);
  assert.deepEqual(h.rowAtY(-500), { row: 0, offsetPx: 0 });
  assert.deepEqual(h.rowAtY(0), { row: 0, offsetPx: 0 });
  const end = h.rowAtY(h.totalHeight() + 1000);
  assert.equal(end.row, 7);
  assert.equal(end.offsetPx, 46);
});

test("an empty file has a row zero to sit on", () => {
  const h = ledger(0);
  assert.deepEqual(h.rowAtY(0), { row: 0, offsetPx: 0 });
  assert.deepEqual(h.rowAtY(900), { row: 0, offsetPx: 0 });
  assert.equal(h.totalHeight(), 0);
});

test("a row's top edge is a pixel that lands on that row", () => {
  const h = ledger(25);
  h.setStructural([
    { row: 5, extra: 26 },
    { row: 12, extra: 46 },
  ]);
  h.measure(12, 110);
  for (let r = 0; r < 25; r++) {
    const at = h.rowAtY(h.heightBefore(r));
    assert.equal(at.row, r);
    assert.equal(at.offsetPx, 0);
  }
});

test("trimming keeps the rows nearest where the reader is looking", () => {
  const h = ledger(20000);
  for (let r = 0; r < 6000; r++) h.measure(r, 42);
  h.trim(5000);
  assert.ok(h.hasMeasured(5000));
  assert.ok(h.hasMeasured(4000));
  assert.ok(!h.hasMeasured(1));
  // The total is still right for the rows that are left.
  assert.equal(h.heightOf(1), 20);
  assert.equal(h.heightOf(5000), 42);
});

test("the first and last rows keep their measurements however far the reader goes", () => {
  const h = ledger(20000);
  h.measure(0, 60);
  h.measure(19999, 60);
  for (let r = 1; r < 8000; r++) h.measure(r, 42);
  h.trim(7000);
  assert.ok(h.hasMeasured(0), "the top of the file was forgotten");
  assert.ok(h.hasMeasured(19999), "the end of the file was forgotten");
  assert.equal(h.heightOf(0), 60);
  assert.equal(h.heightOf(19999), 60);
});

test("clearing the measurements leaves the structure alone", () => {
  const h = ledger(10);
  h.setStructural([{ row: 3, extra: 26 }]);
  h.measure(3, 90);
  h.clearMeasured();
  assert.equal(h.heightOf(3), 46);
  h.clearStructural();
  assert.equal(h.heightOf(3), 20);
});

test("a scattering of extras and measurements bisects the same as it scans", () => {
  // Fixed seed, so a failure can be run again.
  let seed = 12345;
  const rand = (n: number): number => {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    return seed % n;
  };
  for (let round = 0; round < 20; round++) {
    const rows = 5 + rand(80);
    const h = ledger(rows);
    const struct = [];
    for (let i = 0; i < rows; i++) if (rand(4) === 0) struct.push({ row: i, extra: 1 + rand(60) });
    h.setStructural(struct);
    for (let i = 0; i < rows; i++) if (rand(3) === 0) h.measure(i, rand(120));
    const total = h.totalHeight();
    assert.equal(total, scanBefore(h, rows));
    for (let t = 0; t < 40; t++) {
      const y = rand(Math.max(1, total));
      const got = h.rowAtY(y);
      assert.equal(h.heightBefore(got.row) + got.offsetPx, y);
      assert.ok(got.offsetPx < h.heightOf(got.row));
    }
  }
});

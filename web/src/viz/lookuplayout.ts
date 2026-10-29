// The arithmetic of the index-lookup figure (`indexlookup.ts`), with no DOM so
// the tests can check it: where each entry of the table goes across the
// strip, where the index's arrow lands, how far to zoom in at first so the
// entry it lands on can be read, and which ticks the axis under the strip
// gets.
//
// The strip is drawn in the units a number into the table counts in. A
// string table is indexed by a byte offset from its start (an ELF `sh_name`
// of 17 is the string starting 17 bytes in), so its strip is bytes and a long
// name is a wide cell. A palette is indexed by entry number, so its strip is
// entries, one cell each. Either way the axis counts from the table's start
// in those units, and the number in the index field is a tick on it: the
// tick where the arrow lands.

import type { View } from "./boardmath.ts";
import { thinLabels } from "./fit.ts";
import type { LookupTable } from "./tableentries.ts";

/** One entry's cell, in layout pixels before any zoom. */
export type Cell = { readonly x0: number; readonly x1: number; readonly index: number };

export type StripLayout = {
  readonly cells: readonly Cell[];
  /** What the strip's width stands for, in the table's units from its start
   *  (bytes or entries). */
  readonly domain: readonly [number, number];
};

/** Where a number into the table lands, in the table's units. */
export function unitOf(table: Pick<LookupTable, "counts" | "node">, bit: number, hit: number): number {
  return table.counts === "entries" ? hit + 0.5 : (bit - table.node.offset_bits) / 8;
}

/** The cells of the entries read, across `width` pixels. */
export function stripLayout(table: Pick<LookupTable, "counts" | "node" | "entries">, width: number): StripLayout {
  const es = table.entries;
  const first = es[0];
  const last = es[es.length - 1];
  if (first === undefined || last === undefined) return { cells: [], domain: [0, 1] };
  const from = (b: number): number => (b - table.node.offset_bits) / 8;
  // In bytes, from the first byte of any entry to the last: a list placed by
  // offsets need not be in file order, so neither end is the first or last
  // entry's.
  let lo = Infinity;
  let hi = -Infinity;
  for (const e of es) {
    lo = Math.min(lo, e.startBit);
    hi = Math.max(hi, e.endBit);
  }
  const domain: [number, number] = table.counts === "entries" ? [first.index, last.index + 1] : [from(lo), from(hi)];
  const span = Math.max(1e-9, domain[1] - domain[0]);
  const x = (u: number): number => ((u - domain[0]) / span) * width;
  const cells = es.map((e) =>
    table.counts === "entries"
      ? { x0: x(e.index), x1: x(e.index + 1), index: e.index }
      : { x0: x(from(e.startBit)), x1: x(from(e.endBit)), index: e.index },
  );
  return { cells, domain };
}

/** A layout x for a position in the table's units. */
export function unitX(layout: StripLayout, width: number, unit: number): number {
  const [a, b] = layout.domain;
  return ((unit - a) / Math.max(1e-9, b - a)) * width;
}

/**
 * The view to open at: zoomed in until the cell the index lands on is `want`
 * pixels wide (room for its name), no further than `maxK`, and with that cell
 * in the middle, as far as the strip's ends allow. A table whose cells are
 * all wide enough already opens unzoomed.
 */
export function openingView(cell: Cell | null, width: number, want: number, maxK: number): View {
  if (cell === null) return { k: 1, x: 0, y: 0 };
  const w = Math.max(1e-9, cell.x1 - cell.x0);
  const k = Math.max(1, Math.min(maxK, want / w));
  const mid = ((cell.x0 + cell.x1) / 2) * k;
  const x = Math.min(0, Math.max(width - width * k, width / 2 - mid));
  return { k, x, y: 0 };
}

/**
 * The view to open a structure at, which a pointer reaches from its start:
 * zoomed in until a cell of the middle size is `want` pixels wide, no further
 * than `maxK`, from the left end. A structure whose fields are mostly wide
 * enough opens unzoomed.
 */
export function openingFromStart(cells: readonly Cell[], width: number, want: number, maxK: number): View {
  if (cells.length === 0) return { k: 1, x: 0, y: 0 };
  const widths = cells.map((c) => c.x1 - c.x0).sort((a, b) => a - b);
  const middle = Math.max(1e-9, widths[Math.floor(widths.length / 2)] as number);
  const k = Math.max(1, Math.min(maxK, want / middle));
  const start = (cells[0] as Cell).x0 * k;
  // `+ 0` turns the -0 that `Math.min(0, -0)` gives into 0.
  return { k, x: Math.min(0, Math.max(width - width * k, -start)) + 0, y: 0 };
}

/** One tick of the axis under the strip, and which way its label runs. */
export type LookupTick = { readonly value: number; readonly anchor: "start" | "middle" | "end"; readonly hit: boolean };

/**
 * Ticks for the visible stretch `domain` of the table: at most `count`,
 * a power of two apart (offsets are read in hex, and a palette has a power
 * of two entries), plus one at `hit`, the number the index holds, which wins
 * over any tick it would collide with. A label by an edge runs inward so it
 * is not cut off, and only labels clear of their neighbours are kept.
 */
export function lookupTicks(
  domain: readonly [number, number],
  count: number,
  at: (v: number) => number,
  width: number,
  labelWidth: (v: number, hit: boolean) => number,
  hit: number | null,
  step0 = 1,
): LookupTick[] {
  const [a, b] = domain;
  const span = Math.max(1e-9, b - a);
  let step = step0;
  while (span / step > count) step *= 2;
  const values: { v: number; hit: boolean }[] = [];
  for (let v = Math.ceil(a / step) * step; v <= b && values.length <= count + 1; v += step) if (v !== hit) values.push({ v, hit: false });
  if (hit !== null && hit >= a && hit <= b) values.push({ v: hit, hit: true });
  const ticks: LookupTick[] = [];
  const labels: { start: number; end: number; priority: number }[] = [];
  for (const { v, hit: isHit } of values) {
    const x = at(v);
    const w = labelWidth(v, isHit);
    if (x < 0 || x > width || w > width) continue;
    const anchor: LookupTick["anchor"] = x - w / 2 < 0 ? "start" : x + w / 2 > width ? "end" : "middle";
    const start = anchor === "start" ? x : anchor === "end" ? x - w : x - w / 2;
    ticks.push({ value: v, anchor, hit: isHit });
    labels.push({ start, end: start + w, priority: isHit ? 1 : 0 });
  }
  const keep = thinLabels(labels, 6);
  return ticks.filter((_, i) => keep[i] === true).sort((p, q) => p.value - q.value);
}

/** The entry `delta` places from `from`, kept inside the entries read. */
export function stepEntry(from: number, delta: number, first: number, read: number): number {
  if (read <= 0) return from;
  return Math.max(first, Math.min(first + read - 1, from + delta));
}

/** The view that keeps zoom `view.k` and brings layout x `x0..x1` into the
 *  middle of the strip, for a step to an entry off screen. */
export function centreOn(view: View, x0: number, x1: number, width: number): View {
  const k = view.k;
  const mid = ((x0 + x1) / 2) * k;
  const x = Math.min(0, Math.max(width - width * k, width / 2 - mid));
  return { k, x, y: 0 };
}

/** Whether layout x `x0..x1` is fully in view under `view`. */
export function inView(view: View, x0: number, x1: number, width: number): boolean {
  const a = x0 * view.k + view.x;
  const b = x1 * view.k + view.x;
  return a >= 0 && b <= width;
}

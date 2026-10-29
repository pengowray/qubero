// The arithmetic of the stride raster (`strideraster.ts`): how the bytes of a
// file are laid out as rows of a chosen width, where each row and column lands
// on screen at a zoom, what one screen pixel shows when it covers several
// bytes, and which row width to start at.
//
// DOM-free, so the tests can run it without a page, the way `treemap.ts` and
// `bitorder.ts` keep their layout apart from their drawing.
//
// Why the two axes zoom differently. A raster of 2,352-byte CD sectors in a
// rail 175 pixels wide is 13 bytes to a pixel across, and a 50 KiB image has
// only 22 such rows. Square pixels would make the whole picture 36 pixels
// tall, too thin to see the column the sector sync bytes make. A long file has
// the opposite trouble: six million rows of 16 bytes fit no panel with square
// cells. So at no zoom every row fits the view, each as tall as the view
// affords up to `ROW_MAX_PX`, however wide the cells are; and the zoom brings
// the cells to square by the time they are about `ROW_MAX_PX` wide, which is
// when their two hex digits fit (`rowHeight`). A uniform zoom from the
// stretched start would make a CD row 2,000 pixels tall by the time its bytes
// could be read.
//
// The horizontal zoom is d3-zoom's (`board.ts`); the vertical position is kept
// here as the row at the top of the view, moved by the same pointer
// (`followZoom`).

import type { View } from "./boardmath.ts";

/** The tallest a row is drawn before a zoom makes the cells wider than it. */
export const ROW_MAX_PX = 16;
/** The narrowest and widest a row may be, in bytes. 64 KiB is the largest page
 *  size of the formats the design names (SQLite's), and the most one row's
 *  read can cost on every frame. */
export const MIN_STRIDE = 1;
export const MAX_STRIDE = 65536;
/** How wide a byte cell may get at the deepest zoom: room for its eight bits
 *  (`LOD.bitsPx`) and a little over. */
export const MAX_CELL_PX = 48;

/** How many rows `length` bytes make at `stride` bytes a row. The last row
 *  may be short. */
export function strideRows(length: number, stride: number): number {
  return stride > 0 ? Math.ceil(Math.max(0, length) / stride) : 0;
}

export function clampStride(n: number): number {
  if (!Number.isFinite(n)) return MIN_STRIDE;
  return Math.min(MAX_STRIDE, Math.max(MIN_STRIDE, Math.round(n)));
}

/** The raster at no zoom, in CSS pixels. */
export type RasterGeometry = {
  readonly length: number;
  readonly stride: number;
  readonly rows: number;
  /** The width the rows are drawn in. */
  readonly width: number;
  /** The height of the view. */
  readonly height: number;
  /** How wide one byte is at no zoom: the row fills the width. */
  readonly fitX: number;
  /** How tall one row is at no zoom: as tall as the view allows for every row
   *  to be seen, up to `ROW_MAX_PX`. Under a pixel for a file of more rows
   *  than the view has pixels, in which case a pixel row stands for several
   *  file rows and shows one of them. */
  readonly rowBase: number;
};

/** The height a raster of `rows` rows asks for, between `min` and `max`:
 *  every row at `ROW_MAX_PX` when they fit, and otherwise all of `max`. */
export function rasterHeight(rows: number, min: number, max: number): number {
  return Math.round(Math.min(max, Math.max(min, rows * ROW_MAX_PX)));
}

export function rasterGeometry(length: number, stride: number, width: number, height: number): RasterGeometry {
  const s = clampStride(stride);
  const rows = strideRows(length, s);
  const w = Math.max(1, width);
  const h = Math.max(1, height);
  return { length, stride: s, rows, width: w, height: h, fitX: w / s, rowBase: rows > 0 ? Math.min(ROW_MAX_PX, h / rows) : ROW_MAX_PX };
}

/** How wide one byte cell is on screen at zoom `k`. */
export function cellWidth(g: RasterGeometry, k: number): number {
  return g.fitX * k;
}

/** The zoom at which cells become square, for a raster whose rows start out
 *  shorter than its cells are wide. */
function squareZoom(g: RasterGeometry): number {
  return Math.max(2, ROW_MAX_PX / g.fitX);
}

/**
 * How tall one row is on screen at zoom `k`. Two cases:
 * - Rows taller than the cells are wide at no zoom (few rows, as in a CD
 *   image): rows keep their height while the cells widen, and grow with the
 *   cells once those are as wide, so a cell is never wider than it is tall.
 * - Rows shorter than the cells are wide (a long file, whose rows were
 *   squeezed to fit): rows grow faster than the cells, evenly in log scale,
 *   until the two meet at `squareZoom`; square after that.
 * Either way the height only grows as the zoom does, so zooming about the
 * pointer never moves the picture the wrong way.
 */
export function rowHeight(g: RasterGeometry, k: number): number {
  const cw = cellWidth(g, k);
  if (g.rowBase >= g.fitX) return Math.max(g.rowBase, cw);
  const kSq = squareZoom(g);
  if (k >= kSq) return cw;
  const t = Math.log(Math.max(1, k)) / Math.log(kSq);
  return cw * Math.pow(g.rowBase / g.fitX, 1 - t);
}

/** The deepest zoom: a byte cell `MAX_CELL_PX` wide, past the point where the
 *  cells are square, and never less than no zoom at all. */
export function maxZoom(g: RasterGeometry): number {
  const deep = MAX_CELL_PX / g.fitX;
  return Math.max(1, g.rowBase >= g.fitX ? deep : Math.max(deep, squareZoom(g)));
}

/** The most rows that can be above the view: the last row ends at the foot of
 *  the view, or nothing is above it when every row fits. */
export function maxTopRow(g: RasterGeometry, rowH: number): number {
  return Math.max(0, g.rows - g.height / rowH);
}

export function clampTopRow(g: RasterGeometry, rowH: number, topRow: number): number {
  return Math.min(maxTopRow(g, rowH), Math.max(0, topRow));
}

/**
 * The row at the top after d3-zoom moved from `prev` to `next`. A zoom keeps
 * the row under the pointer under the pointer, as d3-zoom keeps the column;
 * the pointer's height is worked out from the two transforms, which d3-zoom
 * moved about it, so a zoom from a key or a button (about the middle) is
 * followed the same way. A pan moves the rows by the pixels it moved.
 */
export function followZoom(g: RasterGeometry, topRow: number, prev: View, next: View): number {
  const hPrev = rowHeight(g, prev.k);
  const hNext = rowHeight(g, next.k);
  if (Math.abs(next.k - prev.k) < 1e-12) return clampTopRow(g, hNext, topRow - (next.y - prev.y) / hPrev);
  // d3-zoom kept the layout y under the pointer fixed: (py - y) / k is the
  // same before and after, so py is where the two lines cross.
  const py = (prev.y / prev.k - next.y / next.k) / (1 / prev.k - 1 / next.k);
  const at = topRow + py / hPrev;
  return clampTopRow(g, hNext, at - py / hNext);
}

/** The file row drawn at screen row `y`, or -1 below the last. */
export function rowAt(g: RasterGeometry, rowH: number, topRow: number, y: number): number {
  const r = Math.floor(topRow + y / rowH);
  return r >= 0 && r < g.rows ? r : -1;
}

/** The column at screen `x`: a fraction, so a caller can tell which byte
 *  inside a cell, or which bytes a pixel covers. */
export function columnAt(g: RasterGeometry, view: View, x: number): number {
  return (x - view.x) / cellWidth(g, view.k);
}

/** Where column `c` starts on screen. */
export function columnX(g: RasterGeometry, view: View, c: number): number {
  return c * cellWidth(g, view.k) + view.x;
}

/** Where row `r` starts on screen. */
export function rowY(rowH: number, topRow: number, r: number): number {
  return (r - topRow) * rowH;
}

/**
 * The bytes each of `pixels` screen columns covers, `px` screen pixels apiece,
 * as half-open column ranges, clipped to the row. A column of the screen
 * outside the row covers nothing (`from === to`). When a byte is wider than a
 * pixel, neighbouring pixels cover the same byte.
 */
export function pixelColumns(g: RasterGeometry, view: View, pixels: number, px: number): { from: Int32Array; to: Int32Array } {
  const from = new Int32Array(pixels);
  const to = new Int32Array(pixels);
  for (let i = 0; i < pixels; i++) {
    const a = columnAt(g, view, i * px);
    const b = columnAt(g, view, (i + 1) * px);
    let c0 = Math.floor(a);
    let c1 = Math.max(c0 + 1, Math.ceil(b));
    c0 = Math.max(0, c0);
    c1 = Math.min(g.stride, c1);
    from[i] = c0;
    to[i] = Math.max(c0, c1);
  }
  return { from, to };
}

/** What a byte is, at a glance: zero, printable text, or anything else. The
 *  same three as the report's byte strip (`report/zoommap.ts`). */
export const ZERO = 0;
export const TEXT = 1;
export const OTHER = 2;

export function byteKind(v: number): number {
  if (v === 0) return ZERO;
  if ((v >= 0x20 && v < 0x7f) || v === 9 || v === 10 || v === 13) return TEXT;
  return OTHER;
}

/**
 * What one pixel shows for the bytes `bytes[from, to)`: the largest value,
 * and the kind that stands out most (other over text over zero). Neither is
 * an average, so a column one byte wide in a row of thousands, such as the
 * sync bytes at the front of every CD sector, still shows when thirteen bytes
 * share a pixel.
 */
export function pixelValue(bytes: Uint8Array, from: number, to: number): { max: number; kind: number } {
  let max = 0;
  let kind = ZERO;
  for (let i = from; i < to; i++) {
    const v = bytes[i] ?? 0;
    if (v > max) max = v;
    const k = byteKind(v);
    if (k > kind) kind = k;
  }
  return { max, kind };
}

/**
 * The grey a byte value is drawn in, 0 to 255. Zero is the panel's own
 * background in either theme, so empty space recedes and the bytes with
 * something in them stand out: dark on light, light on dark.
 */
export function greyLevel(v: number, dark: boolean, bg: number): number {
  const t = Math.max(0, Math.min(255, v)) / 255;
  const far = dark ? 240 : 20;
  return Math.round(bg + (far - bg) * t);
}

/**
 * The row width the file's own template suggests: the size of the records of
 * its widest run of equal-sized records, such as the 2,352-byte sectors of a
 * CD image. `runs` are candidate lists, each with the sizes in bits of its
 * first few children and the bits the list covers. Null when no run has at
 * least two children of one whole-byte size, four bytes or more.
 */
export function templateStride(runs: readonly { readonly sizes: readonly number[]; readonly bits: number }[]): number | null {
  let best: { stride: number; bits: number } | null = null;
  for (const run of runs) {
    const first = run.sizes[0];
    if (run.sizes.length < 2 || first === undefined || first % 8 !== 0 || first < 32) continue;
    if (!run.sizes.every((s) => s === first)) continue;
    const stride = first / 8;
    if (stride > MAX_STRIDE) continue;
    if (best === null || run.bits > best.bits) best = { stride, bits: run.bits };
  }
  return best?.stride ?? null;
}

/**
 * The ticks to label along the row, from the candidates d3's scale offers:
 * each label centred on its tick, or running inward from it by an edge so
 * none is cut off, and only those clear of their neighbours kept. `at` is a
 * tick's screen x and `labelWidth` its label's width.
 */
export function columnTicks(candidates: readonly number[], at: (c: number) => number, width: number, labelWidth: (c: number) => number, gap = 6): { value: number; anchor: "start" | "middle" | "end" }[] {
  const out: { value: number; anchor: "start" | "middle" | "end" }[] = [];
  let lastEnd = Number.NEGATIVE_INFINITY;
  for (const v of candidates) {
    const x = at(v);
    const w = labelWidth(v);
    if (x < 0 || x > width || w > width) continue;
    const anchor = x - w / 2 < 0 ? "start" : x + w / 2 > width ? "end" : "middle";
    const start = anchor === "start" ? x : anchor === "end" ? x - w : x - w / 2;
    if (start < lastEnd + gap) continue;
    out.push({ value: v, anchor });
    lastEnd = start + w;
  }
  return out;
}

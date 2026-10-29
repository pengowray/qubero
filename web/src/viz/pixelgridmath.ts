// The arithmetic of the pixel grid (`pixelgrid.ts`), with no page behind it:
// where the picture sits in the view, which pixel is under the pointer, which
// pixels are on screen, and how a JPEG scan's MCUs and blocks and a PNG's
// passes and scanlines divide the picture.
//
// Kept apart from the drawing so it can be tested without a canvas, as
// `bitorder.ts`, `strips.ts` and `treemap.ts` are. Everything here counts in
// three spaces, named the same way throughout:
// - picture pixels: column `x` and row `y` of the image, from the top left;
// - layout pixels: where the picture is drawn before any zoom, which is the
//   picture scaled by `Placement.scale` and moved to `Placement.x0, y0`;
// - screen pixels: layout under the board's transform (`boardmath.ts`).
//
// The JPEG half reads the core's trace of a scan (`JpegScan`) as it comes:
// the blocks are parallel arrays in coding order, and a block's bits follow
// the one before it, so a block's place in the file is the sum of the bits
// before it. Each MCU's count includes the padding and restart marker after
// its blocks, which is why the blocks of an MCU are summed from the MCU's own
// start rather than from the end of the MCU before.

import type { JpegScan } from "../doc.ts";
import { adam7Passes, passOf } from "../report/pngpasses.ts";
import type { View } from "./boardmath.ts";

// ----- placing the picture -----

/** Where the picture is drawn at no zoom: picture pixel (x, y) covers layout
 *  pixels from (x0 + x * scale, y0 + y * scale), `scale` across and down. */
export type Placement = { readonly scale: number; readonly x0: number; readonly y0: number };

/**
 * Place a `w` by `h` picture in a `boxW` by `boxH` box, centred. A picture
 * larger than the box is scaled down to fit it. A smaller one is magnified by
 * a whole number, so every pixel is the same number of screen pixels square
 * and no row is drawn a pixel taller than the next; `maxMagnify` caps that,
 * since a 2 by 2 icon blown up to fill the box is all edge and no picture.
 */
export function placePicture(w: number, h: number, boxW: number, boxH: number, maxMagnify = 16): Placement {
  const fit = Math.min(boxW / Math.max(1, w), boxH / Math.max(1, h));
  const scale = fit >= 1 ? Math.max(1, Math.min(maxMagnify, Math.floor(fit))) : fit;
  return { scale, x0: (boxW - w * scale) / 2, y0: (boxH - h * scale) / 2 };
}

/** How many screen pixels one picture pixel is across. */
export function cellPx(place: Placement, view: View): number {
  return place.scale * view.k;
}

/** The picture pixel under a screen point, or null off the picture. */
export function pixelAt(place: Placement, view: View, sx: number, sy: number, w: number, h: number): { x: number; y: number } | null {
  const c = cellPx(place, view);
  const x = Math.floor((sx - view.x - place.x0 * view.k) / c);
  const y = Math.floor((sy - view.y - place.y0 * view.k) / c);
  return x < 0 || y < 0 || x >= w || y >= h ? null : { x, y };
}

/** The screen point where picture pixel (x, y) starts: its top left corner. */
export function pixelToScreen(place: Placement, view: View, x: number, y: number): { sx: number; sy: number } {
  return { sx: (place.x0 + x * place.scale) * view.k + view.x, sy: (place.y0 + y * place.scale) * view.k + view.y };
}

/** The pixels on a `viewW` by `viewH` screen, end exclusive and kept to the
 *  picture: what a redraw needs to touch and no more. Empty when the picture
 *  is panned off the view. */
export function visiblePixels(place: Placement, view: View, viewW: number, viewH: number, w: number, h: number): { x0: number; y0: number; x1: number; y1: number } {
  const c = cellPx(place, view);
  const ox = place.x0 * view.k + view.x;
  const oy = place.y0 * view.k + view.y;
  const x0 = Math.max(0, Math.floor(-ox / c));
  const y0 = Math.max(0, Math.floor(-oy / c));
  const x1 = Math.min(w, Math.ceil((viewW - ox) / c));
  const y1 = Math.min(h, Math.ceil((viewH - oy) / c));
  return { x0, y0, x1: Math.max(x0, x1), y1: Math.max(y0, y1) };
}

// ----- the value written in a pixel -----

/** The size a pixel's value is written in, in the monospace stack, and the
 *  height of a line of it. */
export const LABEL_PX = 10;
export const LABEL_LINE_PX = 11;
/** Room kept clear inside a cell, on each side, so a value never touches the
 *  next cell's. */
const LABEL_PAD = 2;

/**
 * Whether a pixel's value fits its cell: one line per channel, each `digits`
 * hex digits wide in a face where a digit is `charRatio` of the size across.
 * Never shrunk to make it fit; a cell that cannot hold every channel shows
 * none of them, and the hover card has the value. Cells do not overlap, so
 * two values that each fit their cell can never meet.
 */
export function labelFits(cell: number, channels: number, digits: number, minCell = 24, charRatio = 0.6): boolean {
  if (cell < minCell || channels <= 0) return false;
  const width = digits * LABEL_PX * charRatio + 2 * LABEL_PAD;
  const height = channels * LABEL_LINE_PX + 2 * LABEL_PAD;
  return cell >= width && cell >= height;
}

/** Hex digits for one sample of `bits` bits: 1 for up to 4, 2 for a byte, 4
 *  for 16 bits. */
export function hexDigits(bits: number): number {
  return Math.max(1, Math.ceil(bits / 4));
}

/** A sample written in hex, padded to its full width so a column of them
 *  lines up. */
export function hexSample(value: number, bits: number): string {
  return value.toString(16).padStart(hexDigits(bits), "0");
}

// ----- a JPEG scan -----

export type ScanShape = Pick<
  JpegScan,
  | "width"
  | "height"
  | "mcus_across"
  | "mcus_down"
  | "channels"
  | "scan"
  | "mcu_bits"
  | "block_channel"
  | "block_x"
  | "block_y"
  | "block_bits"
  | "run_offset_bits"
  | "restart_interval"
>;

/** The picture pixels one MCU covers, across and down. A scan of one channel
 *  has one block to an MCU, whatever that channel's sampling. */
export function mcuSize(scan: ScanShape): { mw: number; mh: number } {
  const hmax = Math.max(1, ...scan.channels.map((c) => c.h));
  const vmax = Math.max(1, ...scan.channels.map((c) => c.v));
  const only = scan.scan.length === 1 ? scan.channels[scan.scan[0] ?? 0] : undefined;
  if (only !== undefined) return { mw: (8 * hmax) / only.h, mh: (8 * vmax) / only.v };
  return { mw: 8 * hmax, mh: 8 * vmax };
}

/** The picture pixels one block of `channel` covers: eight samples each way,
 *  stretched by however much that channel is subsampled. */
export function blockSize(scan: ScanShape, channel: number): { bw: number; bh: number } {
  const hmax = Math.max(1, ...scan.channels.map((c) => c.h));
  const vmax = Math.max(1, ...scan.channels.map((c) => c.v));
  const c = scan.channels[channel];
  return { bw: (8 * hmax) / Math.max(1, c?.h ?? 1), bh: (8 * vmax) / Math.max(1, c?.v ?? 1) };
}

/** The pixels the scan codes, which run past the picture to whole MCUs. */
export function codedSize(scan: ScanShape): { w: number; h: number } {
  const { mw, mh } = mcuSize(scan);
  return { w: scan.mcus_across * mw, h: scan.mcus_down * mh };
}

/** One channel's blocks as a grid: block `index[by * across + bx]` in coding
 *  order, or -1 for a place no block was coded at. */
export type BlockGrid = { readonly across: number; readonly down: number; readonly index: Int32Array };

/** Where every MCU and block of a scan starts in the file, and each channel's
 *  blocks by place. Worked out once per scan. */
export type ScanIndex = {
  /** Bit where MCU `m` starts, for `m` from 0 to the count; the last is the
   *  end of the scan's run. */
  readonly mcuStart: Float64Array;
  /** Bit where block `i` starts, in coding order. */
  readonly blockStart: Float64Array;
  readonly perMcu: number;
  /** By place in `scan.channels`; null for a channel this scan does not
   *  carry. */
  readonly grids: readonly (BlockGrid | null)[];
};

export function indexScan(scan: ScanShape): ScanIndex {
  const mcus = scan.mcu_bits.length;
  const mcuStart = new Float64Array(mcus + 1);
  let at = scan.run_offset_bits;
  for (let m = 0; m < mcus; m++) {
    mcuStart[m] = at;
    at += scan.mcu_bits[m] ?? 0;
  }
  mcuStart[mcus] = at;
  const blocks = scan.block_channel.length;
  const perMcu = mcus === 0 ? 1 : Math.max(1, Math.round(blocks / mcus));
  const blockStart = new Float64Array(blocks);
  for (let m = 0; m < mcus; m++) {
    let b = mcuStart[m] ?? 0;
    for (let i = m * perMcu; i < Math.min(blocks, (m + 1) * perMcu); i++) {
      blockStart[i] = b;
      b += scan.block_bits[i] ?? 0;
    }
  }
  const across = scan.channels.map(() => 0);
  const down = scan.channels.map(() => 0);
  for (let i = 0; i < blocks; i++) {
    const c = scan.block_channel[i] ?? 0;
    across[c] = Math.max(across[c] ?? 0, (scan.block_x[i] ?? 0) + 1);
    down[c] = Math.max(down[c] ?? 0, (scan.block_y[i] ?? 0) + 1);
  }
  const grids = scan.channels.map((_, c): BlockGrid | null => {
    const a = across[c] ?? 0;
    const d = down[c] ?? 0;
    if (a === 0 || d === 0) return null;
    return { across: a, down: d, index: new Int32Array(a * d).fill(-1) };
  });
  for (let i = 0; i < blocks; i++) {
    const g = grids[scan.block_channel[i] ?? 0];
    if (g === null || g === undefined) continue;
    g.index[(scan.block_y[i] ?? 0) * g.across + (scan.block_x[i] ?? 0)] = i;
  }
  return { mcuStart, blockStart, perMcu, grids };
}

/** The MCU that codes picture pixel (x, y), in coding order, or null outside
 *  the coded area. */
export function mcuAt(scan: ScanShape, x: number, y: number): number | null {
  const { mw, mh } = mcuSize(scan);
  const mx = Math.floor(x / mw);
  const my = Math.floor(y / mh);
  if (x < 0 || y < 0 || mx >= scan.mcus_across || my >= scan.mcus_down) return null;
  return my * scan.mcus_across + mx;
}

/** The block of `channel` that codes picture pixel (x, y), or null. */
export function blockAt(scan: ScanShape, index: ScanIndex, channel: number, x: number, y: number): number | null {
  const g = index.grids[channel];
  if (g === null || g === undefined || x < 0 || y < 0) return null;
  const { bw, bh } = blockSize(scan, channel);
  const bx = Math.floor(x / bw);
  const by = Math.floor(y / bh);
  if (bx >= g.across || by >= g.down) return null;
  const i = g.index[by * g.across + bx] ?? -1;
  return i < 0 ? null : i;
}

/** The bits of block `i`, start inclusive and end exclusive. */
export function blockBits(scan: ScanShape, index: ScanIndex, i: number): { startBit: number; endBit: number } {
  const start = index.blockStart[i] ?? 0;
  return { startBit: start, endBit: start + (scan.block_bits[i] ?? 0) };
}

/** The bits of MCU `m`, padding and any restart marker after it included. */
export function mcuBits(index: ScanIndex, m: number): { startBit: number; endBit: number } {
  return { startBit: index.mcuStart[m] ?? 0, endBit: index.mcuStart[m + 1] ?? 0 };
}

/** The MCUs whose bits meet the run from `startBit` to `endBit`, as a range
 *  of coding order, end exclusive; empty when the run is outside the scan. A
 *  run of no bits (a cursor) meets the MCU it is inside. */
export function mcusInBits(index: ScanIndex, startBit: number, endBit: number): { from: number; to: number } {
  const starts = index.mcuStart;
  const mcus = starts.length - 1;
  if (mcus <= 0) return { from: 0, to: 0 };
  const end = Math.max(endBit, startBit + 1);
  if (end <= (starts[0] ?? 0) || startBit >= (starts[mcus] ?? 0)) return { from: 0, to: 0 };
  // The last MCU starting at or before the run's start, then the first one
  // starting at or after its end.
  let lo = 0;
  let hi = mcus - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if ((starts[mid] ?? 0) <= startBit) lo = mid;
    else hi = mid - 1;
  }
  let to = lo + 1;
  while (to < mcus && (starts[to] ?? 0) < end) to++;
  return { from: lo, to };
}

/** Which restart interval MCU `m` is in, from 0, or null for a scan with no
 *  restart markers. */
export function restartInterval(scan: Pick<ScanShape, "restart_interval">, m: number): number | null {
  return scan.restart_interval > 0 ? Math.floor(m / scan.restart_interval) : null;
}

/** Classes of the heat layer's shading, fewest bits first. */
export const HEAT_CLASSES = 7;

/** A round step that splits 0 to `max` into `HEAT_CLASSES` classes: 1, 2,
 *  2.5 or 5 times a power of ten, so the legend reads in round numbers. */
export function heatStep(max: number): number {
  const raw = Math.max(1, max) / HEAT_CLASSES;
  const p = 10 ** Math.floor(Math.log10(raw));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= raw) return Math.max(1, Math.ceil(m * p));
  return Math.ceil(10 * p);
}

/** The class a count of bits falls in, 0 to `HEAT_CLASSES - 1`. */
export function heatClass(bits: number, step: number): number {
  return Math.max(0, Math.min(HEAT_CLASSES - 1, Math.floor(bits / Math.max(1, step))));
}

// ----- a PNG's passes and scanlines -----

/** The pass that stores picture pixel (x, y), 1 to 7, and its row within
 *  that pass; the pass is null and the row is the picture's for a picture
 *  that is not interlaced. */
export function scanlineOf(interlaced: boolean, x: number, y: number): { pass: number | null; row: number } {
  if (!interlaced) return { pass: null, row: y };
  const pass = passOf(x, y);
  const shape = adam7Passes(x + 1, y + 1)[pass - 1];
  return { pass, row: shape === undefined ? y : Math.floor((y - shape.y0) / shape.dy) };
}

/** The pixels one scanline stores, for lighting them: its picture row, and
 *  the first column and the step between columns. */
export function scanlinePixels(width: number, height: number, pass: number | null, row: number): { y: number; x0: number; dx: number; count: number } | null {
  if (pass === null) return row < height ? { y: row, x0: 0, dx: 1, count: width } : null;
  const p = adam7Passes(width, height)[pass - 1];
  if (p === undefined || row >= p.rows || p.cols === 0) return null;
  return { y: p.y0 + row * p.dy, x0: p.x0, dx: p.dx, count: p.cols };
}

/** The key a scanline is found by: its pass (0 for none) and its row. */
export function scanlineKey(pass: number | null, row: number): string {
  return `${pass ?? 0}:${row}`;
}

/** How many columns the filter strip beside a PNG has: one per pass for an
 *  interlaced picture, since each picture row is in several passes' scanlines,
 *  and one for a picture that is not. */
export function filterColumns(interlaced: boolean): number {
  return interlaced ? 7 : 1;
}

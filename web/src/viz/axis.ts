// Where to put the ticks on an axis of byte addresses.
//
// Several pieces draw file offsets along x (the child strip in the At cursor
// panel, the position icicle in the treemap panel), and each asks the same two
// things: which addresses to mark, and which of their labels fit without
// running into each other. d3-axis draws the ticks; this picks them. It was
// first written inside `childstrip.ts`, and moved here when a second piece
// needed it, so one piece is not importing another piece's drawing module.

import { formatOffset } from "../format.ts";
import { monoMeasure, thinLabels } from "./fit.ts";

/** The axis labels are in the monospace stack at 10 pixels (`.viz-axis`). */
export const TICK_PX = 10;
const TICK_MEASURE = monoMeasure();

/** How wide the label for the address `byte` is drawn. */
export function tickLabelWidth(byte: number): number {
  return TICK_MEASURE(formatOffset(byte * 8), TICK_PX);
}

/** A tick, and which way its label runs from it. */
export type HexTick = { readonly byte: number; readonly anchor: "start" | "middle" | "end" };

/**
 * Byte addresses to put ticks at across `domain` (in bytes): at most `count`
 * of them, a power of two apart, since addresses are read in hex and a tick
 * at 0x14 then 0x19 is a step nobody counts in. A label is centred on its
 * tick, or starts at it by the left edge and ends at it by the right, so none
 * is cut off; then only the labels clear of their neighbours are kept
 * (`thinLabels`), so none is drawn over another. d3-axis draws what is left.
 */
export function hexTicks(domain: readonly [number, number], count: number, at: (byte: number) => number, width: number, labelWidth: (byte: number) => number): HexTick[] {
  const [a, b] = domain;
  const span = Math.max(1, b - a);
  let step = 1;
  while (span / step > count) step *= 2;
  const ticks: HexTick[] = [];
  const labels: { start: number; end: number; priority: number }[] = [];
  for (let v = Math.ceil(a / step) * step; v <= b && ticks.length <= count + 1; v += step) {
    const x = at(v);
    const w = labelWidth(v);
    if (x < 0 || x > width || w > width) continue;
    const anchor: HexTick["anchor"] = x - w / 2 < 0 ? "start" : x + w / 2 > width ? "end" : "middle";
    const start = anchor === "start" ? x : anchor === "end" ? x - w : x - w / 2;
    ticks.push({ byte: v, anchor });
    labels.push({ start, end: start + w, priority: 0 });
  }
  const keep = thinLabels(labels, 6);
  return ticks.filter((_, i) => keep[i] === true);
}

/**
 * `hexTicks` with at least two ticks wherever two labels fit, so the span and
 * the scale can both be read off the axis. Snapping the step to a power of
 * two can leave a narrow axis one tick, when the two the spacing allowed fall
 * either side of the edges; asking for twice as many then gives a step half
 * as long, and `thinLabels` still drops any label that would touch another.
 */
export function readableHexTicks(domain: readonly [number, number], spacingPx: number, at: (byte: number) => number, width: number): HexTick[] {
  let count = Math.max(2, Math.floor(width / spacingPx));
  let ticks = hexTicks(domain, count, at, width, tickLabelWidth);
  for (let i = 0; i < 4 && ticks.length < 2; i++) {
    count *= 2;
    ticks = hexTicks(domain, count, at, width, tickLabelWidth);
  }
  return ticks;
}

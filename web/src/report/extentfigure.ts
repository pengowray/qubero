// A length that does not match its part, drawn: where the length field says
// the part ends, where the reader ended it, and where its parent and the file
// end, on one scale.
//
// The difference is usually a few bytes at the end of something long: the
// pipistrelle WAV's RIFF size runs 8 bytes past a file of 301,024. Drawn from
// the part's start, 8 bytes in 300,000 is nothing at all. So where the ends
// are close together compared with the part, the figure shows the stretch
// around the ends and marks the cut on the left, and the scale is the same
// for every mark on it.

import { svgEl } from "../dom.ts";
import { formatOffset } from "../format.ts";
import type { ExtentCheck } from "./coredata.ts";
import { RV } from "./text.ts";

const W = 640;
const LABEL = 150;
const ROW = 22;
const BAR = 12;

type Mark = { readonly at: number; readonly label: string; readonly kind: "stated" | "read" | "room" | "file" };

/** Where the figure's window starts and ends, in bits: the whole part where
 *  the ends are far enough apart to see, and the stretch round them where
 *  they are not. Exported for the tests. */
export function extentWindow(start: number, ends: readonly number[]): { lo: number; hi: number; cut: boolean } {
  const min = Math.min(...ends);
  const max = Math.max(...ends);
  const spread = Math.max(8, max - min);
  // At a twentieth of the width or more, the difference is plain from the
  // start of the part.
  if (spread * 20 >= max - start) return { lo: start, hi: max + spread / 4, cut: false };
  const pad = Math.max(spread * 1.5, 64);
  return { lo: Math.max(start, min - pad), hi: max + pad / 3, cut: min - pad > start };
}

export function extentFigure(c: ExtentCheck): HTMLElement | null {
  if (c.role !== "length") return null;
  const start = c.offset_bits;
  const rows: { label: string; end: number; kind: "stated" | "read" }[] = [];
  if (c.stated !== null) rows.push({ label: RV.extentStated, end: start + c.stated, kind: "stated" });
  if (c.read !== null) rows.push({ label: RV.extentRead, end: start + c.read, kind: "read" });
  if (rows.length === 0) return null;
  const marks: Mark[] = [];
  const roomEnd = start + c.room_bits;
  if (roomEnd < c.space_bits) marks.push({ at: roomEnd, label: RV.extentParentEnds, kind: "room" });
  marks.push({ at: c.space_bits, label: RV.extentFileEnds, kind: "file" });
  const ends = [...rows.map((r) => r.end), ...marks.map((m) => m.at)];
  const { lo, hi, cut } = extentWindow(start, ends);
  const H = rows.length * ROW + 34;
  const x = (bit: number): number => LABEL + ((bit - lo) / Math.max(1, hi - lo)) * (W - LABEL - 8);
  const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, class: "rv-extentfig", role: "img", "aria-label": RV.extentFigureLabel });
  rows.forEach((r, i) => {
    const y = 6 + i * ROW;
    const t = svgEl("text", { x: "0", y: String(y + BAR - 1), class: "rv-ef-label" });
    t.textContent = r.label;
    svg.append(t);
    const x0 = x(Math.max(lo, start));
    const x1 = Math.max(x0 + 1, x(r.end));
    svg.append(svgEl("rect", { x: String(x0), y: String(y), width: String(x1 - x0), height: String(BAR), rx: "2", class: `rv-ef-bar is-${r.kind}` }));
    const end = svgEl("text", { x: String(Math.min(W - 4, x1 + 4)), y: String(y + BAR - 1), class: "rv-ef-end", "text-anchor": x1 + 90 > W ? "end" : "start" });
    if (x1 + 90 > W) end.setAttribute("x", String(x1 - 4));
    end.textContent = formatOffset(r.end);
    svg.append(end);
  });
  const bottom = rows.length * ROW + 6;
  for (const m of marks) {
    if (m.at < lo || m.at > hi) continue;
    const mx = x(m.at);
    svg.append(svgEl("line", { x1: String(mx), y1: "2", x2: String(mx), y2: String(bottom), class: `rv-ef-mark is-${m.kind}` }));
    const t = svgEl("text", { x: String(mx), y: String(bottom + 12), class: "rv-ef-marklabel", "text-anchor": mx > W - 100 ? "end" : "middle" });
    t.textContent = `${m.label} ${formatOffset(m.at)}`;
    svg.append(t);
  }
  if (cut) {
    // The window does not start at the part's start: a break at the left.
    const bx = LABEL - 2;
    svg.append(svgEl("path", { d: `M${bx} 2 l4 5 l-4 5 l4 5 l-4 5 l4 5`, class: "rv-ef-cut" }));
  }
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-extentbox";
  fig.append(svg);
  const cap = document.createElement("figcaption");
  cap.textContent = cut ? RV.extentCaptionCut(formatOffset(start)) : RV.extentCaption(formatOffset(start));
  fig.append(cap);
  return fig;
}

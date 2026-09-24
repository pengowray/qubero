// The ribbon: two rows of boxes, each in its own order, joined by bands.
//
// Every element has an extent on the top row and an extent on the bottom row,
// and a band joins the two, so a band can be narrow at one end and wide at the
// other. The TTF report's "Directory, in directory order" figure is this
// shape: each directory record along the top in stored order, joined to the
// range it describes along the bottom in file order. The same figure draws a
// deflate stream's codes joined to the bytes they write (a literal is a
// narrow band that stays narrow, a copy a narrow band that widens toward the
// bytes it repeats), the bits of a Huffman code in stored order against the
// order they are read, and zigzag order against an 8 × 8 block.
//
// It takes plain data and knows nothing about files: the caller says what the
// two rows are, how long each is, and which box joins which. One SVG path per
// band, drawn with two cubic curves, which is all a band needs.
//
// Hover lights a band and both of its ends and fades the rest; `onHover` gets
// the band's index, so the caller can say what it is.

import { svgEl } from "../dom.ts";

/** One box of a row: a stretch `[from, to)` in that row's own units. */
export type RibbonBox = {
  readonly from: number;
  readonly to: number;
  /** Written inside the box when it is wide enough. */
  readonly label?: string;
  /** A CSS colour. The row's default when absent. */
  readonly color?: string;
  /** Drawn faintly: a box that is there for its place, not its own sake,
   *  such as the bits of a byte a code does not use. */
  readonly dim?: boolean;
};

export type RibbonRow = {
  readonly boxes: readonly RibbonBox[];
  /** The stretch the row stands for, in its own units: `[from, to)` is drawn
   *  across the whole width. */
  readonly from: number;
  readonly to: number;
  /** Said above the top row, or below the bottom one. */
  readonly caption: string;
};

/** A band joins box `top` of the top row to box `bottom` of the bottom row. */
export type RibbonBand = {
  readonly top: number;
  readonly bottom: number;
  readonly color?: string;
};

export type RibbonData = {
  readonly top: RibbonRow;
  readonly bottom: RibbonRow;
  readonly bands: readonly RibbonBand[];
};

export type RibbonOptions = {
  /** The width to draw at. The figure scales to its container after that. */
  readonly width?: number;
  /** The pointer is over band `i`, or over none. */
  readonly onHover?: (band: number | null, e: PointerEvent) => void;
  /** A band or a box was clicked. */
  readonly onPick?: (band: number, e: MouseEvent) => void;
  /** Heights of a box and of the space the bands cross, and how wide one
   *  character of a box's label is, in the figure's own units. */
  readonly box?: number;
  readonly gap?: number;
  readonly charWidth?: number;
  /** How much of a box's width each end of its band takes, from 0 to 1. Less
   *  than 1 draws each band as a strand from the middle of its box, so bands
   *  that cross can be followed one by one. */
  readonly strand?: number;
  /** False to leave the rows' captions out of the figure, for a caller that
   *  writes them beside it. */
  readonly captions?: boolean;
};

/** Heights of the figure's parts, in px, unless the caller says. */
const CAPTION = 16;
const DEFAULT_BOX = 22;
const DEFAULT_GAP = 90;
const CHAR = 7;
/** A box never draws thinner than this, so a one-bit code can still be seen
 *  and pointed at. */
const MIN_BOX = 1.5;

/** A linear map from a row's units to x across `[x0, x1]`. */
export function scaleOf(from: number, to: number, x0: number, x1: number): (v: number) => number {
  const span = to - from;
  if (span <= 0) return () => x0;
  const k = (x1 - x0) / span;
  return (v) => x0 + (v - from) * k;
}

/**
 * The outline of a band from `[t0, t1]` on the line `yTop` to `[b0, b1]` on
 * the line `yBottom`: down the left edge on a curve, along the bottom, and up
 * the right edge on the mirror of that curve. The curves leave each row
 * straight down, so a band reads as coming out of its box rather than out of
 * its neighbour's.
 */
export function bandPath(t0: number, t1: number, yTop: number, b0: number, b1: number, yBottom: number): string {
  const mid = (yTop + yBottom) / 2;
  const f = (n: number): string => (Math.round(n * 100) / 100).toString();
  return (
    `M${f(t0)} ${f(yTop)}` +
    ` C${f(t0)} ${f(mid)}, ${f(b0)} ${f(mid)}, ${f(b0)} ${f(yBottom)}` +
    ` L${f(b1)} ${f(yBottom)}` +
    ` C${f(b1)} ${f(mid)}, ${f(t1)} ${f(mid)}, ${f(t1)} ${f(yTop)} Z`
  );
}

/** Where each box of a row is drawn: its left edge and width, never thinner
 *  than `MIN_BOX`. */
export function boxSpans(row: RibbonRow, width: number): { x: number; w: number }[] {
  const x = scaleOf(row.from, row.to, 0, width);
  return row.boxes.map((b) => {
    const x0 = x(b.from);
    const w = Math.max(MIN_BOX, x(b.to) - x0);
    return { x: x0, w };
  });
}

/** Draw the figure. The caller adds it to the page. */
export function ribbon(data: RibbonData, opts: RibbonOptions = {}): SVGSVGElement {
  const W = opts.width ?? 960;
  const BOX = opts.box ?? DEFAULT_BOX;
  const GAP = opts.gap ?? DEFAULT_GAP;
  const CHAR_W = opts.charWidth ?? CHAR;
  const STRAND = Math.min(1, Math.max(0, opts.strand ?? 1));
  const captions = opts.captions !== false;
  const yTopBox = captions ? CAPTION : 2;
  const yTopEdge = yTopBox + BOX;
  const yBottomBox = yTopEdge + GAP;
  const yBottomEdge = yBottomBox + BOX;
  const H = yBottomEdge + (captions ? CAPTION + 4 : 2);
  const svg = svgEl("svg", { viewBox: `0 0 ${W} ${H}`, class: "rv-ribbon", role: "img", "aria-label": `${data.top.caption}; ${data.bottom.caption}` });
  svg.style.width = "100%";
  const top = boxSpans(data.top, W);
  const bottom = boxSpans(data.bottom, W);
  const bands = svgEl("g", { class: "rv-rb-bands" });
  data.bands.forEach((band, i) => {
    const t = top[band.top];
    const b = bottom[band.bottom];
    if (t === undefined || b === undefined) return;
    const colour = band.color ?? data.top.boxes[band.top]?.color ?? "var(--accent)";
    const narrow = (x: number, w: number): [number, number] => [x + (w * (1 - STRAND)) / 2, x + (w * (1 + STRAND)) / 2];
    const [t0, t1] = narrow(t.x, t.w);
    const [b0, b1] = narrow(b.x, b.w);
    const p = svgEl("path", { d: bandPath(t0, t1, yTopEdge, b0, b1, yBottomBox), class: "rv-rb-band", fill: colour });
    p.dataset.band = String(i);
    bands.append(p);
  });
  const row = (r: RibbonRow, spans: { x: number; w: number }[], y: number, which: "top" | "bottom"): SVGGElement => {
    const g = svgEl("g", { class: `rv-rb-row rv-rb-${which}` });
    r.boxes.forEach((box, i) => {
      const s = spans[i];
      if (s === undefined) return;
      const rect = svgEl("rect", {
        x: String(s.x),
        y: String(y),
        width: String(Math.max(MIN_BOX, s.w - (s.w > 4 ? 1 : 0))),
        height: String(BOX),
        rx: "2",
        class: `rv-rb-box${box.dim === true ? " is-dim" : ""}`,
        fill: box.color ?? "var(--muted)",
      });
      rect.dataset[which] = String(i);
      g.append(rect);
      if (box.label !== undefined && s.w >= box.label.length * CHAR_W + 2) {
        const text = svgEl("text", { x: String(s.x + s.w / 2), y: String(y + BOX / 2 + 4), class: `rv-rb-label${box.dim === true ? " is-dim" : ""}`, "text-anchor": "middle" });
        text.textContent = box.label;
        g.append(text);
      }
    });
    return g;
  };
  const cap = (text: string, y: number): SVGTextElement => {
    const t = svgEl("text", { x: "0", y: String(y), class: "rv-rb-caption" });
    t.textContent = text;
    return t;
  };
  if (captions) svg.append(cap(data.top.caption, CAPTION - 4));
  svg.append(bands, row(data.top, top, yTopBox, "top"), row(data.bottom, bottom, yBottomBox, "bottom"));
  if (captions) svg.append(cap(data.bottom.caption, H - 4));

  // Which bands meet each box, so hovering a box lights its bands too.
  const byTop = new Map<number, number[]>();
  const byBottom = new Map<number, number[]>();
  data.bands.forEach((b, i) => {
    byTop.set(b.top, [...(byTop.get(b.top) ?? []), i]);
    byBottom.set(b.bottom, [...(byBottom.get(b.bottom) ?? []), i]);
  });
  const bandOf = (target: EventTarget | null): number | null => {
    const e = target as SVGElement | null;
    if (e === null || e.dataset === undefined) return null;
    if (e.dataset.band !== undefined) return Number(e.dataset.band);
    if (e.dataset.top !== undefined) return byTop.get(Number(e.dataset.top))?.[0] ?? null;
    if (e.dataset.bottom !== undefined) return byBottom.get(Number(e.dataset.bottom))?.[0] ?? null;
    return null;
  };
  let lit: number | null = null;
  const light = (i: number | null): void => {
    if (i === lit) return;
    lit = i;
    svg.classList.toggle("is-lit", i !== null);
    for (const el of svg.querySelectorAll(".is-on")) el.classList.remove("is-on");
    if (i === null) return;
    const band = data.bands[i];
    if (band === undefined) return;
    svg.querySelector(`[data-band="${i}"]`)?.classList.add("is-on");
    svg.querySelector(`[data-top="${band.top}"]`)?.classList.add("is-on");
    svg.querySelector(`[data-bottom="${band.bottom}"]`)?.classList.add("is-on");
  };
  svg.addEventListener("pointermove", (e) => {
    const i = bandOf(e.target);
    light(i);
    opts.onHover?.(i, e);
  });
  svg.addEventListener("pointerleave", (e) => {
    light(null);
    opts.onHover?.(null, e);
  });
  svg.addEventListener("click", (e) => {
    const i = bandOf(e.target);
    if (i !== null) opts.onPick?.(i, e);
  });
  return svg;
}

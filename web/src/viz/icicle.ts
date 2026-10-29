// Position-true icicle: the field tree as a flame chart in file order.
//
// Candidate `position-icicle` in docs/DESIGN-diagrams.md, section 4.2; built
// by agent B of the first batch (section 5.3). Drawn as the treemap panel's
// sixth mode, In order (`treemappanel.ts`), so it shares the panel's picker,
// its trail of opened parts and its corner button that throws it over the
// window. Test samples: mp4/h264-aac-moov-at-end.mp4,
// png/png10-palette-trns-text.png.
//
// The treemap says how much of the file each part is and loses where it is;
// this says both. One row per depth of the tree, file offset across, and every
// bar exactly as wide as its field's bytes, under the bar of the structure it
// is in. So the top rows are a map of the file (the MP4's `mdat` before its
// `moov`, a PNG's chunks in order) and the rows under them are what each part
// is made of. Holes no field covers are hatched, wrong values are a red edge
// along the bar's foot, and fields too narrow to tell apart are folded into
// one bar that says how many there are.
//
// Explorable the way every piece is (rule 1 of the design):
// - the wheel zooms about the pointer along the address axis only, and a drag
//   pans (`board.ts`, x only); deeper rows are read as bars grow wide enough
//   to hold them (`iciclelayout.ts`, `lod.ts`);
// - hover shows the field's name, type, value, offset and size in the one
//   card (`tip.ts`), and lights its bytes in the hex view (`linked.ts`);
// - a click goes to the bytes (`pick.ts`), a double-click opens a structure
//   so it fills the picture, with the panel's trail as the way back out, the
//   same two verbs as the treemap's boxes; a double-click on a folded bar
//   zooms in until its fields can be drawn;
// - the field the cursor is in is outlined, whichever view moved the cursor.
//
// Names are drawn inside their own bar only, cut with an ellipsis or left off
// when they do not fit (`fit.ts`), so two names never meet. Drawn on a canvas:
// a zoomed-out view of a large file is thousands of bars, redrawn on every
// step of the wheel.

import type { Doc } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { fieldClass } from "../fieldstyle.ts";
import { bitSizeText, childWord, countText, GAP_LABEL, TREEMAP, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { rescaleDomain, type View } from "./boardmath.ts";
import { hexTicks } from "./childstrip.ts";
import { canvasMeasure, ellipsise, monoMeasure, type Measure } from "./fit.ts";
import { barAt, barFor, fieldLabel, IcicleCache, layoutIcicle, type IcicleBar, type IcicleField, type IcicleLayout } from "./iciclelayout.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { goToBits, goToField } from "./pick.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";
import "./icicle.css";

/** One row of the picture. Tall enough for an 11px name with room round it,
 *  short enough that the rail's column holds the ten rows an MP4 nests to. */
export const ROW_H = 18;
const AXIS_H = 18;
const FOOT_H = 22;
/** The name inside a bar: the size of the listing's small text. */
const LABEL_PX = 11;
const LABEL_PAD = 4;
/** Fewer characters than this is not a name worth cutting down to. */
const LABEL_MIN_CHARS = 3;
/** How wide one byte may get at the deepest zoom: wide enough for its two
 *  hex digits in the hex view's terms, so the deepest zoom is a few bytes. */
const MAX_BYTE_PX = 64;
/** Room per axis tick, so d3 is asked for as many as fit. */
const TICK_SPACING_PX = 96;
const TICK_PX = 10;
const TICK_MEASURE = monoMeasure();

/** The picture's height for a panel this tall, split into its rows. */
export function icicleRows(height: number): number {
  return Math.max(1, Math.floor((height - AXIS_H - FOOT_H) / ROW_H));
}

export type IcicleOptions = {
  readonly width: number;
  readonly height: number;
  /** The stretch row 0 stands for, when it is more than the root: the whole
   *  file, while the template describes only part of it. */
  readonly frame?: { readonly startBit: number; readonly endBit: number } | null;
  /** A structure was double-clicked: the fields from the one under the root
   *  down to it, outermost first. The host makes it the picture's root. */
  readonly onOpen?: (chain: readonly IcicleField[]) => void;
};

export type Icicle = {
  readonly el: HTMLElement;
  readonly board: Board;
  /** The view is a new size. */
  resize(width: number, height: number): void;
  /** The document changed: read the tree again and redraw at the same zoom. */
  refresh(): void;
  destroy(): void;
};

/** The colours the canvas draws in, read from the stylesheet so the picture
 *  is the same colours as the hex view and the listing, light or dark. */
type Palette = {
  readonly fg: string;
  readonly muted: string;
  readonly line: string;
  readonly bg: string;
  readonly accent: string;
  readonly warn: string;
  readonly field: (cls: string) => string;
};

let palette: Palette | null = null;
if (typeof matchMedia === "function") matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => (palette = null));

function readPalette(): Palette {
  if (palette !== null) return palette;
  const root = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string): string => root.getPropertyValue(name).trim() || fallback;
  const probe = document.createElement("span");
  probe.style.display = "none";
  document.body.append(probe);
  const fields = new Map<string, string>();
  const field = (cls: string): string => {
    let c = fields.get(cls);
    if (c === undefined) {
      probe.className = cls;
      c = getComputedStyle(probe).getPropertyValue("--field-color").trim() || v("--muted", "#888");
      fields.set(cls, c);
    }
    return c;
  };
  // Every class the tree can need, read now while the probe is on the page.
  for (const cls of ["field-number", "field-text", "field-marker", "field-category", "field-structure", "field-binary", "field-note"]) field(cls);
  probe.remove();
  palette = {
    fg: v("--fg", "#1b1b1f"),
    muted: v("--muted", "#6b6f76"),
    line: v("--line", "#dcdfe4"),
    bg: v("--bg", "#ffffff"),
    accent: v("--accent", "#2457c5"),
    warn: v("--warn", "#b4361f"),
    field: (cls) => fields.get(cls) ?? v("--muted", "#888"),
  };
  return palette;
}

let measure: Measure | null = null;
function measurer(): Measure {
  measure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return measure;
}

type Axis = {
  readonly scale: typeof import("d3-scale");
  readonly axis: typeof import("d3-axis");
  readonly selection: typeof import("d3-selection");
};
let axisLoading: Promise<Axis> | null = null;
function loadAxis(): Promise<Axis> {
  axisLoading ??= Promise.all([import("d3-scale"), import("d3-axis"), import("d3-selection")]).then(([scale, axis, selection]) => ({ scale, axis, selection }));
  return axisLoading;
}

const SVG = "http://www.w3.org/2000/svg";

/** What a fold stands for, in the parent's own word: items, fields, chunks. */
function foldName(bar: IcicleBar): string {
  return countText(bar.count ?? 0, bar.parent === undefined ? "field" : childWord(bar.parent));
}

/** The text drawn on a bar, before fitting. */
function barText(bar: IcicleBar): string {
  if (bar.kind === "gap") return GAP_LABEL;
  if (bar.kind === "fold") return foldName(bar);
  return fieldLabel(bar.field as IcicleField);
}

/** The hover card for a bar. */
function barCard(bar: IcicleBar): HTMLElement {
  const where = `${formatOffset(bar.startBit)} · ${bitSizeText(bar.endBit - bar.startBit)}`;
  if (bar.kind === "gap") return tipCard(GAP_LABEL, [VIZ.icicle.gapDetail], where);
  if (bar.kind === "fold") return tipCard(foldName(bar), [VIZ.icicle.foldDetail], where);
  const f = bar.field as IcicleField;
  const lines: string[] = [];
  const value = f.composite ? countText(f.child_count, childWord(f)) : f.value;
  lines.push(value === "" ? f.type : `${f.type} · ${value}`);
  if (f.problem !== undefined) lines.push(f.problem.text);
  else if (f.problems_within[0] > 0) lines.push(VIZ.icicle.wrongInside(f.problems_within[0]));
  if (bar.unread !== undefined) lines.push(VIZ.icicle.unread(f.child_count - bar.unread, f.child_count, childWord(f)));
  if (f.composite && f.child_count > 0 && bar.depth > 0) lines.push(TREEMAP.openHint);
  return tipCard(fieldLabel(f), lines, where);
}

/**
 * Draw the tree under `root` across `opts.width` pixels, `opts.height` high.
 * The root's bytes fill the width at no zoom; `frame` widens that to the
 * whole file when the root covers less of it.
 */
export function drawIcicle(doc: Doc, root: IcicleField, opts: IcicleOptions): Icicle {
  let width = Math.max(40, Math.floor(opts.width));
  let height = Math.max(ROW_H + AXIS_H + FOOT_H, Math.floor(opts.height));
  let rows = icicleRows(height);
  const frame = opts.frame ?? null;
  const d0 = frame?.startBit ?? root.offset_bits;
  const d1 = Math.max(d0 + 8, frame?.endBit ?? root.offset_bits + root.size_bits);

  const cache = new IcicleCache({
    children: (path, from, to) => {
      const r = doc.templateChildren(path, from, to);
      if (r.status === "ok") return r.node;
      // An error is not going to answer differently next time, and "not yet"
      // would have the picture ask again on every change for ever.
      return r.status === "error" ? [] : null;
    },
  });

  const host = document.createElement("div");
  host.className = "icl";
  const stage = document.createElement("div");
  stage.className = "icl-stage";
  const canvas = document.createElement("canvas");
  canvas.className = "icl-canvas";
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", VIZ.icicle.label(fieldLabel(root)));
  const axisSvg = document.createElementNS(SVG, "svg");
  axisSvg.setAttribute("class", "icl-axis");
  const axisG = document.createElementNS(SVG, "g");
  axisG.setAttribute("class", "viz-axis");
  axisSvg.append(axisG);
  stage.append(canvas, axisSvg);
  const foot = document.createElement("div");
  foot.className = "icl-foot";
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  reset.disabled = true;
  foot.append(reset);
  host.append(stage, foot);

  let view: View = { k: 1, x: 0, y: 0 };
  let layout: IcicleLayout = { rows: [], pending: false };
  let hovered: IcicleBar | null = null;
  let axis: Axis | null = null;
  const bus = linkedFor(doc);
  let selected = bus.state.selection;

  const rowsH = (): number => rows * ROW_H;

  const size = (): void => {
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(rowsH() * dpr);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${rowsH()}px`;
    axisSvg.setAttribute("width", String(width));
    axisSvg.setAttribute("height", String(AXIS_H));
  };

  const visible = (): [number, number] => rescaleDomain(view, [d0, d1], [0, width]);

  const relayout = (): void => {
    const [a, b] = visible();
    layout = layoutIcicle(root, cache, { startBit: a, endBit: b, width, rows, ...(frame === null ? {} : { frame }) });
  };

  let hatch: CanvasPattern | null = null;
  let hatchFor = "";
  const hatchOf = (ctx: CanvasRenderingContext2D, colour: string): CanvasPattern | null => {
    if (hatch !== null && hatchFor === colour) return hatch;
    const tile = document.createElement("canvas");
    const dpr = window.devicePixelRatio || 1;
    tile.width = tile.height = Math.round(6 * dpr);
    const t = tile.getContext("2d");
    if (t === null) return null;
    t.scale(dpr, dpr);
    t.strokeStyle = colour;
    t.lineWidth = 1.2;
    t.beginPath();
    // The same 45 degree hatch the treemap gives unmapped bytes, drawn
    // corner to corner of a repeating tile.
    t.moveTo(-1, 7);
    t.lineTo(7, -1);
    t.moveTo(-1, 1);
    t.lineTo(1, -1);
    t.moveTo(5, 7);
    t.lineTo(7, 5);
    t.stroke();
    hatch = ctx.createPattern(tile, "repeat");
    hatch?.setTransform(new DOMMatrix().scale(1 / dpr));
    hatchFor = colour;
    return hatch;
  };

  const paint = (): void => {
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    const p = readPalette();
    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, rowsH());
    ctx.font = `${LABEL_PX}px ${getComputedStyle(document.body).fontFamily}`;
    ctx.textBaseline = "middle";
    const lit = selected === null ? null : barFor(layout, selected.path, selected.startBit);
    for (const row of layout.rows) {
      for (const bar of row) {
        const laneH = ROW_H / bar.lanes;
        const y = bar.depth * ROW_H + bar.lane * laneH;
        // Clipped to the canvas, so a bar running off both edges is drawn as
        // the stretch of it on screen and its name stays in view.
        const x0 = Math.max(-2, bar.x0);
        const x1 = Math.min(width + 2, bar.x1);
        const w = x1 - x0;
        // A one-pixel gap between neighbours and between rows, so two bars of
        // one colour read as two. Bars narrower than three pixels give up the
        // gap and are drawn solid, or they would vanish.
        const thin = w < 3;
        const bx = thin ? x0 : x0 + 0.5;
        const bw = thin ? Math.max(1, w) : w - 1;
        const by = y + 0.5;
        const bh = laneH - 1;
        if (bar.kind === "gap") {
          const pat = hatchOf(ctx, p.muted);
          ctx.globalAlpha = 0.55;
          ctx.fillStyle = pat ?? p.line;
          ctx.fillRect(bx, by, bw, bh);
          ctx.globalAlpha = 1;
        } else {
          const colour = bar.kind === "fold" ? p.muted : p.field(fieldClass((bar.field as IcicleField).kind));
          ctx.fillStyle = colour;
          ctx.globalAlpha = thin ? 0.75 : bar.kind === "fold" ? 0.22 : 0.3;
          ctx.fillRect(bx, by, bw, bh);
          ctx.globalAlpha = 1;
          if (!thin) {
            ctx.strokeStyle = colour;
            ctx.lineWidth = 1;
            if (bar.kind === "fold") ctx.setLineDash([3, 2]);
            ctx.strokeRect(bx + 0.5, by + 0.5, bw - 1, bh - 1);
            ctx.setLineDash([]);
          }
          // A wrong value is a red edge along the foot of its bar and of every
          // bar it is inside, so a problem deep in the tree shows from the top
          // row down to it.
          const f = bar.field;
          if (f !== undefined && ((f.problem !== undefined && f.problem.tier === "invalid") || f.problems_within[0] > 0)) {
            ctx.fillStyle = p.warn;
            ctx.fillRect(bx, by + bh - 2, Math.max(1, bw), 2);
          }
        }
        if (bar === hovered && !thin) {
          ctx.strokeStyle = p.fg;
          ctx.lineWidth = 1;
          ctx.strokeRect(bx + 0.5, by + 0.5, bw - 1, bh - 1);
        }
        if (bar === lit) {
          ctx.strokeStyle = p.accent;
          ctx.lineWidth = 2;
          ctx.strokeRect(bx + 1, by + 1, Math.max(1, bw - 2), bh - 2);
        }
        // The name, inside its own bar only, so two names never meet.
        if (laneH < LABEL_PX + 4) continue;
        const left = Math.max(0, bar.x0) + LABEL_PAD;
        const right = Math.min(width, bar.x1) - LABEL_PAD;
        if (right - left < LABEL_PX) continue;
        // A field with a value of its own is named the way the hex view's
        // chips name it, `size 32`, where there is room for the name and the
        // start of the value; otherwise the name alone.
        const label = barText(bar);
        const f = bar.field;
        const valued = f !== undefined && !f.composite && f.value !== "" ? ellipsise(`${label} ${f.value}`, right - left, LABEL_PX, measurer(), label.length + 2) : null;
        const text = valued ?? ellipsise(label, right - left, LABEL_PX, measurer(), LABEL_MIN_CHARS);
        if (text === null) continue;
        // The hatch runs through a gap's name, so the name sits on a plate of
        // the page's own background.
        if (bar.kind === "gap") {
          ctx.fillStyle = p.bg;
          ctx.fillRect(left - 2, y + 2, measurer()(text, LABEL_PX) + 4, laneH - 4);
        }
        ctx.fillStyle = bar.kind === "gap" ? p.muted : p.fg;
        ctx.fillText(text, left, y + laneH / 2 + 0.5);
      }
    }
    drawAxis();
  };

  const drawAxis = (): void => {
    if (axis === null) return;
    const [a, b] = visible();
    const bytes: [number, number] = [a / 8, b / 8];
    const x = axis.scale.scaleLinear().domain(bytes).range([0, width]);
    const ticks = hexTicks(bytes, Math.max(2, Math.floor(width / TICK_SPACING_PX)), (v) => x(v), width, (v) => TICK_MEASURE(formatOffset(v * 8), TICK_PX));
    const ax = axis.axis
      .axisBottom(x)
      .tickValues(ticks.map((t) => t.byte))
      .tickSize(3)
      .tickFormat((d) => formatOffset(Number(d) * 8));
    const g = axis.selection.select(axisG);
    g.call(ax);
    const anchors = new Map(ticks.map((t) => [t.byte, t.anchor]));
    g.selectAll<SVGTextElement, number>(".tick text").attr("text-anchor", (d) => anchors.get(Number(d)) ?? "middle");
  };

  const redraw = (): void => {
    relayout();
    paint();
  };

  size();
  const zoom = board(stage, {
    width,
    height: rowsH() + AXIS_H,
    xOnly: true,
    maxScale: Math.max(1, ((d1 - d0) / 8) * (MAX_BYTE_PX / width)),
    onZoom: (v) => {
      view = v;
      reset.disabled = v.k === 1 && v.x === 0;
      hovered = null;
      redraw();
    },
  });
  reset.addEventListener("click", () => zoom.home(true));
  void loadAxis().then((loaded) => {
    axis = loaded;
    drawAxis();
  });

  const barUnder = (e: MouseEvent): IcicleBar | null => {
    const r = canvas.getBoundingClientRect();
    return barAt(layout, e.clientX - r.left, e.clientY - r.top, ROW_H);
  };

  const stopTip = hoverTip(canvas, (e) => {
    const bar = barUnder(e);
    if (bar !== hovered) {
      hovered = bar;
      paint();
    }
    bus.publish({ hover: bar === null ? null : { startBit: bar.startBit, endBit: bar.endBit } });
    return bar === null ? null : barCard(bar);
  });
  const leave = (): void => {
    bus.publish({ hover: null });
    if (hovered !== null) {
      hovered = null;
      paint();
    }
  };
  canvas.addEventListener("pointerleave", leave);
  canvas.addEventListener("click", (e) => {
    const bar = barUnder(e);
    if (bar === null) return;
    e.stopPropagation();
    if (bar.kind === "field" && bar.field !== undefined) {
      const f = bar.field;
      goToField(doc, { path: f.path, startBit: f.offset_bits, endBit: f.offset_bits + f.size_bits });
    } else goToBits(doc, [{ startBit: bar.startBit, endBit: bar.endBit }]);
  });
  canvas.addEventListener("dblclick", (e) => {
    const bar = barUnder(e);
    if (bar === null) return;
    e.stopPropagation();
    if (bar.kind === "fold") {
      // Zoom until the folded fields are wide enough to draw one by one.
      const per = width / (d1 - d0);
      zoom.zoomTo({ x0: (bar.startBit - d0) * per, y0: 0, x1: (bar.endBit - d0) * per, y1: rowsH() }, true);
      return;
    }
    const f = bar.field;
    if (bar.kind !== "field" || f === undefined || !f.composite || f.child_count === 0 || bar.depth === 0) return;
    // The drawn bars from the one under the root down to this one, so the
    // host's trail names the parts a reader sees, not the `contents` levels
    // the picture passes over.
    const chain: IcicleField[] = [];
    for (let d = 1; d <= bar.depth; d++) {
      const above = layout.rows[d]?.find((b) => b.kind === "field" && b.field !== undefined && b.field.path.length <= f.path.length && b.field.path.every((s, i) => s === f.path[i]));
      if (above?.field !== undefined) chain.push(above.field);
    }
    opts.onOpen?.(chain);
  });
  const stopLinked = subscribeWhileConnected(bus, host, (state, changed) => {
    if (changed !== "selection") return;
    selected = state.selection;
    paint();
  });

  return {
    el: host,
    board: zoom,
    resize: (w, h) => {
      const nw = Math.max(40, Math.floor(w));
      const nh = Math.max(ROW_H + AXIS_H + FOOT_H, Math.floor(h));
      if (nw === width && nh === height) return;
      // Keep the stretch of the file on screen across the resize: the same
      // bits at the left and right edges, whatever the new width.
      const [a, b] = visible();
      width = nw;
      height = nh;
      rows = icicleRows(height);
      size();
      zoom.resize(width, rowsH() + AXIS_H, { x0: 0, y0: 0, x1: width, y1: rowsH() + AXIS_H });
      const per = width / (d1 - d0);
      if (view.k !== 1 || view.x !== 0) zoom.zoomTo({ x0: (a - d0) * per, y0: 0, x1: (b - d0) * per, y1: rowsH() });
      redraw();
    },
    refresh: () => {
      cache.clear();
      redraw();
    },
    destroy: () => {
      zoom.destroy();
      stopTip();
      stopLinked();
      hideTip();
      canvas.removeEventListener("pointerleave", leave);
    },
  };
}

/** Drawn by the treemap panel as its In order mode, which imports
 *  `drawIcicle` itself, so there is nothing to add to a registry. */
export function register(): void {}


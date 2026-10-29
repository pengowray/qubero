// Stride raster: the file's bytes as a grid of cells, a chosen number of
// bytes to a row, with an autocorrelation line that suggests the number.
//
// Candidate `stride-grid` in docs/DESIGN-diagrams.md, section 4.2, the slice
// agent N of the first batch builds (section 5.3): the raw raster, the row
// width control and the periodicity line. The page-owner maps (SQLite pages by
// kind, CFB sectors, chains threaded through them) are the later half and need
// per-format adapters. Drawn in a Stride tab in the rail (`overviewtabs.ts`),
// and over the whole window from that tab's Show larger button.
//
// What it is for. A file of fixed-size records drawn at the record size turns
// into vertical stripes: the twelve sync bytes at the front of every 2,352-byte
// CD sector become one bright column down the left edge, and a record a byte
// out of step breaks the column where it starts. At any other width the same
// bytes are diagonal streaks. So the reader can find the record size of data
// nothing describes, or check that a template's record size is right, by
// changing the width (the box, or the up and down keys) until the stripes
// stand straight, and the autocorrelation line says where to start looking:
// its peaks are the distances at which the bytes repeat.
//
// How it is put together, in the order `register.ts` asks:
// - the data is `doc.read`, one row at a time, kept in a small cache, and
//   what is still on its way is drawn hatched until `pump` says it landed;
// - the arithmetic is `stridemath.ts` (rows, the two zooms, what one pixel
//   shows) and `periodicity.ts` (the autocorrelation and its peaks), both
//   DOM-free and tested; the autocorrelation runs in a worker;
// - the grid is a canvas, zoomed and panned through `board.ts`, with column
//   ticks from d3-axis and hex digits in the cells once they are wide enough
//   (`lod.ts`); a fixed-pitch grid has no labels to collide, which is the one
//   exception rule 2 of the design allows;
// - the line is uPlot, with the same board over it for wheel and drag;
// - hover goes through `tip.ts` and the shared bus, so the hex view marks the
//   bytes under the pointer; a click goes to those bytes (`pick.ts`); the
//   field the cursor is in is outlined on the grid from the bus.

import type { Doc } from "../doc.ts";
import { byteClassColor } from "../fieldstyle.ts";
import { formatBytes, formatOffset } from "../format.ts";
import { VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { rescaleDomain, type View } from "./boardmath.ts";
import { monoMeasure } from "./fit.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { LOD } from "./lod.ts";
import { registerOverviewTab, type OverviewTabPanel } from "./overviewtabs.ts";
import { periodicity, type Peak, type PeriodicityRequest, type PeriodicityResult } from "./periodicity.ts";
import { goToBits } from "./pick.ts";
import {
  byteKind,
  clampStride,
  clampTopRow,
  columnAt,
  columnTicks,
  columnX,
  followZoom,
  greyLevel,
  inkOn,
  lagSplits,
  lagTicks,
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
  rowY,
  templateStride,
  TEXT,
  type RasterGeometry,
} from "./stridemath.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";
import "./strideraster.css";

const S = VIZ.stride;

/** How much of the file the autocorrelation looks at, from its first byte:
 *  enough for a hundred CD sectors or five hundred tar blocks, and quick in
 *  the worker. */
const LAG_WINDOW = 256 * 1024;
/** A file shorter than this has too few repeats to find. */
const LAG_MIN_BYTES = 64;
/** The row width before anything better is known. */
const DEFAULT_STRIDE = 64;
/** The grid's height in the rail: every row at full height when they fit,
 *  between these. */
const RAIL_MIN_H = 48;
const RAIL_MAX_H = 420;
/** The rail's inner width when it has not been laid out yet. */
const RAIL_FALLBACK_W = 220;
/** The line's height: enough to see a peak stand out, small beside the grid. */
const LINE_H_RAIL = 64;
const LINE_H_LARGE = 110;
/** The column ticks above the grid. */
const AXIS_H = 16;
const TICK_PX = 10;
const TICK_MEASURE = monoMeasure();
/** Room for the row addresses down the left of the large grid. */
const ROW_AXIS_W = 64;
/** Below this the grid's own width is too narrow to give a column of row
 *  addresses room, and the hover card says the row instead. */
const ROW_AXIS_MIN_WIDTH = 480;
/** A row's bytes kept for redrawing, at most. Rows of a 64 KiB stride over
 *  a tall view are a few megabytes a frame, and panning back should not read
 *  them again. */
const CACHE_BYTES = 32 * 1024 * 1024;
/** The gap drawn between cells and rows once they are this wide, so single
 *  cells can be told apart. */
const GAP_FROM_PX = 6;
/** The value whose grey a zero cell takes once cells are drawn apart: just
 *  off the background, so a run of zeros shows as cells and not as nothing. */
const ZERO_CELL = 12;

type Mode = "value" | "kind";

// ----- one state per document, shared by the rail and the large view -----

type LagState =
  | { readonly status: "idle" }
  | { readonly status: "short" }
  | { readonly status: "computing" }
  | { readonly status: "done"; readonly r: Float32Array; readonly peaks: readonly Peak[]; readonly sample: number };

/** Where the row width came from. Only a width the reader chose is never
 *  replaced by a better guess. */
type Source = "default" | "peak" | "template" | "reader";

class StrideState {
  stride = DEFAULT_STRIDE;
  source: Source = "default";
  mode: Mode = "value";
  lag: LagState = { status: "idle" };
  /** The template the row width was last guessed from. */
  guessedFrom: string | null = null;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly doc: Doc) {}

  /** The file's length now. */
  get length(): number {
    return this.doc.lengthBytes;
  }

  listen(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private changed(): void {
    for (const fn of [...this.listeners]) fn();
  }

  setStride(n: number, source: Source): void {
    const s = clampStride(n);
    if (s === this.stride && source === this.source) return;
    this.stride = s;
    this.source = source;
    this.changed();
  }

  setMode(mode: Mode): void {
    if (mode === this.mode) return;
    this.mode = mode;
    this.changed();
  }

  /** The record size the template gives, once per template, unless the
   *  reader has chosen a width. */
  guessFromTemplate(): void {
    const name = this.doc.template;
    if (name === null || name === this.guessedFrom || this.source === "reader") return;
    const s = templateGuess(this.doc);
    if (s === "pending") return;
    this.guessedFrom = name;
    if (s !== null) this.setStride(s, "template");
  }

  /** Start the autocorrelation, once. */
  startLag(): void {
    if (this.lag.status !== "idle") return;
    const len = this.doc.lengthBytes;
    if (len < LAG_MIN_BYTES) {
      this.lag = { status: "short" };
      this.changed();
      return;
    }
    this.lag = { status: "computing" };
    this.changed();
    const n = Math.min(len, LAG_WINDOW);
    void this.doc
      .ensureRange(0, n)
      .then(() => runPeriodicity({ id: 0, bytes: this.doc.read(0, n).bytes, maxLag: Math.min(MAX_STRIDE, Math.floor(n / 3)) }))
      .then((res) => {
        this.lag = { status: "done", r: res.r, peaks: res.peaks, sample: n };
        const best = res.peaks[0];
        if (best !== undefined && this.source === "default") this.setStride(best.lag, "peak");
        else this.changed();
      })
      .catch(() => {
        this.lag = { status: "short" };
        this.changed();
      });
  }
}

const states = new WeakMap<Doc, StrideState>();
function stateFor(doc: Doc): StrideState {
  let s = states.get(doc);
  if (s === undefined) {
    s = new StrideState(doc);
    states.set(doc, s);
  }
  return s;
}

/**
 * The record size of the template's widest run of equal-sized records: the
 * file's own structure and the structures directly in it, the first few
 * children of each. "pending" while the template is still being read.
 */
function templateGuess(doc: Doc): number | null | "pending" {
  const root = doc.templateNode([]);
  if (root.status !== "ok") return root.status === "error" ? null : "pending";
  const runs: { sizes: number[]; bits: number }[] = [];
  const look = (path: readonly number[], count: number, bits: number): readonly { readonly composite: boolean; readonly child_count: number; readonly size_bits: number; readonly path: readonly number[] }[] | null => {
    if (count < 2) return null;
    const kids = doc.templateChildren(path, 0, Math.min(16, count));
    if (kids.status !== "ok") return null;
    runs.push({ sizes: kids.node.map((k) => k.size_bits), bits });
    return kids.node;
  };
  const top = look([], root.node.child_count, root.node.size_bits);
  for (const k of (top ?? []).slice(0, 8)) if (k.composite) look(k.path, k.child_count, k.size_bits);
  return templateStride(runs);
}

/** The autocorrelation, in a worker where there is one. */
let worker: Worker | null = null;
let nextId = 1;
const waiting = new Map<number, (r: PeriodicityResult) => void>();
function runPeriodicity(req: PeriodicityRequest): Promise<PeriodicityResult> {
  if (typeof Worker === "undefined") return Promise.resolve(periodicity(req));
  try {
    if (worker === null) {
      worker = new Worker(new URL("./periodicityworker.ts", import.meta.url), { type: "module" });
      worker.onmessage = (e: MessageEvent<PeriodicityResult>) => {
        const done = waiting.get(e.data.id);
        waiting.delete(e.data.id);
        done?.(e.data);
      };
    }
  } catch {
    return Promise.resolve(periodicity(req));
  }
  const id = nextId++;
  const w = worker;
  return new Promise((resolve) => {
    waiting.set(id, resolve);
    w.postMessage({ ...req, id }, [req.bytes.buffer]);
  });
}

// ----- colours -----

type Rgb = readonly [number, number, number];

function parseColour(text: string, fallback: Rgb): Rgb {
  const t = text.trim();
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(t);
  if (m === null) return fallback;
  return [parseInt(m[1] as string, 16), parseInt(m[2] as string, 16), parseInt(m[3] as string, 16)];
}

type Palette = {
  readonly dark: boolean;
  readonly bg: Rgb;
  readonly fg: string;
  readonly muted: Rgb;
  readonly accent: string;
  readonly zero: Rgb;
  readonly text: Rgb;
  readonly other: Rgb;
};

/** The theme's colours, read afresh: the theme follows the system. */
function palette(): Palette {
  const css = getComputedStyle(document.body);
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  const bg = parseColour(css.getPropertyValue("--bg"), dark ? [23, 24, 28] : [255, 255, 255]);
  return {
    dark,
    bg,
    fg: css.getPropertyValue("--fg").trim() || (dark ? "#e6e7ea" : "#1b1b1f"),
    muted: parseColour(css.getPropertyValue("--muted"), [128, 128, 128]),
    accent: css.getPropertyValue("--accent").trim() || "#2457c5",
    // The byte classes the rail's map is painted in: zeros, text, and
    // structured data for the rest (`fieldstyle.ts`).
    zero: parseColour(byteClassColor(0), bg),
    text: parseColour(byteClassColor(2), [80, 160, 100]),
    other: parseColour(byteClassColor(3), [100, 140, 220]),
  };
}

function rgbText(c: Rgb): string {
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

// ----- d3 for the axes, loaded once -----

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
function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

function hex2(v: number): string {
  return `0x${v.toString(16).padStart(2, "0")}`;
}

// ----- the grid -----

type Row = { readonly bytes: Uint8Array; readonly complete: boolean };

export type StrideGrid = {
  readonly el: HTMLElement;
  /** Draw again: the bytes, the theme or the selection changed. */
  redraw(): void;
  /** Read again what had not arrived, and draw if anything had not. */
  pump(): void;
  resize(width: number, height: number): void;
  /** Whether the last drawing had bytes still on their way. */
  readonly waiting: boolean;
  /** Back to the whole file. */
  home(): void;
  destroy(): void;
};

/**
 * Draw the grid `width` by `height` CSS pixels, at the row width and colours
 * in `state`, which it follows. `rowAxis` puts the address of each row down
 * the left, when there is room for it. The grid never measures its host.
 */
function drawGrid(doc: Doc, state: StrideState, opts: { width: number; height: number; rowAxis: boolean; onZoomed?: (zoomed: boolean) => void; onWaiting?: (waiting: boolean) => void }): StrideGrid {
  const host = el("div", "sr-grid");
  const axisSvg = svg("svg", { class: "viz-axis sr-cols", height: AXIS_H });
  const body = el("div", "sr-grid-body");
  const rowSvg = svg("svg", { class: "viz-axis sr-rows" });
  const canvas = el("canvas", "sr-canvas");
  canvas.tabIndex = 0;
  canvas.setAttribute("role", "img");
  body.append(rowSvg, canvas);
  host.append(axisSvg, body);
  const axisG = svg("g", { transform: `translate(0,${AXIS_H - 1})` });
  axisSvg.append(axisG);
  const rowG = svg("g");
  rowSvg.append(rowG);

  let width = opts.width;
  let height = opts.height;
  let leftW = 0;
  let g: RasterGeometry = rasterGeometry(doc.lengthBytes, state.stride, 1, 1);
  let view: View = { k: 1, x: 0, y: 0 };
  let topRow = 0;
  let zoom: Board | null = null;
  let axis: Axis | null = null;
  let colours = palette();
  let waiting = false;
  let hover: { row: number; from: number; to: number } | null = null;
  let frame = 0;
  let buffer: ImageData | null = null;
  const bus = linkedFor(doc);

  // Rows of the current width, by row index.
  let cache = new Map<number, Row>();
  let cacheBytes = 0;
  let cacheStride = -1;
  const row = (r: number): Row => {
    if (cacheStride !== g.stride) {
      cache = new Map();
      cacheBytes = 0;
      cacheStride = g.stride;
    }
    const hit = cache.get(r);
    if (hit !== undefined) return hit;
    const at = r * g.stride;
    const n = Math.max(0, Math.min(g.stride, g.length - at));
    const got = doc.read(at, n);
    const value = { bytes: got.bytes, complete: got.complete };
    if (got.complete) {
      if (cacheBytes + n > CACHE_BYTES) {
        cache.clear();
        cacheBytes = 0;
      }
      cache.set(r, value);
      cacheBytes += n;
    }
    return value;
  };

  const layout = (): void => {
    leftW = opts.rowAxis && width >= ROW_AXIS_MIN_WIDTH ? ROW_AXIS_W : 0;
    const cw = Math.max(20, width - leftW);
    g = rasterGeometry(doc.lengthBytes, state.stride, cw, height);
    host.style.width = `${width}px`;
    axisSvg.setAttribute("width", String(width));
    axisG.setAttribute("transform", `translate(${leftW},${AXIS_H - 1})`);
    rowSvg.setAttribute("width", String(leftW));
    rowSvg.setAttribute("height", String(height));
    rowSvg.style.display = leftW > 0 ? "" : "none";
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(cw * dpr);
    canvas.height = Math.round(height * dpr);
    canvas.style.width = `${cw}px`;
    canvas.style.height = `${height}px`;
    canvas.setAttribute("aria-label", S.rasterLabel(g.stride));
  };

  /** A new board for a new geometry: its zoom limits are fixed when made. */
  const makeBoard = (): void => {
    zoom?.destroy();
    view = { k: 1, x: 0, y: 0 };
    topRow = 0;
    let prev: View = view;
    zoom = board(canvas, {
      width: g.width,
      height: g.height,
      minScale: 1,
      maxScale: maxZoom(g),
      // Rows are scrolled here, not by d3-zoom (see `stridemath.ts`), so its
      // y may go anywhere; only the change in it is used.
      content: { x0: 0, y0: -1e9, x1: g.width, y1: 1e9 },
      onZoom: (v) => {
        topRow = followZoom(g, topRow, prev, v);
        prev = v;
        view = v;
        // The cell under the pointer is a different one now; the next move
        // says which.
        if (hover !== null) {
          hover = null;
          hideTip();
        }
        opts.onZoomed?.(v.k > 1 + 1e-9 || topRow > 0 || Math.abs(v.x) > 0.5);
        schedule();
      },
    });
  };

  const schedule = (): void => {
    if (frame !== 0) return;
    if (typeof requestAnimationFrame !== "function") {
      paint();
      return;
    }
    frame = requestAnimationFrame(() => {
      frame = 0;
      paint();
    });
  };

  const paint = (): void => {
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    const dpr = canvas.width / Math.max(1, g.width);
    const W = canvas.width;
    const H = canvas.height;
    // One buffer per size, kept between frames: at twice the pixels over the
    // whole window a new one each wheel tick would be megabytes of garbage.
    if (buffer === null || buffer.width !== W || buffer.height !== H) buffer = ctx.createImageData(W, H);
    const img = buffer;
    const data = img.data;
    const c = colours;
    const rowH = rowHeight(g, view.k);
    const cw = g.fitX * view.k;
    const zeroCell = greyLevel(ZERO_CELL, c.dark, c.bg[0]);
    const cols = pixelColumns(g, view, W, 1 / dpr);
    // Device columns that are the last of a cell, drawn as the gap between
    // cells when cells are wide enough to tell apart.
    const gapCol = new Uint8Array(W);
    if (cw >= GAP_FROM_PX) {
      for (let px = 0; px < W; px++) {
        const a = columnAt(g, view, px / dpr);
        const b = columnAt(g, view, (px + 1) / dpr);
        if (Math.floor(a) !== Math.floor(b)) gapCol[px] = 1;
      }
    }
    // One line of colours per file row, made once per frame. A row still on
    // its way gets a second, hatched line, so unread bytes do not pass for
    // zeros.
    const lines = new Map<number, { line: Uint8ClampedArray; hatched: Uint8ClampedArray | null }>();
    let incomplete = false;
    const lineFor = (r: number): { line: Uint8ClampedArray; hatched: Uint8ClampedArray | null } => {
      const have = lines.get(r);
      if (have !== undefined) return have;
      const got = row(r);
      if (!got.complete) incomplete = true;
      const line = new Uint8ClampedArray(W * 4);
      for (let px = 0; px < W; px++) {
        const from = cols.from[px] as number;
        const to = Math.min(cols.to[px] as number, got.bytes.length);
        let rgb: Rgb = c.bg;
        if (from < to) {
          if (!got.complete) rgb = c.muted;
          else {
            const v = pixelValue(got.bytes, from, to);
            if (state.mode === "value") {
              // A zero is the background, except once cells are drawn apart,
              // when it takes a faint grey so the cells themselves show.
              const l = v.max === 0 && cw >= GAP_FROM_PX ? zeroCell : greyLevel(v.max, c.dark, c.bg[0]);
              rgb = [l, l, l];
            } else rgb = v.kind === OTHER ? c.other : v.kind === TEXT ? c.text : c.zero;
          }
          if (gapCol[px] === 1) rgb = c.bg;
        }
        const o = px * 4;
        line[o] = rgb[0];
        line[o + 1] = rgb[1];
        line[o + 2] = rgb[2];
        line[o + 3] = 255;
      }
      let hatched: Uint8ClampedArray | null = null;
      if (!got.complete) {
        hatched = new Uint8ClampedArray(line);
        for (let px = 0; px < W; px++) if ((cols.from[px] as number) < (cols.to[px] as number)) hatched.set([c.bg[0], c.bg[1], c.bg[2], 255], px * 4);
      }
      const made = { line, hatched };
      lines.set(r, made);
      return made;
    };
    const bgLine: Uint8ClampedArray = new Uint8ClampedArray(W * 4);
    for (let px = 0; px < W; px++) {
      const o = px * 4;
      bgLine[o] = c.bg[0];
      bgLine[o + 1] = c.bg[1];
      bgLine[o + 2] = c.bg[2];
      bgLine[o + 3] = 255;
    }
    const rowGap = rowH >= GAP_FROM_PX;
    for (let py = 0; py < H; py++) {
      const y = (py + 0.5) / dpr;
      const r = rowAt(g, rowH, topRow, y);
      let line = bgLine;
      if (r >= 0) {
        // The last device row of a row is the gap below it.
        const nextR = rowAt(g, rowH, topRow, (py + 1.5) / dpr);
        if (!(rowGap && nextR !== r)) {
          const made = lineFor(r);
          // Stripes four device pixels high across an unread row.
          line = made.hatched !== null && (py >> 2) % 2 === 1 ? made.hatched : made.line;
        }
      }
      data.set(line, py * W * 4);
    }
    ctx.putImageData(img, 0, 0);
    if (incomplete !== waiting) {
      waiting = incomplete;
      opts.onWaiting?.(waiting);
    }

    ctx.save();
    ctx.scale(dpr, dpr);
    const firstRow = Math.max(0, Math.floor(topRow));
    const lastRow = Math.min(g.rows - 1, Math.floor(topRow + g.height / rowH));
    // Each byte's two hex digits, once a cell has room for them. A fixed
    // pitch, so no digit can meet another.
    if (cw >= LOD.hexPx && rowH >= 12) {
      ctx.font = `${Math.min(11, Math.floor(rowH * 0.7))}px ${getComputedStyle(document.body).getPropertyValue("--mono") || "monospace"}`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      // The colour each byte's cell was drawn in, for the digit's ink.
      const cellColour = (v: number): Rgb => {
        if (state.mode === "kind") {
          const k = byteKind(v);
          return k === OTHER ? c.other : k === TEXT ? c.text : c.zero;
        }
        const l = v === 0 ? zeroCell : greyLevel(v, c.dark, c.bg[0]);
        return [l, l, l];
      };
      const c0 = Math.max(0, Math.floor(columnAt(g, view, 0)));
      const c1 = Math.min(g.stride, Math.ceil(columnAt(g, view, g.width)));
      for (let r = firstRow; r <= lastRow; r++) {
        const got = row(r);
        if (!got.complete) continue;
        const y = rowY(rowH, topRow, r) + rowH / 2;
        for (let col = c0; col < Math.min(c1, got.bytes.length); col++) {
          const v = got.bytes[col] as number;
          ctx.fillStyle = inkOn(cellColour(v)) === "black" ? "#000" : "#fff";
          ctx.fillText(v.toString(16).padStart(2, "0"), columnX(g, view, col) + cw / 2, y);
        }
      }
    }
    // The field the cursor is in, outlined row by row.
    const sel = bus.state.selection;
    if (sel !== null && sel.endBit > sel.startBit) {
      ctx.strokeStyle = c.accent;
      ctx.lineWidth = 2;
      outline(ctx, Math.floor(sel.startBit / 8), Math.ceil(sel.endBit / 8), rowH);
    }
    if (hover !== null) {
      ctx.strokeStyle = c.fg;
      ctx.lineWidth = 1;
      const at = hover.row * g.stride;
      outline(ctx, at + hover.from, at + hover.to, rowH);
    }
    ctx.restore();
    drawAxes(rowH, firstRow, lastRow);
  };

  /** Outline the bytes [a, b) (`rangeOutline`). A range inside one row is
   *  at least two pixels wide, so one byte of a long row still shows. */
  const outline = (ctx: CanvasRenderingContext2D, a: number, b: number, rowH: number): void => {
    const corners = rangeOutline(g.stride, a, b);
    if (corners.length === 0) return;
    const h = Math.max(2, rowH);
    const pts = corners.map((p) => ({ x: columnX(g, view, p.col), y: rowY(rowH, topRow, p.row) }));
    if (pts.length === 4) {
      const x0 = pts[0]?.x ?? 0;
      const x1 = Math.max(x0 + 2, pts[1]?.x ?? 0);
      const y0 = pts[0]?.y ?? 0;
      ctx.strokeRect(x0 + 0.5, y0 + 0.5, x1 - x0 - 1, h - 1);
      return;
    }
    ctx.beginPath();
    pts.forEach((p, i) => (i === 0 ? ctx.moveTo(p.x + 0.5, p.y + 0.5) : ctx.lineTo(p.x + 0.5, p.y + 0.5)));
    ctx.closePath();
    ctx.stroke();
  };

  const drawAxes = (rowH: number, firstRow: number, lastRow: number): void => {
    if (axis === null) return;
    const a = axis;
    const cols = rescaleDomain(view, [0, g.stride], [0, g.width]);
    const x = a.scale.scaleLinear().domain(cols).range([0, g.width]);
    const candidates = x.ticks(Math.max(2, Math.floor(g.width / 60))).filter((t) => Number.isInteger(t) && t >= 0 && t <= g.stride);
    const ticks = columnTicks(candidates, (t) => x(t), g.width, (t) => TICK_MEASURE(String(t), TICK_PX));
    const sel = a.selection.select(axisG);
    sel.call(
      a.axis
        .axisTop(x)
        .tickValues(ticks.map((t) => t.value))
        .tickSize(3)
        .tickFormat((d) => String(d)),
    );
    const anchors = new Map(ticks.map((t) => [t.value, t.anchor]));
    sel.selectAll<SVGTextElement, number>(".tick text").attr("text-anchor", (d) => anchors.get(Number(d)) ?? "middle");
    if (leftW === 0) return;
    // Each labelled row's first address, beside the middle of the row (the
    // scale is shifted half a row for that), as many as fit 28 pixels apart,
    // and none so near an end that its label would be cut.
    const y = a.scale
      .scaleLinear()
      .domain([topRow - 0.5, topRow - 0.5 + g.height / rowH])
      .range([0, g.height]);
    const rows = y.ticks(Math.max(2, Math.floor(g.height / 28))).filter((t) => Number.isInteger(t) && t >= firstRow && t <= lastRow && y(t) >= 6 && y(t) <= g.height - 6);
    a.selection.select(rowG).attr("transform", `translate(${leftW - 1},0)`).call(
      a.axis
        .axisLeft(y)
        .tickValues(rows)
        .tickSize(3)
        .tickFormat((d) => formatOffset(Number(d) * g.stride * 8)),
    );
  };

  /** The row and bytes under a pointer event, or null off the grid. */
  const under = (e: MouseEvent): { row: number; from: number; to: number } | null => {
    const rect = canvas.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    const r = rowAt(g, rowHeight(g, view.k), topRow, py);
    if (r < 0 || px < 0 || px > g.width) return null;
    const a = columnAt(g, view, px);
    const b = columnAt(g, view, px + 1);
    const rowLen = Math.min(g.stride, g.length - r * g.stride);
    const from = Math.max(0, Math.floor(a));
    const to = Math.min(rowLen, Math.max(from + 1, Math.ceil(b)));
    if (from >= rowLen) return null;
    return { row: r, from, to };
  };

  const card = (u: { row: number; from: number; to: number }): HTMLElement => {
    const at = u.row * g.stride;
    const got = row(u.row);
    const n = u.to - u.from;
    const lines: string[] = [];
    if (!got.complete) lines.push(S.unread);
    else if (n === 1) {
      const v = got.bytes[u.from] as number;
      lines.push(S.oneByte(hex2(v), v));
    } else lines.push(S.manyBytes(n, hex2(pixelValue(got.bytes, u.from, u.to).max)));
    lines.push(n === 1 ? S.rowColumn(u.row, u.from) : S.rowColumns(u.row, u.from, u.to - 1));
    const a = (at + u.from) * 8;
    const b = (at + u.to) * 8;
    const where = n === 1 ? formatOffset(a) : S.span(formatOffset(a), formatOffset(b - 8));
    const spans = doc.spans(a, b, 4);
    const field = spans.status === "ok" ? spans.node.find((s) => !s.gap && s.count === 0) ?? null : null;
    if (field === null) return tipCard(where, lines);
    return tipCard(field.name, lines, field.trail.length > 0 ? `${where} · ${field.trail.join(" › ")}` : where);
  };

  const stopTip = hoverTip(canvas, (e) => {
    const u = under(e);
    const same = u !== null && hover !== null && u.row === hover.row && u.from === hover.from && u.to === hover.to;
    if (!same) {
      hover = u;
      bus.publish({ hover: u === null ? null : { startBit: (u.row * g.stride + u.from) * 8, endBit: (u.row * g.stride + u.to) * 8 } });
      schedule();
    }
    return u === null ? null : card(u);
  });
  const leave = (): void => {
    hover = null;
    bus.publish({ hover: null });
    schedule();
  };
  canvas.addEventListener("pointerleave", leave);
  canvas.addEventListener("click", (e) => {
    const u = under(e);
    if (u === null) return;
    const at = u.row * g.stride;
    goToBits(doc, [{ startBit: (at + u.from) * 8, endBit: (at + u.to) * 8 }]);
  });
  canvas.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    // The keys are the grid's while it has focus, not the page's shortcuts.
    e.preventDefault();
    e.stopPropagation();
    state.setStride(state.stride + (e.key === "ArrowUp" ? 1 : -1), "reader");
  });
  const stopLinked = subscribeWhileConnected(bus, host, (_s, changed) => {
    if (changed === "selection") schedule();
  });
  const dark = matchMedia("(prefers-color-scheme: dark)");
  // The rail never says when it drops a panel, so the theme listener lets
  // go the first time it fires after the grid has left the page.
  const themed = (): void => {
    if (!host.isConnected) {
      dark.removeEventListener("change", themed);
      return;
    }
    colours = palette();
    schedule();
  };
  dark.addEventListener("change", themed);

  let drawnStride = -1;
  const stopState = state.listen(() => {
    if (state.stride !== drawnStride) {
      drawnStride = state.stride;
      layout();
      makeBoard();
    }
    schedule();
  });

  drawnStride = state.stride;
  layout();
  makeBoard();
  void loadAxis().then((loaded) => {
    axis = loaded;
    schedule();
  });
  paint();

  return {
    el: host,
    redraw: () => {
      colours = palette();
      schedule();
    },
    pump: () => {
      if (waiting) schedule();
    },
    resize: (w, h) => {
      if (w === width && h === height) return;
      width = w;
      height = h;
      layout();
      makeBoard();
      schedule();
    },
    get waiting() {
      return waiting;
    },
    home: () => {
      zoom?.home(false);
      topRow = 0;
      schedule();
    },
    destroy: () => {
      cancelAnimationFrame(frame);
      zoom?.destroy();
      stopTip();
      stopLinked();
      stopState();
      hideTip();
      dark.removeEventListener("change", themed);
      canvas.removeEventListener("pointerleave", leave);
    },
  };
}

// ----- the autocorrelation line -----

type UPlot = typeof import("uplot");
let uplotLoading: Promise<UPlot> | null = null;
/** uPlot and its styles, the first time a line is drawn. */
function loadUPlot(): Promise<UPlot> {
  uplotLoading ??= Promise.all([import("uplot"), import("uplot/dist/uPlot.min.css")]).then(([m]) => m.default);
  return uplotLoading;
}

/** Whether the ticks last made for each line were a round step apart. */
const linearTicks = new WeakMap<object, boolean>();

/** The lag ticks (`lagSplits`), for uPlot's axis. */
function lagAxisSplits(u: InstanceType<UPlot>, _axis: number, min: number, max: number): number[] {
  const { values, linear } = lagSplits(min, max);
  linearTicks.set(u, linear);
  return values;
}

/** The lag ticks to label (`lagTicks`), as uPlot's axis filter wants them:
 *  each tick kept, or null for one left bare. */
function lagAxisLabels(u: InstanceType<UPlot>, splits: number[]): (number | null)[] {
  const keep = new Set(
    lagTicks(
      splits,
      (v) => u.valToPos(v, "x"),
      (v) => TICK_MEASURE(v.toLocaleString(), TICK_PX),
      u.bbox.width / devicePixelRatio,
      linearTicks.get(u) === true,
    ),
  );
  return splits.map((v) => (keep.has(v) ? v : null));
}

export type LagLine = { readonly el: HTMLElement; resize(width: number): void; destroy(): void };

/**
 * The autocorrelation against lag, `width` wide, drawn when the worker
 * answers, with a mark at each peak and a line at the row width in use. The
 * wheel zooms the lags about the pointer and a drag pans them; hover says the
 * lag and its value; a click sets the row width to the peak nearest the
 * pointer.
 */
function drawLagLine(state: StrideState, opts: { width: number; height: number }): LagLine {
  const host = el("div", "sr-lag");
  const head = el("div", "sr-lag-head");
  const reset = el("button", "viz-reset", VIZ.resetZoom);
  reset.type = "button";
  reset.disabled = true;
  reset.addEventListener("click", () => zoom?.home(false));
  head.append(el("span", "sr-lag-title", S.autocorrelation), reset);
  const plotHost = el("div", "sr-lag-plot");
  const note = el("div", "sr-note");
  host.append(head, plotHost, note);
  let width = opts.width;
  let plot: InstanceType<UPlot> | null = null;
  let zoom: Board | null = null;
  let drawn: Float32Array | null = null;
  let stopTip: (() => void) | null = null;
  let destroyed = false;

  const teardown = (): void => {
    zoom?.destroy();
    zoom = null;
    reset.disabled = true;
    stopTip?.();
    stopTip = null;
    plot?.destroy();
    plot = null;
    drawn = null;
  };

  const build = async (): Promise<void> => {
    const lag = state.lag;
    if (lag.status !== "done") {
      teardown();
      plotHost.style.height = lag.status === "computing" ? `${opts.height}px` : "0px";
      reset.hidden = lag.status !== "computing";
      note.textContent = lag.status === "computing" ? S.computing : lag.status === "short" ? S.tooShort : "";
      return;
    }
    const maxLag = lag.r.length - 1;
    note.textContent = lag.peaks.length === 0 ? S.noRepeat(maxLag) : S.lagNote(lag.sample < state.length ? formatBytes(lag.sample) : null);
    if (drawn === lag.r && plot !== null) {
      plot.redraw(false, false);
      return;
    }
    const UPlot = await loadUPlot();
    if (destroyed) return;
    teardown();
    plotHost.style.height = "";
    reset.hidden = false;
    drawn = lag.r;
    const xs = new Float64Array(maxLag);
    const ys = new Float64Array(maxLag);
    for (let i = 1; i <= maxLag; i++) {
      xs[i - 1] = i;
      ys[i - 1] = lag.r[i] as number;
    }
    const peakYs: (number | null)[] = new Array<number | null>(maxLag).fill(null);
    for (const p of lag.peaks) peakYs[p.lag - 1] = p.r;
    const c = palette();
    const lineColour = c.dark ? "#7ea2ff" : "#2a78d6";
    const css = getComputedStyle(document.body);
    const muted = css.getPropertyValue("--muted").trim() || "#888";
    const mono = css.getPropertyValue("--mono").trim() || "monospace";
    const u = new UPlot(
      {
        width,
        height: opts.height,
        legend: { show: false },
        cursor: { drag: { x: false, y: false, setScale: false }, points: { show: false }, y: false },
        select: { show: false, left: 0, top: 0, width: 0, height: 0 },
        scales: { x: { time: false, distr: 3, log: 10, range: (_u, lo, hi) => [Math.max(1, lo), Math.min(maxLag, hi)] }, y: { range: (_u, lo, hi) => [Math.min(0, lo), Math.max(hi, 0.1)] } },
        axes: [
          {
            stroke: muted,
            font: `10px ${mono}`,
            size: 18,
            gap: 2,
            ticks: { size: 3, stroke: muted },
            grid: { show: false },
            splits: lagAxisSplits,
            filter: (self, splits) => lagAxisLabels(self, splits),
            values: (_u, vals) => vals.map((v) => (v === null ? "" : v.toLocaleString())),
          },
          { show: false },
        ],
        series: [
          {},
          { stroke: lineColour, width: 1.5, points: { show: false } },
          { stroke: lineColour, width: 0, paths: () => null, points: { show: true, size: 7, fill: lineColour, stroke: rgbText(c.bg), width: 1.5 } },
        ],
        hooks: {
          draw: [
            (self) => {
              // The row width in use, as a thin line across the plot.
              const x = self.valToPos(state.stride, "x", true);
              if (x < self.bbox.left || x > self.bbox.left + self.bbox.width) return;
              const ctx = self.ctx;
              ctx.save();
              ctx.strokeStyle = c.accent;
              ctx.lineWidth = Math.max(1, devicePixelRatio);
              ctx.beginPath();
              ctx.moveTo(x, self.bbox.top);
              ctx.lineTo(x, self.bbox.top + self.bbox.height);
              ctx.stroke();
              ctx.restore();
            },
          ],
        },
      },
      [Array.from(xs), Array.from(ys), peakYs],
      plotHost,
    );
    plot = u;
    const over = u.over;
    const plotW = (): number => u.bbox.width / devicePixelRatio;
    const logDomain: [number, number] = [0, Math.log10(maxLag)];
    zoom = board(over, {
      width: plotW(),
      height: u.bbox.height / devicePixelRatio,
      xOnly: true,
      minScale: 1,
      maxScale: Math.max(1, Math.log10(maxLag) * 40),
      onZoom: (v) => {
        reset.disabled = v.k <= 1 + 1e-9 && Math.abs(v.x) < 0.5;
        const [a, b] = rescaleDomain(v, logDomain, [0, plotW()]);
        u.setScale("x", { min: Math.pow(10, Math.max(0, a)), max: Math.pow(10, Math.min(logDomain[1], b)) });
      },
    });
    /** The lag a pointer is nearest a peak of: the highest lag within four
     *  pixels either side, so a click need not land on the one-pixel tip. */
    const lagAt = (e: MouseEvent): number | null => {
      const r = over.getBoundingClientRect();
      const x = e.clientX - r.left;
      if (x < 0 || x > r.width) return null;
      const lo = Math.max(1, Math.floor(u.posToVal(x - 4, "x")));
      const hi = Math.min(maxLag, Math.ceil(u.posToVal(x + 4, "x")));
      let best = lo;
      for (let l = lo; l <= hi; l++) if ((lag.r[l] as number) > (lag.r[best] as number)) best = l;
      return best;
    };
    // The lag the card last named, so a click where the pointer stopped sets
    // that lag, and not a neighbour half a pixel away.
    let shown: { x: number; lag: number } | null = null;
    stopTip = hoverTip(over, (e) => {
      const l = lagAt(e);
      shown = l === null ? null : { x: e.clientX, lag: l };
      if (l === null) return null;
      return tipCard(S.lagTitle(l), [S.lagValue(lag.r[l] as number)], S.lagClick(l));
    });
    over.addEventListener("click", (e) => {
      const l = shown !== null && Math.abs(shown.x - e.clientX) < 2 ? shown.lag : lagAt(e);
      if (l !== null) state.setStride(l, "reader");
    });
  };

  const stop = state.listen(() => void build());
  void build();
  return {
    el: host,
    resize: (w) => {
      if (w === width) return;
      width = w;
      drawn = null;
      void build();
    },
    destroy: () => {
      destroyed = true;
      stop();
      teardown();
    },
  };
}

// ----- the panel: the controls, the line and the grid -----

/** The Stride panel at a size: the rail's, or the window's in the large
 *  view. */
class StridePanel implements OverviewTabPanel {
  readonly el: HTMLElement;
  private readonly state: StrideState;
  private readonly input: HTMLInputElement;
  private readonly peaks: HTMLElement;
  private readonly modeSeg: HTMLElement;
  private readonly legend: HTMLElement;
  private readonly reset: HTMLButtonElement;
  private readonly grid: StrideGrid;
  private readonly line: LagLine;
  private readonly stop: () => void;
  private shown = true;

  constructor(
    private readonly doc: Doc,
    private width: number,
    private readonly large: { height: number; onClose: () => void } | null,
  ) {
    this.state = stateFor(doc);
    this.el = el("div", large === null ? "sr-panel" : "sr-panel is-large");

    const controls = el("div", "sr-controls");
    const label = el("label", "sr-perrow");
    const labelText = el("span", "", S.perRow);
    this.input = el("input", "sr-input");
    this.input.type = "number";
    this.input.min = "1";
    this.input.max = String(MAX_STRIDE);
    this.input.step = "1";
    this.input.title = S.perRowTitle;
    this.input.addEventListener("input", () => {
      const n = Number(this.input.value);
      if (Number.isFinite(n) && n >= 1) this.state.setStride(n, "reader");
    });
    label.append(labelText, this.input);
    controls.append(label);
    // Above the grid, beside the picture they act on: put the zoom back, and
    // in the rail, show the same panel over the window.
    const gridHead = el("div", "sr-grid-head");
    if (large === null) {
      const grow = el("button", "sr-grow", S.larger);
      grow.type = "button";
      grow.addEventListener("click", () => openLarge(doc));
      gridHead.append(grow);
    } else {
      const title = el("span", "sr-large-title", doc.name);
      const close = el("button", "sr-grow", S.close);
      close.type = "button";
      close.addEventListener("click", () => large.onClose());
      controls.prepend(title);
      controls.append(close);
    }

    this.peaks = el("div", "sr-peaks");
    this.line = drawLagLine(this.state, { width: this.contentWidth(), height: large === null ? LINE_H_RAIL : LINE_H_LARGE });

    this.reset = el("button", "viz-reset", VIZ.resetZoom);
    this.reset.type = "button";
    this.reset.disabled = true;
    this.reset.addEventListener("click", () => this.grid.home());
    this.grid = drawGrid(doc, this.state, {
      width: this.contentWidth(),
      height: this.gridHeight(),
      rowAxis: large !== null,
      onWaiting: () => this.drawLegend(),
      onZoomed: (z) => {
        this.reset.disabled = !z;
      },
    });

    const foot = el("div", "sr-foot");
    this.modeSeg = el("div", "seg sr-mode");
    this.modeSeg.setAttribute("role", "radiogroup");
    this.modeSeg.setAttribute("aria-label", S.colourBy);
    for (const [value, text] of [
      ["value", S.byValue],
      ["kind", S.byKind],
    ] as const) {
      const b = el("button", "", text);
      b.type = "button";
      b.dataset.mode = value;
      b.setAttribute("role", "radio");
      b.addEventListener("click", () => this.state.setMode(value));
      this.modeSeg.append(b);
    }
    this.legend = el("div", "sr-legend");
    foot.append(this.modeSeg, this.legend);
    gridHead.prepend(this.reset);

    this.el.append(controls, this.line.el, this.peaks, gridHead, this.grid.el, foot);
    this.stop = this.state.listen(() => this.sync());
    this.state.guessFromTemplate();
    this.state.startLag();
    this.sync();
  }

  /** The width to draw in. The rail may ask before it has been laid out, and
   *  then says NaN or 0; the rail's usual width stands in until it resizes. */
  private contentWidth(): number {
    return Number.isFinite(this.width) && this.width > 0 ? Math.max(80, Math.floor(this.width)) : RAIL_FALLBACK_W;
  }

  private gridHeight(): number {
    if (this.large !== null) return rasterHeight(Math.ceil(this.doc.lengthBytes / this.state.stride), 160, Math.max(160, this.large.height));
    return rasterHeight(Math.ceil(this.doc.lengthBytes / this.state.stride), RAIL_MIN_H, RAIL_MAX_H);
  }

  /** The controls and legend, to match the state. */
  private sync(): void {
    if (document.activeElement !== this.input || Number(this.input.value) !== this.state.stride) this.input.value = String(this.state.stride);
    for (const b of this.modeSeg.children) b.setAttribute("aria-checked", String((b as HTMLElement).dataset.mode === this.state.mode));
    // The peaks as buttons: one press to try a row width.
    const lag = this.state.lag;
    const buttons: HTMLElement[] = [];
    if (lag.status === "done" && lag.peaks.length > 0) {
      buttons.push(el("span", "sr-peaks-lead", S.suggested));
      for (const p of lag.peaks) {
        const b = el("button", "sr-peak", S.peakButton(p.lag));
        b.type = "button";
        b.title = S.peakTitle(p.lag);
        b.setAttribute("aria-pressed", String(p.lag === this.state.stride));
        b.addEventListener("click", () => this.state.setStride(p.lag, "reader"));
        buttons.push(b);
      }
    }
    this.peaks.replaceChildren(...buttons);
    this.peaks.hidden = buttons.length === 0;
    this.drawLegend();
    this.grid.resize(this.contentWidth(), this.gridHeight());
  }

  private drawLegend(): void {
    // The grid's first drawing reports on its bytes before the legend exists.
    if ((this.legend as HTMLElement | undefined) === undefined) return;
    const c = palette();
    const items: HTMLElement[] = [];
    if (this.state.mode === "value") {
      const ramp = el("span", "sr-ramp");
      const lo = greyLevel(0, c.dark, c.bg[0]);
      const hi = greyLevel(255, c.dark, c.bg[0]);
      ramp.style.background = `linear-gradient(to right, rgb(${lo},${lo},${lo}), rgb(${hi},${hi},${hi}))`;
      items.push(el("span", "sr-mono", S.low), ramp, el("span", "sr-mono", S.high));
    } else {
      for (const [rgb, text] of [
        [c.zero, S.zero],
        [c.text, S.text],
        [c.other, S.other],
      ] as const) {
        const item = el("span", "sr-key");
        const sw = el("span", "sr-swatch");
        sw.style.background = rgbText(rgb);
        item.append(sw, text);
        items.push(item);
      }
    }
    if (this.grid?.waiting) {
      const item = el("span", "sr-key");
      item.append(el("span", "sr-swatch is-unread"), S.unread);
      items.push(item);
    }
    this.legend.replaceChildren(...items);
  }

  setShown(shown: boolean): void {
    if (shown && !this.shown) this.grid.redraw();
    this.shown = shown;
  }

  resize(width: number): void {
    if (!Number.isFinite(width) || width <= 0 || Math.floor(width) === Math.floor(this.width)) return;
    this.width = width;
    this.line.resize(this.contentWidth());
    this.grid.resize(this.contentWidth(), this.gridHeight());
  }

  pump(): void {
    if (!this.shown) return;
    this.state.guessFromTemplate();
    this.grid.pump();
  }

  destroy(): void {
    this.stop();
    this.grid.destroy();
    this.line.destroy();
  }
}

/** The panel over the whole window, until Close or Escape. */
function openLarge(doc: Doc): void {
  const back = el("div", "sr-overlay");
  back.setAttribute("role", "dialog");
  back.setAttribute("aria-label", S.tab);
  const box = el("div", "sr-overlay-box");
  back.append(box);
  const pad = 24;
  const w = window.innerWidth - 2 * pad - 32;
  // The controls, the line and its note, the peaks, the zoom buttons, the
  // column ticks and the foot take about this much.
  const h = window.innerHeight - 2 * pad - 32 - (LINE_H_LARGE + 230);
  const close = (): void => {
    panel.destroy();
    back.remove();
    window.removeEventListener("keydown", key, true);
  };
  const key = (e: KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  };
  const panel = new StridePanel(doc, w, { height: h, onClose: close });
  box.append(panel.el);
  back.addEventListener("click", (e) => {
    if (e.target === back) close();
  });
  window.addEventListener("keydown", key, true);
  document.body.append(back);
  (panel.el.querySelector(".sr-canvas") as HTMLElement | null)?.focus();
}

/** The Stride tab in the rail, for any file with bytes in it. */
export function register(): void {
  registerOverviewTab({
    id: "stride",
    label: S.tab,
    title: S.tabTitle,
    offers: (doc) => doc.lengthBytes > 0,
    mount: (doc, width) => new StridePanel(doc, width, null),
  });
}

// A numeric run as a plot: the samples of a WAV as a waveform, a tensor's
// weights as a line or a grid of colours, a miniSEED record's decoded samples.
//
// Candidate `value-plot` in docs/DESIGN-diagrams.md, section 4.2; built by
// agent K of the first batch (section 5.3). The listing, the hex view and the
// list pane all show a run of numbers one value at a time, as text, and a run
// of twenty-four thousand samples is not something anybody reads as text: the
// silence, the clipping, the spike, the layout of a weight matrix are the
// shape of the whole run, and only a picture of the whole run shows them.
//
// Three forms of one piece, all drawn at a size the caller gives:
// - The line (`drawLinePlot`). One lane per channel, stacked over one x axis,
//   on one y scale so the channels can be compared: interleaved stereo is two
//   lanes, a mono run or a tensor is one. Drawn by uPlot, with the run
//   decimated to the pixels it has (`decimate.ts`: the lowest and highest of
//   each pixel's share, so a spike survives every zoom). The board (`board.ts`,
//   x only) zooms about the pointer and pans; every change of view decimates
//   the visible stretch again, so zooming in is more detail and not bigger
//   pixels, down to one sample a few dozen pixels wide.
// - The grid (`drawGridPlot`), for a run of two dimensions such as a GGUF
//   tensor's weight matrix: one cell per value, coloured on a diverging scale
//   around zero, with a colour bar. Wider than the pixels it has, it shows
//   each pixel's mean and says so above the grid. Zooms both ways.
// - The record's samples (`samplesSection`), for a miniSEED record in the At
//   cursor panel. A Steim sample is the sum of the differences before it, so
//   it is at no one place in the file; hovering one lights the 64-byte frame
//   its difference was read from, and a click selects that frame.
//
// Every element is a route to its bytes: hover publishes the element's bits
// on the shared bus (`linked.ts`) so the hex view marks them, a click selects
// the element as a row of the listing would (`pick.ts`), and the element the
// cursor is on is marked on the plot whichever view moved the cursor.
//
// Where the numbers come from: the run's own bytes, read with `Doc.read` and
// decoded with a DataView by the element type the core names (`i16 le[]`),
// since `runCells` writes each element as text and a plot wants millions of
// them. A run whose bytes are elsewhere (inside a decoded stream) is read with
// `fieldBytes`. Steim samples come decoded from the core (`SamplesInfo`),
// which gives the first 32 of them.

import "../toggleseg.css";
import "./valueplot.css";
import type uPlot from "uplot";
import type { Doc, FieldPick, SamplesInfo, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { listPaneFor, setPanePlotter, type PaneFigure } from "../listpane.ts";
import { frameOfSample, showingLine } from "../samplespanel.ts";
import { childWord, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { rescaleDomain, type View } from "./boardmath.ts";
import { decode, elementType, envelope, extentOf, finiteRange, frameCount, nearestFrame, numType, paddedExtent, rangeOf, valueText, type Channel, type NumType } from "./decimate.ts";
import { headingButton, registerHeadingButton } from "./headingbuttons.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { linkedFor, subscribeWhileConnected, type BitRange } from "./linked.ts";
import { goToBits, goToField } from "./pick.ts";
import { onSchemeChange } from "./schemewatch.ts";
import { hideTip, hoverTip, tipAt, tipCard } from "./tip.ts";
import { drawGridPlot } from "./valuegrid.ts";

// ----- the run -----

/** The most bytes of one run the plot reads. A run past this is plotted from
 *  its start, and the plot says how much of it that is. Sixteen MiB is eight
 *  million 16-bit samples, three minutes of CD audio, decoded in well under a
 *  second. */
const MAX_PLOT_BYTES = 16 << 20;
/** Most channels drawn as lanes. A table of more numeric columns than this
 *  has outgrown lanes, and is the Table view's to show. */
const MAX_LANES = 8;

/** Whether `node` is a run the plot can draw, from the node alone, since the
 *  listing asks this of every heading on every paint: a list of fixed-width
 *  numbers, packed one after another from a whole byte. */
export function plottable(node: TemplateNode): boolean {
  if (!node.composite || node.list !== true || node.child_count < 2) return false;
  const t = numType(elementType(node.type));
  if (t === null) return false;
  return node.offset_bits % 8 === 0 && !node.lsb_first && node.child_count * t.bytes * 8 === node.size_bits;
}

/** A run, and how to read it. */
export type RunSource = {
  readonly path: readonly number[];
  readonly node: TemplateNode;
  readonly type: NumType;
  /** Elements to one frame: the channels of interleaved audio. One for a run
   *  that is not a table. */
  readonly stride: number;
  /** A name per channel, or one empty name when there is one channel. */
  readonly names: readonly string[];
  /** What the template calls one frame of a table: a sample. Null for a run
   *  that is not a table, whose elements are numbered as the listing numbers
   *  them. */
  readonly rowWord: string | null;
  /** What one element is called, for counting them: values, weights. */
  readonly unit: string;
  /** Frames a second, when the run is spaced in time. */
  readonly rate: number | null;
  /** The run's two dimensions, for a tensor that says it has two: `columns`
   *  values to a row, stored row after row. */
  readonly dims: { readonly rows: number; readonly columns: number } | null;
};

/**
 * Everything the plot needs to know about the run at `path` beyond its bytes:
 * the channels the template's table shape gives it (a WAV's `channels`), and
 * the two dimensions a tensor's element count is the product of (a GGUF
 * tensor's `dims`). Null for a run the plot cannot draw.
 */
export function runSource(doc: Doc, path: readonly number[]): RunSource | null {
  const reply = doc.templateNode(path);
  if (reply.status !== "ok" || !plottable(reply.node)) return null;
  const node = reply.node;
  const type = numType(elementType(node.type));
  if (type === null) return null;
  let stride = 1;
  let names: string[] = [""];
  let rowWord: string | null = null;
  let rate: number | null = null;
  const shape = node.table === true ? doc.tableShape(path) : null;
  if (shape !== null && shape.status === "ok" && shape.node !== null) {
    const s = shape.node;
    const columns = s.columns ?? 1;
    if (columns > 1 && columns <= MAX_LANES && node.child_count % columns === 0) {
      stride = columns;
      names = Array.from({ length: columns }, (_, i) => (s.names.length === columns ? (s.names[i] ?? "") : `${s.column_word ?? "column"} ${i + 1}`));
    }
    if (s.row_word !== null) rowWord = s.row_word;
    if (s.rate !== null && s.rate > 0) rate = s.rate;
  }
  return { path, node, type, stride, names, rowWord, unit: childWord(node), rate, dims: stride === 1 ? dimsOf(doc, path, node.child_count) : null };
}

/**
 * The two dimensions of a run whose element count the template works out as
 * the product of a list of sizes: a GGUF tensor's `dims`, which are written
 * innermost first, so the first is the length of a row. Null for a run of one
 * dimension, or of more than two, which have no one grid to draw.
 */
function dimsOf(doc: Doc, path: readonly number[], count: number): { rows: number; columns: number } | null {
  const origins = doc.origins(path);
  if (origins.status !== "ok") return null;
  for (const o of origins.node) {
    if (o.role !== "count") continue;
    const list = doc.templateNode(o.path);
    if (list.status !== "ok" || !list.node.composite || list.node.child_count < 2) continue;
    const kids = doc.templateChildren(o.path, 0, Math.min(8, list.node.child_count));
    if (kids.status !== "ok") continue;
    const sizes = kids.node.map((k) => Number(k.value));
    if (sizes.some((s) => !Number.isInteger(s) || s < 1)) continue;
    const big = sizes.filter((s) => s > 1);
    if (big.length !== 2 || sizes.reduce((a, b) => a * b, 1) !== count) continue;
    const columns = sizes[0] === 1 ? (big[0] as number) : (sizes[0] as number);
    return { rows: count / columns, columns };
  }
  return null;
}

/** The decoded run, or null while its bytes are still arriving. */
export type RunValues = {
  readonly values: Float64Array;
  /** How many elements of the run the values are: all of them, unless the run
   *  is longer than `MAX_PLOT_BYTES`. */
  readonly shown: number;
};

function readRun(doc: Doc, src: RunSource): RunValues | null {
  const n = src.node;
  const frameBytes = src.type.bytes * src.stride;
  const total = n.child_count * src.type.bytes;
  const want = Math.min(total, Math.floor(MAX_PLOT_BYTES / frameBytes) * frameBytes);
  let bytes: Uint8Array;
  if (n.space === doc.space && !n.joined) {
    const r = doc.read(n.offset_bits / 8, want);
    if (!r.complete) return null;
    bytes = r.bytes;
  } else {
    const r = doc.fieldBytes(src.path, want);
    if (r.status !== "ok") return null;
    bytes = Uint8Array.from(r.node.bytes);
  }
  const shown = Math.floor(bytes.length / src.type.bytes);
  return { values: decode(bytes, src.type, shown), shown };
}

// ----- formatting -----

const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumSignificantDigits: 3 });
const plain = new Intl.NumberFormat(undefined, { maximumSignificantDigits: 4 });

/** A y-axis tick: compact from ten thousand up (12K), otherwise as written. */
export function tickText(v: number): string {
  return Math.abs(v) >= 10_000 ? compact.format(v) : plain.format(v);
}

/** Seconds to as many decimals as a tick step of `step` seconds needs. */
export function secondsText(s: number, step: number): string {
  const decimals = step >= 1 ? 0 : Math.min(6, Math.ceil(-Math.log10(step) - 1e-9));
  return `${s.toFixed(decimals)} s`;
}

/** Tick steps in frames that are round numbers of seconds at `rate`, or
 *  round numbers of frames with no rate. */
function xIncrs(rate: number | null): number[] {
  const out: number[] = [];
  for (let e = -6; e <= 12; e++) {
    for (const m of [1, 2, 5]) {
      // Written as a quotient for the negative powers, so 0.1 is 1 / 10 and
      // not 1 * 0.1000000001.
      const step = e < 0 ? m / 10 ** -e : m * 10 ** e;
      const frames = rate === null ? step : step * rate;
      // A tick between two frames of a run with no rate has nothing to mark.
      if (frames >= 1 && (rate !== null || Number.isInteger(step))) out.push(frames);
    }
  }
  return out;
}

/** A colour from the figure's own stylesheet variables, so the canvas follows
 *  the theme the page is in. */
function cssVar(el: Element, name: string, fallback: string): string {
  const v = getComputedStyle(el).getPropertyValue(name).trim();
  return v === "" ? fallback : v;
}

let uplotLoading: Promise<typeof uPlot> | null = null;
/** uPlot and its stylesheet, once, the first time a plot is drawn. A load
 *  that fails is not kept, so the next plot asks again: Firefox fetches the
 *  module again, while Chrome answers with the failure it already has until
 *  the page is reloaded, which is what the plot then says. */
function loadUPlot(): Promise<typeof uPlot> {
  if (uplotLoading === null) {
    const loading = Promise.all([import("uplot"), import("uplot/dist/uPlot.min.css")]).then(([m]) => m.default);
    loading.catch(() => {
      if (uplotLoading === loading) uplotLoading = null;
    });
    uplotLoading = loading;
  }
  return uplotLoading;
}

const SERIES_VARS = ["--vp-1", "--vp-2", "--vp-3", "--vp-4", "--vp-5", "--vp-6", "--vp-7", "--vp-8"];
const SERIES_FALLBACK = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#6f7887"];

// ----- the line -----

/** What the line is drawn from. Built from a run (`lineFromRun`) or from a
 *  record's decoded samples (`samplesSection`). */
export type LineData = {
  readonly channels: readonly Channel[];
  /** A name per channel; one empty name when there is one. */
  readonly names: readonly string[];
  /** What a frame is called, or null to number frames as `[31]`. */
  readonly rowWord: string | null;
  /** What one element is called, for counting them. */
  readonly unit: string;
  readonly rate: number | null;
  readonly float: boolean;
  /** Where frame `frame` of channel `channel` is: what a click selects, the
   *  bits a hover lights, and the foot of the hover card. */
  readonly locate: (frame: number, channel: number) => { readonly pick: FieldPick | null; readonly bits: BitRange; readonly foot: string };
  /** The frame and channel the cursor is on, from the shared selection. */
  readonly selected: (sel: FieldPick | null) => { readonly frame: number; readonly channel: number } | null;
};

export type LinePlot = { readonly el: HTMLElement; readonly board: Board; resize(width: number, height: number): void; destroy(): void };

/** The width of the y axis, the same in every lane so the lanes line up. */
const Y_AXIS_PX = 58;
/** The x axis under the last lane. */
const X_AXIS_PX = 26;
/** Room above each lane's line for the lane's name. */
const LANE_TOP_PX = 18;
const PAD_RIGHT_PX = 10;
/** Shortest a lane gets; more lanes than fit at this make the figure taller
 *  than it was asked to be rather than unreadable. */
const MIN_LANE_PX = 56;
/** How wide one frame may get at the deepest zoom. */
const MAX_FRAME_PX = 32;
/** Fewer frames a pixel than this (five pixels a frame), and each frame gets
 *  a marker as well, so a short run reads as the points it is. */
const MARKER_FRAMES_PER_PX = 1 / 5;

/** A pointer position, from a pointer event or remembered from the last one. */
type Pointer = { readonly clientX: number; readonly clientY: number };

/** Draw the line `width` by `height` pixels. `range` is the lowest and
 *  highest finite value, for a caller that has already worked it out. */
export function drawLinePlot(
  doc: Doc,
  data: LineData,
  opts: { readonly width: number; readonly height: number; readonly range?: { readonly min: number; readonly max: number } | null },
): LinePlot {
  const lanes = Math.max(1, data.channels.length);
  const frames = Math.max(1, ...data.channels.map(frameCount));
  let width = Math.max(120, opts.width);
  let laneH = Math.max(MIN_LANE_PX, Math.floor((Math.max(80, opts.height) - X_AXIS_PX) / lanes));
  const plotLeft = Y_AXIS_PX;
  let plotW = Math.max(20, width - plotLeft - PAD_RIGHT_PX);
  const totalH = (): number => laneH * lanes + X_AXIS_PX;
  // A little room above the highest value and below the lowest, so a peak is
  // not drawn on the edge of the lane.
  const [lowest, highest] = opts.range === undefined ? extentOf(data.channels) : paddedExtent(opts.range);
  const pad = (highest - lowest) * 0.04;
  const [y0, y1] = [lowest - pad, highest + pad];

  const host = document.createElement("div");
  host.className = "vp-lanes";
  host.style.width = `${width}px`;
  host.style.height = `${totalH()}px`;
  const syncKey = `vp-${Math.random().toString(36).slice(2)}`;

  let view: View = { k: 1, x: 0, y: 0 };
  /** Frames on screen, as fractional frame numbers. */
  let span: [number, number] = [0, frames - 1];
  const bus = linkedFor(doc);
  let sel = data.selected(bus.state.selection);
  const plots: uPlot[] = [];
  const nameEls: HTMLElement[] = [];
  let destroyed = false;
  /** Where the pointer is while it is over the plot with no button down, so
   *  a zoom by the wheel can redo the hover card for the new scale. */
  let pointer: Pointer | null = null;

  data.names.forEach((name, i) => {
    if (name === "") return;
    const tag = document.createElement("span");
    tag.className = "vp-lane-name";
    const sw = document.createElement("span");
    sw.className = "vp-swatch";
    sw.style.background = `var(${SERIES_VARS[i % SERIES_VARS.length]})`;
    tag.append(sw, name);
    host.append(tag);
    nameEls.push(tag);
  });
  const placeNames = (): void => {
    let n = 0;
    data.names.forEach((name, i) => {
      if (name === "") return;
      const tag = nameEls[n++];
      if (tag === undefined) return;
      tag.style.left = `${plotLeft + 4}px`;
      tag.style.top = `${i * laneH + 2}px`;
    });
  };
  placeNames();

  const xValues = (_u: uPlot, splits: number[], _ax: number, _space: number, incr: number): string[] =>
    data.rate === null ? splits.map((s) => Math.round(s).toLocaleString()) : splits.map((s) => secondsText(s / (data.rate as number), incr / (data.rate as number)));

  const paintMarker = (u: uPlot, lane: number): void => {
    if (sel === null) return;
    const x = u.valToPos(sel.frame, "x", true);
    const b = u.bbox;
    if (x < b.left - 1 || x > b.left + b.width + 1) return;
    const ctx = u.ctx;
    const ratio = b.width / Math.max(1, plotW);
    ctx.save();
    ctx.strokeStyle = cssVar(host, "--accent", "#2457c5");
    ctx.lineWidth = Math.max(1, ratio);
    ctx.beginPath();
    ctx.moveTo(x, b.top);
    ctx.lineTo(x, b.top + b.height);
    ctx.stroke();
    if (lane === sel.channel) {
      const c = data.channels[lane];
      const v = c === undefined ? NaN : (c.values[sel.frame * c.stride + c.channel] ?? NaN);
      if (Number.isFinite(v)) {
        const y = u.valToPos(v, "y", true);
        ctx.beginPath();
        ctx.arc(x, y, 4 * ratio, 0, Math.PI * 2);
        ctx.fillStyle = cssVar(host, SERIES_VARS[lane % SERIES_VARS.length] as string, SERIES_FALLBACK[lane % SERIES_FALLBACK.length] as string);
        ctx.strokeStyle = cssVar(host, "--bg", "#fff");
        ctx.lineWidth = 2 * ratio;
        ctx.fill();
        ctx.stroke();
      }
    }
    ctx.restore();
  };

  /** uPlot's crosshair is an element over the canvas, so where it stands on
   *  the cursor's marker it would hide it: there it takes the marker's
   *  colour. */
  const markCursor = (u: uPlot): void => {
    const left = u.cursor.left ?? -1;
    const on = sel !== null && left >= 0 && Math.abs(u.valToPos(sel.frame, "x") - left) < 1.5;
    u.root.classList.toggle("is-on-marker", on);
  };

  const laneData = (lane: number): uPlot.AlignedData => {
    const c = data.channels[lane];
    if (c === undefined) return [[], []];
    const env = envelope(c, span[0], span[1] + 1, plotW);
    return [env.frames, env.values];
  };

  const showPoints = (): boolean => (span[1] - span[0]) / plotW < MARKER_FRAMES_PER_PX;

  const build = (U: typeof uPlot): void => {
    if (destroyed) return;
    const font = `10px ${cssVar(host, "--mono", "monospace")}`;
    const muted = (): string => cssVar(host, "--muted", "#6b6f76");
    const line = (): string => cssVar(host, "--line", "#dcdfe4");
    for (let lane = 0; lane < lanes; lane++) {
      const last = lane === lanes - 1;
      const el = document.createElement("div");
      el.style.position = "absolute";
      el.style.left = "0";
      el.style.top = `${lane * laneH}px`;
      host.append(el);
      const colour = (): string => cssVar(host, SERIES_VARS[lane % SERIES_VARS.length] as string, SERIES_FALLBACK[lane % SERIES_FALLBACK.length] as string);
      const u = new U(
        {
          width,
          height: laneH + (last ? X_AXIS_PX : 0),
          padding: [LANE_TOP_PX, PAD_RIGHT_PX, last ? 0 : 2, 0],
          legend: { show: false },
          scales: { x: { time: false, auto: false }, y: { auto: false, range: [y0, y1] } },
          axes: [
            {
              show: last,
              size: X_AXIS_PX,
              font,
              stroke: muted,
              grid: { stroke: line, width: 1 },
              ticks: { stroke: line, width: 1, size: 3 },
              space: 84,
              incrs: xIncrs(data.rate),
              values: xValues,
            },
            {
              size: Y_AXIS_PX,
              font,
              stroke: muted,
              grid: { stroke: line, width: 1 },
              ticks: { stroke: line, width: 1, size: 3 },
              space: 22,
              values: (_u, splits) => splits.map(tickText),
            },
          ],
          series: [
            {},
            {
              stroke: colour,
              width: 1.25,
              spanGaps: false,
              points: { show: showPoints, size: 6, stroke: colour, fill: colour },
            },
          ],
          cursor: { drag: { x: false, y: false, setScale: false }, points: { show: false }, y: false, sync: { key: syncKey } },
          hooks: { draw: [(self) => paintMarker(self, lane)], setCursor: [markCursor] },
        },
        laneData(lane),
        el,
      );
      u.setScale("x", { min: span[0], max: span[1] });
      plots.push(u);
    }
  };

  const redrawAll = (): void => {
    plots.forEach((u, lane) => {
      u.batch(() => {
        u.setData(laneData(lane), false);
        u.setScale("x", { min: span[0], max: span[1] });
      });
    });
  };

  const zoom = board(host, {
    width,
    height: totalH(),
    xOnly: true,
    maxScale: Math.max(1, (frames * MAX_FRAME_PX) / plotW),
    content: { x0: plotLeft, y0: 0, x1: plotLeft + plotW, y1: totalH() },
    onZoom: (v) => {
      view = v;
      const [a, b] = rescaleDomain(v, [0, frames - 1], [plotLeft, plotLeft + plotW]);
      span = [a, b];
      redrawAll();
      // The card under a still pointer described the old scale: how many
      // frames the pixel holds, and their range.
      if (pointer !== null) {
        const c = card(pointer);
        if (c === null) hideTip();
        else tipAt(c, pointer.clientX, pointer.clientY);
      }
    },
  });

  // uPlot is loaded the first time a plot is drawn. If it does not arrive,
  // the plot says so in place of the lanes.
  loadUPlot().then(build, () => {
    if (destroyed) return;
    const box = document.createElement("div");
    box.className = "vp-failed";
    box.textContent = VIZ.valuePlot.loadFailed;
    // Not a press on the plot: no pan, no zoom, and no click through to the
    // bytes.
    for (const type of ["pointerdown", "mousedown", "click", "wheel"]) box.addEventListener(type, (e) => e.stopPropagation());
    host.append(box);
  });

  /** The frame and lane under a pointer, or null off the plot. */
  const under = (e: Pointer): { frame: number; lane: number; x: number } | null => {
    const r = host.getBoundingClientRect();
    const x = e.clientX - r.left - plotLeft;
    const y = e.clientY - r.top;
    if (x < -4 || x > plotW + 4 || y < 0 || y > laneH * lanes) return null;
    const at = span[0] + (x / plotW) * (span[1] - span[0]);
    return { frame: nearestFrame(at, frames), lane: Math.max(0, Math.min(lanes - 1, Math.floor(y / laneH))), x };
  };

  const card = (e: Pointer): HTMLElement | null => {
    const hit = under(e);
    if (hit === null) {
      bus.publish({ hover: null });
      return null;
    }
    const spot = data.locate(hit.frame, hit.lane);
    bus.publish({ hover: spot.bits });
    // A run the template calls a table has a word for its rows (a sample),
    // and the card leads with it; any other run is numbered as the listing
    // numbers its elements.
    const title =
      data.rowWord === null
        ? VIZ.valuePlot.index(hit.frame)
        : data.rate === null
          ? VIZ.valuePlot.at(data.rowWord, hit.frame)
          : VIZ.valuePlot.atTime(data.rowWord, hit.frame, secondsText(hit.frame / data.rate, 1 / data.rate));
    const lines: string[] = [];
    data.channels.forEach((c, i) => {
      const v = valueText(c.values[hit.frame * c.stride + c.channel] ?? NaN, data.float);
      const name = data.names[i] ?? "";
      lines.push(name === "" ? VIZ.valuePlot.oneValue(v) : VIZ.valuePlot.channelValue(name, v));
    });
    // One pixel of the line stands for every frame under it; the card says
    // which, and the lowest and highest, since that is what is drawn there.
    const per = (span[1] - span[0]) / plotW;
    if (per > 1) {
      const px = Math.floor(hit.x);
      const from = Math.max(0, Math.floor(span[0] + px * per));
      const to = Math.min(frames, Math.floor(span[0] + (px + 1) * per));
      if (to - from > 1) {
        lines.push(VIZ.valuePlot.inPixel(data.rowWord ?? data.unit, from, to - 1));
        data.channels.forEach((c, i) => {
          const r = rangeOf(c, from, to);
          if (r === null) return;
          const lo = valueText(r.min, data.float);
          const hi = valueText(r.max, data.float);
          const name = data.names[i] ?? "";
          lines.push(name === "" ? VIZ.valuePlot.oneRange(lo, hi) : VIZ.valuePlot.channelRange(name, lo, hi));
        });
      }
    }
    return tipCard(title, lines, spot.foot);
  };
  const stopTip = hoverTip(host, card);
  const move = (e: PointerEvent): void => {
    pointer = e.buttons === 0 ? { clientX: e.clientX, clientY: e.clientY } : null;
  };
  const press = (): void => {
    pointer = null;
  };
  const leave = (): void => {
    pointer = null;
    bus.publish({ hover: null });
  };
  host.addEventListener("pointermove", move);
  host.addEventListener("pointerdown", press);
  host.addEventListener("pointerleave", leave);
  const click = (e: MouseEvent): void => {
    const hit = under(e);
    if (hit === null) return;
    const spot = data.locate(hit.frame, hit.lane);
    if (spot.pick !== null) goToField(doc, spot.pick);
    else goToBits(doc, [spot.bits]);
  };
  host.addEventListener("click", click);

  // The element the cursor is on is marked, whichever view moved the cursor,
  // and brought into view when the plot is zoomed in somewhere else.
  const stopLinked = subscribeWhileConnected(bus, host, (state, changed) => {
    if (changed !== "selection") return;
    sel = data.selected(state.selection);
    if (sel !== null && (sel.frame < span[0] || sel.frame > span[1]) && view.k > 1) {
      const lx = plotLeft + (sel.frame / Math.max(1, frames - 1)) * plotW;
      const half = width / (2 * view.k);
      zoom.zoomTo({ x0: lx - half, x1: lx + half, y0: 0, y1: totalH() }, true);
    }
    for (const u of plots) {
      u.redraw(false);
      markCursor(u);
    }
  });

  // The canvas takes its colours from the stylesheet when it paints; a change
  // of theme is a repaint.
  const stopScheme = onSchemeChange(host, () => {
    for (const u of plots) u.redraw(false, true);
  });

  return {
    el: host,
    board: zoom,
    resize: (w, h) => {
      width = Math.max(120, w);
      laneH = Math.max(MIN_LANE_PX, Math.floor((Math.max(80, h) - X_AXIS_PX) / lanes));
      plotW = Math.max(20, width - plotLeft - PAD_RIGHT_PX);
      host.style.width = `${width}px`;
      host.style.height = `${totalH()}px`;
      plots.forEach((u, lane) => {
        const el = u.root.parentElement;
        if (el !== null) el.style.top = `${lane * laneH}px`;
        u.setSize({ width, height: laneH + (lane === lanes - 1 ? X_AXIS_PX : 0) });
      });
      placeNames();
      zoom.resize(width, totalH(), { x0: plotLeft, y0: 0, x1: plotLeft + plotW, y1: totalH() });
    },
    destroy: () => {
      destroyed = true;
      zoom.destroy();
      stopTip();
      stopLinked();
      hideTip();
      stopScheme();
      host.removeEventListener("pointermove", move);
      host.removeEventListener("pointerdown", press);
      host.removeEventListener("pointerleave", leave);
      host.removeEventListener("click", click);
      for (const u of plots) u.destroy();
    },
  };
}

/** The line for a run: its channels, and every element at its own place. */
export function lineFromRun(src: RunSource, run: RunValues): LineData {
  const n = src.node;
  const bits = src.type.bytes * 8;
  const channels = src.names.map((_, channel) => ({ values: run.values, stride: src.stride, channel }));
  const runPath = src.path;
  return {
    channels,
    names: src.names,
    rowWord: src.rowWord,
    unit: src.unit,
    rate: src.rate,
    float: src.type.float,
    locate: (frame, channel) => {
      const index = frame * src.stride + channel;
      const startBit = n.offset_bits + index * bits;
      const range = { startBit, endBit: startBit + bits };
      // The element's index only where it is not the frame number the card
      // leads with: interleaved channels.
      const foot =
        src.stride > 1 ? VIZ.valuePlot.element(index, formatOffset(startBit), src.type.name) : VIZ.valuePlot.where(formatOffset(startBit), src.type.name);
      return { pick: { path: [...runPath, index], ...range }, bits: range, foot };
    },
    selected: (s) => {
      if (s === null || s.path.length !== runPath.length + 1 || !runPath.every((step, i) => s.path[i] === step)) return null;
      const index = s.path[runPath.length] as number;
      if (index >= run.shown) return null;
      return { frame: Math.floor(index / src.stride), channel: index % src.stride };
    },
  };
}

// ----- the figure in the list pane -----

/**
 * The plot of the run at `path`, for the list pane: a heading row with what
 * is drawn and its controls, and the line or the grid under it. Reads the run
 * itself, and says it is reading until the bytes have all arrived.
 */
export function paneFigure(doc: Doc, path: readonly number[], width: number, height: number): PaneFigure {
  const root = document.createElement("figure");
  root.className = "vp";
  const head = document.createElement("figcaption");
  head.className = "vp-head";
  const note = document.createElement("span");
  note.className = "vp-note vp-title";
  // The choices and Reset zoom stay together on one line, which goes under
  // the note when the pane is too narrow for both.
  const tools = document.createElement("span");
  tools.className = "vp-tools";
  const controls = document.createElement("span");
  controls.className = "vp-controls";
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  reset.disabled = true;
  tools.append(controls, reset);
  head.append(note, tools);
  const body = document.createElement("div");
  const reading = document.createElement("div");
  reading.className = "vp-reading";
  reading.textContent = VIZ.valuePlot.reading;
  body.append(reading);
  root.append(head, body);

  let size = { width, height };
  let src = runSource(doc, path);
  let run: RunValues | null = null;
  /** The run's lowest and highest value, worked out once when it is read:
   *  the line's y scale, and the note over it. */
  let range: { readonly min: number; readonly max: number } | null = null;
  let mode: "line" | "grid" = src?.dims !== null && src?.dims !== undefined ? "grid" : "line";
  let clip = false;
  let drawn: { resize(w: number, h: number): void; destroy(): void; board: Board } | null = null;
  let stopWatch: (() => void) | null = null;
  let frame = 0;
  /** The head row's height before it is on the page, so the plot under it
   *  gets the rest. Once it is on the page its own height is used, since its
   *  note wraps in a narrow pane. This is the figure's own row, not the host. */
  const HEAD_PX = 28;

  const bodySize = (): { width: number; height: number } => ({
    width: size.width,
    height: Math.max(80, size.height - (head.offsetHeight > 0 ? head.offsetHeight + 4 : HEAD_PX)),
  });

  const seg = (choices: { label: string; title: string; on: boolean; press: () => void }[]): HTMLElement => {
    const s = document.createElement("span");
    s.className = "toggle-seg";
    for (const c of choices) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = c.label;
      b.title = c.title;
      b.setAttribute("aria-pressed", String(c.on));
      b.addEventListener("click", c.press);
      s.append(b);
    }
    return s;
  };

  const draw = (): void => {
    if (src === null || run === null) return;
    drawn?.destroy();
    drawn = null;
    const s = src;
    const r = run;
    const notes: string[] = [];
    if (r.shown < s.node.child_count) notes.push(VIZ.valuePlot.firstOf(r.shown, s.node.child_count, childWord(s.node)));
    const parts: HTMLElement[] = [];
    if (s.dims !== null && r.shown === s.node.child_count) {
      parts.push(
        seg([
          { label: VIZ.valuePlot.grid, title: VIZ.valuePlot.gridTitle, on: mode === "grid", press: () => setMode("grid") },
          { label: VIZ.valuePlot.line, title: VIZ.valuePlot.lineTitle, on: mode === "line", press: () => setMode("line") },
        ]),
      );
    }
    if (mode === "grid" && s.dims !== null && r.shown === s.node.child_count) {
      notes.unshift(VIZ.valuePlot.shape(s.dims.rows, s.dims.columns));
      parts.push(
        seg([
          { label: VIZ.valuePlot.rangeFull, title: VIZ.valuePlot.rangeFullTitle, on: !clip, press: () => setClip(false) },
          { label: VIZ.valuePlot.rangeClip, title: VIZ.valuePlot.rangeClipTitle, on: clip, press: () => setClip(true) },
        ]),
      );
      const g = drawGridPlot(doc, s, r, { ...bodySize(), clip, onNote: (text) => (note.textContent = [...notes, text].filter((t) => t !== "").join(" · ")) });
      drawn = g;
      body.replaceChildren(g.el);
    } else {
      const data = lineFromRun(s, r);
      // The sentence that goes with the picture: how low and how high the
      // values go, which the y axis otherwise leaves to be read off.
      range ??= finiteRange(data.channels);
      if (range !== null) notes.push(VIZ.valuePlot.oneRange(valueText(range.min, s.type.float), valueText(range.max, s.type.float)));
      const line = drawLinePlot(doc, data, { ...bodySize(), range });
      drawn = line;
      body.replaceChildren(line.el);
    }
    controls.replaceChildren(...parts);
    note.textContent = notes.join(" · ");
    note.title = note.textContent;
    const b = drawn.board;
    const onReset = (): void => b.home(true);
    reset.onclick = onReset;
    reset.disabled = false;
  };

  const setMode = (m: "line" | "grid"): void => {
    mode = m;
    draw();
  };
  const setClip = (c: boolean): void => {
    clip = c;
    draw();
  };

  const attempt = (): void => {
    src ??= runSource(doc, path);
    if (src === null) return;
    run = readRun(doc, src);
    if (run === null) return;
    range = null;
    stopWatch?.();
    stopWatch = null;
    draw();
  };
  attempt();
  // Bytes still arriving: try again once a frame has passed after each chunk,
  // however many land in it.
  if (run === null) {
    stopWatch = doc.onChange(() => {
      if (frame !== 0) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        attempt();
      });
    });
  }

  return {
    el: root,
    resize: (w, h) => {
      size = { width: w, height: h };
      drawn?.resize(bodySize().width, bodySize().height);
    },
    destroy: () => {
      cancelAnimationFrame(frame);
      stopWatch?.();
      drawn?.destroy();
    },
  };
}

// ----- a miniSEED record's samples, in the At cursor panel -----

/** The At cursor panel's figure is this tall: a glance at the record, with the
 *  pane for anything longer. */
const SECTION_PX = 150;

/** The last field asked about and its answer, since the inspector asks
 *  `applies` and then `draw` of the same field on one render. */
let lastAsked: { doc: Doc; key: string; info: SamplesInfo | null } | null = null;

/** The documents whose edits already forget `lastAsked`. */
const watched = new WeakSet<Doc>();

function samplesAt(doc: Doc, path: readonly number[]): SamplesInfo | null {
  if (!watched.has(doc)) {
    watched.add(doc);
    // An edit to a Steim word changes the samples, and the bytes arriving can
    // turn a record that would not decode into one that does.
    doc.onChange(() => {
      if (lastAsked?.doc === doc) lastAsked = null;
    });
  }
  const key = path.join(".");
  if (lastAsked !== null && lastAsked.doc === doc && lastAsked.key === key) return lastAsked.info;
  const reply = doc.typeInfo(path);
  const info = reply.status === "ok" && reply.node.kind === "samples" ? reply.node : null;
  lastAsked = { doc, key, info };
  return info;
}

/** Where the record's data starts: the field around `path` that is exactly the
 *  data the samples were decoded from. */
function dataStart(doc: Doc, path: readonly number[], info: SamplesInfo): number | null {
  for (let n = path.length; n > 0; n--) {
    const r = doc.templateNode(path.slice(0, n));
    if (r.status === "ok" && r.node.size_bits === info.bytes * 8) return r.node.offset_bits;
  }
  return null;
}

/** A Steim frame is 64 bytes, by the SEED manual. */
const STEIM_FRAME_BITS = 64 * 8;

function samplesSection(doc: Doc, path: readonly number[], width: number): HTMLElement | null {
  const info = samplesAt(doc, path);
  if (info === null || info.values.length < 2) return null;
  const values = Float64Array.from(info.values, Number);
  const start = dataStart(doc, path, info);
  const root = document.createElement("figure");
  root.className = "vp";
  const head = document.createElement("figcaption");
  head.className = "vp-head";
  const title = document.createElement("span");
  title.className = "vp-title";
  title.textContent = VIZ.valuePlot.samplesTitle;
  const note = document.createElement("span");
  note.className = "vp-note";
  // Only when the plot is short of the record: the panel's own heading above
  // already gives the count.
  const showing = showingLine(info);
  note.textContent = showing ?? "";
  note.hidden = showing === null;
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  head.append(title, reset);
  /** Sample 0 is the forward integration constant, the second 32-bit word of
   *  frame 0; every other sample is lit by the frame its difference is in. */
  const constant = info.steim && info.x0 !== null && start !== null;
  const frameBits = (frame: number): BitRange | null => {
    if (frame === 0 && constant && start !== null) return { startBit: start + 32, endBit: start + 64 };
    const f = info.steim ? frameOfSample(info, frame) : null;
    if (f === null || start === null) return null;
    return { startBit: start + f * STEIM_FRAME_BITS, endBit: start + (f + 1) * STEIM_FRAME_BITS };
  };
  const data: LineData = {
    channels: [{ values, stride: 1, channel: 0 }],
    names: [""],
    rowWord: "sample",
    unit: "sample",
    rate: null,
    float: false,
    locate: (frame) => {
      const bits = frameBits(frame) ?? { startBit: start ?? 0, endBit: (start ?? 0) + info.bytes * 8 };
      const f = info.steim ? frameOfSample(info, frame) : null;
      const at = formatOffset(bits.startBit);
      const foot = frame === 0 && constant ? VIZ.valuePlot.fromConstant(at) : f === null ? at : VIZ.valuePlot.fromFrame(f, at);
      return { pick: null, bits, foot };
    },
    // A sample is not a field of its own, so no selection is one.
    selected: () => null,
  };
  const line = drawLinePlot(doc, data, { width, height: SECTION_PX });
  reset.addEventListener("click", () => line.board.home(true));
  root.append(head, line.el, note);
  return root;
}

// ----- registration -----

/** Add the plot to the listing's headings, the list pane and the At cursor
 *  panel. Called once, from `register.ts`. */
export function register(): void {
  setPanePlotter({ applies: plottable, draw: paneFigure });
  registerHeadingButton({
    id: "value-plot",
    applies: (_c, node) => plottable(node),
    draw: (c, node) => {
      const pane = listPaneFor(c.doc);
      const on = pane !== undefined && pane.showing(node.path) === "plot";
      return headingButton(VIZ.valuePlot.button, VIZ.valuePlot.buttonTitle(node.child_count, childWord(node)), on, () => {
        const p = listPaneFor(c.doc);
        if (p === undefined) return;
        if (p.showing(node.path) === "plot") p.close();
        else p.open(node.path, "plot");
      });
    },
  });
  registerInspectorSection({
    id: "value-plot-samples",
    applies: (doc, node) => samplesAt(doc, node.path) !== null,
    // The record's data, so moving between its frames keeps the figure and
    // its zoom; and the samples, so an edit that changes them draws again.
    key: (doc, node, path) => {
      const info = samplesAt(doc, path);
      return info === null ? path.join(".") : `samples:${dataStart(doc, path, info) ?? path.join(".")}:${info.total}:${info.values.join(",")}`;
    },
    draw: ({ doc, path, width }) => samplesSection(doc, path, width),
  });
}

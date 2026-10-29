// The line of a value plot: one lane per channel over one x axis, drawn by
// uPlot from a run decimated to the pixels it has.
//
// Part of candidate `value-plot` (`valueplot.ts` has the whole piece's
// story). Used by the list pane's figure (`valuepane.ts`) for a run, and by
// the At cursor panel's figure of a miniSEED record (`valuesamples.ts`) for
// samples the core decoded. Both hand it a `LineData`, which says where each
// frame's bytes are, so the line itself never knows which it is drawing.
//
// The board (`board.ts`, x only) zooms about the pointer and pans; every
// change of view decimates the visible stretch again (`decimate.ts`), so
// zooming in is more detail and not bigger pixels. uPlot is loaded the first
// time a line is drawn.

import type uPlot from "uplot";
import type { Doc, FieldPick } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { rescaleDomain, type View } from "./boardmath.ts";
import { envelope, extentOf, frameCount, nearestFrame, paddedExtent, rangeOf, valueText, type Channel } from "./decimate.ts";
import { linkedFor, subscribeWhileConnected, type BitRange } from "./linked.ts";
import { goToBits, goToField } from "./pick.ts";
import { onSchemeChange } from "./schemewatch.ts";
import { hideTip, hoverTip, tipAt, tipCard } from "./tip.ts";
import type { RunSource, RunValues } from "./valuerun.ts";

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
export function cssVar(el: Element, name: string, fallback: string): string {
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

export const SERIES_VARS = ["--vp-1", "--vp-2", "--vp-3", "--vp-4", "--vp-5", "--vp-6", "--vp-7", "--vp-8"];
export const SERIES_FALLBACK = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#6f7887"];

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
    // Not the plot: no hover card, no pan, no zoom, and no click through to
    // the bytes.
    for (const type of ["pointermove", "pointerdown", "mousedown", "click", "wheel"]) box.addEventListener(type, (e) => e.stopPropagation());
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

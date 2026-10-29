// The grid form of `valueplot.ts`: a run of two dimensions, such as a GGUF
// tensor's weight matrix, as one cell of colour per value.
//
// Part of candidate `value-plot` in docs/DESIGN-diagrams.md. A line of a
// weight matrix shows the values in storage order and nothing of their
// layout; the grid puts each value at its row and column, so a row of large
// weights, a dead column or a block pattern from quantisation shows as what it
// is. The cells fill the room they are given, so a tall matrix is not a
// sliver; the axes say which row and column is where.
//
// Drawn on a canvas, one pixel of the canvas per pixel of the view, from
// `sampleGrid` in `gridmath.ts`: zoomed out each pixel is the mean of the
// block of values under it, and the figure says how many values that is;
// zoomed in, each value is a block of pixels drawn without smoothing. The
// board (`board.ts`) zooms both ways about the pointer and pans. Row and
// column ticks and the colour bar's ticks come from d3-axis, which spaces
// them; values are only in the hover card.

import type { Doc } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { applyX, applyY, invertX, invertY, type View } from "./boardmath.ts";
import { valueText } from "./decimate.ts";
import { colourRange, parseHex, pixelBlock, ramp, rampIndex, sampleGrid, type Rgb } from "./gridmath.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { goToField } from "./pick.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";
import type { RunSource, RunValues } from "./valueplot.ts";

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

/** The figure's parts, left to right and top to bottom. */
const ROW_AXIS_PX = 44;
const COL_AXIS_PX = 22;
const TOP_PX = 4;
const BAR_GAP_PX = 12;
const BAR_PX = 12;
const BAR_AXIS_PX = 56;
/** A cell is never drawn larger than this at the deepest zoom. */
const MAX_CELL_PX = 40;
/** Room per tick on each axis, so d3 is asked for as many as fit. */
const TICK_X_PX = 56;
const TICK_Y_PX = 28;

function cssVar(el: Element, name: string, fallback: string): string {
  const v = getComputedStyle(el).getPropertyValue(name).trim();
  return v === "" ? fallback : v;
}

export type GridPlot = { readonly el: HTMLElement; readonly board: Board; resize(width: number, height: number): void; destroy(): void };

export function drawGridPlot(
  doc: Doc,
  src: RunSource,
  run: RunValues,
  opts: { readonly width: number; readonly height: number; readonly clip: boolean; readonly onNote: (text: string) => void },
): GridPlot {
  const dims = src.dims ?? { rows: 1, columns: run.shown };
  const { rows, columns } = dims;
  const values = run.values;
  let width = Math.max(160, opts.width);
  let height = Math.max(120, opts.height);
  let gw = Math.max(20, width - ROW_AXIS_PX - BAR_GAP_PX - BAR_PX - BAR_AXIS_PX);
  let gh = Math.max(20, height - COL_AXIS_PX - TOP_PX);
  const range = colourRange(values, opts.clip);

  const host = document.createElement("div");
  host.style.position = "relative";
  const canvas = document.createElement("canvas");
  canvas.className = "vp-grid";
  canvas.style.position = "absolute";
  canvas.style.left = `${ROW_AXIS_PX}px`;
  canvas.style.top = `${TOP_PX}px`;
  canvas.style.imageRendering = "pixelated";
  const bar = document.createElement("canvas");
  bar.className = "vp-bar";
  bar.style.position = "absolute";
  bar.style.top = `${TOP_PX}px`;
  const overlay = svg("svg", { class: "viz-axis" });
  overlay.style.position = "absolute";
  overlay.style.left = "0";
  overlay.style.top = "0";
  overlay.style.pointerEvents = "none";
  overlay.style.overflow = "visible";
  const colAxis = svg("g");
  const rowAxis = svg("g");
  const barAxis = svg("g");
  const marker = svg("rect", { fill: "none", "stroke-width": 2, visibility: "hidden" });
  overlay.append(colAxis, rowAxis, barAxis, marker);
  host.append(canvas, bar, overlay);

  let view: View = { k: 1, x: 0, y: 0 };
  let axis: Axis | null = null;
  let per = 1;
  const bus = linkedFor(doc);
  const runPath = src.path;
  const elementBits = src.type.bytes * 8;
  const selectedIndex = (): number | null => {
    const s = bus.state.selection;
    if (s === null || s.path.length !== runPath.length + 1 || !runPath.every((step, i) => s.path[i] === step)) return null;
    return s.path[runPath.length] as number;
  };
  let selected = selectedIndex();

  const colours = (): { table: Uint8ClampedArray; n: number } => {
    const rgb = (name: string, fallback: string): Rgb => parseHex(cssVar(host, name, fallback)) ?? (parseHex(fallback) as Rgb);
    const n = 256;
    return { table: ramp(rgb("--vp-neg", "#2a78d6"), rgb("--vp-mid", "#f0efec"), rgb("--vp-pos", "#eb6834"), range.diverging, n), n };
  };

  /** The rows and columns on screen, as fractional cell numbers. */
  const visible = (): { c0: number; c1: number; r0: number; r1: number } => ({
    c0: (invertX(view, 0) / gw) * columns,
    c1: (invertX(view, gw) / gw) * columns,
    r0: (invertY(view, 0) / gh) * rows,
    r1: (invertY(view, gh) / gh) * rows,
  });

  const layout = (): void => {
    host.style.width = `${width}px`;
    host.style.height = `${height}px`;
    canvas.width = gw;
    canvas.height = gh;
    canvas.style.width = `${gw}px`;
    canvas.style.height = `${gh}px`;
    bar.width = BAR_PX;
    bar.height = gh;
    bar.style.left = `${ROW_AXIS_PX + gw + BAR_GAP_PX}px`;
    bar.style.width = `${BAR_PX}px`;
    bar.style.height = `${gh}px`;
    overlay.setAttribute("width", String(width));
    overlay.setAttribute("height", String(height));
  };

  const paintBar = (): void => {
    const ctx = bar.getContext("2d");
    if (ctx === null) return;
    const { table, n } = colours();
    const img = ctx.createImageData(BAR_PX, gh);
    for (let y = 0; y < gh; y++) {
      const i = rampIndex(1 - y / Math.max(1, gh - 1), 0, 1, n) * 4;
      for (let x = 0; x < BAR_PX; x++) img.data.set(table.subarray(i, i + 4), (y * BAR_PX + x) * 4);
    }
    ctx.putImageData(img, 0, 0);
  };

  const paintMarker = (): void => {
    if (selected === null || selected >= rows * columns) {
      marker.setAttribute("visibility", "hidden");
      return;
    }
    const r = Math.floor(selected / columns);
    const c = selected % columns;
    let x0 = applyX(view, (c / columns) * gw);
    let x1 = applyX(view, ((c + 1) / columns) * gw);
    let y0 = applyY(view, (r / rows) * gh);
    let y1 = applyY(view, ((r + 1) / rows) * gh);
    // Never smaller than a box the eye finds: a cell a pixel wide is marked by
    // a box around it.
    const grow = (a: number, b: number): [number, number] => (b - a >= 6 ? [a, b] : [(a + b) / 2 - 3, (a + b) / 2 + 3]);
    [x0, x1] = grow(x0, x1);
    [y0, y1] = grow(y0, y1);
    const visibleBox = x1 > 0 && x0 < gw && y1 > 0 && y0 < gh;
    marker.setAttribute("visibility", visibleBox ? "visible" : "hidden");
    marker.setAttribute("x", String(ROW_AXIS_PX + x0));
    marker.setAttribute("y", String(TOP_PX + y0));
    marker.setAttribute("width", String(x1 - x0));
    marker.setAttribute("height", String(y1 - y0));
    marker.setAttribute("stroke", cssVar(host, "--fg", "#1b1b1f"));
  };

  const drawAxes = (): void => {
    if (axis === null) return;
    const v = visible();
    const intTicks = (lo: number, hi: number, count: number, limit: number): number[] => {
      const s = axis!.scale.scaleLinear().domain([lo, hi]);
      return s.ticks(count).filter((t) => Number.isInteger(t) && t >= 0 && t < limit);
    };
    const x = axis.scale.scaleLinear().domain([v.c0, v.c1]).range([0, gw]);
    const y = axis.scale.scaleLinear().domain([v.r0, v.r1]).range([0, gh]);
    // A tick marks the middle of its column or row, where the cell is.
    const cx = (t: number): number => x(t + 0.5);
    const cy = (t: number): number => y(t + 0.5);
    const xs = axis.selection.select(colAxis).attr("transform", `translate(${ROW_AXIS_PX},${TOP_PX + gh})`);
    xs.call(
      axis.axis
        .axisBottom(axis.scale.scaleLinear().domain([0, gw]).range([0, gw]))
        .tickValues(intTicks(v.c0, v.c1, Math.max(2, Math.floor(gw / TICK_X_PX)), columns).map(cx).filter((p) => p >= 0 && p <= gw))
        .tickFormat((d) => Math.round(x.invert(Number(d)) - 0.5).toLocaleString())
        .tickSize(3),
    );
    const ys = axis.selection.select(rowAxis).attr("transform", `translate(${ROW_AXIS_PX},${TOP_PX})`);
    ys.call(
      axis.axis
        .axisLeft(axis.scale.scaleLinear().domain([0, gh]).range([0, gh]))
        .tickValues(intTicks(v.r0, v.r1, Math.max(2, Math.floor(gh / TICK_Y_PX)), rows).map(cy).filter((p) => p >= 0 && p <= gh))
        .tickFormat((d) => Math.round(y.invert(Number(d)) - 0.5).toLocaleString())
        .tickSize(3),
    );
    const b = axis.scale.scaleLinear().domain([range.lo, range.hi]).range([gh, 0]);
    axis.selection
      .select(barAxis)
      .attr("transform", `translate(${ROW_AXIS_PX + gw + BAR_GAP_PX + BAR_PX},${TOP_PX})`)
      .call(
        axis.axis
          .axisRight(b)
          .ticks(Math.max(2, Math.floor(gh / 36)))
          .tickFormat((d) => {
            const n = Number(d);
            return Math.abs(n) >= 10_000 ? n.toExponential(1) : String(Number(n.toPrecision(3)));
          })
          .tickSize(3),
      );
  };

  const paint = (): void => {
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    const sample = sampleGrid(values, columns, rows, visible(), gw, gh);
    per = sample.per;
    const { table, n } = colours();
    const img = ctx.createImageData(gw, gh);
    const px = img.data;
    for (let i = 0; i < sample.means.length; i++) {
      const m = sample.means[i] as number;
      if (Number.isNaN(m)) continue;
      const at = rampIndex(m, range.lo, range.hi, n) * 4;
      px[i * 4] = table[at] as number;
      px[i * 4 + 1] = table[at + 1] as number;
      px[i * 4 + 2] = table[at + 2] as number;
      px[i * 4 + 3] = 255;
    }
    ctx.putImageData(img, 0, 0);
    opts.onNote(per > 1 ? VIZ.valuePlot.meanOf(per) : "");
    paintMarker();
    drawAxes();
  };

  layout();
  const zoom = board(canvas, {
    width: gw,
    height: gh,
    maxScale: Math.max(1, MAX_CELL_PX / Math.min(gw / columns, gh / rows)),
    onZoom: (v) => {
      view = v;
      paint();
    },
  });
  void loadAxis().then((loaded) => {
    axis = loaded;
    drawAxes();
  });
  paintBar();

  /** The cell under a pointer event, and the block of cells its pixel shows. */
  const under = (e: MouseEvent): { row: number; column: number; block: { rows: [number, number]; cols: [number, number] } } | null => {
    const r = canvas.getBoundingClientRect();
    const px = Math.floor(e.clientX - r.left);
    const py = Math.floor(e.clientY - r.top);
    if (px < 0 || py < 0 || px >= gw || py >= gh) return null;
    const column = Math.floor((invertX(view, px + 0.5) / gw) * columns);
    const row = Math.floor((invertY(view, py + 0.5) / gh) * rows);
    if (column < 0 || column >= columns || row < 0 || row >= rows) return null;
    const v = visible();
    return { row, column, block: { rows: pixelBlock(v.r0, v.r1, gh, py, rows), cols: pixelBlock(v.c0, v.c1, gw, px, columns) } };
  };

  const stopTip = hoverTip(canvas, (e) => {
    const hit = under(e);
    if (hit === null) {
      bus.publish({ hover: null });
      return null;
    }
    const index = hit.row * columns + hit.column;
    const startBit = src.node.offset_bits + index * elementBits;
    bus.publish({ hover: { startBit, endBit: startBit + elementBits } });
    const lines = [VIZ.valuePlot.oneValue(valueText(values[index] ?? NaN, src.type.float))];
    const n = (hit.block.rows[1] - hit.block.rows[0]) * (hit.block.cols[1] - hit.block.cols[0]);
    if (n > 1) {
      let sum = 0;
      let k = 0;
      for (let r = hit.block.rows[0]; r < hit.block.rows[1]; r++) {
        for (let c = hit.block.cols[0]; c < hit.block.cols[1]; c++) {
          const v = values[r * columns + c] ?? NaN;
          if (Number.isFinite(v)) {
            sum += v;
            k++;
          }
        }
      }
      if (k > 0) lines.push(VIZ.valuePlot.cellMean(n, valueText(sum / k, true)));
    }
    return tipCard(VIZ.valuePlot.cell(hit.row, hit.column), lines, VIZ.valuePlot.element(index, formatOffset(startBit), src.type.name));
  });
  const leave = (): void => bus.publish({ hover: null });
  canvas.addEventListener("pointerleave", leave);
  const click = (e: MouseEvent): void => {
    const hit = under(e);
    if (hit === null) return;
    const index = hit.row * columns + hit.column;
    const startBit = src.node.offset_bits + index * elementBits;
    goToField(doc, { path: [...runPath, index], startBit, endBit: startBit + elementBits });
  };
  canvas.addEventListener("click", click);

  const stopLinked = subscribeWhileConnected(bus, host, (_state, changed) => {
    if (changed !== "selection") return;
    selected = selectedIndex();
    paintMarker();
  });

  const scheme = typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)") : null;
  const repaint = (): void => {
    paintBar();
    paint();
  };
  scheme?.addEventListener("change", repaint);

  return {
    el: host,
    board: zoom,
    resize: (w, h) => {
      width = Math.max(160, w);
      height = Math.max(120, h);
      gw = Math.max(20, width - ROW_AXIS_PX - BAR_GAP_PX - BAR_PX - BAR_AXIS_PX);
      gh = Math.max(20, height - COL_AXIS_PX - TOP_PX);
      layout();
      paintBar();
      zoom.resize(gw, gh, { x0: 0, y0: 0, x1: gw, y1: gh });
      paint();
    },
    destroy: () => {
      zoom.destroy();
      stopTip();
      stopLinked();
      hideTip();
      scheme?.removeEventListener("change", repaint);
      canvas.removeEventListener("pointerleave", leave);
      canvas.removeEventListener("click", click);
    },
  };
}

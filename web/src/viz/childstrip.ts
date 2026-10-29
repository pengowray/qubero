// The example piece: the structure the cursor is in, its fields drawn as bars
// over the bytes they take up, in an inspector section.
//
// Built with piece zero to prove the shared parts work together on real data,
// and kept as the worked example the batch pieces copy. Everything a piece is
// asked to do is here once, in the order a piece does it:
// - the data: read the Doc by path, answer null while the bytes are on their
//   way (`childStripData`);
// - the layout, DOM-free: a layout pixel for every bar, lanes so two fields
//   that overlap never share a row (`childStripLayout`, `lanes.ts`);
// - the size-independent draw: `drawChildStrip(doc, data, {width})` returns
//   `{el, destroy}` and never measures anything;
// - pan and zoom over the address axis (`board.ts`, x only), with names fitted
//   to the bars at every zoom and dropped where they do not fit (`fit.ts`),
//   and ticks from d3-axis, which spaces them;
// - hover through the one card (`tip.ts`) and the shared hover bus, so the
//   hex view marks the bytes under the pointer (`linked.ts`);
// - click through the pick contract, hit-tested so a one-pixel field is still
//   24 pixels to the pointer (`pick.ts`);
// - registration from `register.ts`, as an inspector section.
//
// Registered only when the page is opened with `?vizdemo`, since the At cursor
// panel has not been designed to hold it.

import type { Doc, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { fieldClass } from "../fieldstyle.ts";
import { bitSizeText, childWord, countText, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { applyX, invertX, rescaleDomain, type View } from "./boardmath.ts";
import { canvasMeasure, ellipsise, type Measure } from "./fit.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { assignLanes } from "./lanes.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { LOD } from "./lod.ts";
import { goToField, nearestOnAxis, type AxisTarget } from "./pick.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

/** Most children drawn. A list of fifty thousand records is a different
 *  piece (`recordtiles.ts`); this one says how many it left out. */
const MAX_CHILDREN = 2000;

export type ChildStripData = {
  /** The structure drawn. */
  readonly parent: TemplateNode;
  /** Its children, the first `MAX_CHILDREN` of them. */
  readonly kids: readonly TemplateNode[];
  /** The field the cursor is in, when it is one of `kids`. */
  readonly focus: readonly number[] | null;
};

/** The structure to draw for the field at `path`: the field itself when it
 *  has children, and otherwise the structure it is in. Null while the
 *  children are being read, and for a field with nothing around it. */
export function childStripData(doc: Doc, node: TemplateNode, path: readonly number[]): ChildStripData | null {
  const own = node.composite && node.child_count > 0;
  const parentPath = own ? path : path.slice(0, -1);
  if (!own && path.length === 0) return null;
  let p: TemplateNode | null = node;
  if (!own) {
    const up = doc.templateNode(parentPath);
    p = up.status === "ok" ? up.node : null;
  }
  if (p === null || p.child_count === 0) return null;
  const reply = doc.templateChildren(parentPath, 0, Math.min(p.child_count, MAX_CHILDREN));
  if (reply.status !== "ok") return null;
  return { parent: p, kids: reply.node, focus: own ? null : path };
}

/** One bar, in layout pixels. */
export type StripBar = { readonly x0: number; readonly x1: number; readonly lane: number; readonly node: TemplateNode };

export type StripLayout = {
  readonly bars: readonly StripBar[];
  readonly lanes: number;
  /** The bits the strip's width stands for, end exclusive. */
  readonly startBit: number;
  readonly endBit: number;
};

/**
 * Where each child goes across `width` pixels. The strip covers the parent's
 * bits and any child placed outside them, so a field reached by an offset is
 * still drawn where it is. Children that overlap in the file (a union, a
 * field read twice) take separate lanes.
 */
export function childStripLayout(parent: TemplateNode, kids: readonly TemplateNode[], width: number): StripLayout {
  let startBit = parent.offset_bits;
  let endBit = parent.offset_bits + parent.size_bits;
  for (const k of kids) {
    if (k.size_bits === 0) continue;
    startBit = Math.min(startBit, k.offset_bits);
    endBit = Math.max(endBit, k.offset_bits + k.size_bits);
  }
  const span = Math.max(1, endBit - startBit);
  const at = (bit: number): number => ((bit - startBit) / span) * width;
  const sized = kids.filter((k) => k.size_bits > 0);
  const { lane, count } = assignLanes(sized, (k) => ({ start: k.offset_bits, end: k.offset_bits + k.size_bits }));
  const bars = sized.map((node, i) => ({ x0: at(node.offset_bits), x1: at(node.offset_bits + node.size_bits), lane: lane[i] ?? 0, node }));
  return { bars, lanes: Math.max(1, count), startBit, endBit };
}

/** The strip's own parts, top to bottom. */
const BARS_TOP = 2;
const BARS_H = 28;
const AXIS_TOP = BARS_TOP + BARS_H + 2;
const AXIS_H = 18;
const HEIGHT = AXIS_TOP + AXIS_H;
/** The name's size inside a bar, the size of the listing's small text. */
const LABEL_PX = 11;
const LABEL_PAD = 3;
/** Fewer characters than this is not a name worth cutting down to. */
const LABEL_MIN_CHARS = 4;
/** How wide one byte may get at the deepest zoom. */
const MAX_BYTE_PX = 48;
/** Room per axis tick, so d3 is asked for as many as fit. */
const TICK_SPACING_PX = 90;

const SVG = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
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

export type ChildStrip = { readonly el: HTMLElement; readonly board: Board; destroy(): void };

/** Draw the strip `width` pixels wide. */
export function drawChildStrip(doc: Doc, data: ChildStripData, opts: { readonly width: number }): ChildStrip {
  const width = Math.max(40, opts.width);
  const layout = childStripLayout(data.parent, data.kids, width);
  const laneH = BARS_H / layout.lanes;

  const host = document.createElement("figure");
  host.className = "viz-strip";
  const head = document.createElement("figcaption");
  head.className = "viz-strip-head";
  const title = document.createElement("span");
  title.className = "viz-strip-title";
  title.textContent = VIZ.childStrip.title(data.parent.name);
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  head.append(title, reset);

  const root = svg("svg", { width, height: HEIGHT, class: "viz-strip-svg", role: "img" });
  root.setAttribute("aria-label", VIZ.childStrip.title(data.parent.name));
  const barsG = svg("g");
  const labelsG = svg("g", { class: "viz-strip-names" });
  const axisG = svg("g", { class: "viz-axis", transform: `translate(0,${AXIS_TOP})` });
  root.append(barsG, labelsG, axisG);
  host.append(head, root);
  if (data.parent.child_count > data.kids.length) {
    const more = document.createElement("div");
    more.className = "viz-strip-note";
    more.textContent = VIZ.childStrip.firstOf(data.kids.length, data.parent.child_count, childWord(data.parent));
    host.append(more);
  }

  const rects = layout.bars.map((bar) => {
    const r = svg("rect", { y: BARS_TOP + bar.lane * laneH, height: Math.max(1, laneH - 1), class: `viz-strip-bar ${fieldClass(bar.node.kind)}` });
    barsG.append(r);
    return r;
  });
  /** The bars as hit targets along the axis, sorted by where they start. */
  const targets: AxisTarget<StripBar>[] = layout.bars
    .map((bar) => ({ start: bar.x0, end: bar.x1, value: bar }))
    .sort((a, b) => a.start - b.start);

  let view: View = { k: 1, x: 0, y: 0 };
  let axis: Axis | null = null;
  const bus = linkedFor(doc);
  let selected: readonly number[] | null = data.focus ?? bus.state.selection?.path ?? null;

  const drawAxis = (): void => {
    if (axis === null) return;
    const bytes = rescaleDomain(view, [layout.startBit / 8, layout.endBit / 8], [0, width]);
    const x = axis.scale.scaleLinear().domain(bytes).range([0, width]);
    // Whole bytes only: a tick at byte 3.5 names no address.
    const ticks = x.ticks(Math.max(2, Math.floor(width / TICK_SPACING_PX))).filter((t) => Number.isInteger(t));
    const a = axis.axis
      .axisBottom(x)
      .tickValues(ticks)
      .tickSize(3)
      .tickFormat((d) => formatOffset(Number(d) * 8));
    axis.selection.select(axisG).call(a);
  };

  const draw = (): void => {
    const names: SVGTextElement[] = [];
    layout.bars.forEach((bar, i) => {
      const r = rects[i] as SVGRectElement;
      const x0 = applyX(view, bar.x0);
      const x1 = applyX(view, bar.x1);
      // At least one pixel wide, so a one-byte field of a large structure is
      // still on the picture; the hit test makes it 24 pixels to the pointer.
      r.setAttribute("x", String(x0));
      r.setAttribute("width", String(Math.max(1, x1 - x0)));
      const on = selected !== null && samePath(selected, bar.node.path);
      r.classList.toggle("is-on", on);
      // A name only where it fits inside its own bar, so two names can never
      // meet: the bars do not overlap within a lane.
      if (laneH < LABEL_PX + 2 || x1 < 0 || x0 > width) return;
      const left = Math.max(0, x0);
      const right = Math.min(width, x1);
      const text = ellipsise(bar.node.name, right - left - 2 * LABEL_PAD, LABEL_PX, measurer(), LABEL_MIN_CHARS);
      if (text === null) return;
      const t = svg("text", { x: left + LABEL_PAD, y: BARS_TOP + bar.lane * laneH + laneH / 2, "dominant-baseline": "central" });
      t.textContent = text;
      names.push(t);
    });
    labelsG.replaceChildren(...names);
    drawAxis();
  };

  const zoom = board(root, {
    width,
    height: HEIGHT,
    xOnly: true,
    maxScale: Math.max(1, ((layout.endBit - layout.startBit) / 8) * (MAX_BYTE_PX / width)),
    onZoom: (v) => {
      view = v;
      reset.disabled = v.k === 1 && v.x === 0;
      draw();
    },
  });
  reset.addEventListener("click", () => zoom.home(true));
  void loadAxis().then((loaded) => {
    axis = loaded;
    drawAxis();
  });

  /** The bar under a pointer event, by the hit test. */
  const barAt = (e: MouseEvent): StripBar | null => {
    const r = root.getBoundingClientRect();
    if (e.clientY - r.top > BARS_TOP + BARS_H + 2) return null;
    const x = invertX(view, e.clientX - r.left);
    return nearestOnAxis(targets, x, LOD.targetPx / view.k)?.value ?? null;
  };

  const stopTip = hoverTip(root, (e) => {
    const bar = barAt(e);
    bus.publish({ hover: bar === null ? null : { startBit: bar.node.offset_bits, endBit: bar.node.offset_bits + bar.node.size_bits } });
    if (bar === null) return null;
    const n = bar.node;
    const value = n.composite ? countText(n.child_count, childWord(n)) : n.value;
    return tipCard(n.name, [`${n.type} · ${value}`], `${formatOffset(n.offset_bits)} · ${bitSizeText(n.size_bits)}`);
  });
  const leave = (): void => bus.publish({ hover: null });
  root.addEventListener("pointerleave", leave);
  root.addEventListener("click", (e) => {
    const bar = barAt(e);
    if (bar === null) return;
    const n = bar.node;
    goToField(doc, { path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits });
  });
  // The field the cursor is in is lit, whichever view moved the cursor.
  const stopLinked = subscribeWhileConnected(bus, host, (state, changed) => {
    if (changed !== "selection") return;
    selected = state.selection?.path ?? null;
    draw();
  });

  draw();
  return {
    el: host,
    board: zoom,
    destroy: () => {
      zoom.destroy();
      stopTip();
      stopLinked();
      hideTip();
      root.removeEventListener("pointerleave", leave);
    },
  };
}

function samePath(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((s, i) => s === b[i]);
}

/** Put the strip in the At cursor panel, for every field inside a structure. */
export function register(): void {
  registerInspectorSection({
    id: "child-strip",
    applies: (_doc, n) => (n.composite && n.child_count > 0) || n.path.length > 0,
    // The structure drawn, so moving between its fields keeps the figure and
    // its zoom; the new field is lit from the shared selection.
    key: (_doc, n, path) => (n.composite && n.child_count > 0 ? path : path.slice(0, -1)).join("/"),
    draw: ({ doc, node, path, width }) => {
      const data = childStripData(doc, node, path);
      return data === null ? null : drawChildStrip(doc, data, { width }).el;
    },
  });
}

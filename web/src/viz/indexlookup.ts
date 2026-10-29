// Index and string-table lookup: the field holding a number, the table the
// number picks from with the entry it lands on lit, and what that entry says.
//
// Candidate `index-lookup` in docs/DESIGN-diagrams.md, section 4.2; built by
// agent L of the first batch (section 5.3), as an inspector section only. The
// listing's `-> reads` hover card is the same figure and comes later, since
// it lives in `listingdraw.ts`.
//
// The commonest indirection in a binary is a small number in one place that
// means an entry somewhere else. An ELF section header's `name_offset` of 17
// means the string 17 bytes into `.shstrtab`; a pixel of 1 in an indexed PNG
// means the second colour of PLTE. The At cursor panel already says the
// number, and its Points to row the address; neither shows the table, which
// is where a reader checks the answer: the strings either side, the colours
// around it, how big the table is and whether the number lands at the start
// of an entry or inside one.
//
// Three rows, top to bottom, each in a fixed slot so nothing can collide:
// - the index: the field's name, its bytes and the number they hold, and
//   the working for the address under it where the core wrote one
//   (`section_name_base + name_offset = 1130461 + 17`);
// - the table as a strip of cells, drawn in the units a number into it
//   counts (bytes for a string table, entries for a palette), with an arrow
//   from the index row down to where the number lands, a bracket under the
//   entry, and the number itself a tick on the axis (`lookuplayout.ts`);
// - the entry: what it says, its colour for a palette.
//
// Explorable as every piece is: the strip zooms about the pointer and pans
// across (`board.ts`, x only, since a table is in file order), opening zoomed
// in on the entry so its name can be read; every cell has a hover card; a
// click on a cell, the index row or the entry row goes to those bytes. The
// left and right arrow keys (and the two step buttons) move the lit entry
// along the table without moving the cursor, and the index row then shows
// which fields point at that entry (`scanPointers`), so a reader can ask of
// any string "who names themselves this".
//
// The readers are `tableentries.ts`, the arithmetic `lookuplayout.ts`. The
// styles are `indexlookup.css`, loaded with this module.

import "./indexlookup.css";
import type { Doc, TemplateNode } from "../doc.ts";
import { fieldClass } from "../fieldstyle.ts";
import { formatAddress } from "../format.ts";
import { bitSizeText, childWord, countText, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { invertX, rescaleDomain, type View } from "./boardmath.ts";
import { ellipsise, monoMeasure, type Measure } from "./fit.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { linkedFor, subscribeWhileConnected, type BitRange } from "./linked.ts";
import { LOD } from "./lod.ts";
import { centreOn, inView, lookupTicks, openingFromStart, openingView, stepEntry, stripLayout, unitOf, unitX, type Cell } from "./lookuplayout.ts";
import { followPicks, goToField, markPick, nearestOnAxis, type AxisTarget } from "./pick.ts";
import { lookupFor, mayLookUp, PENDING, scanPointers, type Lookup, type Pointer, type Pointers, type TableEntry } from "./tableentries.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

const T = VIZ.indexLookup;

/** The strip's rows, top to bottom, in pixels. */
const ARROW_H = 16;
const STRIP_TOP = ARROW_H;
const STRIP_H = 26;
const AXIS_TOP = STRIP_TOP + STRIP_H + 1;
const AXIS_H = 16;
const BRACKET_TOP = AXIS_TOP + AXIS_H;
const BRACKET_H = 12;
const HEIGHT = BRACKET_TOP + BRACKET_H;
/** A name inside a cell: the listing's small text, with room either side. */
const LABEL_PX = 11;
const LABEL_PAD = 3;
const LABEL_MIN_CHARS = 3;
/** The axis labels, in the monospace stack at 10 pixels (`.viz-axis`). */
const TICK_PX = 10;
const TICK_SPACING_PX = 70;
/** How wide the lit entry opens: room for a name of this many characters,
 *  or this many pixels for a colour. */
const OPEN_CHARS = 12;
const OPEN_SWATCH_PX = 28;
/** How wide one unit (a byte, or one palette entry) may get at the deepest
 *  zoom. */
const MAX_UNIT_PX = 64;

const SVG = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== "") e.textContent = text;
  return e;
}

/** The axis labels' width, estimated from the monospace advance: a tick
 *  label is a few characters and needs no canvas. */
const MONO = monoMeasure();
/**
 * Names in cells are drawn in the monospace stack (`.viz-lookup-names`) and
 * measured in it at the size they are drawn, so a name that is kept fits its
 * cell. Not `fit.ts`'s `canvasMeasure`, which measures at 100 pixels and
 * scales down: at 11 pixels the browser rounds each advance of the fallback
 * monospace face up to 7 pixels, where the scaled width says 6.6, and a name
 * measured to fit came out a few pixels too long for its cell.
 */
let cellMeasure: Measure | null = null;
function namesMeasure(): Measure {
  if (cellMeasure !== null) return cellMeasure;
  const family = getComputedStyle(document.documentElement).getPropertyValue("--mono").trim();
  const ctx = family === "" ? null : document.createElement("canvas").getContext("2d");
  if (ctx === null) return (cellMeasure = MONO);
  const cache = new Map<string, number>();
  cellMeasure = (text, size) => {
    const key = `${size}|${text}`;
    let w = cache.get(key);
    if (w === undefined) {
      ctx.font = `${size}px ${family}`;
      w = ctx.measureText(text).width;
      if (cache.size > 20_000) cache.clear();
      cache.set(key, w);
    }
    return w;
  };
  return cellMeasure;
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

/** A number as the axis writes it: hex for a byte offset, decimal for an
 *  entry number. */
function unitText(v: number, counts: "bytes" | "entries"): string {
  return counts === "bytes" ? `0x${Math.round(v).toString(16)}` : String(Math.round(v));
}

/** A field's number as the index row writes it: decimal, as the listing and
 *  the At cursor panel do, with the hex beside it where the axis is hex. */
function numberText(n: TemplateNode, counts: "bytes" | "entries"): string {
  const v = Number(n.edit_text !== "" ? n.edit_text : n.value);
  if (!Number.isSafeInteger(v)) return n.value;
  return counts === "bytes" && v >= 10 ? `${v} (0x${v.toString(16)})` : String(v);
}

/** What a field holds, in words: its value, a structure's one-line reading,
 *  or how many children a list has (`32 items`), never the bare count. */
function valueText(n: TemplateNode): string {
  if (!n.composite) return n.value;
  return n.line ?? countText(n.child_count, childWord(n));
}

function hexBytes(bytes: readonly number[]): string {
  return bytes.map((b) => b.toString(16).padStart(2, "0")).join(" ");
}

function rangeOf(n: { readonly startBit: number; readonly endBit: number; readonly space: number }): BitRange | null {
  // The bus and the hex view count in the file; a field of a decoded stream
  // has no bytes there to light.
  return n.space === 0 ? { startBit: n.startBit, endBit: n.endBit } : null;
}

function nodeRange(n: TemplateNode): { startBit: number; endBit: number; space: number } {
  return { startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits, space: n.space };
}

export type IndexLookup = { readonly el: HTMLElement; readonly board: Board; destroy(): void };

/** Draw the figure `width` pixels wide. */
export function drawIndexLookup(doc: Doc, data: Lookup, opts: { readonly width: number }): IndexLookup {
  const width = Math.max(80, opts.width);
  const table = data.table;
  const layout = stripLayout(table, width);
  const cells = layout.cells;
  const byIndex = new Map<number, { cell: Cell; entry: TableEntry }>();
  table.entries.forEach((entry, i) => byIndex.set(entry.index, { cell: cells[i] as Cell, entry }));
  const tableName = table.node.name;
  const fieldName = data.index.name;
  const title = table.reached ? T.pointsTo(fieldName) : T.title(tableName);
  const measure = namesMeasure();

  const host = el("figure", "viz-strip viz-lookup");
  host.setAttribute("aria-label", title);

  // ----- heading -----
  const head = el("figcaption", "viz-strip-head");
  const titleEl = el("span", "viz-strip-title", title);
  titleEl.title = title;
  const tools = el("span", "viz-lookup-tools");
  const prev = el("button", "viz-reset viz-lookup-step", "‹");
  const next = el("button", "viz-reset viz-lookup-step", "›");
  prev.type = next.type = "button";
  prev.title = T.previous;
  next.title = T.next;
  prev.setAttribute("aria-label", T.previous);
  next.setAttribute("aria-label", T.next);
  const reset = el("button", "viz-reset", VIZ.resetZoom);
  reset.type = "button";
  tools.append(prev, next, reset);
  head.append(titleEl, tools);

  // ----- the index row and the entry row: filled by `showFocus` -----
  const indexRow = el("button", "viz-lookup-box viz-lookup-index");
  indexRow.type = "button";
  const via = el("div", "viz-lookup-via");
  const resultRow = el("button", "viz-lookup-box viz-lookup-result");
  resultRow.type = "button";

  // ----- the strip -----
  const root = svg("svg", { width, height: HEIGHT, class: "viz-strip-svg viz-lookup-svg", role: "img", tabindex: 0 });
  root.setAttribute("aria-label", title);
  const cellsG = svg("g");
  const namesG = svg("g", { class: "viz-strip-names viz-lookup-names" });
  const marksG = svg("g", { class: "viz-lookup-marks" });
  const axisG = svg("g", { class: "viz-axis", transform: `translate(0,${AXIS_TOP})` });
  root.append(cellsG, namesG, axisG, marksG);

  host.append(head, indexRow);
  if (data.via !== null) {
    via.textContent = `${data.via.written} = ${data.via.substituted} = ${data.via.result}`;
    host.append(via);
  }
  host.append(root);
  const last = table.entries[table.entries.length - 1];
  if (last !== undefined && table.entries.length < table.count) host.append(el("div", "viz-strip-note", T.window(table.first, last.index, table.count)));
  host.append(resultRow);

  const rects = table.entries.map((entry) => {
    const r = svg("rect", { y: STRIP_TOP, height: STRIP_H, class: table.kind === "palette" ? "viz-lookup-swatch" : `viz-lookup-cell ${entry.label === "" && table.kind === "strings" ? "is-empty" : ""}` });
    if (entry.colour !== null) r.style.fill = entry.colour;
    cellsG.append(r);
    return r;
  });
  /** The cells as hit targets along the axis, in file order already. */
  const targets: AxisTarget<TableEntry>[] = table.entries.map((entry, i) => ({ start: (cells[i] as Cell).x0, end: (cells[i] as Cell).x1, value: entry }));

  const bus = linkedFor(doc);
  let view: View = { k: 1, x: 0, y: 0 };
  let axis: Axis | null = null;
  // A pointer to one structure reaches all of it, so the figure opens with
  // the whole structure under the bracket (focus -1) and the step buttons go
  // through its fields one at a time. A table opens on the entry looked up.
  const whole = table.reached;
  const WHOLE = -1;
  let focus = whole ? WHOLE : data.hit >= 0 ? data.hit : (table.entries[0]?.index ?? 0);
  /** Whether the index row shows the field at the cursor: always for a
   *  structure, and for a table while the entry it looks up is lit. */
  const atCursor = (): boolean => whole || focus === data.hit;
  let hovered: number | null = null;
  let pointers: Pointers | null | undefined;
  /** The fields pointing at each entry, read the first time they are asked for. */
  const pointersNow = (): Pointers | null => {
    if (pointers !== undefined) return pointers;
    const p = scanPointers(doc, data);
    if (p === PENDING) return null;
    pointers = p;
    return p;
  };
  const pointedAt = (i: number): boolean => whole || i === data.hit || (pointersNow()?.byEntry.get(i)?.length ?? 0) > 0;
  const xOf = (u: number): number => unitX(layout, width, u) * view.k + view.x;

  // ----- the rows above and below the strip -----
  const showIndex = (): void => {
    indexRow.replaceChildren();
    indexRow.classList.remove("is-none");
    // The working is for the field at the cursor, not for a field stepped to.
    via.hidden = !atCursor();
    let n: TemplateNode | null = null;
    let extra = 0;
    if (atCursor()) n = data.index;
    else {
      const ps = pointersNow()?.byEntry.get(focus) ?? [];
      const first = ps[0];
      if (first !== undefined) {
        const r = doc.templateNode(first.path);
        n = r.status === "ok" ? r.node : null;
        extra = ps.length - 1;
      }
    }
    if (n === null) {
      const p = pointersNow();
      const scanned = p?.scanned ?? 0;
      const total = p?.total ?? 0;
      indexRow.classList.add("is-none");
      indexRow.disabled = true;
      indexRow.append(el("span", "viz-lookup-none", data.how === "palette" ? T.noPixel(scanned, total) : T.noPointer(fieldName, scanned, total)));
      indexRow.removeAttribute("data-reads");
      indexRow.removeAttribute("data-at");
      return;
    }
    indexRow.disabled = false;
    const label = atCursor() ? data.indexLabel : (pointersNow()?.byEntry.get(focus)?.[0]?.label ?? n.name);
    indexRow.append(el("span", "viz-lookup-name", label));
    const bytes = doc.fieldBytes(n.path, 8);
    if (bytes.status === "ok" && bytes.node.bytes.length > 0) indexRow.append(el("span", "viz-lookup-bytes", hexBytes(bytes.node.bytes) + (bytes.node.truncated ? " …" : "")));
    indexRow.append(el("span", "viz-lookup-value", `= ${numberText(n, table.counts)}`));
    if (extra > 0) indexRow.append(el("span", "viz-lookup-more", T.andMore(extra)));
    markPick(indexRow, { path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits });
    indexNode = n;
    indexShown = label;
  };
  let indexNode: TemplateNode | null = data.index;
  /** The index row's name as shown, which its hover card repeats in full. */
  let indexShown = data.indexLabel;

  const showResult = (): void => {
    resultRow.replaceChildren();
    if (focus === WHOLE) {
      const r = data.result;
      resultRow.append(el("span", "viz-lookup-name", r.name), el("span", "viz-lookup-value", valueText(r)));
      markPick(resultRow, { path: r.path, startBit: r.offset_bits, endBit: r.offset_bits + r.size_bits });
      resultNode = r;
      return;
    }
    const hit = byIndex.get(focus);
    if (hit === undefined) return;
    const entry = hit.entry;
    let n: TemplateNode | null = null;
    {
      const r = doc.templateNode(entry.path);
      n = r.status === "ok" ? r.node : null;
    }
    if (table.kind === "palette") {
      const sw = el("span", "viz-lookup-chip");
      if (entry.colour !== null) sw.style.background = entry.colour;
      resultRow.append(sw);
    }
    const label = table.kind === "records" ? entry.label : T.entry(entry.index);
    resultRow.append(el("span", "viz-lookup-name", label));
    // A palette entry's channels; its hex is in the row's hover card.
    const value = entry.value;
    const v = el("span", "viz-lookup-value", table.kind === "strings" && value === "" ? T.emptyString : value);
    if (table.kind === "strings" && value === "") v.classList.add("is-empty");
    resultRow.append(v);
    const alpha = table.kind === "palette" ? alphaOf(entry.index) : null;
    if (alpha !== null) resultRow.append(el("span", "viz-lookup-detail", T.alpha(alpha)));
    if (n !== null) {
      markPick(resultRow, { path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits });
      resultNode = n;
    }
  };
  let resultNode: TemplateNode | null = data.result;
  /** The alpha the tRNS chunk gives an entry; an entry past the end of tRNS
   *  is opaque. Null for a palette with no tRNS, which says nothing. */
  const alphaOf = (i: number): number | null => (data.alphas.length === 0 ? null : (data.alphas[i] ?? 255));

  // ----- the strip -----
  const drawAxis = (): void => {
    if (axis === null) return;
    const visible = rescaleDomain(view, layout.domain, [0, width]);
    const x = axis.scale.scaleLinear().domain(visible).range([0, width]);
    // Ticks only over the table, not the margin a pan can pull into view;
    // an entry number stands at the middle of its cell.
    const inside: [number, number] = [Math.max(visible[0], layout.domain[0]), Math.min(visible[1], layout.domain[1] - (table.counts === "entries" ? 1 : 0))];
    const mid = table.counts === "entries" ? 0.5 : 0;
    // The number in the index row is a tick of its own: the target for the
    // field at the cursor, and the entry's start for a field stepped to.
    const f = byIndex.get(focus);
    const hitUnit =
      table.counts === "entries" ? focus : atCursor() ? Math.round(unitOf(table, data.target, data.hit)) : f !== undefined && pointedAt(focus) ? Math.round(unitOf(table, f.entry.startBit, focus)) : null;
    const ticks = lookupTicks(inside, Math.max(2, Math.floor(width / TICK_SPACING_PX)), (v) => x(v + mid), width, (v) => MONO(unitText(v, table.counts), TICK_PX), hitUnit);
    const a = axis.axis
      .axisBottom(axis.scale.scaleLinear().domain([visible[0] - mid, visible[1] - mid]).range([0, width]))
      .tickValues(ticks.map((t) => t.value))
      .tickSize(3)
      .tickFormat((d) => unitText(Number(d), table.counts));
    const g = axis.selection.select(axisG);
    g.call(a);
    const anchors = new Map(ticks.map((t) => [t.value, t]));
    g.selectAll<SVGGElement, number>(".tick").classed("is-hit", (d) => anchors.get(Number(d))?.hit === true);
    g.selectAll<SVGTextElement, number>(".tick text").attr("text-anchor", (d) => anchors.get(Number(d))?.anchor ?? "middle");
  };

  const drawMarks = (): void => {
    const marks: SVGElement[] = [];
    const f = byIndex.get(focus);
    const firstCell = cells[0];
    const lastCell = cells[cells.length - 1];
    const span = focus === WHOLE && firstCell !== undefined && lastCell !== undefined ? { x0: firstCell.x0, x1: lastCell.x1 } : f?.cell;
    if (span !== undefined) {
      const x0 = span.x0 * view.k + view.x;
      const x1 = span.x1 * view.k + view.x;
      // The bracket under the lit entry, and its stem down to the entry row.
      if (x1 > 0 && x0 < width) {
        const a = Math.max(1, x0);
        const b = Math.min(width - 1, Math.max(a, x1));
        const mid = (a + b) / 2;
        const y = BRACKET_TOP + 4;
        marks.push(svg("path", { class: "viz-lookup-bracket", d: `M${a},${BRACKET_TOP} V${y} H${b} V${BRACKET_TOP} M${mid},${y} V${HEIGHT}` }));
      }
      // The arrow from the index row to where the number lands, while some
      // field points at this entry.
      if (pointedAt(focus)) {
        const unit = atCursor() || f === undefined ? unitOf(table, data.target, data.hit) : table.counts === "entries" ? focus + 0.5 : unitOf(table, f.entry.startBit, focus);
        const at = xOf(unit);
        if (at >= 0 && at <= width) {
          // Kept a head's half-width inside the strip, so an arrow at an end
          // is drawn whole.
          const ax = Math.max(4, Math.min(width - 4, at));
          marks.push(svg("path", { class: "viz-lookup-arrow", d: `M${ax},0 V${STRIP_TOP - 5}` }));
          marks.push(svg("path", { class: "viz-lookup-head", d: `M${ax - 4},${STRIP_TOP - 6} L${ax + 4},${STRIP_TOP - 6} L${ax},${STRIP_TOP} Z` }));
        } else {
          // Off the strip: a mark at the edge it is past.
          const e = at < 0 ? 2 : width - 2;
          const s = at < 0 ? 1 : -1;
          marks.push(svg("path", { class: "viz-lookup-head", d: `M${e},${ARROW_H / 2} L${e + s * 7},${ARROW_H / 2 - 5} L${e + s * 7},${ARROW_H / 2 + 5} Z` }));
        }
      }
    }
    marksG.replaceChildren(...marks);
  };

  const draw = (): void => {
    const names: SVGTextElement[] = [];
    table.entries.forEach((entry, i) => {
      const cell = cells[i] as Cell;
      const r = rects[i] as SVGRectElement;
      const x0 = cell.x0 * view.k + view.x;
      const x1 = cell.x1 * view.k + view.x;
      // A cell is at least a pixel wide, so a one-byte string of a large
      // table is still drawn; the hit test makes it 24 pixels to the pointer.
      // A palette's cells keep a one-pixel gap between them once they are
      // wide enough to spare it.
      const gap = table.kind === "palette" && x1 - x0 > 6 ? 1 : 0;
      r.setAttribute("x", String(x0 + gap / 2));
      r.setAttribute("width", String(Math.max(1, x1 - x0 - gap)));
      r.classList.toggle("is-on", entry.index === focus);
      r.classList.toggle("is-hover", entry.index === hovered);
      if (table.kind === "palette" || entry.label === "" || x1 < 0 || x0 > width) return;
      const left = Math.max(0, x0);
      const right = Math.min(width, x1);
      const text = ellipsise(entry.label, right - left - 2 * LABEL_PAD, LABEL_PX, measure, LABEL_MIN_CHARS);
      if (text === null) return;
      const t = svg("text", { x: left + LABEL_PAD, y: STRIP_TOP + STRIP_H / 2, "dominant-baseline": "central", class: entry.index === focus ? "is-on" : "" });
      t.textContent = text;
      names.push(t);
    });
    namesG.replaceChildren(...names);
    root.classList.toggle("is-whole", focus === WHOLE);
    drawMarks();
    drawAxis();
  };

  const want = table.kind === "palette" ? OPEN_SWATCH_PX : Math.min(width * 0.6, measure("m".repeat(OPEN_CHARS), LABEL_PX) + 2 * LABEL_PAD + 2);
  const span = Math.max(1, layout.domain[1] - layout.domain[0]);
  const maxScale = Math.max(1, (span * MAX_UNIT_PX) / width);
  const opening = whole ? openingFromStart(cells, width, want, maxScale) : openingView(byIndex.get(focus)?.cell ?? null, width, want, maxScale);
  const atOpening = (v: View): boolean => Math.abs(v.k - opening.k) < 1e-6 && Math.abs(v.x - opening.x) < 0.5;
  const zoom = board(root, {
    width,
    height: HEIGHT,
    xOnly: true,
    maxScale,
    onZoom: (v) => {
      view = v;
      reset.disabled = atOpening(v);
      draw();
    },
  });
  /** The opening view as a box of the layout, which is what the board takes. */
  const openingBox = { x0: -opening.x / opening.k, x1: (width - opening.x) / opening.k, y0: 0, y1: HEIGHT };
  const goOpening = (animate: boolean): void => {
    if (opening.k === 1 && opening.x === 0) zoom.home(animate);
    else zoom.zoomTo(openingBox, animate);
  };

  const stepTo = (to: number): void => {
    if (to === focus) return;
    focus = to;
    const f = byIndex.get(focus);
    showIndex();
    showResult();
    updateSteps();
    if (f !== undefined && !inView(view, f.cell.x0, f.cell.x1, width)) {
      const v = centreOn(view, f.cell.x0, f.cell.x1, width);
      zoom.zoomTo({ x0: -v.x / v.k, x1: (width - v.x) / v.k, y0: 0, y1: HEIGHT }, false);
    } else draw();
  };
  // A structure's steps start at the whole of it, one before its first field.
  const low = whole ? WHOLE : table.first;
  const step = (delta: number): void => stepTo(stepEntry(focus, delta, low, table.first + table.entries.length - low));
  const updateSteps = (): void => {
    prev.disabled = focus <= low;
    next.disabled = focus >= table.first + table.entries.length - 1;
  };
  prev.addEventListener("click", () => step(-1));
  next.addEventListener("click", () => step(1));
  reset.addEventListener("click", () => goOpening(true));
  root.addEventListener("keydown", (e) => {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      step(e.key === "ArrowLeft" ? -1 : 1);
    }
  });

  void loadAxis().then((loaded) => {
    axis = loaded;
    drawAxis();
  });

  // ----- hover and click on the strip -----
  const cellAt = (e: MouseEvent): TableEntry | null => {
    const r = root.getBoundingClientRect();
    const y = e.clientY - r.top;
    if (y < STRIP_TOP - 4 || y > AXIS_TOP + 4) return null;
    const x = invertX(view, e.clientX - r.left);
    return nearestOnAxis(targets, x, LOD.targetPx / view.k)?.value ?? null;
  };
  const entryCard = (entry: TableEntry): HTMLElement => {
    const lines: string[] = [];
    // A palette entry's card is headed by its colour, with its channels
    // under it; its place is the line after.
    if (table.kind === "palette") lines.push(entry.value);
    else if (table.kind === "strings" && entry.label === "") lines.push(T.emptyString);
    else if (table.kind === "records" && entry.value !== entry.name) lines.push(entry.value);
    if (!table.reached) lines.push(T.entryOf(entry.index, table.count));
    if (table.counts === "bytes") lines.push(T.offsetIn(unitText((entry.startBit - table.node.offset_bits) / 8, "bytes"), tableName));
    const a = table.kind === "palette" ? alphaOf(entry.index) : null;
    if (a !== null) lines.push(T.alpha(a));
    const ps = whole ? null : pointersNow();
    if (ps !== null) lines.push(pointedLine(ps, ps.byEntry.get(entry.index) ?? []));
    const name = table.kind === "strings" ? entry.label || T.entry(entry.index) : table.kind === "palette" ? (entry.colour ?? T.entry(entry.index)) : entry.name;
    return tipCard(name, lines, `${formatAddress(entry.startBit, entry.space)} · ${bitSizeText(entry.endBit - entry.startBit)}`);
  };
  const pointedLine = (ps: Pointers, here: readonly Pointer[]): string => {
    const tail = ps.scanned < ps.total ? ` ${T.ofFirst(ps.scanned)}` : "";
    if (here.length === 0) return data.how === "palette" ? T.noPixel(ps.scanned, ps.total) : T.noPointer(fieldName, ps.scanned, ps.total);
    if (data.how === "palette") return T.usedBy(here.length) + tail;
    return (here.length === 1 ? T.pointedAtBy((here[0] as Pointer).label) : T.pointedAtByCount(here.length, fieldName)) + tail;
  };
  const stopTip = hoverTip(root, (e) => {
    const entry = cellAt(e);
    const was = hovered;
    hovered = entry?.index ?? null;
    if (was !== hovered) draw();
    bus.publish({ hover: entry === null ? null : rangeOf(entry) });
    return entry === null ? null : entryCard(entry);
  });
  const leave = (): void => {
    bus.publish({ hover: null });
    if (hovered !== null) {
      hovered = null;
      draw();
    }
  };
  root.addEventListener("pointerleave", leave);
  root.addEventListener("click", (e) => {
    const entry = cellAt(e);
    if (entry === null) return;
    goToField(doc, { path: entry.path, startBit: entry.startBit, endBit: entry.endBit });
  });

  // ----- hover and click on the two rows -----
  const rowCard = (n: TemplateNode, lines: string[], name = n.name): HTMLElement =>
    tipCard(name, [`${n.type} · ${valueText(n)}`, ...lines], `${formatAddress(n.offset_bits, n.space)} · ${bitSizeText(n.size_bits)}`);
  const stopIndexTip = hoverTip(indexRow, () => {
    if (indexNode === null || indexRow.disabled) return null;
    bus.publish({ hover: rangeOf(nodeRange(indexNode)) });
    const lines = focus === data.hit && data.via !== null ? [data.via.written, `${data.via.substituted} = ${data.via.result}`] : [];
    return rowCard(indexNode, lines, indexShown);
  });
  const stopResultTip = hoverTip(resultRow, () => {
    if (resultNode === null) return null;
    bus.publish({ hover: rangeOf(nodeRange(resultNode)) });
    const colour = table.kind === "palette" ? (byIndex.get(focus)?.entry.colour ?? null) : null;
    return rowCard(resultNode, colour === null ? [] : [colour]);
  });
  const rowLeave = (): void => bus.publish({ hover: null });
  indexRow.addEventListener("pointerleave", rowLeave);
  resultRow.addEventListener("pointerleave", rowLeave);
  const stopPicks = followPicks(host, doc);

  // Another piece's hover lights the entry it covers.
  const stopLinked = subscribeWhileConnected(bus, host, (state, changed) => {
    if (changed !== "hover") return;
    const h = state.hover;
    const over = h === null ? null : (table.entries.find((e) => e.space === 0 && e.startBit < h.endBit && h.startBit < e.endBit)?.index ?? null);
    if (over !== hovered) {
      hovered = over;
      draw();
    }
  });

  showIndex();
  showResult();
  updateSteps();
  goOpening(false);
  draw();
  return {
    el: host,
    board: zoom,
    destroy: () => {
      zoom.destroy();
      stopTip();
      stopIndexTip();
      stopResultTip();
      stopPicks();
      stopLinked();
      hideTip();
      root.removeEventListener("pointerleave", leave);
    },
  };
}

/**
 * Fields known not to be an index, per document, so the inspector asking
 * again on every render (as it does for a section that drew nothing) costs a
 * map lookup rather than a walk. Keyed by path and value: a field edited to
 * a new value is asked again.
 */
const notLookups = new WeakMap<Doc, Set<string>>();

/** Put the figure in the At cursor panel, for a field whose number picks an
 *  entry of a table. */
export function register(): void {
  registerInspectorSection({
    id: "index-lookup",
    applies: (doc, n) => mayLookUp(n) && notLookups.get(doc)?.has(`${n.path.join("/")}|${n.value}`) !== true,
    draw: ({ doc, node, width }) => {
      const data = lookupFor(doc, node);
      if (data === PENDING) return null;
      if (data === null) {
        let set = notLookups.get(doc);
        if (set === undefined) notLookups.set(doc, (set = new Set()));
        set.add(`${node.path.join("/")}|${node.value}`);
        return null;
      }
      return drawIndexLookup(doc, data, { width }).el;
    },
  });
}

// Struct layout with padding holes, in the style of pahole: a structure drawn
// as rows of 4, 8 or 16 bytes, each field a box at its offset, and every
// stretch no field covers hatched and labelled with its byte count.
//
// Candidate `struct-layout-padding` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent F of the first batch (section 5.3). It answers where each
// member sits inside its record, how much of the record is padding or bytes
// no field describes, and whether a number sits off its natural alignment,
// which no listing of offsets shows at a glance. Test samples:
// elf/busybox-x86_64 (the ELF header, the program headers),
// cdrom/hello-mode1.bin (a 2,352-byte sector).
//
// Where it is drawn: a card in the listing, in place of a heading's fields,
// for any structure in any format (`registerKindCard` in `formatcards.ts`).
// Cards by kind wait to be asked for, so a heading button (`layout`, through
// `headingbuttons.ts`) toggles the card's key in the listing's card set.
//
// How it is put together, in the order `register.ts` asks:
// - the data: `structLayoutData` reads the structure and its children by
//   path, opening structures inside it a few levels deep, and answers
//   "pending" while any of them is still being read; a list is drawn one
//   block per element, the first `MAX_ELEMENTS` of them;
// - the layout, DOM-free: `structlayoutmath.ts` says which children are
//   members (a field reached by an offset is listed under the figure, not
//   drawn in it), where the gaps are and what each is called, and how
//   members wrap onto rows, folding long runs of rows wholly one field;
// - the draw, at a width it is given (`drawStructLayout`); the card measures
//   its own box with a ResizeObserver and draws again at the new width, since
//   the listing gives a card no width. The card's height depends only on the
//   data and the row width, never on the pixel width, so the listing's
//   measurement of it holds;
// - pan and zoom on both axes (`board.ts`): the wheel zooms about the
//   pointer and a drag pans, names are fitted again at every zoom
//   (`fit.ts`), and a name with no room inside its box is a number there and
//   a name in the key beside the row;
// - hover through the one card (`tip.ts`) and the shared bus, so the hex view
//   marks the bytes under the pointer; the selection is lit from the bus;
// - click through the pick contract, hit-tested at 24 pixels (`pick.ts`): a
//   field selects the field, a gap selects its bytes.
//
// The listing builds a card again from nothing whenever its rows are laid out
// again, so nothing the reader chose can live in the element: the row width
// is a key in the listing's card set (`DrawContext.cards`), and the zoom is
// kept here by document and path.

import "./structlayout.css";
import type { Doc, TemplateNode } from "../doc.ts";
import type { Item } from "../flatten.ts";
import { pathKey } from "../flatten.ts";
import { fieldClass } from "../fieldstyle.ts";
import { formatAddress, formatBytes } from "../format.ts";
import { askedCardKey, formatCardKind, registerKindCard } from "../formatcards.ts";
import type { DrawContext } from "../listingdraw.ts";
import { bitSizeText, childWord, countText, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { applyX, applyY, invertX, invertY, type View } from "./boardmath.ts";
import { canvasMeasure, monoMeasure, thinLabels, type Measure } from "./fit.ts";
import { headingButton, registerHeadingButton } from "./headingbuttons.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { goToBits, goToField, nearestTarget, type HitTarget } from "./pick.ts";
import {
  buildRecord,
  coverGaps,
  defaultRowWidth,
  fitKey,
  misaligned,
  naturalAlign,
  opensInside,
  ROW_WIDTHS,
  rowLabels,
  summarise,
  wrapRows,
  type Cell,
  type Gathered,
  type Line,
  type Piece,
  type StructRecord,
} from "./structlayoutmath.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

const ID = "struct-layout";

/** How many levels of structure inside the structure are opened into their
 *  members. A deeper one is one box, which its own heading can draw. */
const MAX_DEPTH = 3;
/** Most children read for one record, at every level together. */
const MAX_MEMBERS = 1024;
/** Elements of a list drawn, each as a block of its own. */
const MAX_ELEMENTS = 16;
/** A record longer than this is not drawn as rows. Long fields fold, so the
 *  rows drawn stay few, but every row is cut before it is folded; four MiB is
 *  a quarter of a million rows at 16 bytes, and room for the sections of a
 *  small executable laid out where they sit. */
const MAX_RECORD_BYTES = 4 << 20;

type N = TemplateNode;

export type StructLayoutData =
  | {
      readonly status: "ok";
      /** The structure the heading stands for: a record, or a list of them. */
      readonly root: N;
      /** One record, or one per element drawn. */
      readonly records: readonly StructRecord<N>[];
      /** True when the records are a list's elements, each drawn under a
       *  heading of its own. */
      readonly elements: boolean;
      /** How many records the list has, when more than were drawn. */
      readonly total: number;
      /** For one record with more children than are read for a figure: how
       *  many it has, and the figure stops at the last one read. */
      readonly cutAt: { readonly shown: number; readonly total: number } | null;
    }
  | { readonly status: "pending" }
  | { readonly status: "too-large"; readonly bytes: number };

/** The children of `node`, with the structures among them opened `depth`
 *  levels down. Null while any of them is still being read. A structure
 *  inside is opened only when all of its children fit in what is left of
 *  `budget`, so a structure is never drawn with half its members, which would
 *  leave the rest looking like bytes no field covers; it stays one box. */
function gather(doc: Doc, node: N, depth: number, budget: { left: number }): Gathered<N>[] | null {
  const count = Math.min(node.child_count, budget.left);
  if (count <= 0) return [];
  const reply = doc.templateChildren(node.path, 0, count);
  if (reply.status !== "ok") return null;
  budget.left -= reply.node.length;
  const out: Gathered<N>[] = [];
  for (const kid of reply.node) {
    if (depth < MAX_DEPTH && opensInside(kid) && kid.child_count <= budget.left) {
      const inner = gather(doc, kid, depth + 1, budget);
      if (inner === null) return null;
      out.push({ node: kid, kids: inner });
    } else out.push({ node: kid, kids: null });
  }
  return out;
}

/** Most entries asked of the annotation column for one gap. A gap is
 *  classed by whether any of it is read elsewhere; the first few entries
 *  say that as well as all of them would. */
const COVER_SPANS = 64;

/** What fields elsewhere in the file are read from a gap in `record`: the
 *  names of the parts they belong to, one level under what holds the
 *  record, since that level is where the two part ways. */
function coverFor(doc: Doc, record: N): (startBit: number, endBit: number) => readonly string[] | null {
  return (startBit, endBit) => {
    if (record.space !== 0) return [];
    const reply = doc.spans(startBit, endBit, COVER_SPANS);
    if (reply.status !== "ok") return null;
    const owners: string[] = [];
    const level = Math.max(0, record.path.length - 1);
    for (const span of reply.node) {
      if (span.gap) continue;
      const owner = span.trail[level] ?? span.name;
      if (!owners.includes(owner)) owners.push(owner);
    }
    return owners;
  };
}

/** What the figure draws for the structure at a heading: its record, or one
 *  record per element when it is a list of structures. */
export function structLayoutData(doc: Doc, node: N): StructLayoutData {
  if (node.list) {
    const shown = Math.min(node.child_count, MAX_ELEMENTS);
    const reply = doc.templateChildren(node.path, 0, shown);
    if (reply.status !== "ok") return { status: "pending" };
    // A list of structures is drawn element by element; a list of numbers or
    // of strings is one record whose members are its elements, as an array
    // of `u32` is laid out in C.
    if (reply.node.length > 0 && reply.node.every((e) => opensInside(e))) {
      const records: StructRecord<N>[] = [];
      for (const e of reply.node) {
        const kids = gather(doc, e, 1, { left: MAX_MEMBERS });
        if (kids === null) return { status: "pending" };
        const r = coverGaps(buildRecord(e, kids), coverFor(doc, e));
        if (r.endBit - r.startBit > MAX_RECORD_BYTES * 8) return { status: "too-large", bytes: (r.endBit - r.startBit) / 8 };
        records.push(r);
      }
      return { status: "ok", root: node, records, elements: true, total: node.child_count, cutAt: null };
    }
  }
  const kids = gather(doc, node, 0, { left: MAX_MEMBERS });
  if (kids === null) return { status: "pending" };
  // A record with more children than are read ends at the last one read:
  // the bytes after it are fields not drawn, not a gap.
  const cut = node.child_count > MAX_MEMBERS;
  const r = coverGaps(buildRecord(node, kids, cut), coverFor(doc, node));
  if (r.endBit - r.startBit > MAX_RECORD_BYTES * 8) return { status: "too-large", bytes: (r.endBit - r.startBit) / 8 };
  return { status: "ok", root: node, records: [r], elements: false, total: 1, cutAt: cut ? { shown: kids.length, total: node.child_count } : null };
}

// ----- the draw -----

/** Layout pixels: the parts of a row, left to right, and their heights. */
const GUTTER_MIN = 58;
/** Room either side of an offset in the gutter. */
const GUTTER_PAD = 6;
/** Room for the bracket of each level of structure inside the record. */
const NEST_W = 7;
const KEY_MIN = 120;
const KEY_MAX = 260;
const KEY_GAP = 10;
const METRICS = { laneH: 24, breakH: 16, titleH: 22, blockGap: 8 };
/** The tallest the figure's view gets: past this it pans and zooms. */
const MAX_VIEW_H = 440;
/** Names inside boxes, the key, and the gutter's offsets. */
const LABEL_PX = 11;
const LABEL_PAD = 4;
const LABEL_MIN_CHARS = 3;
const OFFSET_PX = 10;
/** A row shorter than this on screen has no room for its offset or names. */
const TEXT_ROOM_PX = 13;
/** How wide one byte may get at the deepest zoom. */
const MAX_BYTE_PX = 160;

const SVG = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: { readonly [k: string]: string | number } = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

let sansMeasure: Measure | null = null;
function measurer(): Measure {
  sansMeasure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return sansMeasure;
}
const MONO = monoMeasure();

/** `+0x1c`: an offset from the start of a record, in the hex every address
 *  in the app is written in. */
function relText(bits: number): string {
  const bytes = Math.floor(bits / 8);
  const rest = bits % 8;
  return `+0x${bytes.toString(16).padStart(2, "0")}${rest === 0 ? "" : `.${rest}`}`;
}

/** The view each figure was left at, by document and path, so a figure the
 *  listing draws again comes back where the reader had it. */
const VIEWS = new WeakMap<Doc, Map<string, View>>();
function viewsOf(doc: Doc): Map<string, View> {
  let m = VIEWS.get(doc);
  if (m === undefined) {
    m = new Map();
    VIEWS.set(doc, m);
  }
  return m;
}

/** The byte numbers across the top of the rows, which stay put while the
 *  rows pan under them. */
const RULER_H = 16;

/** The view's height for rows `height` pixels tall, ruler included. */
export function viewHeight(height: number): number {
  return Math.max(METRICS.laneH + RULER_H, Math.min(MAX_VIEW_H, Math.ceil(height) + RULER_H));
}

type Hit =
  | { readonly kind: "piece"; readonly block: number; readonly piece: Piece<N> }
  | { readonly kind: "group"; readonly block: number; readonly node: N; readonly startBit: number; readonly endBit: number }
  | { readonly kind: "title"; readonly block: number };

export type StructLayoutFigure = {
  readonly el: SVGSVGElement;
  readonly board: Board;
  /** Draw again `width` pixels wide. */
  resize(width: number): void;
  destroy(): void;
};

/**
 * The figure itself: an SVG `width` pixels wide and as tall as `viewHeight`
 * says for its rows, which the caller can know before drawing. `key` is where
 * its zoom is remembered.
 */
export function drawStructLayout(doc: Doc, data: Extract<StructLayoutData, { status: "ok" }>, opts: { readonly width: number; readonly bytesPerRow: number; readonly key: string; readonly onZoom?: (zoomed: boolean) => void }): StructLayoutFigure {
  const bytesPerRow = opts.bytesPerRow;
  const records = data.records;
  const rows = wrapRows(records, bytesPerRow, METRICS, data.elements);
  const viewH = viewHeight(rows.height);
  const depth = Math.max(0, ...records.flatMap((r) => r.groups.map((g) => g.depth + 1)));
  // Wide enough for the longest offset the rows count to.
  const longest = Math.max(0, ...records.map((r) => r.endBit - r.startBit));
  const gutter = Math.max(GUTTER_MIN, Math.ceil(MONO(relText(longest), OFFSET_PX)) + 2 * GUTTER_PAD);
  let width = Math.max(200, opts.width);

  const root = svg("svg", { width, height: viewH, class: "sl-svg", role: "img" });
  root.setAttribute("aria-label", VIZ.structLayout.title(data.root.name));
  const defs = svg("defs");
  const uid = `sl${Math.random().toString(36).slice(2, 8)}`;
  // The two hatches: padding the lighter, single one; unread bytes the
  // crossed one, since they are the stretch a reader asks about first.
  const hatch = (name: string, crossed: boolean): void => {
    const p = svg("pattern", { id: `${uid}-${name}`, width: 6, height: 6, patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)" });
    p.append(svg("line", { x1: 0, y1: 0, x2: 0, y2: 6, class: "sl-hatch" }));
    if (crossed) p.append(svg("line", { x1: 0, y1: 3, x2: 6, y2: 3, class: "sl-hatch" }));
    defs.append(p);
  };
  hatch("padding", false);
  hatch("unread", true);
  const clip = svg("clipPath", { id: `${uid}-clip` });
  const clipRect = svg("rect", { x: 0, y: 0, width: 0, height: viewH });
  clip.append(clipRect);
  defs.append(clip);
  const gridG = svg("g", { "clip-path": `url(#${uid}-clip)` });
  const sideG = svg("g");
  root.append(defs, gridG, sideG);

  /** Where each piece's label goes: the line and cell where its box is
   *  widest, so a field split over two rows is named once. */
  const homes = new Map<string, Cell<N>>();
  rows.lines.forEach((line) => {
    if (line.kind !== "row") return;
    for (const cell of line.cells) {
      const k = `${line.block}:${cell.index}`;
      const was = homes.get(k);
      if (was === undefined || cell.x1 - cell.x0 > was.x1 - was.x0) homes.set(k, cell);
    }
  });

  let view: View = { k: 1, x: 0, y: 0 };
  const bus = linkedFor(doc);
  let selected: readonly number[] | null = bus.state.selection?.path ?? null;
  let targets: HitTarget<Hit>[] = [];

  const geometry = (): { x0: number; w: number; keyX: number; keyW: number } => {
    const keyW = Math.max(KEY_MIN, Math.min(KEY_MAX, Math.round(width * 0.22)));
    const x0 = gutter + depth * NEST_W;
    const w = Math.max(40, width - x0 - keyW - KEY_GAP);
    return { x0, w, keyX: x0 + w + KEY_GAP, keyW };
  };

  const isOn = (path: readonly number[]): boolean => selected !== null && selected.length >= path.length && path.every((s, i) => s === selected?.[i]);

  const draw = (): void => {
    const g = geometry();
    clipRect.setAttribute("x", String(g.x0));
    clipRect.setAttribute("width", String(g.w));
    const k = view.k;
    const X = (f: number): number => applyX(view, g.x0 + f * g.w);
    const Y = (y: number): number => applyY(view, y + RULER_H);
    const grid: SVGElement[] = [];
    const side: SVGElement[] = [];
    const hits: HitTarget<Hit>[] = [];
    let number = 0;
    const numbers = new Map<string, number>();
    const measure = measurer();
    const gridRight = g.x0 + g.w;
    const clipX = (x: number): number => Math.max(g.x0, Math.min(gridRight, x));

    for (const line of rows.lines) {
      const top = Y(line.y);
      const h = line.h * k;
      if (top + h < 0 || top > viewH) continue;
      const record = records[line.block] as StructRecord<N>;
      if (line.kind === "title") {
        drawTitle(side, hits, record, line, top, h);
        continue;
      }
      const roomy = h >= TEXT_ROOM_PX;
      // The offset in the gutter, counted from the record's start.
      if (roomy && line.kind === "row") {
        const t = svg("text", { x: gutter - GUTTER_PAD, y: top + Math.min(h, METRICS.laneH * k) / 2, class: "sl-offset", "text-anchor": "end", "dominant-baseline": "central" });
        t.textContent = relText(line.rel);
        side.push(t);
      }
      if (line.kind === "break") {
        drawBreak(grid, hits, line, top, h, X, clipX, measure);
        continue;
      }
      const laneH = (line.h / record.lanes) * k;
      const { labels, key } = rowLabels(
        line.cells,
        (cell) => clipX(X(cell.x1)) - clipX(X(cell.x0)),
        (index) => homes.get(`${line.block}:${index}`) === line.cells.find((c) => c.index === index),
        (p) => nameOf(p),
        (index) => {
          const nk = `${line.block}:${index}`;
          let v = numbers.get(nk);
          if (v === undefined) {
            v = ++number;
            numbers.set(nk, v);
          }
          return v;
        },
        { size: LABEL_PX, pad: LABEL_PAD, minChars: LABEL_MIN_CHARS, measure: roomy ? measure : () => Number.POSITIVE_INFINITY },
      );
      for (const cell of line.cells) {
        const x0 = X(cell.x0);
        const x1 = X(cell.x1);
        const y0 = top + cell.lane * laneH;
        const p = cell.piece;
        // A piece carried on from the row above, or on to the row below, is
        // drawn open on that side and flush with the next row, so a field
        // that wraps reads as one box.
        const rowStart = record.startBit + line.rel;
        const above = p.startBit < rowStart;
        const below = p.endBit > rowStart + bytesPerRow * 8;
        const cls = p.kind === "field" ? `sl-box ${fieldClass(p.node.kind)}${isOn(p.node.path) ? " is-on" : ""}` : `sl-gap sl-${p.kind}`;
        grid.push(...box(x0, x1, y0, y0 + laneH, above, below, cls, p.kind === "field" ? null : `url(#${uid}-${p.kind})`));
        // A field the format calls padding keeps its colour, hatched over.
        if (p.kind === "field" && p.padding) grid.push(svg("rect", { x: x0 + 0.5, y: y0 + 1, width: Math.max(1, x1 - x0 - 1), height: Math.max(1, laneH - 2), fill: `url(#${uid}-padding)`, class: "sl-padfield" }));
        const label = labels.get(cell);
        if (label !== undefined && laneH >= TEXT_ROOM_PX) {
          const left = Math.max(clipX(x0), g.x0);
          const t = svg("text", { x: left + LABEL_PAD, y: y0 + laneH / 2, class: label.keyed ? "sl-name sl-num" : p.kind === "field" ? "sl-name" : "sl-name sl-gapname", "dominant-baseline": "central" });
          t.textContent = label.text;
          grid.push(t);
        }
        hits.push({ x0: clipX(x0), y0, x1: clipX(x1), y1: y0 + laneH, value: { kind: "piece", block: line.block, piece: p } });
      }
      // The key beside the row: structures starting on it, then the names
      // with no room in their boxes, in the order they start.
      if (!roomy) continue;
      const starts = record.groups.filter((gr) => gr.startBit - record.startBit >= line.rel && gr.startBit - record.startBit < line.rel + bytesPerRow * 8);
      const entries = [...starts.map((gr) => ({ text: `${gr.node.name} {`, index: -1 - record.groups.indexOf(gr) })), ...key];
      if (entries.length === 0) continue;
      const fitted = fitKey(entries, g.keyW, KEY_GAP, LABEL_PX, measure, VIZ.structLayout.keyMore);
      let kx = g.keyX;
      const ky = top + Math.min(h, METRICS.laneH * k) / 2;
      for (const e of fitted.shown) {
        const w = measure(e.text, LABEL_PX);
        const t = svg("text", { x: kx, y: ky, class: e.index < 0 ? "sl-key sl-key-group" : "sl-key", "dominant-baseline": "central" });
        t.textContent = e.text;
        side.push(t);
        if (e.index < 0) {
          const gr = record.groups[-1 - e.index] as (typeof record.groups)[number];
          hits.push({ x0: kx, y0: ky - 8, x1: kx + w, y1: ky + 8, value: { kind: "group", block: line.block, node: gr.node, startBit: gr.startBit, endBit: gr.endBit } });
        } else {
          hits.push({ x0: kx, y0: ky - 8, x1: kx + w, y1: ky + 8, value: { kind: "piece", block: line.block, piece: record.pieces[e.index] as Piece<N> } });
        }
        kx += w + KEY_GAP;
      }
      if (fitted.rest > 0) {
        const t = svg("text", { x: kx, y: ky, class: "sl-key sl-key-more", "dominant-baseline": "central" });
        t.textContent = VIZ.structLayout.keyMore(fitted.rest);
        side.push(t);
      }
    }

    // A bracket in the margin for each structure inside the record, from its
    // first row to its last, nearest the bytes for the deepest.
    for (const [block, record] of records.entries()) {
      for (const gr of record.groups) {
        const span = linesOf(rows.lines, block, gr.startBit - record.startBit, gr.endBit - record.startBit, bytesPerRow);
        if (span === null) continue;
        const y0 = Y(span.y0) + 2;
        const y1 = Y(span.y1) - 2;
        if (y1 < 0 || y0 > viewH) continue;
        const x = g.x0 - (depth - gr.depth) * NEST_W + 2;
        const path = svg("path", { d: `M${x + 4},${y0} H${x} V${y1} H${x + 4}`, class: `sl-bracket${isOn(gr.node.path) ? " is-on" : ""}` });
        side.push(path);
        hits.push({ x0: x - 2, y0, x1: x + 5, y1, value: { kind: "group", block, node: gr.node, startBit: gr.startBit, endBit: gr.endBit } });
      }
    }
    // The ruler over the rows: a byte's number over its column, kept only
    // where it clears its neighbours when zoomed out.
    const ruler: SVGElement[] = [svg("rect", { x: 0, y: 0, width, height: RULER_H, class: "sl-ruler-bg" })];
    const marks: { start: number; end: number; priority: number }[] = [];
    const at: number[] = [];
    for (let i = 0; i < bytesPerRow; i++) {
      const x = X((i + 0.5) / bytesPerRow);
      const w = MONO(i.toString(16), OFFSET_PX);
      at.push(x);
      marks.push({ start: x - w / 2, end: x + w / 2, priority: i % 4 === 0 ? 2 : i % 2 === 0 ? 1 : 0 });
    }
    const keep = thinLabels(marks, 4);
    for (let i = 0; i < bytesPerRow; i++) {
      const x = at[i] as number;
      if (keep[i] !== true || x < g.x0 || x > gridRight) continue;
      const t = svg("text", { x, y: RULER_H / 2 + 1, class: "sl-ruler", "text-anchor": "middle", "dominant-baseline": "central" });
      t.textContent = i.toString(16);
      ruler.push(t);
    }
    gridG.replaceChildren(...grid);
    sideG.replaceChildren(...side, ...ruler);
    targets = hits;
  };

  /** An element's heading, for a list drawn element by element. */
  const drawTitle = (side: SVGElement[], hits: HitTarget<Hit>[], record: StructRecord<N>, line: Line<N>, top: number, h: number): void => {
    if (h < TEXT_ROOM_PX) return;
    const n = record.node;
    const text = `${n.name} · ${formatAddress(record.startBit, n.space)} · ${bitSizeText(record.endBit - record.startBit)}`;
    const t = svg("text", { x: 4, y: top + h / 2 + 1, class: `sl-title${isOn(n.path) ? " is-on" : ""}`, "dominant-baseline": "central" });
    t.textContent = text;
    side.push(t);
    hits.push({ x0: 0, y0: top, x1: Math.min(width, 8 + measurer()(text, LABEL_PX)), y1: top + h, value: { kind: "title", block: line.block } });
  };

  /** A break: the rows between a long field's first and last, as one band. */
  const drawBreak = (grid: SVGElement[], hits: HitTarget<Hit>[], line: Extract<Line<N>, { kind: "break" }>, top: number, h: number, X: (f: number) => number, clipX: (x: number) => number, measure: Measure): void => {
    const p = line.piece;
    const x0 = X(0);
    const x1 = X(1);
    const cls = p.kind === "field" ? `sl-fold ${fieldClass(p.node.kind)}${isOn(p.node.path) ? " is-on" : ""}` : `sl-gapfold sl-gap sl-${p.kind}`;
    grid.push(...box(x0, x1, top, top + h, true, true, cls, p.kind === "field" ? null : `url(#${uid}-${p.kind})`));
    if (h >= TEXT_ROOM_PX - 2) {
      const bytes = line.rows * bytesPerRow;
      const text = p.kind === "field" ? VIZ.structLayout.folded(line.rows, p.node.name, bytes) : VIZ.structLayout.foldedGap(line.rows, bytes);
      const left = clipX(x0);
      const right = clipX(x1);
      if (measure(text, LABEL_PX - 1) + 2 * LABEL_PAD <= right - left) {
        const t = svg("text", { x: (left + right) / 2, y: top + h / 2, class: "sl-foldname", "text-anchor": "middle", "dominant-baseline": "central" });
        t.textContent = text;
        grid.push(t);
      }
    }
    hits.push({ x0: clipX(x0), y0: top, x1: clipX(x1), y1: top + h, value: { kind: "piece", block: line.block, piece: p } });
  };

  const g0 = geometry();
  const content = { x0: g0.x0, y0: 0, x1: g0.x0 + g0.w, y1: rows.height + RULER_H };
  const minScale = Math.min(1, viewH / Math.max(1, rows.height + RULER_H));
  const zoom = board(root, {
    width,
    height: viewH,
    minScale,
    maxScale: Math.max(4, MAX_BYTE_PX / (g0.w / bytesPerRow)),
    content,
    onZoom: (v) => {
      view = v;
      viewsOf(doc).set(opts.key, v);
      opts.onZoom?.(!(v.k === 1 && v.x === 0 && v.y === 0));
      draw();
    },
  });
  // Back where the reader left it, when the listing drew the card again.
  const was = viewsOf(doc).get(opts.key);
  if (was !== undefined && !(was.k === 1 && was.x === 0 && was.y === 0)) {
    zoom.zoomTo({ x0: invertX(was, 0), y0: invertY(was, 0), x1: invertX(was, width), y1: invertY(was, viewH) });
  }

  const hitAt = (e: MouseEvent): Hit | null => {
    const r = root.getBoundingClientRect();
    // Rows panned up under the ruler are hidden there, and not hit.
    if (e.clientY - r.top < RULER_H) return null;
    return nearestTarget(targets, e.clientX - r.left, e.clientY - r.top)?.value ?? null;
  };
  const bitsOf = (hit: Hit): { startBit: number; endBit: number } => {
    if (hit.kind === "piece") return { startBit: hit.piece.startBit, endBit: hit.piece.endBit };
    if (hit.kind === "group") return { startBit: hit.startBit, endBit: hit.endBit };
    const r = records[hit.block] as StructRecord<N>;
    return { startBit: r.startBit, endBit: r.endBit };
  };

  const stopTip = hoverTip(root, (e) => {
    const hit = hitAt(e);
    bus.publish({ hover: hit === null ? null : bitsOf(hit) });
    if (hit === null) return null;
    return tipFor(hit, records[hit.block] as StructRecord<N>);
  });
  const leave = (): void => bus.publish({ hover: null });
  root.addEventListener("pointerleave", leave);
  root.addEventListener("click", (e) => {
    // The listing reads a click in a card as nothing of its own, but the
    // figure answers it here and nowhere else.
    e.stopPropagation();
    const hit = hitAt(e);
    if (hit === null) return;
    if (hit.kind === "piece") {
      const p = hit.piece;
      if (p.kind === "field") goToField(doc, { path: p.node.path, startBit: p.startBit, endBit: p.endBit });
      else goToBits(doc, [{ startBit: p.startBit, endBit: p.endBit }]);
    } else {
      const n = hit.kind === "group" ? hit.node : (records[hit.block] as StructRecord<N>).node;
      const b = bitsOf(hit);
      goToField(doc, { path: n.path, startBit: b.startBit, endBit: b.endBit });
    }
  });
  const stopLinked = subscribeWhileConnected(bus, root, (state, changed) => {
    if (changed !== "selection") return;
    selected = state.selection?.path ?? null;
    draw();
  });

  draw();
  return {
    el: root,
    board: zoom,
    resize: (w) => {
      width = Math.max(200, w);
      root.setAttribute("width", String(width));
      const g = geometry();
      zoom.resize(width, viewH, { x0: g.x0, y0: 0, x1: g.x0 + g.w, y1: rows.height + RULER_H });
      draw();
    },
    destroy: () => {
      zoom.destroy();
      stopTip();
      stopLinked();
      hideTip();
      root.removeEventListener("pointerleave", leave);
    },
  };
}

/** A box from `x0` to `x1` and `y0` to `y1`: a fill, and an outline with
 *  its top and bottom left off where the piece carries on into the next row
 *  (`openTop`, `openBottom`). */
function box(x0: number, x1: number, y0: number, y1: number, openTop: boolean, openBottom: boolean, cls: string, fill: string | null): SVGElement[] {
  const l = x0 + 0.5;
  const r = Math.max(l + 1, x1 - 0.5);
  const t = openTop ? y0 : y0 + 1;
  const b = Math.max(t + 1, openBottom ? y1 : y1 - 1);
  const rect = svg("rect", { x: l, y: t, width: r - l, height: b - t, class: `${cls} sl-fill` });
  if (fill !== null) rect.setAttribute("fill", fill);
  const d = [`M${l},${t} V${b}`, `M${r},${t} V${b}`];
  if (!openTop) d.push(`M${l},${t} H${r}`);
  if (!openBottom) d.push(`M${l},${b} H${r}`);
  return [rect, svg("path", { d: d.join(" "), class: `${cls} sl-edge` })];
}

/** The label a piece would like inside its box. A gap's is only its size. */
function nameOf(p: Piece<N>): { readonly full: string; readonly short: string | null } {
  if (p.kind === "field") return { full: p.node.name, short: null };
  const bytes = (p.endBit - p.startBit) / 8;
  if (!Number.isInteger(bytes)) return { full: bitSizeText(p.endBit - p.startBit), short: null };
  const S = VIZ.structLayout;
  const full = p.kind === "padding" ? S.padding(bytes) : p.kind === "other" ? (p.owners.length === 1 ? S.gapOf(bytes, p.owners[0] as string) : S.gapOfOthers(bytes)) : S.unread(bytes);
  return { full, short: S.gapShort(bytes) };
}

/** The first and last layout pixel of the rows a stretch of a record covers,
 *  in block `block`. */
function linesOf(lines: readonly Line<N>[], block: number, rel0: number, rel1: number, bytesPerRow: number): { y0: number; y1: number } | null {
  const rowBits = bytesPerRow * 8;
  let y0 = Number.POSITIVE_INFINITY;
  let y1 = Number.NEGATIVE_INFINITY;
  for (const l of lines) {
    if (l.block !== block || l.kind === "title") continue;
    const end = l.kind === "break" ? l.rel + l.rows * rowBits : l.rel + rowBits;
    if (end <= rel0 || l.rel >= rel1) continue;
    y0 = Math.min(y0, l.y);
    y1 = Math.max(y1, l.y + l.h);
  }
  return y0 <= y1 ? { y0, y1 } : null;
}

/** The hover card for whatever the pointer is on. */
function tipFor(hit: Hit, record: StructRecord<N>): HTMLElement {
  const S = VIZ.structLayout;
  if (hit.kind === "title") {
    const n = record.node;
    const sum = summarise(record);
    return tipCard(n.name, [`${n.type} · ${S.size(sum.bytes, sum.fields)}`], formatAddress(record.startBit, n.space));
  }
  if (hit.kind === "group") {
    const n = hit.node;
    return tipCard(n.name, [`${n.type} · ${countText(n.child_count, childWord(n))}`, S.fromStart(relText(hit.startBit - record.startBit), bitSizeText(hit.endBit - hit.startBit))], formatAddress(hit.startBit, n.space));
  }
  const p = hit.piece;
  const rel = relText(p.startBit - record.startBit);
  const size = bitSizeText(p.endBit - p.startBit);
  const where = `${formatAddress(p.startBit, record.node.space)} · ${size}`;
  if (p.kind === "field") {
    const n = p.node;
    const lines = [n.composite ? `${n.type} · ${countText(n.child_count, childWord(n))}` : `${n.type} · ${n.value}`, S.fromStart(rel, size)];
    const natural = naturalAlign(n);
    if (natural !== null) lines.push(misaligned(n, p.startBit - record.startBit) ? S.notAligned(natural, rel) : S.aligned(natural));
    if (p.padding) lines.push(S.namedPadding);
    return tipCard(n.name, lines, where);
  }
  const line = (() => {
    if (p.kind === "unread") return S.gapUnread;
    if (p.kind === "other") return S.gapOther(record.node.name, p.owners);
    if (p.next !== null) return S.gapPaddingNext(p.next.name, naturalAlign(p.next) ?? 1);
    const widest = Math.max(1, ...record.pieces.map((q) => (q.kind === "field" ? (naturalAlign(q.node) ?? 1) : 1)));
    return S.gapPaddingTail(record.node.name, widest);
  })();
  return tipCard(nameOf(p).full, [line, S.fromStart(rel, size)], where);
}

// ----- the card -----

/** The card set's key for the row width chosen for one structure. */
function rowKey(path: readonly number[], n: number): string {
  return `sl-row:${pathKey(path)}:${n}`;
}
function rowWidthOf(c: DrawContext, path: readonly number[], fallback: number): number {
  for (const n of ROW_WIDTHS) if (c.cards.has(rowKey(path, n))) return n;
  return fallback;
}

/** The summary line under the heading: size and fields, then the gaps, then
 *  any number off its alignment. For a list, how many elements it has and
 *  their size when they are all one size, then the gaps in the elements
 *  drawn, added up. */
function summaryText(data: Extract<StructLayoutData, { status: "ok" }>): string {
  const S = VIZ.structLayout;
  const sums = data.records.map((r) => summarise(r));
  const first = sums[0];
  if (first === undefined) return "";
  const parts: string[] = [];
  if (data.elements) {
    const word = childWord(data.root);
    // "Each" only when every element was drawn: the rest were not read.
    const alike = data.total === sums.length && sums.every((s) => s.bytes === first.bytes && s.fields === first.fields);
    parts.push(alike ? S.elementsEach(data.total, word, first.bytes, first.fields) : countText(data.total, word));
  } else parts.push(S.size(first.bytes, first.fields));
  const padding = sums.reduce((n, s) => n + s.paddingBytes, 0);
  const unread = sums.reduce((n, s) => n + s.unreadBytes, 0);
  const other = sums.reduce((n, s) => n + s.otherBytes, 0);
  const gaps = sums.reduce((n, s) => n + s.gaps, 0);
  const off = sums.reduce((n, s) => n + s.misaligned, 0);
  if (padding > 0) parts.push(S.padding(padding));
  if (unread > 0) parts.push(S.unread(unread));
  if (other > 0) parts.push(S.other(other));
  if (gaps === 0) parts.push(S.noGaps);
  if (off > 0) parts.push(S.misaligned(off));
  return parts.join(" · ");
}

const HEAD_H = 24;
const SUMMARY_H = 18;
const FOOT_H = 22;
const NOTE_H = 18;

/** The card in place of a heading's fields. */
function drawCard(c: DrawContext, item: Extract<Item, { kind: "formatcard" }>): HTMLElement {
  const S = VIZ.structLayout;
  const host = el("div", "sl-card");
  const body = el("div", "sl-body");
  host.append(body);
  const node = item.node;
  const data = structLayoutData(c.doc, node);

  const head = el("div", "sl-head");
  head.style.height = `${HEAD_H}px`;
  head.append(el("b", "sl-heading", S.title(node.name)));
  body.append(head);

  if (data.status !== "ok") {
    const note = el("div", "sl-summary", data.status === "pending" ? S.reading : S.tooLarge(formatBytes(data.bytes)));
    note.style.height = `${SUMMARY_H}px`;
    body.append(note);
    if (data.status === "pending") {
      // Drawn again once the fields are in. A card's height is measured by
      // the listing only when it lays its rows out, and asking for that is
      // what turning a card key over does; this key is the card's own and is
      // turned straight back.
      const stop = c.doc.onChange(() => {
        if (!host.isConnected) {
          stop();
          return;
        }
        if (structLayoutData(c.doc, node).status === "pending") return;
        stop();
        const k = `sl-refresh:${pathKey(node.path)}`;
        c.toggleCard(k);
        c.toggleCard(k);
      });
    }
    return host;
  }

  const first = data.records[0];
  const fallback = defaultRowWidth(first === undefined ? 0 : (first.endBit - first.startBit) / 8);
  const bytesPerRow = rowWidthOf(c, node.path, fallback);

  // Row width, then Reset zoom, at the right of the heading.
  const controls = el("span", "sl-controls");
  const group = el("span", "sl-rowwidth");
  group.setAttribute("role", "group");
  group.setAttribute("aria-label", S.rowWidth);
  group.append(el("span", "sl-rowwidth-label", S.rowWidth));
  for (const n of ROW_WIDTHS) {
    const b = el("button", `sl-seg${n === bytesPerRow ? " is-on" : ""}`, String(n));
    b.type = "button";
    b.title = S.rowWidthTitle(n);
    b.setAttribute("aria-pressed", String(n === bytesPerRow));
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      if (n === bytesPerRow) return;
      viewsOf(c.doc).delete(pathKey(node.path));
      const old = rowKey(node.path, bytesPerRow);
      if (c.cards.has(old)) c.toggleCard(old);
      c.toggleCard(rowKey(node.path, n));
    });
    group.append(b);
  }
  const reset = el("button", "viz-reset", VIZ.resetZoom);
  reset.type = "button";
  controls.append(group, reset);
  head.append(controls);

  const summary = el("div", "sl-summary", summaryText(data));
  summary.style.height = `${SUMMARY_H}px`;
  body.append(summary);

  const rows = wrapRows(data.records, bytesPerRow, METRICS, data.elements);
  const frame = el("div", "sl-frame");
  frame.style.height = `${viewHeight(rows.height)}px`;
  body.append(frame);

  const placed = data.elements ? [] : (data.records[0]?.placed ?? []);
  if (placed.length > 0) {
    const foot = el("div", "sl-foot");
    foot.style.height = `${FOOT_H}px`;
    foot.append(el("span", "sl-foot-label", S.placed));
    for (const p of placed) {
      const target = pointee(c.doc, p);
      const b = el("button", "sl-link");
      b.type = "button";
      b.append(document.createTextNode(p.name), el("span", "sl-link-at", target.size_bits > 0 ? formatAddress(target.offset_bits, target.space) : ""));
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        goToField(c.doc, { path: target.path, startBit: target.offset_bits, endBit: target.offset_bits + target.size_bits });
      });
      foot.append(b);
    }
    body.append(foot);
  }
  if (data.elements && data.total > data.records.length) {
    const more = el("div", "sl-note", VIZ.childStrip.firstOf(data.records.length, data.total, childWord(node)));
    more.style.height = `${NOTE_H}px`;
    body.append(more);
  } else if (data.cutAt !== null) {
    const more = el("div", "sl-note", VIZ.childStrip.firstOf(data.cutAt.shown, data.cutAt.total, childWord(node)));
    more.style.height = `${NOTE_H}px`;
    body.append(more);
  }

  // Drawn at the width the card turns out to have, and again whenever that
  // changes. The observer reports before the first paint, so the figure is
  // never seen without its names.
  let figure: StructLayoutFigure | null = null;
  const observer = new ResizeObserver((entries) => {
    if (!host.isConnected) {
      observer.disconnect();
      figure?.destroy();
      return;
    }
    const w = Math.floor(entries[0]?.contentRect.width ?? 0);
    if (w <= 0) return;
    if (figure === null) {
      figure = drawStructLayout(c.doc, data, {
        width: w,
        bytesPerRow,
        key: pathKey(node.path),
        onZoom: (zoomed) => {
          reset.disabled = !zoomed;
        },
      });
      frame.append(figure.el);
      reset.onclick = (e) => {
        e.stopPropagation();
        figure?.board.home(true);
      };
    } else figure.resize(w);
  });
  observer.observe(frame);
  reset.disabled = true;
  return host;
}

/** What a field reached by offset points at, for the link under the figure:
 *  the one child an `at →` field holds, or the field itself. */
function pointee(doc: Doc, p: N): N {
  if (p.size_bits === 0 && p.composite && p.child_count === 1) {
    const r = doc.templateChildren(p.path, 0, 1);
    const only = r.status === "ok" ? r.node[0] : undefined;
    if (only !== undefined) return only;
  }
  return p;
}

/** Whether a heading's structure can be drawn as a layout. Asked of every
 *  heading on every paint, so it looks at the node alone. */
function offers(n: TemplateNode): boolean {
  // A list placed by offsets is drawn as its elements where they landed, gaps
  // between them included; only a pointer's own row (an `at →` field, of no
  // bytes) has nothing here to draw, and the listing draws its target under
  // the heading instead.
  return n.composite && n.child_count > 0 && n.size_bits > 0 && !n.decoded && !n.framed;
}

/** A card in place of any structure's fields, asked for from a button on its
 *  heading in the listing. */
export function register(): void {
  registerKindCard({
    id: ID,
    offers: (_doc, n) => offers(n),
    draw: drawCard,
  });
  registerHeadingButton({
    id: ID,
    applies: (c, n) => {
      if (!offers(n)) return false;
      // A format's own card for this structure comes first, and asking for
      // this one there would do nothing.
      return formatCardKind(c.doc, n) === null;
    },
    draw: (c, n) => {
      const key = askedCardKey(ID, n.path);
      const on = c.cards.has(key);
      return headingButton(VIZ.structLayout.button, on ? VIZ.structLayout.buttonTitleOn(n.name) : VIZ.structLayout.buttonTitle(n.name), on, () => c.toggleCard(key));
    },
  });
}

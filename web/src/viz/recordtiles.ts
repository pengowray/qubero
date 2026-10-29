// Small multiples: one tile per record of a list, under a listing heading.
//
// Candidate `record-small-multiples` in docs/DESIGN-diagrams.md, section 4.2,
// the heading tile grid of its first-batch slice (section 5.3, agent I). The
// sparkline half of the slice is `sparkline.ts`.
//
// A list read row by row says what each record is and nothing about the list:
// whether the IDAT chunks are all one size, where the one odd chunk sits,
// which ZIP records are data descriptors. The tiles are that list at a glance.
// Every record is one square, in file order, wrapping like text; its colour is
// what kind of record it is and its fill is its size against the largest, so
// a run of equal records is a run of equal squares and the one that differs
// is the one that looks different. A record with a wrong value inside it has
// a red edge, the colour that means a wrong value everywhere else.
//
// Where it is drawn. A heading offers "View chunks as tiles" when it is an
// element of a list (a PNG chunk, a ZIP record) and when its contents are a
// list (a MIDI track's events). The figure opens inside the heading, under its
// title line (`headingfigures.ts`), so the fields of the part are still there
// under it: the tiles say where to look, and the rows are what is there. One
// list is drawn under one heading at a time.
//
// Explorable, by the design's first rule: the wheel zooms about the pointer
// and a drag pans (`board.ts`); hovering a tile gives its name, type, size and
// address (`tip.ts`) and marks its bytes in the hex view (`linked.ts`); a
// click goes to the record (`pick.ts`). The record the cursor is in is
// outlined, whichever view moved it. Zoomed in far enough, each tile shows the
// bytes of its own fields as a strip along its top, and then its type.
// Clicking a type in the legend dims every other type.
//
// Drawn on a canvas, one rectangle per tile, so a list of thousands costs
// one draw per frame; the arithmetic is `tilelayout.ts`.

import type { Doc, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { fieldClass, UNMAPPED_COLOR } from "../fieldstyle.ts";
import { isElementName } from "../elementname.ts";
import type { DrawContext } from "../listingdraw.ts";
import { stripIndex, typeWords, variantText } from "../report/partrules.ts";
import { bitSizeText, REPORT, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { applyX, applyY, invertX, invertY, type View } from "./boardmath.ts";
import { canvasMeasure, ellipsise, type Measure } from "./fit.ts";
import { headingButton, registerHeadingButton } from "./headingbuttons.ts";
import { figureKey, figurePath, registerHeadingFigure } from "./headingfigures.ts";
import { linkedFor, type LinkedBus } from "./linked.ts";
import { LOD } from "./lod.ts";
import { goToField } from "./pick.ts";
import { assignCategories, fillPx, OTHER_SLOT, recordWord, tileAt, tileBox, tileGrid, type Categories, type TileGrid } from "./tilelayout.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

/** Most records read for one grid. Past this the grid says how many it left
 *  out; reading every record of a list of a million is a different piece. */
const READ_MAX = 4096;
/** Records read per call to the core. */
const READ_PAGE = 512;
/** Records whose type is looked for in what decided it (`origins`) rather
 *  than in their names alone. Each costs a few calls to the core. */
const VARIANT_MAX = 2048;
/** How tall the grid may be with no zoom, before its tiles get smaller. About
 *  five rows of the listing. */
const GRID_MAX_H = 112;
/** The largest a tile gets at the deepest zoom, in screen pixels. */
const MAX_TILE_PX = 160;
/** Room around the grid inside the canvas, so an outline drawn round an edge
 *  tile is not cut off. */
const PAD = 3;
/** A tile this wide on screen shows its fields as a strip along its top. */
const STRIP_FROM_PX = 32;
/** A tile this wide and tall on screen writes its type under the strip. */
const NAME_FROM_PX = 48;
const NAME_TALL_PX = 24;
const NAME_PX = 11;

// ----- the data -----

export type TilesData = {
  readonly list: TemplateNode;
  /** The records drawn, the first `READ_MAX` of the list. */
  readonly records: readonly TemplateNode[];
  /** Each record's type, as the legend names it. */
  readonly kinds: readonly string[];
  readonly categories: Categories;
  /** The index of the largest record, which the fill is measured against. */
  readonly largest: number;
};

/** The children of `path`, the first `READ_MAX` of them, or null while any
 *  are still being read. */
function readRecords(doc: Doc, list: TemplateNode): TemplateNode[] | null {
  const n = Math.min(list.child_count, READ_MAX);
  const out: TemplateNode[] = [];
  for (let from = 0; from < n; from += READ_PAGE) {
    const reply = doc.templateChildren(list.path, from, Math.min(n, from + READ_PAGE));
    if (reply.status === "error") return out;
    if (reply.status !== "ok") return null;
    out.push(...reply.node);
  }
  return out;
}

/**
 * What kind of record this is, in the words the legend uses. What decided the
 * record's case first, as the report names its parts (`variantOf` in
 * `report/model.ts`): a ZIP record by its signature, a PNG chunk by its type
 * field. Then the record's own name without its place in the list (`[4]
 * IDAT` is `IDAT`), then its type. The first is what tells a ZIP's local file
 * header from its central directory entry for the same file, which share a
 * name.
 */
function kindOf(doc: Doc, n: TemplateNode, probe: boolean): string {
  if (probe && n.composite) {
    const own = doc.origins(n.path);
    const direct = own.status === "ok" ? own.node.find((o) => o.role === "type" && o.value !== "") : undefined;
    const v = direct === undefined ? null : variantText(direct.value);
    if (v !== null) return v;
    const kids = doc.templateChildren(n.path, 0, Math.min(n.child_count, 8));
    if (kids.status === "ok") {
      for (const k of kids.node.filter((c) => c.composite).slice(0, 2)) {
        const o = doc.origins(k.path);
        const t = o.status === "ok" ? o.node.find((x) => x.role === "type" && x.value !== "") : undefined;
        const kv = t === undefined ? null : variantText(t.value);
        if (kv !== null) return kv;
      }
    }
  }
  const bare = stripIndex(n.name);
  return bare !== "" ? bare : typeWords(n.type);
}

/** The tiles for the list at `listPath`, or null while it is being read. */
export function tilesData(doc: Doc, list: TemplateNode): TilesData | null {
  const records = readRecords(doc, list);
  if (records === null) return null;
  const probe = records.length <= VARIANT_MAX;
  const kinds = records.map((r) => kindOf(doc, r, probe));
  let largest = 0;
  records.forEach((r, i) => {
    if (r.size_bits > (records[largest]?.size_bits ?? 0)) largest = i;
  });
  return { list, records, kinds, categories: assignCategories(kinds), largest };
}

// ----- colours -----

/** The dataviz palette's first seven hues, light and dark, in its fixed
 *  order; its eighth, red, is left for wrong values. Then the grey shared by
 *  every type past the seventh. */
const HUES_LIGHT = ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", UNMAPPED_COLOR];
const HUES_DARK = ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300", "#9085e9", "#6f7680"];

function isDark(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
}

export function slotColor(slot: number, dark = isDark()): string {
  const hues = dark ? HUES_DARK : HUES_LIGHT;
  return hues[Math.min(slot, OTHER_SLOT)] ?? UNMAPPED_COLOR;
}

/** What the page's own colours are, for a canvas, which cannot read CSS
 *  variables. Read on every draw: the theme follows the system. */
type Ink = { readonly bg: string; readonly fg: string; readonly accent: string; readonly warn: string; readonly field: (kind: string) => string };

const fieldInk = new Map<string, string>();
let fieldInkDark: boolean | null = null;

/** The colour `fieldClass` gives a kind of field, read off a probe element
 *  once per theme. */
function fieldColor(kind: string): string {
  const dark = isDark();
  if (fieldInkDark !== dark) {
    fieldInk.clear();
    fieldInkDark = dark;
  }
  const cls = fieldClass(kind);
  let c = fieldInk.get(cls);
  if (c === undefined) {
    const probe = document.createElement("span");
    probe.className = cls;
    probe.hidden = true;
    document.body.append(probe);
    c = getComputedStyle(probe).getPropertyValue("--field-color").trim() || UNMAPPED_COLOR;
    probe.remove();
    fieldInk.set(cls, c);
  }
  return c;
}

function ink(host: Element): Ink {
  const s = getComputedStyle(host);
  const v = (name: string, fallback: string): string => s.getPropertyValue(name).trim() || fallback;
  return { bg: v("--bg", "#fff"), fg: v("--fg", "#000"), accent: v("--accent", "#2457c5"), warn: v("--warn", "#b4361f"), field: fieldColor };
}

let measure: Measure | null = null;
function measurer(): Measure {
  measure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return measure;
}

// ----- the drawing -----

export type TilesFigure = { readonly el: HTMLElement; readonly board: Board; destroy(): void };

/** Draw the grid `width` pixels wide. `word` is what the list calls one
 *  record. */
export function drawRecordTiles(doc: Doc, data: TilesData, opts: { readonly width: number; readonly word: string }): TilesFigure {
  const width = Math.max(120, Math.floor(opts.width));
  const word = opts.word;
  const grid = tileGrid(data.records.length, width - 2 * PAD, GRID_MAX_H);
  // A list that fits one row zooms across only, so its tiles grow wider at
  // their own height, with room for their fields and their type; zooming it
  // both ways would grow each tile past the top and bottom of a figure one
  // tile tall. A grid of several rows zooms both ways, like a map.
  const oneRow = grid.rows <= 1;
  const height = grid.height + 2 * PAD;
  const maxBits = data.records[data.largest]?.size_bits ?? 0;

  const host = document.createElement("figure");
  host.className = "viz-tiles";
  const head = document.createElement("figcaption");
  head.className = "viz-tiles-head";
  const title = document.createElement("span");
  title.className = "viz-tiles-title";
  title.textContent = VIZ.tiles.title(data.list.child_count, word);
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  head.append(title, reset);
  host.append(head);
  if (data.list.child_count > data.records.length) {
    const more = document.createElement("div");
    more.className = "viz-tiles-note";
    more.textContent = VIZ.tiles.firstOf(data.records.length, data.list.child_count, word);
    host.append(more);
  }

  const canvas = document.createElement("canvas");
  canvas.className = "viz-tiles-canvas";
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", VIZ.tiles.title(data.list.child_count, word));
  host.append(canvas);

  // The legend: one entry per colour, each a button that dims the others.
  const legend = document.createElement("div");
  legend.className = "viz-tiles-legend";
  let only: number | null = null;
  const entries = data.categories.legend.map((entry) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "viz-tiles-key";
    const swatch = document.createElement("span");
    swatch.className = "viz-tiles-swatch";
    swatch.style.background = slotColor(entry.slot);
    const name = document.createElement("span");
    name.className = "viz-tiles-kind";
    name.textContent = entry.name ?? VIZ.tiles.otherTypes(entry.kinds.length);
    const count = document.createElement("span");
    count.className = "viz-tiles-count";
    count.textContent = entry.count.toLocaleString();
    b.append(swatch, name, count);
    const says = entry.name === null ? VIZ.tiles.highlightOther : VIZ.tiles.highlight(entry.name);
    b.title = says;
    b.setAttribute("aria-pressed", "false");
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      only = only === entry.slot ? null : entry.slot;
      for (const [i, other] of entries.entries()) {
        const on = only === data.categories.legend[i]?.slot;
        other.classList.toggle("is-on", on);
        other.setAttribute("aria-pressed", String(on));
        other.title = on ? VIZ.tiles.showAll : other.dataset["says"] ?? "";
      }
      draw();
    });
    b.dataset["says"] = says;
    legend.append(b);
    return b;
  });
  host.append(legend);
  const largest = data.records[data.largest];
  if (largest !== undefined && maxBits > 0) {
    const key = document.createElement("div");
    key.className = "viz-tiles-note";
    key.textContent = VIZ.tiles.key(word, data.kinds[data.largest] ?? largest.name, bitSizeText(maxBits));
    host.append(key);
  }

  let view: View = { k: 1, x: 0, y: 0 };
  const bus: LinkedBus = linkedFor(doc);
  let hovered: number | null = null;
  const listDepth = data.list.path.length;
  /** Which record the cursor is in, from the shared selection. */
  const selectedOf = (path: readonly number[] | undefined): number | null => {
    if (path === undefined || path.length <= listDepth) return null;
    for (let i = 0; i < listDepth; i++) if (path[i] !== data.list.path[i]) return null;
    const j = path[listDepth] as number;
    return j < data.records.length ? j : null;
  };
  let selected = selectedOf(bus.state.selection?.path);

  /** The fields of the records drawn big enough to show them, by record. */
  const fields = new Map<number, readonly TemplateNode[] | null>();
  const fieldsOf = (i: number): readonly TemplateNode[] | null => {
    if (fields.has(i)) return fields.get(i) ?? null;
    const r = data.records[i];
    if (r === undefined || !r.composite || r.child_count === 0) {
      fields.set(i, null);
      return null;
    }
    const reply = doc.templateChildren(r.path, 0, Math.min(r.child_count, 64));
    if (reply.status !== "ok") return null;
    fields.set(i, reply.node);
    return reply.node;
  };

  const draw = (): void => {
    const dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
    const wantW = Math.round(width * dpr);
    const wantH = Math.round(height * dpr);
    if (canvas.width !== wantW) canvas.width = wantW;
    if (canvas.height !== wantH) canvas.height = wantH;
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    const colours = ink(host);
    const dark = isDark();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    // Only the rows and columns on screen: at the deepest zoom a few tiles of
    // thousands are.
    const col0 = Math.max(0, Math.floor(invertX(view, 0) / grid.pitch) - 1);
    const col1 = Math.min(grid.cols - 1, Math.ceil(invertX(view, width) / grid.pitch) + 1);
    const row0 = oneRow ? 0 : Math.max(0, Math.floor(invertY(view, 0) / grid.pitch) - 1);
    const row1 = oneRow ? 0 : Math.min(grid.rows - 1, Math.ceil(invertY(view, height) / grid.pitch) + 1);
    const names: { x: number; y: number; w: number; text: string }[] = [];
    for (let row = row0; row <= row1; row++) {
      for (let col = col0; col <= col1; col++) {
        const i = row * grid.cols + col;
        const r = data.records[i];
        if (r === undefined) continue;
        const { x, y, w, h } = screenBox(i);
        const slot = data.categories.slot[i] ?? OTHER_SLOT;
        const alpha = only === null || only === slot ? 1 : 0.15;
        // The square in a pale tint of its type, then its fill from the
        // bottom in the full colour: an empty record is still its colour.
        ctx.fillStyle = slotColor(slot, dark);
        ctx.globalAlpha = alpha * (dark ? 0.32 : 0.24);
        ctx.fillRect(x, y, w, h);
        ctx.globalAlpha = alpha;
        const fill = fillPx(r.size_bits, maxBits, grid.cell) * (h / grid.cell);
        ctx.fillRect(x, y + h - fill, w, fill);
        const strip = w >= STRIP_FROM_PX ? drawFields(ctx, i, x, y, w, h, colours) : 0;
        if (w >= NAME_FROM_PX && h >= NAME_TALL_PX) {
          const text = ellipsise(data.kinds[i] ?? "", w - 8, NAME_PX, measurer(), 3);
          if (text !== null) names.push({ x: x + 4, y: y + strip + 3, w: w - 8, text });
        }
        const [invalid] = r.problems_within;
        if (invalid > 0 || r.problem?.tier === "invalid") {
          ctx.globalAlpha = 1;
          ctx.strokeStyle = colours.warn;
          ctx.lineWidth = 2;
          ctx.strokeRect(x + 1, y + 1, w - 2, h - 2);
        }
      }
    }
    ctx.globalAlpha = 1;
    // Names on a plate of the background, so a dark fill under one does not
    // take it. A name only where it fits inside its own tile, so two never
    // meet.
    if (names.length > 0) {
      ctx.font = `${NAME_PX}px ${getComputedStyle(document.body).fontFamily}`;
      ctx.textBaseline = "top";
      ctx.lineJoin = "round";
      for (const n of names) {
        ctx.strokeStyle = colours.bg;
        ctx.lineWidth = 3;
        ctx.strokeText(n.text, n.x, n.y, n.w);
        ctx.fillStyle = colours.fg;
        ctx.fillText(n.text, n.x, n.y, n.w);
      }
    }
    const outline = (i: number | null, colour: string, lineWidth: number): void => {
      if (i === null) return;
      const { x, y, w, h } = screenBox(i);
      ctx.strokeStyle = colour;
      ctx.lineWidth = lineWidth;
      const around = Math.min(PAD - 1, Math.max(1, grid.gap * view.k) / 2);
      ctx.strokeRect(x - around, y - around, w + 2 * around, h + 2 * around);
    };
    outline(hovered, colours.fg, 1);
    outline(selected, colours.accent, 2);
  };

  /** Where tile `i` is on screen under the zoom now. */
  const screenBox = (i: number): { x: number; y: number; w: number; h: number } => {
    const b = tileBox(grid, i);
    const w = grid.cell * view.k;
    if (oneRow) return { x: PAD + applyX(view, b.x0), y: PAD + b.y0, w, h: grid.cell };
    return { x: PAD + applyX(view, b.x0), y: PAD + applyY(view, b.y0), w, h: w };
  };

  /** A record's fields as a strip along the top of its tile, each as wide as
   *  its share of the record's bytes, in the colour of its kind of value.
   *  Returns how tall the strip is. */
  const drawFields = (ctx: CanvasRenderingContext2D, i: number, x: number, y: number, w: number, h: number, colours: Ink): number => {
    const r = data.records[i];
    const kids = fieldsOf(i);
    if (r === undefined || kids === null || r.size_bits <= 0) return 0;
    const sh = Math.max(4, Math.round(h * 0.16));
    ctx.save();
    ctx.globalAlpha = 1;
    ctx.fillStyle = colours.bg;
    ctx.fillRect(x, y, w, sh);
    for (const k of kids) {
      if (k.size_bits <= 0 || k.space !== r.space) continue;
      const x0 = x + ((k.offset_bits - r.offset_bits) / r.size_bits) * w;
      const kw = (k.size_bits / r.size_bits) * w;
      if (x0 >= x + w || x0 + kw <= x) continue;
      ctx.fillStyle = colours.field(k.kind);
      // A pixel short of its share, so two neighbouring fields of one kind
      // still read as two.
      ctx.fillRect(Math.max(x, x0), y, Math.max(1, Math.min(kw, x + w - x0) - 1), sh);
    }
    ctx.restore();
    return sh;
  };

  const zoom = board(canvas, {
    width,
    height,
    xOnly: oneRow,
    // Deep enough that a tile is as big as the figure has room for, and in
    // one row about as wide as a field strip needs to be read.
    maxScale: Math.max(1, (oneRow ? MAX_TILE_PX : Math.min(MAX_TILE_PX, height - 2 * PAD - 4)) / grid.cell),
    content: { x0: 0, y0: 0, x1: width, y1: height },
    onZoom: (v) => {
      view = v;
      reset.disabled = v.k === 1 && v.x === 0 && v.y === 0;
      draw();
    },
  });
  reset.addEventListener("click", (e) => {
    e.stopPropagation();
    zoom.home(true);
  });

  /** The record under a pointer event, by grid arithmetic. */
  const recordAt = (e: MouseEvent): number | null => {
    const r = canvas.getBoundingClientRect();
    const x = invertX(view, e.clientX - r.left - PAD);
    const y = oneRow ? e.clientY - r.top - PAD : invertY(view, e.clientY - r.top - PAD);
    return tileAt(grid, x, y, LOD.targetPx / view.k);
  };

  const stopTip = hoverTip(canvas, (e) => {
    const i = recordAt(e);
    if (i !== hovered) {
      hovered = i;
      draw();
    }
    const r = i === null ? undefined : data.records[i];
    bus.publish({ hover: r === undefined ? null : { startBit: r.offset_bits, endBit: r.offset_bits + r.size_bits } });
    if (r === undefined || i === null) return null;
    return tipCard(r.name, tipLines(data, i), `${formatOffset(r.offset_bits)} · ${bitSizeText(r.size_bits)}`);
  });
  const leave = (): void => {
    bus.publish({ hover: null });
    if (hovered !== null) {
      hovered = null;
      draw();
    }
  };
  canvas.addEventListener("pointerleave", leave);
  // Every click inside the figure is the figure's: the heading it sits in
  // would otherwise read it as a press on the heading and fold the part.
  host.addEventListener("click", (e) => e.stopPropagation());
  canvas.addEventListener("click", (e) => {
    const i = recordAt(e);
    const r = i === null ? undefined : data.records[i];
    if (r === undefined) return;
    goToField(doc, { path: r.path, startBit: r.offset_bits, endBit: r.offset_bits + r.size_bits });
  });
  // The record the cursor is in is outlined, whichever view moved the cursor.
  // Subscribed for as long as the figure lives rather than while it is on the
  // page: the listing takes the heading off the page and puts the same
  // figure back in the next one, and a figure that let go the first time it
  // was off the page would stop following the cursor.
  const stopLinked = bus.subscribe((state, changed) => {
    if (changed !== "selection") return;
    const next = selectedOf(state.selection?.path);
    if (next === selected) return;
    selected = next;
    if (host.isConnected) draw();
  });
  const theme = typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)") : null;
  const retheme = (): void => {
    for (const [i, entry] of data.categories.legend.entries()) {
      const sw = entries[i]?.querySelector<HTMLElement>(".viz-tiles-swatch");
      if (sw) sw.style.background = slotColor(entry.slot);
    }
    draw();
  };
  theme?.addEventListener("change", retheme);

  draw();
  return {
    el: host,
    board: zoom,
    destroy: () => {
      zoom.destroy();
      stopTip();
      stopLinked();
      theme?.removeEventListener("change", retheme);
      canvas.removeEventListener("pointerleave", leave);
    },
  };
}

/** What the hover card says about record `i`, under its name. */
function tipLines(data: TilesData, i: number): string[] {
  const r = data.records[i];
  if (r === undefined) return [];
  const lines: string[] = [];
  // The type, where the name does not already say it: a ZIP record is named
  // for its file and is a local file header or a central directory entry.
  const kind = data.kinds[i] ?? "";
  if (kind !== "" && kind !== stripIndex(r.name)) lines.push(VIZ.tiles.typeLine(kind));
  const [invalid, undef] = r.problems_within;
  const problems = VIZ.tiles.problems(invalid, undef);
  if (problems !== "") lines.push(problems);
  return lines;
}

// ----- where it is drawn -----

/** The list a heading's node is an element of, when it is one of at least
 *  two. */
function parentList(doc: Doc, node: TemplateNode): TemplateNode | null {
  if (node.path.length === 0 || !isElementName(node.name)) return null;
  const up = doc.templateNode(node.path.slice(0, -1));
  return up.status === "ok" && up.node.list && up.node.child_count >= 2 ? up.node : null;
}

/** The list a heading's node is, or holds as its contents: a MIDI track's
 *  events are its `body`, which the listing draws as the track's rows. */
function innerList(doc: Doc, node: TemplateNode): TemplateNode | null {
  if (node.list) return node.child_count >= 2 ? node : null;
  if (!node.composite || node.child_count === 0 || node.child_count > 16) return null;
  const kids = doc.templateChildren(node.path, 0, node.child_count);
  if (kids.status !== "ok") return null;
  const lists = kids.node.filter((k) => k.list && k.child_count >= 2);
  return lists.length === 1 && lists[0] !== undefined && (lists[0].contents || kids.node.length <= 4) ? lists[0] : null;
}

/** The figures drawn, by document and figure key, so a heading built again on
 *  the next paint gets the same figure, zoom and all. */
const drawn = new WeakMap<Doc, Map<string, { readonly sig: string; readonly fig: TilesFigure }>>();
/** Figures kept per document. A reader with more open than this has scrolled
 *  some far away, and those are drawn again when they come back. */
const KEEP = 8;

function cached(doc: Doc, key: string, sig: string, make: () => TilesFigure): HTMLElement {
  let byKey = drawn.get(doc);
  if (byKey === undefined) {
    byKey = new Map();
    drawn.set(doc, byKey);
  }
  const had = byKey.get(key);
  if (had !== undefined && had.sig === sig) {
    // Most recently used last, so the oldest is the one let go.
    byKey.delete(key);
    byKey.set(key, had);
    return had.fig.el;
  }
  had?.fig.destroy();
  const fig = make();
  byKey.set(key, { sig, fig });
  while (byKey.size > KEEP) {
    const oldest = byKey.keys().next().value as string;
    byKey.get(oldest)?.fig.destroy();
    byKey.delete(oldest);
  }
  return fig.el;
}

function figureFor(c: DrawContext, key: string, list: TemplateNode, width: number): HTMLElement {
  const data = tilesData(c.doc, list);
  if (data === null) {
    const wait = document.createElement("div");
    wait.className = "viz-tiles-note";
    wait.textContent = REPORT.reading;
    return wait;
  }
  // What the figure was drawn from: a list that has grown, moved or changed
  // size, or a listing of another width, is drawn again.
  const sig = `${width}:${list.child_count}:${list.offset_bits}:${list.size_bits}:${data.records.length}:${data.kinds.join("\u0000").length}`;
  return cached(c.doc, key, sig, () => drawRecordTiles(c.doc, data, { width, word: recordWord(list) }));
}

/** Open the tiles of `list` under the heading at `path`, and close the tiles
 *  of the same list under any other element's heading: one list is drawn
 *  once. */
function toggleTiles(c: DrawContext, id: string, path: readonly number[], list: TemplateNode): void {
  const key = figureKey(id, path);
  if (!c.cards.has(key)) {
    const listKey = list.path.join(".");
    for (const other of [...c.cards]) {
      if (other === key || !other.startsWith(`fig:${TILES_OF_LIST}:`)) continue;
      const at = figurePath(other);
      if (at === null || at === "") continue;
      if (at.split(".").slice(0, -1).join(".") === listKey) c.toggleCard(other);
    }
  }
  c.toggleCard(key);
}

const TILES_OF_LIST = "record-tiles";
const TILES_INSIDE = "record-tiles-inside";

/** Put the tiles on the listing's headings. */
export function register(): void {
  // On a heading that is one element of a list: the list it is in.
  registerHeadingButton({
    id: TILES_OF_LIST,
    applies: (c, node) => parentList(c.doc, node) !== null,
    draw: (c, node) => {
      const list = parentList(c.doc, node) as TemplateNode;
      const word = recordWord(list);
      const on = c.cards.has(figureKey(TILES_OF_LIST, node.path));
      return headingButton(VIZ.tiles.button(word), VIZ.tiles.buttonTitle(word), on, () => toggleTiles(c, TILES_OF_LIST, node.path, list));
    },
  });
  registerHeadingFigure({
    id: TILES_OF_LIST,
    draw: (c, node, _item, width) => {
      const list = parentList(c.doc, node);
      return list === null ? null : figureFor(c, figureKey(TILES_OF_LIST, node.path), list, width);
    },
  });
  // On a heading whose contents are a list: that list.
  registerHeadingButton({
    id: TILES_INSIDE,
    applies: (c, node) => innerList(c.doc, node) !== null,
    draw: (c, node) => {
      const list = innerList(c.doc, node) as TemplateNode;
      const word = recordWord(list);
      const on = c.cards.has(figureKey(TILES_INSIDE, node.path));
      return headingButton(VIZ.tiles.button(word), VIZ.tiles.buttonTitle(word), on, () => toggleTiles(c, TILES_INSIDE, node.path, list));
    },
  });
  registerHeadingFigure({
    id: TILES_INSIDE,
    draw: (c, node, _item, width) => {
      const list = innerList(c.doc, node);
      return list === null ? null : figureFor(c, figureKey(TILES_INSIDE, node.path), list, width);
    },
  });
}

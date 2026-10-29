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
// Records in a row of the same type and size are one tile, with a cut corner
// and the count on it (`collapseRuns`): a run of three hundred equal IDAT
// chunks is one thing to the eye, and drawing it once leaves the room to the
// records that differ. A record with a wrong value is never folded into a run.
//
// The records are read a slice at a time with the page given its turn in
// between (`readahead.ts`); the heading says the list is being read until the
// tiles are drawn, and the listing then lays itself out again around them.
//
// Drawn on a canvas, one rectangle per tile, so a list of thousands costs
// one draw per frame; the arithmetic is `tilelayout.ts`.

import type { Doc, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { fieldClass, UNMAPPED_COLOR } from "../fieldstyle.ts";
import { isElementName } from "../elementname.ts";
import type { DrawContext } from "../listingdraw.ts";
import { stripIndex, typeWords, variantText } from "../report/partrules.ts";
import { bitSizeText, PROBLEMS, REPORT, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { applyX, applyY, invertX, invertY, type View } from "./boardmath.ts";
import { canvasMeasure, ellipsise, type Measure } from "./fit.ts";
import { headingButton, registerHeadingButton } from "./headingbuttons.ts";
import { figureKey, figurePath, registerHeadingFigure } from "./headingfigures.ts";
import { linkedFor, type LinkedBus } from "./linked.ts";
import { LOD } from "./lod.ts";
import { goToField } from "./pick.ts";
import { queued, recordsOf, walk, WAIT, type Wait, type Walk } from "./readahead.ts";
import { assignCategories, cellOf, collapseRuns, fillPx, OTHER_SLOT, recordWord, tileAt, tileBox, tileGrid, type Categories, type Cells } from "./tilelayout.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

/** Most records read for one grid. Past this the grid says how many it left
 *  out; reading every record of a list of a million is a different piece. */
const READ_MAX = 4096;
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
/** The count on a tile that stands for a run of records. */
const COUNT_PX = 10;
/** How much of the figure may show empty past the tiles when it is dragged
 *  or zoomed as far as it goes: the last tile can reach the middle. */
const HOLD = 0.5;

// ----- the data -----

export type TilesData = {
  readonly list: TemplateNode;
  /** The records drawn, the first `READ_MAX` of the list. */
  readonly records: readonly TemplateNode[];
  /** Each record's type, as the legend names it. */
  readonly kinds: readonly string[];
  /** What the kinds are, for the key: `type` where what decided some record's
   *  case named it (a PNG chunk's type field, a ZIP signature), `name` where
   *  every kind is only the record's own name, as a WAD lump's is. Calling
   *  3,298 lump names "types" would say the directory holds 3,298 kinds of
   *  thing. */
  readonly what: "type" | "name";
  readonly categories: Categories;
  /** The index of the largest record, which the fill is measured against. */
  readonly largest: number;
  /** Whether every record is the same size, so the fill says nothing. */
  readonly sameSize: boolean;
  /** One tile per run of records of the same type and size. */
  readonly cells: Cells;
};

/**
 * What kind of record this is, in the words the legend uses. What decided the
 * record's case first, as the report names its parts (`variantOf` in
 * `report/model.ts`): a ZIP record by its signature, a PNG chunk by its type
 * field. Then the record's own name without its place in the list (`[4]
 * IDAT` is `IDAT`), then its type. The first is what tells a ZIP's local file
 * header from its central directory entry for the same file, which share a
 * name.
 */
function kindOf(doc: Doc, n: TemplateNode, probe: boolean): { readonly text: string; readonly decided: boolean } {
  if (probe && n.composite) {
    const own = doc.origins(n.path);
    const direct = own.status === "ok" ? own.node.find((o) => o.role === "type" && o.value !== "") : undefined;
    const v = direct === undefined ? null : variantText(direct.value);
    if (v !== null) return { text: v, decided: true };
    const kids = doc.templateChildren(n.path, 0, Math.min(n.child_count, 8));
    if (kids.status === "ok") {
      for (const k of kids.node.filter((c) => c.composite).slice(0, 2)) {
        const o = doc.origins(k.path);
        const t = o.status === "ok" ? o.node.find((x) => x.role === "type" && x.value !== "") : undefined;
        const kv = t === undefined ? null : variantText(t.value);
        if (kv !== null) return { text: kv, decided: true };
      }
    }
  }
  const bare = stripIndex(n.name);
  return bare !== "" ? { text: bare, decided: false } : { text: typeWords(n.type), decided: true };
}

/** The tiles for `list`, read a slice at a time (`readahead.ts`), or `WAIT`
 *  while some of its records have not arrived or the document changed. */
export async function readTiles(w: Walk, list: TemplateNode): Promise<TilesData | Wait> {
  const records = await recordsOf(w, list, Math.min(list.child_count, READ_MAX));
  if (records === WAIT || !w.alive()) return WAIT;
  const probe = records.length <= VARIANT_MAX;
  const kinds: string[] = [];
  let decided = false;
  for (const r of records) {
    const k = kindOf(w.doc, r, probe);
    kinds.push(k.text);
    decided ||= k.decided;
    await w.step();
    if (!w.alive()) return WAIT;
  }
  let largest = 0;
  records.forEach((r, i) => {
    if (r.size_bits > (records[largest]?.size_bits ?? 0)) largest = i;
  });
  const sameSize = records.every((r) => r.size_bits === records[0]?.size_bits);
  // A record with a wrong value inside it keeps a tile of its own, so its red
  // edge is there to be found.
  const keys = records.map((r, i) => (hasProblem(r) ? null : `${kinds[i] ?? ""}\u0000${r.size_bits}`));
  return { list, records, kinds, what: decided ? "type" : "name", categories: assignCategories(kinds), largest, sameSize, cells: collapseRuns(keys) };
}

function hasProblem(r: TemplateNode): boolean {
  const [invalid, undef] = r.problems_within;
  return invalid > 0 || undef > 0 || r.problem !== undefined;
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

export type TilesFigure = {
  readonly el: HTMLElement;
  readonly board: Board;
  /** Draw again once the figure is back on the page, for a figure the
   *  listing has just put into a new heading: the cursor may have moved while
   *  it was off the page, and its colours are only readable on the page. */
  refresh(): void;
  destroy(): void;
};

/** Draw the grid `width` pixels wide. `word` is what the list calls one
 *  record. */
export function drawRecordTiles(doc: Doc, data: TilesData, opts: { readonly width: number; readonly word: string }): TilesFigure {
  const width = Math.max(120, Math.floor(opts.width));
  const word = opts.word;
  const cells = data.cells;
  const cellCount = cells.first.length;
  const grid = tileGrid(cellCount, width - 2 * PAD, GRID_MAX_H);
  // A list that fits one row zooms across only, so its tiles grow wider at
  // their own height, with room for their fields and their type; zooming it
  // both ways would grow each tile past the top and bottom of a figure one
  // tile tall. A grid of several rows zooms both ways, like a map.
  const oneRow = grid.rows <= 1;
  const height = grid.height + 2 * PAD;
  const maxBits = data.records[data.largest]?.size_bits ?? 0;
  /** The record a tile starts with, and how many it stands for. */
  const recordOf = (cell: number): TemplateNode | undefined => data.records[cells.first[cell] ?? -1];
  const runOf = (cell: number): number => cells.count[cell] ?? 1;

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
    b.append(swatch, name);
    if (entry.name !== null) {
      name.textContent = entry.name;
      const count = document.createElement("span");
      count.className = "viz-tiles-count";
      count.textContent = entry.count.toLocaleString();
      b.append(count);
    } else {
      // Two counts, so each says what it counts.
      name.textContent = VIZ.tiles.otherTypes(entry.kinds.length, entry.count, word, data.what);
    }
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
    // When every record is one size, every tile is full and the fill carries
    // nothing; the key says the size once instead of naming a largest.
    key.textContent = data.sameSize
      ? VIZ.tiles.keySameSize(word, data.what, bitSizeText(maxBits))
      : VIZ.tiles.key(word, data.what, data.kinds[data.largest] ?? largest.name, bitSizeText(maxBits));
    if (cellCount < data.records.length) key.textContent += ` ${VIZ.tiles.keyRuns(word, data.what)}`;
    host.append(key);
  }

  let view: View = { k: 1, x: 0, y: 0 };
  const bus: LinkedBus = linkedFor(doc);
  let hovered: number | null = null;
  const listDepth = data.list.path.length;
  /** Which tile holds the record the cursor is in, from the shared selection. */
  const selectedOf = (path: readonly number[] | undefined): number | null => {
    if (path === undefined || path.length <= listDepth) return null;
    for (let i = 0; i < listDepth; i++) if (path[i] !== data.list.path[i]) return null;
    return cellOf(cells, path[listDepth] as number);
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
    const font = getComputedStyle(document.body).fontFamily;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    // Only the rows and columns on screen: at the deepest zoom a few tiles of
    // thousands are.
    const col0 = Math.max(0, Math.floor(invertX(view, 0) / grid.pitch) - 1);
    const col1 = Math.min(grid.cols - 1, Math.ceil(invertX(view, width) / grid.pitch) + 1);
    const row0 = oneRow ? 0 : Math.max(0, Math.floor(invertY(view, 0) / grid.pitch) - 1);
    const row1 = oneRow ? 0 : Math.min(grid.rows - 1, Math.ceil(invertY(view, height) / grid.pitch) + 1);
    const labels: { x: number; y: number; w: number; text: string; px: number }[] = [];
    for (let row = row0; row <= row1; row++) {
      for (let col = col0; col <= col1; col++) {
        const i = row * grid.cols + col;
        const first = cells.first[i];
        const r = recordOf(i);
        if (r === undefined || first === undefined) continue;
        const { x, y, w, h } = screenBox(i);
        const slot = data.categories.slot[first] ?? OTHER_SLOT;
        const alpha = only === null || only === slot ? 1 : 0.15;
        // The square in a pale tint of its type, then its fill from the
        // bottom in the full colour: an empty record is still its colour.
        ctx.fillStyle = slotColor(slot, dark);
        ctx.globalAlpha = alpha * (dark ? 0.32 : 0.24);
        ctx.fillRect(x, y, w, h);
        ctx.globalAlpha = alpha;
        const fill = fillPx(r.size_bits, maxBits, grid.cell) * (h / grid.cell);
        ctx.fillRect(x, y + h - fill, w, fill);
        const strip = w >= STRIP_FROM_PX ? drawFields(ctx, first, x, y, w, h, colours) : 0;
        // What of the tile is on screen. Its type and its count are written
        // inside that part and only where they fit whole, so a tile half off
        // the edge never shows half a word.
        const vx0 = Math.max(x, 0) + 4;
        const vx1 = Math.min(x + w, width) - 4;
        const vy1 = Math.min(y + h, height);
        const run = runOf(i);
        const count = run > 1 ? `×${run.toLocaleString()}` : "";
        const countW = count === "" ? 0 : measurer()(count, COUNT_PX);
        // The count goes in the bottom corner where the tile is tall enough
        // to hold it under the type, and on the type's line, after it, where
        // it is not: a one-row grid zoomed in is wide and short.
        const ty = y + strip + 3;
        const countBelowY = vy1 - COUNT_PX - 3;
        const countBelow = count !== "" && countBelowY >= ty + NAME_PX + 3;
        let countDone = false;
        if (w >= NAME_FROM_PX && h >= NAME_TALL_PX && vx1 - vx0 >= 12 && ty >= 0 && ty + NAME_PX + 2 <= vy1) {
          const side = count !== "" && !countBelow ? countW + 6 : 0;
          const text = ellipsise(data.kinds[first] ?? "", vx1 - vx0 - side, NAME_PX, measurer(), 3);
          if (text !== null) labels.push({ x: vx0, y: ty, w: vx1 - vx0 - side, text, px: NAME_PX });
          if (text !== null && side > 0) {
            labels.push({ x: vx1 - countW, y: ty + 1, w: countW, text: count, px: COUNT_PX });
            countDone = true;
          }
        }
        if (run > 1) {
          // The repeat glyph: a corner cut in the text colour on every tile
          // that stands for several records, and their count on it when the
          // count fits.
          const m = Math.min(7, Math.floor(Math.min(w, h) / 3));
          if (m >= 2) {
            ctx.globalAlpha = alpha;
            ctx.fillStyle = colours.fg;
            ctx.beginPath();
            ctx.moveTo(x + w - m, y);
            ctx.lineTo(x + w, y);
            ctx.lineTo(x + w, y + m);
            ctx.closePath();
            ctx.fill();
          }
          const top = Math.max(y, 0) + strip + 1;
          if (!countDone && vx0 + countW <= vx1 && countBelowY >= top) labels.push({ x: vx0, y: countBelowY, w: countW, text: count, px: COUNT_PX });
        }
        if (hasProblem(r) && (r.problems_within[0] > 0 || r.problem?.tier === "invalid")) {
          ctx.globalAlpha = 1;
          ctx.strokeStyle = colours.warn;
          ctx.lineWidth = 2;
          ctx.strokeRect(x + 1, y + 1, w - 2, h - 2);
        }
      }
    }
    ctx.globalAlpha = 1;
    // Labels on a plate of the background, so a dark fill under one does not
    // take it. Each only where it fits inside its own tile, so two never meet.
    if (labels.length > 0) {
      ctx.textBaseline = "top";
      ctx.lineJoin = "round";
      for (const n of labels) {
        ctx.font = `${n.px}px ${font}`;
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
    // The tiles, not the canvas: a row of six chunks covers the first two
    // hundred pixels of a figure as wide as the listing, and the limits
    // keep some of them on screen however far it is dragged or zoomed.
    content: { x0: 0, y0: 0, x1: grid.width + 2 * PAD, y1: height },
    hold: HOLD,
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

  /** The tile under a pointer event, by grid arithmetic. */
  const cellAt = (e: MouseEvent): number | null => {
    const r = canvas.getBoundingClientRect();
    const x = invertX(view, e.clientX - r.left - PAD);
    const y = oneRow ? e.clientY - r.top - PAD : invertY(view, e.clientY - r.top - PAD);
    return tileAt(grid, x, y, LOD.targetPx / view.k);
  };

  const stopTip = hoverTip(canvas, (e) => {
    const i = cellAt(e);
    if (i !== hovered) {
      hovered = i;
      draw();
    }
    const first = i === null ? undefined : cells.first[i];
    const r = first === undefined ? undefined : data.records[first];
    if (r === undefined || i === null || first === undefined) {
      bus.publish({ hover: null });
      return null;
    }
    const run = runOf(i);
    const last = data.records[first + run - 1] ?? r;
    const endBit = last.space === r.space ? last.offset_bits + last.size_bits : r.offset_bits + r.size_bits;
    bus.publish({ hover: { startBit: r.offset_bits, endBit } });
    if (run > 1) {
      const kind = data.kinds[first] ?? "";
      const lines = [data.what === "type" ? VIZ.tiles.typeLine(kind) : VIZ.tiles.nameLine(kind), VIZ.tiles.runLine(run, word, bitSizeText(r.size_bits))];
      return tipCard(VIZ.tiles.runTitle(word, first, first + run - 1), lines, `${formatOffset(r.offset_bits)} · ${bitSizeText(endBit - r.offset_bits)}`);
    }
    return tipCard(r.name, tipLines(data, first), `${formatOffset(r.offset_bits)} · ${bitSizeText(r.size_bits)}`);
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
    const i = cellAt(e);
    const r = i === null ? undefined : recordOf(i);
    if (r === undefined) return;
    // A tile that stands for a run goes to the first record of it.
    goToField(doc, { path: r.path, startBit: r.offset_bits, endBit: r.offset_bits + r.size_bits });
  });
  // The tile the cursor is in is outlined, whichever view moved the cursor.
  // Subscribed for as long as the figure lives rather than while it is on the
  // page: the listing takes the heading off the page and puts the same
  // figure back in the next one, and a figure that let go the first time it
  // was off the page would stop following the cursor.
  const stopLinked = bus.subscribe((state, changed) => {
    if (changed !== "selection") return;
    const next = selectedOf(state.selection?.path);
    if (next === selected) return;
    selected = next;
    draw();
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
    refresh: () => requestAnimationFrame(() => {
      if (host.isConnected) draw();
    }),
    destroy: () => {
      zoom.destroy();
      stopTip();
      hideTip();
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
  const problems = PROBLEMS.overview(invalid, undef);
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

/** What was read and drawn for one document: the tiles of each list, read
 *  once, and the figures drawn from them, by figure key, so a heading built
 *  again on the next paint gets the same figure, zoom and all. The tiles go
 *  when the document changes: bytes arriving can finish a list that was cut
 *  short, and an edit can change any record's type or size. A figure drawn
 *  from tiles that have gone is drawn again the next time it is asked for. */
type Memo = {
  readonly data: Map<string, TilesData>;
  /** The lists being read now, so a heading painted again while its list is
   *  read does not start a second read. */
  readonly reading: Set<string>;
  readonly figures: Map<string, { readonly data: TilesData; readonly width: number; readonly fig: TilesFigure }>;
};
const memo = new WeakMap<Doc, Memo>();
/** Figures kept per document. A reader with more open than this has scrolled
 *  some far away, and those are drawn again when they come back. */
const KEEP = 8;

function memoFor(doc: Doc): Memo {
  let m = memo.get(doc);
  if (m === undefined) {
    const fresh: Memo = { data: new Map(), reading: new Set(), figures: new Map() };
    m = fresh;
    memo.set(doc, fresh);
    doc.onChange(() => {
      fresh.data.clear();
      fresh.reading.clear();
    });
  }
  return m;
}

/** Read the tiles of `list` a slice at a time, and lay the listing out again
 *  once they are there, so the heading is built again with its figure. */
function startReading(c: DrawContext, m: Memo, list: TemplateNode, listKey: string): void {
  if (m.reading.has(listKey)) return;
  m.reading.add(listKey);
  const w = walk(c.doc);
  const done = c.figuresChanged;
  void queued(() => readTiles(w, list)).then((got) => {
    if (!w.alive()) return;
    m.reading.delete(listKey);
    if (got === WAIT) return;
    m.data.set(listKey, got);
    done?.();
  });
}

function figureFor(c: DrawContext, key: string, list: TemplateNode, width: number): HTMLElement {
  const m = memoFor(c.doc);
  const listKey = list.path.join(".");
  const data = m.data.get(listKey);
  if (data === undefined) {
    startReading(c, m, list, listKey);
    const wait = document.createElement("div");
    wait.className = "viz-tiles-note";
    wait.textContent = REPORT.reading;
    return wait;
  }
  const had = m.figures.get(key);
  if (had !== undefined && had.data === data && had.width === width) {
    // Most recently used last, so the oldest is the one let go.
    m.figures.delete(key);
    m.figures.set(key, had);
    had.fig.refresh();
    return had.fig.el;
  }
  had?.fig.destroy();
  const fig = drawRecordTiles(c.doc, data, { width, word: recordWord(list) });
  m.figures.set(key, { data, width, fig });
  while (m.figures.size > KEEP) {
    const oldest = m.figures.keys().next().value as string;
    m.figures.get(oldest)?.fig.destroy();
    m.figures.delete(oldest);
  }
  return fig.el;
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
  // On a heading that is one element of a list: the list it is in. Only on a
  // top-level heading, which is where the listing puts the elements of a list
  // it has not given a heading of its own; under a list's heading, the list's
  // heading has the button.
  registerHeadingButton({
    id: TILES_OF_LIST,
    applies: (c, node, item) => item.level === 0 && parentList(c.doc, node) !== null,
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

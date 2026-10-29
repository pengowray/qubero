// Bit ruler, RFC-style header diagram and register map.
//
// Candidate `bit-ruler-register` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent A of the first batch (section 5.3). An inspector section,
// and a row glyph in the listing (`rowglyphs.ts`).
//
// The hex view cannot say which bits of a byte belong to which field. This
// draws a word's bits in rows, as an RFC draws a header: a bit-number ruler
// on top, the byte's address down the left, one cell per bit with its digit
// in it, and each field boxed under the bits it takes, its name inside the
// box where it fits and a number keyed to a legend where it does not. Two
// kinds of word:
// - A flags field: one box per named bit, set bits filled, and set bits the
//   format has no name for hatched, since those are what a reader is looking
//   for. The ruler numbers the bits of the value (bit 3 is the one worth 8),
//   while the cells stay in file order, so a little-endian word reads
//   `7 … 0 15 … 8` and the byte order is there to see.
// - A field smaller than a byte, or not on a byte boundary: the bytes it
//   touches and every field in them, such as a deflate block's BFINAL and
//   BTYPE and the start of its first code.
//
// Inside each byte the bits are drawn high bit first, as the hex view shows
// them, or low bit first, the order a stream packed from the low end reads
// them, where a field crossing into the next byte becomes one unbroken box.
// The second is the default for a field read low bit first. A switch picks 8,
// 16 or 32 bits a row; by default it is the widest that fits the panel at a
// readable size, and a 32-bit word in a narrow panel wraps into two rows of 16
// rather than shrinking its digits.
//
// When the flags field is one of a list's elements (the general-purpose flags
// of every ZIP record, the characteristics of every PE section), the same bits
// of every element are stacked as rows under one ruler: a bit set in every
// row reads as a stripe, and the one record that differs stands out.
//
// Explored by pointing and clicking: a cell or a box gives its name, value,
// bit range, mask and the shift-and-mask that reads it (`bitFormula`), and
// lights those bits in the hex view (`linked.ts`). A click goes to the bit,
// or to the field; a record's row goes to that record's field. No zoom: the
// grid is a fixed pitch and wraps to the panel, so the wheel scrolls the
// panel as usual.
//
// The cell arithmetic is in `bitlayout.ts`, with no DOM, and is tested there.

import "./bits.css";
import type { Doc, FieldPick, FlagsInfo, TemplateNode } from "../doc.ts";
import { bitFormula, bitFormulaLow } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { bitSizeText, countText, VIZ } from "../strings.ts";
import { bytesOf, fieldValueBits, rowBitsFor, rulerCells, runsOf, valueBit, wordEndian, type CellOrder, type RulerCell } from "./bitlayout.ts";
import { canvasMeasure, ellipsise, monoMeasure, type Measure } from "./fit.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { linkedFor, subscribeWhileConnected, type BitRange } from "./linked.ts";
import { goToBits, goToField } from "./pick.ts";
import { registerRowGlyph } from "./rowglyphs.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

const T = VIZ.bitRuler;

/** The widest word drawn: a 64-bit flags field, or the bytes around a field. */
const MAX_BYTES = 8;
/** The most records stacked as rows. Past this the rows are a table to read
 *  in the listing, not a picture. */
const MAX_RECORDS = 100;

/** One field of the word: a box under its bits. */
export type RulerField = {
  readonly name: string;
  /** The bits it takes, relative to the word: bit `k` is bit `k % 8` from the
   *  top of byte `k / 8` of the word, as the hex view numbers them. */
  readonly bits: ReadonlySet<number>;
  /** What the hover card says under the name. */
  readonly lines: readonly string[];
  /** The shift and mask that reads it, for the hover card. */
  readonly formula: string;
  /** What the legend says beside its key. */
  readonly legend: string;
  /** A set flag, a clear one, or a field that is not a flag. */
  readonly set: boolean | null;
  /** Where a click on its box goes. */
  readonly pick: FieldPick;
};

/** One element of a list whose flags are stacked as rows. */
export type RulerRecord = {
  readonly name: string;
  readonly node: TemplateNode;
  readonly bytes: Uint8Array;
};

export type RulerData = {
  readonly title: string;
  readonly firstByte: number;
  readonly byteCount: number;
  readonly bytes: Uint8Array;
  readonly fields: readonly RulerField[];
  /** What the ruler over a cell says: the bit of the value for a flags word,
   *  the bit of the byte for anything else. */
  readonly number: (byteIndex: number, low: number) => number;
  /** For a flags word, the value bit a word bit is, and whether the format
   *  names it. Null for a word of fields. */
  readonly flags: { readonly valueBitOf: (wordBit: number) => number; readonly named: (valueBit: number) => string | null } | null;
  /** The order the bits are drawn in unless the reader picks one. */
  readonly order: CellOrder;
  /** The field under the cursor. */
  readonly node: TemplateNode;
  /** Stacked rows, one per element, or null for one word. */
  readonly records: readonly RulerRecord[] | null;
  readonly recordTotal: number;
};

/** A field in the word's own bit numbering. */
function wordBits(firstByte: number, offsetBits: number, sizeBits: number, lsb: boolean): Set<number> {
  const out = new Set<number>();
  for (const b of fieldValueBits(offsetBits, sizeBits, lsb)) out.add(b - firstByte * 8);
  return out;
}

/** The flags field at the cursor as a ruler, or null while its bytes are on
 *  the way and for a flags field whose bytes do not give its value. */
function flagsData(doc: Doc, n: TemplateNode, path: readonly number[]): RulerData | null {
  const info = doc.typeInfo(path);
  if (info.status !== "ok" || info.node.kind !== "flags") return null;
  const flags: FlagsInfo = info.node;
  const aligned = n.offset_bits % 8 === 0 && n.size_bits % 8 === 0;
  const [firstByte, byteCount] = bytesOf(fieldValueBits(n.offset_bits, n.size_bits, n.lsb_first));
  if (byteCount === 0 || byteCount > MAX_BYTES) return null;
  const read = doc.read(firstByte, byteCount);
  if (!read.complete) return null;
  // Where each bit of the value is in the word. A whole-byte word is read in
  // one byte order or the other, which its bytes and its value settle; a word
  // that is not whole bytes is its bits in order.
  let place: (valueBit: number) => number;
  if (aligned) {
    const endian = wordEndian(read.bytes, flags.current);
    if (endian === null) return null;
    place = (v) => {
      const byte = endian === "le" ? Math.floor(v / 8) : byteCount - 1 - Math.floor(v / 8);
      return byte * 8 + (7 - (v % 8));
    };
  } else {
    const bits = fieldValueBits(n.offset_bits, n.size_bits, n.lsb_first);
    place = (v) => (bits[v] ?? 0) - firstByte * 8;
  }
  const valueOf = new Map<number, number>();
  for (let v = 0; v < n.size_bits; v++) valueOf.set(place(v), v);
  const names = new Map(flags.bits.map((b) => [b.bit, b.name]));
  const fields: RulerField[] = [];
  for (const b of flags.bits) {
    if (b.name === null) continue;
    const at = place(b.bit);
    const fileBit = firstByte * 8 + at;
    fields.push({
      name: b.name,
      bits: new Set([at]),
      lines: [b.set ? T.set : T.clear, T.bitRange(b.bit, b.bit), T.mask(`0x${(2 ** b.bit).toString(16).padStart(Math.ceil(n.size_bits / 4), "0")}`)],
      formula: bitFormula(fileBit, 1),
      legend: b.name,
      set: b.set,
      pick: { path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits },
    });
  }
  const records = aligned ? recordsOf(doc, n, path, byteCount) : null;
  return {
    title: records === null ? T.titleFlags(n.name) : T.titleRecords(n.name, records.count),
    firstByte,
    byteCount,
    bytes: read.bytes,
    fields,
    number: (byteIndex, low) => valueOf.get(byteIndex * 8 + (7 - low)) ?? 0,
    flags: { valueBitOf: (k) => valueOf.get(k) ?? 0, named: (v) => names.get(v) ?? null },
    order: n.lsb_first ? "low-first" : "high-first",
    node: n,
    records: records?.list ?? null,
    recordTotal: records?.total ?? 0,
  };
}

/**
 * The same field in every element of the nearest list above it, by the same
 * steps down from the element and of the same type: the general-purpose flags
 * of every ZIP local header, the characteristics of every PE section. Null
 * when fewer than two elements have one, and while any of their bytes are on
 * the way (the figure is drawn again when they land).
 */
function recordsOf(doc: Doc, n: TemplateNode, path: readonly number[], byteCount: number): { list: RulerRecord[]; total: number; count: string } | null {
  for (let depth = path.length - 2; depth >= 0; depth--) {
    const listPath = path.slice(0, depth);
    const list = doc.templateNode(listPath);
    if (list.status !== "ok" || !list.node.list) continue;
    const rest = path.slice(depth + 1);
    const kids = doc.templateChildren(listPath, 0, Math.min(list.node.child_count, 4 * MAX_RECORDS));
    if (kids.status !== "ok") return null;
    const out: RulerRecord[] = [];
    for (const kid of kids.node) {
      if (out.length >= MAX_RECORDS) break;
      const r = doc.templateNode([...kid.path, ...rest]);
      if (r.status !== "ok" || r.node.type !== n.type || r.node.size_bits !== n.size_bits || r.node.space !== n.space) continue;
      const bytes = doc.read(r.node.offset_bits / 8, byteCount);
      if (!bytes.complete) return null;
      out.push({ name: kid.name, node: r.node, bytes: bytes.bytes });
    }
    if (out.length < 2) return null;
    return { list: out, total: out.length, count: recordCount(list.node, out.length) };
  }
  return null;
}

/** How many elements are stacked, in the list's own word for them: its unit
 *  (`2 records`), or failing that its name, which is already plural
 *  (`10 sections`). */
function recordCount(list: TemplateNode, n: number): string {
  return list.unit !== undefined ? countText(n, list.unit) : `${n.toLocaleString()} ${list.name}`;
}

/** The bytes a sub-byte field touches and every field in them, or null while
 *  they are on the way. */
function bytesData(doc: Doc, n: TemplateNode): RulerData | null {
  const [firstByte, byteCount] = bytesOf(fieldValueBits(n.offset_bits, n.size_bits, n.lsb_first));
  if (byteCount === 0 || byteCount > MAX_BYTES) return null;
  const read = doc.read(firstByte, byteCount);
  if (!read.complete) return null;
  const leaves = leavesIn(doc, n, firstByte, byteCount);
  if (leaves === null) return null;
  const fields: RulerField[] = [];
  for (const { node: s, bits } of leaves) {
    const value = s.value;
    fields.push({
      name: s.name,
      bits,
      lines: [value, `${s.type} · ${bitSizeText(s.size_bits)}`],
      formula: (s.lsb_first ? bitFormulaLow : bitFormula)(s.offset_bits, s.size_bits),
      legend: value === "" ? s.name : `${s.name} = ${value}`,
      set: null,
      pick: { path: s.path, startBit: s.offset_bits, endBit: s.offset_bits + s.size_bits },
    });
  }
  const from = formatOffset(firstByte * 8);
  const to = formatOffset((firstByte + byteCount - 1) * 8);
  return {
    title: T.titleBytes(from, to),
    firstByte,
    byteCount,
    bytes: read.bytes,
    fields,
    number: (_i, low) => low,
    flags: null,
    order: n.lsb_first ? "low-first" : "high-first",
    node: n,
    records: null,
    recordTotal: 0,
  };
}

/** The most fields looked at under one structure, and the deepest a word's
 *  fields are looked for below the field's parent. */
const MAX_LEAVES = 64;
const MAX_DEPTH = 3;

/**
 * The fields with bits in the word, from the structure the field is in: its
 * siblings, and inside a sibling structure that runs into the word (a deflate
 * block's codes after its header bits), the fields of that structure that do.
 * Null while any of them are being read.
 */
function leavesIn(doc: Doc, n: TemplateNode, firstByte: number, byteCount: number): { node: TemplateNode; bits: Set<number> }[] | null {
  const out: { node: TemplateNode; bits: Set<number> }[] = [];
  const lo = firstByte * 8;
  const hi = (firstByte + byteCount) * 8;
  const inWord = (k: TemplateNode): Set<number> => {
    const bits = new Set<number>();
    for (const b of wordBits(firstByte, k.offset_bits, k.size_bits, k.lsb_first)) if (b >= 0 && b < byteCount * 8) bits.add(b);
    return bits;
  };
  // The bytes a node may touch, wide enough for either way of counting bits.
  const near = (k: TemplateNode): boolean => k.offset_bits < hi + 8 && k.offset_bits + k.size_bits > lo - 8;
  // Siblings are looked at around the field itself, since a word of at most
  // eight bytes holds only its neighbours; inside a structure that runs into
  // the word, only its first fields can reach it.
  const walk = (path: readonly number[], from: number, count: number, depth: number): boolean => {
    const reply = doc.templateChildren(path, from, from + Math.min(count, MAX_LEAVES));
    if (reply.status !== "ok") return false;
    for (const k of reply.node) {
      if (k.size_bits === 0 || k.space !== n.space || !near(k)) continue;
      if (k.composite && k.child_count > 0 && depth < MAX_DEPTH) {
        if (!walk(k.path, 0, k.child_count, depth + 1)) return false;
        continue;
      }
      const bits = inWord(k);
      if (bits.size > 0) out.push({ node: k, bits });
    }
    return true;
  };
  if (n.path.length === 0) return [{ node: n, bits: inWord(n) }];
  const parent = doc.templateNode(n.path.slice(0, -1));
  if (parent.status !== "ok") return null;
  const at = n.path[n.path.length - 1] ?? 0;
  const from = Math.max(0, at - MAX_LEAVES / 2);
  return walk(parent.node.path, from, Math.min(parent.node.child_count - from, MAX_LEAVES), 0) ? out : null;
}

/** What the ruler draws for the field at the cursor, or null. */
export function bitRulerData(doc: Doc, n: TemplateNode, path: readonly number[]): RulerData | null {
  if (n.composite || n.space !== 0 || n.size_bits === 0) return null;
  if (n.kind === "flags") return flagsData(doc, n, path);
  return bytesData(doc, n);
}

/** Whether a field is drawn by the ruler: a flags field of up to 64 bits, or
 *  any value smaller than a byte or off a byte boundary. A deflate code is
 *  the bit-order ribbon's (`bitpack.ts`) and not drawn here. */
function rulerApplies(n: TemplateNode): boolean {
  if (n.composite || n.space !== 0 || n.size_bits === 0 || n.kind === "str") return false;
  if (n.kind === "flags") return n.size_bits <= 64;
  return (n.offset_bits % 8 !== 0 || n.size_bits % 8 !== 0) && n.size_bits <= 32;
}

// ----- drawing -----

const SVG = "http://www.w3.org/2000/svg";
function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

/** Heights of a block's parts: the ruler's numbers, the lane of boxes, one
 *  row of cells (a stacked record's row is shorter), and the gap between
 *  blocks when the word wraps. */
const NUM_H = 13;
const BOX_H = 18;
const CELL_H = 18;
const REC_H = 15;
const BLOCK_GAP = 10;
/** Extra room between two bytes, so a byte reads as one. */
const BYTE_GAP = 3;
/** The narrowest and widest a cell is drawn. 13 holds a two-digit bit number
 *  at the ruler's size; past 22 the grid is only getting sparser. */
const MIN_CELL = 13;
const MAX_CELL = 22;
const NAME_PX = 11;
const NAME_PAD = 3;
const GUTTER_PAD = 6;
/** Widest a record's name is drawn down the left before it is cut short. */
const MAX_REC_NAME = 96;

let measure: Measure | null = null;
function measurer(): Measure {
  measure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return measure;
}
const MONO = monoMeasure();

/** What the reader picked, kept across fields: the order of the bits in each
 *  byte, the bits per row, and whether the clear flags are listed. */
let orderChoice: CellOrder | null = null;
let rowChoice: 8 | 16 | 32 | null = null;
let showClear = false;

export type BitRuler = { readonly el: HTMLElement; destroy(): void };

/** Draw the ruler `width` pixels wide. */
export function drawBitRuler(doc: Doc, data: RulerData, opts: { readonly width: number }): BitRuler {
  const fig = document.createElement("figure");
  fig.className = "vb-fig vb-ruler";
  const head = document.createElement("figcaption");
  head.className = "vb-head";
  const title = document.createElement("span");
  title.className = "vb-title";
  title.textContent = data.title;
  head.append(title);
  const controls = document.createElement("div");
  controls.className = "vb-controls";
  const scroller = document.createElement("div");
  scroller.style.overflowX = "auto";
  const legendEl = document.createElement("ol");
  legendEl.className = "br-legend";
  const more = document.createElement("button");
  more.type = "button";
  more.className = "vb-toggle";
  fig.append(head, controls, scroller, legendEl);

  const bus = linkedFor(doc);
  let selected: readonly number[] | null = data.node.path;
  const cleanups: (() => void)[] = [];

  /** A segmented control: one button per choice, the current one pressed. */
  const segment = <V,>(label: string, choices: readonly { value: V; text: string; title?: string }[], current: V, pick: (v: V) => void): HTMLElement => {
    const box = document.createElement("span");
    box.className = "vb-seg";
    box.setAttribute("role", "group");
    box.setAttribute("aria-label", label);
    box.append(`${label}:`);
    for (const c of choices) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = c.text;
      if (c.title !== undefined) b.title = c.title;
      b.setAttribute("aria-pressed", String(c.value === current));
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        pick(c.value);
      });
      box.append(b);
    }
    return box;
  };

  const render = (): void => {
    for (const c of cleanups.splice(0)) c();
    const order = orderChoice ?? data.order;
    const records = data.records;
    // The gutter down the left: the address of each row's first byte, or the
    // name of each record.
    const addrW = MONO(formatOffset((data.firstByte + data.byteCount) * 8), 10);
    const recW = records === null ? 0 : Math.min(MAX_REC_NAME, Math.max(...records.map((r) => measurer()(r.name, NAME_PX))));
    const gutter = Math.ceil(Math.max(addrW, recW)) + GUTTER_PAD;
    const fits = rowBitsFor(data.byteCount, opts.width - gutter, MIN_CELL, BYTE_GAP);
    const most = Math.min(32, data.byteCount * 8);
    const perRow = Math.min(most, rowChoice ?? fits) as 8 | 16 | 32;
    const bytesPerRow = perRow / 8;
    const room = opts.width - gutter - (bytesPerRow - 1) * BYTE_GAP;
    const pitch = Math.max(MIN_CELL, Math.min(MAX_CELL, Math.floor(room / perRow)));
    const cells = rulerCells(0, data.byteCount, perRow, order);
    const rows = Math.ceil(cells.length / perRow);
    const colX = (col: number): number => gutter + col * pitch + Math.floor(col / 8) * BYTE_GAP;
    const gridW = colX(perRow - 1) + pitch;
    const linesPerBlock = records === null ? 1 : records.length;
    const lineH = records === null ? CELL_H : REC_H;
    const blockH = NUM_H + BOX_H + 2 + linesPerBlock * lineH;
    const height = rows * blockH + (rows - 1) * BLOCK_GAP + 2;

    controls.replaceChildren(
      segment(T.order, [
        { value: "high-first" as CellOrder, text: T.highFirst, title: T.highFirstTitle },
        { value: "low-first" as CellOrder, text: T.lowFirst, title: T.lowFirstTitle },
      ], order, (v) => {
        orderChoice = v;
        render();
      }),
      ...(data.byteCount > 1
        ? [segment(T.perRow, ([8, 16, 32] as const).filter((w) => w <= most).map((w) => ({ value: w, text: String(w) })), perRow, (v) => {
            rowChoice = v;
            render();
          })]
        : []),
    );

    const root = svg("svg", { width: gridW, height, class: "vb-svg br-svg", role: "img", "aria-label": data.title });
    const defs = svg("defs");
    const hatch = svg("pattern", { id: "br-hatch", width: 5, height: 5, patternUnits: "userSpaceOnUse", patternTransform: "rotate(45)" });
    hatch.append(svg("rect", { width: 5, height: 5, class: "br-cell is-set" }), svg("line", { x1: 0, y1: 0, x2: 0, y2: 5, class: "br-hatch-line" }));
    defs.append(hatch);
    root.append(defs);
    const blockTop = (row: number): number => row * (blockH + BLOCK_GAP);

    // Keys for the fields whose names do not fit their own boxes, numbered
    // in the order the boxes are drawn so the numbers run left to right.
    const runsByField = data.fields.map((f) => runsOf(cells, (b) => f.bits.has(b)));
    const order1st = data.fields.map((_, i) => i).sort((a, b) => {
      const ra = runsByField[a]?.[0];
      const rb = runsByField[b]?.[0];
      return (ra?.row ?? 0) - (rb?.row ?? 0) || (ra?.col0 ?? 0) - (rb?.col0 ?? 0);
    });
    const keyOf = new Map<number, number>();
    const inBox = new Map<number, { run: number; text: string }>();
    // A flag is one bit, and the ruler over it already says which bit of the
    // value it is: that number is its key, and the panel's own Flags list and
    // the legend both name it by that number. Its box carries no text.
    const legendKey = new Map<number, number>();
    for (const i of order1st) {
      const runs = runsByField[i] ?? [];
      const f = data.fields[i];
      if (f === undefined) continue;
      if (data.flags !== null) {
        legendKey.set(i, data.flags.valueBitOf([...f.bits][0] ?? 0));
        continue;
      }
      // The name goes in the widest of the field's boxes, if it fits there.
      let best = -1;
      let bestW = 0;
      runs.forEach((r, j) => {
        const w = colX(r.col1 - 1) + pitch - colX(r.col0);
        if (w > bestW) {
          bestW = w;
          best = j;
        }
      });
      const text = ellipsise(f.name, bestW - 2 * NAME_PAD, NAME_PX, measurer(), 4);
      if (text !== null && best >= 0) inBox.set(i, { run: best, text });
      else {
        keyOf.set(i, keyOf.size + 1);
        legendKey.set(i, keyOf.size);
      }
    }

    const boxEls: { field: number; el: SVGRectElement }[] = [];
    const cellEls = new Map<string, SVGRectElement>();
    for (let row = 0; row < rows; row++) {
      const top = blockTop(row);
      const g = svg("g");
      // The ruler: the bit number over each cell.
      for (const c of cells) {
        if (c.row !== row) continue;
        const t = svg("text", { x: colX(c.col) + pitch / 2, y: top + NUM_H - 3, class: "br-num", "text-anchor": "middle" });
        t.textContent = String(data.number(Math.floor((c.bit) / 8), c.low));
        g.append(t);
      }
      // The boxes.
      data.fields.forEach((f, i) => {
        const runs = runsByField[i] ?? [];
        runs.forEach((r, j) => {
          if (r.row !== row) return;
          const x = colX(r.col0) + 0.5;
          const w = colX(r.col1 - 1) + pitch - colX(r.col0) - 1;
          const rect = svg("rect", { x, y: top + NUM_H + 0.5, width: Math.max(1, w), height: BOX_H - 1, rx: 2, class: "br-box" });
          rect.dataset.field = String(i);
          g.append(rect);
          boxEls.push({ field: i, el: rect });
          const named = inBox.get(i);
          const label = named !== undefined && named.run === j ? named.text : keyOf.has(i) && j === 0 ? String(keyOf.get(i)) : null;
          if (label !== null) {
            const t = svg("text", { x: named !== undefined ? x + NAME_PAD : x + w / 2, y: top + NUM_H + BOX_H / 2 + 0.5, class: named !== undefined ? "br-name" : "br-key", "dominant-baseline": "central", "text-anchor": named !== undefined ? "start" : "middle" });
            t.textContent = label;
            g.append(t);
          }
        });
      });
      // The cells: one row for the word, or one per record.
      const cellTop = top + NUM_H + BOX_H + 2;
      for (let line = 0; line < linesPerBlock; line++) {
        const y = cellTop + line * lineH;
        const rec = records?.[line];
        const bytes = rec?.bytes ?? data.bytes;
        if (rec === undefined) {
          const first = cells.find((c) => c.row === row);
          const a = svg("text", { x: gutter - GUTTER_PAD, y: y + lineH / 2, class: "br-addr", "text-anchor": "end", "dominant-baseline": "central" });
          a.textContent = formatOffset((data.firstByte + Math.floor((first?.bit ?? 0) / 8)) * 8);
          g.append(a);
        } else {
          // Measured in the regular weight and drawn bold for the current record,
          // so a tenth is kept in hand for the bold.
          const name = ellipsise(rec.name, (gutter - GUTTER_PAD) * 0.9, NAME_PX, measurer(), 3) ?? "";
          const a = svg("text", { x: gutter - GUTTER_PAD, y: y + lineH / 2, class: `br-rec${samePath(rec.node.path, data.node.path) ? " is-on" : ""}`, "text-anchor": "end", "dominant-baseline": "central" });
          a.textContent = name;
          a.dataset.record = String(line);
          g.append(a);
        }
        for (const c of cells) {
          if (c.row !== row) continue;
          const digit = ((bytes[Math.floor(c.bit / 8)] ?? 0) >> c.low) & 1;
          const unnamed = data.flags !== null && digit === 1 && data.flags.named(data.flags.valueBitOf(c.bit)) === null;
          const cls = `br-cell${digit === 1 && data.flags !== null ? " is-set" : ""}${unnamed ? " is-unnamed" : ""}`;
          const rect = svg("rect", { x: colX(c.col) + 0.5, y: y + 0.5, width: pitch - 1, height: lineH - 1, class: cls });
          rect.dataset.bit = String(c.bit);
          rect.dataset.line = String(line);
          cellEls.set(`${line}/${c.bit}`, rect);
          g.append(rect);
          if (lineH >= 14) {
            const t = svg("text", { x: colX(c.col) + pitch / 2, y: y + lineH / 2 + 0.5, class: `br-digit${digit === 0 ? " is-clear" : ""}`, "text-anchor": "middle", "dominant-baseline": "central" });
            t.textContent = String(digit);
            g.append(t);
          }
        }
      }
      root.append(g);
    }
    scroller.replaceChildren(root);

    // ----- hover, click and the linked highlight -----
    const cellOf = new Map<number, RulerCell>(cells.map((c) => [c.bit, c]));
    const fieldAt = (bit: number): number => data.fields.findIndex((f) => f.bits.has(bit));
    const lit = (field: number | null, line: number | null, bit: number | null): void => {
      for (const b of boxEls) b.el.classList.toggle("is-lit", b.field === field);
      for (const [k, el] of cellEls) {
        const [l, bt] = k.split("/").map(Number);
        const inField = field !== null && field >= 0 && data.fields[field]?.bits.has(bt ?? -1) === true;
        el.classList.toggle("is-lit", (bit !== null && bt === bit && (line === null || l === line)) || (inField && (line === null || l === line)));
      }
    };
    const fileRange = (bits: Iterable<number>, base: number): BitRange => {
      const list = [...bits];
      return { startBit: base * 8 + Math.min(...list), endBit: base * 8 + Math.max(...list) + 1 };
    };
    const recordBase = (line: number | null): number => {
      const rec = line === null ? undefined : records?.[line];
      return rec === undefined ? data.firstByte : rec.node.offset_bits / 8;
    };
    cleanups.push(
      hoverTip(root, (e) => {
        const d = (e.target as SVGElement | null)?.dataset;
        if (d === undefined) return null;
        if (d.field !== undefined) {
          const i = Number(d.field);
          const f = data.fields[i];
          if (f === undefined) return null;
          lit(i, null, null);
          bus.publish({ hover: fileRange(f.bits, data.firstByte) });
          return withFormula(tipCard(f.name, f.lines, rangeWhere(data, f.bits)), f.formula);
        }
        if (d.bit !== undefined) {
          const bit = Number(d.bit);
          const line = d.line === undefined ? null : Number(d.line);
          const c = cellOf.get(bit);
          if (c === undefined) return null;
          const base = recordBase(line);
          lit(null, line, bit);
          bus.publish({ hover: { startBit: base * 8 + bit, endBit: base * 8 + bit + 1 } });
          return cellCard(data, c, line, base);
        }
        if (d.record !== undefined) {
          const rec = records?.[Number(d.record)];
          if (rec === undefined) return null;
          lit(null, null, null);
          bus.publish({ hover: { startBit: rec.node.offset_bits, endBit: rec.node.offset_bits + rec.node.size_bits } });
          return tipCard(rec.name, [rec.node.value === "" ? T.clear : rec.node.value], formatOffset(rec.node.offset_bits));
        }
        return null;
      }),
    );
    const leave = (): void => {
      lit(null, null, null);
      bus.publish({ hover: null });
    };
    root.addEventListener("pointerleave", leave);
    cleanups.push(() => root.removeEventListener("pointerleave", leave));
    root.addEventListener("click", (e) => {
      const d = (e.target as SVGElement | null)?.dataset;
      if (d === undefined) return;
      if (d.field !== undefined) {
        const f = data.fields[Number(d.field)];
        if (f !== undefined) goToField(doc, f.pick);
        return;
      }
      if (d.bit !== undefined) {
        const base = recordBase(d.line === undefined ? null : Number(d.line));
        const bit = base * 8 + Number(d.bit);
        goToBits(doc, [{ startBit: bit, endBit: bit + 1 }]);
        return;
      }
      if (d.record !== undefined) {
        const rec = records?.[Number(d.record)];
        if (rec !== undefined) goToField(doc, { path: rec.node.path, startBit: rec.node.offset_bits, endBit: rec.node.offset_bits + rec.node.size_bits });
      }
    });
    const mark = (): void => {
      data.fields.forEach((f, i) => {
        const on = selected !== null && samePath(selected, f.pick.path) && data.flags === null;
        for (const b of boxEls) if (b.field === i) b.el.classList.toggle("is-on", on);
      });
      for (const el of root.querySelectorAll<SVGTextElement>(".br-rec")) {
        const rec = records?.[Number(el.dataset.record)];
        el.classList.toggle("is-on", rec !== undefined && selected !== null && samePath(rec.node.path, selected));
      }
    };
    mark();
    cleanups.push(
      subscribeWhileConnected(bus, fig, (state, changed) => {
        if (changed !== "selection") return;
        selected = state.selection?.path ?? null;
        mark();
      }),
    );

    drawLegend(legendKey, inBox, order1st);
  };

  /** The legend: every field whose name is not in its box, by its key; for a
   *  flags word, the set bits first and the clear ones behind a button. */
  const drawLegend = (keyOf: ReadonlyMap<number, number>, inBox: ReadonlyMap<number, unknown>, drawn: readonly number[]): void => {
    const items: HTMLElement[] = [];
    // In the order the boxes are drawn, which is the order of the keys.
    const entries = drawn.map((i) => ({ f: data.fields[i] as RulerField, i, key: keyOf.get(i) }));
    let clear = 0;
    const records = data.records;
    /** How many of the stacked records have a flag's bit set. */
    const setIn = (f: RulerField): number => {
      const k = [...f.bits][0] ?? 0;
      return (records ?? []).filter((r) => (((r.bytes[Math.floor(k / 8)] ?? 0) >> (7 - (k % 8))) & 1) === 1).length;
    };
    for (const { f, i, key } of entries) {
      if (data.flags === null && inBox.has(i) && f.legend === f.name) continue;
      // One flags word's names and states are the panel's own Flags list,
      // just above; only a stack of them has something to add.
      if (data.flags !== null && records === null) continue;
      const count = records === null ? 0 : setIn(f);
      const isClear = records === null ? f.set === false : f.set !== null && count === 0;
      if (isClear) {
        clear++;
        if (!showClear) continue;
      }
      const li = document.createElement("li");
      const k = document.createElement("span");
      k.className = "br-legend-key";
      k.textContent = key === undefined ? "" : String(key);
      const v = document.createElement("span");
      v.textContent =
        f.set === null ? f.legend
        : records !== null ? `${f.legend}: ${T.setIn(count, records.length)}`
        : `${f.legend}: ${f.set ? T.set.toLowerCase() : T.clear.toLowerCase()}`;
      if (isClear) v.className = "is-clear";
      li.append(k, v);
      items.push(li);
    }
    // A set bit the format has no name for is listed too: it is the one bit
    // a reader most needs to hear about.
    if (data.flags !== null) {
      let any = false;
      for (let b = 0; b < data.byteCount * 8; b++) {
        for (const bytes of records === null ? [data.bytes] : records.map((r) => r.bytes)) {
          const digit = ((bytes[Math.floor(b / 8)] ?? 0) >> (7 - (b % 8))) & 1;
          if (digit === 1 && data.flags.named(data.flags.valueBitOf(b)) === null) any = true;
        }
      }
      if (any) {
        const li = document.createElement("li");
        const k = document.createElement("span");
        k.className = "br-legend-key";
        const sw = document.createElementNS(SVG, "svg");
        sw.setAttribute("width", "10");
        sw.setAttribute("height", "10");
        sw.append(svg("rect", { width: 10, height: 10, class: "br-cell is-unnamed" }));
        k.append(sw);
        const v = document.createElement("span");
        v.textContent = T.unnamedSet;
        li.append(k, v);
        items.push(li);
      }
    }
    legendEl.replaceChildren(...items);
    legendEl.hidden = items.length === 0;
    if (clear > 0) {
      more.textContent = showClear ? T.hideClear(clear) : T.showClear(clear);
      more.onclick = (e) => {
        e.stopPropagation();
        showClear = !showClear;
        drawLegend(keyOf, inBox, drawn);
      };
      if (more.parentElement === null) fig.append(more);
    } else more.remove();
  };

  render();
  return {
    el: fig,
    destroy: () => {
      for (const c of cleanups.splice(0)) c();
      hideTip();
    },
  };
}

/** Where a field's bits are, for the foot of its card. */
function rangeWhere(data: RulerData, bits: ReadonlySet<number>): string {
  const first = Math.min(...bits);
  return formatOffset((data.firstByte + Math.floor(first / 8)) * 8);
}

/** The card for one cell: the flag it is (or the field it is in), whether it
 *  is set, and the mask and shift that read it. */
function cellCard(data: RulerData, c: RulerCell, line: number | null, base: number): HTMLElement {
  const rec = line === null ? undefined : data.records?.[line];
  const bytes = rec?.bytes ?? data.bytes;
  const digit = ((bytes[Math.floor(c.bit / 8)] ?? 0) >> c.low) & 1;
  const where = T.byteBit(formatOffset((base + Math.floor(c.bit / 8)) * 8), c.low);
  const formula = bitFormula(base * 8 + c.bit, 1);
  if (data.flags !== null) {
    const v = data.flags.valueBitOf(c.bit);
    const name = data.flags.named(v);
    const lines = [digit === 1 ? (name === null ? T.setUnnamed : T.set) : T.clear, T.bitRange(v, v)];
    if (rec !== undefined) lines.unshift(rec.name);
    return withFormula(tipCard(name ?? T.bit(v), lines, where), formula);
  }
  const f = data.fields.find((x) => x.bits.has(c.bit));
  return withFormula(tipCard(f?.name ?? T.bit(c.low), [`${T.bitValue(digit)}`], where), formula);
}

/** A card with the shift-and-mask expression under its lines, in the panel's
 *  own code style. */
function withFormula(card: HTMLElement, formula: string): HTMLElement {
  const code = document.createElement("code");
  code.className = "insp-formula-code";
  code.textContent = formula;
  card.insertBefore(code, card.querySelector(".viz-tip-where"));
  return card;
}

function samePath(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((s, i) => s === b[i]);
}

// ----- the row glyph -----

/** Glyphs already drawn, by field, so a scroll of the listing does not read
 *  the same bytes again on every frame. */
const GLYPHS = new WeakMap<Doc, Map<string, HTMLElement>>();

/** The listing's word-sized form: one small cell per bit of the bytes the
 *  field touches, in file order. A flags field's set bits are filled; a field
 *  smaller than a byte has its own bits marked among its neighbours'. */
function rulerGlyph(doc: Doc, n: TemplateNode): HTMLElement | null {
  const key = `${n.path.join("/")}|${n.offset_bits}|${n.size_bits}|${n.value}`;
  let cache = GLYPHS.get(doc);
  if (cache === undefined) {
    cache = new Map();
    GLYPHS.set(doc, cache);
  }
  const hit = cache.get(key);
  if (hit !== undefined) return glyphTip(hit.cloneNode(true) as HTMLElement, n);
  const bits = fieldValueBits(n.offset_bits, n.size_bits, n.lsb_first);
  const [first, count] = bytesOf(bits);
  if (count === 0 || count > 4) return null;
  const read = doc.read(first, count);
  if (!read.complete) return null;
  const mine = new Set(bits);
  const glyph = document.createElement("span");
  glyph.className = "vb-glyph";
  glyph.setAttribute("aria-hidden", "true");
  for (let k = 0; k < count * 8; k++) {
    if (k > 0 && k % 8 === 0) glyph.append(Object.assign(document.createElement("i"), { className: "is-gap" }));
    const digit = ((read.bytes[Math.floor(k / 8)] ?? 0) >> (7 - (k % 8))) & 1;
    const cell = document.createElement("i");
    if (n.kind === "flags") cell.className = digit === 1 ? "is-set" : "";
    else cell.className = mine.has(first * 8 + k) ? "is-in" : "";
    glyph.append(cell);
  }
  glyph.dataset.bytes = Array.from(read.bytes, (b) => b.toString(2).padStart(8, "0")).join(" ");
  cache.set(key, glyph);
  return glyphTip(glyph.cloneNode(true) as HTMLElement, n);
}

/** The glyph's hover card: the field's value and its bytes' bits, high bit
 *  first as the glyph draws them. */
function glyphTip(glyph: HTMLElement, n: TemplateNode): HTMLElement {
  hoverTip(glyph, () => tipCard(n.name, [n.value, T.glyphBits(glyph.dataset.bytes ?? "")], formatOffset(Math.floor(n.offset_bits / 8) * 8)));
  return glyph;
}

/** Put the ruler in the At cursor panel, and its glyph on the listing's rows,
 *  for flags fields and for fields smaller than a byte. */
export function register(): void {
  registerInspectorSection({
    id: "bit-ruler",
    applies: (_doc, n) => rulerApplies(n),
    // A word of fields is keyed by its bytes, so moving between BFINAL and
    // BTYPE keeps the figure and lights the other box. Stacked records are
    // keyed by the field's place in its element, so moving to the next
    // record's flags keeps the stack.
    key: (_doc, n, path) => (n.kind === "flags" ? `flags|${n.type}|${path.slice(0, -2).join("/")}|${path[path.length - 1]}|${n.value}` : `bytes|${Math.floor(n.offset_bits / 8)}|${n.lsb_first}`),
    draw: ({ doc, node, path, width }) => {
      const data = bitRulerData(doc, node, path);
      return data === null ? null : drawBitRuler(doc, data, { width }).el;
    },
  });
  registerRowGlyph({
    id: "bit-ruler",
    applies: (_doc, n) => rulerApplies(n) && n.size_bits <= 32,
    draw: rulerGlyph,
  });
}

// The arithmetic half of the struct layout figure (`structlayout.ts`): which
// children of a structure are its members, where the gaps between them are,
// and how members and gaps wrap onto rows a fixed number of bytes wide, the
// way pahole prints a C struct. No page behind any of it, so the tests can
// run it on made-up nodes.
//
// What counts as a member is the part that needs care. A structure's
// `size_bits` is the span of everything under it, and that includes what it
// reaches by offset: the ELF header's three `at →` fields and its `offsets →`
// list of sections make the header as big as the file, though the header
// itself is 48 bytes. So the members are the children that are sized and
// written where they stand; a child reached by an offset (`at →`, `offsets →`,
// `chain →`, `descriptors →`) is kept aside as placed elsewhere, and a child
// of no bytes (a value the template worked out, a list that came out empty)
// is not drawn at all. The record runs from its own start to the end of its
// last member, or to the end of its own bytes when nothing under it was
// placed elsewhere, so a gap the format left at the end still shows.
//
// A gap is a stretch inside the record that no member covers. A gap that ends
// where the next member reaches its natural alignment (a four-byte number at a
// multiple of four from the start of the record), and is shorter than that
// alignment, is what a compiler leaves, and is called padding; any other gap
// is bytes no field describes, called unread. A field the format itself names
// as padding (`name_padding` in cpio) stays a field, since the template read
// it, and is hatched over its colour and counted as padding.
//
// Rows: the record's bits are cut into rows of `bytesPerRow` bytes, and each
// member or gap becomes one cell per row it touches. Members that overlap (a
// union, a field read twice) go in separate lanes of the same row, from
// `lanes.ts`. Where four or more rows in a row are wholly one member, as the
// 2,048 bytes of a CD sector's data are, the rows between the first and the
// last are drawn as one short break saying how many rows it stands for, so a
// sector's anatomy fits on a screen and a reader still sees where the long
// field starts and ends.

import { assignLanes } from "./lanes.ts";
import type { Measure } from "./fit.ts";

/** The parts of a template node the layout reads. `TemplateNode` has them
 *  all; the tests make them up. */
export type NodeLike = {
  readonly path: readonly number[];
  readonly name: string;
  readonly type: string;
  readonly kind: string;
  readonly value: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly composite: boolean;
  readonly list: boolean;
  readonly inline: boolean;
  readonly child_count: number;
  readonly read_at: number | null;
};

/** A structure's children as the figure needs them: each one either a member
 *  drawn as a box, a structure inside this one whose own members are drawn
 *  in its place (`kids`), or a field reached by an offset. */
export type Gathered<N extends NodeLike = NodeLike> = {
  readonly node: N;
  /** The members of a structure drawn inside this one; null for a member
   *  drawn as one box. */
  readonly kids: readonly Gathered<N>[] | null;
};

/** The pointer types the core writes for a field read somewhere other than
 *  where it stands. See `TemplateNode.list`. */
const PLACED = /^(?:at|offsets|chain|descriptors) →/;

/** Whether a child is reached by an offset rather than written where it
 *  stands, and so is not part of the structure's own bytes. */
export function placedElsewhere(n: NodeLike): boolean {
  return n.read_at !== null || PLACED.test(n.type);
}

/** Whether a child is one of the structure's own members: written where it
 *  stands, and with bytes to draw. */
export function isMember(n: NodeLike): boolean {
  return n.size_bits > 0 && !placedElsewhere(n);
}

/** Whether a member is drawn as the members inside it rather than as one
 *  box: a structure of its own, not a list (an array is one box, named with
 *  its count) and not a value with parts (`inline`). */
export function opensInside(n: NodeLike): boolean {
  return n.composite && !n.list && !n.inline && n.child_count > 0 && !placedElsewhere(n);
}

/** One box: a member, or a stretch no member covers. Offsets are in bits
 *  from the start of the file, end exclusive. */
export type Piece<N extends NodeLike = NodeLike> =
  | {
      readonly kind: "field";
      readonly node: N;
      readonly startBit: number;
      readonly endBit: number;
      readonly lane: number;
      readonly depth: number;
      /** True for a run of bytes the format itself calls padding. */
      readonly padding: boolean;
    }
  | {
      /** Padding and unread are gaps no field in the file covers; other is a
       *  gap in this record that fields elsewhere in the file are read from,
       *  as the program headers sit among the ELF sections. */
      readonly kind: "padding" | "unread" | "other";
      readonly startBit: number;
      readonly endBit: number;
      readonly lane: 0;
      readonly next: N | null;
      /** For `other`: the parts of the file those fields belong to. */
      readonly owners: readonly string[];
    };

/** A structure inside the record, drawn as the members it holds, with a
 *  bracket over them in the key. */
export type Group<N extends NodeLike = NodeLike> = { readonly node: N; readonly startBit: number; readonly endBit: number; readonly depth: number };

export type StructRecord<N extends NodeLike = NodeLike> = {
  readonly node: N;
  /** The record's own bits, end exclusive. */
  readonly startBit: number;
  readonly endBit: number;
  /** Members and gaps, by where they start. */
  readonly pieces: readonly Piece<N>[];
  readonly groups: readonly Group<N>[];
  /** Fields reached by an offset, drawn as links under the figure. */
  readonly placed: readonly N[];
  /** Lanes a row needs: more than one only where members overlap. */
  readonly lanes: number;
};

/** A run of bytes the template calls padding: `padding`, `name_padding`,
 *  `pad2`. Only by that word, which says what the bytes are for; `reserved`
 *  and `unused` are kept for later versions of a format as often as they are
 *  filler, and are left as fields. */
const PADDING_NAME = /^(?:.*_)?pad(?:ding)?\d*$/i;
export function namedPadding(n: NodeLike): boolean {
  return (n.kind === "bytes" || n.kind === "unread") && PADDING_NAME.test(n.name);
}

/** The largest alignment worth inferring: a 16-byte number is rare, and
 *  aligning one to more than 8 is the platform's business, not the format's. */
const MAX_ALIGN = 8;

const NUMBER_KINDS = new Set(["uint", "int", "float", "enum", "flags", "unset"]);
/** A number written in binary has a type like `u32 le`, `i16` or `f64`. A
 *  number written as text (`base-16 digits` in cpio, `ascii decimal` in tar)
 *  is a string to the machine and has no alignment to want. */
const BINARY_TYPE = /^[uif]\d+\b/;

/** What `naturalAlign` reads of a member. */
export type Sized = { readonly kind: string; readonly type: string; readonly size_bits: number };

/** The alignment a number of this size would want in a C struct: its own
 *  size, for 2, 4 and 8 bytes. Null for anything else, which has no natural
 *  alignment to infer: text, bytes, structures, bit fields, numbers written
 *  as digits. An enum or a set of flags is read as its own type name, and is
 *  taken to be the binary number it nearly always is. */
export function naturalAlign(n: Sized): number | null {
  if (!NUMBER_KINDS.has(n.kind) || n.size_bits % 8 !== 0) return null;
  if ((n.kind === "uint" || n.kind === "int" || n.kind === "float") && !BINARY_TYPE.test(n.type)) return null;
  const bytes = n.size_bits / 8;
  if (bytes < 2 || bytes > MAX_ALIGN || (bytes & (bytes - 1)) !== 0) return null;
  return bytes;
}

/** The largest power of two, up to `MAX_ALIGN`, that an offset in bytes from
 *  the start of the record is a multiple of. The start itself is aligned to
 *  everything, and says so as `MAX_ALIGN`. */
export function offsetAlign(relBytes: number): number {
  if (!Number.isInteger(relBytes)) return 0;
  let a = 1;
  while (a < MAX_ALIGN && relBytes % (a * 2) === 0) a *= 2;
  return a;
}

/** Whether a member sits off its natural alignment: an eight-byte number at
 *  +0x0c, say. Only for whole-byte numbers, the only members with one. */
export function misaligned(n: Sized, relBits: number): boolean {
  const natural = naturalAlign(n);
  if (natural === null || relBits % 8 !== 0) return false;
  return (relBits / 8) % natural !== 0;
}

/**
 * The record for a structure and its gathered children: its members, the
 * structures inside it, the gaps between them, and the fields placed
 * elsewhere. How deep structures inside structures are opened is the
 * gathering's choice: a child gathered with no `kids` is one box. `cut` says
 * the children are only the first of them, and the record then ends with the
 * last one given rather than at the end of its own bytes.
 */
export function buildRecord<N extends NodeLike>(node: N, kids: readonly Gathered<N>[], cut = false): StructRecord<N> {
  const fields: { node: N; depth: number }[] = [];
  const groups: Group<N>[] = [];
  const placed: N[] = [];
  let reachedOut = false;
  const walk = (list: readonly Gathered<N>[], depth: number): void => {
    for (const g of list) {
      const n = g.node;
      if (placedElsewhere(n)) {
        // Only the record's own pointers are listed under it: one inside a
        // member structure belongs to that structure's own figure.
        if (depth === 0) placed.push(n);
        reachedOut = true;
        continue;
      }
      if (n.size_bits <= 0) continue;
      if (g.kids !== null && g.kids.length > 0) {
        const before = fields.length;
        walk(g.kids, depth + 1);
        const inner = fields.slice(before);
        if (inner.length === 0) {
          fields.push({ node: n, depth });
          continue;
        }
        // The structure's span is its members', for the same reason the
        // record's is: its own size counts what it reaches by offset.
        const start = Math.min(...inner.map((f) => f.node.offset_bits));
        const end = Math.max(...inner.map((f) => f.node.offset_bits + f.node.size_bits));
        groups.push({ node: n, startBit: start, endBit: end, depth });
        continue;
      }
      fields.push({ node: n, depth });
    }
  };
  walk(kids, 0);

  let startBit = node.offset_bits;
  let endBit = node.offset_bits + node.size_bits;
  if (fields.length > 0) {
    const first = Math.min(...fields.map((f) => f.node.offset_bits));
    const last = Math.max(...fields.map((f) => f.node.offset_bits + f.node.size_bits));
    startBit = Math.min(startBit, first);
    // A structure that reached out has a size that says so; its own bytes end
    // with its last member. One that did not has its own size to go by, and a
    // gap after its last member is part of it.
    endBit = reachedOut || cut ? last : Math.max(endBit, last);
  }

  fields.sort((a, b) => a.node.offset_bits - b.node.offset_bits || b.node.size_bits - a.node.size_bits);
  const { lane, count } = assignLanes(fields, (f) => ({ start: f.node.offset_bits, end: f.node.offset_bits + f.node.size_bits }));
  const pieces: Piece<N>[] = fields.map((f, i) => ({
    kind: "field",
    node: f.node,
    startBit: f.node.offset_bits,
    endBit: f.node.offset_bits + f.node.size_bits,
    lane: lane[i] ?? 0,
    depth: f.depth,
    padding: namedPadding(f.node),
  }));

  // The gaps, from how far the members reach so far: overlapping members
  // leave no gap between them.
  const gaps: Piece<N>[] = [];
  let reach = startBit;
  const widest = Math.max(1, ...fields.map((f) => naturalAlign(f.node) ?? 1));
  for (const f of fields) {
    const s = f.node.offset_bits;
    if (s > reach) gaps.push(gapPiece(reach, s, f.node, startBit, widest));
    reach = Math.max(reach, s + f.node.size_bits);
  }
  if (endBit > reach) gaps.push(gapPiece<N>(reach, endBit, null, startBit, widest));

  const all = [...pieces, ...gaps].sort((a, b) => a.startBit - b.startBit || a.lane - b.lane);
  groups.sort((a, b) => a.startBit - b.startBit || a.depth - b.depth);
  return { node, startBit, endBit, pieces: all, groups, placed, lanes: Math.max(1, count) };
}

/** A gap from `start` to `end`, called padding when it is what aligns the
 *  member after it (or, at the end, what rounds the record up to its widest
 *  member's alignment), and unread otherwise. */
function gapPiece<N extends NodeLike>(start: number, end: number, next: N | null, recordStart: number, widest: number): Piece<N> {
  const bits = end - start;
  const padding = (() => {
    if (bits % 8 !== 0 || (start - recordStart) % 8 !== 0) return false;
    const at = (start - recordStart) / 8;
    const size = bits / 8;
    if (next !== null) {
      const want = naturalAlign(next);
      return want !== null && size < want && (at + size) % want === 0 && at % want !== 0;
    }
    return widest > 1 && size < widest && (at + size) % widest === 0 && at % widest !== 0;
  })();
  return { kind: padding ? "padding" : "unread", startBit: start, endBit: end, lane: 0, next, owners: [] };
}

/** What is in a gap's bytes, as far as the rest of the file says: nothing
 *  (`[]`), the names of the parts whose fields are there, or null when that
 *  is not known yet. */
export type GapCover = (startBit: number, endBit: number) => readonly string[] | null;

/**
 * The record with each gap that other fields of the file are read from
 * called `other`, with the names of what those fields belong to. A gap
 * inside a record is only bytes no field describes when no field anywhere
 * does; a list placed by offsets has whole other structures between its
 * elements.
 */
export function coverGaps<N extends NodeLike>(r: StructRecord<N>, cover: GapCover): StructRecord<N> {
  let changed = false;
  const pieces = r.pieces.map((p): Piece<N> => {
    if (p.kind === "field") return p;
    const owners = cover(p.startBit, p.endBit);
    if (owners === null || owners.length === 0) return p;
    changed = true;
    return { ...p, kind: "other", owners };
  });
  return changed ? { ...r, pieces } : r;
}

/** What the figure says about a record in its summary line. */
export type Summary = {
  readonly bytes: number;
  readonly fields: number;
  readonly paddingBytes: number;
  readonly unreadBytes: number;
  /** Bytes in gaps that fields elsewhere in the file are read from. */
  readonly otherBytes: number;
  readonly gaps: number;
  readonly misaligned: number;
};

export function summarise(r: StructRecord): Summary {
  let fields = 0;
  let paddingBits = 0;
  let unreadBits = 0;
  let otherBits = 0;
  let gaps = 0;
  let off = 0;
  for (const p of r.pieces) {
    if (p.kind === "field") {
      fields++;
      if (p.padding) paddingBits += p.endBit - p.startBit;
      if (misaligned(p.node, p.startBit - r.startBit)) off++;
    } else {
      gaps++;
      if (p.kind === "padding") paddingBits += p.endBit - p.startBit;
      else if (p.kind === "other") otherBits += p.endBit - p.startBit;
      else unreadBits += p.endBit - p.startBit;
    }
  }
  return {
    bytes: (r.endBit - r.startBit) / 8,
    fields,
    paddingBytes: paddingBits / 8,
    unreadBytes: unreadBits / 8,
    otherBytes: otherBits / 8,
    gaps,
    misaligned: off,
  };
}

/** One piece's part of one row, as fractions of the row's width. */
export type Cell<N extends NodeLike = NodeLike> = {
  readonly piece: Piece<N>;
  /** Index of the piece in `StructRecord.pieces`. */
  readonly index: number;
  readonly x0: number;
  readonly x1: number;
  readonly lane: number;
};

/** A row of the figure: a row of bytes, a break standing for rows that are
 *  wholly one member, or the heading of one element of a list. `y` and `h`
 *  are in layout pixels from the top of the figure. */
export type Line<N extends NodeLike = NodeLike> =
  | { readonly kind: "row"; readonly block: number; readonly rel: number; readonly y: number; readonly h: number; readonly cells: readonly Cell<N>[] }
  | { readonly kind: "break"; readonly block: number; readonly rel: number; readonly y: number; readonly h: number; readonly rows: number; readonly piece: Piece<N>; readonly index: number }
  | { readonly kind: "title"; readonly block: number; readonly y: number; readonly h: number };

export type Rows<N extends NodeLike = NodeLike> = {
  readonly lines: readonly Line<N>[];
  readonly height: number;
};

export type RowMetrics = {
  /** Height of one lane of a row of bytes. */
  readonly laneH: number;
  /** Height of a break. */
  readonly breakH: number;
  /** Height of an element's heading, for a list drawn element by element. */
  readonly titleH: number;
  /** Space between one element and the next. */
  readonly blockGap: number;
};

/** Fewest rows wholly one member before the middle ones are drawn as a
 *  break: with three, a break would stand for one row, which is no saving. */
export const COLLAPSE_ROWS = 4;

/**
 * Cut records into rows `bytesPerRow` wide, stacked top to bottom: one record
 * for a structure, one block per element for a list (`titled`, with a
 * heading line over each). Offsets in each row count from its own record's
 * start, as pahole counts them.
 */
export function wrapRows<N extends NodeLike>(records: readonly StructRecord<N>[], bytesPerRow: number, m: RowMetrics, titled: boolean): Rows<N> {
  const lines: Line<N>[] = [];
  let y = 0;
  records.forEach((r, block) => {
    if (block > 0) y += m.blockGap;
    if (titled) {
      lines.push({ kind: "title", block, y, h: m.titleH });
      y += m.titleH;
    }
    const rowBits = bytesPerRow * 8;
    const span = r.endBit - r.startBit;
    const count = Math.max(1, Math.ceil(span / rowBits));
    const rowH = m.laneH * r.lanes;
    /** Each row's cells, before any are folded into a break. */
    const rows: Cell<N>[][] = [];
    for (let i = 0; i < count; i++) rows.push([]);
    r.pieces.forEach((p, index) => {
      const s = p.startBit - r.startBit;
      const e = p.endBit - r.startBit;
      const first = Math.floor(s / rowBits);
      const last = Math.max(first, Math.ceil(e / rowBits) - 1);
      for (let row = first; row <= last && row < count; row++) {
        const a = Math.max(s, row * rowBits) - row * rowBits;
        const b = Math.min(e, (row + 1) * rowBits) - row * rowBits;
        if (b <= a) continue;
        (rows[row] as Cell<N>[]).push({ piece: p, index, x0: a / rowBits, x1: b / rowBits, lane: p.lane });
      }
    });
    /** The piece a row is wholly made of, or -1. */
    const whole = rows.map((cells) => (cells.length === 1 && (cells[0] as Cell<N>).x0 === 0 && (cells[0] as Cell<N>).x1 === 1 ? (cells[0] as Cell<N>).index : -1));
    let i = 0;
    while (i < count) {
      const piece = whole[i] ?? -1;
      let run = 1;
      if (piece >= 0) while (i + run < count && whole[i + run] === piece) run++;
      if (piece >= 0 && run >= COLLAPSE_ROWS) {
        lines.push({ kind: "row", block, rel: i * rowBits, y, h: rowH, cells: rows[i] as Cell<N>[] });
        y += rowH;
        lines.push({ kind: "break", block, rel: (i + 1) * rowBits, y, h: m.breakH, rows: run - 2, piece: r.pieces[piece] as Piece<N>, index: piece });
        y += m.breakH;
        const end = i + run - 1;
        lines.push({ kind: "row", block, rel: end * rowBits, y, h: rowH, cells: rows[end] as Cell<N>[] });
        y += rowH;
        i += run;
        continue;
      }
      lines.push({ kind: "row", block, rel: i * rowBits, y, h: rowH, cells: rows[i] as Cell<N>[] });
      y += rowH;
      i++;
    }
  });
  return { lines, height: y };
}

/** The bytes per row a reader is offered, and the one a record starts at:
 *  16 for anything long enough to need the width, 8 otherwise, since most
 *  file headers are written in fields of up to eight bytes. */
export const ROW_WIDTHS = [4, 8, 16] as const;
export function defaultRowWidth(bytes: number): number {
  return bytes > 256 ? 16 : 8;
}

const ELLIPSIS = "…";

/**
 * `text` cut in the middle to fit `room` pixels: its head, an ellipsis and its
 * tail, whole if it fits and null if fewer than `minChars` characters would
 * be left. Fields in one structure differ at the end as often as at the
 * start (`linker_version_major`, `linker_version_minor`), so keeping both
 * ends keeps them apart where cutting the end would make them one name.
 */
export function middleCut(text: string, room: number, size: number, measure: Measure, minChars: number): string | null {
  if (measure(text, size) <= room) return text;
  const cut = (n: number): string => {
    const head = Math.ceil(n / 2);
    return text.slice(0, head) + ELLIPSIS + text.slice(text.length - (n - head));
  };
  if (text.length <= minChars || measure(cut(minChars), size) > room) return null;
  // The most characters kept that still fit, by halving: the width grows
  // with the count kept, in every face.
  let lo = minChars;
  let hi = text.length - 1;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(cut(mid), size) <= room) lo = mid;
    else hi = mid - 1;
  }
  return cut(lo);
}

/** What is drawn inside one cell: the name, a number keyed beside the row,
 *  or nothing (the hover card still has it). */
export type CellLabel = { readonly text: string; readonly keyed: boolean };

/** A row's key: the names of its members drawn as numbers, or too small for
 *  even that, in the order the members start. */
export type KeyEntry = { readonly text: string; readonly index: number };

/**
 * The labels of one row: which cell carries each piece's label (its widest
 * cell in this row, and only in the row where that cell is widest overall,
 * which `home` says), what fits there, and the key for the rest. `room` is
 * how many pixels of a cell are on screen at the current zoom. `name` is the
 * label a piece would like; `numberOf` hands out the number a keyed member
 * wears. Two names cut to the same text on one row would read as one field,
 * so both are keyed instead.
 */
export function rowLabels<N extends NodeLike>(
  cells: readonly Cell<N>[],
  room: (cell: Cell<N>) => number,
  home: (index: number) => boolean,
  name: (piece: Piece<N>) => { readonly full: string; readonly short: string | null },
  numberOf: (index: number) => number,
  opts: { readonly size: number; readonly pad: number; readonly minChars: number; readonly measure: Measure },
): { readonly labels: Map<Cell<N>, CellLabel>; readonly key: readonly KeyEntry[] } {
  const labels = new Map<Cell<N>, CellLabel>();
  const key: KeyEntry[] = [];
  const fitted = new Map<Cell<N>, string | null>();
  const seen = new Map<string, number>();
  for (const cell of cells) {
    if (!home(cell.index)) continue;
    const inside = room(cell) - 2 * opts.pad;
    const want = name(cell.piece);
    // A gap's label says how many bytes, and a cut one would not: it is drawn
    // whole, or as its short form, or not at all.
    const whole = cell.piece.kind === "field" ? middleCut(want.full, inside, opts.size, opts.measure, opts.minChars) : opts.measure(want.full, opts.size) <= inside ? want.full : null;
    const text = whole ?? (want.short !== null && opts.measure(want.short, opts.size) <= inside ? want.short : null);
    fitted.set(cell, text);
    if (text !== null && text !== want.full) seen.set(text, (seen.get(text) ?? 0) + 1);
  }
  for (const cell of cells) {
    if (!fitted.has(cell)) continue;
    const shown = room(cell);
    const want = name(cell.piece);
    const text = fitted.get(cell) ?? null;
    if (text !== null && (text === want.full || (seen.get(text) ?? 0) < 2)) {
      labels.set(cell, { text, keyed: false });
      continue;
    }
    // A gap's label is only ever its byte count, and a gap too narrow for one
    // has nothing a key could add that the hover does not.
    if (cell.piece.kind !== "field") continue;
    const n = String(numberOf(cell.index));
    if (opts.measure(n, opts.size) <= shown - 2) {
      labels.set(cell, { text: n, keyed: true });
      key.push({ text: `${n} ${want.full}`, index: cell.index });
    } else key.push({ text: want.full, index: cell.index });
  }
  return { labels, key };
}

/** Which key entries fit on one line `room` pixels wide, each `gap` from the
 *  next, and how many are left over for a "+n" at the end. */
export function fitKey(entries: readonly KeyEntry[], room: number, gap: number, size: number, measure: Measure, more: (n: number) => string): { readonly shown: readonly KeyEntry[]; readonly rest: number } {
  const shown: KeyEntry[] = [];
  let used = 0;
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i] as KeyEntry;
    const w = measure(e.text, size) + (shown.length > 0 ? gap : 0);
    const left = entries.length - i - 1;
    const tail = left > 0 ? gap + measure(more(left), size) : 0;
    if (used + w + tail > room) {
      // The entry does not fit with room for the count after it. Leave the
      // count room for what is left, taking back the last one if it must.
      while (shown.length > 0 && used + gap + measure(more(entries.length - shown.length), size) > room) {
        const last = shown.pop() as KeyEntry;
        used -= measure(last.text, size) + (shown.length > 0 ? gap : 0);
      }
      return { shown, rest: entries.length - shown.length };
    }
    shown.push(e);
    used += w;
  }
  return { shown, rest: 0 };
}

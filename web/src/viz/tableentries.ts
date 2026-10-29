// Readers for lookups: a number in one field that picks an entry in a table
// somewhere else, and the table it picks from, for `indexlookup.ts`.
//
// Part of candidate `index-lookup` in docs/DESIGN-diagrams.md (section 4.2),
// built by agent L of the first batch. The figure draws three things: the
// field holding the number, the table with the entry it lands on lit, and
// what that entry says. This module finds all three and draws nothing, so it
// can be tested against a stand-in for the document.
//
// Two readers, because files say "this number means that entry" in two ways:
//
// - The core says so. A field whose origins include `points` holds an offset,
//   and `target_bits` is where it lands: an ELF section header's `name_offset`
//   lands in `.shstrtab`, a TIFF header's `ifd_offset` lands on the first
//   directory. `Doc.locate` names the field at the landing bit, and the
//   nearest list above that field is the table. Where there is no list (a
//   pointer to one structure, as TIFF's is), the structure reached is drawn
//   as the table, its fields and the elements of its lists as the entries.
//   This reader recognises no format: every template whose offsets the core
//   follows gets the figure.
// - The format says so and the template does not. A PNG pixel of an indexed
//   image is a number into the palette, and nothing in the template links
//   the two; `paletteLookup` knows PNG's chunk layout the way `pngcards.ts`
//   does. It is the one format-specific reader here, and the place to add the
//   next (a Java constant pool, a GIF colour table) when one is wanted.
//
// "Which fields point at this entry", for the figure's step through the
// table, is `scanPointers`: the fields beside the one at the cursor that hold
// the same kind of number (every section header's `name_offset`, every pixel
// of the image), each read for the entry it lands on. It is bounded, and says
// how many it looked at, so a reader is never told "nothing points here" of a
// table it only half searched.
//
// A reply still on its way comes back as `PENDING`, and the caller asks again
// on the next render; null means the field is not an index into anything.

import type { Doc, Relation, TemplateNode, TemplateReply } from "../doc.ts";

/** The calls these readers make, so a test can stand in for the document. */
export type Reads = Pick<Doc, "templateNode" | "templateChildren" | "origins" | "relations" | "locate">;

export const PENDING = "pending" as const;
export type Pending = typeof PENDING;
export type Read<T> = T | null | Pending;

/** Most entries of one table read at once. A string table can hold tens of
 *  thousands; the figure reads this many around the entry it lands on. */
export const WINDOW = 2048;
/** Most fields read to say which entry each points at. */
export const SCAN = 1024;
/** Most siblings searched for the field that reads at an offset, and most
 *  chunks searched for a PNG's palette. Both sit near the start. */
const NEAR = 64;

/** How a table's entries are drawn: names for a string table, colours for a
 *  palette, and the record's own name for anything else. */
export type TableKind = "strings" | "palette" | "records";

export type TableEntry = {
  /** Its place in the table, from 0. */
  readonly index: number;
  readonly path: readonly number[];
  /** What to write inside its cell: the string, or the record's name. */
  readonly label: string;
  /** What its hover card says it holds. */
  readonly value: string;
  readonly startBit: number;
  readonly endBit: number;
  /** A palette entry's colour, `#rrggbb`. */
  readonly colour: string | null;
  /** The address space its bits count in: 0 is the file. */
  readonly space: number;
};

export type LookupTable = {
  readonly node: TemplateNode;
  readonly kind: TableKind;
  /** Entries in the whole table. */
  readonly count: number;
  /** The entries read, `first` onwards. */
  readonly first: number;
  readonly entries: readonly TableEntry[];
  /** What a number into the table counts: bytes from the table's start (a
   *  string table, a pointer) or entries (a palette). */
  readonly counts: "bytes" | "entries";
  /** True when a pointer reached one structure, drawn as its own fields,
   *  rather than one entry of a list. */
  readonly reached: boolean;
};

export type Lookup = {
  /** The field holding the number. */
  readonly index: TemplateNode;
  /** Where that number lands, in bits of the table's space. */
  readonly target: number;
  /** The working the core wrote for the landing address, where there is
   *  some: `section_name_base + name_offset`. */
  readonly via: Relation | null;
  readonly table: LookupTable;
  /** The entry the number lands on, by its place in the table; -1 when it
   *  lands outside every entry. */
  readonly hit: number;
  /** What the number resolves to: the field read at the far end. */
  readonly result: TemplateNode;
  /** For a palette, the colour of the entry looked up, and the alphas the
   *  tRNS chunk gives the first entries (none where there is no tRNS). */
  readonly colour: string | null;
  readonly alphas: readonly number[];
  /** The index field as a reader names it: `section_headers[2].name_offset`. */
  readonly indexLabel: string;
  /** Which reader found it, which is how the scan reads the neighbours. */
  readonly how: "points" | "palette";
};

// ----- reading helpers -----

function ok<T>(r: TemplateReply<T>): T | null | Pending {
  if (r.status === "ok") return r.node;
  return r.status === "error" ? null : PENDING;
}

function node(doc: Reads, path: readonly number[]): Read<TemplateNode> {
  return ok(doc.templateNode(path));
}

function children(doc: Reads, path: readonly number[], from: number, to: number): Read<TemplateNode[]> {
  if (to <= from) return [];
  return ok(doc.templateChildren(path, from, to));
}

function isPrefix(prefix: readonly number[], path: readonly number[]): boolean {
  return prefix.length <= path.length && prefix.every((s, i) => s === path[i]);
}

/** The number a field holds, or null where it holds none. */
export function numberOf(n: TemplateNode): number | null {
  if (n.composite) return null;
  const v = Number(n.edit_text !== "" ? n.edit_text : n.value);
  return Number.isSafeInteger(v) ? v : null;
}

/** A record's name without the `[3] ` its place in the list puts in front,
 *  since the cell already stands at that place: `[0] image width` is drawn
 *  as `image width`. A name that is only the place stays as it is. */
export function entryLabel(name: string, index: number): string {
  const prefix = `[${index}] `;
  return name.startsWith(prefix) && name.length > prefix.length ? name.slice(prefix.length) : name;
}

/** `#rrggbb` from three channel values. */
export function hexColour(r: number, g: number, b: number): string {
  const two = (v: number): string => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0");
  return `#${two(r)}${two(g)}${two(b)}`;
}

// ----- the pointer reader -----

/**
 * The lookup for a field that holds an offset the core follows, or for the
 * field read at the far end of one (`at → cstr`, whose `read_at` is set).
 */
export function pointsLookup(doc: Reads, n: TemplateNode): Read<Lookup> {
  let index: TemplateNode | null = null;
  let target: number | null = null;
  let reader: TemplateNode | null = null;
  if (n.read_at !== null) {
    // The field read somewhere else: the number is a sibling's.
    target = n.read_at;
    reader = n;
    const found = siblingPointingAt(doc, n.path, target);
    if (found === PENDING) return PENDING;
    index = found;
  } else if (!n.composite) {
    const o = ok(doc.origins(n.path));
    if (o === PENDING) return PENDING;
    const p = o?.find((x) => x.role === "points" && x.target_bits !== null);
    if (p === undefined || p.target_bits === null) return null;
    index = n;
    target = p.target_bits;
    const r = siblingReadingAt(doc, n.path, target);
    if (r === PENDING) return PENDING;
    reader = r;
  }
  if (index === null || target === null) return null;

  // What was read at the far end, and the working for its address.
  let result: TemplateNode | null = null;
  let via: Relation | null = null;
  if (reader !== null && reader.child_count > 0) {
    const k = children(doc, reader.path, 0, 1);
    if (k === PENDING) return PENDING;
    result = k?.[0] ?? null;
    if (result !== null) {
      const rel = ok(doc.relations(result.path));
      if (rel === PENDING) return PENDING;
      via = rel?.find((x) => x.role === "position") ?? null;
    }
  }

  const located = ok(doc.locate(target));
  if (located === PENDING) return PENDING;
  const table = located === null ? null : listAround(doc, located, index.path);
  if (table === PENDING) return PENDING;
  if (table !== null) {
    const read = readTable(doc, table.list, table.hit);
    if (read === PENDING) return PENDING;
    if (read === null) return null;
    const entry = read.entries.find((e) => e.index === table.hit) ?? null;
    if (result === null && entry !== null) {
      const e = node(doc, entry.path);
      if (e === PENDING) return PENDING;
      result = e;
    }
    if (result === null) return null;
    return { index, target, via, table: read, hit: table.hit, result, colour: null, alphas: [], how: "points", indexLabel: index.name };
  }
  // No list: a pointer to one structure, drawn as its own fields.
  if (result === null || !result.composite) return null;
  const reached = readReached(doc, result);
  if (reached === PENDING) return PENDING;
  if (reached === null) return null;
  const hit = reached.entries.find((e) => e.startBit <= target && target < e.endBit)?.index ?? -1;
  return { index, target, via, table: reached, hit, result, colour: null, alphas: [], how: "points", indexLabel: index.name };
}

/** The sibling of `path` that holds an offset landing at `target`. */
function siblingPointingAt(doc: Reads, path: readonly number[], target: number): Read<TemplateNode> {
  if (path.length === 0) return null;
  const parent = path.slice(0, -1);
  const up = node(doc, parent);
  if (up === PENDING || up === null) return up;
  const kids = children(doc, parent, 0, Math.min(up.child_count, NEAR));
  if (kids === PENDING || kids === null) return kids;
  for (const k of kids) {
    if (k.composite) continue;
    const o = ok(doc.origins(k.path));
    if (o === PENDING) return PENDING;
    if (o?.some((x) => x.role === "points" && x.target_bits === target)) return k;
  }
  return null;
}

/** The sibling of `path` that is read at `target`: the `at` field the offset
 *  places. */
function siblingReadingAt(doc: Reads, path: readonly number[], target: number): Read<TemplateNode> {
  if (path.length === 0) return null;
  const parent = path.slice(0, -1);
  const up = node(doc, parent);
  if (up === PENDING || up === null) return up;
  const kids = children(doc, parent, 0, Math.min(up.child_count, NEAR));
  if (kids === PENDING || kids === null) return kids;
  return kids.find((k) => k.read_at === target) ?? null;
}

/**
 * The nearest list above the field at `located`, and which of its entries
 * holds that field. A list whose entry holds the index itself is passed over:
 * an ELF section header reads its name through a field of its own, and the
 * table the name is in is not the list of headers.
 */
export function listAround(doc: Reads, located: readonly number[], indexPath: readonly number[]): Read<{ list: TemplateNode; hit: number }> {
  for (let i = located.length - 1; i >= 1; i--) {
    const entry = located.slice(0, i + 1);
    if (isPrefix(entry, indexPath)) continue;
    const up = node(doc, located.slice(0, i));
    if (up === PENDING) return PENDING;
    if (up === null) return null;
    if (up.list) return { list: up, hit: located[i] as number };
  }
  return null;
}

/** Which entries to read: `WINDOW` of them, with `hit` as near the middle as
 *  the ends of the table allow. */
export function windowAround(hit: number, count: number, size = WINDOW): { first: number; end: number } {
  if (count <= size) return { first: 0, end: count };
  const first = Math.max(0, Math.min(count - size, hit - Math.floor(size / 2)));
  return { first, end: first + size };
}

/** A list's entries around `hit`, as a table. */
function readTable(doc: Reads, list: TemplateNode, hit: number): Read<LookupTable> {
  const { first, end } = windowAround(hit, list.child_count);
  const kids = children(doc, list.path, first, end);
  if (kids === PENDING || kids === null) return kids;
  const strings = kids.length > 0 && kids.every((k) => k.kind === "str");
  const entries = kids.map((k, i) => {
    const index = first + i;
    const label = strings ? k.value : entryLabel(k.name, index);
    return { index, path: k.path, label, value: strings ? k.value : (k.line ?? k.value), startBit: k.offset_bits, endBit: k.offset_bits + k.size_bits, colour: null, space: k.space };
  });
  return { node: list, kind: strings ? "strings" : "records", count: list.child_count, first, entries, counts: "bytes", reached: false };
}

/** A structure a pointer reached, as a table of its fields, with the elements
 *  of any list among them standing for that list: a TIFF directory is its
 *  count, its twelve entries and the offset of the next one. */
function readReached(doc: Reads, s: TemplateNode): Read<LookupTable> {
  const kids = children(doc, s.path, 0, Math.min(s.child_count, WINDOW));
  if (kids === PENDING || kids === null) return kids;
  const entries: TableEntry[] = [];
  const add = (k: TemplateNode, label: string): void => {
    if (k.size_bits === 0) return;
    entries.push({ index: entries.length, path: k.path, label, value: k.composite ? (k.line ?? k.value) : k.value, startBit: k.offset_bits, endBit: k.offset_bits + k.size_bits, colour: null, space: k.space });
  };
  for (const k of kids) {
    if (k.list && k.child_count > 0 && entries.length < WINDOW) {
      const els = children(doc, k.path, 0, Math.min(k.child_count, WINDOW - entries.length));
      if (els === PENDING) return PENDING;
      for (const [i, e] of (els ?? []).entries()) add(e, entryLabel(e.name, i));
    } else add(k, k.name);
  }
  if (entries.length === 0) return null;
  return { node: s, kind: "records", count: entries.length, first: 0, entries, counts: "bytes", reached: true };
}

// ----- the PNG palette reader -----

/** The type the PNG template gives one pixel of an indexed image. */
const INDEXED_PIXEL = "IndexedPixel";

/**
 * The palette entry an indexed PNG pixel picks. The cursor can be on the
 * pixel or on the `index` inside it.
 */
export function paletteLookup(doc: Reads, n: TemplateNode): Read<Lookup> {
  let index: TemplateNode | null = null;
  if (n.type === INDEXED_PIXEL && n.child_count > 0) {
    const k = children(doc, n.path, 0, 1);
    if (k === PENDING) return PENDING;
    index = k?.[0] ?? null;
  } else if (!n.composite && n.path.length > 0) {
    const up = node(doc, n.path.slice(0, -1));
    if (up === PENDING) return PENDING;
    if (up?.type === INDEXED_PIXEL) index = n;
  }
  if (index === null) return null;
  const value = numberOf(index);
  if (value === null) return null;
  const palette = findPalette(doc);
  if (palette === PENDING || palette === null) return palette;
  const { first, end } = windowAround(value, palette.entries.child_count);
  const kids = children(doc, palette.entries.path, first, end);
  if (kids === PENDING || kids === null) return kids;
  const entries: TableEntry[] = [];
  for (const [i, k] of kids.entries()) {
    const colour = colourOf(doc, k);
    if (colour === PENDING) return PENDING;
    entries.push({ index: first + i, path: k.path, label: "", value: k.line ?? k.value, startBit: k.offset_bits, endBit: k.offset_bits + k.size_bits, colour, space: k.space });
  }
  const table: LookupTable = { node: palette.chunk, kind: "palette", count: palette.entries.child_count, first, entries, counts: "entries", reached: false };
  const hit = value < table.count ? value : -1;
  const entry = entries.find((e) => e.index === hit);
  const result = hit < 0 ? null : kids[hit - first] ?? null;
  return {
    index,
    target: entry?.startBit ?? -1,
    via: null,
    table,
    hit,
    result: result ?? index,
    colour: entry?.colour ?? null,
    alphas: palette.alphas,
    how: "palette",
    indexLabel: index.name,
  };
}

/** A palette entry's colour: the one the template gives it, or its three
 *  channels put together. */
function colourOf(doc: Reads, entry: TemplateNode): Read<string> {
  if (entry.colour !== undefined) return entry.colour.slice(0, 7);
  const rgb = children(doc, entry.path, 0, 3);
  if (rgb === PENDING || rgb === null) return rgb;
  const [r, g, b] = rgb.map((c) => numberOf(c));
  if (r == null || g == null || b == null) return null;
  return hexColour(r, g, b);
}

/** A PNG's PLTE chunk, its list of entries, and the alphas its tRNS chunk
 *  gives the first entries. Found the way `pngcards.ts` finds it: a chunk is
 *  a `Chunk` of length, type, data and crc, and the data says what it is. */
function findPalette(doc: Reads): Read<{ chunk: TemplateNode; entries: TemplateNode; alphas: number[] }> {
  const root = node(doc, []);
  if (root === PENDING || root === null) return root;
  const top = children(doc, [], 0, Math.min(root.child_count, NEAR));
  if (top === PENDING || top === null) return top;
  const list = top.find((k) => k.list && k.child_count > 0);
  if (list === undefined) return null;
  const chunks = children(doc, list.path, 0, Math.min(list.child_count, NEAR));
  if (chunks === PENDING || chunks === null) return chunks;
  let found: { chunk: TemplateNode; entries: TemplateNode } | null = null;
  let alphas: number[] = [];
  for (const c of chunks) {
    if (c.type !== "Chunk" || c.child_count !== 4) continue;
    const data = children(doc, c.path, 2, 3);
    if (data === PENDING) return PENDING;
    const d = data?.[0];
    if (d === undefined || d.child_count === 0) continue;
    if (d.type === "PLTE" && found === null) {
      const e = children(doc, d.path, 0, 1);
      if (e === PENDING) return PENDING;
      const entries = e?.[0];
      if (entries === undefined || !entries.list) return null;
      found = { chunk: c, entries };
    } else if (d.type.startsWith("tRNS") && found !== null) {
      const a = children(doc, d.path, 0, 1);
      if (a === PENDING) return PENDING;
      const list = a?.[0];
      if (list === undefined || !list.list) continue;
      const vals = children(doc, list.path, 0, Math.min(list.child_count, 256));
      if (vals === PENDING) return PENDING;
      alphas = (vals ?? []).map((v) => numberOf(v) ?? 255);
      break;
    } else if (d.type === "IDAT") break;
  }
  return found === null ? null : { ...found, alphas };
}

// ----- either -----

/** The lookup for the field at the cursor, by whichever reader knows it. */
export function lookupFor(doc: Reads, n: TemplateNode): Read<Lookup> {
  if (n.absent) return null;
  const p = paletteLookup(doc, n);
  const l = p !== null ? p : pointsLookup(doc, n);
  if (l === null || l === PENDING) return l;
  const kin = kinOf(doc, l.index.path);
  if (kin === PENDING) return PENDING;
  return kin === null ? l : { ...l, indexLabel: kin.label(kin.at) };
}

/** Whether a field could be an index at all, asked on every render, so only
 *  a look at the field: a number, the pixel holding one, or a field read
 *  somewhere else. */
export function mayLookUp(n: TemplateNode): boolean {
  if (n.absent) return false;
  if (n.read_at !== null) return true;
  if (n.type === INDEXED_PIXEL) return true;
  return !n.composite && (n.kind === "uint" || n.kind === "int" || n.kind === "enum");
}

// ----- which fields point at each entry -----

/** The entry of `table` whose bits hold `bit`, by its place in the table, or
 *  -1. The entries are in file order, so this is a binary search. */
export function entryAt(entries: readonly TableEntry[], bit: number): number {
  let lo = 0;
  let hi = entries.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((entries[mid] as TableEntry).endBit <= bit) lo = mid + 1;
    else hi = mid;
  }
  const e = entries[lo];
  return e !== undefined && e.startBit <= bit && bit < e.endBit ? e.index : -1;
}

export type Pointers = {
  /** The fields read, and how many fields like them there are. */
  readonly scanned: number;
  readonly total: number;
  /** For each entry, by its place in the table, the fields that land on it. */
  readonly byEntry: ReadonlyMap<number, readonly Pointer[]>;
};

export type Pointer = {
  readonly path: readonly number[];
  /** As a reader names it: `section_headers[2].name_offset`. */
  readonly label: string;
  readonly value: string;
  readonly startBit: number;
  readonly endBit: number;
  readonly space: number;
};

/**
 * The fields beside the index that hold the same kind of number, and which
 * entry each lands on. "Beside" is the same field of every element of the
 * nearest list the index is in: every section header's `name_offset`, every
 * pixel of the image. An index in no list is the only one of its kind.
 */
export function scanPointers(doc: Reads, lookup: Lookup): Read<Pointers> {
  const kin = kinOf(doc, lookup.index.path);
  if (kin === PENDING) return PENDING;
  if (kin === null) {
    const hit = landing(doc, lookup, lookup.index);
    if (hit === PENDING) return PENDING;
    const byEntry = new Map<number, Pointer[]>();
    if (hit >= 0) byEntry.set(hit, [pointer(lookup.index, lookup.indexLabel)]);
    return { scanned: 1, total: 1, byEntry };
  }
  const total = kin.list.child_count;
  const scanned = Math.min(total, SCAN);
  const byEntry = new Map<number, Pointer[]>();
  for (let r = 0; r < scanned; r++) {
    const n = node(doc, [...kin.list.path, r, ...kin.tail]);
    if (n === PENDING) return PENDING;
    if (n === null) continue;
    const hit = landing(doc, lookup, n);
    if (hit === PENDING) return PENDING;
    if (hit < 0) continue;
    const p = pointer(n, kin.label(r));
    const was = byEntry.get(hit);
    if (was === undefined) byEntry.set(hit, [p]);
    else was.push(p);
  }
  return { scanned, total, byEntry };
}

/**
 * The nearest list the field at `path` is in, and how to name the same field
 * of any of its elements, as the core's origins name fields:
 * `section_headers[2].name_offset`. Null for a field in no list.
 */
export function kinOf(doc: Reads, path: readonly number[]): Read<{ list: TemplateNode; at: number; tail: readonly number[]; label: (r: number) => string }> {
  for (let i = path.length - 1; i >= 1; i--) {
    const up = node(doc, path.slice(0, i));
    if (up === PENDING) return PENDING;
    if (up?.list !== true) continue;
    // The names of the steps below the list, the same for every element.
    const names: string[] = [];
    for (let j = i + 2; j <= path.length; j++) {
      const n = node(doc, path.slice(0, j));
      if (n === PENDING) return PENDING;
      if (n === null) return null;
      names.push(n.name);
    }
    const list = up;
    return { list, at: path[i] as number, tail: path.slice(i + 1), label: (r) => [`${list.name}[${r}]`, ...names].join(".") };
  }
  return null;
}

function pointer(n: TemplateNode, label: string): Pointer {
  return { path: n.path, label, value: n.value, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits, space: n.space };
}

/** The entry one field like the index lands on, or -1 for none. */
function landing(doc: Reads, lookup: Lookup, n: TemplateNode): number | Pending {
  if (lookup.how === "palette") {
    const v = numberOf(n);
    return v === null || v >= lookup.table.count ? -1 : v;
  }
  const o = ok(doc.origins(n.path));
  if (o === PENDING) return PENDING;
  const p = o?.find((x) => x.role === "points" && x.target_bits !== null);
  if (p === undefined || p.target_bits === null) return -1;
  return entryAt(lookup.table.entries, p.target_bits);
}

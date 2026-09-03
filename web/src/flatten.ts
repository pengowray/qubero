// The template tree as a flat list of things to draw, in the order the bytes
// are in the file.
//
// The listing is one view at two zoom levels: collapsed it reads as the
// file's structure, opened it reads as one line per field. Both are the same
// list, so this produces items rather than rows: a heading, a field, a run of
// bytes nothing claimed. What each one looks like is `listingreport`'s
// business; nothing here writes a word the reader sees.
//
// Only what is open is walked. A collapsed structure is one item however many
// fields it has, and a list too long to draw stops at a page and says how many
// are left, so the item count follows what the reader has opened rather than
// how big the file is.

import type { TemplateNode, TemplateReply } from "./doc.js";

/** Children of one list drawn before the reader asks for more. */
export const PAGE = 200;
/** A list of this many or fewer composites can have its elements as sections
 *  of their own: three SQLite pages are three parts of the file, and a hundred
 *  thousand tensors are one. */
export const SECTION_LIST_MAX = 64;
/** How much of the file a list has to cover before its elements count as
 *  divisions of it. Three SQLite pages are two thirds of the database and are
 *  what the file is made of; a GGUF's thirty-three metadata entries are one
 *  per cent of it and are one part of a header, however many of them there
 *  are. */
export const SECTION_SHARE = 0.25;
/** Rows a table shows without being asked. A table read as a table is the
 *  point of rule 7, and a reader who has to know to open it will not find it;
 *  a table of a hundred thousand rows is a different problem, and waits. */
export const RECORD_OPEN_MAX = 200;

export type TreeSource = {
  node(path: readonly number[]): TemplateReply<TemplateNode>;
  children(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]>;
};

/** Which elements of a long list are drawn. A window rather than a count,
 *  because the reader can arrive at element 19,974 by clicking its bytes, and
 *  drawing the 19,973 before it to reach it is the thing this avoids. */
export type Window = { readonly from: number; readonly to: number };

/** What the reader has opened. Keys are `pathKey` strings so that the state
 *  survives a re-flatten, which happens on every scroll and every chunk. */
export type ListingState = {
  /** Structures showing their children, and headings showing their bytes. */
  readonly open: ReadonlySet<string>;
  /** Which stretch of a long list is drawn, for the lists that have been
   *  moved off their first page. */
  readonly shown: ReadonlyMap<string, Window>;
  /** Items showing their bytes, by the key of the item the strip belongs to. */
  readonly bytes: ReadonlySet<string>;
};

export const emptyState: ListingState = { open: new Set(), shown: new Map(), bytes: new Set() };

/** The children of a list that are drawn, and where in it they start. */
export type Slice = { readonly from: number; readonly nodes: readonly TemplateNode[] };

type Common = {
  readonly key: string;
  /** Which top-level division of the file this sits in. */
  readonly section: number;
  /** Steps of structure below its section heading. */
  readonly depth: number;
  readonly offsetBits: number;
  readonly sizeBits: number;
};

export type Item = Common &
  (
    | {
        /** A division of the file. `level` 0 is a top-level one, 1 a named
         *  part inside it; anything deeper is rows. */
        readonly kind: "heading";
        readonly level: 0 | 1;
        readonly path: readonly number[];
        /** Absent for a heading over a run of sibling fields, which is a
         *  division of the file with no field of its own to name it. */
        readonly node: TemplateNode | null;
        /** What the heading says. The field's own name, except that an
         *  element of a list carries the list's name too: a part of the file
         *  called `[1]` says nothing from across the room, and `pages[1]`
         *  says which list it is one of. Empty when there is no node. */
        readonly title: string;
        /** The children it covers, when it covers a run of them. */
        readonly from: number;
        readonly to: number;
        readonly open: boolean;
      }
    | {
        readonly kind: "row";
        readonly path: readonly number[];
        readonly node: TemplateNode;
        /** True while this row's children are listed below it. */
        readonly open: boolean;
        /** The sibling that reads this field for a length, a count, a type
         *  or a position, when there is one and it is drawn. The row says so:
         *  a field exists because something uses it, and that is the answer
         *  to what a length prefix is for. */
        readonly reads: { readonly name: string; readonly path: readonly number[] } | null;
      }
    | {
        /** Bytes no row of the listing covers. `unmapped` says which kind:
         *  inside a structure it is the format leaving space, and between or
         *  after the file's own parts it is the template not reaching them.
         *  Both rows read "unmapped"; the difference is in where they sit and
         *  in what the rail and the hex view make of them, which is what the
         *  flag is for. */
        readonly kind: "gap";
        readonly path: readonly number[];
        readonly unmapped: boolean;
      }
    | {
        /** A structure the format stores as a table of rows. */
        readonly kind: "record";
        readonly path: readonly number[];
        readonly node: TemplateNode;
        readonly count: number;
      }
    | {
        /** One end of a list that is only partly drawn: the elements before
         *  the drawn ones, or the ones after them. Its own extent is the
         *  bytes of the elements it stands for, so the two of them and the
         *  drawn rows between them account for the whole list. */
        readonly kind: "more";
        readonly side: "earlier" | "later";
        readonly path: readonly number[];
        /** The window it is an end of, so a click can move that end. */
        readonly from: number;
        readonly to: number;
        readonly remaining: number;
      }
    | {
        /** The bytes behind an item, shown under it. Its own extent is what
         *  the strip covers, which is not always the whole of the item: a
         *  four-kilobyte page opens on its header, not on four kilobytes. */
        readonly kind: "bytes";
        readonly path: readonly number[];
        /** The item the strip belongs to, so closing it finds its own key. */
        readonly owner: string;
        readonly name: string;
      }
    | {
        /** The bytes behind this stretch have not been read yet. */
        readonly kind: "pending";
        readonly path: readonly number[];
        readonly reachedBytes: number;
      }
    | {
        /** What the file is a picture of, ahead of how it is written: the
         *  decoded image at the top of an image file. Not a part of the file,
         *  so it covers no bytes, sits in no section, and is neither in the
         *  outline nor on the file map. */
        readonly kind: "card";
        readonly card: CardKind;
        readonly path: readonly number[];
      }
    | {
        /** A structure the format keeps in a shape that is not the shape it
         *  means: a JPEG quantisation table written along the diagonals, a
         *  Huffman table whose codes are written nowhere. Drawn as a card
         *  instead of as its fields, the way a `record` is drawn as a table.
         *  Not a heading, so like the content card it is out of the outline
         *  and off the file map; unlike it, it does cover the bytes of the
         *  structure it stands for. */
        readonly kind: "formatcard";
        /** Which card the format's reader asked for. Opaque here: nothing in
         *  this file knows one from another. */
        readonly card: string;
        readonly path: readonly number[];
        readonly node: TemplateNode;
      }
  );

/** The kinds of content a file can open with. Only pictures so far. */
export type CardKind = "image";

export type FlatOptions = {
  /** Whether a structure is drawn as a table of records rather than field by
   *  field. Format-specific, and settled by the caller: see the handover's
   *  open question about where that belongs. */
  readonly isRecord?: (node: TemplateNode) => boolean;
  /** Whether a structure is drawn as a card of its own rather than field by
   *  field, and which card. Format-specific, and settled by the caller for
   *  the same reason `isRecord` is. A node with a card is always opened: a
   *  card the reader has to find is a card nobody sees. */
  readonly formatCard?: (node: TemplateNode) => string | null;
  readonly page?: number;
  readonly sectionListMax?: number;
  /** What the file opens with, before its first part: the picture an image
   *  file is of. Null or absent for a file that is only its bytes. */
  readonly card?: CardKind | null;
  /** How long the file is, in bits, when the root structure does not say.
   *  An HDF5 root is ninety-six bytes and reaches the rest of the file by
   *  address, so the root's own size is not the file's; the bytes past it
   *  are still the file's and are still a row. */
  readonly fileBits?: number;
};

export type Flattened = {
  readonly items: readonly Item[];
  /** True while any part of the list is waiting on bytes. Ask again once the
   *  chunks the source wanted have arrived. */
  readonly pending: boolean;
  readonly reachedBytes: number;
};

export function pathKey(path: readonly number[]): string {
  return path.join(".");
}

/** The bit one past the last this node covers. */
function endBits(n: TemplateNode): number {
  return n.offset_bits + n.size_bits;
}

/** Children in the order their bytes are, which is not the order they are
 *  declared in once a format places them by offset. A stable sort keeps
 *  fields of no size beside the field they were declared next to. */
function inFileOrder(nodes: readonly TemplateNode[]): TemplateNode[] {
  return nodes.map((n, i) => ({ n, i })).sort((a, b) => a.n.offset_bits - b.n.offset_bits || a.i - b.i).map((x) => x.n);
}

/** True when a list's elements are parts of the file in their own right,
 *  rather than a run of values inside one part. */
function elementsAreSections(node: TemplateNode, kids: readonly TemplateNode[], max: number, fileBits: number): boolean {
  if (!node.composite || kids.length === 0 || kids.length > max) return false;
  if (!kids.every((k) => k.composite)) return false;
  return fileBits > 0 && node.size_bits >= fileBits * SECTION_SHARE;
}

/**
 * Where a parent's children divide into top-level parts of the file. The
 * answer is the child indices that begin a new one; a run of fields between
 * two of them is one part with no field of its own.
 *
 * The rule that matched the mockups: a structure's plain fields run together,
 * and each composite child among them is a part of its own. A SQLite file is
 * its hundred-byte header and then its pages, and the template declares all
 * of that as one flat structure.
 */
export function sectionBreaks(kids: readonly TemplateNode[]): number[] {
  const breaks: number[] = [];
  let inLeafRun = false;
  // Fields with no bytes, which is what a computed value is, are not a part
  // of the file: a heading saying `Fields, 0 bytes` names nothing. They go
  // with the leaf run that follows them, or, when a structure follows, with
  // whatever part they are already in.
  let weightless: number | null = null;
  kids.forEach((k, i) => {
    if (k.composite) {
      breaks.push(i);
      inLeafRun = false;
      weightless = null;
    } else if (k.size_bits === 0) {
      if (!inLeafRun && weightless === null) weightless = i;
    } else if (!inLeafRun) {
      breaks.push(weightless ?? i);
      inLeafRun = true;
      weightless = null;
    }
  });
  if (breaks.length === 0 && kids.length > 0) breaks.push(0);
  return breaks;
}

/** Whether two siblings are in the same top-level part of the file. Machinery
 *  is only machinery in the reading where the field it places is beside it:
 *  SQLite's `page_size` sizes pages that are not in the header it is written
 *  in, and calling it plumbing there would be calling the file's own shape
 *  plumbing. */
function sameSection(breaks: readonly number[], a: number, b: number): boolean {
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  return !breaks.some((i) => i > lo && i <= hi);
}

/** Whether a field is this structure's machinery. What it places is the
 *  template's own answer; whether that counts as this structure's plumbing
 *  is this view's, and depends on where the two of them land. */
function isMachinery(node: TemplateNode, index: number, breaks: readonly number[]): boolean {
  // A computed value has no bytes, so it cannot be the bytes that place other
  // bytes, which is the whole of what this asks. What it is instead is the
  // template working something out in the open: ZIP's `data_size` is the
  // answer to whether the size in the header or the one in the ZIP64 extra
  // field is the real one.
  if (node.type === "computed") return false;
  if (node.machinery !== null) return node.machinery;
  return node.consumed_by !== null && sameSection(breaks, index, node.consumed_by);
}

class Walk {
  readonly items: Item[] = [];
  pending = false;
  reachedBytes = 0;
  readonly page: number;
  readonly sectionListMax: number;
  section = -1;
  /** The whole file, for deciding what counts as a division of it. */
  fileBits = 0;

  readonly src: TreeSource;
  readonly state: ListingState;
  readonly opts: FlatOptions;

  constructor(src: TreeSource, state: ListingState, opts: FlatOptions) {
    this.src = src;
    this.state = state;
    this.opts = opts;
    this.page = opts.page ?? PAGE;
    this.sectionListMax = opts.sectionListMax ?? SECTION_LIST_MAX;
  }

  isOpen(key: string): boolean {
    return this.state.open.has(key);
  }

  /** The strip under an item, when the reader has asked for its bytes.
   *  `over` is the stretch it covers, which the caller works out: an item and
   *  the bytes worth opening it on are not always the same. */
  strip(owner: string, path: readonly number[], name: string, over: { readonly start: number; readonly end: number }, depth: number): void {
    if (!this.state.bytes.has(owner)) return;
    this.push({
      kind: "bytes",
      key: `bytes:${owner}`,
      section: this.section,
      depth,
      offsetBits: over.start,
      sizeBits: Math.max(0, over.end - over.start),
      path,
      owner,
      name,
    });
  }

  /** Which children of `path` are drawn: the reader's window on it, or the
   *  first page when they have not moved one. */
  window(path: readonly number[], count: number): Window {
    const win = this.state.shown.get(pathKey(path));
    if (count <= 0) return { from: 0, to: 0 };
    const from = Math.min(Math.max(0, win?.from ?? 0), count - 1);
    return { from, to: Math.min(count, Math.max(from + 1, win?.to ?? this.page)) };
  }

  /** Children of `path` the reader has asked to see. Returns null once, having
   *  recorded a pending item, when the bytes are not there yet. */
  kids(path: readonly number[], count: number): Slice | null {
    const { from, to } = this.window(path, count);
    const reply = this.src.children(path, from, to);
    if (reply.status === "ok") return { from, nodes: reply.node };
    if (reply.status === "error") return { from, nodes: [] };
    this.pending = true;
    this.reachedBytes = Math.max(this.reachedBytes, reply.reachedBytes);
    return null;
  }

  push(item: Item): void {
    this.items.push(item);
  }

  waiting(path: readonly number[], depth: number, at: TemplateNode): void {
    this.push({
      kind: "pending",
      key: `wait:${pathKey(path)}`,
      section: this.section,
      depth,
      offsetBits: at.offset_bits,
      sizeBits: at.size_bits,
      path,
      reachedBytes: this.reachedBytes,
    });
  }
}

/**
 * The file as a list of items, top to bottom.
 *
 * `pending` says the answer is incomplete rather than wrong: the parts that
 * could be read are in the list and the rest is marked where it belongs, so
 * a file being streamed draws as far as it has got and fills in behind.
 */
export function flatten(src: TreeSource, state: ListingState, opts: FlatOptions = {}): Flattened {
  const w = new Walk(src, state, opts);
  const root = src.node([]);
  if (root.status === "error") return { items: [], pending: false, reachedBytes: 0 };
  if (root.status !== "ok") return { items: [], pending: true, reachedBytes: root.reachedBytes };
  w.fileBits = Math.max(root.node.size_bits, opts.fileBits ?? 0);
  // The content comes before the structure. It is not a part of the file, so
  // it takes no section number: the first heading below it is still part 0.
  const card = opts.card ?? null;
  if (card !== null) w.push({ kind: "card", key: `card:${card}`, section: -1, depth: 0, offsetBits: 0, sizeBits: 0, card, path: [] });
  const kids = w.kids([], root.node.child_count);
  if (kids === null) {
    w.waiting([], 0, root.node);
    return { items: w.items, pending: true, reachedBytes: w.reachedBytes };
  }
  sections(w, [], root.node, kids.nodes, kids.from);
  return { items: w.items, pending: w.pending, reachedBytes: w.reachedBytes };
}

/** A stretch of the list to replace, and what to put there. `from` to `to` is
 *  the run the item and everything under it occupied; `items` is that run as it
 *  is now. */
export type Refold = {
  readonly items: readonly Item[];
  readonly from: number;
  readonly to: number;
  readonly pending: boolean;
  readonly reachedBytes: number;
};

/**
 * One item's fold, walked again on its own.
 *
 * Opening a structure changes what is under that structure and nothing else:
 * the parts of the file, their numbering, the rows above it and the rows after
 * it are all what they were. So the whole tree does not have to be walked to
 * find that out. `state` is the state as it is now, `at` the index of the item
 * whose fold moved, and the answer is the run of the list to replace.
 *
 * It walks the item itself as well as its contents, since the item says
 * whether it is open and whether its bytes are showing, and it goes through the
 * same `child` and `heading` the first walk used: two ways of producing the
 * same rows would be two things to keep in step. That is why the same call
 * answers a byte strip opening as well as a fold — a strip is part of what an
 * item stands over, so the run to replace is the same run either way.
 *
 * Null for an item with nothing of its own to walk, which the caller answers by
 * walking the whole tree.
 */
export function refold(src: TreeSource, state: ListingState, opts: FlatOptions, items: readonly Item[], at: number): Refold | null {
  const item = items[at];
  if (item === undefined) return null;
  const w = new Walk(src, state, opts);
  w.section = item.section;
  if (item.kind === "row") {
    child(w, item.node, item.depth, item.reads);
  } else if (item.kind === "heading" && item.node !== null && item.level === 1) {
    // A heading at this level is what `child` makes of a composite nothing
    // reads, so that is what to hand back to it.
    child(w, item.node, item.depth, null);
  } else if (item.kind === "heading" && item.node !== null) {
    // A top-level part, which `sections` made by reading its children and
    // handing them to `heading`. Reading them again is the same question with
    // the same answer, since nothing above this item moved.
    const inner = w.kids(item.path, item.node.child_count);
    heading(w, item.path, item.node, item.level, item.from, item.to, inner, item.title);
  } else return null;
  // What the item stood over: its own byte strip, which sits at its depth, and
  // then everything indented under it. The first item at its depth or above is
  // the next sibling, or the end of the part.
  let to = at + 1;
  const strip = items[to];
  if (strip?.kind === "bytes" && strip.owner === item.key) to += 1;
  while (to < items.length && (items[to]?.depth ?? 0) > item.depth) to += 1;
  return { items: w.items, from: at, to, pending: w.pending, reachedBytes: w.reachedBytes };
}

/** What a part is called on its heading. An element of a list is named `[n]`
 *  by the template, which is its place and not its name; the list's name in
 *  front of it is what a reader can find it by again. */
function titleOf(node: TemplateNode, list: string | null): string {
  return list !== null && /^\[\d+\]$/.test(node.name) ? `${list}${node.name}` : node.name;
}

/** The top-level parts of the file, and what is in each. `list` is the name
 *  of the list these are the elements of, when they are. */
function sections(w: Walk, path: readonly number[], parent: TemplateNode, kids: readonly TemplateNode[], base = 0, list: string | null = null): void {
  const breaks = sectionBreaks(kids);
  // What each part of the file actually covers, worked out before any of them
  // is drawn. Template order is not file order: a PDF declares the offset of
  // its cross-reference table before the table itself — so a running cursor
  // through the declarations answers the wrong question, and a field that
  // points elsewhere answers it at the pointer rather than at the target.
  type Part = { readonly from: number; readonly to: number; readonly inner: Slice | null; readonly extent: Span | null };
  const parts: Part[] = [];
  breaks.forEach((from, b) => {
    const to = breaks[b + 1] ?? kids.length;
    const first = kids[from];
    if (first === undefined) return;
    if (to - from === 1 && first.composite) {
      const inner = w.kids([...path, from], first.child_count);
      const covers = pointee(w, [...path, from], first, inner);
      parts.push({ from, to, inner, extent: extentOf([covers]) });
      return;
    }
    parts.push({ from, to, inner: null, extent: extentOf(kids.slice(from, to)) });
  });
  // Rule 1 is about the file, not only about the insides of a structure. The
  // parts of a 450 MiB HDF5 file that its template describes are its first
  // ninety-six bytes; everything after that is reached through addresses
  // rather than by lying next to them, and a listing that simply stops at
  // 0x60 has lost four hundred and fifty megabytes without a word.
  //
  // The root's end is the file's end, whatever the root structure says its
  // own size is; anything inside the root ends where it ends.
  const end = path.length === 0 ? Math.max(endBits(parent), w.fileBits) : endBits(parent);
  const holes = uncovered(parent.offset_bits, end, parts.map((p) => p.extent));
  // Every hole starts either where the parent does or where some part ends,
  // so each one has a place in the list without the parts being reordered.
  const drawnHoles = new Set<number>();
  const at = (bit: number): void => {
    for (const hole of holes) {
      if (hole.start !== bit || drawnHoles.has(hole.start)) continue;
      drawnHoles.add(hole.start);
      gap(w, path, hole.start, hole.end, 0, true);
    }
  };
  at(parent.offset_bits);
  for (const part of parts) {
    const first = kids[part.from];
    if (first === undefined) continue;
    w.section += 1;
    if (part.to - part.from === 1 && first.composite) {
      const kidPath = [...path, part.from];
      // A list of a handful of structures is a handful of parts of the file,
      // not one part holding a list: three SQLite pages read as three. Only a
      // list drawn whole can be, since a part of the file that is only some of
      // a list is not a part of the file.
      if (part.inner !== null && part.inner.from === 0 && part.inner.nodes.length === first.child_count && elementsAreSections(first, part.inner.nodes, w.sectionListMax, w.fileBits)) {
        w.section -= 1;
        sections(w, kidPath, first, part.inner.nodes, part.inner.from, first.name);
      } else {
        heading(w, kidPath, first, 0, part.from, part.to, part.inner, titleOf(first, list));
      }
    } else {
      runHeading(w, path, kids, part.from, part.to, breaks, base);
    }
    if (part.extent !== null) at(part.extent.end);
  }
}

/** A stretch of bits. */
type Span = { readonly start: number; readonly end: number };

/** What a run of siblings covers between them. Fields of no bytes contribute
 *  nothing: a computed value is not a stretch of the file. */
function extentOf(nodes: readonly TemplateNode[]): Span | null {
  const real = nodes.filter((n) => n.size_bits > 0);
  if (real.length === 0) return null;
  return { start: Math.min(...real.map((n) => n.offset_bits)), end: Math.max(...real.map(endBits)) };
}

/** The stretches of `start` to `end` that none of `spans` covers, in order. */
function uncovered(start: number, end: number, spans: readonly (Span | null)[]): Span[] {
  const sorted = spans.filter((s): s is Span => s !== null).sort((a, b) => a.start - b.start);
  const holes: Span[] = [];
  let cursor = start;
  for (const span of sorted) {
    if (span.start > cursor) holes.push({ start: cursor, end: Math.min(span.start, end) });
    cursor = Math.max(cursor, span.end);
    if (cursor >= end) break;
  }
  if (cursor < end) holes.push({ start: cursor, end });
  return holes.filter((h) => h.end > h.start);
}

/** What a heading covers. A field that points elsewhere costs nothing where
 *  it is declared, so its node is empty and sits at the pointer; a heading
 *  saying `directory, 0 bytes` there is not where the directory is. The one
 *  thing it points at is. */
function pointee(w: Walk, path: readonly number[], node: TemplateNode, inner: Slice | null): TemplateNode {
  if (node.size_bits !== 0 || !node.composite || node.child_count !== 1) return node;
  const only = (inner ?? w.kids(path, 1))?.nodes[0];
  return only !== undefined && only.size_bits > 0 ? only : node;
}

/** One part of the file with a field of its own to name it. */
function heading(
  w: Walk,
  path: readonly number[],
  node: TemplateNode,
  level: 0 | 1,
  from: number,
  to: number,
  inner: Slice | null,
  title: string = node.name,
): void {
  const key = pathKey(path);
  const open =
    level === 0 ||
    w.isOpen(key) ||
    (node !== null && w.opts.isRecord?.(node) === true && node.child_count <= RECORD_OPEN_MAX) ||
    w.opts.formatCard?.(node) != null;
  const covers = pointee(w, path, node, inner);
  w.push({
    kind: "heading",
    key: `h:${key}`,
    section: w.section,
    depth: level,
    offsetBits: covers.offset_bits,
    sizeBits: covers.size_bits,
    level,
    path,
    node,
    title,
    from,
    to,
    open,
  });
  // What the strip covers must not depend on whether the reader has opened
  // the heading: the same bytes button gave two different stretches before and
  // after a click. So when the strip is up, the children are read for it
  // whether or not they are being listed.
  const stripKey = `h:${key}`;
  const seen = inner ?? (w.state.bytes.has(stripKey) ? w.kids(path, node.child_count) : null);
  w.strip(stripKey, path, node?.name ?? "", frontOf(covers, seen), level + 1);
  if (!open) return;
  if (inner === null) {
    w.waiting(path, level + 1, node);
    return;
  }
  body(w, path, node, inner, level + 1);
}

/** What a heading opens on. A four-kilobyte page is not four kilobytes of hex:
 *  what a reader wants from it is the machinery at its front, which is where
 *  its first payload field begins. A part with no machinery opens on itself,
 *  and so does a list whose drawn elements do not include its first: the front
 *  of what is drawn is the middle of the part. */
function frontOf(node: TemplateNode, kids: Slice | null): { readonly start: number; readonly end: number } {
  const start = node.offset_bits;
  const whole = { start, end: start + node.size_bits };
  if (kids === null || kids.from > 0) return whole;
  const order = inFileOrder(kids.nodes);
  const first = order.find((k, i) => !isMachinery(k, i, []));
  if (first === undefined || first.offset_bits <= start) return whole;
  // Only a bulk structure is worth leaving out: the page's cells, which have
  // rows of their own to be read in. A plain field after the machinery is
  // part of what the reader asked to see, and the strip already cuts a long
  // one short by itself. A descriptor whose one byte of machinery is followed
  // by its magic string opens on the descriptor, not on the byte.
  if (!first.composite || first.child_count === 0) return whole;
  // And only when there is a front to look at. A list whose elements sit at
  // the back of a page has three and a half thousand bytes of free space
  // before its first element, and opening on that shows a strip with nothing
  // in it; a front of one field is not a front but a field, and the strip
  // would be that field's column and nothing else.
  if (order.filter((k) => k.offset_bits >= start && k.offset_bits < first.offset_bits).length < 2) return whole;
  return { start, end: first.offset_bits };
}

/** A part of the file made of a run of a structure's plain fields, which has
 *  no field of its own to name it: a file header declared inline with what it
 *  describes. */
function runHeading(
  w: Walk,
  path: readonly number[],
  kids: readonly TemplateNode[],
  from: number,
  to: number,
  breaks: readonly number[],
  base: number,
): void {
  const run = kids.slice(from, to);
  const first = run[0];
  const last = run[run.length - 1];
  if (first === undefined || last === undefined) return;
  w.push({
    kind: "heading",
    key: `h:${pathKey(path)}[${from}-${to}]`,
    section: w.section,
    depth: 0,
    offsetBits: first.offset_bits,
    sizeBits: endBits(last) - first.offset_bits,
    level: 0,
    path,
    node: null,
    title: "",
    from,
    to,
    open: true,
  });
  // The run's own edges are where the parts either side of it begin, so there
  // is nothing unaccounted for at them.
  // The run is a slice of a slice, so what reads one of its fields may be a
  // sibling outside it: SQLite's `page_size` is written in the header and read
  // by a page. The whole of the parent's children go along for that lookup.
  rows(w, path, run, base + from, breaks, 1, null, kids, base);
}

/** What is inside one heading. */
function body(
  w: Walk,
  path: readonly number[],
  node: TemplateNode,
  kids: Slice,
  depth: number,
): void {
  const card = w.opts.formatCard?.(node) ?? null;
  if (card !== null) {
    w.push({
      kind: "formatcard",
      key: `fc:${pathKey(path)}`,
      section: w.section,
      depth,
      offsetBits: node.offset_bits,
      sizeBits: node.size_bits,
      path,
      node,
      card,
    });
    rowStrips(w, path, depth);
    return;
  }
  if (w.opts.isRecord?.(node) === true) {
    w.push({
      kind: "record",
      key: `rec:${pathKey(path)}`,
      section: w.section,
      depth,
      offsetBits: node.offset_bits,
      sizeBits: node.size_bits,
      path,
      node,
      count: node.child_count,
    });
    rowStrips(w, path, depth);
    return;
  }
  drawn(w, path, node, kids, depth);
}

/** The byte strips under a view that draws its own rows: a record table or a
 *  format card. Neither lists the fields it was built from, so nothing in
 *  them answers "how is that row written"; asking a row for its bytes puts
 *  its strip under the view, which is the way back to the fields. Only rows
 *  already asked for are looked up, so a table of a million costs nothing
 *  until one of them is. */
function rowStrips(w: Walk, path: readonly number[], depth: number): void {
  const prefix = `r:${pathKey(path)}.`;
  for (const key of w.state.bytes) {
    if (!key.startsWith(prefix)) continue;
    const rowPath = key.slice(prefix.length).split(".").map(Number);
    if (rowPath.some((n) => !Number.isInteger(n))) continue;
    const full = [...path, ...rowPath];
    const reply = w.src.node(full);
    if (reply.status !== "ok") continue;
    w.strip(key, full, reply.node.name, { start: reply.node.offset_bits, end: endBits(reply.node) }, depth + 1);
  }
}

/**
 * A structure's children, with the ends of a list that is only partly drawn
 * standing for the elements that are not.
 *
 * The rows either side own the bytes of what they stand for, so the window's
 * edges are not holes: without that, jumping to element 19,974 would put
 * nineteen thousand elements' worth of "unused space" above it.
 */
function drawn(w: Walk, path: readonly number[], node: TemplateNode, slice: Slice, depth: number): void {
  const before = slice.from;
  const after = node.child_count - (slice.from + slice.nodes.length);
  const order = inFileOrder(slice.nodes);
  const first = order[0];
  const last = order[order.length - 1];
  // A compressed stream's contents are not bytes of the stream: they are
  // counted in a space of their own, and none of them accounts for any of the
  // run the stream occupies. Measuring them against the parent's extent would
  // find the whole run unaccounted for and draw a gap over it.
  const elsewhere = first !== undefined && first.space !== node.space;
  // A field of no bytes has no edges of its own to measure its children
  // against: a pointer costs nothing where it is declared and its target
  // lives somewhere else entirely. Measuring the target against the pointer
  // would find every byte between the two unaccounted for and draw a gap over
  // them, inside a heading that is nowhere near them. So the children are the
  // extent.
  const hollow = node.size_bits === 0 && order.length > 0;
  const start = (before > 0 || hollow) && first !== undefined ? first.offset_bits : node.offset_bits;
  const end = hollow ? Math.max(...order.map(endBits)) : after > 0 && last !== undefined ? endBits(last) : endBits(node);
  // Where the elements this window leaves out are. Unknown for a hollow node,
  // whose own extent says nothing about them, so those ends stand for no
  // bytes rather than for the wrong ones.
  const outer = hollow ? { start, end } : { start: node.offset_bits, end: endBits(node) };
  const to = slice.from + slice.nodes.length;
  if (before > 0 && !elsewhere) {
    edge(w, "earlier", path, depth, before, slice.from, to, { start: outer.start, end: start });
  }
  rows(w, path, slice.nodes, slice.from, [], depth, elsewhere ? null : { start, end });
  if (after > 0 && !elsewhere) {
    edge(w, "later", path, depth, after, slice.from, to, { start: end, end: outer.end });
  }
}

/** A structure's children as items: a row or a heading each, in the order
 *  their bytes are, with a gap wherever nothing covers them.
 *
 *  `bounds` is the stretch the children have to account for between them,
 *  which is the parent's own. Free space in a b-tree page sits between the
 *  cell pointers at the front and the cells at the back, and both edges are
 *  the parent's rather than a child's: without the bounds, three and a half
 *  thousand bytes of a four-thousand-byte page go unmentioned. Null for a run
 *  of fields that is not a whole structure, where the edges belong to the
 *  parts either side. */
function rows(
  w: Walk,
  path: readonly number[],
  kids: readonly TemplateNode[],
  base: number,
  breaks: readonly number[],
  depth: number,
  bounds: { readonly start: number; readonly end: number } | null,
  siblings: readonly TemplateNode[] = kids,
  sibBase: number = base,
): void {
  const order = inFileOrder(kids);
  let cursor = bounds?.start ?? order[0]?.offset_bits ?? 0;
  // Which space these are counted in. A run of siblings is all in one, so the
  // first of them settles it; a gap between two of them is real, and one
  // measured against an offset from another space is not.
  const space = order[0]?.space ?? 0;
  for (const kid of order) {
    if (kid.offset_bits > cursor && kid.space === space) gap(w, path, cursor, kid.offset_bits, depth);
    cursor = Math.max(cursor, endBits(kid));
    // A field that is only its parent's contents has no name worth a level of
    // structure: its children stand in its place, at its depth.
    if (!kid.contents || !kid.composite || kid.child_count === 0) {
      child(w, kid, depth, consumerOf(kid, siblings, sibBase));
      continue;
    }
    const inner = w.kids(kid.path, kid.child_count);
    if (inner === null) w.waiting(kid.path, depth, kid);
    else drawn(w, kid.path, kid, inner, depth);
  }
  if (bounds !== null && cursor < bounds.end) gap(w, path, cursor, bounds.end, depth);
}

function gap(w: Walk, path: readonly number[], from: number, to: number, depth: number, unmapped = false): void {
  const key = `gap:${pathKey(path)}@${from}`;
  w.push({
    kind: "gap",
    key,
    section: w.section,
    depth,
    offsetBits: from,
    sizeBits: to - from,
    path,
    unmapped,
  });
  // A gap the reader can look into. What is in bytes nothing describes is the
  // one question the row cannot answer for them, and the verdict beside it is
  // a summary of exactly the thing they would want to read.
  w.strip(key, path, "", { start: from, end: to }, depth + 1);
}

/** One child of a structure: a heading when it is a named part with something
 *  inside it, a row otherwise. Rule 2: depth past a sub-heading is rows, not
 *  more indentation. */
/** The sibling that reads this field, when it is one of the children in hand
 *  and is a row the reader can be sent to. A field that is only its parent's
 *  contents is spliced away and never drawn, so pointing at it would point at
 *  nothing. */
function consumerOf(node: TemplateNode, kids: readonly TemplateNode[], base: number): { readonly name: string; readonly path: readonly number[] } | null {
  if (node.consumed_by === null) return null;
  const owner = kids[node.consumed_by - base];
  if (owner === undefined || owner.contents) return null;
  return { name: owner.name, path: owner.path };
}

function child(w: Walk, node: TemplateNode, depth: number, reads: { readonly name: string; readonly path: readonly number[] } | null = null): void {
  const key = pathKey(node.path);
  // A structure that places another field is not a division of the file, so it
  // stays a row: an array of cell pointers belongs beside the fields it was
  // written among, not as a heading over the page's contents.
  if (node.composite && node.child_count > 0 && depth === 1 && reads === null) {
    // A short table opens itself, since the table is what it is for.
    const open =
      w.isOpen(key) || (node.child_count <= RECORD_OPEN_MAX && w.opts.isRecord?.(node) === true) || w.opts.formatCard?.(node) != null;
    // Null rather than an empty slice: "not read" and "read, and empty" are
    // different answers, and the strip's extent turns on which it is.
    heading(w, node.path, node, 1, 0, node.child_count, open ? w.kids(node.path, node.child_count) : null);
    return;
  }
  const open = node.composite && node.child_count > 0 && w.isOpen(key);
  w.push({
    kind: "row",
    key: `r:${key}`,
    section: w.section,
    depth,
    offsetBits: node.offset_bits,
    sizeBits: node.size_bits,
    path: node.path,
    node,
    open,
    reads,
  });
  w.strip(`r:${key}`, node.path, node.name, { start: node.offset_bits, end: endBits(node) }, depth);
  if (!open) return;
  const inner = w.kids(node.path, node.child_count);
  if (inner === null) {
    w.waiting(node.path, depth + 1, node);
    return;
  }
  drawn(w, node.path, node, inner, depth + 1);
}

/** One end of a list that is only partly drawn. */
function edge(
  w: Walk,
  side: "earlier" | "later",
  path: readonly number[],
  depth: number,
  remaining: number,
  from: number,
  to: number,
  over: { readonly start: number; readonly end: number },
): void {
  w.push({
    kind: "more",
    side,
    key: `more:${side}:${pathKey(path)}`,
    section: w.section,
    depth,
    offsetBits: over.start,
    sizeBits: Math.max(0, over.end - over.start),
    path,
    from,
    to,
    remaining,
  });
}

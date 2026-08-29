// The template tree as a flat list of things to draw, in the order the bytes
// are in the file.
//
// The listing is one view at two zoom levels: collapsed it reads as the
// file's structure, opened it reads as one line per field. Both are the same
// list, so this produces items rather than rows: a heading, a field, a run of
// bytes nothing claimed, a fold hiding the machinery behind the field it
// places. What each one looks like is `listingview`'s business; nothing here
// writes a word the reader sees.
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

export type TreeSource = {
  node(path: readonly number[]): TemplateReply<TemplateNode>;
  children(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]>;
};

/** What the reader has opened. Keys are `pathKey` strings so that the state
 *  survives a re-flatten, which happens on every scroll and every chunk. */
export type ListingState = {
  /** Structures showing their children, and headings showing their bytes. */
  readonly open: ReadonlySet<string>;
  /** Children of a long list asked for so far, over the first page. */
  readonly shown: ReadonlyMap<string, number>;
};

export const emptyState: ListingState = { open: new Set(), shown: new Map() };

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
      }
    | {
        /** Bytes inside a structure that none of its fields covers. */
        readonly kind: "gap";
        readonly path: readonly number[];
      }
    | {
        /** The fields that place another field, hidden behind it. */
        readonly kind: "fold";
        readonly reason: "machinery";
        readonly path: readonly number[];
        readonly nodes: readonly TemplateNode[];
        /** The field they place, when it is one of their siblings. */
        readonly owner: number | null;
        readonly open: boolean;
      }
    | {
        /** A structure the format stores as a table of rows. */
        readonly kind: "record";
        readonly path: readonly number[];
        readonly node: TemplateNode;
        readonly count: number;
      }
    | {
        /** The tail of a list longer than the page drawn so far. */
        readonly kind: "more";
        readonly path: readonly number[];
        readonly shown: number;
        readonly remaining: number;
      }
    | {
        /** The bytes behind this stretch have not been read yet. */
        readonly kind: "pending";
        readonly path: readonly number[];
        readonly reachedBytes: number;
      }
  );

export type FlatOptions = {
  /** Whether a structure is drawn as a table of records rather than field by
   *  field. Format-specific, and settled by the caller: see the handover's
   *  open question about where that belongs. */
  readonly isRecord?: (node: TemplateNode) => boolean;
  readonly page?: number;
  readonly sectionListMax?: number;
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
  kids.forEach((k, i) => {
    if (k.composite) {
      breaks.push(i);
      inLeafRun = false;
    } else if (!inLeafRun) {
      breaks.push(i);
      inLeafRun = true;
    }
  });
  return breaks;
}

/** Whether two siblings are in the same top-level part of the file. Machinery
 *  folds behind the field it places, and there is nothing to fold it behind
 *  when that field is somewhere else entirely: SQLite's `page_size` sizes
 *  pages that are not in the header it is written in. */
function sameSection(breaks: readonly number[], a: number, b: number): boolean {
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  return !breaks.some((i) => i > lo && i <= hi);
}

/** Whether a field is this structure's machinery. What it places is the
 *  template's own answer; whether that counts as folding it away is this
 *  view's, and depends on where the two of them land. */
function isMachinery(node: TemplateNode, index: number, breaks: readonly number[]): boolean {
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

  /** Children of `path`, up to what the reader has asked to see. Returns null
   *  once, having recorded a pending item, when the bytes are not there yet. */
  kids(path: readonly number[], count: number): TemplateNode[] | null {
    const key = pathKey(path);
    const want = Math.min(count, Math.max(this.page, this.state.shown.get(key) ?? 0));
    const reply = this.src.children(path, 0, want);
    if (reply.status === "ok") return reply.node;
    if (reply.status === "error") return [];
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
  w.fileBits = root.node.size_bits;
  const kids = w.kids([], root.node.child_count);
  if (kids === null) {
    w.waiting([], 0, root.node);
    return { items: w.items, pending: true, reachedBytes: w.reachedBytes };
  }
  sections(w, [], root.node, kids);
  return { items: w.items, pending: w.pending, reachedBytes: w.reachedBytes };
}

/** The top-level parts of the file, and what is in each. */
function sections(w: Walk, path: readonly number[], parent: TemplateNode, kids: readonly TemplateNode[]): void {
  const breaks = sectionBreaks(kids);
  breaks.forEach((from, b) => {
    const to = breaks[b + 1] ?? kids.length;
    const first = kids[from];
    if (first === undefined) return;
    w.section += 1;
    if (to - from === 1 && first.composite) {
      const kidPath = [...path, from];
      const inner = w.kids(kidPath, first.child_count);
      // A list of a handful of structures is a handful of parts of the file,
      // not one part holding a list: three SQLite pages read as three.
      if (inner !== null && elementsAreSections(first, inner, w.sectionListMax, w.fileBits)) {
        w.section -= 1;
        sections(w, kidPath, first, inner);
        return;
      }
      heading(w, kidPath, first, 0, from, to, inner);
      return;
    }
    runHeading(w, path, kids, from, to, breaks);
  });
}

/** One part of the file with a field of its own to name it. */
function heading(
  w: Walk,
  path: readonly number[],
  node: TemplateNode,
  level: 0 | 1,
  from: number,
  to: number,
  inner: readonly TemplateNode[] | null,
): void {
  const key = pathKey(path);
  const open = level === 0 || w.isOpen(key);
  w.push({
    kind: "heading",
    key: `h:${key}`,
    section: w.section,
    depth: level,
    offsetBits: node.offset_bits,
    sizeBits: node.size_bits,
    level,
    path,
    node,
    from,
    to,
    open,
  });
  if (!open) return;
  if (inner === null) {
    w.waiting(path, level + 1, node);
    return;
  }
  body(w, path, node, inner, level + 1);
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
    from,
    to,
    open: true,
  });
  // The run's own edges are where the parts either side of it begin, so there
  // is nothing unaccounted for at them.
  rows(w, path, run, from, breaks, 1, null);
}

/** What is inside one heading. */
function body(
  w: Walk,
  path: readonly number[],
  node: TemplateNode,
  kids: readonly TemplateNode[],
  depth: number,
): void {
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
    return;
  }
  rows(w, path, kids, 0, [], depth, { start: node.offset_bits, end: endBits(node) });
  more(w, path, node, kids.length, depth);
}

/** A structure's children as items: gaps where nothing covers the bytes,
 *  machinery folded into one item per run, and everything else a row or a
 *  heading of its own.
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
): void {
  const order = inFileOrder(kids);
  const indexOf = new Map(kids.map((k, i) => [k, base + i]));
  let cursor = bounds?.start ?? order[0]?.offset_bits ?? 0;
  let fold: TemplateNode[] = [];
  const flush = () => {
    if (fold.length === 0) return;
    const first = fold[0];
    const last = fold[fold.length - 1];
    if (first === undefined || last === undefined) return;
    const owners = fold.map((n) => n.consumed_by).filter((c): c is number => c !== null);
    w.push({
      kind: "fold",
      key: `fold:${pathKey(first.path)}`,
      section: w.section,
      depth,
      offsetBits: first.offset_bits,
      sizeBits: endBits(last) - first.offset_bits,
      reason: "machinery",
      path,
      nodes: fold,
      owner: owners[owners.length - 1] ?? null,
      open: w.isOpen(`fold:${pathKey(first.path)}`),
    });
    fold = [];
  };
  for (const kid of order) {
    if (kid.offset_bits > cursor) {
      flush();
      gap(w, path, cursor, kid.offset_bits, depth);
    }
    cursor = Math.max(cursor, endBits(kid));
    const index = indexOf.get(kid) ?? 0;
    if (isMachinery(kid, index, breaks)) {
      fold.push(kid);
      continue;
    }
    flush();
    child(w, kid, depth);
  }
  flush();
  if (bounds !== null && cursor < bounds.end) gap(w, path, cursor, bounds.end, depth);
}

function gap(w: Walk, path: readonly number[], from: number, to: number, depth: number): void {
  w.push({
    kind: "gap",
    key: `gap:${pathKey(path)}@${from}`,
    section: w.section,
    depth,
    offsetBits: from,
    sizeBits: to - from,
    path,
  });
}

/** One child of a structure: a heading when it is a named part with something
 *  inside it, a row otherwise. Rule 2: depth past a sub-heading is rows, not
 *  more indentation. */
function child(w: Walk, node: TemplateNode, depth: number): void {
  const key = pathKey(node.path);
  if (node.composite && node.child_count > 0 && depth === 1) {
    const open = w.isOpen(key);
    heading(w, node.path, node, 1, 0, node.child_count, open ? w.kids(node.path, node.child_count) : []);
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
  });
  if (!open) return;
  const inner = w.kids(node.path, node.child_count);
  if (inner === null) {
    w.waiting(node.path, depth + 1, node);
    return;
  }
  rows(w, node.path, inner, 0, [], depth + 1, { start: node.offset_bits, end: endBits(node) });
  more(w, node.path, node, inner.length, depth + 1);
}

/** How much of a long list is still not drawn. */
function more(w: Walk, path: readonly number[], node: TemplateNode, drawn: number, depth: number): void {
  const remaining = node.child_count - drawn;
  if (remaining <= 0) return;
  w.push({
    kind: "more",
    key: `more:${pathKey(path)}`,
    section: w.section,
    depth,
    offsetBits: node.offset_bits,
    sizeBits: node.size_bits,
    path,
    shown: drawn,
    remaining,
  });
}

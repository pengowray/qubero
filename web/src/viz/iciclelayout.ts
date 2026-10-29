// The arithmetic half of the position icicle (`icicle.ts`): which bars to draw
// for the template's tree at a given zoom, and where.
//
// One row per depth of the tree, the file's address axis across. Every bar is
// exactly as wide as its field's bytes and sits where those bytes are, so the
// picture is a map of the file as well as of its structure: unlike the
// treemap, which sizes boxes by area and so loses where anything is, this
// keeps order and position. What that costs is that the tree has to be read
// as the reader zooms, since a file of a hundred megabytes has more fields
// than any picture has pixels. So reading follows the pixels:
// - A bar's children are read only once the bar is drawn at least
//   `LOD.childrenPx` wide. Narrower, they would be slivers nobody can point
//   at, and reading them is a walk for nothing.
// - Only the children inside the visible stretch of the axis are laid out. A
//   node with more children than `ALL_MAX` is read by window: the children
//   are found by halving on their offsets, so zooming into the middle of a
//   quarter of a million instructions reads the few hundred on screen.
// - Neighbouring children too narrow to tell apart (under `FOLD_PX` each) are
//   drawn as one bar that says how many there are. Zooming in splits it.
// - Bytes inside a structure that none of its children covers are a bar of
//   their own, hatched, so a hole in the description is visible as a hole.
//
// Two kinds of node are passed over rather than drawn:
// - A child with `contents` set is only its parent's contents (a box's body,
//   a chunk's data). Its children are drawn in its place, one level up, so the
//   rows go box, then the box's size, type and inner boxes, as the MP4 and
//   RIFF specs draw them, and no row is spent repeating the bar above with
//   the word "body" on it.
// - A child in another address space (the bytes a compressed stream unpacks
//   to) has no place on this file's axis, so it is not drawn here.
//
// DOM-free, and reading through `IcicleSource` rather than the Doc, so the
// tests can hand it a tree of their own. Every read is cached until `clear`,
// which the caller does when the document changes.

import type { Problem, ProblemCount } from "../doc.ts";
import { LOD } from "./lod.ts";
import { assignLanes } from "./lanes.ts";
import { nearestOnAxis, type AxisTarget } from "./pickmath.ts";

/** What the icicle needs to know about a field. `TemplateNode` is one. */
export type IcicleField = {
  readonly path: readonly number[];
  readonly name: string;
  readonly type: string;
  readonly kind: string;
  readonly value: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly child_count: number;
  readonly composite: boolean;
  readonly contents: boolean;
  readonly framed: boolean;
  readonly list: boolean;
  readonly unit?: string;
  readonly space: number;
  readonly problem?: Problem;
  readonly problems_within: ProblemCount;
};

export type IcicleSource = {
  /** Children `from` to `to` (exclusive) of the node at `path`, or null while
   *  the bytes they are read from are still on the way. */
  children(path: readonly number[], from: number, to: number): readonly IcicleField[] | null;
};

/** Children read in one call. */
const PAGE = 512;
/** A node with more children than this is read by window, by halving on the
 *  children's offsets, rather than all at once. */
export const ALL_MAX = 4096;
/** Neighbours narrower than this many pixels each are drawn as one bar. Two
 *  bars a pixel wide with the one-pixel gap between them read as one grey
 *  smear, so two pixels is the narrowest a bar is drawn on its own. */
export const FOLD_PX = 2;
/** The most children of one node laid out at once. A window holding more
 *  than this is drawn as one folded bar until the reader zooms in. */
const WINDOW_MAX = 4096;

const key = (path: readonly number[]): string => path.join(".");

export class IcicleCache {
  private readonly pages = new Map<string, readonly IcicleField[]>();
  private readonly singles = new Map<string, IcicleField>();
  /** Set when a read answered "not yet" since the last `clear`. */
  pending = false;

  private readonly source: IcicleSource;

  constructor(source: IcicleSource) {
    this.source = source;
  }

  /** Forget everything read. The document changed, or more of it arrived. */
  clear(): void {
    this.pages.clear();
    this.singles.clear();
    this.pending = false;
  }

  /** Children `from` to `to` of `node`, by pages of `PAGE`, or null while any
   *  of them is not read yet. */
  range(node: IcicleField, from: number, to: number): IcicleField[] | null {
    const out: IcicleField[] = [];
    const end = Math.min(to, node.child_count);
    for (let page = Math.floor(from / PAGE); page * PAGE < end; page++) {
      const k = `${key(node.path)}#${page}`;
      let got = this.pages.get(k);
      if (got === undefined) {
        const read = this.source.children(node.path, page * PAGE, Math.min(node.child_count, (page + 1) * PAGE));
        if (read === null) {
          this.pending = true;
          return null;
        }
        got = read;
        this.pages.set(k, got);
      }
      const base = page * PAGE;
      for (let i = Math.max(from, base); i < Math.min(end, base + got.length); i++) out.push(got[i - base] as IcicleField);
    }
    return out;
  }

  /** One child, for the halving search. Read on its own rather than by page,
   *  so finding a window in a quarter of a million children reads eighteen
   *  children and not eighteen pages. */
  at(node: IcicleField, i: number): IcicleField | null {
    const page = this.pages.get(`${key(node.path)}#${Math.floor(i / PAGE)}`);
    if (page !== undefined) return page[i % PAGE] ?? null;
    const k = `${key(node.path)}@${i}`;
    let got = this.singles.get(k);
    if (got === undefined) {
      const read = this.source.children(node.path, i, i + 1);
      if (read === null) {
        this.pending = true;
        return null;
      }
      const first = read[0];
      if (first === undefined) return null;
      got = first;
      this.singles.set(k, got);
    }
    return got;
  }
}

/** A stretch of children too many or too narrow to draw one by one. */
export type IcicleFold = { readonly count: number; readonly startBit: number; readonly endBit: number };

/** The children of `node` worth laying out between `fromBit` and `toBit`. */
export type VisibleKids = {
  /** Children to draw, in file order, with `contents` children replaced by
   *  their own. */
  readonly fields: readonly IcicleField[];
  /** A window with more children in it than are laid out at once. */
  readonly fold: IcicleFold | null;
  /** True when every child inside the window was read, so a stretch no child
   *  covers really is bytes nothing describes. */
  readonly complete: boolean;
  /** Children left out because their offsets are not in order and only the
   *  first `ALL_MAX` were read. */
  readonly unread: number;
};

const endOf = (f: IcicleField): number => f.offset_bits + f.size_bits;

/** Whether a child belongs on its parent's row: it takes up bytes, and they
 *  are bytes of the same address space. */
function placed(parent: IcicleField, f: IcicleField): boolean {
  return f.size_bits > 0 && f.space === parent.space;
}

/**
 * `kids` with each `contents` child replaced by its own children, as deep as
 * that goes. A contents child whose children are not read yet stays as
 * itself, and one with more than `ALL_MAX` children stays as itself too: its
 * children are read by window when its own bar is drawn.
 */
function hoist(cache: IcicleCache, parent: IcicleField, kids: readonly IcicleField[], depth = 0): IcicleField[] {
  const out: IcicleField[] = [];
  for (const k of kids) {
    if (!placed(parent, k)) continue;
    if (k.contents && k.composite && k.child_count > 0 && k.child_count <= ALL_MAX && depth < 8) {
      const inner = cache.range(k, 0, k.child_count);
      if (inner !== null) {
        out.push(...hoist(cache, k, inner, depth + 1));
        continue;
      }
    }
    out.push(k);
  }
  return out;
}

/**
 * The children of `node` that overlap `fromBit` to `toBit`, or null while
 * they are being read. A node with few children is read whole. One with many
 * is searched by offset for the window, which only works while the children
 * are in file order end to end, as an array is; the first, middle and last
 * are checked for that, and a node that fails the check has its first
 * `ALL_MAX` children drawn and says how many it left out.
 */
export function visibleKids(cache: IcicleCache, node: IcicleField, fromBit: number, toBit: number): VisibleKids | null {
  const n = node.child_count;
  if (n <= ALL_MAX) {
    const all = cache.range(node, 0, n);
    if (all === null) return null;
    return { fields: hoist(cache, node, all), fold: null, complete: true, unread: 0 };
  }
  const first = cache.at(node, 0);
  const mid = cache.at(node, n >> 1);
  const last = cache.at(node, n - 1);
  if (first === null || mid === null || last === null) return null;
  const ordered = first.offset_bits <= mid.offset_bits && mid.offset_bits <= last.offset_bits;
  if (!ordered) {
    const some = cache.range(node, 0, ALL_MAX);
    if (some === null) return null;
    return { fields: hoist(cache, node, some), fold: null, complete: false, unread: n - ALL_MAX };
  }
  // The first child that ends after the window starts, and the first that
  // starts at or after the window ends.
  const search = (test: (f: IcicleField) => boolean): number | null => {
    let lo = 0;
    let hi = n;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      const f = cache.at(node, m);
      if (f === null) return null;
      if (test(f)) hi = m;
      else lo = m + 1;
    }
    return lo;
  };
  const lo = search((f) => endOf(f) > fromBit);
  const hi = search((f) => f.offset_bits >= toBit);
  if (lo === null || hi === null) return null;
  if (hi <= lo) return { fields: [], fold: null, complete: true, unread: 0 };
  if (hi - lo > WINDOW_MAX) {
    const a = cache.at(node, lo);
    const b = cache.at(node, hi - 1);
    if (a === null || b === null) return null;
    return { fields: [], fold: { count: hi - lo, startBit: a.offset_bits, endBit: endOf(b) }, complete: true, unread: 0 };
  }
  const some = cache.range(node, lo, hi);
  if (some === null) return null;
  return { fields: hoist(cache, node, some), fold: null, complete: true, unread: 0 };
}

/** One drawn bar. `x0` and `x1` are screen pixels and may run past either
 *  edge; the drawing clips. */
export type IcicleBar = {
  readonly kind: "field" | "fold" | "gap";
  readonly depth: number;
  /** Which lane of its row, and how many lanes its siblings needed: children
   *  that overlap in the file (a union, a field read twice) split the row. */
  readonly lane: number;
  readonly lanes: number;
  readonly x0: number;
  readonly x1: number;
  readonly startBit: number;
  readonly endBit: number;
  /** The field, for a field bar. */
  readonly field?: IcicleField;
  /** Whose children a fold or a gap is among. */
  readonly parent?: IcicleField;
  /** How many children a fold stands for. */
  readonly count?: number;
  /** A field with children not drawn: too narrow to read them yet, below the
   *  last row, or still being read. */
  readonly more?: boolean;
  /** Children left out of a node whose children are not in file order. */
  readonly unread?: number;
};

export type IcicleView = {
  /** The bits the width shows, end exclusive. */
  readonly startBit: number;
  readonly endBit: number;
  readonly width: number;
  /** How many rows there is room for. */
  readonly rows: number;
  /** The stretch row 0 stands for. Bytes in it the root does not cover (a
   *  template that stops before the file ends) are a gap bar. Defaults to the
   *  root's own bytes. */
  readonly frame?: { readonly startBit: number; readonly endBit: number };
};

export type IcicleLayout = {
  /** Bars by row, each row sorted by where its bars start. */
  readonly rows: readonly (readonly IcicleBar[])[];
  /** Some reads answered "not yet": lay out again when the document changes. */
  readonly pending: boolean;
};

/** Lay out the tree under `root` for `view`. */
export function layoutIcicle(root: IcicleField, cache: IcicleCache, view: IcicleView): IcicleLayout {
  const span = Math.max(1e-9, view.endBit - view.startBit);
  const per = view.width / span;
  const px = (bit: number): number => (bit - view.startBit) * per;
  const rows: IcicleBar[][] = Array.from({ length: Math.max(1, view.rows) }, () => []);
  let pending = false;

  const onScreen = (x0: number, x1: number): boolean => x1 > 0 && x0 < view.width;

  const gapsIn = (parent: IcicleField | undefined, from: number, to: number, covered: readonly { s: number; e: number }[], depth: number): void => {
    const sorted = [...covered].sort((a, b) => a.s - b.s);
    let at = from;
    const emit = (s: number, e: number): void => {
      const x0 = px(s);
      const x1 = px(e);
      if (x1 - x0 < 1 || !onScreen(x0, x1)) return;
      rows[depth]?.push({ kind: "gap", depth, lane: 0, lanes: 1, x0, x1, startBit: s, endBit: e, ...(parent === undefined ? {} : { parent }) });
    };
    for (const c of sorted) {
      if (c.s > at) emit(at, Math.min(c.s, to));
      at = Math.max(at, c.e);
      if (at >= to) break;
    }
    if (at < to) emit(at, to);
  };

  const walk = (node: IcicleField, depth: number, lane: number, lanes: number): void => {
    const x0 = px(node.offset_bits);
    const x1 = px(endOf(node));
    if (!onScreen(x0, x1)) return;
    const has = node.composite && node.child_count > 0;
    const bar: { -readonly [K in keyof IcicleBar]: IcicleBar[K] } = {
      kind: "field",
      depth,
      lane,
      lanes,
      x0,
      x1,
      startBit: node.offset_bits,
      endBit: endOf(node),
      field: node,
    };
    rows[depth]?.push(bar);
    if (!has) return;
    if (depth + 1 >= rows.length || x1 - x0 < LOD.childrenPx) {
      bar.more = true;
      return;
    }
    // Only the part of the node that is on screen.
    const from = Math.max(node.offset_bits, view.startBit);
    const to = Math.min(endOf(node), view.endBit);
    const kids = visibleKids(cache, node, from, to);
    if (kids === null) {
      pending = true;
      bar.more = true;
      return;
    }
    if (kids.unread > 0) bar.unread = kids.unread;
    const d = depth + 1;
    const covered: { s: number; e: number }[] = [];
    if (kids.fold !== null) {
      const f = kids.fold;
      rows[d]?.push({ kind: "fold", depth: d, lane: 0, lanes: 1, x0: px(f.startBit), x1: px(f.endBit), startBit: f.startBit, endBit: f.endBit, parent: node, count: f.count });
      covered.push({ s: f.startBit, e: f.endBit });
    }
    const fields = kids.fields.filter((f) => onScreen(px(f.offset_bits), px(endOf(f)))).sort((a, b) => a.offset_bits - b.offset_bits);
    for (const f of fields) covered.push({ s: f.offset_bits, e: endOf(f) });
    const { lane: laneOf, count } = assignLanes(fields, (f) => ({ start: f.offset_bits, end: endOf(f) }));
    if (count > 1) {
      // Siblings that overlap are drawn in lanes and never folded: a fold is
      // a run of neighbours, and overlapping fields are not one.
      fields.forEach((f, i) => walk(f, d, laneOf[i] ?? 0, count));
    } else {
      let run: IcicleField[] = [];
      const flush = (): void => {
        if (run.length >= 2) {
          const s = (run[0] as IcicleField).offset_bits;
          const e = endOf(run[run.length - 1] as IcicleField);
          rows[d]?.push({ kind: "fold", depth: d, lane: 0, lanes: 1, x0: px(s), x1: px(e), startBit: s, endBit: e, parent: node, count: run.length });
        } else if (run.length === 1) walk(run[0] as IcicleField, d, 0, 1);
        run = [];
      };
      for (const f of fields) {
        const fx0 = px(f.offset_bits);
        const narrow = px(endOf(f)) - fx0 < FOLD_PX;
        const last = run[run.length - 1];
        const touching = last === undefined || fx0 - px(endOf(last)) < FOLD_PX;
        if (narrow && touching) {
          run.push(f);
          continue;
        }
        flush();
        if (narrow) run.push(f);
        else walk(f, d, 0, 1);
      }
      flush();
    }
    // A hole is only a hole when every child inside it was read, and a framed
    // node's leftover bytes are its own punctuation rather than a hole.
    if (kids.complete && !node.framed) gapsIn(node, from, to, covered, d);
  };

  walk(root, 0, 0, 1);
  const frame = view.frame;
  if (frame !== undefined) {
    gapsIn(undefined, Math.max(frame.startBit, view.startBit), Math.min(frame.endBit, view.endBit), [{ s: root.offset_bits, e: endOf(root) }], 0);
  }
  for (const row of rows) row.sort((a, b) => a.x0 - b.x0 || a.lane - b.lane);
  return { rows, pending: pending || cache.pending };
}

/**
 * The bar under a pointer at (`x`, `y`) when each row is `rowH` pixels high,
 * or null. Every bar is at least `LOD.targetPx` wide to the pointer, so a
 * one-pixel field is still something to point at.
 */
export function barAt(layout: IcicleLayout, x: number, y: number, rowH: number): IcicleBar | null {
  if (y < 0) return null;
  const row = layout.rows[Math.floor(y / rowH)];
  if (row === undefined) return null;
  const within = y - Math.floor(y / rowH) * rowH;
  // Fields and folds first, gaps only where neither is in reach. A gap is
  // usually what surrounds a narrow field, and a pointer a few pixels off a
  // one-byte field means the field, not the unmapped bytes beside it.
  const things: AxisTarget<IcicleBar>[] = [];
  const gaps: AxisTarget<IcicleBar>[] = [];
  for (const b of row) {
    const h = rowH / b.lanes;
    if (within < b.lane * h || within >= (b.lane + 1) * h) continue;
    (b.kind === "gap" ? gaps : things).push({ start: b.x0, end: Math.max(b.x1, b.x0 + 1), value: b });
  }
  return nearestOnAxis(things, x)?.value ?? nearestOnAxis(gaps, x)?.value ?? null;
}

/**
 * The bar that stands for the field at `path`: the field's own bar, or the
 * deepest drawn bar the field is inside (a fold it is folded into, a parent
 * too narrow to have its children drawn). Null when nothing on screen holds
 * it.
 */
export function barFor(layout: IcicleLayout, path: readonly number[], startBit: number): IcicleBar | null {
  let best: IcicleBar | null = null;
  let bestLen = -1;
  for (const row of layout.rows) {
    for (const b of row) {
      if (b.kind === "field" && b.field !== undefined) {
        const p = b.field.path;
        if (p.length <= path.length && p.length > bestLen && p.every((s, i) => s === path[i])) {
          best = b;
          bestLen = p.length;
        }
      } else if (b.kind === "fold" && b.parent !== undefined && startBit >= b.startBit && startBit < b.endBit) {
        const p = b.parent.path;
        if (p.length < path.length && p.length + 0.5 > bestLen && p.every((s, i) => s === path[i])) {
          best = b;
          bestLen = p.length + 0.5;
        }
      }
    }
  }
  // An ancestor whose children are drawn stands for none of them: if the
  // field is not among them it is off screen, and outlining the ancestor
  // would point at the wrong bytes.
  if (best !== null && best.kind === "field" && best.field !== undefined && best.field.path.length < path.length && best.more !== true) return null;
  return best;
}

/**
 * What a bar is called on the picture. A template that names its parts is
 * left alone; a part whose name is only its index (`[3]`) gets its type after
 * it, without the brackets that only mean "a list of these", so 256 palette
 * entries are `[0] PaletteEntry` and not a row of bare numbers.
 */
export function fieldLabel(f: { readonly name: string; readonly type: string }): string {
  if (!/^\[\d+\]$/.test(f.name) || f.type === "") return f.name;
  const type = f.type.replace(/\[\]$/, "");
  return type === "" ? f.name : `${f.name} ${type}`;
}

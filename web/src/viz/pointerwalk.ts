// Finding every pointer in a file: the offsets, lengths and counts the
// template says one field holds about another, for the pointer arcs
// (`pointerarcs.ts`) to draw.
//
// The core already knows each of these. `Doc.origins` answers, for one field,
// which other fields settled where it is and how long it is, by path, and
// what each holds. This walks the template tree breadth-first, asks every
// field, and keeps the answers as arcs. It reads no format of its own: a PDF
// object's arc from its xref entry, a ZIP local header's from the central
// directory, a PNG chunk's from its length are all the same answer from the
// core.
//
// Why not `Doc.graph`, which answers for a whole subtree at once: it caps the
// fields it returns breadth-first and drops every edge whose far end fell past
// the cap. On a seven-kilobyte PDF two thousand nodes go on one object's
// tokens before the walk reaches the xref entries' offset fields, so the
// graph has the trailer's arcs and none of the xref's. Asking field by field
// costs about fifteen microseconds a field and is never cut short in the
// middle of a table.
//
// The walk is incremental. `step` does as much as fits before a deadline and
// says whether it finished, ran out of time, or is waiting on bytes; the
// caller decides when to call it again. It stops at `maxFields` and says so,
// so a file of a million records costs a bounded walk and the picture says
// how much of the file it covers.
//
// Only the root's own address space is walked (the file's, unless the
// document is a stream opened on its own). A field inside a decoded stream
// has an offset counted from the front of what the stream unpacked to, which
// is no place on the file's axis.

import type { Origin, TemplateNode, TemplateReply } from "../doc.ts";
import type { ArcEnd, PointerArc } from "./arcmath.ts";

/** The part of `Doc` the walk reads. A test passes a small fake. */
export type WalkDoc = {
  templateNode(path: readonly number[]): TemplateReply<TemplateNode>;
  templateChildren(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]>;
  origins(path: readonly number[]): TemplateReply<Origin[]>;
};

/** Children asked for at once. */
const PAGE = 512;
/** Fields the walk stops at unless told otherwise. */
export const MAX_FIELDS = 300_000;

export type WalkState = "done" | "more" | "pending";

/** A structure whose children are still to be read, with the name the walk
 *  gives its children. */
type Open = { readonly node: TemplateNode; readonly label: string; next: number };

/** A deciding field whose node was not readable yet when its arc was found. */
type Unresolved = { readonly origin: Origin; readonly target: ArcEnd };

/** How a field is named in the hover card: steps from the root joined by
 *  dots, list elements by index, the way a programmer writes a path into a
 *  structure (`records[4].local_header_offset`). Both ends of an arc are
 *  named this way, from the tree, since the core's `Origin.label` names a
 *  field from wherever the asking field is, and two arcs from the same kind
 *  of field would read differently. */
export function childLabel(parent: TemplateNode, parentLabel: string, child: TemplateNode): string {
  const index = child.path[child.path.length - 1] ?? 0;
  if (parent.list) return `${parentLabel}[${index}]`;
  // A structure that is only its parent's contents (a ZIP entry's `body`),
  // and the one field a field placed elsewhere wraps, which has the same
  // name, add a step the reader never sees in the listing.
  if (child.contents || child.name === parent.name || (isWrapper(parent) && child.name.startsWith(`${parent.name} `))) return parentLabel;
  return parentLabel === "" ? child.name : `${parentLabel}.${child.name}`;
}

/** A field placed somewhere else is a wrapper with no bytes holding the field
 *  that has them. The wrapper's offset is where it would have been, not where
 *  its bytes are, so it is no end for an arc. */
function isWrapper(n: TemplateNode): boolean {
  return n.composite && n.size_bits === 0 && n.child_count > 0;
}

function pathKey(path: readonly number[]): string {
  return path.join(".");
}

export class PointerWalk {
  /** Every arc found so far, in the order found. */
  readonly arcs: PointerArc[] = [];
  /** Fields asked about so far. */
  visited = 0;
  /** True when the walk stopped at `maxFields` with fields left. */
  capped = false;
  private readonly open: Open[] = [];
  private head = 0;
  private started = false;
  /** Each arc's index in `arcs`, by where it leaves and lands. */
  private readonly seen = new Map<string, number>();
  private readonly sources = new Map<string, TemplateNode | null>();
  /** The name of every field visited, by path, for the far ends of arcs. */
  private readonly labels = new Map<string, string>();
  private unresolved: Unresolved[] = [];
  private finished = false;
  /** The address space of the root, the one the axis is drawn in. */
  private space = 0;

  private readonly doc: WalkDoc;
  private readonly maxFields: number;

  constructor(doc: WalkDoc, maxFields = MAX_FIELDS) {
    this.doc = doc;
    this.maxFields = maxFields;
  }

  get done(): boolean {
    return this.finished;
  }

  /**
   * Walk until `deadline` (in the units of `now`), the end, or a read that
   * has to wait. `now` is passed in so the tests can run without a clock.
   */
  step(deadline: number, now: () => number): WalkState {
    if (this.finished) return "done";
    if (!this.started) {
      const root = this.doc.templateNode([]);
      if (root.status === "pending" || root.status === "working") return "pending";
      if (root.status !== "ok") {
        this.finished = true;
        return "done";
      }
      this.started = true;
      this.space = root.node.space;
      this.visit(root.node, "");
    }
    this.retryUnresolved();
    while (this.head < this.open.length) {
      if (now() >= deadline) return "more";
      if (this.visited >= this.maxFields) {
        this.capped = true;
        break;
      }
      const o = this.open[this.head] as Open;
      const to = Math.min(o.node.child_count, o.next + PAGE, o.next + (this.maxFields - this.visited));
      const reply = this.doc.templateChildren(o.node.path, o.next, to);
      if (reply.status === "pending" || reply.status === "working") return "pending";
      if (reply.status !== "ok" || reply.node.length === 0) {
        this.head++;
        continue;
      }
      for (const kid of reply.node) this.visit(kid, childLabel(o.node, o.label, kid));
      o.next += reply.node.length;
      if (o.next >= o.node.child_count) this.head++;
    }
    if (this.unresolved.length > 0) return "pending";
    this.finished = true;
    // The structures still open are what the cap left out.
    this.open.length = 0;
    return "done";
  }

  private visit(node: TemplateNode, label: string): void {
    this.visited++;
    if (node.space !== this.space) return;
    this.labels.set(pathKey(node.path), label);
    if (node.child_count > 0 && !node.decoded) this.open.push({ node, label, next: 0 });
    const reply = this.doc.origins(node.path);
    if (reply.status !== "ok") return;
    const target: ArcEnd = { path: node.path, startBit: node.offset_bits, endBit: node.offset_bits + node.size_bits, label };
    for (const o of reply.node) {
      if (o.role === "points") {
        // This field holds the offset; the core says the bit it lands on.
        if (o.target_bits === null || node.size_bits === 0 || isWrapper(node)) continue;
        this.add("position", target, { path: [], startBit: o.target_bits, endBit: o.target_bits, label: o.label }, o.value);
        continue;
      }
      if (o.path.length === 0 || isWrapper(node)) continue;
      this.resolve({ origin: o, target });
    }
  }

  /** Turn an origin into an arc once its deciding field can be read. */
  private resolve(u: Unresolved): boolean {
    const key = pathKey(u.origin.path);
    let src = this.sources.get(key);
    if (src === undefined) {
      const r = this.doc.templateNode(u.origin.path);
      if (r.status === "pending" || r.status === "working") {
        this.unresolved.push(u);
        return false;
      }
      src = r.status === "ok" ? r.node : null;
      this.sources.set(key, src);
    }
    if (src === null || src.space !== this.space || src.size_bits === 0) return true;
    const from: ArcEnd = { path: src.path, startBit: src.offset_bits, endBit: src.offset_bits + src.size_bits, label: this.labelOf(src.path) ?? u.origin.label };
    this.add(u.origin.role, from, u.target, u.origin.value);
    return true;
  }

  /** A field's name the way `childLabel` gives it, worked out from the
   *  fields above it when the walk has not reached it yet. Null when one of
   *  those cannot be read yet. */
  private labelOf(path: readonly number[]): string | null {
    const known = this.labels.get(pathKey(path));
    if (known !== undefined) return known;
    if (path.length === 0) return "";
    const parentPath = path.slice(0, -1);
    const parentLabel = this.labelOf(parentPath);
    const parent = this.nodeAt(parentPath);
    const node = this.nodeAt(path);
    if (parentLabel === null || parent === null || node === null) return null;
    const label = childLabel(parent, parentLabel, node);
    this.labels.set(pathKey(path), label);
    return label;
  }

  private nodeAt(path: readonly number[]): TemplateNode | null {
    const key = pathKey(path);
    const had = this.sources.get(key);
    if (had !== undefined) return had;
    const r = this.doc.templateNode(path);
    if (r.status !== "ok") return null;
    this.sources.set(key, r.node);
    return r.node;
  }

  private retryUnresolved(): void {
    if (this.unresolved.length === 0) return;
    const waiting = this.unresolved;
    this.unresolved = [];
    for (const u of waiting) this.resolve(u);
  }

  private add(role: string, from: ArcEnd, to: ArcEnd, value: string): void {
    // An offset said twice, once as the bit the pointer lands on and once as
    // the field the pointer places, is one arc. The field is the better end:
    // it has a path to go to and a name, so it replaces the bare bit, which
    // the walk usually meets first, since the pointer comes before what it
    // places.
    const key = `${role}|${from.startBit}|${from.endBit}|${to.startBit}`;
    const at = this.seen.get(key);
    if (at !== undefined) {
      const had = this.arcs[at] as PointerArc;
      const better = had.to.path.length === 0 && to.path.length > 0 ? to : had.to;
      this.arcs[at] = { role, from: had.from, to: better, value: had.value !== "" ? had.value : value };
      return;
    }
    this.seen.set(key, this.arcs.length);
    this.arcs.push({ role, from, to, value });
  }
}

/** A top-level part of the file, for the bar the arcs stand on. */
export type FilePart = { readonly path: readonly number[]; readonly name: string; readonly startBit: number; readonly endBit: number };

/** Parts drawn on the bar at most. */
const MAX_PARTS = 4096;

/** What a part is called on the bar: its own name, without the index a list
 *  element's name starts with when it has a name besides (`[2] notes.txt`). */
export function partName(n: TemplateNode): string {
  const m = /^\[\d+\]\s+(.+)$/.exec(n.name);
  return m?.[1] ?? n.name;
}

/**
 * The parts of the file to draw the bar from: the root's children, with a
 * field placed elsewhere replaced by the field it wraps, and any part that
 * takes up half the file or more and is a list, or is the only part (a ZIP's
 * one list of records, a PDF's objects), opened out into its own children,
 * up to three levels down. Null
 * while the tree is still being read.
 */
export function fileParts(doc: WalkDoc, fileBits: number): FilePart[] | null {
  const kids = (n: TemplateNode): TemplateNode[] | null => {
    const r = doc.templateChildren(n.path, 0, Math.min(n.child_count, MAX_PARTS));
    if (r.status === "pending" || r.status === "working") return null;
    return r.status === "ok" ? r.node : [];
  };
  const unwrap = (n: TemplateNode): TemplateNode | null => {
    if (!isWrapper(n) || n.child_count !== 1) return n;
    const k = kids(n);
    return k === null ? null : (k[0] ?? n);
  };
  const root = doc.templateNode([]);
  if (root.status === "pending" || root.status === "working") return null;
  if (root.status !== "ok") return [];
  let parts = kids(root.node);
  if (parts === null) return null;
  for (let round = 0; round < 3; round++) {
    const next: TemplateNode[] = [];
    let opened = false;
    for (const p of parts) {
      const u = unwrap(p);
      if (u === null) return null;
      // A list, or the one part there is: one big record among small ones is
      // a part of its own, not a list to open.
      const big = (u.list || parts.length === 1) && u.composite && u.child_count > 0 && !u.decoded && u.size_bits * 2 >= fileBits;
      if (big && next.length + u.child_count <= MAX_PARTS) {
        const k = kids(u);
        if (k === null) return null;
        for (const c of k) {
          const uc = unwrap(c);
          if (uc === null) return null;
          next.push(uc);
        }
        opened = true;
      } else next.push(u);
    }
    parts = next;
    if (!opened) break;
  }
  return parts
    .filter((p) => p.space === root.node.space && p.size_bits > 0)
    .sort((a, b) => a.offset_bits - b.offset_bits)
    .map((p) => ({ path: p.path, name: partName(p), startBit: p.offset_bits, endBit: p.offset_bits + p.size_bits }));
}

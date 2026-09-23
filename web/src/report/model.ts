// The parts of the file the report talks about, read from the template.
//
// Not the listing's outline. The outline is right for the rail, which wants a
// short list of places to jump to, and too coarse for a report: a JPEG's
// outline is a two-byte header and one heading over everything else, where the
// report wants the scan, the tables, and the headers as parts of their own.
// The rules, from docs/DESIGN-report-view.md:
//
// - A root that is one structure or one list around the whole file is that
//   structure or list: a ZIP's parts are its records, not the one list of them.
// - A run of plain fields next to each other is one part, named by where it
//   sits (Header, Trailer, Fields), as the listing names it.
// - A list of up to `EXPAND_MAX` structures is its elements, grouped by the
//   variant the file chose for each: the switch case or enum value `origins`
//   names as deciding its type. A ZIP's 23 records are 11 local file records,
//   11 central directory records, and 1 end of central directory record.
// - A longer list, or a list of numbers, is one part.
//
// Everything is bounded, so a report on a gigabyte file asks a few hundred
// questions of the core and not one per field.

import type { Doc, Origin, TemplateNode, TemplateReply } from "../doc.ts";
import { sectionColor, UNMAPPED_COLOR } from "../fieldstyle.ts";
import { childWord, REPORT } from "../strings.ts";
import { gapsOf, readingOrder, runPosition, runsOf, stripIndex, variantText, type Extent } from "./partrules.ts";
import { WAIT } from "./section.ts";

/** Children of one node looked at, at most. Past this the rest of the node is
 *  one stretch the report says it did not list. */
const KIDS_MAX = 256;
/** A list of this many structures or fewer is read as its elements. */
const EXPAND_MAX = 256;
/** Composite children of an element asked which field decided their type. */
const VARIANT_PROBES = 3;
/** Children of a part asked what placed them, for a part placed field by field
 *  (a directory's targets). */
const PLACED_PROBES = 16;

/** One part of the file, before grouping. */
export type Unit = {
  readonly key: string;
  /** The node, or the parent of a run of fields. */
  readonly path: readonly number[];
  /** The node itself, or null for a run of plain fields. */
  readonly node: TemplateNode | null;
  /** The fields of a run, in order. Empty for a node. */
  readonly fields: readonly TemplateNode[];
  readonly label: string;
  /** True when the label is a field's name, which is set in code font. */
  readonly named: boolean;
  readonly offsetBits: number;
  readonly sizeBits: number;
  /** The list this is an element of, if it is one. */
  readonly list: TemplateNode | null;
  /** What decided this element's type, when something did. */
  readonly variant: string | null;
  /** True for bytes no field covers. */
  readonly gap: boolean;
};

/** Parts that are the same kind of thing, drawn as one section: the four
 *  Huffman table segments of a JPEG. Most groups are one part. */
export type Group = {
  readonly key: string;
  readonly label: string;
  readonly named: boolean;
  readonly units: readonly Unit[];
  readonly sizeBits: number;
  /** Where the first part starts. */
  readonly offsetBits: number;
  readonly color: string;
  /** What one of the parts is called, for counting them: `segment`. */
  readonly unitWord: string;
  /** Its place in file order, which is the order of the map and the ledger. */
  readonly index: number;
  readonly gap: boolean;
};

/** A list the parts met, for the facts table. */
export type ListFact = { readonly node: TemplateNode; readonly count: number; readonly word: string };

export type PartsModel = {
  readonly fileBits: number;
  /** In file order. */
  readonly groups: readonly Group[];
  /** The order the report reads them in: parts that others place or size
   *  first, the parts that place them after, largest first otherwise. */
  readonly order: readonly Group[];
  readonly lists: readonly ListFact[];
  /** Children past `KIDS_MAX` that were not looked at. */
  readonly unlisted: number;
  /** The node the parts were read from. */
  readonly container: TemplateNode;
};

const pathKey = (p: readonly number[]): string => p.join("/");

/** A reply as the three things a section does with it: use it, wait for it,
 *  or go without. */
export function ok<T>(r: TemplateReply<T>): T | typeof WAIT | null {
  switch (r.status) {
    case "ok":
      return r.node;
    case "pending":
    case "working":
      return WAIT;
    default:
      return null;
  }
}

/** The parts of the file, or `WAIT` while the bytes they need are read. Null
 *  for a file with no template. */
export function buildParts(doc: Doc): PartsModel | typeof WAIT | null {
  if (doc.template === null) return null;
  const fileBits = doc.lengthBits;
  const rootR = ok(doc.templateNode([]));
  if (rootR === WAIT) return WAIT;
  if (rootR === null) return null;
  let container: TemplateNode = rootR;
  // A root that is one wrapper around the whole file is that wrapper.
  for (let depth = 0; depth < 4 && container.composite && !container.list; depth++) {
    const kids = ok(doc.templateChildren(container.path, 0, Math.min(container.child_count, 8)));
    if (kids === WAIT) return WAIT;
    if (kids === null) break;
    const real = kids.filter((k) => k.size_bits > 0 && !k.absent);
    const only = real[0];
    if (real.length !== 1 || only === undefined || !only.composite || only.size_bits < container.size_bits * 0.9) break;
    if (container.child_count > 8) break;
    container = only;
  }
  const units: Unit[] = [];
  const lists: ListFact[] = [];
  let unlisted = 0;
  const kidsR = ok(doc.templateChildren(container.path, 0, Math.min(container.child_count, KIDS_MAX)));
  if (kidsR === WAIT) return WAIT;
  const kids = (kidsR ?? []).filter((k) => !k.absent && k.size_bits > 0 && k.space === 0);
  unlisted = Math.max(0, container.child_count - KIDS_MAX);
  if (container.list) {
    lists.push({ node: container, count: container.child_count, word: childWord(container) });
    for (const k of kids) units.push(elementUnit(k, container));
  } else {
    for (const run of runsOf(kids, (k) => !k.composite || k.inline)) {
      const first = run[0];
      if (first === undefined) continue;
      if (run.length > 1 || !first.composite || first.inline) {
        const last = run[run.length - 1] ?? first;
        const ext = { offsetBits: first.offset_bits, sizeBits: last.offset_bits + last.size_bits - first.offset_bits };
        units.push({
          key: `run:${pathKey(first.path)}`,
          path: container.path,
          node: null,
          fields: run,
          label: REPORT.unnamedPart(runPosition(ext, fileBits)),
          named: false,
          ...ext,
          list: null,
          variant: null,
          gap: false,
        });
        continue;
      }
      if (first.list) {
        lists.push({ node: first, count: first.child_count, word: childWord(first) });
        if (first.child_count > 0 && first.child_count <= EXPAND_MAX) {
          const els = ok(doc.templateChildren(first.path, 0, first.child_count));
          if (els === WAIT) return WAIT;
          const real = (els ?? []).filter((e) => !e.absent && e.size_bits > 0 && e.space === 0);
          if (real.length > 0 && real.every((e) => e.composite)) {
            for (const e of real) units.push(elementUnit(e, first));
            continue;
          }
        }
      }
      units.push(nodeUnit(first));
    }
  }
  // Which case each element took, so like elements can be read as one part.
  for (let i = 0; i < units.length; i++) {
    const u = units[i];
    if (u === undefined || u.list === null || u.node === null) continue;
    const v = variantOf(doc, u.node);
    if (v === WAIT) return WAIT;
    if (v !== null) units[i] = { ...u, variant: v };
  }
  // Bytes no part covers, from the front of the file to the end of it.
  const covered: Extent[] = units.map((u) => ({ offsetBits: u.offsetBits, sizeBits: u.sizeBits }));
  const listedTo = unlisted > 0 ? Math.max(...covered.map((c) => c.offsetBits + c.sizeBits), 0) : fileBits;
  for (const g of gapsOf(covered, 0, listedTo)) units.push(gapUnit(g));
  units.sort((a, b) => a.offsetBits - b.offsetBits);
  const groups = groupUnits(units);
  const order = readingOrder(
    groups.map((g) => g.sizeBits),
    placements(doc, groups),
  ).map((i) => groups[i]).filter((g): g is Group => g !== undefined);
  return { fileBits, groups, order, lists, unlisted, container };
}

function nodeUnit(n: TemplateNode): Unit {
  return {
    key: `node:${pathKey(n.path)}`,
    path: n.path,
    node: n,
    fields: [],
    label: n.name,
    named: true,
    offsetBits: n.offset_bits,
    sizeBits: n.size_bits,
    list: null,
    variant: null,
    gap: false,
  };
}

function elementUnit(n: TemplateNode, list: TemplateNode): Unit {
  const bare = stripIndex(n.name);
  const index = n.path[n.path.length - 1] ?? 0;
  return {
    ...nodeUnit(n),
    label: bare === "" ? `${list.name}[${index}]` : bare,
    named: bare === "",
    list,
  };
}

function gapUnit(g: Extent): Unit {
  return {
    key: `gap:${g.offsetBits}`,
    path: [],
    node: null,
    fields: [],
    label: REPORT.gap,
    named: false,
    ...g,
    list: null,
    variant: null,
    gap: true,
  };
}

/** What decided an element's type: the value `origins` names with the role
 *  `type`, on the element or on one of its first structures. A ZIP record's
 *  body is decided by its signature, a JPEG segment's by its marker. */
function variantOf(doc: Doc, n: TemplateNode): string | null | typeof WAIT {
  const own = ok(doc.origins(n.path));
  if (own === WAIT) return WAIT;
  const direct = (own ?? []).find((o) => o.role === "type");
  if (direct !== undefined && direct.value !== "") return variantText(direct.value);
  const kids = ok(doc.templateChildren(n.path, 0, Math.min(n.child_count, 8)));
  if (kids === WAIT) return WAIT;
  for (const k of (kids ?? []).filter((c) => c.composite).slice(0, VARIANT_PROBES)) {
    const o = ok(doc.origins(k.path));
    if (o === WAIT) return WAIT;
    const t = (o ?? []).find((x) => x.role === "type");
    if (t !== undefined && t.value !== "") return variantText(t.value);
  }
  return null;
}

/** Parts of one list that took the same case go together; everything else is
 *  a group of one. Colours follow file order, one hue per group, from the same
 *  palette the listing's sections use. */
function groupUnits(units: readonly Unit[]): Group[] {
  const byKey = new Map<string, Unit[]>();
  const order: string[] = [];
  for (const u of units) {
    const key = u.list !== null && u.variant !== null ? `v:${pathKey(u.list.path)}:${u.variant}` : u.key;
    let g = byKey.get(key);
    if (g === undefined) {
      g = [];
      byKey.set(key, g);
      order.push(key);
    }
    g.push(u);
  }
  let hue = 0;
  return order.map((key, index) => {
    const us = byKey.get(key) ?? [];
    const first = us[0] as Unit;
    const gap = first.gap;
    const many = us.length > 1;
    return {
      key,
      label: many ? (first.variant ?? first.label) : first.label,
      named: many ? false : first.named,
      units: us,
      sizeBits: us.reduce((s, u) => s + u.sizeBits, 0),
      offsetBits: first.offsetBits,
      color: gap ? UNMAPPED_COLOR : sectionColor(hue++),
      unitWord: first.list !== null ? childWord(first.list) : "part",
      index,
      gap,
    };
  });
}

/** Which groups each group places or sizes, from what `origins` says decided
 *  the length, count, or position of its parts and of their first fields. */
function placements(doc: Doc, groups: readonly Group[]): number[][] {
  const out: Set<number>[] = groups.map(() => new Set());
  const unitOf = new Map<string, number>();
  groups.forEach((g, gi) => {
    for (const u of g.units) if (u.node !== null) unitOf.set(pathKey(u.path), gi);
  });
  /** The group a field is in: the nearest ancestor that is a part. */
  const groupOf = (path: readonly number[]): number | null => {
    for (let i = path.length; i >= 0; i--) {
      const hit = unitOf.get(pathKey(path.slice(0, i)));
      if (hit !== undefined) return hit;
    }
    // A field of a run of plain fields: the run's fields are not parts of
    // their own, so find it by where it sits among them.
    for (let gi = 0; gi < groups.length; gi++) {
      for (const u of groups[gi]?.units ?? []) if (u.fields.some((f) => pathKey(f.path) === pathKey(path))) return gi;
    }
    return null;
  };
  const note = (placed: number, o: Origin): void => {
    if (o.role !== "length" && o.role !== "count" && o.role !== "position") return;
    if (o.path.length === 0) return;
    const placer = groupOf(o.path);
    if (placer !== null && placer !== placed) out[placer]?.add(placed);
  };
  const listsAsked = new Set<string>();
  groups.forEach((g, gi) => {
    for (const u of g.units) {
      if (u.node === null) continue;
      // What sizes or counts a list sizes every element of it: a RIFF size
      // over the chunks, a count over the records.
      if (u.list !== null && !listsAsked.has(pathKey(u.list.path))) {
        listsAsked.add(pathKey(u.list.path));
        const whole = doc.origins(u.list.path);
        if (whole.status === "ok") for (const o of whole.node) for (const other of groupsOfList(groups, u.list)) note(other, o);
      }
      const own = doc.origins(u.path);
      if (own.status === "ok") for (const o of own.node) note(gi, o);
      if (!u.node.composite || u.node.list) continue;
      const kids = doc.templateChildren(u.path, 0, Math.min(u.node.child_count, PLACED_PROBES));
      if (kids.status !== "ok") continue;
      for (const k of kids.node) {
        if (!k.composite) continue;
        const r = doc.origins(k.path);
        if (r.status === "ok") for (const o of r.node) if (o.role === "position") note(gi, o);
      }
    }
  });
  return out.map((s) => [...s]);
}

/** The groups holding the elements of one list. */
function groupsOfList(groups: readonly Group[], list: TemplateNode): number[] {
  const key = pathKey(list.path);
  const out: number[] = [];
  groups.forEach((g, gi) => {
    if (g.units.some((u) => u.list !== null && pathKey(u.list.path) === key)) out.push(gi);
  });
  return out;
}

/** The group a bit of the file is in, or null. */
export function groupAt(model: PartsModel, bit: number): Group | null {
  for (const g of model.groups) {
    for (const u of g.units) if (bit >= u.offsetBits && bit < u.offsetBits + u.sizeBits) return g;
  }
  return null;
}

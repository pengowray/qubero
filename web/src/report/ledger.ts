// The core's byte ledger, read for the report: every bit of the file in one
// row, keyed by part, by the variant of the nearest list element, and by role.
// See "The byte ledger" in docs/DESIGN-report-view.md.
//
// The report shows it twice. Section 6 has one row per part and group, with
// its share of the file; section 12 has every role under each of those. The
// grouping here is the part that can be wrong, so it has no DOM in it and the
// tests run it.

import type { TemplateNode } from "../doc.ts";
import type { LedgerRole, LedgerRow } from "./coredata.ts";
import { stripIndex } from "./partrules.ts";

/** One line of the ledger as the report shows it: a part, or one group of the
 *  elements of a list inside a part. */
export type LedgerLine = {
  readonly key: string;
  /** The part's name, or null for the fields of a structure taken together. */
  readonly part: string | null;
  /** What the fields taken together belong to, for a line of plain fields. */
  readonly parentPath: readonly number[] | null;
  /** The group's name, "" where there is none. */
  readonly group: string;
  /** True when the group is named by a type in the template rather than by a
   *  value in the file: `TableLeaf`, which the report writes as words. */
  readonly byType: boolean;
  /** The part the group is in, and whether the report names it after the
   *  group: only where the grouped lines are in more than one part. A group
   *  in the one list the file is, or in the only list with groups, is named
   *  by the group alone, since "in <list>" would say nothing. */
  readonly partPath: readonly number[];
  readonly partShown: boolean;
  /** The names of the fields taken together, for a line of plain fields. */
  readonly fields: readonly string[];
  readonly bits: number;
  readonly firstOffsetBits: number;
  readonly firstPath: readonly number[];
  /** The line's bits by role, largest first. */
  readonly roles: readonly RoleShare[];
};

export type RoleShare = {
  readonly role: LedgerRole;
  readonly bits: number;
  /** For padding and gaps: bits in zero bytes, and bits not read. */
  readonly zeroBits: number;
  readonly unscannedBits: number;
  /** For padding: the alignments it pads to. */
  readonly aligns: readonly number[];
};

const samePath = (a: readonly number[], b: readonly number[]): boolean => a.length === b.length && a.every((x, i) => x === b[i]);

/** Paths in the order the template lists them, for parts that start at the
 *  same bit. */
function comparePaths(a: readonly number[], b: readonly number[]): number {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d !== 0) return d;
  }
  return a.length - b.length;
}

/** Whether a row is the bits of one plain value that is a whole part: not a
 *  group of list elements, not a gap or padding, and not a stream. The walk
 *  counts a stream (a `Decoded` or `Stitched` field) as one leaf of the file
 *  too, so its row's `first_path` is the part's own path, as for a plain
 *  value. But a stream has contents of its own: a firmware update's scrambled
 *  image is `image`, not one of the fields of a trailer. */
function plainRow(r: LedgerRow): boolean {
  return r.group === "" && r.role !== "gap" && r.role !== "padding" && r.stream !== true && samePath(r.first_path, r.part);
}

/**
 * The line each part that is one plain field goes on, by the part's path.
 *
 * A line is a run of plain fields of one structure that follow each other in
 * the file with nothing between them, and it is keyed by the run's first
 * part. Any other part ends the run: a structure, a list, a stream or a
 * padding field. So does a gap, since then the next field does not start
 * where the run ends. A zlib stream's header fields are one line, and its
 * `adler32`, after the compressed data, is a line of its own. The report's
 * map divides a file into the same runs (`unitsOf` in `model.ts`), and both
 * name a run of more than one field by where it is in the file.
 *
 * A part is a plain field only when every row it has, other than gaps, is
 * one: a JSON object's braces and commas are a row of its own whose
 * `first_path` is the object, and they belong on the object's line.
 */
function plainRuns(rows: readonly LedgerRow[]): Map<string, string> {
  type Part = { path: readonly number[]; plain: boolean; start: number; bits: number };
  const parts = new Map<string, Part>();
  for (const r of rows) {
    if (r.role === "gap") continue;
    const k = r.part.join("/");
    let p = parts.get(k);
    if (p === undefined) {
      p = { path: r.part, plain: true, start: r.first_offset_bits, bits: 0 };
      parts.set(k, p);
    }
    p.start = Math.min(p.start, r.first_offset_bits);
    p.bits += r.bits;
    if (!plainRow(r)) p.plain = false;
  }
  const order = [...parts.values()].sort((a, b) => a.start - b.start || comparePaths(a.path, b.path));
  const lineOf = new Map<string, string>();
  let run: { key: string; parent: string; end: number } | null = null;
  for (const p of order) {
    if (!p.plain) {
      run = null;
      continue;
    }
    const parent = p.path.slice(0, -1).join("/");
    if (run === null || run.parent !== parent || run.end !== p.start) run = { key: `fields:${p.path.join("/")}`, parent, end: p.start };
    run.end = p.start + p.bits;
    lineOf.set(p.path.join("/"), run.key);
  }
  return lineOf;
}

/**
 * The ledger's rows as the report's lines, in file order.
 *
 * - A part with groups under it is a line per group: a ZIP's records are the
 *   local files, the central directory and its end.
 * - Parts that are one plain field each are taken together, a run of them
 *   that are next to each other in the file to a line: the thirty fields of
 *   an ELF header are one line, not thirty. See `plainRuns`. One such field
 *   on its own keeps its own name.
 * - Bytes no field describes are one line, and padding another.
 */
export function ledgerLines(rows: readonly LedgerRow[]): LedgerLine[] {
  type Acc = {
    key: string;
    part: string | null;
    parentPath: readonly number[] | null;
    group: string;
    fields: string[];
    bits: number;
    firstOffsetBits: number;
    firstPath: readonly number[];
    partPath: readonly number[];
    /** The group is named by the structure's type, not by a value. */
    byType: boolean;
    /** The group's name is a type's, whether the list's one type or the
     *  type of the case a switch took. */
    typeNamed: boolean;
    roles: Map<LedgerRole, RoleAcc>;
  };
  const lines = new Map<string, Acc>();
  const runs = plainRuns(rows);
  for (const r of rows) {
    const run = r.role === "gap" || r.role === "padding" ? undefined : runs.get(r.part.join("/"));
    const plain = run !== undefined;
    const key = r.role === "gap" ? "gap" : r.role === "padding" ? "padding" : (run ?? `part:${r.part.join("/")}\u0000${r.group}`);
    let a = lines.get(key);
    if (a === undefined) {
      a = {
        key,
        part: plain || r.role === "gap" || r.role === "padding" ? null : r.part_name,
        parentPath: plain ? r.part.slice(0, -1) : null,
        group: plain ? "" : r.group,
        fields: [],
        bits: 0,
        firstOffsetBits: r.first_offset_bits,
        firstPath: r.first_path,
        partPath: r.part,
        byType: r.group_from === "type",
        typeNamed: r.group_from === "type" || r.group_from === "case",
        roles: new Map(),
      };
      lines.set(key, a);
    }
    if (plain && !a.fields.includes(r.part_name)) a.fields.push(r.part_name);
    a.bits += r.bits;
    if (r.first_offset_bits < a.firstOffsetBits) {
      a.firstOffsetBits = r.first_offset_bits;
      a.firstPath = r.first_path;
    }
    const role: RoleAcc = a.roles.get(r.role) ?? { bits: 0, zero: 0, unscanned: 0, aligns: new Set<number>() };
    role.bits += r.bits;
    role.zero += r.zero_bits;
    role.unscanned += r.unscanned_bits;
    if (r.align > 0) role.aligns.add(r.align);
    a.roles.set(r.role, role);
  }
  // A structure inside another group's element that is named only by its
  // type is part of that group's line: a JPEG dht segment's Huffman tables,
  // a ZIP local file's extra fields. The line then says how much of the file
  // the segments take. The inner group's first field lies in the element
  // where the outer group's first field is, further in. A group named by a
  // value in the file stays a line of its own, since the value says
  // something: a MIDI track's note-on events, a JPEG table's precision.
  const owners = new Map<string, Acc>();
  const nested: Acc[] = [];
  const byDepth = [...lines.values()].filter((a) => a.key.startsWith("part:")).sort((x, y) => x.firstPath.length - y.firstPath.length);
  for (const a of byDepth) {
    const element = `${a.partPath.join("/")}|${a.firstPath.slice(0, a.partPath.length + 1).join("/")}`;
    const owner = owners.get(element);
    if (owner === undefined) owners.set(element, a);
    else if (a.byType && owner.firstPath.length < a.firstPath.length) {
      mergeInto(owner, a);
      nested.push(a);
    }
  }
  for (const a of nested) lines.delete(a.key);
  // The parts that hold groups. Only where there are two or more does a
  // group's line say which part it is in: a MIDI file's events are all in the
  // one list the file is, and "meta in file" would read as "in the file".
  const grouped = new Set([...lines.values()].filter((a) => a.group !== "").map((a) => a.partPath.join("/")));
  return [...lines.values()]
    .map((a) => ({
      key: a.key,
      // One plain field on its own is that field.
      part: a.part ?? (a.fields.length === 1 ? (a.fields[0] ?? null) : null),
      parentPath: a.fields.length === 1 ? null : a.parentPath,
      group: a.group,
      byType: a.group !== "" && a.typeNamed,
      partPath: a.partPath,
      partShown: a.group !== "" && a.partPath.length > 0 && grouped.size > 1,
      fields: a.fields,
      bits: a.bits,
      firstOffsetBits: a.firstOffsetBits,
      firstPath: a.firstPath,
      roles: [...a.roles.entries()]
        .map(([role, v]) => ({ role, bits: v.bits, zeroBits: v.zero, unscannedBits: v.unscanned, aligns: [...v.aligns].sort((x, y) => x - y) }))
        .sort((x, y) => y.bits - x.bits),
    }))
    .sort((x, y) => x.firstOffsetBits - y.firstOffsetBits);
}

type RoleAcc = { bits: number; zero: number; unscanned: number; aligns: Set<number> };
type LineAcc = { bits: number; firstOffsetBits: number; firstPath: readonly number[]; roles: Map<LedgerRole, RoleAcc> };

function mergeInto(owner: LineAcc, a: LineAcc): void {
  owner.bits += a.bits;
  if (a.firstOffsetBits < owner.firstOffsetBits) {
    owner.firstOffsetBits = a.firstOffsetBits;
    owner.firstPath = a.firstPath;
  }
  for (const [role, v] of a.roles) {
    const o = owner.roles.get(role) ?? { bits: 0, zero: 0, unscanned: 0, aligns: new Set<number>() };
    o.bits += v.bits;
    o.zero += v.zero;
    o.unscanned += v.unscanned;
    for (const x of v.aligns) o.aligns.add(x);
    owner.roles.set(role, o);
  }
}

/** True for the line of bytes no field describes. */
export function isGapLine(l: LedgerLine): boolean {
  return l.key === "gap";
}

/**
 * A part's fields as rows: one that covers the whole part would repeat it and
 * is left out, and neighbours of the same name are one row with a count, so
 * four dht segments in a row are `dht × 4`.
 */
export function fieldRows(u: { offsetBits: number; sizeBits: number }, kids: readonly Pick<TemplateNode, "name" | "path" | "offset_bits" | "size_bits">[]): { name: string; path: readonly number[]; startBit: number; endBit: number; count: number }[] {
  const out: { name: string; path: readonly number[]; startBit: number; endBit: number; count: number }[] = [];
  for (const k of kids) {
    if (k.offset_bits === u.offsetBits && k.size_bits === u.sizeBits) continue;
    const name = stripIndex(k.name) || k.name;
    const last = out[out.length - 1];
    if (last !== undefined && last.name === name) {
      last.endBit = Math.max(last.endBit, k.offset_bits + k.size_bits);
      last.count++;
      continue;
    }
    out.push({ name, path: k.path, startBit: k.offset_bits, endBit: k.offset_bits + k.size_bits, count: 1 });
  }
  return out;
}

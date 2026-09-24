// The core's byte ledger, read for the report: every bit of the file in one
// row, keyed by part, by the variant of the nearest list element, and by role.
// See "The byte ledger" in docs/DESIGN-report-view.md.
//
// The report shows it twice. Section 6 has one row per part and group, with
// its share of the file; section 12 has every role under each of those. The
// grouping here is the part that can be wrong, so it has no DOM in it and the
// tests run it.

import type { LedgerRole, LedgerRow } from "./coredata.ts";

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

/**
 * The ledger's rows as the report's lines, in file order.
 *
 * - A part with groups under it is a line per group: a ZIP's records are the
 *   local files, the central directory and its end.
 * - A part that is one plain field is taken together with the other plain
 *   fields of the same structure: the thirty fields of an ELF header are one
 *   line, not thirty. One such field on its own keeps its own name.
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
    roles: Map<LedgerRole, { bits: number; zero: number; unscanned: number; aligns: Set<number> }>;
  };
  const lines = new Map<string, Acc>();
  for (const r of rows) {
    const plain = r.group === "" && r.role !== "gap" && r.role !== "padding" && samePath(r.first_path, r.part);
    const key =
      r.role === "gap"
        ? "gap"
        : r.role === "padding"
          ? "padding"
          : plain
            ? `fields:${r.part.slice(0, -1).join("/")}`
            : `part:${r.part.join("/")}\u0000${r.group}`;
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
    const role = a.roles.get(r.role) ?? { bits: 0, zero: 0, unscanned: 0, aligns: new Set<number>() };
    role.bits += r.bits;
    role.zero += r.zero_bits;
    role.unscanned += r.unscanned_bits;
    if (r.align > 0) role.aligns.add(r.align);
    a.roles.set(r.role, role);
  }
  return [...lines.values()]
    .map((a) => ({
      key: a.key,
      // One plain field on its own is that field.
      part: a.part ?? (a.fields.length === 1 ? (a.fields[0] ?? null) : null),
      parentPath: a.fields.length === 1 ? null : a.parentPath,
      group: a.group,
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

/** True for the line of bytes no field describes. */
export function isGapLine(l: LedgerLine): boolean {
  return l.key === "gap";
}

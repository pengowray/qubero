// A pickled list of records, shown as the table the core says it is.
//
// `[{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]` is how rows are pickled
// when nobody reached for pandas: one dict a row, the same keys in each. Read
// field by field that is a list of dicts of entries of instructions, and the
// question a reader has is what is in the table.
//
// Which node is that table, and what its columns are called, is the core's
// answer now rather than this file's. A Familiar Pickle Form knows where every
// value went, so `Evaluator::pickle_table` declares the shape and names the
// columns, and the shape's `cells` says where a cell is: rows are the `dict`
// nodes under the list, a cell is the `entry` inside a row named by its
// column, and what it is worth is that entry's `value`. This file walks that.
//
// Only under the familiar-form template, where a dict is a node with an entry
// for each key. Under the opcode listing the same file is a program, and the
// rows do not exist until something runs it.

import type { Doc, TableCells, TableShape, TemplateNode } from "./doc.ts";

/** The case of `TableCells` this file walks: cells that are nodes of the tree,
 *  named by their column. */
type NamedCells = Extract<TableCells, { kind: "named" }>;
import type { RecordCell, RecordPlan, RecordRow, RecordTable } from "./records.ts";

/** A reference, whose two bytes say nothing on their own: what it names is the
 *  row beside it, which is what the cell shows. */
const REFERENCE = "reference";
const REFERS_TO_FIELD = "refers to";

/** The shape the core hung on this node, when it is one whose cells are named
 *  nodes. Everything else is a run of values and is not this file's. */
function namedCells(doc: Doc, node: TemplateNode): { shape: TableShape; cells: NamedCells } | null {
  if (node.table !== true) return null;
  const reply = doc.tableShape(node.path);
  const shape = reply.status === "ok" ? reply.node : null;
  if (shape === null || shape.cells === null || shape.cells.kind !== "named") return null;
  return { shape, cells: shape.cells };
}

export function picklePlan(doc: Doc, node: TemplateNode): RecordPlan | null {
  const said = namedCells(doc, node);
  if (said === null) return null;
  return { build: () => build(doc, node, said.shape, said.cells) };
}

function build(doc: Doc, node: TemplateNode, shape: TableShape, cells: NamedCells): RecordTable | null {
  const columns = shape.names.map((n) => n);
  const children = doc.templateChildren(node.path, 0, node.child_count);
  if (children.status !== "ok") return { columns, rows: [], pending: true };
  const at = new Map<string, number>();
  columns.forEach((name, i) => at.set(name, i));
  let pending = false;
  const rows: RecordRow[] = [];
  for (const row of children.node) {
    if (row.type !== cells.row) continue;
    const parts = doc.templateChildren(row.path, 0, row.child_count);
    if (parts.status !== "ok") {
      pending = true;
      continue;
    }
    // A key the core did not name has no column, and a row without a column's
    // key has an empty cell under it rather than the next key's value.
    const filled: RecordCell[] = columns.map(() => ABSENT);
    for (const part of parts.node) {
      if (part.type !== cells.cell) continue;
      const column = at.get(part.name);
      if (column !== undefined) filled[column] = entryCell(doc, part, cells.value);
    }
    rows.push({ cells: filled, path: row.path, offsetBits: row.offset_bits, sizeBits: row.size_bits });
  }
  const word = shape.row_word;
  return word === null ? { columns, rows, pending } : { columns, rows, pending, rowWord: word };
}

/** A key this row does not have. */
const ABSENT: RecordCell = { text: "", kind: "absent" };

/** What a cell is worth: the named field inside it, what a reference refers
 *  to, or for a value that holds others, what kind of thing it is. */
function entryCell(doc: Doc, entry: TemplateNode, field: string | null): RecordCell {
  if (field === null) return leafCell(entry);
  const parts = doc.templateChildren(entry.path, 0, entry.child_count);
  if (parts.status !== "ok") return { text: "", kind: "unread" };
  const value = parts.node.find((p) => p.name === field);
  if (value === undefined) return { text: "", kind: "unread" };
  if (!value.composite) return leafCell(value);
  if (value.type === REFERENCE) {
    const inside = doc.templateChildren(value.path, 0, value.child_count);
    const said = inside.status === "ok" ? inside.node.find((p) => p.name === REFERS_TO_FIELD) : undefined;
    return said === undefined ? { text: "", kind: "unread" } : leafCell(said);
  }
  return { text: value.type, kind: "composite" };
}

/** A leaf as a cell. `None`, `True` and `False` are opcodes with no operand,
 *  so the tree reads them as a named byte, `True (136)`; in a table of values
 *  the number is the instruction's and not the value's, and it goes. */
function leafCell(n: TemplateNode): RecordCell {
  const text = n.kind === "enum" ? n.value.replace(/\s*\(\d+\)\s*$/, "") : n.value;
  return n.problem === undefined ? { text, kind: n.kind } : { text, kind: n.kind, problem: n.problem };
}

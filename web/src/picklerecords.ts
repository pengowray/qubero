// A pickled list of dicts, shown as the table it is.
//
// `[{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]` is how rows are pickled
// when nobody reached for pandas: one dict a row, the same keys in each. Read
// field by field that is a list of dicts of entries of instructions, and the
// question a reader has is what is in the table. The keys are the columns, and
// they are written in the file beside the values, so nothing is looked up.
//
// Only under the familiar-form template, where a dict is a node with an entry
// for each key. Under the opcode listing the same file is a program, and the
// rows do not exist until something runs it.

import type { Doc, TemplateNode } from "./doc.ts";
import type { RecordCell, RecordPlan, RecordRow, RecordTable } from "./records.ts";

const TEMPLATE = "picklefpf";
/** The type names the familiar-form tree gives its nodes. */
const DICT = "dict";
const ENTRY = "entry";
const REFERENCE = "reference";
/** The field of an entry that is what the key maps to, and the field of a
 *  reference that says what it refers to. */
const VALUE_FIELD = "value";
const REFERS_TO_FIELD = "refers to";
/** How many of a list's children are looked at to decide whether it is rows.
 *  The first few are its own instructions, and the question is asked of every
 *  heading the listing draws. */
const LOOKED_AT = 12;
/** What one dict of the list is, once the list is a table. */
const ROW_WORD = "row";
/** The fewest dicts that make a table. One dict is a record, not a list of them. */
const FEWEST_ROWS = 2;

/** A list whose values are all dicts: every child is a dict or an instruction
 *  of the list's own, which is a leaf. A list of lists or of numbers is not
 *  rows, and neither is a list with one dict among other things. */
export function picklePlan(doc: Doc, node: TemplateNode): RecordPlan | null {
  if (doc.template !== TEMPLATE) return null;
  const first = doc.templateChildren(node.path, 0, Math.min(node.child_count, LOOKED_AT));
  if (first.status !== "ok") return null;
  const values = first.node.filter((c) => c.composite);
  if (values.length < FEWEST_ROWS || !values.every((c) => c.type === DICT)) return null;
  return { build: () => build(doc, node) };
}

function build(doc: Doc, node: TemplateNode): RecordTable | null {
  const children = doc.templateChildren(node.path, 0, node.child_count);
  if (children.status !== "ok") return { columns: [], rows: [], pending: true };
  // A value that is not a dict, further down than the first few: the list is
  // not rows after all, and half a table would hide the odd one out.
  if (children.node.some((c) => c.composite && c.type !== DICT)) return null;
  // The columns are every key any row has, in the order first met. Rows of a
  // pickled table need not all have every key, and a row without one has an
  // empty cell under it rather than the next key's value.
  const columns: string[] = [];
  const at = new Map<string, number>();
  const read: { dict: TemplateNode; entries: TemplateNode[] }[] = [];
  let pending = false;
  for (const dict of children.node.filter((c) => c.composite)) {
    const parts = doc.templateChildren(dict.path, 0, dict.child_count);
    if (parts.status !== "ok") {
      pending = true;
      continue;
    }
    const entries = parts.node.filter((p) => p.type === ENTRY);
    for (const e of entries) {
      if (!at.has(e.name)) {
        at.set(e.name, columns.length);
        columns.push(e.name);
      }
    }
    read.push({ dict, entries });
  }
  const rows: RecordRow[] = read.map(({ dict, entries }) => {
    const cells: RecordCell[] = columns.map(() => ABSENT);
    for (const e of entries) cells[at.get(e.name) ?? 0] = entryCell(doc, e);
    return { cells, path: dict.path, offsetBits: dict.offset_bits, sizeBits: dict.size_bits };
  });
  return { columns, rows, pending, rowWord: ROW_WORD };
}

/** A key this row does not have. */
const ABSENT: RecordCell = { text: "", kind: "absent" };

/** What an entry's key maps to: the value itself, what a reference refers to,
 *  or for a value that holds others, what kind of thing it is. */
function entryCell(doc: Doc, entry: TemplateNode): RecordCell {
  const parts = doc.templateChildren(entry.path, 0, entry.child_count);
  if (parts.status !== "ok") return { text: "", kind: "unread" };
  const value = parts.node.find((p) => p.name === VALUE_FIELD);
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

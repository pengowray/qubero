// GGUF's metadata block, drawn as the key/value table it is.
//
// Read field by field, one entry is four levels deep for a single string:
// a key that is a length and its text, a type code, and a value that is again
// a length and its text — and the interesting part, the key, is the one thing
// the reader has to unfold twice to see. Every entry has the same three parts,
// so they are columns, and the block is a table of a few dozen rows.
//
// Unlike SQLite, GGUF names its own columns: `key`, `value_type` and `value`
// are the template's field names, so nothing here parses anything to find
// them. What does need work is the value column, which holds three different
// shapes: a scalar reads as itself, a string is a length and its text, and an
// array is a type, a length and its items. An array's items are not spread
// across the row — a token list of a quarter of a million strings is not a
// table cell — so the cell says how many of what, and the row's "stored at"
// is the way into them.

import type { Doc, TemplateNode } from "./doc.js";
import type { RecordCell, RecordPlan, RecordRow, RecordTable } from "./records.js";
import { countText } from "./strings.js";

/** GGUF's own field names, which is why they are not in `strings.ts`. */
const COLUMNS = ["key", "value_type", "value"] as const;

export function ggufPlan(doc: Doc, node: TemplateNode): RecordPlan | null {
  if (doc.template !== "gguf") return null;
  if (node.name !== "metadata") return null;
  return { build: () => build(doc, node) };
}

function build(doc: Doc, node: TemplateNode): RecordTable | null {
  const entries = doc.templateChildren(node.path, 0, node.child_count);
  if (entries.status !== "ok") return { columns: [...COLUMNS], rows: [], pending: true };
  const rows: RecordRow[] = [];
  let pending = false;
  for (const entry of entries.node) {
    const parts = doc.templateChildren(entry.path, 0, 3);
    if (parts.status !== "ok") {
      pending = true;
      continue;
    }
    const [key, type, value] = parts.node;
    if (key === undefined || type === undefined || value === undefined) {
      pending = true;
      continue;
    }
    const cells: RecordCell[] = [valueCell(doc, key), { text: type.value, kind: type.kind }, valueCell(doc, value)];
    rows.push({ cells, path: entry.path, offsetBits: entry.offset_bits, sizeBits: entry.size_bits });
  }
  return { columns: [...COLUMNS], rows, pending };
}

/** What one entry's value reads as: itself, its text, or how many items it
 *  holds and of what. */
function valueCell(doc: Doc, value: TemplateNode): RecordCell {
  if (!value.composite) return { text: value.value, kind: value.kind };
  const parts = doc.templateChildren(value.path, 0, 3);
  if (parts.status !== "ok") return { text: "", kind: "unread" };
  const named = new Map(parts.node.map((p) => [p.name, p]));
  const text = named.get("text");
  if (text !== undefined) return { text: text.value, kind: text.kind };
  const items = named.get("items");
  const of = named.get("value_type");
  if (items === undefined) return { text: "", kind: "unread" };
  // The element type as GGUF spells it, with the enum's raw number taken off:
  // "4 string (8)" would read as a fourth column.
  const unit = (of?.value ?? "item").replace(/\s*\(\d+\)\s*$/, "");
  return { text: countText(items.child_count, unit), kind: "composite" };
}

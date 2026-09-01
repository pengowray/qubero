// Formats that keep a table, shown as one.
//
// A SQLite page holds rows. Read field by field it is a run of cells, each a
// payload size, a rowid and a record of serial types and columns, and the
// reader has to hold five levels in their head to answer "what is in this
// table". The format knows the columns' names; the per-format readers below
// find them and lay the rows out under them.
//
// **Registry, not IR.** The handover asked for this to be settled once a
// second and third format arrived. Three are here now — SQLite's b-tree pages,
// SQLite's schema page and GGUF's metadata block — and the answer is still the
// registry. The reason is what the three have in common, which is nothing
// declarative:
//
//   - A SQLite table's column names are not in the page, in the template, or
//     anywhere a declaration could point at. They are inside a `CREATE TABLE`
//     statement stored as a string in a *different page*, and getting them out
//     means parsing SQL.
//   - An index page's columns come from a `CREATE INDEX` statement, plus a
//     trailing rowid the statement never mentions.
//   - An interior page has no user columns at all; its table is the b-tree's
//     own (child page, key) pairs, and one of its rows is a field of the page
//     header rather than a cell.
//   - GGUF's metadata is a flat key/type/value array whose value column has to
//     be summarised differently for scalars, strings and arrays.
//
// An IR field saying "this array is a record list with columns X" fits none of
// them. What would fit is an escape hatch per format, which is this file with
// the code moved to Rust and a call bridged through wasm: the same amount of
// format-specific logic, further from the DOM it exists to draw, and paid for
// by every non-web consumer of the IR that will never render a table. The IR
// describes bytes. A record view describes what those bytes mean once several
// parts of the file are read together, which is a view's job.
//
// The one thing that would change the answer is a format whose records really
// are declared in its own template — column names written beside the fields,
// no lookups. If two of those turn up, the declarative half belongs in the IR
// and this file keeps the awkward cases.

import type { Doc, TemplateNode } from "./doc.js";
import { sqlitePlan } from "./sqliterecords.js";
import { ggufPlan } from "./ggufrecords.js";

/** Somewhere else in the same file that this cell names. Drawn as a link with
 *  a direction arrow, which is rule 7's cross-reference. */
export type RecordLink = {
  /** What the link reads as, arrow included. */
  readonly text: string;
  /** The same thing spelled out, for a tooltip and for a screen reader. */
  readonly label: string;
  /** The field to go to, which the listing reveals and selects. */
  readonly path: readonly number[];
};

/** A cell of the table, and where its bytes are so the reader can go there. */
export type RecordCell = {
  readonly text: string;
  /** What sort of value it is, for the same colouring the rows use. */
  readonly kind: string;
  /** Set when the value names another part of the file. */
  readonly link?: RecordLink;
};

export type RecordRow = {
  readonly cells: readonly RecordCell[];
  /** The field this row was read from, to go back to it. */
  readonly path: readonly number[];
  readonly offsetBits: number;
  readonly sizeBits: number;
};

export type RecordTable = {
  readonly columns: readonly string[];
  readonly rows: readonly RecordRow[];
  /** True while some of it is still being read. */
  readonly pending: boolean;
};

/**
 * What a format's reader answers when it recognises a node: that this is a
 * table, and how to build it.
 *
 * Two steps rather than one because the two questions cost different amounts.
 * Every heading in the listing is asked whether it is a record list, on every
 * walk of the tree; only the ones on screen are asked for their rows.
 */
export type RecordPlan = {
  readonly build: () => RecordTable | null;
};

/** Every format that draws its records as records, in the order they are
 *  tried. A reader returns null for anything it does not recognise, which is
 *  almost everything, so the order is not significant. */
const READERS: readonly ((doc: Doc, node: TemplateNode) => RecordPlan | null)[] = [sqlitePlan, ggufPlan];

function planFor(doc: Doc, node: TemplateNode): RecordPlan | null {
  if (!node.composite || node.child_count === 0) return null;
  for (const reader of READERS) {
    const plan = reader(doc, node);
    if (plan !== null) return plan;
  }
  return null;
}

/** Whether this node is a table some format can name the columns of. Cheap
 *  enough to ask of every heading, since it stops at the name. */
export function isRecordList(doc: Doc, node: TemplateNode): boolean {
  return planFor(doc, node) !== null;
}

/**
 * The rows of the table this node holds, under the columns the format names.
 *
 * Gives up rather than guessing. A row with more columns than the format
 * accounts for means the names have been read wrong, and a name over the wrong
 * column is worse than no name at all, so the whole table falls back to
 * nothing and the reader gets the fields.
 */
export function recordTable(doc: Doc, node: TemplateNode): RecordTable | null {
  return planFor(doc, node)?.build() ?? null;
}

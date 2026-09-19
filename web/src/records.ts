// Formats that keep a table, shown as one.
//
// A SQLite page holds rows. Read field by field it is a run of cells, each a
// payload size, a rowid and a record of serial types and columns, and the
// reader has to hold five levels in their head to answer "what is in this
// table". The format knows the columns' names; the per-format readers below
// find them and lay the rows out under them.
//
// **Registry for the awkward cases, IR for the declared ones.** The handover
// asked for this to be settled once a second and third format arrived. Three
// were here then — SQLite's b-tree pages, SQLite's schema page and GGUF's
// metadata block — and the answer was the registry, because what those three
// have in common is nothing declarative:
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
// The last paragraph of this comment used to say that the answer would change
// for a format whose records really are declared — column names written in the
// file beside the values, no lookups — once two of those turned up. Two have:
// a pickled list of records, whose keys sit beside its values, and a pandas
// frame, whose column names are in its index. So the declarative half moved to
// the IR. `TableShape::cells` says where a cell is when a row is a node rather
// than a run of values, `Evaluator::pickle_table` declares it and names the
// columns, and `picklerecords.ts` is now a walk of that answer rather than a
// reader that works the columns out for itself. This file keeps the awkward
// cases, which are still awkward and still here.

import type { Doc, Problem, TemplateNode } from "./doc.ts";
import { sqlitePlan } from "./sqliterecords.ts";
import { ggufPlan } from "./ggufrecords.ts";
import { picklePlan } from "./picklerecords.ts";

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
  /** What is wrong with this value, when something is. A table is often the
   *  only place a reader ever sees a run's values, so a cell that cannot say
   *  its value is wrong hides the finding the reader opened the file for.
   *  See docs/DESIGN-wrong-values.md. */
  readonly problem?: Problem;
  /** Where this cell's own bytes are, for a table whose rows are not a run of
   *  the file: a pandas frame, a torch tensor, a checkpoint's summary. Not
   *  set for a record table's cells, where the row says where it is. */
  readonly at?: CellPlace;
  /** Why this cell has no bytes, when it has none and the reason is worth
   *  saying: a counted index label was never written down, and a fact the
   *  pickle states about a tensor is not stored as a value. */
  readonly noBytes?: "counted" | "nowhere";
  /** Set for an entry a masked array's mask hides. Not a `noBytes` reason:
   *  the cell has bytes, and the number in them is readable at the address it
   *  carries. What it has not got is a value the array counts. */
  readonly masked?: boolean;
};

/** Where one cell's bytes are. `space` is 0 for the tab's own bytes, which is
 *  what the hex view shows, and anything else for a stream unpacked inside
 *  them. See `FrameCell` in doc.ts. */
export type CellPlace = {
  readonly space: number;
  readonly offsetBits: number;
  readonly sizeBits: number;
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
  /** What one row is, where the reader knows better than the node's own word
   *  for its children: a pickled list's children are instructions as well as
   *  dicts, and "20 fields" over twenty rows counts neither. */
  readonly rowWord?: string;
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
const READERS: readonly ((doc: Doc, node: TemplateNode) => RecordPlan | null)[] = [sqlitePlan, ggufPlan, picklePlan];

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

// Formats that keep a table, shown as one.
//
// A SQLite page holds rows. Read field by field it is a run of cells, each a
// payload size, a rowid and a record of serial types and columns, and the
// reader has to hold five levels in their head to answer "what is in this
// table". The format knows the columns' names; this finds them and lays the
// rows out under them.
//
// Which formats do this is a question with no answer in the IR: nothing in a
// template says "these are records". Keeping the answer here, keyed by
// template name, is the smaller commitment of the two the handover weighs. If
// a third or fourth format wants it, that is the evidence for moving it into
// the IR; one format is not.

import type { Doc, TemplateNode } from "./doc.js";

/** A cell of the table, and where its bytes are so the reader can go there. */
export type RecordCell = {
  readonly text: string;
  /** What sort of value it is, for the same colouring the rows use. */
  readonly kind: string;
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

/** SQLite's own code for a page that holds rows rather than pointers to more
 *  pages. Interior pages and index pages are not tables of rows and are left
 *  to read field by field. */
const TABLE_LEAF = 13;

function value(doc: Doc, path: readonly number[]): string | null {
  const r = doc.templateNode(path);
  return r.status === "ok" ? r.node.value : null;
}

/**
 * The column names written in a `CREATE TABLE` statement.
 *
 * Not a parser for the grammar, which is deep enough that a wrong answer is
 * likely and a wrong answer here puts a real column's data under another
 * column's name. This takes the names positionally and gives up rather than
 * guessing: the text between the first bracket and its match, split on the
 * commas that are not inside brackets or quotes, and the first identifier of
 * each part. A part that opens with a constraint word describes the table
 * rather than a column, and ends the list.
 */
export function columnNames(sql: string): string[] {
  const open = sql.indexOf("(");
  if (open < 0) return [];
  const parts: string[] = [];
  let depth = 0;
  let quote = "";
  let from = open + 1;
  let i = open;
  for (; i < sql.length; i++) {
    const c = sql[i] ?? "";
    if (quote !== "") {
      // Doubling is how every one of SQLite's quotes escapes itself.
      if (c === quote && sql[i + 1] === quote) i++;
      else if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "[") quote = "]";
    else if (c === "(") depth++;
    else if (c === ")") {
      depth--;
      if (depth === 0) break;
    } else if (c === "," && depth === 1) {
      parts.push(sql.slice(from, i));
      from = i + 1;
    }
  }
  if (depth !== 0) return [];
  parts.push(sql.slice(from, i));
  const names: string[] = [];
  for (const part of parts) {
    const name = leadingName(part.trim());
    if (name === null) break;
    names.push(name);
  }
  return names;
}

const CONSTRAINT = /^(primary|unique|check|foreign|constraint)\b/i;

/** The identifier a column definition starts with, or null when the part is a
 *  constraint on the table rather than a column of it. */
function leadingName(part: string): string | null {
  if (part === "" || CONSTRAINT.test(part)) return null;
  const first = part[0] ?? "";
  const close = first === '"' ? '"' : first === "`" ? "`" : first === "[" ? "]" : first === "'" ? "'" : "";
  if (close !== "") {
    let out = "";
    for (let i = 1; i < part.length; i++) {
      const c = part[i] ?? "";
      if (c === close && part[i + 1] === close) {
        out += c;
        i++;
      } else if (c === close) return out;
      else out += c;
    }
    return out === "" ? null : out;
  }
  const bare = /^[A-Za-z_][\w$]*/.exec(part);
  return bare === null ? null : bare[0];
}

/** Whether a column definition makes its column an alias for the rowid, which
 *  SQLite stores as a null and reads back from the row's own number. */
function isRowidAlias(part: string): boolean {
  return /\binteger\b[\s\S]*\bprimary\s+key\b/i.test(part) && !/\bdesc\b/i.test(part);
}

/** The `CREATE TABLE` text of the schema object whose root is this page, and
 *  whether it is a table at all. */
function schemaFor(doc: Doc, page: number): { readonly sql: string; readonly name: string } | null {
  const schema = doc.templateNode([23, 6]);
  if (schema.status !== "ok") return null;
  for (let i = 0; i < schema.node.child_count; i++) {
    const record = [23, 6, i, 2];
    if (value(doc, [...record, 2])?.toLowerCase() !== "table") continue;
    if (Number(value(doc, [...record, 5])) !== page) continue;
    return { sql: value(doc, [...record, 6]) ?? "", name: value(doc, [...record, 3]) ?? "" };
  }
  return null;
}

/** Which page of the file a byte offset is in, counting from one. */
function pageOf(doc: Doc, offsetBits: number): number | null {
  const raw = Number(value(doc, [1]));
  const size = raw === 1 ? 65_536 : raw;
  if (!Number.isFinite(size) || size <= 0) return null;
  return Math.floor(offsetBits / 8 / size) + 1;
}

/** Whether this node is a run of rows some table's schema names the columns
 *  of. Cheap enough to ask of every heading, since it stops at the name. */
export function isRecordList(doc: Doc, node: TemplateNode): boolean {
  return sqliteTable(doc, node) !== null;
}

function sqliteTable(doc: Doc, node: TemplateNode): { readonly names: string[]; readonly rowidAt: number } | null {
  if (doc.template !== "sqlite" && doc.template !== "self") return null;
  if (node.name !== "cells" || !node.composite || node.child_count === 0) return null;
  const pagePath = node.path.slice(0, -1);
  if (Number(value(doc, [...pagePath, 0])?.replace(/^.*\((\d+)\)$/, "$1")) !== TABLE_LEAF) return null;
  const page = pageOf(doc, node.offset_bits);
  if (page === null) return null;
  const schema = schemaFor(doc, page);
  if (schema === null) return null;
  const names = columnNames(schema.sql);
  if (names.length === 0) return null;
  const parts = splitDefinitions(schema.sql);
  return { names, rowidAt: parts.findIndex(isRowidAlias) };
}

/** The same split as `columnNames`, kept so the rowid test can see the whole
 *  definition rather than only the name taken from it. */
function splitDefinitions(sql: string): string[] {
  const open = sql.indexOf("(");
  if (open < 0) return [];
  const out: string[] = [];
  let depth = 0;
  let quote = "";
  let from = open + 1;
  let i = open;
  for (; i < sql.length; i++) {
    const c = sql[i] ?? "";
    if (quote !== "") {
      if (c === quote && sql[i + 1] === quote) i++;
      else if (c === quote) quote = "";
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "[") quote = "]";
    else if (c === "(") depth++;
    else if (c === ")") {
      depth--;
      if (depth === 0) break;
    } else if (c === "," && depth === 1) {
      out.push(sql.slice(from, i).trim());
      from = i + 1;
    }
  }
  out.push(sql.slice(from, i).trim());
  return out;
}

/**
 * The rows of the table this node holds, under the columns the format names.
 *
 * Gives up rather than guessing. A row with more columns than the schema
 * accounts for means the names have been read wrong, and a name over the wrong
 * column is worse than no name at all, so the whole table falls back to
 * nothing and the reader gets the fields.
 */
export function recordTable(doc: Doc, node: TemplateNode): RecordTable | null {
  const table = sqliteTable(doc, node);
  if (table === null) return null;
  const rows: RecordRow[] = [];
  let pending = false;
  let widest = 0;
  for (let i = 0; i < node.child_count; i++) {
    const cellPath = [...node.path, i];
    const cell = doc.templateNode(cellPath);
    if (cell.status !== "ok") {
      pending = true;
      continue;
    }
    const columns = doc.templateChildren([...cellPath, 2, 2], 0, table.names.length + 1);
    if (columns.status !== "ok") {
      pending = true;
      continue;
    }
    widest = Math.max(widest, columns.node.length);
    const rowid = value(doc, [...cellPath, 1]) ?? "";
    const cells = table.names.map((_, c) => {
      // An INTEGER PRIMARY KEY column is stored as a null; its value is the
      // row's own number, which is written once at the front of the cell.
      if (c === table.rowidAt) return { text: rowid, kind: "int" };
      const column = columns.node[c];
      return column === undefined ? { text: "", kind: "unread" } : { text: column.value, kind: column.kind };
    });
    rows.push({ cells, path: cellPath, offsetBits: cell.node.offset_bits, sizeBits: cell.node.size_bits });
  }
  if (widest > table.names.length) return null;
  // Rows go in the order the table has them, not the order the page keeps
  // them: a b-tree leaf fills from the back, so the last row is first in the
  // file. "Stored at" is where the physical order shows.
  rows.sort((a, b) => Number(a.cells[table.rowidAt]?.text ?? 0) - Number(b.cells[table.rowidAt]?.text ?? 0));
  return { columns: table.names, rows, pending };
}

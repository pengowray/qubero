// SQLite's b-tree pages, drawn as the tables they are.
//
// Four page kinds arrive here and only one of them is a table of user rows:
//
//   - **Table leaf** (0x0d): rows of a user table. The columns are named in a
//     `CREATE TABLE` statement stored in page 1, so the names come from a
//     different page than the data.
//   - **The schema page**: also a table leaf, but page 1's rows are
//     `sqlite_master` rows, whose five columns SQLite fixes rather than
//     declares. Its `rootpage` column is a pointer at another page and is
//     drawn as a link.
//   - **Index leaf** (0x0a): the payload is a key, not a row. A key is still a
//     tuple with names — the columns of the `CREATE INDEX` statement, then the
//     rowid of the table row it points at — so it is a table with the index's
//     own column names and one more column the index never wrote down.
//   - **Interior pages** (0x05 table, 0x02 index): not rows at all. What the
//     page stores is a range partition: keys up to K1 are on child page P1,
//     keys up to K2 on P2, and everything above the last key is on the pointer
//     in the page header. That is two columns and one row per branch, so it is
//     a table — of the b-tree rather than of the data — and the child pages
//     are links. The header's `right_most_page` is the last row: it has no key
//     because it has no upper bound, and its "stored at" points at the header
//     field it really is rather than at a cell that does not exist.
//
// Which page belongs to which table is not written anywhere either. Only the
// root of each b-tree is named in the schema; every other page is reached by
// following child pointers, so `owners` walks the trees once and remembers.

import type { Doc, TemplateNode } from "./doc.js";
import type { RecordCell, RecordPlan, RecordRow, RecordTable } from "./records.js";
import { REPORT } from "./strings.js";
import { columnNames, isRowidAlias, splitDefinitions } from "./sqlitesql.js";

/** SQLite's page type byte, as the enum's raw value. */
const TABLE_INTERIOR = 5;
const INDEX_INTERIOR = 2;
const INDEX_LEAF = 10;
const TABLE_LEAF = 13;

/** How far a tree walk may go before it is treated as a loop. A corrupt or
 *  half-read file can point a page at itself; the map is a convenience and is
 *  not worth hanging the view over. */
const WALK_LIMIT = 100_000;

function value(doc: Doc, path: readonly number[]): string | null {
  const r = doc.templateNode(path);
  return r.status === "ok" ? r.node.value : null;
}

/**
 * A composite's children by field name. SQLite's structures are short, so the
 * whole of one is read at once and looked up by name rather than by a position
 * that moves when a page has a `right_most_page` and its neighbour does not.
 *
 * A node's `name` is its field name and then whatever the structure is named
 * after, so a schema row's payload arrives as `payload notes`. The field name
 * is the part before the space; template field names have no spaces in them.
 */
function fieldsOf(doc: Doc, path: readonly number[], count: number): Map<string, TemplateNode> | null {
  const kids = doc.templateChildren(path, 0, count);
  if (kids.status !== "ok") return null;
  const out = new Map<string, TemplateNode>();
  for (const kid of kids.node) out.set(kid.name.split(" ")[0] ?? kid.name, kid);
  return out;
}

/** The number inside an enum's `name (raw)`, which is how a page type reads. */
function rawEnum(text: string | undefined): number | null {
  if (text === undefined) return null;
  const m = /\((\d+)\)\s*$/.exec(text);
  const n = Number(m === null ? text : m[1]);
  return Number.isFinite(n) ? n : null;
}

// ---- what the database as a whole says, worked out once ----

type SchemaEntry = {
  readonly kind: string;
  readonly name: string;
  readonly rootpage: number;
  readonly sql: string;
};

type Database = {
  readonly pageSizeBytes: number;
  /** Root children: where page 1 is, and where the rest of the pages are. */
  readonly page1At: number;
  readonly pagesAt: number;
  readonly schema: readonly SchemaEntry[];
  /** Page number to the schema object whose b-tree it is part of, filled in by
   *  following child pointers from each root. */
  readonly owners: Map<number, SchemaEntry>;
};

/** One `Database` per open file, thrown away whenever the file changes: a
 *  chunk arriving turns an unread page into a readable one, and an edit can
 *  change what the schema says. Without this the whole schema would be read
 *  again for every heading of every walk of the tree. */
const cache = new WeakMap<Doc, Database | null>();
const hooked = new WeakSet<Doc>();

function database(doc: Doc): Database | null {
  if (!hooked.has(doc)) {
    hooked.add(doc);
    doc.onChange(() => cache.delete(doc));
  }
  if (cache.has(doc)) return cache.get(doc) ?? null;
  const built = readDatabase(doc);
  cache.set(doc, built);
  return built;
}

function readDatabase(doc: Doc): Database | null {
  const root = fieldsOf(doc, [], 64);
  if (root === null) return null;
  const size = root.get("page_size");
  const page1 = root.get("page1");
  const pages = root.get("pages");
  if (size === undefined || page1 === undefined || pages === undefined) return null;
  const raw = Number(size.value);
  // SQLite writes a 65,536-byte page size as 1, which is the one value that
  // does not fit the two bytes it is stored in.
  const pageSizeBytes = raw === 1 ? 65_536 : raw;
  if (!Number.isFinite(pageSizeBytes) || pageSizeBytes <= 0) return null;
  const db: Database = {
    pageSizeBytes,
    page1At: page1.path[page1.path.length - 1] ?? 23,
    pagesAt: pages.path[pages.path.length - 1] ?? 24,
    schema: readSchema(doc, page1.path),
    owners: new Map(),
  };
  claimPages(doc, db);
  return db;
}

/** The rows of `sqlite_master`, read off page 1. A schema too big for one page
 *  puts an interior page here instead, and then nothing is read: the columns
 *  of the user tables stay unknown and every page falls back to its fields,
 *  which is the honest answer rather than a partial one. */
function readSchema(doc: Doc, page1Path: readonly number[]): SchemaEntry[] {
  const page = fieldsOf(doc, page1Path, 8);
  const cells = page?.get("cells");
  if (page === null || cells === undefined) return [];
  if (rawEnum(page.get("page_type")?.value) !== TABLE_LEAF) return [];
  const out: SchemaEntry[] = [];
  for (let i = 0; i < cells.child_count; i++) {
    const cell = fieldsOf(doc, [...cells.path, i], 4);
    const payload = cell?.get("payload");
    if (payload === undefined) continue;
    const record = fieldsOf(doc, payload.path, 8);
    if (record === null) continue;
    const rootpage = Number(record.get("rootpage")?.value);
    out.push({
      kind: (record.get("type")?.value ?? "").toLowerCase(),
      name: record.get("name")?.value ?? "",
      rootpage: Number.isFinite(rootpage) ? rootpage : 0,
      sql: record.get("sql")?.value ?? "",
    });
  }
  return out;
}

function pagePath(db: Database, page: number): readonly number[] {
  return page === 1 ? [db.page1At] : [db.pagesAt, page - 2];
}

/** Which page of the file a byte offset is in, counting from one. */
function pageOf(db: Database, offsetBits: number): number {
  return Math.floor(offsetBits / 8 / db.pageSizeBytes) + 1;
}

/**
 * Follow every b-tree from its root and note which object owns each page.
 *
 * The schema names only the roots. A table of four hundred rows spills over
 * twenty leaf pages, none of which says what it is a page of, and without this
 * only the root of each tree could be drawn as a table.
 */
function claimPages(doc: Doc, db: Database): void {
  let steps = 0;
  for (const entry of db.schema) {
    if (entry.rootpage <= 0) continue;
    if (entry.kind !== "table" && entry.kind !== "index") continue;
    const stack = [entry.rootpage];
    while (stack.length > 0 && steps++ < WALK_LIMIT) {
      const page = stack.pop();
      if (page === undefined || db.owners.has(page)) continue;
      db.owners.set(page, entry);
      const fields = fieldsOf(doc, pagePath(db, page), 8);
      if (fields === null) continue;
      const type = rawEnum(fields.get("page_type")?.value);
      if (type !== TABLE_INTERIOR && type !== INDEX_INTERIOR) continue;
      const right = Number(fields.get("right_most_page")?.value);
      if (Number.isFinite(right) && right > 0) stack.push(right);
      const cells = fields.get("cells");
      if (cells === undefined) continue;
      const kids = doc.templateChildren(cells.path, 0, cells.child_count);
      if (kids.status !== "ok") continue;
      for (const cell of kids.node) {
        const child = Number(value(doc, [...cell.path, 0]));
        if (Number.isFinite(child) && child > 0) stack.push(child);
      }
    }
  }
}

// ---- what shape this page's cells are ----

type Shape =
  /** Page 1: `sqlite_master`, whose columns SQLite fixes. */
  | { readonly kind: "schema" }
  /** Rows of a user table, under the names its `CREATE TABLE` gives. */
  | { readonly kind: "table"; readonly names: readonly string[]; readonly rowidAt: number }
  /** Index keys: the indexed columns, then the rowid they point at. */
  | { readonly kind: "index"; readonly names: readonly string[] }
  /** A branch of the b-tree: child pages and the keys that bound them. */
  | { readonly kind: "interior"; readonly names: readonly string[]; readonly keyed: boolean };

/** The five columns of `sqlite_master`, in the order the record stores them.
 *  SQLite's own names, which is why they are not in `strings.ts`. */
const SCHEMA_COLUMNS = ["type", "name", "tbl_name", "rootpage", "sql"] as const;

/** The column an index key ends with, which the `CREATE INDEX` statement never
 *  mentions: the rowid of the table row this key belongs to. SQLite's word. */
const ROWID = "rowid";

/** The two columns of a b-tree branch, as the template names its fields. */
const CHILD = "left_child_page";
const RIGHT_MOST = "right_most_page";

function shapeOf(doc: Doc, db: Database, node: TemplateNode): Shape | null {
  const page = pageOf(db, node.offset_bits);
  const type = rawEnum(fieldsOf(doc, node.path.slice(0, -1), 8)?.get("page_type")?.value);
  if (type === TABLE_INTERIOR) return { kind: "interior", names: [CHILD, ROWID], keyed: false };
  if (type === INDEX_INTERIOR) {
    const names = indexColumns(db, page);
    return names === null ? null : { kind: "interior", names: [CHILD, ...names], keyed: true };
  }
  if (type === INDEX_LEAF) {
    const names = indexColumns(db, page);
    return names === null ? null : { kind: "index", names };
  }
  if (type !== TABLE_LEAF) return null;
  if (page === 1) return { kind: "schema" };
  const owner = db.owners.get(page);
  if (owner === undefined || owner.kind !== "table") return null;
  const parts = splitDefinitions(owner.sql);
  const names = columnNames(owner.sql);
  if (names.length === 0) return null;
  return { kind: "table", names, rowidAt: parts.findIndex(isRowidAlias) };
}

/** The columns of the index whose b-tree this page is in, with the rowid the
 *  key carries after them. */
function indexColumns(db: Database, page: number): readonly string[] | null {
  const owner = db.owners.get(page);
  if (owner === undefined || owner.kind !== "index") return null;
  const names = columnNames(owner.sql);
  return names.length === 0 ? null : [...names, ROWID];
}

// ---- building the table ----

/** A cell holding a page number, drawn as the link it is. */
function pageCell(db: Database, text: string): RecordCell {
  const n = Number(text);
  if (!Number.isFinite(n) || n < 1) return { text, kind: "uint" };
  return {
    text,
    kind: "uint",
    link: { text: REPORT.pageLink(n), label: REPORT.pageLinkLabel(n), path: pagePath(db, n) },
  };
}

export function sqlitePlan(doc: Doc, node: TemplateNode): RecordPlan | null {
  if (doc.template !== "sqlite" && doc.template !== "self") return null;
  if (node.name !== "cells") return null;
  const db = database(doc);
  if (db === null) return null;
  const shape = shapeOf(doc, db, node);
  if (shape === null) return null;
  return { build: () => build(doc, db, node, shape) };
}

function build(doc: Doc, db: Database, node: TemplateNode, shape: Shape): RecordTable | null {
  if (shape.kind === "interior") return interior(doc, db, node, shape);
  return payloads(doc, db, node, shape);
}

/**
 * A branch page: one row per (child page, key) pair, then the header's
 * right-most pointer as a row with no key.
 *
 * The cells of an interior page are already in key order, since that is what
 * the cell pointer array is for, so nothing is sorted here.
 */
function interior(
  doc: Doc,
  db: Database,
  node: TemplateNode,
  shape: Extract<Shape, { kind: "interior" }>,
): RecordTable | null {
  const rows: RecordRow[] = [];
  let pending = false;
  for (let i = 0; i < node.child_count; i++) {
    const cellPath = [...node.path, i];
    const cell = doc.templateNode(cellPath);
    const fields = fieldsOf(doc, cellPath, 4);
    if (cell.status !== "ok" || fields === null) {
      pending = true;
      continue;
    }
    const cells: RecordCell[] = [pageCell(db, fields.get(CHILD)?.value ?? "")];
    if (shape.keyed) {
      // An index branch carries a whole key, read the same way a leaf's is.
      const key = keyCells(doc, fields, shape.names.length - 1);
      if (key === "wrong") return null;
      if (key === "wait") pending = true;
      cells.push(...(key === "wait" ? shape.names.slice(1).map(() => ({ text: "", kind: "unread" })) : key));
    } else {
      cells.push({ text: fields.get(ROWID)?.value ?? "", kind: "int" });
    }
    rows.push({ cells, path: cellPath, offsetBits: cell.node.offset_bits, sizeBits: cell.node.size_bits });
  }
  // The last branch is a field of the page header, not a cell: everything past
  // the last key is on it. Its "stored at" is the header field's own address,
  // which is what says why this row is not one of the ones above.
  const header = fieldsOf(doc, node.path.slice(0, -1), 8);
  const right = header?.get(RIGHT_MOST);
  if (right !== undefined) {
    const note: RecordCell = { text: shape.keyed ? REPORT.rightMostKeys : REPORT.rightMostRowids, kind: "note" };
    const rest = shape.names.slice(1).map((_, i) => (i === 0 ? note : { text: "", kind: "note" }));
    rows.push({
      cells: [pageCell(db, right.value), ...rest],
      path: right.path,
      offsetBits: right.offset_bits,
      sizeBits: right.size_bits,
    });
  }
  return { columns: shape.names, rows, pending };
}

/**
 * The columns of one record payload.
 *
 * `"wait"` when they cannot be read yet: a payload that spilled onto an
 * overflow page is not parsed here, and a page whose bytes have not arrived
 * has nothing to read at all. `"wrong"` when the record holds more columns
 * than the names account for, which means the names were read wrong; a value
 * under another column's heading is worse than no heading, so that answer
 * takes the whole table down rather than one row.
 */
function keyCells(doc: Doc, cell: Map<string, TemplateNode>, want: number): RecordCell[] | "wait" | "wrong" {
  const payload = cell.get("payload");
  if (payload === undefined) return "wait";
  const record = fieldsOf(doc, payload.path, 8);
  const columns = record?.get("columns");
  if (columns === undefined) return "wait";
  const kids = doc.templateChildren(columns.path, 0, want + 1);
  if (kids.status !== "ok") return "wait";
  if (kids.node.length > want) return "wrong";
  return Array.from({ length: want }, (_, c) => {
    const column = kids.node[c];
    return column === undefined ? { text: "", kind: "unread" } : { text: column.value, kind: column.kind };
  });
}

/** A leaf page: one row per cell, read out of the record in its payload. */
function payloads(doc: Doc, db: Database, node: TemplateNode, shape: Shape): RecordTable | null {
  const columns = shape.kind === "schema" ? [...SCHEMA_COLUMNS] : [...shape.names];
  const rows: RecordRow[] = [];
  let pending = false;
  for (let i = 0; i < node.child_count; i++) {
    const cellPath = [...node.path, i];
    const cell = doc.templateNode(cellPath);
    const fields = fieldsOf(doc, cellPath, 4);
    if (cell.status !== "ok" || fields === null) {
      pending = true;
      continue;
    }
    const cells = shape.kind === "schema" ? schemaCells(doc, db, fields) : rowCells(doc, fields, shape, columns.length);
    if (cells === "wrong") return null;
    if (cells === "wait") {
      pending = true;
      continue;
    }
    rows.push({ cells, path: cellPath, offsetBits: cell.node.offset_bits, sizeBits: cell.node.size_bits });
  }
  if (shape.kind === "table") {
    // Rows go in the order the table has them, not the order the page keeps
    // them: a b-tree leaf fills from the back, so the last row is first in the
    // file. "Stored at" is where the physical order shows.
    const at = shape.rowidAt;
    if (at >= 0) rows.sort((a, b) => Number(a.cells[at]?.text ?? 0) - Number(b.cells[at]?.text ?? 0));
  }
  return { columns, rows, pending };
}

/** One row of `sqlite_master`, whose `rootpage` names the page its object's
 *  b-tree starts on. That link is rule 7's other cross-reference. */
function schemaCells(doc: Doc, db: Database, cell: Map<string, TemplateNode>): RecordCell[] | "wait" {
  const payload = cell.get("payload");
  if (payload === undefined) return "wait";
  const record = fieldsOf(doc, payload.path, 8);
  if (record === null) return "wait";
  return SCHEMA_COLUMNS.map((name) => {
    const field = record.get(name);
    if (field === undefined) return { text: "", kind: "unread" };
    if (name !== "rootpage") return { text: field.value, kind: field.kind };
    // A view or a trigger has no b-tree and stores 0, which points nowhere.
    return pageCell(db, field.value);
  });
}

/** One row of a user table, or one key of an index. */
function rowCells(
  doc: Doc,
  cell: Map<string, TemplateNode>,
  shape: Shape,
  want: number,
): RecordCell[] | "wait" | "wrong" {
  const cells = keyCells(doc, cell, want);
  if (typeof cells === "string" || shape.kind !== "table" || shape.rowidAt < 0) return cells;
  // An INTEGER PRIMARY KEY column is stored as a null; its value is the row's
  // own number, which is written once at the front of the cell.
  const out = [...cells];
  out[shape.rowidAt] = { text: cell.get(ROWID)?.value ?? "", kind: "int" };
  return out;
}

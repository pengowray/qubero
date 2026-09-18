// Whether a list reads as a table, and what its rows and columns are.
//
// Three answers, tried in that order, because they know different amounts:
//
//   1. The template said so. A `TableShape` on the field says how many
//      elements make a row, what the columns are called, what one row is and
//      how fast the rows come. Nothing here has to recognise a format.
//   2. `records.ts` said so. SQLite and GGUF keep their columns somewhere no
//      declaration could point at, and those readers already find them; a plan
//      of theirs is wrapped as rows here rather than built again.
//   3. The elements look like one. A list whose first few elements are records
//      with the same field names in the same order is a table whether or not
//      anybody wrote that down, which is what gets dBase and every other
//      record file a table without a shape.
//
// The plan is lazy on purpose: a ten-minute stereo WAV is twenty-six million
// rows, and the count, the columns and the facts are all answerable without
// reading one of them. Rows arrive a window at a time, and a row whose bytes
// are still coming answers null rather than a guess.
//
// No DOM here, so the naming, the grouping arithmetic and the heuristic can be
// run under `node --test`. `tableview.ts` is the half that draws.

import type { Doc, FrameCell, TableFact, TableShape, TemplateNode } from "./doc.ts";
import { isRecordList, recordTable, type RecordCell, type RecordTable } from "./records.ts";
import { childWord, countText, TABLE } from "./strings.ts";

/** One column of the table: what it is called, and what it is measured in.
 *  The unit is a UCUM code and is empty for a column that has none. */
export type TableColumn = {
  readonly name: string;
  readonly unit: string;
};

/** One row, and where its bytes are so the reader can go to them. The cells
 *  are `records.ts`'s, so a table drawn here and a record table drawn in the
 *  listing colour their values the same way. */
export type TableRow = {
  readonly cells: readonly RecordCell[];
  readonly offsetBits: number;
  readonly sizeBits: number;
  /** The field the row was read from, which is what picking it goes to. */
  readonly path: readonly number[];
};

export type TablePlan = {
  readonly path: readonly number[];
  /** How many rows there are. */
  readonly count: number;
  /** What one row is: a sample, a record, a value. */
  readonly rowWord: string;
  readonly columns: readonly TableColumn[];
  /** What the shape calls one column, for the sentence saying what a row is.
   *  Null for a table whose columns are named individually or not at all. */
  readonly columnWord: string | null;
  /** The fields that describe the table, shown above it. Empty for a table
   *  nothing describes, which is every table without a shape. */
  readonly facts: readonly TableFact[];
  /** Rows a second, when the rows are spaced in time. */
  readonly rate: number | null;
  /** Row `i`, or null while its bytes are still being read. */
  row: (i: number) => TableRow | null;
  /** Which row a bit of the file falls in, or null for a bit outside the
   *  table. The cursor moving is the one question the view cannot answer for
   *  itself: how a row is put together out of elements is the plan's business,
   *  and a table of a format reader's rows is not made of elements at all. */
  rowFor: (bit: number) => number | null;
  /** Throw away what has been read. The plan has no listener of its own: what
   *  a row said may have been read from bytes that have since arrived or been
   *  edited, and the view holding the plan is the one that hears about it. */
  forget: () => void;
};

/** How many elements are read in one go. One call covers a screenful several
 *  times over, which is what keeps a scroll from being one wait per row. */
const WINDOW = 256;

/** How many elements the heuristic looks at before deciding. Enough that a
 *  list whose first element is unlike the rest is caught, few enough that
 *  asking costs one read. */
const SAMPLED = 4;

// ----- the pure half: naming, arithmetic, and the shape of a record -----

/** Which elements make row `i` when `columns` of them do. Half-open, and
 *  clamped to the end: the last row of a list that does not divide evenly is
 *  short rather than reaching past the list. */
export function rowRange(i: number, columns: number, count: number): { readonly from: number; readonly to: number } {
  const from = i * columns;
  return { from, to: Math.min(count, from + columns) };
}

/** How many rows `count` elements make, at `columns` elements each. */
export function rowCount(count: number, columns: number): number {
  return columns <= 1 ? count : Math.ceil(count / columns);
}

/**
 * What the columns of a shaped table are called.
 *
 * The names are used only when there are exactly as many of them as a row has
 * columns. A file whose channel count disagrees with the names the template
 * wrote is a file where one name would sit over another column's data, and a
 * wrong name is worse than a number: `channel 1` says nothing untrue.
 */
export function shapeColumns(shape: TableShape, columns: number): TableColumn[] {
  const named = shape.names.length === columns;
  const unit = (i: number): string => (shape.units.length === columns ? (shape.units[i] ?? "") : "");
  const word = shape.column_word ?? TABLE.columnFallback;
  return Array.from({ length: columns }, (_, i) => ({
    name: named ? (shape.names[i] ?? TABLE.column(word, i + 1)) : TABLE.column(word, i + 1),
    unit: unit(i),
  }));
}

/**
 * The name of one element of a list, as a column heading.
 *
 * A list's elements arrive named by their place in it, and where the format
 * named them as well the name follows the index: `[0] NAME` is the dBase field
 * called NAME. The index is what the row already is, so only the name is kept.
 * An element with nothing but an index keeps it, since a column has to be
 * called something.
 */
export function columnNameOf(name: string): string {
  const named = /^\[\d+\]\s+(.+)$/.exec(name);
  return named?.[1] ?? name;
}

/** What one cell of a row says: a value, or the count of what is under a field
 *  holding more fields. A wrong value is carried through so the table can mark
 *  it, which for a run of samples is the only view that ever shows it. */
export function cellOf(n: TemplateNode): RecordCell {
  const text = n.composite ? countText(n.child_count, childWord(n)) : n.value;
  return n.problem === undefined ? { text, kind: n.kind } : { text, kind: n.kind, problem: n.problem };
}

/**
 * The columns one element of a list flattens to, in order.
 *
 * One level of nesting is expanded, and one only: dBase keeps a record as
 * `{deleted, values[]}`, and the columns a reader wants are `deleted` and the
 * named values inside it, not a cell reading `12 values`. Expanding further
 * would run a whole tree into one row.
 *
 * `elements` is asked for the elements of a nested list, and answers null
 * while they are still being read, which makes the whole answer null: a row
 * half flattened would have its cells under the wrong headings.
 */
export function flatFields(
  children: readonly TemplateNode[],
  elements: (child: TemplateNode) => readonly TemplateNode[] | null,
): TemplateNode[] | null {
  const out: TemplateNode[] = [];
  for (const child of children) {
    if (!child.list || child.child_count === 0) {
      out.push(child);
      continue;
    }
    const inner = elements(child);
    if (inner === null) return null;
    out.push(...inner);
  }
  return out;
}

/** The headings those fields make. */
export function flatNames(fields: readonly TemplateNode[]): string[] {
  return fields.map((f) => columnNameOf(f.name));
}

/**
 * Whether a run of elements is uniform enough to be rows of one table.
 *
 * The same names in the same order, at least twice, with at least two columns
 * to put under them. Two is the floor because one column of values is a list,
 * and the pane already shows a list better than a table of one column would.
 */
export function uniformColumns(rows: readonly (readonly string[])[]): string[] | null {
  const first = rows[0];
  if (first === undefined || first.length < 2 || rows.length < 2) return null;
  for (const row of rows.slice(1)) {
    if (row.length !== first.length || !row.every((name, i) => name === first[i])) return null;
  }
  return [...first];
}

/**
 * How many decimals a time column needs for two rows in a row to differ.
 *
 * At 44,100 rows a second neighbouring rows are 23 microseconds apart, and a
 * column showing three decimals would print the same number for forty-four of
 * them. Nine is the cap: past that the digits are longer than the double they
 * came from is exact.
 */
export function timeDigits(rate: number): number {
  if (!(rate > 0)) return 0;
  return Math.min(9, Math.ceil(Math.log10(rate)) + 1);
}

/** Where row `i` falls in time, in seconds. */
export function timeText(i: number, rate: number): string {
  const digits = timeDigits(rate);
  return (i / rate).toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

/**
 * How wide a column is drawn, in characters, and which way its values sit.
 *
 * A track list has to be one list shared by the header and every row, so the
 * widths cannot be measured row by row the way a real table's are. They are
 * worked out from the rows that have been read instead: the heading to start
 * with, widened by the longest value seen and never narrowed, so a table that
 * has settled does not shift under a scroll. The cap keeps one long string
 * from making a column a screen wide; the rest is on hover.
 *
 * `numeric` is decided by the first value seen and then kept, since a column
 * that changed sides as the rows came in would be worse than one that sits
 * on the wrong side. Null until a value has been seen.
 */
export type ColumnFit = {
  readonly width: number;
  readonly numeric: boolean | null;
};

/** Narrower than this and a heading has nowhere to go. */
export const FIT_MIN = 6;
/** Wider than this and one column is most of the tab. */
export const FIT_MAX = 40;

/** A column's fit before any row has been read: its heading's width. */
export function fitOf(header: string): ColumnFit {
  return { width: Math.min(FIT_MAX, Math.max(FIT_MIN, header.length)), numeric: null };
}

/** The same fit after one more cell of the column has been seen. Returns the
 *  fit it was given when nothing changed, so a caller can tell by identity. */
export function fitCell(fit: ColumnFit, cell: RecordCell | undefined): ColumnFit {
  if (cell === undefined) return fit;
  const width = Math.min(FIT_MAX, Math.max(fit.width, cell.text.length));
  const numeric = fit.numeric ?? isNumberKind(cell.kind);
  return width === fit.width && numeric === fit.numeric ? fit : { width, numeric };
}

/** Whether a value of this kind reads as a number, which is what decides the
 *  side of the column it sits against: digits line up on the right. The same
 *  kinds `fieldstyle.ts` colours as numbers. */
export function isNumberKind(kind: string): boolean {
  return kind === "uint" || kind === "int" || kind === "float" || kind === "unset";
}

/** How wide the row-number column is, for the last row's number: a table of
 *  twenty-six million rows needs room for the separators too. */
export function indexWidth(count: number): number {
  return Math.max(3, Math.max(0, count - 1).toLocaleString().length);
}

/** How wide the time column is, for the last row's time. */
export function timeWidth(count: number, rate: number): number {
  return Math.max(TABLE.time.length, timeText(Math.max(0, count - 1), rate).length);
}

// ----- asking the file -----

/** The elements of a list, read a window at a time and kept until the file
 *  changes. Every plan holds one; a window is one call for a screenful rather
 *  than one wait per row. */
class Elements {
  private readonly have = new Map<number, TemplateNode>();
  private readonly doc: Doc;
  private readonly path: readonly number[];
  private readonly count: number;

  constructor(doc: Doc, path: readonly number[], count: number) {
    this.doc = doc;
    this.path = path;
    this.count = count;
  }

  clear(): void {
    this.have.clear();
  }

  /** Element `i`, or null while it is being read. */
  at(i: number): TemplateNode | null {
    const known = this.have.get(i);
    if (known !== undefined) return known;
    if (i < 0 || i >= this.count) return null;
    const from = Math.floor(i / WINDOW) * WINDOW;
    const to = Math.min(this.count, from + WINDOW);
    const reply = this.doc.templateChildren(this.path, from, to);
    // Not an answer, so nothing is kept: the draw after the bytes land asks
    // again, and `Doc` has already gone for them.
    if (reply.status !== "ok") return null;
    reply.node.forEach((node, at) => this.have.set(from + at, node));
    return this.have.get(i) ?? null;
  }
}

/** The children of one node, all of them, or null while they are being read. */
function childrenOf(doc: Doc, node: TemplateNode): readonly TemplateNode[] | null {
  if (node.child_count === 0) return [];
  const reply = doc.templateChildren(node.path, 0, node.child_count);
  return reply.status === "ok" ? reply.node : null;
}

/** Which element of a list a bit falls in, by walking the tree down to it.
 *  A list is its own index, so this is a step of the path rather than a
 *  search. Null for a bit that is not inside the list at all. */
function elementAt(doc: Doc, path: readonly number[], bit: number): number | null {
  const at = doc.locate(bit);
  if (at.status !== "ok" || at.node.length <= path.length) return null;
  if (!path.every((step, i) => at.node[i] === step)) return null;
  return at.node[path.length] ?? null;
}

/** One element of a list flattened into the fields a row is made of. */
function rowFields(doc: Doc, element: TemplateNode): TemplateNode[] | null {
  const kids = childrenOf(doc, element);
  if (kids === null) return null;
  return flatFields(kids, (child) => childrenOf(doc, child));
}

// ----- the three plans -----

/** A list the template gave a shape: the columns, the row word, the rate and
 *  the facts all come from it, and a row is however many elements it says. */
function shapedPlan(doc: Doc, node: TemplateNode, shape: TableShape): TablePlan | null {
  const columns = Math.max(1, shape.columns ?? 1);
  const elements = new Elements(doc, node.path, node.child_count);
  const headings = shapedColumns(doc, node, shape, columns, elements);
  if (headings === null) return null;
  return {
    path: node.path,
    forget: () => elements.clear(),
    rowFor: (bit) => {
      const at = elementAt(doc, node.path, bit);
      return at === null ? null : Math.floor(at / columns);
    },
    count: rowCount(node.child_count, columns),
    rowWord: shape.row_word ?? childWord(node),
    columns: headings,
    columnWord: shape.column_word,
    facts: shape.facts,
    rate: shape.rate,
    row: (i) => {
      const { from, to } = rowRange(i, columns, node.child_count);
      const cells: RecordCell[] = [];
      let first: TemplateNode | null = null;
      let last: TemplateNode | null = null;
      for (let at = from; at < to; at++) {
        const element = elements.at(at);
        if (element === null) return null;
        // A composite element is a record of its own and flattens the way the
        // heuristic's rows do; a scalar one is a value and is one cell.
        if (element.composite) {
          const fields = rowFields(doc, element);
          if (fields === null) return null;
          cells.push(...fields.map(cellOf));
        } else cells.push(cellOf(element));
        first ??= element;
        last = element;
      }
      if (first === null || last === null) return null;
      return {
        cells,
        offsetBits: first.offset_bits,
        sizeBits: last.offset_bits + last.size_bits - first.offset_bits,
        path: [...node.path, from],
      };
    },
  };
}

/**
 * The columns of a shaped table.
 *
 * A shape that groups elements into rows names the columns itself: `columns`
 * channels, called what `names` says. A shape that groups nothing is a claim
 * about a list of records, dBase's, and the columns are the record's fields,
 * read from the first element the way the heuristic reads them; the shape
 * still says what a row is called and what describes the table. The names in
 * the shape are used over those only when there are as many, which for a
 * format whose fields are named in the file there never are. Null while the
 * first element is being read.
 */
function shapedColumns(doc: Doc, node: TemplateNode, shape: TableShape, columns: number, elements: Elements): TableColumn[] | null {
  if (shape.columns !== null || node.child_count === 0) return shapeColumns(shape, columns);
  const first = elements.at(0);
  if (first === null) return null;
  if (!first.composite) return shapeColumns(shape, columns);
  const fields = rowFields(doc, first);
  if (fields === null) return null;
  const names = flatNames(fields);
  if (shape.names.length === names.length) return shapeColumns(shape, names.length);
  return names.map((name) => ({ name, unit: "" }));
}

/** A table one of the format readers already builds. It answers with every row
 *  at once, so the plan is a window onto what it made rather than a lazy read
 *  of its own. */
function recordsPlan(doc: Doc, node: TemplateNode): TablePlan | null {
  let built: RecordTable | null = recordTable(doc, node);
  if (built === null) return null;
  const table = (): RecordTable | null => (built ??= recordTable(doc, node));
  return {
    path: node.path,
    forget: () => {
      built = null;
    },
    // A format reader's rows are not elements of anything: a SQLite page's
    // cells are scattered through it and one row can even be a field of the
    // page header. So the row holding a bit is found by asking the rows where
    // they are, which is cheap because a page holds tens of them, not
    // millions.
    rowFor: (bit) => {
      const rows = table()?.rows ?? [];
      const at = rows.findIndex((r) => bit >= r.offsetBits && bit < r.offsetBits + r.sizeBits);
      return at < 0 ? null : at;
    },
    count: built.rows.length,
    rowWord: built.rowWord ?? childWord(node),
    columns: built.columns.map((name) => ({ name, unit: "" })),
    columnWord: null,
    facts: [],
    rate: null,
    row: (i) => {
      const rows = table()?.rows;
      const row = rows?.[i];
      if (row === undefined) return null;
      return { cells: row.cells, offsetBits: row.offsetBits, sizeBits: row.sizeBits, path: row.path };
    },
  };
}

/** The names the first few elements of a list flatten to, for the heuristic
 *  and for the columns it settles on. Null while any of them is being read. */
function sampledNames(doc: Doc, node: TemplateNode, elements: Elements): string[][] | null {
  const looked = Math.min(SAMPLED, node.child_count);
  const rows: string[][] = [];
  for (let i = 0; i < looked; i++) {
    const element = elements.at(i);
    if (element === null || !element.composite) return null;
    const fields = rowFields(doc, element);
    if (fields === null) return null;
    rows.push(flatNames(fields));
  }
  return rows;
}

/** A list nobody declared, whose elements are records of the same shape. */
function guessedPlan(doc: Doc, node: TemplateNode): TablePlan | null {
  if (!node.list || node.child_count < 2) return null;
  const elements = new Elements(doc, node.path, node.child_count);
  const names = sampledNames(doc, node, elements);
  if (names === null) return null;
  const columns = uniformColumns(names);
  if (columns === null) return null;
  return {
    path: node.path,
    forget: () => elements.clear(),
    rowFor: (bit) => elementAt(doc, node.path, bit),
    count: node.child_count,
    rowWord: childWord(node),
    columns: columns.map((name) => ({ name, unit: "" })),
    columnWord: null,
    facts: [],
    rate: null,
    row: (i) => {
      const element = elements.at(i);
      if (element === null) return null;
      const fields = rowFields(doc, element);
      if (fields === null) return null;
      return {
        cells: fields.map(cellOf),
        offsetBits: element.offset_bits,
        sizeBits: element.size_bits,
        path: [...node.path, i],
      };
    },
  };
}

/** The shape the template put on this field, or null for the field it did
 *  not, for the file that cannot answer yet, and for a `src/pkg` built before
 *  the core could be asked. */
function shapeOf(doc: Doc, node: TemplateNode): TableShape | null {
  if (node.table !== true) return null;
  const reply = doc.tableShape(node.path);
  const shape = reply.status === "ok" ? reply.node : null;
  // A shape that says where its cells are is not a run of values, so it is not
  // laid out from a count: `computedPlan` below reads one, and one of the
  // readers in `records.ts` walks the other.
  return shape !== null && shape.cells !== null ? null : shape;
}

/** The same shape, for the two cases that are not laid out from a count. */
function cellShapeOf(doc: Doc, node: TemplateNode): TableShape | null {
  if (node.table !== true) return null;
  const reply = doc.tableShape(node.path);
  const shape = reply.status === "ok" ? reply.node : null;
  return shape !== null && shape.cells !== null ? shape : null;
}

/**
 * A table whose cells the core works out, which is a pandas frame.
 *
 * Lazy like the shaped plan and for the same reason: a frame is as long as it
 * is, and the columns, the count and the row word are all answerable without
 * reading a cell. A window of rows is read at a time and kept until the view
 * says to forget it.
 */
/** How many columns a computed table that named none has: whatever one row
 *  came back with, since the core works the cells out and the rows are all the
 *  same width. One column for a table with no rows to ask about. */
function columnsOf(doc: Doc, node: TemplateNode, rows: number): number {
  if (rows === 0) return 1;
  const reply = doc.pickleCells(node.path, 0, 1);
  if (reply.status !== "ok") return 1;
  return Math.max(1, reply.node[0]?.length ?? 1);
}

function computedPlan(doc: Doc, node: TemplateNode, shape: TableShape, rows: number): TablePlan {
  let held: { from: number; cells: readonly (readonly FrameCell[])[] } | null = null;
  const read = (i: number): readonly FrameCell[] | null => {
    if (held === null || i < held.from || i >= held.from + held.cells.length) {
      const from = Math.floor(i / WINDOW) * WINDOW;
      const reply = doc.pickleCells(node.path, from, Math.min(from + WINDOW, rows));
      if (reply.status !== "ok") return null;
      held = { from, cells: reply.node };
    }
    return held.cells[i - held.from] ?? null;
  };
  return {
    path: node.path,
    count: rows,
    rowWord: shape.row_word ?? childWord(node),
    // A frame names its columns; an array's are places along an axis and it
    // names none, so those fall back the same way the ordinary table over a
    // run of numbers does rather than leaving the table with no columns.
    columns:
      shape.names.length > 0
        ? shape.names.map((name, i) => ({ name, unit: shape.units[i] ?? "" }))
        : shapeColumns(shape, columnsOf(doc, node, rows)),
    columnWord: shape.column_word,
    facts: shape.facts,
    rate: shape.rate,
    row: (i) => {
      const cells = read(i);
      if (cells === null) return null;
      return { cells: cells.map((c) => ({ text: c.text, kind: c.kind })), offsetBits: 0, sizeBits: 0, path: node.path };
    },
    // A frame's rows are not a run of bytes: one row is one value out of each
    // of several blocks, scattered through the file. So a bit of the file is
    // in no row rather than in the wrong one.
    rowFor: () => null,
    forget: () => {
      held = null;
    },
  };
}

/**
 * Whether this node is worth offering a table of. Asked of the field at the
 * cursor and of every ancestor above it, so it stops at the first answer and
 * never builds the rows.
 */
export function isTable(doc: Doc, node: TemplateNode): boolean {
  if (shapeOf(doc, node) !== null) return true;
  if (cellShapeOf(doc, node) !== null) return true;
  if (isRecordList(doc, node)) return true;
  return guessedPlan(doc, node) !== null;
}

/** How this node reads as a table, or null when it does not read as one. */
export function tablePlan(doc: Doc, node: TemplateNode): TablePlan | null {
  const shape = shapeOf(doc, node);
  if (shape !== null) return shapedPlan(doc, node, shape);
  const cells = cellShapeOf(doc, node);
  if (cells !== null && cells.cells?.kind === "computed") return computedPlan(doc, node, cells, cells.cells.rows);
  if (isRecordList(doc, node)) return recordsPlan(doc, node);
  return guessedPlan(doc, node);
}

// The columns of a table: how wide each is, which side its values sit, what
// its heading says, and the header row itself.
//
// A table here has one track list shared by the header and every row, so the
// widths cannot be measured row by row the way a real table's are. They are
// worked out from the rows that have been read instead: the heading to start
// with, widened by the longest value seen and never narrowed, so a table that
// has settled does not shift under a scroll. That makes the columns a thing
// with a memory of their own, kept here, away from the drawing.
//
// Turned, the same answers come out the other way round: a track a record, a
// line of heading for each thing said about one, written by the same
// `turnedLines` a copy uses so the two cannot drift apart.

import { el } from "./dom.ts";
import { PROBLEMS, TABLE } from "./strings.ts";
import { copiedAddress } from "./tableaddress.ts";
import { FIT_MAX, FIT_MIN, fitCell, fitOf, indexWidth, timeWidth, type ColumnFit, type TablePlan, type TableRow } from "./tableplan.ts";
import { headerCells, leadKinds, recordCells, turnedLines, type Lead } from "./tabletext.ts";

/** How wide a column of addresses and one of sizes start out, in characters.
 *  They widen to what is in them like any other column: a size of
 *  `4,000 bytes` in a column cut for `27 bytes` read `4,000 by...`, with
 *  nothing to say what the rest was. */
const AT_WIDTH = 9;
const SIZE_WIDTH = 8;

/** One data column as it is drawn: how wide it is and which side its values
 *  sit, its header cell, and how many wrong values its cells hold over the
 *  rows read so far, invalid then undefined.
 *
 *  One record rather than three arrays indexed alike, because the three
 *  answers change together: a count that grows rewrites a heading, and a
 *  heading that no longer fits widens the column. Apart, that was three
 *  lookups a step, each to be checked for a column that was not there. */
type Column = {
  fit: ColumnFit;
  readonly head: HTMLElement;
  problems: [invalid: number, undefined: number];
};

/** What the columns have to ask the view, which is what the reader has
 *  changed and what has been read so far. */
export type ColumnsOf = {
  readonly plan: TablePlan;
  /** Where the track list is set, and where the header row goes. */
  readonly el: HTMLElement;
  readonly head: HTMLElement;
  /** Which way round the table is drawn now. */
  turned: () => boolean;
  addresses: () => boolean;
  /** Rows a second, or null for rows not spaced in time. */
  rate: () => number | null;
  /** The columns that are about a record rather than in it. */
  lead: () => Lead;
  /** The rows read so far, which is what the widths are worked out from. */
  readonly have: ReadonlyMap<number, TableRow>;
  /** Every record, or null while any of them is still being read. Only asked
   *  when the table is drawn turned. */
  records: () => readonly TableRow[] | null;
};

export class TableColumns {
  private readonly of: ColumnsOf;
  private readonly columns: Column[];
  /** How wide the column of row names is, in a table whose rows have them. */
  private nameFit: ColumnFit = fitOf(TABLE.rowName);
  /** How wide the two address columns are: as wide as the widest seen. */
  private atWidth = Math.max(AT_WIDTH, TABLE.storedAt.length);
  private sizeWidth = Math.max(SIZE_WIDTH, TABLE.size.length);

  constructor(of: ColumnsOf) {
    this.of = of;
    this.columns = of.plan.columns.map((_, c) => ({
      fit: fitOf(this.headingOf(c)),
      head: el("span", { className: "tbl-th" }),
      problems: [0, 0],
    }));
  }

  /** How many data columns there are, which is how many rows a turned table
   *  draws. */
  get length(): number {
    return this.columns.length;
  }

  /** Whether the values of column `c` read as numbers and so sit against the
   *  right edge. */
  numeric(c: number): boolean {
    return this.columns[c]?.fit.numeric === true;
  }

  /** The heading of one data column, unit included. */
  headingOf(c: number): string {
    const column = this.of.plan.columns[c];
    if (column === undefined) return "";
    return column.unit === "" ? column.name : `${column.name} (${column.unit})`;
  }

  /** Every heading, as an export and a copy want them. */
  headings(): string[] {
    return this.columns.map((_, c) => this.headingOf(c));
  }

  /** The heading as it is drawn: the column's name and unit, then how many of
   *  its cells hold a wrong value. A reader scrolling a run of samples sees
   *  one marked cell at a time and no way to tell whether it is the only one,
   *  so the count is what says how much there is to look for. */
  headTextOf(c: number): string {
    const column = this.columns[c];
    if (column === undefined) return this.headingOf(c);
    const more = PROBLEMS.column(column.problems[0], column.problems[1], this.of.have.size < this.of.plan.count);
    return more === "" ? this.headingOf(c) : `${this.headingOf(c)} ${more}`;
  }

  /** The counts go when the rows they were made from go: a heading still
   *  reading `2 invalid so far` after those rows have been thrown away would
   *  be saying something nothing holds up. */
  forget(): void {
    for (const column of this.columns) column.problems = [0, 0];
    this.writeHeads();
  }

  /** A row has been read: widen any column it does not fit, settle the side of
   *  any column whose first value this is, and add whatever is wrong in it to
   *  its columns' counts. The track list is written again only when a width
   *  changed, which is a few times at the start and then not at all. */
  read(row: TableRow): void {
    this.fitRow(row);
    this.countRow(row);
  }

  /** One track list for the header and every row, so the two line up without
   *  either measuring the other. Every track is a fixed width: the data
   *  columns are as wide as what has been seen in them (see `ColumnFit`), so a
   *  table of one channel is a narrow table and not one number adrift in a
   *  tab-wide column. */
  lay(): void {
    const widths = this.of.turned() ? this.turnedWidths() : this.widths();
    this.of.el.style.setProperty("--tbl-cols", widths.map((w) => `${w}ch`).join(" "));
  }

  /** The header row, assembled from the column headings and whichever of the
   *  index, time and address cells this table shows. The headings are the same
   *  elements every time, so what a column has learnt about itself (its width,
   *  its side, its count) survives the addresses being turned on. */
  fillHead(): void {
    this.writeHeads();
    if (this.of.turned()) return this.fillTurnedHead();
    const plan = this.of.plan;
    const cells: HTMLElement[] = [el("span", { className: "tbl-th tbl-index tbl-num", textContent: TABLE.index })];
    if (plan.rowNames) cells.push(el("span", { className: "tbl-th", textContent: TABLE.rowName }));
    if (this.of.rate() !== null) cells.push(el("span", { className: "tbl-th tbl-num", textContent: TABLE.time }));
    cells.push(...this.columns.map((column) => column.head));
    if (this.of.addresses()) {
      cells.push(el("span", { className: "tbl-th", textContent: TABLE.storedAt }));
      cells.push(el("span", { className: "tbl-th tbl-num", textContent: TABLE.size }));
    }
    this.of.head.replaceChildren(el("div", { className: "tbl-headline" }, ...cells));
  }

  // ----- the widths -----

  /** The tracks of the table the right way up, in `headerCells`' order. */
  private widths(): number[] {
    const plan = this.of.plan;
    const rate = this.of.rate();
    const out = [indexWidth(plan.count)];
    if (plan.rowNames) out.push(this.nameFit.width);
    if (rate !== null) out.push(timeWidth(plan.count, rate));
    for (const column of this.columns) out.push(column.fit.width);
    if (this.of.addresses()) out.push(this.atWidth, this.sizeWidth);
    return out;
  }

  /** The tracks of the table turned: the labels, then a column a record, each
   *  as wide as the widest thing in it, heading lines included. Before the
   *  records are all read a column is as wide as its number. */
  private turnedWidths(): number[] {
    const clamp = (n: number): number => Math.min(FIT_MAX, Math.max(FIT_MIN, n));
    const lead = this.of.lead();
    // Loops, not `Math.max(...lengths)`: a strip of samples has a field for
    // every sample, and an argument for each of a hundred thousand of them is
    // more than a call can take.
    let labels = 0;
    for (const text of headerCells([], lead)) labels = Math.max(labels, text.length);
    for (let c = 0; c < this.columns.length; c++) labels = Math.max(labels, this.headTextOf(c).length);
    const out = [clamp(labels)];
    for (let i = 0; i < this.of.plan.count; i++) {
      const row = this.of.have.get(i);
      if (row === undefined) {
        out.push(clamp(i.toLocaleString().length));
        continue;
      }
      let widest = 0;
      for (const text of recordCells(i, row, 0, lead)) widest = Math.max(widest, text.length);
      for (const cell of row.cells) widest = Math.max(widest, cell.text.length);
      out.push(clamp(widest));
    }
    return out;
  }

  // ----- what a row teaches the columns -----

  private fitRow(row: TableRow): void {
    let widened = false;
    if (row.name !== undefined && row.name.length > this.nameFit.width) {
      this.nameFit = { width: Math.min(FIT_MAX, row.name.length), numeric: false };
      widened = true;
    }
    // Measured from what the columns will say, which for a row whose cells are
    // apart is `per cell` and for one with no bytes is a word.
    const [address, sized] = copiedAddress(row);
    if (address.length > this.atWidth || sized.length > this.sizeWidth) {
      this.atWidth = Math.max(this.atWidth, address.length);
      this.sizeWidth = Math.max(this.sizeWidth, sized.length);
      widened = widened || this.of.addresses();
    }
    for (const [c, column] of this.columns.entries()) {
      const was = column.fit;
      const now = fitCell(was, row.cells[c]);
      if (now === was) continue;
      column.fit = now;
      if (now.width !== was.width) widened = true;
      if (now.numeric !== was.numeric) column.head.classList.toggle("tbl-num", now.numeric === true);
    }
    if (widened) this.lay();
  }

  /** The heading is allowed to widen its column, since a count nobody can read
   *  is not a count. */
  private countRow(row: TableRow): void {
    let widened = false;
    // Every heading is rewritten once the last row arrives, because that is
    // when `so far` comes off all of them at once.
    const settled = this.of.have.size >= this.of.plan.count;
    for (const [c, column] of this.columns.entries()) {
      const problem = row.cells[c]?.problem;
      if (problem === undefined && !settled) continue;
      if (problem !== undefined) column.problems[problem.tier === "invalid" ? 0 : 1]++;
      widened = this.writeHead(c) || widened;
    }
    if (widened) this.lay();
  }

  /** Every heading, drawn again with what it says now. */
  private writeHeads(): void {
    let widened = false;
    for (let c = 0; c < this.columns.length; c++) widened = this.writeHead(c) || widened;
    if (widened) this.lay();
  }

  /** One heading, drawn again with what it says now. True when the column had
   *  to grow to hold it. */
  private writeHead(c: number): boolean {
    const column = this.columns[c];
    if (column === undefined) return false;
    const text = this.headTextOf(c);
    if (column.head.textContent === text) return false;
    column.head.textContent = text;
    column.head.title = text;
    const width = Math.min(FIT_MAX, Math.max(column.fit.width, text.length));
    if (width === column.fit.width) return false;
    column.fit = { width, numeric: column.fit.numeric };
    return true;
  }

  /**
   * The headings of a turned table: a line for each thing that is said about
   * a record, with a cell for every record. What is a column beside the values
   * the right way up (the number, the name, the time, the address) is a line
   * above them here, written by the same code that writes a copy of them.
   *
   * Until the records have all been read only their numbers are known, so
   * that is the one line there is.
   */
  private fillTurnedHead(): void {
    const records = this.of.records();
    const lead = this.of.lead();
    const lines =
      records === null
        ? [[TABLE.index, ...Array.from({ length: this.of.plan.count }, (_, i) => String(i))]]
        : turnedLines([], records, lead, { from: 0, to: 0 });
    const kinds = leadKinds(lead);
    // A column of numbers has its headings over the digits.
    const numeric = this.columns.every((column) => column.fit.numeric === true);
    const out = lines.map((line, at) => {
      const kind = kinds[at] ?? "index";
      const tint = kind === "index" || kind === "at" ? " tbl-at" : kind === "size" ? " tbl-size" : "";
      const cells = line.map((text, c) => {
        if (c === 0) return el("span", { className: "tbl-th tbl-label", textContent: text });
        const said = kind === "index" ? Number(text).toLocaleString() : text;
        const cell = el("span", { className: `tbl-th${tint}${numeric ? " tbl-num" : ""}`, textContent: said });
        cell.title = said;
        return cell;
      });
      return el("div", { className: "tbl-headline" }, ...cells);
    });
    this.of.head.replaceChildren(...out);
  }
}

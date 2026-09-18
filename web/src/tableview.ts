// A list of records, or a run of samples, read as a table in a tab of its own.
//
// The first view here that is an abstraction away from the bytes: a row is a
// record or a sample, not a stretch of the file. What is on screen is what the
// format says the data is, and the bytes are an option the reader turns on --
// two extra columns saying where each row is kept -- rather than the frame
// everything else sits in. Clicking a row still puts the file tab's cursor on
// it, because a link back to the bytes that is always there is worth more than
// a column that says the same thing in every row.
//
// The rows are virtual the way `listpane.ts`'s are: one height each, so which
// ones are on screen is division rather than search. What is new here is the
// scroll mapping. Ten minutes of stereo sound is twenty-six million rows,
// which is six hundred million pixels of canvas, and browsers stop honouring
// an element's height long before that. Past the cap the canvas stops growing
// and the scroll bar is read as a ratio instead: where it sits in its travel
// is where the reader is in the rows, and the rows on screen are drawn against
// the viewport rather than against the canvas.
//
// The column headings are inside the scroller, stuck to its top edge. They
// used to sit above it as a sibling, which kept them in view going down and
// left them behind going across: a table five hundred columns wide scrolled
// its values out from under headings that never moved, and the headings, too
// wide for the tab, made the page itself scroll sideways through nothing.
// Inside, one scroll position moves both, and the sheet holding them is as
// wide as the columns and no wider.
//
// A table can be drawn turned: a column for each record and a row for each
// field, which is how two channels of five hundred values want to be read.
// What the table MEANS does not turn with it. The plan still answers in
// records, and everything here that is written out as text is made by
// `tabletext.ts` from the table the right way up and then read the other way.
// What does turn is everything the reader sees and touches: the rows that
// scroll, the rows a click selects, the headings, and the facts about each
// record (its number, its name, its time, where it is stored), which are
// columns beside the values one way up and lines of heading above them the
// other.

import { formatOffset } from "./doc.ts";
import type { Doc } from "./doc.ts";
import { el } from "./dom.ts";
import { fieldClass } from "./fieldstyle.ts";
import type { RecordCell } from "./records.ts";
import { bitSizeText, PROBLEMS, REPORT, TABLE } from "./strings.ts";
import { rememberChoice, storedText } from "./stored.ts";
import { canTurn, FIT_MAX, FIT_MIN, fitCell, fitOf, indexWidth, startsTurned, timeText, timeWidth, TURN_MAX, turnsByDefault, type ColumnFit, type TablePlan, type TableRow } from "./tableplan.ts";
import { headerCells, leadKinds, recordCells, tsvLine, turnedLines, type Lead } from "./tabletext.ts";

/** Height of one row, which must match `--tbl-row` in the stylesheet: the rows
 *  are placed by arithmetic on it, so a row that drew taller would slide out
 *  from under its own place. */
const ROW = 22;
/** Rows drawn above and below the window, so a wheel notch has somewhere to go
 *  before the next paint. */
const OVERSCAN = 8;
/** How tall the canvas is allowed to get. Past this the scroll bar stands for
 *  the rows by ratio rather than by pixels. */
const MAX_CANVAS = 16_000_000;
/** How many rows one copy may hold. The text is built in memory before the
 *  clipboard sees it, and a hundred thousand rows of samples is already a few
 *  megabytes; a reader after more than that wants an export, not a paste. */
const COPY_LIMIT_ROWS = 100_000;
/** How long a notice stays up before it goes away on its own. */
const NOTICE_MS = 5000;
/** Whether the reader last left the address columns on. Per browser, not per
 *  table: it is a way of reading, and a reader who wants the bytes wants them
 *  for the next file too. */
const ADDRESSES_KEY = "qubero.table.addresses";
/** Whether the reader wants the tables that arrive turned to arrive turned.
 *  Kept for those tables only; see `startsTurned`. */
const TURNED_KEY = "qubero.table.turned";
/** How wide a column of addresses and one of sizes are, in characters. */
const AT_WIDTH = 12;
const SIZE_WIDTH = 9;

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

/** What picking a row hands on: the row's field, and the bits it covers. */
export type TablePick = {
  readonly path: readonly number[];
  readonly startBit: number;
  readonly endBit: number;
};

/** The one mark both tiers wear, the same shape the listing and the hex chips
 *  use. It only says look here; the words are on the cell's hover and the
 *  colour repeats the tier. Hidden from a screen reader, which gets the
 *  words. See docs/DESIGN-wrong-values.md. */
function glyph(invalid: boolean): HTMLElement {
  const dot = el("span", {
    className: `problem-glyph ${invalid ? "is-invalid" : "is-undefined"}`,
    textContent: PROBLEMS.glyph,
  });
  dot.setAttribute("aria-hidden", "true");
  return dot;
}

export class TableView {
  readonly el: HTMLElement;
  private readonly doc: Doc;
  private readonly plan: TablePlan;
  private readonly head: HTMLElement;
  private readonly scroller: HTMLElement;
  private readonly canvas: HTMLElement;
  /** The rows already read, by index. Cleared whenever the file changes, since
   *  that is when what one of them says can stop being true. */
  private readonly have = new Map<number, TableRow>();
  /** The data columns, in the order they are drawn. Built once and kept: the
   *  header cells are the same elements from then on, so a column found to be
   *  numeric is turned to face its values without the header being built
   *  again. */
  private readonly columns: Column[];
  private addresses = false;
  /** Whether the table is drawn turned, a record to a column. */
  private turned = false;
  /** Turned, whether the headings and the widths have been worked out from
   *  the records. They need every record, so they wait for the last of them
   *  and are done once. */
  private turnedLaid = false;
  /** Turned, the record whose column the reader last clicked in: a row is
   *  one field of every record, and this is the one whose bytes a pick goes
   *  to. */
  private pickedRecord = 0;
  /** How wide the column of row names is, in a table whose rows have them. */
  private nameFit: ColumnFit = fitOf(TABLE.rowName);
  private readonly meaning: HTMLElement;
  /** The selected rows, as they are drawn: records, or fields when turned. the one the selection started on, and the one it was
   *  last extended to. Equal for a single row; the range runs between them
   *  either way round. Null when nothing is selected. */
  private anchor: number | null = null;
  private focus: number | null = null;
  private readonly copyButton: HTMLButtonElement;
  private readonly notice: HTMLElement;
  private noticeTimer = 0;
  /** True while a pick this view made is being sent out, so the cursor move it
   *  causes does not come back and undo the scroll position. */
  private picking = false;
  private drawn: { from: number; to: number; base: number; addresses: boolean } | null = null;
  private frame = 0;

  /** The reader picked a row. */
  onPick: (pick: TablePick) => void = () => {};
  /** The reader followed one of the facts above the table, or a link in a
   *  cell, to the field it names. */
  onFactPick: (path: readonly number[]) => void = () => {};

  constructor(doc: Doc, plan: TablePlan, opts: { readonly title: string }) {
    this.doc = doc;
    this.plan = plan;
    this.addresses = storedText(ADDRESSES_KEY) === "1";
    this.turned = startsTurned(plan, storedText(TURNED_KEY));
    this.meaning = el("span", { className: "tbl-meaning" });
    this.el = el("div", { className: "tableview" });
    this.head = el("div", { className: "tbl-head" });
    this.scroller = el("div", { className: "tbl-scroll" });
    this.scroller.tabIndex = 0;
    this.canvas = el("div", { className: "tbl-canvas" });
    // The sheet is as wide as its columns. The header is what sizes it, being
    // the one thing in it that is laid out in the ordinary flow: the rows are
    // placed by arithmetic and take whatever width the sheet has.
    this.scroller.append(el("div", { className: "tbl-sheet" }, this.head, this.canvas));
    this.copyButton = el("button", { type: "button", className: "tbl-copy" });
    this.copyButton.addEventListener("click", () => void this.copySelection());
    this.notice = el("div", { className: "tbl-notice", hidden: true });
    this.el.append(this.bar(opts.title), this.scroller, this.notice);
    this.refreshCopy();
    this.columns = plan.columns.map((_, c) => ({
      fit: fitOf(this.headingOf(c)),
      head: el("span", { className: "tbl-th" }),
      problems: [0, 0],
    }));
    this.sizeCanvas();
    this.layColumns();
    this.fillHead();
    this.scroller.addEventListener("scroll", () => this.paint(), { passive: true });
    this.scroller.addEventListener("click", (e) => this.onClick(e));
    this.scroller.addEventListener("keydown", (e) => this.onKey(e));
    new ResizeObserver(() => this.paintAgain()).observe(this.scroller);
    // Bytes arriving turn a waiting row into a row; so does an edit.
    doc.onChange(() => this.schedule());
  }

  /** The page is in the document now, so the rows can be measured and drawn.
   *  Also every time the tab comes back to the front. */
  shown(): void {
    this.paintAgain();
  }

  // ----- the bar above the table -----

  private bar(title: string): HTMLElement {
    const bar = el("header", { className: "tbl-bar" });
    bar.append(el("b", { className: "tbl-title", textContent: title }));
    bar.append(el("span", { className: "tbl-count", textContent: TABLE.count(this.plan.count, this.plan.rowWord) }));
    for (const fact of this.plan.facts) {
      const button = el("button", {
        type: "button",
        className: "tbl-fact",
        textContent: `${fact.label} ${factValue(fact.value)}`,
      });
      button.title = TABLE.factTitle(fact.label);
      button.addEventListener("click", () => this.onFactPick(fact.path));
      bar.append(button);
    }
    if (this.rate !== null) bar.append(this.meaning);
    this.sayMeaning();
    const turn = el("input", { type: "checkbox", checked: this.turned, disabled: !canTurn(this.plan.count) });
    turn.addEventListener("change", () => this.turn(turn.checked));
    const turnLabel = el("label", { className: "tbl-check" }, turn, TABLE.turn(this.plan.rowWord));
    turnLabel.title = turn.disabled ? TABLE.turnTooMany(this.plan.rowWord, TURN_MAX) : TABLE.turnTitle(this.plan.rowWord);
    turnLabel.classList.toggle("is-off", turn.disabled);
    const box = el("input", { type: "checkbox", className: "tbl-addr-box", checked: this.addresses });
    box.addEventListener("change", () => {
      this.addresses = box.checked;
      rememberChoice(ADDRESSES_KEY, box.checked ? "1" : "0");
      this.layAgain();
    });
    // The controls sit together at the far end, away from the facts.
    bar.append(el("div", { className: "tbl-controls" }, turnLabel, el("label", { className: "tbl-check" }, box, TABLE.addresses), this.copyButton));
    return bar;
  }

  /** Rows a second, or null for a table whose rows are not spaced in time. */
  private get rate(): number | null {
    const rate = this.plan.rate;
    return rate !== null && rate > 0 ? rate : null;
  }

  /** The columns that are about a record rather than in it. */
  private get lead(): Lead {
    return { named: this.plan.rowNames, rate: this.rate, addresses: this.addresses };
  }

  /** The sentence saying what one row is, which is one column when the table
   *  is turned. */
  private sayMeaning(): void {
    const rate = this.rate;
    if (rate === null) return;
    const word = this.plan.columnWord;
    const said = rate.toLocaleString();
    const rows = this.plan.rowWord;
    if (this.turned) this.meaning.textContent = word === null ? TABLE.columnMeaningPlain(rows, said) : TABLE.columnMeaning(rows, word, said);
    else this.meaning.textContent = word === null ? TABLE.rowMeaningPlain(rows, said) : TABLE.rowMeaning(rows, word, said);
  }

  /**
   * Draw the table the other way round.
   *
   * The selection goes, because it was of rows and the rows are different
   * things now: three selected records do not become three selected fields.
   * So does the scroll position, for the same reason. The choice is kept only
   * for a table that would have arrived turned; see `startsTurned`.
   */
  private turn(on: boolean): void {
    this.turned = on;
    if (turnsByDefault(this.plan)) rememberChoice(TURNED_KEY, on ? "1" : "0");
    this.anchor = null;
    this.focus = null;
    this.pickedRecord = 0;
    this.scroller.scrollTop = 0;
    this.scroller.scrollLeft = 0;
    this.sayMeaning();
    this.refreshCopy();
    this.sizeCanvas();
    this.layAgain();
  }

  /** How many rows are drawn: the records, or turned, the fields. */
  private get shownRows(): number {
    return this.turned ? this.columns.length : this.plan.count;
  }

  private sizeCanvas(): void {
    this.canvas.style.height = `${Math.min(MAX_CANVAS, this.shownRows * ROW)}px`;
  }

  /** The columns, the headings and the rows, all again: which columns there
   *  are has changed. */
  private layAgain(): void {
    this.turnedLaid = false;
    this.layColumns();
    this.fillHead();
    this.paintAgain();
  }

  // ----- the columns -----

  /** One track list for the header and every row, so the two line up without
   *  either measuring the other. Every track is a fixed width: the data
   *  columns are as wide as what has been seen in them (see `ColumnFit`), so a
   *  table of one channel is a narrow table and not one number adrift in a
   *  tab-wide column. */
  private layColumns(): void {
    const widths = this.turned ? this.turnedWidths() : this.widths();
    this.el.style.setProperty("--tbl-cols", widths.map((w) => `${w}ch`).join(" "));
  }

  /** The tracks of the table the right way up, in `headerCells`' order. */
  private widths(): number[] {
    const out = [indexWidth(this.plan.count)];
    if (this.plan.rowNames) out.push(this.nameFit.width);
    if (this.rate !== null) out.push(timeWidth(this.plan.count, this.rate));
    out.push(...this.columns.map((column) => column.fit.width));
    if (this.addresses) out.push(AT_WIDTH, SIZE_WIDTH);
    return out;
  }

  /** The tracks of the table turned: the labels, then a column a record, each
   *  as wide as the widest thing in it, heading lines included. Before the
   *  records are all read a column is as wide as its number. */
  private turnedWidths(): number[] {
    const clamp = (n: number): number => Math.min(FIT_MAX, Math.max(FIT_MIN, n));
    const labels = [...headerCells([], this.lead).map((text) => text.length), ...this.columns.map((_, c) => this.headTextOf(c).length)];
    const out = [clamp(Math.max(...labels))];
    for (let i = 0; i < this.plan.count; i++) {
      const row = this.have.get(i);
      if (row === undefined) {
        out.push(clamp(i.toLocaleString().length));
        continue;
      }
      const heads = recordCells(i, row, 0, this.lead).map((text) => text.length);
      out.push(clamp(Math.max(...heads, ...row.cells.map((cell) => cell.text.length))));
    }
    return out;
  }

  /** The heading of one data column, unit included. */
  private headingOf(c: number): string {
    const column = this.plan.columns[c];
    if (column === undefined) return "";
    return column.unit === "" ? column.name : `${column.name} (${column.unit})`;
  }

  /** The heading as it is drawn: the column's name and unit, then how many of
   *  its cells hold a wrong value. A reader scrolling a run of samples sees
   *  one marked cell at a time and no way to tell whether it is the only one,
   *  so the count is what says how much there is to look for. */
  private headTextOf(c: number): string {
    const column = this.columns[c];
    if (column === undefined) return this.headingOf(c);
    const more = PROBLEMS.column(column.problems[0], column.problems[1], this.have.size < this.plan.count);
    return more === "" ? this.headingOf(c) : `${this.headingOf(c)} ${more}`;
  }

  /** A row has been read: add whatever is wrong in it to its columns' counts,
   *  and write any heading whose count changed. The heading is allowed to
   *  widen its column, since a count nobody can read is not a count. */
  private countRow(row: TableRow): void {
    let widened = false;
    // Every heading is rewritten once the last row arrives, because that is
    // when `so far` comes off all of them at once.
    const settled = this.have.size >= this.plan.count;
    for (const [c, column] of this.columns.entries()) {
      const problem = row.cells[c]?.problem;
      if (problem === undefined && !settled) continue;
      if (problem !== undefined) column.problems[problem.tier === "invalid" ? 0 : 1]++;
      widened = this.writeHead(c) || widened;
    }
    if (widened) this.layColumns();
  }

  /** Every heading, drawn again with what it says now. Used when the counts
   *  are thrown away: a heading still reading `2 invalid so far` after the
   *  rows behind it have gone would be saying something nothing holds up. */
  private writeHeads(): void {
    let widened = false;
    for (let c = 0; c < this.columns.length; c++) widened = this.writeHead(c) || widened;
    if (widened) this.layColumns();
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

  /** A row has been read: widen any column it does not fit, and settle the
   *  side of any column whose first value this is. The track list is written
   *  again only when a width changed, which is a few times at the start and
   *  then not at all. */
  private fitRow(row: TableRow): void {
    let widened = false;
    if (row.name !== undefined && row.name.length > this.nameFit.width) {
      this.nameFit = { width: Math.min(FIT_MAX, row.name.length), numeric: false };
      widened = true;
    }
    for (const [c, column] of this.columns.entries()) {
      const was = column.fit;
      const now = fitCell(was, row.cells[c]);
      if (now === was) continue;
      column.fit = now;
      if (now.width !== was.width) widened = true;
      if (now.numeric !== was.numeric) column.head.classList.toggle("tbl-num", now.numeric === true);
    }
    if (widened) this.layColumns();
  }

  /** The header row, assembled from the column headings and whichever of the
   *  index, time and address cells this table shows. The headings are the same
   *  elements every time, so what a column has learnt about itself (its width,
   *  its side, its count) survives the addresses being turned on. */
  private fillHead(): void {
    this.writeHeads();
    if (this.turned) return this.fillTurnedHead();
    const cells: HTMLElement[] = [el("span", { className: "tbl-th tbl-index tbl-num", textContent: TABLE.index })];
    if (this.plan.rowNames) cells.push(el("span", { className: "tbl-th", textContent: TABLE.rowName }));
    if (this.rate !== null) cells.push(el("span", { className: "tbl-th tbl-num", textContent: TABLE.time }));
    cells.push(...this.columns.map((column) => column.head));
    if (this.addresses) {
      cells.push(el("span", { className: "tbl-th", textContent: TABLE.storedAt }));
      cells.push(el("span", { className: "tbl-th tbl-num", textContent: TABLE.size }));
    }
    this.head.replaceChildren(el("div", { className: "tbl-headline" }, ...cells));
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
    const records = this.allRecords();
    const lead = this.lead;
    const lines =
      records === null
        ? [[TABLE.index, ...Array.from({ length: this.plan.count }, (_, i) => String(i))]]
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
    this.head.replaceChildren(...out);
  }

  /** Every record, or null while any of them is still being read. Only asked
   *  of a table short enough to turn. */
  private allRecords(): TableRow[] | null {
    const out: TableRow[] = [];
    for (let i = 0; i < this.plan.count; i++) {
      const row = this.rowAt(i);
      if (row === null) return null;
      out.push(row);
    }
    return out;
  }

  // ----- drawing -----

  /** Draw again once the frame is over, however many things asked. Streaming a
   *  file's first megabyte fires `onChange` far faster than a screen redraws. */
  private schedule(): void {
    if (this.frame !== 0) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      // What a row said may have been read from bytes that have since
      // arrived, so the answers go rather than being drawn again. The counts
      // are answers about those rows and go with them.
      this.have.clear();
      this.turnedLaid = false;
      for (const column of this.columns) column.problems = [0, 0];
      this.writeHeads();
      this.plan.forget();
      this.paintAgain();
    });
  }

  private paintAgain(): void {
    this.drawn = null;
    this.paint();
  }

  /** How many rows fit on screen, at least one so a short tab still draws.
   *  The header is stuck over the top of the scroller, so what is left for the
   *  rows is the scroller less the header. */
  private onScreen(): number {
    return Math.max(1, Math.floor((this.scroller.clientHeight - this.head.offsetHeight) / ROW));
  }

  /** How far the scroll bar can go. The header is in the scroller with the
   *  canvas, so it is part of what is scrolled through. */
  private travel(): number {
    return this.head.offsetHeight + this.canvas.clientHeight - this.scroller.clientHeight;
  }

  /** True once the rows are taller than a canvas is allowed to be, which is
   *  where the scroll bar stops standing for pixels. */
  private get capped(): boolean {
    return this.shownRows * ROW > MAX_CANVAS;
  }

  /** The first row the scroll position is asking for. Under the cap that is
   *  division; past it the bar's place in its own travel is the reader's place
   *  in the rows, which is the only mapping left once the pixels run out. */
  private firstVisible(): number {
    if (!this.capped) return Math.floor(this.scroller.scrollTop / ROW);
    const travel = this.travel();
    const ratio = travel <= 0 ? 0 : this.scroller.scrollTop / travel;
    return Math.round(ratio * Math.max(0, this.shownRows - this.onScreen()));
  }

  private paint(): void {
    // A turned row is one field of every record, so nothing of it can be
    // drawn until they are all here; and once they are, the headings and the
    // widths that were waiting on them are worked out, before the rows are
    // measured against a header that is about to grow.
    const records = this.turned ? this.allRecords() : null;
    if (records !== null && !this.turnedLaid) {
      this.turnedLaid = true;
      this.layColumns();
      this.fillHead();
    }
    const firstVisible = this.firstVisible();
    const first = Math.max(0, firstVisible - OVERSCAN);
    const last = Math.min(this.shownRows, firstVisible + this.onScreen() + OVERSCAN);
    // Under the cap a row sits at its own place on the canvas. Past it the
    // canvas is shorter than the rows would need, so the window is drawn where
    // the reader is looking: at the top of the viewport, wherever that is.
    const base = this.capped ? this.scroller.scrollTop - (firstVisible - first) * ROW : first * ROW;
    const was = this.drawn;
    if (was !== null && was.from === first && was.to === last && was.base === base && was.addresses === this.addresses) return;
    this.drawn = { from: first, to: last, base, addresses: this.addresses };
    const out: HTMLElement[] = [];
    for (let i = first; i < last; i++) {
      const element = this.turned ? this.drawTurned(i, records) : this.drawRecord(i);
      element.style.top = `${base + (i - first) * ROW}px`;
      element.dataset["index"] = String(i);
      out.push(element);
    }
    this.canvas.replaceChildren(...out);
  }

  /** Row `i`, kept once it has been read: a scroll back up should not be a
   *  second read of rows the view has already seen. */
  private rowAt(i: number): TableRow | null {
    const known = this.have.get(i);
    if (known !== undefined) return known;
    const row = this.plan.row(i);
    if (row !== null) {
      this.have.set(i, row);
      this.fitRow(row);
      this.countRow(row);
    }
    return row;
  }

  /** The selected rows as a half-open range, or null when none are. */
  private range(): { from: number; to: number } | null {
    if (this.anchor === null || this.focus === null) return null;
    return { from: Math.min(this.anchor, this.focus), to: Math.max(this.anchor, this.focus) + 1 };
  }

  private isOn(i: number): boolean {
    const range = this.range();
    return range !== null && i >= range.from && i < range.to;
  }

  private drawRecord(i: number): HTMLElement {
    const row = this.rowAt(i);
    return row === null ? this.drawWaiting(i.toLocaleString(), "tbl-index") : this.drawRow(i, row);
  }

  /** Row `j` of a turned table: the field's heading, then its value in every
   *  record. Each cell says which record it is in, which is what a click needs
   *  to know to find the bytes. */
  private drawTurned(j: number, records: readonly TableRow[] | null): HTMLElement {
    const label = this.headTextOf(j);
    if (records === null) return this.drawWaiting(label, "tbl-label");
    const element = el("div", { className: this.isOn(j) ? "tbl-row is-on" : "tbl-row" });
    const heading = el("span", { className: "tbl-cell tbl-label", textContent: label });
    heading.title = label;
    element.append(heading);
    const numeric = this.columns[j]?.fit.numeric === true;
    for (const [i, row] of records.entries()) {
      const cell = this.drawCell(row.cells[j], numeric);
      cell.dataset["record"] = String(i);
      element.append(cell);
    }
    return element;
  }

  private drawRow(i: number, row: TableRow): HTMLElement {
    const element = el("div", { className: this.isOn(i) ? "tbl-row is-on" : "tbl-row" });
    element.append(el("span", { className: "tbl-cell tbl-index tbl-num", textContent: i.toLocaleString() }));
    if (this.plan.rowNames) {
      const name = el("span", { className: "tbl-cell tbl-name", textContent: row.name ?? "" });
      name.title = row.name ?? "";
      element.append(name);
    }
    const rate = this.rate;
    if (rate !== null) element.append(el("span", { className: "tbl-cell tbl-time tbl-num", textContent: timeText(i, rate) }));
    for (let c = 0; c < this.plan.columns.length; c++) {
      element.append(this.drawCell(row.cells[c], this.columns[c]?.fit.numeric === true));
    }
    if (this.addresses) {
      element.append(el("span", { className: "tbl-cell tbl-at", textContent: formatOffset(row.offsetBits) }));
      element.append(el("span", { className: "tbl-cell tbl-size tbl-num", textContent: bitSizeText(row.sizeBits) }));
    }
    return element;
  }

  /** One cell. A cell naming another part of the file is a link to it, the
   *  same cross-reference a record table in the listing draws. */
  private drawCell(cell: RecordCell | undefined, numeric: boolean): HTMLElement {
    if (cell === undefined) return el("span", { className: "tbl-cell" });
    const link = cell.link;
    if (link !== undefined) {
      const button = el("button", { type: "button", className: "tbl-cell rec-link", textContent: link.text });
      button.title = link.label;
      button.addEventListener("click", (e) => {
        e.stopPropagation();
        this.onFactPick(link.path);
      });
      return button;
    }
    const problem = cell.problem;
    const invalid = problem?.tier === "invalid";
    let className = `tbl-cell ${fieldClass(cell.kind)}${numeric ? " tbl-num" : ""}`;
    if (invalid) className += " is-invalid";
    const element = el("span", { className, textContent: cell.text });
    // The reason is on the cell's hover rather than in it: a column is as wide
    // as its values and a sentence in every marked cell would take the table
    // apart. The column heading says how many there are; the glyph says which.
    element.title = problem === undefined ? cell.text : `${cell.text}\n${problem.text}`;
    if (problem !== undefined) element.prepend(glyph(invalid));
    return element;
  }

  /** A row whose bytes are not here yet. It keeps its place and its number
   *  (turned, its heading), so the table does not jump when the answer
   *  arrives in it. */
  private drawWaiting(label: string, labelClass: string): HTMLElement {
    const element = el("div", { className: "tbl-row tbl-waiting" });
    element.append(el("span", { className: `tbl-cell ${labelClass}`, textContent: label }));
    element.append(el("span", { className: "tbl-cell", textContent: REPORT.paneWaiting }));
    return element;
  }

  // ----- selection -----

  /**
   * Follow the file's cursor. A bit inside the table selects the row holding
   * it and scrolls to it; a bit outside leaves the table where it is, since
   * the reader moving the cursor somewhere else is not asking this tab a
   * question.
   */
  setBit(bit: number): void {
    if (this.picking) return;
    const record = this.plan.rowFor(bit);
    if (record === null) return;
    const at = this.turned ? this.fieldAt(record, bit) : record;
    if (at === null) return;
    this.anchor = at;
    this.focus = at;
    this.scrollToRow(at);
    this.refreshCopy();
    this.paintAgain();
    if (this.turned) {
      this.pickedRecord = record;
      this.showRecord(record);
    }
  }

  /** Which field of a record a bit falls in: turned, that is the row. Null
   *  where the plan cannot say where its cells are. */
  private fieldAt(record: number, bit: number): number | null {
    const spans = this.rowAt(record)?.spans;
    if (spans === undefined) return null;
    const at = spans.findIndex((span) => bit >= span.offsetBits && bit < span.offsetBits + span.sizeBits);
    return at < 0 ? null : at;
  }

  /** Turned, scroll across until a record's column is in view, clear of the
   *  labels stuck over the near edge. */
  private showRecord(record: number): void {
    const cell = this.canvas.querySelector<HTMLElement>(`.tbl-row [data-record="${record}"]`);
    const label = this.canvas.querySelector<HTMLElement>(".tbl-row .tbl-label");
    if (cell === null || label === null) return;
    const near = cell.offsetLeft - label.offsetWidth;
    const far = cell.offsetLeft + cell.offsetWidth - this.scroller.clientWidth;
    if (near < this.scroller.scrollLeft) this.scroller.scrollLeft = near;
    else if (far > this.scroller.scrollLeft) this.scroller.scrollLeft = far;
  }

  clearSelection(): void {
    this.anchor = null;
    this.focus = null;
    this.refreshCopy();
    this.paintAgain();
  }

  /** The copy button says how many rows it will copy, and is disabled until
   *  there are any. */
  private refreshCopy(): void {
    const range = this.range();
    const n = range === null ? 0 : range.to - range.from;
    this.copyButton.disabled = n === 0;
    this.copyButton.textContent = n === 0 ? TABLE.copy : TABLE.copyRows(n, this.turned ? TABLE.rowFallback : this.plan.rowWord);
    this.copyButton.title = n === 0 ? TABLE.copyTitleNone : TABLE.copyTitle;
  }

  private scrollToRow(i: number): void {
    const onScreen = this.onScreen();
    const first = this.firstVisible();
    if (i >= first && i < first + onScreen) return;
    const want = Math.max(0, i - Math.floor(onScreen / 2));
    if (!this.capped) {
      this.scroller.scrollTop = want * ROW;
      return;
    }
    const travel = this.travel();
    const rows = Math.max(1, this.shownRows - onScreen);
    this.scroller.scrollTop = Math.max(0, Math.min(travel, (want / rows) * travel));
  }

  // ----- input -----

  private onClick(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof Element)) return;
    const at = target.closest<HTMLElement>(".tbl-row")?.dataset["index"];
    if (at === undefined) return;
    const record = target.closest<HTMLElement>("[data-record]")?.dataset["record"];
    if (record !== undefined) this.pickedRecord = Number(record);
    this.pick(Number(at), e.shiftKey);
  }

  /**
   * The keys a long table is read with.
   *
   * They move the selected row rather than the scroll position, which is the
   * same thing under the cap and is not past it: up there one row of scroll is
   * several rows of table, so an arrow key that moved pixels would skip rows
   * the reader was stepping through.
   */
  private onKey(e: KeyboardEvent): void {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "c") {
      if (this.range() === null) return;
      e.preventDefault();
      void this.copySelection();
      return;
    }
    if (mod && e.key.toLowerCase() === "a") {
      e.preventDefault();
      this.anchor = 0;
      this.pick(this.shownRows - 1, true);
      return;
    }
    const page = Math.max(1, this.onScreen() - 1);
    const by: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page };
    const move = by[e.key];
    const from = this.focus ?? this.firstVisible();
    if (move !== undefined) {
      e.preventDefault();
      this.pick(Math.max(0, Math.min(this.shownRows - 1, from + move)), e.shiftKey);
      return;
    }
    if (e.key === "Home") {
      e.preventDefault();
      this.pick(0, e.shiftKey);
    } else if (e.key === "End") {
      e.preventDefault();
      this.pick(this.shownRows - 1, e.shiftKey);
    }
  }

  /**
   * Select row `i`, or with `extend` make it the far end of the selection
   * that began at the anchor, the way shift-click and shift-arrow work in
   * every list. The file tab is sent the bytes of the whole selection: its
   * cursor goes to the first row and the mark covers to the last.
   */
  private pick(i: number, extend = false): void {
    this.focus = i;
    if (!extend || this.anchor === null) this.anchor = i;
    this.scrollToRow(i);
    this.refreshCopy();
    this.paintAgain();
    const range = this.range();
    if (range === null) return;
    if (this.turned) return this.pickTurned(range);
    const first = this.rowAt(range.from);
    if (first === null) return;
    // The last row of a long selection may not be read yet; then the mark
    // reaches as far as the row the reader just picked, which is read.
    const last = this.rowAt(range.to - 1) ?? this.rowAt(i) ?? first;
    this.picking = true;
    this.onPick({ path: first.path, startBit: first.offsetBits, endBit: last.offsetBits + last.sizeBits });
    this.picking = false;
  }

  /**
   * Send the file tab to a selection in a turned table. The rows are fields,
   * and a field is stored once in every record, so the bytes are the ones in
   * the record whose column was clicked: the fields selected, first to last,
   * which in one record are a single run. A plan that cannot say where its
   * cells are sends the whole record, which is near and is never wrong.
   */
  private pickTurned(range: { from: number; to: number }): void {
    const row = this.rowAt(this.pickedRecord);
    if (row === null) return;
    const first = row.spans?.[range.from];
    const last = row.spans?.[range.to - 1];
    this.picking = true;
    if (first === undefined || last === undefined) this.onPick({ path: row.path, startBit: row.offsetBits, endBit: row.offsetBits + row.sizeBits });
    else {
      const startBit = Math.min(first.offsetBits, last.offsetBits);
      const endBit = Math.max(first.offsetBits + first.sizeBits, last.offsetBits + last.sizeBits);
      this.onPick({ path: range.to - range.from === 1 ? first.path : row.path, startBit, endBit });
    }
    this.picking = false;
  }

  // ----- copying -----

  /**
   * Put the selected rows on the clipboard as tab-separated text, with a
   * heading line: what a spreadsheet or a script takes as it is. The columns
   * are the ones on screen, address columns included when they are shown, and
   * a turned table is copied turned, so what is copied is what the reader is
   * looking at.
   */
  private async copySelection(): Promise<void> {
    const range = this.range();
    if (range === null) return;
    const n = range.to - range.from;
    if (n > COPY_LIMIT_ROWS) return this.say(TABLE.copyTooBig(n, COPY_LIMIT_ROWS));
    const headings = this.columns.map((_, c) => this.headingOf(c));
    let lines: string[][];
    if (this.turned) {
      const records = this.allRecords();
      if (records === null) return this.say(TABLE.copyPending);
      lines = turnedLines(headings, records, this.lead, range);
    } else {
      lines = [headerCells(headings, this.lead)];
      for (let i = range.from; i < range.to; i++) {
        const row = this.rowAt(i);
        if (row === null) return this.say(TABLE.copyPending);
        lines.push(recordCells(i, row, headings.length, this.lead));
      }
    }
    try {
      await navigator.clipboard.writeText(lines.map(tsvLine).join("\n"));
    } catch {
      return this.say(TABLE.copyFailed);
    }
    this.say(TABLE.copied(n, this.turned ? TABLE.rowFallback : this.plan.rowWord));
  }

  /** A message about something the reader just asked for, which goes away on
   *  its own. */
  private say(text: string): void {
    this.notice.textContent = text;
    this.notice.hidden = false;
    clearTimeout(this.noticeTimer);
    this.noticeTimer = window.setTimeout(() => {
      this.notice.hidden = true;
    }, NOTICE_MS);
  }
}

/** A fact's value as the bar shows it: a number gets its thousands separators,
 *  since `44,100` is read at a glance and `44100` is counted. Anything else is
 *  shown as the core wrote it. */
function factValue(value: string): string {
  const n = Number(value);
  return value.trim() !== "" && Number.isFinite(n) ? n.toLocaleString() : value;
}

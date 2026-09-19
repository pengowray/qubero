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
//
// Addresses follow the display too. The two address columns are about the
// record, so turned they are two of those heading lines, a cell a record. What
// the drawn row is then -- one field of every record -- has an address of its
// own, and it is on the row's heading, because a frame keeps a column's values
// in one block and the run the unturned table could only call `per cell` reads
// straight down the page. A cell says where it is on its own hover either way
// up, and a click goes to the bytes of the cell that was clicked.

import type { Doc } from "./doc.ts";
import { el } from "./dom.ts";
import { fieldClass } from "./fieldstyle.ts";
import type { RecordCell } from "./records.ts";
import { PROBLEMS, REPORT, TABLE } from "./strings.ts";
import { rememberChoice, storedText } from "./stored.ts";
import { cellPlaceIn, drawAddress, fieldWhereLines, rowHasBytes, whatLines, whereLines } from "./tableaddress.ts";
import { tableBar, type Bar } from "./tablebar.ts";
import { TableColumns } from "./tablecolumns.ts";
import { TableExportPanel } from "./tableexportpanel.ts";
import { rowRun, startsTurned, timeText, turnsByDefault, type TablePlan, type TableRow } from "./tableplan.ts";
import { ROW, RowScroll } from "./tablescroll.ts";
import { headerCells, recordCells, tsvLine, turnedLines, type Lead } from "./tabletext.ts";

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
/** What in the view is cut short with an ellipsis when it does not fit, and so
 *  needs its full text somewhere. */
const CUT_SHORT = ".tbl-th, .tbl-cell";

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
  private readonly plan: TablePlan;
  private readonly head: HTMLElement;
  private readonly scroller: HTMLElement;
  private readonly canvas: HTMLElement;
  /** The rows already read, by index. Cleared whenever the file changes, since
   *  that is when what one of them says can stop being true. */
  private readonly have = new Map<number, TableRow>();
  /** The data columns: their widths, their sides, their headings and what
   *  their cells have been found to hold. */
  private readonly columns: TableColumns;
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
  private readonly barLine: Bar;
  /** The selected rows, as they are drawn: records, or fields when turned. the one the selection started on, and the one it was
   *  last extended to. Equal for a single row; the range runs between them
   *  either way round. Null when nothing is selected. */
  private anchor: number | null = null;
  private focus: number | null = null;
  private readonly copyButton: HTMLButtonElement;
  private readonly exporter: TableExportPanel;
  private readonly notice: HTMLElement;
  private noticeTimer = 0;
  /** True while a pick this view made is being sent out, so the cursor move it
   *  causes does not come back and undo the scroll position. */
  private picking = false;
  /** Which rows the scroll position is asking for, and where to put them. */
  private readonly rows: RowScroll;
  private drawn: { from: number; to: number; base: number; addresses: boolean } | null = null;
  private frame = 0;

  /** The reader picked a row. */
  onPick: (pick: TablePick) => void = () => {};
  /** The reader followed one of the facts above the table, or a link in a
   *  cell, to the field it names. */
  onFactPick: (path: readonly number[]) => void = () => {};

  constructor(doc: Doc, plan: TablePlan, opts: { readonly title: string }) {
    this.plan = plan;
    this.addresses = storedText(ADDRESSES_KEY) === "1";
    this.turned = startsTurned(plan, storedText(TURNED_KEY));
    this.el = el("div", { className: "tableview" });
    this.head = el("div", { className: "tbl-head" });
    this.scroller = el("div", { className: "tbl-scroll" });
    this.scroller.tabIndex = 0;
    this.canvas = el("div", { className: "tbl-canvas" });
    this.rows = new RowScroll(this.scroller, this.canvas, this.head);
    // The sheet is as wide as its columns. The header is what sizes it, being
    // the one thing in it that is laid out in the ordinary flow: the rows are
    // placed by arithmetic and take whatever width the sheet has.
    this.scroller.append(el("div", { className: "tbl-sheet" }, this.head, this.canvas));
    this.copyButton = el("button", { type: "button", className: "tbl-copy" });
    this.copyButton.addEventListener("click", () => void this.copySelection());
    this.notice = el("div", { className: "tbl-notice", hidden: true });
    // A save reads the plan rather than the rows this view keeps: the view
    // keeps every row it has drawn, and a save of a few million would leave
    // them all here.
    this.exporter = new TableExportPanel({
      file: doc.name,
      table: opts.title,
      rowWord: plan.rowWord,
      count: plan.count,
      headings: () => this.columns.headings(),
      lead: () => this.lead,
      turned: () => this.turned,
      selected: () => this.range(),
      row: (i) => plan.row(i),
      release: () => plan.forget(),
      say: (text) => this.say(text),
    });
    this.barLine = tableBar({
      title: opts.title,
      plan,
      turned: this.turned,
      addresses: this.addresses,
      copyButton: this.copyButton,
      exporter: this.exporter.el,
      onTurn: (turned) => this.turn(turned),
      onAddresses: (on) => {
        this.addresses = on;
        rememberChoice(ADDRESSES_KEY, on ? "1" : "0");
        this.layAgain();
      },
      onFactPick: (path) => this.onFactPick(path),
    });
    this.el.append(this.barLine.el, this.scroller, this.notice);
    this.refreshCopy();
    this.columns = new TableColumns({
      plan,
      el: this.el,
      head: this.head,
      turned: () => this.turned,
      addresses: () => this.addresses,
      rate: () => this.rate,
      lead: () => this.lead,
      have: this.have,
      records: () => this.allRecords(),
    });
    this.sizeCanvas();
    this.columns.lay();
    this.columns.fillHead();
    this.scroller.addEventListener("scroll", () => this.paint(), { passive: true });
    this.scroller.addEventListener("click", (e) => this.onClick(e));
    this.scroller.addEventListener("keydown", (e) => this.onKey(e));
    this.scroller.addEventListener("mouseover", (e) => this.sayInFull(e));
    new ResizeObserver(() => this.paintAgain()).observe(this.scroller);
    // Bytes arriving turn a waiting row into a row; so does an edit.
    doc.onChange(() => this.schedule());
  }

  /** The page is in the document now, so the rows can be measured and drawn.
   *  Also every time the tab comes back to the front. */
  shown(): void {
    this.paintAgain();
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
    this.rows.home();
    this.barLine.sayMeaning(on);
    this.refreshCopy();
    this.sizeCanvas();
    this.layAgain();
  }

  /** How many rows are drawn: the records, or turned, the fields. */
  private get shownRows(): number {
    return this.turned ? this.columns.length : this.plan.count;
  }

  private sizeCanvas(): void {
    this.rows.setCount(this.shownRows);
  }

  /** The columns, the headings and the rows, all again: which columns there
   *  are has changed. */
  private layAgain(): void {
    this.turnedLaid = false;
    this.columns.lay();
    this.columns.fillHead();
    this.paintAgain();
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

  /**
   * Give a cell that is cut short its full text on hover.
   *
   * Asked as the pointer arrives rather than written on every cell as it is
   * drawn, because whether a cell is cut short is a fact about how wide it
   * came out, and a tooltip repeating a value that can be read where it is
   * only gets in the way of the one beside it.
   *
   * A cell that already has something to say on hover keeps it, under the
   * full text rather than instead of it: a value cell leads with its own text
   * already, and an address cell reading `This row's cells are in different
   * places` still has to be able to say what the address it cut short was. The
   * text goes on once, so a second hover changes nothing.
   */
  private sayInFull(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof HTMLElement) || !target.matches(CUT_SHORT)) return;
    const text = target.textContent ?? "";
    if (text === "" || target.title.startsWith(text)) return;
    if (target.scrollWidth > target.clientWidth) target.title = target.title === "" ? text : `${text}\n${target.title}`;
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
      this.columns.forget();
      this.plan.forget();
      this.paintAgain();
    });
  }

  private paintAgain(): void {
    this.drawn = null;
    this.paint();
  }

  /** How many rows fit on screen, at least one so a short tab still draws. */
  private onScreen(): number {
    return this.rows.onScreen();
  }

  private paint(): void {
    // A turned row is one field of every record, so nothing of it can be
    // drawn until they are all here; and once they are, the headings and the
    // widths that were waiting on them are worked out, before the rows are
    // measured against a header that is about to grow.
    const records = this.turned ? this.allRecords() : null;
    if (records !== null && !this.turnedLaid) {
      this.turnedLaid = true;
      this.columns.lay();
      this.columns.fillHead();
    }
    const { from, to, base } = this.rows.window();
    const was = this.drawn;
    if (was !== null && was.from === from && was.to === to && was.base === base && was.addresses === this.addresses) return;
    this.drawn = { from, to, base, addresses: this.addresses };
    const out: HTMLElement[] = [];
    for (let i = from; i < to; i++) {
      const element = this.turned ? this.drawTurned(i, records) : this.drawRecord(i);
      element.style.top = `${base + (i - from) * ROW}px`;
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
      this.columns.read(row);
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
   *  to know to find the bytes. The heading carries where the drawn row is,
   *  since the two address columns are heading lines when the table is turned
   *  and are about the records rather than about this row. */
  private drawTurned(j: number, records: readonly TableRow[] | null): HTMLElement {
    const label = this.columns.headTextOf(j);
    if (records === null) return this.drawWaiting(label, "tbl-label");
    const element = el("div", { className: this.isOn(j) ? "tbl-row is-on" : "tbl-row" });
    const heading = el("span", { className: "tbl-cell tbl-label", textContent: label });
    heading.title = this.addresses ? [label, ...fieldWhereLines(records, j)].join("\n") : label;
    element.append(heading);
    const numeric = this.columns.numeric(j);
    for (const [i, row] of records.entries()) {
      const cell = this.drawCell(row.cells[j], numeric);
      cell.dataset["record"] = String(i);
      cell.dataset["column"] = String(j);
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
      const cell = this.drawCell(row.cells[c], this.columns.numeric(c));
      // Which column was clicked, for a table whose cells are each in a place
      // of their own: the cell is what a pick goes to, not the whole row.
      cell.dataset["column"] = String(c);
      element.append(cell);
    }
    if (this.addresses) element.append(...drawAddress(row));
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
    const lines = [cell.text];
    // What the cell is, where that is not its value: a masked array's hidden
    // entry is drawn empty, and this is what says which nothing it is.
    lines.push(...whatLines(cell));
    if (problem !== undefined) lines.push(problem.text);
    // Where this one cell's bytes are, for the tables whose rows are not a run
    // of the file. Only with the address columns on: a reader who has not
    // asked for addresses is reading the values.
    if (this.addresses) lines.push(...whereLines(cell));
    element.title = lines.filter((line) => line !== "").join("\n");
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

  /** Which field of a record a bit falls in: turned, that is the row. Asked
   *  cell by cell the way a pick is, so the two halves of the turned selection
   *  agree about where a cell is. Null where nothing can place them, and for a
   *  bit in another space, which is not the one the cursor counts in. */
  private fieldAt(record: number, bit: number): number | null {
    const row = this.rowAt(record);
    if (row === null) return null;
    for (let c = 0; c < row.cells.length; c++) {
      const place = cellPlaceIn(row, c);
      if (place !== null && place.space === 0 && bit >= place.offsetBits && bit < place.offsetBits + place.sizeBits) return c;
    }
    return null;
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
    this.copyButton.textContent = n === 0 ? TABLE.copy : TABLE.copyRows(n);
    this.copyButton.title = n === 0 ? TABLE.copyTitleNone : TABLE.copyTitle;
  }

  private scrollToRow(i: number): void {
    this.rows.scrollToRow(i);
  }

  // ----- input -----

  private onClick(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof Element)) return;
    const at = target.closest<HTMLElement>(".tbl-row")?.dataset["index"];
    if (at === undefined) return;
    const record = target.closest<HTMLElement>("[data-record]")?.dataset["record"];
    if (record !== undefined) this.pickedRecord = Number(record);
    const column = target.closest<HTMLElement>(".tbl-cell")?.dataset["column"];
    this.pick(Number(at), e.shiftKey, column === undefined ? null : Number(column));
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
    const from = this.focus ?? this.rows.firstVisible();
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
   *
   * `column` is the cell the reader clicked, for a table whose cells are in
   * several places. One cell is what was clicked and its bytes are what the
   * file tab is sent; a cell with no bytes, or with bytes inside an unpacked
   * stream rather than in this tab, leaves the cursor where it is rather than
   * moving it somewhere the reader did not click.
   */
  private pick(i: number, extend = false, column: number | null = null): void {
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
    const one = column === null || extend ? undefined : first.cells[column]?.at;
    if (one !== undefined) {
      if (one.space !== 0) return;
      this.send(first.path, one.offsetBits, one.offsetBits + one.sizeBits);
      return;
    }
    // The last row of a long selection may not be read yet; then the mark
    // reaches as far as the row the reader just picked, which is read.
    const last = this.rowAt(range.to - 1) ?? this.rowAt(i) ?? first;
    if ((first.space ?? 0) !== 0 || (last.space ?? 0) !== 0) return;
    if (!rowHasBytes(first) || !rowHasBytes(last)) return;
    this.send(first.path, first.offsetBits, last.offsetBits + last.sizeBits);
  }

  /**
   * Send the file tab to a selection in a turned table. The rows are fields,
   * and a field is stored once in every record, so the bytes are the ones in
   * the record whose column was clicked: the fields selected, in that one
   * record.
   *
   * Those are a single run when the record is one, which is the ordinary case
   * and covers the whole selection. Where they are scattered, which is what a
   * computed table's cells usually are, only the first of them is sent: a mark
   * from the first to the last would cover blocks the reader did not select.
   * A plan that cannot say where its cells are sends the whole record, which
   * is near and is never wrong.
   */
  private pickTurned(range: { from: number; to: number }): void {
    const row = this.rowAt(this.pickedRecord);
    if (row === null) return;
    const places = [];
    for (let c = range.from; c < range.to; c++) places.push(cellPlaceIn(row, c));
    if (places.every((place) => place === null)) {
      if (rowHasBytes(row) && (row.space ?? 0) === 0) this.send(row.path, row.offsetBits, row.offsetBits + row.sizeBits);
      return;
    }
    const run = rowRun(places);
    if (run.space !== 0) return;
    const path = (range.to - range.from === 1 ? row.spans?.[range.from]?.path : undefined) ?? row.path;
    this.send(path, run.offsetBits, run.offsetBits + run.sizeBits);
  }

  private send(path: readonly number[], startBit: number, endBit: number): void {
    this.picking = true;
    this.onPick({ path, startBit, endBit });
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
    const headings = this.columns.headings();
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
    this.say(TABLE.copied(n));
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

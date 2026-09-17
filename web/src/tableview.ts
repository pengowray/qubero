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

import { formatOffset } from "./doc.ts";
import type { Doc } from "./doc.ts";
import { el } from "./dom.ts";
import { fieldClass } from "./fieldstyle.ts";
import type { RecordCell } from "./records.ts";
import { bitSizeText, REPORT, TABLE } from "./strings.ts";
import { timeText, type TablePlan, type TableRow } from "./tableplan.ts";

/** Height of one row, which must match `--tv-row` in the stylesheet: the rows
 *  are placed by arithmetic on it, so a row that drew taller would slide out
 *  from under its own place. */
const ROW = 22;
/** Rows drawn above and below the window, so a wheel notch has somewhere to go
 *  before the next paint. */
const OVERSCAN = 8;
/** How tall the canvas is allowed to get. Past this the scroll bar stands for
 *  the rows by ratio rather than by pixels. */
const MAX_CANVAS = 16_000_000;
/** Whether the reader last left the address columns on. Per browser, not per
 *  table: it is a way of reading, and a reader who wants the bytes wants them
 *  for the next file too. */
const ADDRESSES_KEY = "qubero.table.addresses";

/** What picking a row hands on: the row's field, and the bits it covers. */
export type TablePick = {
  readonly path: readonly number[];
  readonly startBit: number;
  readonly endBit: number;
};

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
  private addresses = false;
  private selected: number | null = null;
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
    this.addresses = localStorage.getItem(ADDRESSES_KEY) === "1";
    this.el = el("div", { className: "tableview" });
    this.head = el("div", { className: "tv-head" });
    this.scroller = el("div", { className: "tv-scroll" });
    this.scroller.tabIndex = 0;
    this.canvas = el("div", { className: "tv-canvas" });
    this.scroller.append(this.canvas);
    this.el.append(this.bar(opts.title), this.head, this.scroller);
    this.canvas.style.height = `${Math.min(MAX_CANVAS, plan.count * ROW)}px`;
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
    const bar = el("header", { className: "tv-bar" });
    bar.append(el("b", { className: "tv-title", textContent: title }));
    bar.append(el("span", { className: "tv-count", textContent: TABLE.count(this.plan.count, this.plan.rowWord) }));
    for (const fact of this.plan.facts) {
      const button = el("button", {
        type: "button",
        className: "tv-fact",
        textContent: `${fact.label} ${factValue(fact.value)}`,
      });
      button.title = TABLE.factTitle(fact.label);
      button.addEventListener("click", () => this.onFactPick(fact.path));
      bar.append(button);
    }
    const rate = this.plan.rate;
    if (rate !== null && rate > 0) {
      const word = this.plan.columnWord;
      const said = rate.toLocaleString();
      bar.append(
        el("span", {
          className: "tv-meaning",
          textContent:
            word === null
              ? TABLE.rowMeaningPlain(this.plan.rowWord, said)
              : TABLE.rowMeaning(this.plan.rowWord, word, said),
        }),
      );
    }
    const box = el("input", { type: "checkbox", className: "tv-addr-box", checked: this.addresses });
    box.addEventListener("change", () => {
      this.addresses = box.checked;
      localStorage.setItem(ADDRESSES_KEY, box.checked ? "1" : "0");
      this.layColumns();
      this.fillHead();
      this.paintAgain();
    });
    bar.append(el("label", { className: "tv-addr" }, box, TABLE.addresses));
    return bar;
  }

  // ----- the columns -----

  /** One track list for the header and every row, so the two line up without
   *  either measuring the other. The data columns share what is left over, so
   *  a table of three columns and a table of thirty both fill the tab and
   *  neither scrolls sideways. */
  private layColumns(): void {
    const time = this.plan.rate !== null && this.plan.rate > 0 ? " 12ch" : "";
    const data = this.plan.columns.map(() => "minmax(6ch, 1fr)").join(" ");
    const addresses = this.addresses ? " 12ch 9ch" : "";
    this.el.style.setProperty("--tv-cols", `8ch${time} ${data}${addresses}`);
  }

  private fillHead(): void {
    const cells: HTMLElement[] = [el("span", { className: "tv-th tv-index", textContent: TABLE.index })];
    if (this.plan.rate !== null && this.plan.rate > 0) {
      cells.push(el("span", { className: "tv-th", textContent: TABLE.time }));
    }
    for (const column of this.plan.columns) {
      const text = column.unit === "" ? column.name : `${column.name} (${column.unit})`;
      const cell = el("span", { className: "tv-th", textContent: text });
      cell.title = text;
      cells.push(cell);
    }
    if (this.addresses) {
      cells.push(el("span", { className: "tv-th", textContent: TABLE.storedAt }));
      cells.push(el("span", { className: "tv-th", textContent: TABLE.size }));
    }
    this.head.replaceChildren(...cells);
  }

  // ----- drawing -----

  /** Draw again once the frame is over, however many things asked. Streaming a
   *  file's first megabyte fires `onChange` far faster than a screen redraws. */
  private schedule(): void {
    if (this.frame !== 0) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      // What a row said may have been read from bytes that have since
      // arrived, so the answers go rather than being drawn again.
      this.have.clear();
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
    return Math.max(1, Math.floor(this.scroller.clientHeight / ROW));
  }

  /** True once the rows are taller than a canvas is allowed to be, which is
   *  where the scroll bar stops standing for pixels. */
  private get capped(): boolean {
    return this.plan.count * ROW > MAX_CANVAS;
  }

  /** The first row the scroll position is asking for. Under the cap that is
   *  division; past it the bar's place in its own travel is the reader's place
   *  in the rows, which is the only mapping left once the pixels run out. */
  private firstVisible(): number {
    if (!this.capped) return Math.floor(this.scroller.scrollTop / ROW);
    const travel = this.canvas.clientHeight - this.scroller.clientHeight;
    const ratio = travel <= 0 ? 0 : this.scroller.scrollTop / travel;
    return Math.round(ratio * Math.max(0, this.plan.count - this.onScreen()));
  }

  private paint(): void {
    const firstVisible = this.firstVisible();
    const first = Math.max(0, firstVisible - OVERSCAN);
    const last = Math.min(this.plan.count, firstVisible + this.onScreen() + OVERSCAN);
    // Under the cap a row sits at its own place on the canvas. Past it the
    // canvas is shorter than the rows would need, so the window is drawn where
    // the reader is looking: at the top of the viewport, wherever that is.
    const base = this.capped ? this.scroller.scrollTop - (firstVisible - first) * ROW : first * ROW;
    const was = this.drawn;
    if (was !== null && was.from === first && was.to === last && was.base === base && was.addresses === this.addresses) return;
    this.drawn = { from: first, to: last, base, addresses: this.addresses };
    const out: HTMLElement[] = [];
    for (let i = first; i < last; i++) {
      const row = this.rowAt(i);
      const element = row === null ? this.drawWaiting(i) : this.drawRow(i, row);
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
    if (row !== null) this.have.set(i, row);
    return row;
  }

  private drawRow(i: number, row: TableRow): HTMLElement {
    const element = el("div", { className: i === this.selected ? "tv-row is-on" : "tv-row" });
    element.append(el("span", { className: "tv-cell tv-index", textContent: i.toLocaleString() }));
    const rate = this.plan.rate;
    if (rate !== null && rate > 0) {
      element.append(el("span", { className: "tv-cell tv-time", textContent: timeText(i, rate) }));
    }
    for (let c = 0; c < this.plan.columns.length; c++) {
      element.append(this.drawCell(row.cells[c]));
    }
    if (this.addresses) {
      element.append(el("span", { className: "tv-cell tv-at", textContent: formatOffset(row.offsetBits) }));
      element.append(el("span", { className: "tv-cell tv-size", textContent: bitSizeText(row.sizeBits) }));
    }
    return element;
  }

  /** One cell. A cell naming another part of the file is a link to it, the
   *  same cross-reference a record table in the listing draws. */
  private drawCell(cell: RecordCell | undefined): HTMLElement {
    if (cell === undefined) return el("span", { className: "tv-cell" });
    const link = cell.link;
    if (link !== undefined) {
      const button = el("button", { type: "button", className: "tv-cell rec-link", textContent: link.text });
      button.title = link.label;
      button.addEventListener("click", (e) => {
        e.stopPropagation();
        this.onFactPick(link.path);
      });
      return button;
    }
    const element = el("span", { className: `tv-cell ${fieldClass(cell.kind)}`, textContent: cell.text });
    element.title = cell.text;
    return element;
  }

  /** A row whose bytes are not here yet. It keeps its place and its number, so
   *  the table does not jump when the answer arrives in it. */
  private drawWaiting(i: number): HTMLElement {
    const element = el("div", { className: "tv-row tv-waiting" });
    element.append(el("span", { className: "tv-cell tv-index", textContent: i.toLocaleString() }));
    element.append(el("span", { className: "tv-cell", textContent: REPORT.paneWaiting }));
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
    const at = this.plan.rowFor(bit);
    if (at === null) return;
    this.selected = at;
    this.scrollToRow(at);
    this.paintAgain();
  }

  clearSelection(): void {
    this.selected = null;
    this.paintAgain();
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
    const travel = this.canvas.clientHeight - this.scroller.clientHeight;
    const rows = Math.max(1, this.plan.count - onScreen);
    this.scroller.scrollTop = Math.max(0, Math.min(travel, (want / rows) * travel));
  }

  // ----- input -----

  private onClick(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof Element)) return;
    const at = target.closest<HTMLElement>(".tv-row")?.dataset["index"];
    if (at === undefined) return;
    this.pick(Number(at));
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
    const page = Math.max(1, this.onScreen() - 1);
    const by: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page };
    const move = by[e.key];
    const from = this.selected ?? this.firstVisible();
    if (move !== undefined) {
      e.preventDefault();
      this.pick(Math.max(0, Math.min(this.plan.count - 1, from + move)));
      return;
    }
    if (e.key === "Home") {
      e.preventDefault();
      this.pick(0);
    } else if (e.key === "End") {
      e.preventDefault();
      this.pick(this.plan.count - 1);
    }
  }

  private pick(i: number): void {
    const row = this.rowAt(i);
    this.selected = i;
    this.scrollToRow(i);
    this.paintAgain();
    if (row === null) return;
    this.picking = true;
    this.onPick({ path: row.path, startBit: row.offsetBits, endBit: row.offsetBits + row.sizeBits });
    this.picking = false;
  }
}

/** A fact's value as the bar shows it: a number gets its thousands separators,
 *  since `44,100` is read at a glance and `44100` is counted. Anything else is
 *  shown as the core wrote it. */
function factValue(value: string): string {
  const n = Number(value);
  return value.trim() !== "" && Number.isFinite(n) ? n.toLocaleString() : value;
}

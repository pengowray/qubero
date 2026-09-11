// The elements the hex grid is made of, and the writing of one frame into them.
//
// Nothing here decides anything: `hexview.ts` works out where the view is,
// what the bytes and the fields on screen are and how tall the rows will be,
// gathers the lot into a `Frame`, and hands it over. What this file owns is
// the pool of elements and the rule that governs it.
//
// **The pool is never taken apart.** A redraw writes over the elements that
// are there rather than building them again: a row with fewer chips hides the
// spare ones, a row past the end of the file is hidden rather than emptied,
// and a line a row no longer needs is left in the pool. Two reasons, and the
// second is the one that bites: on a touch screen a finger may be resting on
// an element, and an element that leaves the document is a touch the browser
// calls off, which stops the drag that is scrolling the view.
//
// **An element stands for an address, not for a place on screen.** The pool
// used to be read by the row's place in the view: element 0 drew whatever the
// top row was, so a scroll of two rows rewrote all twenty-nine of them, and
// scrolling by a notch cost what scrolling by a screenful cost. Now `place`
// hands each address the element that already held it, and only the rows
// arriving at an edge are written. Which row is drawn where is said by the
// flex `order` on the element, so the document's order never changes: nothing
// is moved, nothing is taken out, and a finger stays on what it was on. See
// `place` for what the number is and why it is not just the row's address.
//
// The frame's type is imported back from `hexview.ts`. Only the type: the
// class here is what that file imports, so at run time the two go one way.

import type { OutlineHeading } from "./outline.ts";
import type { Span } from "./doc.ts";
import type { Frame } from "./hexview.ts";
import { chipsHead, NO_TEMPLATE } from "./strings.ts";
import type { ChipMeasure } from "./chipfit.ts";
import { pinnedNoteKey, planRowChips, rowNoteKey, valsBeforeChips, type Chip, type ChipBlock, type Reading } from "./chipplan.ts";
import { cellDraw, covers, HEX, highlightBits, selectionBits, setText, type Run } from "./hexcell.ts";
import { chipsOf, fillNote, fillPlain, newChip, readChipFonts, valsOf, type ChipEl } from "./hexchips.ts";
import { fillHeadings, rowPieces, type RowPieces } from "./hexheadings.ts";
import { fillVals, markVals, newVals, readValFont } from "./valuecells.ts";
import { NO_VALUES, type PlacedCell, type RowValues } from "./valuetable.ts";

/** What pressing something in a row does. Held as one object for the life of
 *  the view: every chip and every heading keeps the function it was built
 *  with, so a chip filled again is not a chip built again. */
export type RowPicks = {
  /** `throughBit` is the end of a folded run, when the chip pressed stands for
   *  one: the pick is the whole run and not its first element. */
  readonly field: (path: readonly number[], throughBit?: number) => void;
  readonly value: (path: readonly number[], bit: number) => void;
  readonly heading: (h: OutlineHeading) => void;
};

/**
 * Put the table of a folded run's values where the bytes say it goes.
 *
 * The table is the block's last child and stays that way: `valsOf` reads it
 * off the end and `fillNote` puts a new chip in front of it. What moves is
 * where it is *drawn*, by the flex `order` the rows themselves are placed
 * with, so nothing is taken out of the document under a finger.
 *
 * `valsBeforeChips` holds the reasoning and is where the rule is tested.
 */
function orderVals(block: HTMLElement, first: ChipBlock | undefined, vals: RowValues): void {
  const last = vals.cells[vals.cells.length - 1];
  const order = valsBeforeChips(first, last === undefined ? null : last.endBit) ? "-1" : "";
  if (block.style.order !== order) block.style.order = order;
}

/** What a row's table key is set to when what the block holds is no longer
 *  known: a row past the end of the file, or one whose lines were laid out
 *  again. Not the empty string, which is the key of a row with no table, so
 *  that a table left in the block by the row before is written over rather
 *  than taken for the empty block the new row wants. */
const VALS_UNKNOWN = "\u0000";

/** How far the view may wander from the mark the flex orders are counted from
 *  before the mark is moved, and half of which is left free on each side of it
 *  when it is. See `place`: the order is a 32-bit integer and a row number is
 *  not, so what goes on the element is a distance rather than an address. */
const ORDER_SPAN = 1_000_000;

/** One line of cells: an address, the bytes, their text and their fields. A row
 *  is one of these unless a part starts part-way along it. */
type LineParts = {
  readonly line: HTMLElement;
  readonly addr: HTMLElement;
  readonly cells: HTMLElement;
  readonly asc: HTMLElement;
  readonly note: HTMLElement;
  /** The parts that start where this line begins, drawn above it. Always
   *  present, and empty when none do. */
  readonly head: HTMLElement;
  readonly hex: readonly HTMLElement[];
  readonly text: readonly HTMLElement[];
};

/** The elements one row on screen is drawn in, and what they last said. */
type RowParts = {
  /** The lines of cells the row is drawn as. One unless a part starts
   *  part-way along the row, in which case the row is cut where it starts so
   *  the heading can sit between the bytes before it and the bytes after.
   *  Spare lines are kept for reuse and left out of the row. */
  lines: LineParts[];
  /** Which line's cell each byte of the row is drawn in, by position in the
   *  row. The same position in every other line is left blank, so the bytes
   *  stay under their column whichever line they ended up on. */
  hexCells: HTMLElement[];
  textCells: HTMLElement[];
  /** The byte the row starts at, so the addresses on its cells are written
   *  again only when the view has moved. */
  start: number;
  /** The row this element is standing for, or -1 for one that stands for
   *  nothing yet. Kept apart from `start`, which is the address the cells were
   *  last written with: a row past the end of the file is given an element and
   *  never written, so the two say different things and only this one says
   *  which element `place` should hand that row next time. */
  holds: number;
  /** True for a row past the end of the file, which is emptied rather than
   *  drawn. */
  blank: boolean;
  /** The headings on the row and where the row is cut for them, so the lines
   *  and heading blocks are built again only when either changes. */
  layoutKey: string;
  /** What the row's chips last said, so they are built again only when they
   *  would say something else. Cleared whenever the lines they live in are. */
  noteKey: string;
  /** The same for the row's table of values, kept apart from the chips so
   *  that a value changing does not rewrite the chips beside it. */
  valsKey: string;
};

/** What the side column came out at, once there is a row to read it off. */
export type NoteMetrics = {
  /** How wide the column is inside its own padding, which is the width a
   *  chip has to fit in. */
  readonly width: number;
  /** Where it starts, so the pinned strip can stand over it. */
  readonly left: number;
};

export class HexRows {
  /** The row of column numbers over the bytes. */
  readonly header: HTMLElement;
  /** The rows themselves, shifted up by the scroll position so the top row
   *  can be partly above the edge. The caller clips it. */
  readonly inner: HTMLElement;
  /** The chips for fields that began above the visible rows, pinned over the
   *  top edge of the rows rather than drawn inside the top one. In the flow it
   *  made the top row a line of chips taller than the same row is anywhere
   *  else, so every row below it jumped as a long field scrolled past. Hidden
   *  when nothing is carried. */
  readonly pinned: HTMLElement;
  /** What the pinned strip last said, so it is filled again only when it would
   *  say something else. */
  private pinnedKey = "";
  /** The pool, in the order the elements were made and put in the document.
   *  Nothing ever moves inside it. */
  private rowEls: HTMLElement[] = [];
  /** The elements each row is made of, kept between draws. See `fitParts`. */
  private parts: RowParts[] = [];
  /** Which element of the pool draws each row of the view, top row first, and
   *  the same elements again for the callers that want them that way. Both are
   *  written by `place` and read by everything after it. */
  private win: number[] = [];
  private winEls: HTMLElement[] = [];
  /** What is taken off a row's address to get its flex `order`. The order is a
   *  32-bit integer in the browser and a row number is not: a file can have
   *  more rows than that, and `rowheights.ts` counts them in doubles for the
   *  same reason. So the number written is the row's distance from a mark that
   *  is moved up to the view whenever the view has wandered far from it, which
   *  keeps every order small and positive and costs a rewrite of the
   *  twenty-nine orders once every million rows. */
  private orderBase = 0;
  private partsShape = "";
  private headerShape = "";
  /** What `fitParts` last built the lines for, so a line added mid-draw for a
   *  row that had to be cut is built the same way. */
  private lineShape = { bpr: 16, binary: false, showText: true, fields: false, below: false };
  /** What the top row carries in from above, found by the row that names it
   *  and read by the strip a moment later in the same `write`. */
  private carried: ChipBlock | null = null;

  constructor(private readonly picks: RowPicks) {
    this.header = document.createElement("div");
    this.header.className = "hv-header";
    this.inner = document.createElement("div");
    this.inner.className = "hv-rows-inner";
    this.pinned = document.createElement("span");
    this.pinned.className = "hv-note hv-note-pinned hv-empty";
  }

  /** The row elements in the order they are drawn, top row first, for reading
   *  a point on screen back to a byte. Not the order they sit in the document,
   *  which stopped meaning anything when rows began to be reused. */
  get rows(): readonly HTMLElement[] {
    return this.winEls;
  }

  /** The last row of the view, which is what a drag pulled below the rows is
   *  pinned to. Read in drawn order for the same reason `rows` is. */
  lastRow(): HTMLElement | null {
    return this.winEls[this.winEls.length - 1] ?? null;
  }

  /** Put the rows at the scroll position: the top row starts above the edge
   *  by however much of it is hidden. */
  setOffset(topPx: number): void {
    this.inner.style.transform = topPx === 0 ? "" : `translateY(${-topPx}px)`;
  }

  /**
   * Write a frame into the rows, and say how tall each of them should come
   * out. A row past the end of the file is zero, which is what tells the
   * caller which rows are there at all.
   */
  write(f: Frame): number[] {
    this.drawHeader(f);
    this.fitParts(f.bpr, f.binary, f.showText, f.fields, f.below);
    this.place(f.bpr === 0 ? 0 : Math.floor(f.start / f.bpr));
    this.carried = null;
    const heights: number[] = [];
    for (let r = 0; r < this.win.length; r++) {
      const h = this.drawRow(r, f);
      if (h !== null) heights.push(h);
    }
    this.drawPinned();
    return heights;
  }

  /**
   * Hand each row of the view the element that is to draw it.
   *
   * A row that was on screen last draw keeps the element it was in, wherever
   * on screen it has moved to; the rows arriving at an edge take the elements
   * the rows that left the other edge have given up. The element that drew a
   * row still says so, in `start` and in the keys beside it, so a row that
   * kept its element finds every one of those guards already holding what it
   * was going to write and writes nothing at all. That is the whole of the
   * saving: a one-notch scroll used to rewrite twenty-nine rows and now writes
   * the two that arrived.
   *
   * Where a row is drawn is said by the flex `order` on its element rather
   * than by its place among the children. Two things follow from that and both
   * are the point:
   *
   *  - Nothing is ever moved in the document, so a finger resting on a row
   *    keeps the element it is resting on. Reusing rows makes the rule about
   *    not detaching a row under a touch easier to keep, not harder.
   *  - The rows are still laid out one under the next by the browser, at
   *    whatever height each came out. Nothing here positions a row, so a row
   *    whose height was predicted wrong still pushes the ones below it down
   *    rather than landing on top of them. The other way of doing this —
   *    absolute tops written from the measured heights — would have made the
   *    draw responsible for that, and for nothing gained.
   *
   * **What stops a reused row saying something out of date.** Nothing here
   * decides that a row needs no work: every row is still drawn in full on
   * every draw, `cellDraw` still works out what each of its cells says, and
   * every write is still `if (it differs) write it`. A row that kept its
   * element finds those guards already holding what it was going to write, so
   * it writes nothing; a row given a recycled element finds them holding the
   * row that element used to draw, so it writes everything. The guards are the
   * ones that were already there and not one of them was loosened:
   *
   *  - `start` is the address the cells were written with, so a recycled
   *    element has `moved` true and every `data-off` is written again.
   *  - `layoutKey` covers what the row is *made of*: where it is cut, which
   *    parts start on it, how wide the address column is. `noteKey` covers
   *    every chip the row shows, taken from the plan's own output rather than
   *    from what went into it; `valsKey` covers the table of values.
   *  - The headings themselves are not keyed at all. They were, by the part's
   *    identity, and that turned out to be a key that misses: a part's range,
   *    its size and its share of the file all change while it stays the same
   *    part, and an unmapped run at the end of a file does it on every edit.
   *    While an element drew a different row on every scroll the stale text
   *    was always written over before anyone saw it. It is now `drawHeads`'s
   *    business, every draw, guarded write by guarded write like the cells.
   *  - `fitParts` throws every one of them away when the shape of the view
   *    changes, and clears `holds` with them, so a row drawn at eight bytes to
   *    the line cannot be handed an element that drew sixteen.
   *  - The three things a row says because of *where it is* rather than which
   *    address it holds — the top row's carried chips, the last row's "more
   *    fields below", and the rule under the column header — all reach the
   *    element through `noteKey` or through the `hv-row-top` class written
   *    below.
   *
   * `noteKey` was built from what a chip *says* and not from which field it is,
   * which left two fields with the same name and the same value in different
   * structures keying the same: an element recycled from one to the other kept
   * the tooltip and the path a press on it follows. It was as possible before,
   * when every element drew a new row every scroll, but a recycled element made
   * it the ordinary way in rather than the rare one. The field's path is part
   * of the key now; see `fieldKey` in `chipplan.ts`.
   *
   * None of this was left to reasoning: `web/tools/staleness.mjs` runs the same
   * script of scrolls, cursor moves, selections, an edit, mode changes, resizes
   * and row widths against a server drawing the old way and one drawing this
   * way, and compares every visible cell, class, chip and heading after every
   * step, as well as asking the browser whether the rows really do fall down
   * the screen in the order the view hands them out. Twelve sample files, and
   * the only differences it turned up are the headings, in the old drawing:
   *
   *  - `tagged.mp3`: type a byte past the end of the file and the heading over
   *    the unclaimed run at the end goes on saying what it was a byte ago.
   *  - `initramfs.img`: the same edit leaves a heading drawn at the wrong
   *    level, which is a different height as well as a different size of text,
   *    so the row it sits on is the wrong height too.
   *
   * Both outlive five more draws at the same place, so they are not a frame's
   * lag; they last until something else moves the row.
   *
   * Everything else it reported was about something other than the drawing,
   * and each took a control run to say so. Worth knowing before spending a day
   * on one:
   *
   *  - Two pages doing the same thing are not doing it at the same moment. The
   *    listing walks the file in the background and the chips come from an
   *    answer that arrives when it arrives, so a step taken a quarter of a
   *    second after a long jump or a change of row width can catch one page
   *    with a heading or a highlight the other does not have yet. Drive the
   *    same step on its own and both agree; wait and redraw and both agree.
   *  - The old build's checkout has to be one nobody is working in. Half of
   *    these differences turned out to be another branch's uncommitted work
   *    and a wasm rebuild being served as the thing to beat.
   *
   * **What it is worth.** `web/tools/wheelcost.mjs` on `hello.exe` at
   * 1280x800, runs interleaved against a server on the commit before this one:
   *
   * | | attributes written | text written | browser layout | browser style | draw |
   * |---|---|---|---|---|---|
   * | one notch, before      |  1,406 |    508 |   6-8ms |  2-3ms | 16-23ms |
   * | one notch, after       |     60 |     18 |     1ms |    1ms |  7-8ms |
   * | thirty notches, before | 32,158 | 12,358 | 118ms | 50ms | 311ms |
   * | thirty notches, after  |  1,742 |    464 |  13ms | 12ms | 114ms |
   *
   * The counts are what to read: they are deterministic, and the times on this
   * machine are not — the same code measured 311ms and 571ms for the same
   * thirty notches an hour apart. Measure the two alternately, never all of
   * one and then all of the other, and take the "before" from a checkout that
   * nobody is editing: half a day went into differences that turned out to be
   * another branch's uncommitted work being served as the thing to beat.
   *
   * What is left in the draw is script — working out what every row would say,
   * so as to find that it already says it — and the next thing worth attacking
   * is `placeSpans`, at about 1.8ms a draw, rather than anything here.
   */
  private place(topRow: number): void {
    const n = this.rowEls.length;
    // Far from the mark the orders are counted from, or behind it: move it,
    // and leave room on both sides of the view. Putting the mark on the top
    // row instead would leave the next scroll upward crossing it again, and
    // every notch up after that would rewrite all twenty-nine orders. Every
    // order below is written again on the draw that moves the mark, which is
    // one draw in half a million rows.
    if (topRow < this.orderBase || topRow - this.orderBase > ORDER_SPAN) this.orderBase = Math.max(0, topRow - ORDER_SPAN / 2);
    const win: number[] = new Array(n).fill(-1);
    const spare: number[] = [];
    for (let i = 0; i < n; i++) {
      const w = (this.parts[i] as RowParts).holds - topRow;
      // `win[w] === -1` as well as the range: two elements claiming one row
      // can only happen if something below got out of step, and the second one
      // becoming spare is the reading that leaves every row drawn once.
      if (w >= 0 && w < n && win[w] === -1) win[w] = i;
      else spare.push(i);
    }
    let s = 0;
    for (let w = 0; w < n; w++) {
      if (win[w] === -1) win[w] = spare[s++] as number;
      const i = win[w] as number;
      (this.parts[i] as RowParts).holds = topRow + w;
      const el = this.rowEls[i] as HTMLElement;
      const order = String(topRow + w - this.orderBase);
      if (el.style.order !== order) el.style.order = order;
      // Which row of the file this is. The grid's children are no longer in
      // the order they are drawn, so a reader going through them one at a time
      // would be told the wrong thing by their arrangement; this is the
      // attribute `role="grid"` has for saying it outright. It is the row's
      // own number, so it is written when a row is given a new element and not
      // again while it keeps it.
      const at = String(topRow + w + 1);
      if (el.getAttribute("aria-rowindex") !== at) el.setAttribute("aria-rowindex", at);
      // The top row is named on the element: the stylesheet takes the rule off
      // the first heading of the view so it does not double the column
      // header's own, and which element that is no longer follows from where
      // it sits among the children.
      if (el.classList.contains("hv-row-top") !== (w === 0)) el.classList.toggle("hv-row-top", w === 0);
      this.winEls[w] = el;
    }
    this.win = win;
    this.winEls.length = n;
  }

  /**
   * How tall each row really came out, against what `write` predicted.
   *
   * One forced layout for the lot. The prediction decides how many lines of
   * chips a row holds and what it counts as left over; what the view scrolls
   * by has to be what the browser drew, or a row taller than it was reckoned
   * to be spills over the one below it.
   *
   * **This read looks like the draw's biggest cost and is not.** It shows up
   * that way because `offsetHeight` makes the browser lay the page out before
   * it answers, so the frame's layout happens inside this call and a timer
   * around the draw charges the draw for it. Take the call out and the layout
   * simply happens at the end of the frame instead, for the same money. Two
   * ways of taking it out were measured against the same machine in the same
   * session, three to four interleaved runs each, on `hello.exe`,
   * `notes.sqlite` and `zarr-zip64.zip`, at viewports up to 1920x2400 where
   * frames were being dropped:
   *
   *  - Answering from the prediction and never reading at all.
   *  - Reading at the head of the next draw instead, off the rows the browser
   *    has already laid out and painted, which is free.
   *
   * Both halved the browser's layout count, 52 to 30 over thirty wheel
   * notches, and neither moved its layout time, its style time, its script
   * time, how many draws a spin got through, or how many frames went late.
   * The second layout was the cheap one: nothing had been written since the
   * first, so there was nothing for it to redo. Both were reverted. Attacking
   * this read again needs a reason better than its share of the draw timer,
   * and `web/tools/wheelcost.mjs` will show you the same flat numbers.
   *
   * What the prediction is worth is a separate question, and mostly it is
   * worth a lot. Compared against this read, row by row: at 1280x800 and 16
   * bytes to the row, sixty draws of scrolling over `hello.exe`, `notes.sqlite`
   * and `bat.wav` predicted every row exactly. Put a viewport resize in the
   * middle of the run and it stops agreeing: most rows then come out one or
   * two pixels taller than predicted, and a handful come out a whole chip line
   * shorter. Why a resize should do that was not chased down. So nothing
   * downstream may assume the two agree, and the ones that are out by a line
   * rather than a pixel are the ones that could move a row under a reader.
   */
  heights(predicted: readonly number[]): number[] {
    return predicted.map((h, i) => (h === 0 ? 0 : (this.winEls[i]?.offsetHeight ?? h)));
  }

  /** The fonts a chip's name and its value are drawn in, read off a chip that
   *  has been drawn. Null until there is one. */
  chipFont(): ChipMeasure | null {
    return readChipFonts(this.inner);
  }

  /**
   * The font a value cell is drawn in, read off a cell that has been drawn.
   * Null until there is one.
   *
   * Asked for on its own rather than beside the chips'. Most files draw no
   * table of values at all, so that answer was null on every draw for ever and
   * the caller, having nothing to remember, asked again next draw: a search of
   * the whole grid, a computed style and a throwaway canvas, once a frame, for
   * a font nothing was going to be measured in. The caller now asks only on a
   * frame that has a cell to read.
   */
  valueFont(): ChipMeasure | null {
    return readValFont(this.inner);
  }

  /** The side column's width and where it starts, read off the first row.
   *  Null when there is no column to read. */
  noteMetrics(): NoteMetrics | null {
    const noteEl = this.winEls[0]?.querySelector(".hv-note") as HTMLElement | null;
    if (noteEl === null || noteEl === undefined) return null;
    // `clientWidth` counts the note's own left padding, which no chip can be
    // drawn in.
    const pad = parseFloat(getComputedStyle(noteEl).paddingLeft) || 0;
    return {
      width: Math.max(0, noteEl.clientWidth - pad),
      left: noteEl.getBoundingClientRect().left - this.inner.getBoundingClientRect().left,
    };
  }

  /** How wide a byte of the bytes is drawn, so a byte of an aligned value
   *  table can be drawn at the same pitch. Zero when there is none to read. */
  hexPitch(binary: boolean): number {
    const cell = this.winEls[0]?.querySelector(binary ? ".hv-bits > span" : ".hv-hex > span");
    return cell instanceof HTMLElement ? cell.getBoundingClientRect().width : 0;
  }

  /** The cell one byte of a row on screen is drawn in, by the row's place in
   *  the pool and the byte's place in the row. A heading may have cut the row,
   *  so which line the cell is on is the pool's business, not the caller's. */
  cellFor(row: number, at: number): HTMLElement | undefined {
    return this.parts[this.win[row] ?? -1]?.hexCells[at];
  }

  /** How wide a whole row is, which is what a note below the bytes gets. */
  rowWidth(): number {
    return this.winEls[0]?.clientWidth ?? 0;
  }

  /** Stand the pinned strip over the side column rather than over the whole
   *  row, so the bytes underneath stay readable and the chips keep the
   *  column's hairline and indent. */
  setPinnedSide(side: boolean, left: number): void {
    if (this.pinned.classList.contains("hv-note-pinned-side") !== side)
      this.pinned.classList.toggle("hv-note-pinned-side", side);
    const pinLeft = side ? `${left}px` : "";
    if (this.pinned.style.left !== pinLeft) this.pinned.style.left = pinLeft;
  }

  /** One line of cells, built for the shape the view is currently drawn in. */
  private makeLine(): LineParts {
    const { bpr, binary, showText, fields, below } = this.lineShape;
    const line = document.createElement("div");
    line.className = "hv-line";
    const addr = document.createElement("span");
    addr.className = "hv-addr";
    const cells = document.createElement("span");
    cells.className = binary ? "hv-bits" : "hv-hex";
    const asc = document.createElement("span");
    asc.className = "hv-ascii";
    const note = document.createElement("span");
    note.className = below ? "hv-note hv-note-below" : "hv-note";
    const head = document.createElement("div");
    head.className = "hv-headings";
    const hex: HTMLElement[] = [];
    const text: HTMLElement[] = [];
    for (let i = 0; i < bpr; i++) {
      const h = document.createElement("span");
      const a = document.createElement("span");
      // Which pane a cell belongs to never changes, so it is written once.
      h.setAttribute("data-pane", "hex");
      a.setAttribute("data-pane", "ascii");
      cells.append(h);
      asc.append(a);
      hex.push(h);
      text.push(a);
    }
    line.append(addr, cells);
    if (showText) line.append(asc);
    // Beside the bytes the note is part of the line; below them it is a block
    // of its own after the line, so that it can use the row's whole width.
    if (fields && !below) line.append(note);
    return { line, addr, cells, asc, note, head, hex, text };
  }

  /**
   * Lay a row out: its lines, the heading blocks between them, and which line
   * draws each byte.
   *
   * A part that starts part-way along a row cuts the row there. Both pieces
   * keep their place in the columns — the bytes before the cut leave the rest
   * of the first line blank, the bytes after it leave the front of the second
   * line blank — so a byte is always under the column header that names it.
   * Only the first line carries the address, since a row address is a multiple
   * of the row width and the address of a cut is not.
   */
  /**
   * Write the headings above a row's lines, on every draw.
   *
   * Not behind `layoutKey`, which names the parts that start on the row and
   * not what any of them says. A heading's name, its range, how big it is and
   * how much of the file that is all change under the same key: an unmapped
   * run at the end of a file grows as the file is edited or as more of it
   * arrives, and the part that names it keeps its key throughout. While a row
   * element drew a different row on every scroll that never showed, because
   * the key changed for the row rather than for the heading. It shows now, so
   * the headings are worked out every draw and written where they differ, the
   * way the cells are. `fillHeadings` guards every write, and a row with no
   * heading — which is nearly all of them — costs two comparisons.
   */
  private drawHeads(parts: RowParts, at: RowPieces, fileBits: number): void {
    const { rowStart, segs } = at;
    for (const [j, lp] of parts.lines.entries()) {
      const on = j < segs.length;
      const pos = on ? (segs[j] as number) : 0;
      fillHeadings(lp.head, on ? (at.heads[j] ?? []) : [], fileBits, rowStart + pos, this.picks.heading);
    }
  }

  private layOutRow(row: HTMLElement, parts: RowParts, at: RowPieces, addrWidth: number): void {
    const { segs } = at;
    const { bpr, binary, fields, below } = this.lineShape;
    while (parts.lines.length < segs.length) parts.lines.push(this.makeLine());
    // Every line the row has ever needed, in order, whether or not this
    // drawing uses it. A row that stops being cut hides its second line rather
    // than dropping it, so the list of things in the row only ever grows and a
    // scroll never takes an element out from under a finger.
    const kids: HTMLElement[] = [];
    for (const [j, lp] of parts.lines.entries()) {
      const on = j < segs.length;
      // Always in the row, empty when no part starts here, so that a heading
      // arriving or leaving writes into a block that is already there. What
      // goes in it is `drawHeads`'s business, on every draw rather than only
      // on the ones that lay the row out again.
      kids.push(lp.head);
      if (lp.line.hidden === on) lp.line.hidden = !on;
      kids.push(lp.line);
      if (fields && below) {
        if (lp.note.hidden === on) lp.note.hidden = !on;
        kids.push(lp.note);
      }
    }
    // Only when the row really is made of different things. `replaceChildren`
    // takes every child out and puts it back even when the list it is given is
    // the one already there, and a finger resting on an element that is taken
    // out of the document is a touch the browser calls off — which stops the
    // drag that is scrolling the view.
    for (const [i, kid] of kids.entries()) {
      if (row.childNodes[i] !== kid) row.insertBefore(kid, row.childNodes[i] ?? null);
    }
    const blankHex = binary ? "        " : "  ";
    for (const [j, from] of segs.entries()) {
      const to = segs[j + 1] ?? bpr;
      const lp = parts.lines[j] as LineParts;
      // Every line but the first has the address column held open and empty,
      // so its bytes line up with the ones above.
      if (j > 0) setText(lp.addr, " ".repeat(addrWidth));
      for (let i = 0; i < bpr; i++) {
        const h = lp.hex[i] as HTMLElement;
        const a = lp.text[i] as HTMLElement;
        if (i >= from && i < to) {
          parts.hexCells[i] = h;
          parts.textCells[i] = a;
          continue;
        }
        // Held open but empty. Dropping `data-off` is what keeps a click on
        // the blank half of a cut row from landing on the byte the cell used
        // to draw.
        h.className = "";
        h.style.backgroundImage = "";
        h.textContent = blankHex;
        h.removeAttribute("data-off");
        a.className = "";
        a.textContent = " ";
        a.removeAttribute("data-off");
      }
    }
  }

  ensure(want: number, shrink = true): void {
    while (this.rowEls.length < want) {
      const r = document.createElement("div");
      r.className = "hv-row";
      r.setAttribute("role", "row");
      this.inner.append(r);
      this.rowEls.push(r);
    }
    // `shrink` is false while a finger is on a row. Taking an element out of
    // the document is a touch the browser calls off, which stops the drag that
    // is scrolling the view, and the pool is not in screen order: the element
    // this would pop holds whichever file row it last drew, which can be the
    // row under the finger while the row is still on screen. So a viewport
    // that got smaller mid-drag keeps its spare rows until the finger lifts.
    // They go on being drawn below the bottom edge for the length of the drag,
    // which is a few rows of work against a drag that stops dead.
    if (shrink) {
      while (this.rowEls.length > want) {
        this.rowEls.pop()?.remove();
        this.parts.pop();
      }
    }
    // What `place` last worked out is about a pool that is no longer this
    // size. Trimmed rather than emptied: `rows` is read between here and the
    // next `write` -- `fitRows` reads a row to take its height off the
    // stylesheet -- and an empty answer there reads as a view with no rows at
    // all. A pool that grew leaves the old window standing, which is a true
    // answer about fewer rows than there now are, and `place` replaces it
    // whole on the next draw either way.
    if (shrink && this.win.length > want) {
      this.win.length = want;
      this.winEls.length = want;
    }
  }

  /**
   * Make sure every row on screen has its spans, and that they are the spans
   * this shape of view wants.
   *
   * A redraw writes over them rather than building them again. Moving the
   * cursor one byte changes two cells out of six hundred, and throwing the
   * six hundred away to say so was most of what a keypress cost. The shape —
   * how many bytes to a row, which columns are showing, hex or binary —
   * decides what the spans are, so a change to any of it starts them again.
   */
  private fitParts(bpr: number, binary: boolean, showText: boolean, fields: boolean, below: boolean): void {
    const shape = `${bpr}|${binary}|${showText}|${fields}|${below}`;
    if (shape === this.partsShape && this.parts.length === this.rowEls.length) return;
    const changed = shape !== this.partsShape;
    this.partsShape = shape;
    this.lineShape = { bpr, binary, showText, fields, below };
    if (changed) this.parts = [];
    for (let i = this.parts.length; i < this.rowEls.length; i++) {
      const row = this.rowEls[i] as HTMLElement;
      const first = this.makeLine();
      row.replaceChildren(first.line);
      this.parts.push({
        lines: [first],
        hexCells: [...first.hex],
        textCells: [...first.text],
        start: -1,
        holds: -1,
        blank: false,
        layoutKey: "",
        noteKey: "",
        valsKey: "",
      });
    }
  }

  /** The eight bits of one byte, split into spans only where that is needed. */
  private fillBits(f: Frame, cell: HTMLElement, byte: number | null, off: number, hl: readonly Run[], sel: Run | null): void {
    const text = byte === null ? "········" : byte.toString(2).padStart(8, "0");
    if (byte === null) cell.classList.add("hv-pending");
    const onCursor = off === f.cursor;
    const whole = covers(hl, 0, 8);
    const selClass = f.pane === "hex" ? "hv-sel" : "hv-sel-weak";
    const selWhole = sel !== null && sel.from <= 0 && sel.to >= 8;
    // A whole selected byte is marked on the cell rather than on its bits, so
    // the space between two bytes is inside the selection and not a hole in it.
    if (selWhole) cell.classList.add(selClass);
    if (!onCursor && (hl.length === 0 || whole) && (sel === null || selWhole)) {
      cell.textContent = text;
      if (whole) cell.classList.add("hv-hl");
      return;
    }
    for (let k = 0; k < 8; k++) {
      const s = document.createElement("span");
      s.textContent = text[k] ?? "0";
      s.setAttribute("data-off", String(off));
      s.setAttribute("data-bit", String(k));
      s.setAttribute("data-pane", "hex");
      if (hl.some((r) => k >= r.from && k < r.to)) s.classList.add("hv-hl");
      if (sel !== null && !selWhole && k >= sel.from && k < sel.to) s.classList.add(selClass);
      if (onCursor && k === f.bit) {
        s.classList.add("hv-cur", f.pane === "hex" ? "hv-focus" : "hv-dim");
        if (f.insertMode) s.classList.add("hv-ins");
      }
      cell.append(s);
    }
  }

  /** The row of column numbers over the bytes, and the word over the column
   *  beside them. */
  private drawHeader(f: Frame): void {
    // What the column is holding, which is part of the header's shape: the
    // heading follows the entries, so a screen of one thing and a screen of
    // several head differently and the row has to be written again between
    // them.
    //
    // Read off the chips the rows were given rather than off `f.spans`, which
    // is whatever the last answer covered: spans describe whole fields, so an
    // answer taken for one window is kept while it reaches, and a screen deep
    // inside a compressed run still carries the header fields it was first
    // asked about.
    //
    // Only where there is a heading to write. The rows are walked once per
    // draw and a draw is once per frame of a scroll, so the walk is skipped
    // outright where the column is not headed rather than thrown away after.
    const head = f.fields && !f.below ? chipsHead(chipSpans(f.byRow)) : "";
    const shape = `${f.addrWidth}|${f.bpr}|${f.binary}|${f.showText}|${f.fields}|${f.below}|${head}`;
    if (shape === this.headerShape) return;
    this.headerShape = shape;
    const columns = document.createElement("span");
    columns.textContent =
      " ".repeat(f.addrWidth) +
      "  " +
      Array.from({ length: f.bpr }, (_, i) => (f.binary ? (HEX[i] ?? "").padEnd(8) : HEX[i])).join(" ");
    this.header.replaceChildren(columns);
    if (f.showText) {
      // Nothing to label, but the width has to be held so the heading over the
      // fields lands over the fields.
      const gap = document.createElement("span");
      gap.className = "hv-ascii";
      gap.textContent = " ".repeat(f.bpr);
      this.header.append(gap);
    }
    // Nothing to head when the chips are below the bytes: the header sits over
    // the bytes, and the fields no longer do.
    if (f.fields && !f.below) {
      const title = document.createElement("span");
      title.className = "hv-note hv-head-note";
      title.textContent = head;
      this.header.append(title);
    }
  }

  /**
   * Draw one row and say what it will be tall, or null for a row there is
   * nothing to draw into.
   *
   * The height is worked out from what was put in the row rather than read
   * back off it: reading would force a layout per row, and the draw is
   * arranged so the browser is asked once, at the end.
   */
  private drawRow(r: number, f: Frame): number | null {
    const i = this.win[r] ?? -1;
    const row = this.rowEls[i];
    const parts = this.parts[i];
    if (!row || parts === undefined) return null;
    const { bpr, len, start } = f;
    const rowStart = start + r * bpr;
    if (rowStart > len) {
      if (!parts.blank) {
        // Hidden, not emptied: a row past the end of the file can scroll back
        // into use, and taking its elements away would drop whatever a finger
        // is on. `layOutRow` shows them again.
        for (const kid of row.children) (kid as HTMLElement).hidden = true;
        parts.blank = true;
        parts.layoutKey = "";
        parts.noteKey = "";
        parts.valsKey = VALS_UNKNOWN;
      }
      return 0;
    }
    parts.blank = false;
    const heads = f.headsByRow[r] ?? [];
    const at = rowPieces(heads, rowStart, bpr, f.condensed, f.sizes, f.rowHeight);
    // Which parts start on this row and where the row is cut for them, which
    // is what the lines and the blocks between them are made of, and how wide
    // the address column is, which is what a cut row's later lines hold open
    // and empty. Not what any of those parts says: a heading's name, range and
    // share change under the same key, and they are written every draw by
    // `drawHeads` rather than keyed here.
    const layoutKey = `${at.segs.join(",")}#${heads.map((h) => h.key).join("|")}@${f.addrWidth}`;
    if (layoutKey !== parts.layoutKey) {
      this.layOutRow(row, parts, at, f.addrWidth);
      parts.layoutKey = layoutKey;
      parts.noteKey = "";
      parts.valsKey = VALS_UNKNOWN;
      // Cells that changed line have to be told which byte they draw again.
      parts.start = -1;
    }
    this.drawHeads(parts, at, len * 8);
    let height = f.rowHeight * at.segs.length;
    for (const h of at.headHeights) height += h;
    const addr = (parts.lines[0] as LineParts).addr;
    setText(addr, rowStart.toString(16).padStart(f.addrWidth, "0"));
    // Which bytes a row stands for only changes when the view moves. A
    // cursor key leaves every address where it was, and writing them all
    // again would be the largest part of the redraw it causes.
    const moved = parts.start !== rowStart;
    parts.start = rowStart;
    this.drawCells(parts, rowStart, moved, f);
    if (f.fields) height += this.drawNotes(r, parts, at, f);
    return height;
  }

  /** Write one row's bytes, and their text, into the cells they are drawn
   *  in. */
  private drawCells(parts: RowParts, rowStart: number, moved: boolean, f: Frame): void {
    const { bpr, len, start, binary, complete, bytes, fields, selection, windowBytes, spans, byteSpan } = f;
    for (let i = 0; i < bpr; i++) {
      const off = rowStart + i;
      const h = parts.hexCells[i] as HTMLElement;
      const a = parts.textCells[i] as HTMLElement;
      // What each cell is, gathered as strings by `cellDraw` and written
      // only where it is not what the cell already says. Most of a redraw
      // changes nothing — a cursor key moves a mark two cells — and a class
      // written back unchanged still costs the browser the styling of that
      // cell.
      if (binary && h.firstChild !== null) h.textContent = "";
      if (moved) {
        // `setAttribute` rather than `dataset`: they write the same
        // attribute and read back the same way, but the property setter
        // goes through a proxy per write.
        const at = String(off);
        h.setAttribute("data-off", at);
        a.setAttribute("data-off", at);
      }
      // A user-selected range temporarily replaces the active-field mark.
      // Keeping both over the same bytes made adjacent or overlapping state
      // impossible to parse; clearing the selection reveals the field again.
      const hl = selection === null ? highlightBits(f.highlight, off) : [];
      const sb = selection === null ? null : selectionBits(selection, off);
      const si = fields && off >= start && off < start + windowBytes ? byteSpan[off - start] ?? -1 : -1;
      const s = si >= 0 ? spans[si] : undefined;
      const draw = cellDraw({
        off,
        len,
        binary,
        complete,
        byte: bytes[off - start] ?? 0,
        span: s === undefined || s.gap ? null : { kind: s.kind, startsHere: off === Math.floor(s.offset_bits / 8) },
        hl,
        sel: sb,
        link: f.linked,
        cursor: f.cursor,
        glyphs: f.glyphs,
        pane: f.pane,
        nibble: f.nibble,
        insertMode: f.insertMode,
      });
      if (h.style.backgroundImage !== draw.bits) h.style.backgroundImage = draw.bits;
      setText(a, draw.asciiText);
      if (draw.hexText !== null) setText(h, draw.hexText);
      // The bits inside a cell carry their own marks, so in binary the cell
      // has only what `fillBits` puts on it.
      if (h.className !== draw.hex) h.className = draw.hex;
      if (binary && off < len) this.fillBits(f, h, complete ? bytes[off - start] ?? 0 : null, off, hl, sb);
      if (a.className !== draw.ascii) a.className = draw.ascii;
    }
  }

  /** The record the top row starts in, for the strip pinned over the rows.
   *
   *  Records only. A run of records draws a ditto wherever one reads as the
   *  one before it, so the top of a screenful taken from the middle of such a
   *  run says nothing at all without this; a run of numbers has its numbers on
   *  every row and needs no help. */
  private topReading(vals: RowValues): Reading | null {
    const cell = vals.cells.find((c) => c.kind === "composite");
    if (cell === undefined) return null;
    return { path: cell.path.join(","), index: cell.index, text: cell.tip };
  }

  /** Put a row's chips in the blocks beside or below its bytes, and say what
   *  they add to its height. What the top row carries goes on the frame, for
   *  the strip pinned over the rows. */
  private drawNotes(
    r: number,
    parts: RowParts,
    at: RowPieces,
    f: Frame,
  ): number {
    const { rowStart, segs } = at;
    const firstNote = (parts.lines[0] as LineParts).note;
    if (!f.templated || f.trouble !== null) {
      const key = `!${r === 0 ? (f.trouble ?? NO_TEMPLATE) : ""}`;
      if (key !== parts.noteKey) {
        parts.noteKey = key;
        parts.valsKey = VALS_UNKNOWN;
        for (let j = 1; j < segs.length; j++) {
          const note = (parts.lines[j] as LineParts).note;
          for (const c of chipsOf(note)) c.remove();
        }
        const say = r === 0 ? (f.trouble ?? NO_TEMPLATE) : null;
        // The table of values goes with the fields it belongs to, and is left
        // in place rather than taken away: a finger may be on it.
        const vals = valsOf(firstNote);
        if (vals !== null) fillVals(vals, NO_VALUES, 0, f.bpr);
        const chips = chipsOf(firstNote);
        for (const c of chips.slice(say === null ? 0 : 1)) c.remove();
        if (say !== null) {
          const chip = chips[0] ?? (firstNote.insertBefore(newChip(this.picks.field), vals) as ChipEl);
          fillPlain(chip, "hv-chip-wide", say, f.trouble ?? "");
        }
      }
      return 0;
    }
    // What each block of chips will say, worked out before any of it is
    // written, and where each goes on a row a heading has cut. See
    // `chipplan.ts`, which holds the reasoning and the arithmetic.
    const vals = f.values[r] ?? NO_VALUES;
    const planned = planRowChips({
      chips: f.byRow[r] ?? [],
      segs,
      rowStart,
      top: r === 0,
      noteWidth: f.noteWidth,
      maxLines: f.maxLines,
      measure: f.chipMeasure,
      below: f.below,
      rowHeight: f.rowHeight,
      chipLine: f.sizes.chipLine,
      // How far the top row is scrolled up, and what stands in the way, so
      // that the strip pinned over the rows names only the fields that reach
      // a byte still on screen. Every other row sits square against nothing.
      topPx: r === 0 ? f.topPx : 0,
      headHeights: at.headHeights,
      valsHeight: vals.height,
      reading: r === 0 ? this.topReading(vals) : null,
    });
    if (planned.pinned !== null) this.carried = planned.pinned;
    const trailer = f.more && r === this.win.length - 1;
    const key = rowNoteKey(planned.blocks, trailer);
    // The table goes in the first line's block: a heading may cut the row, but
    // the table spans the row's whole width and belongs to all of it.
    let block = valsOf(firstNote);
    const valsChanged = vals.key !== parts.valsKey;
    if (vals.lines > 0 || block !== null) {
      if (block === null) {
        block = newVals(this.picks.value);
        firstNote.append(block);
      }
      if (vals.key !== parts.valsKey) {
        parts.valsKey = vals.key;
        fillVals(block, vals, f.valsWidth, f.bpr);
        // The condensed cap counts chip lines; the table is not one of them,
        // and without this the block is cut off at three lines of chips.
        const room = `${vals.height}px`;
        if (firstNote.style.getPropertyValue("--hv-vals-h") !== room) firstNote.style.setProperty("--hv-vals-h", room);
        // A block below or above the bytes takes no room when it is empty, and
        // a table is something in it whether or not a chip is.
        if (vals.lines > 0) firstNote.classList.remove("hv-empty");
        else if (chipsOf(firstNote).every((c) => c.hidden)) firstNote.classList.add("hv-empty");
      }
      // Which cell the cursor is in is not part of what the row says, so it is
      // marked on its own: a cursor key moves the mark two cells rather than
      // rewriting every value on screen.
      markVals(block, vals, f.cursorBit);
    }
    const chipsChanged = key !== parts.noteKey;
    if (chipsChanged) {
      parts.noteKey = key;
      for (const [j, b] of planned.blocks.entries()) {
        fillNote((parts.lines[j] as LineParts).note, b, false, trailer && j === segs.length - 1, this.picks.field);
      }
    }
    if (block !== null && (chipsChanged || valsChanged)) orderVals(block, planned.blocks[0], vals);
    // The chips and the table share a block, and beside the bytes the first
    // line of it is the row's own height. Which is why the chips' own
    // `extraHeight` is not what is returned: the table is a further line under
    // them, and the two together are what the row has to hold.
    let extra = 0;
    for (const [j, h] of planned.chipHeights.entries()) {
      const total = h + (j === 0 ? vals.height : 0);
      extra += f.below ? total : Math.max(0, total - f.rowHeight);
    }
    return extra;
  }

  /** The strip over the top edge, filled in place: it is inside `.hv-rows`,
   *  which is where a touch drag is captured, so it is emptied and hidden
   *  rather than taken away. */
  private drawPinned(): void {
    const pinnedKey = pinnedNoteKey(this.carried);
    if (pinnedKey !== this.pinnedKey) {
      this.pinnedKey = pinnedKey;
      fillNote(this.pinned, this.carried, true, false, this.picks.field);
    }
  }
}

/** The entries on the rows, in order, without building a list of them first:
 *  the heading over the column is worked out from these on every draw, and a
 *  draw is once a frame while the view is scrolling. */
function* chipSpans(byRow: readonly Chip[][]): Generator<Span> {
  for (const row of byRow) for (const chip of row) yield chip.span;
}

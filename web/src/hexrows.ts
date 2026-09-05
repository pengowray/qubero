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
// The frame's type is imported back from `hexview.ts`. Only the type: the
// class here is what that file imports, so at run time the two go one way.

import type { OutlineHeading } from "./outline.js";
import type { Frame } from "./hexview.js";
import { NO_TEMPLATE } from "./strings.js";
import type { ChipMeasure } from "./chipfit.js";
import { pinnedNoteKey, planRowChips, rowNoteKey, type ChipBlock } from "./chipplan.js";
import { cellDraw, covers, HEX, highlightBits, selectionBits, setText, type Run } from "./hexcell.js";
import { chipsOf, fillNote, fillPlain, newChip, readChipFonts, valsOf, type ChipEl } from "./hexchips.js";
import { fillHeadings, rowPieces, type RowPieces } from "./hexheadings.js";
import { fillVals, markVals, newVals, readValFont } from "./valuecells.js";
import { NO_VALUES } from "./valuetable.js";

/** What pressing something in a row does. Held as one object for the life of
 *  the view: every chip and every heading keeps the function it was built
 *  with, so a chip filled again is not a chip built again. */
export type RowPicks = {
  readonly field: (path: readonly number[]) => void;
  readonly value: (path: readonly number[], bit: number) => void;
  readonly heading: (h: OutlineHeading) => void;
};

/** What a row's table key is set to when what the block holds is no longer
 *  known: a row past the end of the file, or one whose lines were laid out
 *  again. Not the empty string, which is the key of a row with no table, so
 *  that a table left in the block by the row before is written over rather
 *  than taken for the empty block the new row wants. */
const VALS_UNKNOWN = "\u0000";

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
  private rowEls: HTMLElement[] = [];
  /** The elements each row is made of, kept between draws. See `fitParts`. */
  private parts: RowParts[] = [];
  private partsShape = "";
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

  /** The row elements, for reading a point on screen back to a byte. */
  get rows(): readonly HTMLElement[] {
    return this.rowEls;
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
    this.carried = null;
    const heights: number[] = [];
    for (let r = 0; r < this.rowEls.length; r++) {
      const h = this.drawRow(r, f);
      if (h !== null) heights.push(h);
    }
    this.drawPinned();
    return heights;
  }

  /**
   * How tall each row really came out, against what `write` predicted.
   *
   * One forced layout for the lot. The prediction decides how many lines of
   * chips a row holds and what it counts as left over; what the view scrolls
   * by has to be what the browser drew, or a row taller than it was reckoned
   * to be spills over the one below it.
   */
  heights(predicted: readonly number[]): number[] {
    return predicted.map((h, i) => (h === 0 ? 0 : (this.rowEls[i]?.offsetHeight ?? h)));
  }

  /** The fonts a chip's name and value are drawn in, and the one a value cell
   *  is, read off elements that have been drawn. Null until there is one. */
  fonts(): { chip: ChipMeasure | null; value: ChipMeasure | null } {
    return { chip: readChipFonts(this.inner), value: readValFont(this.inner) };
  }

  /** The side column's width and where it starts, read off the first row.
   *  Null when there is no column to read. */
  noteMetrics(): NoteMetrics | null {
    const noteEl = this.rowEls[0]?.querySelector(".hv-note") as HTMLElement | null;
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
    const cell = this.rowEls[0]?.querySelector(binary ? ".hv-bits > span" : ".hv-hex > span");
    return cell instanceof HTMLElement ? cell.getBoundingClientRect().width : 0;
  }

  /** The cell one byte of a row on screen is drawn in, by the row's place in
   *  the pool and the byte's place in the row. A heading may have cut the row,
   *  so which line the cell is on is the pool's business, not the caller's. */
  cellFor(row: number, at: number): HTMLElement | undefined {
    return this.parts[row]?.hexCells[at];
  }

  /** How wide a whole row is, which is what a note below the bytes gets. */
  rowWidth(): number {
    return this.rowEls[0]?.clientWidth ?? 0;
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
  private layOutRow(row: HTMLElement, parts: RowParts, at: RowPieces, fileBits: number, addrWidth: number): void {
    const { rowStart, segs } = at;
    const { bpr, binary, fields, below } = this.lineShape;
    while (parts.lines.length < segs.length) parts.lines.push(this.makeLine());
    // Every line the row has ever needed, in order, whether or not this
    // drawing uses it. A row that stops being cut hides its second line rather
    // than dropping it, so the list of things in the row only ever grows and a
    // scroll never takes an element out from under a finger.
    const kids: HTMLElement[] = [];
    for (const [j, lp] of parts.lines.entries()) {
      const on = j < segs.length;
      const pos = on ? (segs[j] as number) : 0;
      // Always in place, empty when no part starts here, so that a heading
      // arriving or leaving writes into a block that is already there.
      fillHeadings(lp.head, on ? (at.heads[j] ?? []) : [], fileBits, rowStart + pos, this.picks.heading);
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

  ensure(want: number): void {
    while (this.rowEls.length < want) {
      const r = document.createElement("div");
      r.className = "hv-row";
      r.setAttribute("role", "row");
      this.inner.append(r);
      this.rowEls.push(r);
    }
    while (this.rowEls.length > want) {
      this.rowEls.pop()?.remove();
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
    this.partsShape = shape;
    this.lineShape = { bpr, binary, showText, fields, below };
    this.parts = this.rowEls.map((row) => {
      const first = this.makeLine();
      row.replaceChildren(first.line);
      return {
        lines: [first],
        hexCells: [...first.hex],
        textCells: [...first.text],
        start: -1,
        blank: false,
        layoutKey: "",
        noteKey: "",
        valsKey: "",
      };
    });
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
      title.textContent = "Fields";
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
    const row = this.rowEls[r];
    const parts = this.parts[r];
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
    // The share of the file changes with its length, so the key does too.
    const layoutKey = `${at.segs.join(",")}#${heads.map((h) => h.key).join("|")}@${len}`;
    if (layoutKey !== parts.layoutKey) {
      this.layOutRow(row, parts, at, len * 8, f.addrWidth);
      parts.layoutKey = layoutKey;
      parts.noteKey = "";
      parts.valsKey = VALS_UNKNOWN;
      // Cells that changed line have to be told which byte they draw again.
      parts.start = -1;
    }
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
    });
    if (planned.pinned !== null) this.carried = planned.pinned;
    const trailer = f.more && r === this.rowEls.length - 1;
    const key = rowNoteKey(planned.blocks, trailer);
    // The table goes in the first line's block: a heading may cut the row, but
    // the table spans the row's whole width and belongs to all of it.
    let block = valsOf(firstNote);
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
    if (key !== parts.noteKey) {
      parts.noteKey = key;
      for (const [j, b] of planned.blocks.entries()) {
        fillNote((parts.lines[j] as LineParts).note, b, false, trailer && j === segs.length - 1, this.picks.field);
      }
    }
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

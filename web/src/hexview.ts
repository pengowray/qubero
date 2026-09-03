// Virtualised address + hex + ascii view.
//
// The browser cannot host a scroll container billions of pixels tall, so this view
// does not use native scrolling for the document. It keeps a `topRow` and renders
// only the rows that fit, with its own scrollbar mapped row <-> file offset.

import type { Doc, Span } from "./doc.js";
import { formatBytes } from "./doc.js";
import type { OutlineHeading, Viewport } from "./outline.js";
import { GAP_LABEL, NO_TEMPLATE, REPORT } from "./strings.js";
import { fieldClass } from "./fieldstyle.js";
import { CHIP_LINES, chipDetail, chipLayout, chipWidth, GUESS_TEXT, runDetail, type ChipMeasure } from "./chipfit.js";
import { rangeText, shareText } from "./listingdraw.js";

export type Pane = "hex" | "ascii";
/**
 * What sits to the right of the bytes: the text, the fields, or both. The
 * `-condensed` readings cap a row's chips at three lines and count the rest;
 * without it a row is as tall as its chips need.
 */
export type RightColumn = "text" | "fields" | "fields-condensed" | "both" | "both-condensed";
/** Every reading of the column setting, in the order they are offered. */
export const RIGHT_COLUMNS: readonly RightColumn[] = ["text", "fields", "fields-condensed", "both", "both-condensed"];
/** Whether a saved or typed value is one of them. */
export function isRightColumn(v: string | null): v is RightColumn {
  return v !== null && (RIGHT_COLUMNS as readonly string[]).includes(v);
}
/** A run of bits, `[startBit, endBit)`. A run of no bits is a place rather than
 *  a stretch, which is what a field of no length has. */
export type BitRange = { readonly startBit: number; readonly endBit: number };

/** The part of one byte a run covers, as bit positions 0 to 8 counting from the
 *  top of the byte. */
type Run = { from: number; to: number };

/** Whether the runs together cover every bit from `from` to `to`. */
function covers(runs: readonly Run[], from: number, to: number): boolean {
  let at = from;
  for (const r of runs) {
    if (r.from > at) return false;
    at = Math.max(at, r.to);
  }
  return at >= to;
}

/** Hex shows two digits per byte; binary shows the eight bits. */
export type ViewMode = "hex" | "binary";

export type CursorState = {
  readonly offset: number;
  /** Absolute bit position. In hex mode this is always `offset * 8`. */
  readonly bitOffset: number;
  readonly pane: Pane;
  readonly insertMode: boolean;
  readonly mode: ViewMode;
};

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

/** How many entries one screenful of the annotation column may hold. */
const SPAN_LIMIT = 600;
/** How many bytes one copy may carry. A selection can be the whole file, and
 *  the bytes have to be fetched and turned into a string before the clipboard
 *  sees them, so past some size the honest answer is no. */
const COPY_LIMIT_BYTES = 256 * 1024;

/** How long a message stays up before it goes away on its own. */
const NOTICE_MS = 5000;

/** What is left of a throw's speed after a millisecond: about half of it every
 *  350ms, so a hard flick covers tens of rows and a gentle one a handful. */
const GLIDE_DECAY = 0.998;
/** Rows per millisecond below which a throw is over. One row every four
 *  seconds is not scrolling. */
const GLIDE_STOP = 0.004;

const COPY_TOO_BIG = (bytes: number): string =>
  `Selection too large to copy: ${Math.round(bytes / 1024).toLocaleString()} KB, limit ${COPY_LIMIT_BYTES / 1024} KB.`;
const COPY_PENDING = "Selection is still loading. Try again in a moment.";
const COPY_FAILED = "Couldn't copy to the clipboard.";
const COPY_DONE = (bytes: number, asText: boolean): string =>
  `Copied ${bytes.toLocaleString()} ${bytes === 1 ? "byte" : "bytes"} as ${asText ? "text" : "hex"}.`;

/** What a cursor move does to the selection. An anchor extends from that bit,
 *  which is the one thing a plain "keep" cannot say. */
type Select = "keep" | "clear" | { readonly anchor: number };

/** A span named on a row, whether it started above the view, and the elements
 *  of its list it stands for when a run of them is drawn as one chip: empty
 *  for a chip that is one field. */
type Chip = { span: Span; carried: boolean; run: Span[] };

/** What a chip says: the name in bold and the value after it. */
type ChipText = { readonly name: string; readonly detail: string };

/** The name as it is actually drawn. A chip for a field that began above the
 *  view is marked with an arrow by `.hv-chip-carried`, which is a CSS
 *  `::before` and so invisible to measuring the name's own text. Without it
 *  every carried chip is measured a character and a half short. */
function carriedName(name: string, c: Chip | undefined): string {
  return c?.carried === true ? `↑ ${name}` : name;
}

/** The name a list gives its elements. */
const ELEMENT = /^\[\d+\]$/;

/** Whether a span is an element of a list that reads as one of many, so that
 *  a run of its siblings on one row can be one chip. Text is not: each string
 *  is worth reading. Nor is a structure that reads on one line, for the same
 *  reason, or a run the core has already folded. */
function foldable(s: Span): boolean {
  return !s.gap && s.count === 0 && s.line === null && s.kind !== "str" && ELEMENT.test(s.name);
}

/** Whether two spans are elements of the same list, read the same way. */
function sameList(a: Span, b: Span): boolean {
  return a.type === b.type && a.trail.length === b.trail.length && a.trail.every((t, i) => t === b.trail[i]);
}

/** The list an element belongs to, by name. */
function listName(s: Span): string {
  return s.trail[s.trail.length - 1] ?? s.name;
}

/** What a heading calls a part with no name of its own: the listing's word
 *  for a run of fields at the front, the back or the middle of the file. */
function headingName(h: OutlineHeading, fileBits: number): string {
  if (h.name !== "") return h.name;
  const where = h.offsetBits === 0 ? "start" : fileBits > 0 && h.offsetBits + h.sizeBits >= fileBits ? "end" : "middle";
  return REPORT.unnamedPart(where);
}

/** What pressing a heading does, for a reader who cannot guess. */
const HEADING_TIP = (name: string): string => `Move the cursor to the first byte of ${name}`;

/** Where the spans on screen land. See `HexView.placeSpans`. */
type Placed = {
  spans: Span[];
  more: boolean;
  trouble: string | null;
  byteSpan: Int32Array;
  byRow: Chip[][];
};

/** Nothing to place: no template, or the column is turned off. */
const NO_SPANS = (windowBytes: number): Placed => ({
  spans: [],
  more: false,
  trouble: null,
  byteSpan: new Int32Array(windowBytes).fill(-1),
  byRow: [],
});

/** Write a cell's characters, unless they are already the ones it shows.
 *  Scrolling changes every one of them and a cursor key changes none, and the
 *  browser charges for a write either way. */
function setText(el: HTMLElement, text: string): void {
  if (el.textContent !== text) el.textContent = text;
}

function asciiGlyph(b: number): string {
  return b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "·";
}

/** One line of cells: an address, the bytes, their text and their fields. A row
 *  is one of these unless a part starts part-way along it. */
type LineParts = {
  readonly line: HTMLElement;
  readonly addr: HTMLElement;
  readonly cells: HTMLElement;
  readonly asc: HTMLElement;
  readonly note: HTMLElement;
  readonly hex: readonly HTMLElement[];
  readonly text: readonly HTMLElement[];
};

export class HexView {
  readonly el: HTMLElement;
  private readonly header: HTMLElement;
  private readonly rowsEl: HTMLElement;
  private readonly track: HTMLElement;
  private readonly thumb: HTMLElement;
  private rowEls: HTMLElement[] = [];
  /** The spans each row is made of, kept between draws. See `fitParts`. */
  private parts: {
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
  }[] = [];
  private partsShape = "";
  /** What `fitParts` last built the lines for, so a line added mid-draw for a
   *  row that had to be cut is built the same way. */
  private lineShape = { bpr: 16, binary: false, showText: true, fields: false, below: false };

  private topRow = 0;
  private visibleRows = 1;
  private rowHeight = 20;
  private bytesPerRow = 16;
  private cursor = 0;
  private nibble: 0 | 1 = 0;
  /** Bit within the cursor byte, 0 = most significant. Set by a bit move or by
   * picking a field that starts inside a byte; any byte-level move clears it. */
  private bit = 0;
  private mode: ViewMode = "hex";
  private pane: Pane = "hex";
  private insertMode = false;
  /**
   * A touch drag that is scrolling. Enough of the recent movement is kept to
   * throw the view when the finger lifts.
   */
  private dragging: { startY: number; startRow: number; lastY: number; lastT: number; velocity: number } | null = null;
  /** The frame request of a throw still slowing down, if one is running. */
  private glide: number | null = null;
  /**
   * A mouse drag that is selecting rather than scrolling. The pane is fixed at
   * the button press: a drag that wandered from the bytes into the text column
   * would be selecting two different things at once.
   */
  private selDrag: { pane: Pane; anchor: number; unit: number; x: number; y: number; raf: number | null } | null = null;
  /**
   * The two ends of the selection, in absolute bits. A null anchor means
   * nothing is selected, and so does an anchor equal to the focus.
   *
   * Bits rather than bytes because everything else here is bits: a selection
   * dragged in hex or text snaps to whole bytes, one dragged over binary bits
   * does not, and both are the same value.
   */
  private selAnchor: number | null = null;
  private selFocus = 0;
  /**
   * The bit runs to highlight: usually one, the field the cursor is in, but a
   * value the format does not keep in one piece takes more than one. A five-bit
   * ggml weight is four bits of `qs` and one bit of `qh` sixteen bytes away,
   * and marking only the four would be marking the wrong thing.
   */
  private highlight: readonly BitRange[] = [];
  /**
   * The stretch this view's bytes are linked to another tab's by: the bits a
   * byte of an unpacked stream came from, or the bytes a compressed field
   * unpacked to. Drawn as an outline rather than as a fill, because it is
   * neither the cursor nor the selection: it says where the *other* tab is
   * looking, and clicking it goes there.
   */
  private linked: BitRange | null = null;
  private rightColumn: RightColumn = "text";
  /** Spans for the rows on screen, kept until the view or the file moves. */
  private spanCache: { key: string; spans: Span[]; more: boolean; error: string | null } | null = null;
  /** Width of the annotation column, measured from the last frame. */
  private noteWidth = 0;
  /**
   * Where the chips are drawn. Beside the bytes while there is room for them;
   * below the bytes, across the whole row, when a wide row has squeezed the
   * side column down to nothing.
   *
   * Three states rather than a flag, because the measurement that decides it
   * is the one the decision changes: chips below are as wide as the row, so
   * measuring them again would always say "there is room" and send them back.
   * Only `unknown` decides; `remeasure` puts it back there.
   */
  private arrangement: "unknown" | "side" | "below" = "unknown";
  /** Below which side column the chips go under the bytes instead. A column
   *  narrower than this holds one short chip and cuts every other one off. */
  private static readonly NOTE_MIN = 120;
  /** How a chip's two runs of text are measured, read from a drawn chip's own
   *  fonts. Null until there has been a chip on screen to read. */
  private chipFonts: ChipMeasure | null = null;
  /** The two sizes only the browser can answer: how wide the field column is,
   *  which decides how many chips fit on a row, and how tall the scrollbar
   *  track is. Both are asked for at the end of a redraw, after it has written
   *  to the document, so each one costs a fresh layout of the whole view. Only
   *  a resize or a change of shape moves them, and `relayout` — which every one
   *  of those goes through — throws them away. */
  private metrics: { readonly noteWidth: number; readonly trackH: number } | null = null;
  /** The third size only the browser can answer: how tall a row is and how
   *  many of them fit. Measuring it reads `clientHeight` and a computed style,
   *  which forces a layout of everything the draw just wrote, so it is held
   *  here and thrown away by `relayout` alongside `metrics`. */
  private fit: { readonly rowHeight: number; readonly visibleRows: number } | null = null;
  /** How tall the space for rows is, from the same measurement. */
  private viewH = 0;
  /** What the stylesheet says a heading line, a smaller one, and one line of
   *  chips are tall, so a row's height can be worked out before it is drawn.
   *  Read with `fit`, since only a change of style moves them. */
  private sizes = { heading: [26, 20] as readonly [number, number], chipLine: 22 };
  /** Rows the view may scroll past the usual last top row. Rows are not all
   *  one height, so a screenful counted in rows can be taller than the screen,
   *  and at the end of the file that would leave the last rows unreachable.
   *  Worked out from the rows' heights whenever the end is drawn. */
  private endSlack = 0;
  /** The last PageDown: the top row it left and the one it landed on, and how
   *  far the cursor went. A PageUp straight after goes back the same way, which
   *  counting a screenful of rows again cannot promise when the rows are not
   *  all one height. */
  private lastPage: { from: number; to: number; rows: number } | null = null;
  /** True while a move is still settling, so `render` puts the work off to the
   *  end of it. Moving the cursor draws the rows, then tells the rest of the
   *  app, which comes straight back with the field to highlight and draws them
   *  a second time. The first drawing is never seen: both happen inside one
   *  event, and the browser paints once at the end of it. */
  private settling = false;

  onCursorChange: (c: CursorState) => void = () => {};
  /** A field picked in the annotation column. */
  onPickField: (path: readonly number[]) => void = () => {};
  /** The selection after it changed, or null when there is none. */
  onSelectionChange: (r: BitRange | null) => void = () => {};
  /** The stretch of the file on screen, after every draw. The rail marks the
   *  part of the file it falls in. */
  onViewport: (v: Viewport) => void = () => {};
  /** The parts of the file, as the listing names them. Drawn as heading lines
   *  before the row each one starts in. */
  sections: readonly OutlineHeading[] = [];

  /** Where a one-off message goes. The grid has no status bar of its own, and
   *  a copy that did not happen has to say so where the user is looking. */
  private readonly notice: HTMLElement;
  private noticeTimer = 0;

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("div");
    this.el.className = "hexview";
    this.el.tabIndex = 0;
    this.el.setAttribute("role", "grid");
    this.el.setAttribute("aria-label", "File contents");

    this.header = document.createElement("div");
    this.header.className = "hv-header";
    this.rowsEl = document.createElement("div");
    this.rowsEl.className = "hv-rows";
    const body = document.createElement("div");
    body.className = "hv-body";
    body.append(this.header, this.rowsEl);

    this.track = document.createElement("div");
    this.track.className = "hv-track";
    this.track.setAttribute("aria-hidden", "true");
    this.thumb = document.createElement("div");
    this.thumb.className = "hv-thumb";
    this.track.append(this.thumb);

    this.notice = document.createElement("div");
    this.notice.className = "hv-notice";
    this.notice.setAttribute("aria-live", "polite");
    this.notice.hidden = true;

    this.el.append(body, this.track, this.notice);

    new ResizeObserver(() => this.relayout()).observe(this.rowsEl);
    this.el.addEventListener("wheel", (e) => this.onWheel(e), { passive: false });
    // The view scrolls itself, so no touch here is ever a page gesture. Saying
    // so with `touch-action` alone is not enough: a drag down means scroll up,
    // and the browser reads a drag down from the top of the page as pull to
    // refresh, which throws the file away mid-scan.
    this.el.addEventListener("touchmove", (e) => e.preventDefault(), { passive: false });
    this.el.addEventListener("keydown", (e) => this.onKey(e));
    this.el.addEventListener("relayout", () => this.relayout());
    this.rowsEl.addEventListener("pointerdown", (e) => this.onPointerDown(e));
    this.rowsEl.addEventListener("pointermove", (e) => this.onPointerMove(e));
    this.rowsEl.addEventListener("pointerup", (e) => this.onPointerUp(e));
    this.rowsEl.addEventListener("pointercancel", (e) => this.onPointerUp(e));
    this.track.addEventListener("pointerdown", (e) => this.onTrackDown(e));
    this.track.addEventListener("pointermove", (e) => this.onTrackMove(e));
    this.track.addEventListener("pointerup", (e) => this.onTrackUp(e));
    this.track.addEventListener("pointercancel", (e) => this.onTrackUp(e));
    doc.onChange(() => {
      this.spanCache = null;
      this.endSlack = 0;
      this.render();
    });
  }

  // ----- geometry -----

  private get totalRows(): number {
    return Math.max(1, Math.ceil((this.doc.lengthBytes + 1) / this.bytesPerRow));
  }
  private get maxTopRow(): number {
    return Math.max(0, this.totalRows - this.visibleRows + this.endSlack);
  }

  /** Take the parts of the file to draw headings for. Headings above the
   *  cursor's row push it down, so it is brought back on screen if that
   *  pushed it off; a cursor that was already off screen is left there. */
  setSections(sections: readonly OutlineHeading[]): void {
    this.sections = sections;
    this.endSlack = 0;
    this.render();
    this.revealCursor();
  }

  setBytesPerRow(n: number): void {
    this.bytesPerRow = n;
    this.remeasure();
    this.showCursor();
  }

  /** Whether the field column is drawn at all. */
  private get showsFields(): boolean {
    return this.rightColumn !== "text";
  }
  /** Whether the bytes' text is drawn. */
  private get showsText(): boolean {
    return this.rightColumn === "text" || this.rightColumn.startsWith("both");
  }
  /** Whether a row's chips are capped and the rest counted. */
  private get isCondensed(): boolean {
    return this.rightColumn.endsWith("-condensed");
  }

  /** Throw away every size the browser answered, and ask again. */
  private remeasure(): void {
    this.metrics = null;
    this.fit = null;
    this.endSlack = 0;
    // Where the chips go is decided from a measurement that has just been
    // thrown away, so it is decided again.
    this.arrangement = "unknown";
    this.fitRows();
  }

  setRightColumn(c: RightColumn): void {
    this.rightColumn = c;
    // The text column is where the "ascii" pane lives; without it the cursor
    // has nowhere to be but the bytes.
    if (!this.showsText && this.pane === "ascii") this.pane = "hex";
    this.spanCache = null;
    // Rows are taller while the field column is shown, so the number of rows
    // that fit has to be worked out again.
    this.el.classList.toggle("has-notes", this.showsFields);
    this.el.classList.toggle("is-condensed", this.isCondensed);
    this.remeasure();
    this.showCursor();
  }

  relayout(): void {
    this.remeasure();
    // The slack past the last screenful was thrown away with the other sizes,
    // so the limit is not known until the end has been drawn again. Clamping
    // first would pull a view sitting at the end back up by that slack, and
    // the next scroll would put it back: a view that jumps while the reader
    // holds the wheel. So draw where it was, then clamp by what was found.
    this.topRow = Math.min(this.topRow, Math.max(0, this.totalRows - 1));
    this.render();
    const limit = this.maxTopRow;
    if (this.topRow > limit) {
      this.topRow = limit;
      this.render();
    }
  }

  /**
   * Match the number of rows to the space there is for them.
   *
   * The row height is the stylesheet's `min-height` for a row rather than a
   * row's measured box: rows grow to hold their chips and headings, and one
   * row measured at three lines tall would leave the view showing a third of
   * the file it could. The count of rows is how many of that height fit; the
   * rows that grow push the last ones off the bottom, where they are clipped.
   *
   * Called on every render, but it only measures once per layout: both reads
   * force the browser to lay the view out again, and a redraw has just written
   * to every row, so the answer costs a full layout every time it is asked for.
   * Nothing a draw does changes it — only a resize, a change of shape, or the
   * first pass where the rows do not exist yet, and all of those come through
   * `relayout`, which drops the cache. A measurement taken before there was a
   * row to measure is not kept, so the real height is picked up on the pass
   * after the rows are made.
   */
  private fitRows(): void {
    if (this.fit !== null) {
      this.ensureRowEls();
      return;
    }
    const probe = this.rowEls[0];
    const h = probe === undefined ? 0 : parseFloat(getComputedStyle(probe).minHeight);
    if (h > 0) this.rowHeight = h;
    const style = getComputedStyle(this.el);
    const px = (name: string, fallback: number): number => {
      const v = parseFloat(style.getPropertyValue(name));
      return v > 0 ? v : fallback;
    };
    this.sizes = { heading: [px("--hv-heading", 26), px("--hv-subheading", 20)], chipLine: px("--hv-chip-line", 22) };
    this.viewH = this.rowsEl.clientHeight;
    const fit = Math.max(1, Math.floor(this.viewH / this.rowHeight));
    if (fit !== this.visibleRows) {
      this.visibleRows = fit;
      // Only as far as the file goes: how far past the last screenful the
      // view may sit is found by drawing the end, which `relayout` clamps by.
      this.topRow = Math.min(this.topRow, Math.max(0, this.totalRows - 1));
    }
    if (probe !== undefined) this.fit = { rowHeight: this.rowHeight, visibleRows: fit };
    // Unconditional: on the first pass there are no row elements yet, however
    // many of them fit.
    this.ensureRowEls();
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
      };
    });
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
    return { line, addr, cells, asc, note, hex, text };
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
  private layOutRow(
    row: HTMLElement,
    parts: HexView["parts"][number],
    rowStart: number,
    segs: readonly number[],
    heads: ReadonlyMap<number, OutlineHeading[]>,
    fileBits: number,
    addrWidth: number,
  ): void {
    const { bpr, binary, fields, below } = this.lineShape;
    while (parts.lines.length < segs.length) parts.lines.push(this.makeLine());
    const kids: HTMLElement[] = [];
    for (const [j, at] of segs.entries()) {
      const here = heads.get(at);
      if (here !== undefined) kids.push(this.headingBlock(here, fileBits, rowStart + at));
      const lp = parts.lines[j] as LineParts;
      kids.push(lp.line);
      if (fields && below) kids.push(lp.note);
    }
    row.replaceChildren(...kids);
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

  private ensureRowEls(): void {
    while (this.rowEls.length < this.visibleRows) {
      const r = document.createElement("div");
      r.className = "hv-row";
      r.setAttribute("role", "row");
      this.rowsEl.append(r);
      this.rowEls.push(r);
    }
    while (this.rowEls.length > this.visibleRows) {
      this.rowEls.pop()?.remove();
    }
  }

  // ----- cursor & scrolling -----

  get cursorState(): CursorState {
    return {
      offset: this.cursor,
      bitOffset: this.cursor * 8 + this.bit,
      pane: this.pane,
      insertMode: this.insertMode,
      mode: this.mode,
    };
  }

  setMode(mode: ViewMode): void {
    this.mode = mode;
    this.nibble = 0;
    // Eight digits a byte instead of two: the bytes take three times the room
    // and what is left for the fields has to be measured again.
    this.remeasure();
    this.showCursor();
    this.onCursorChange(this.cursorState);
  }

  /** The selected bits, or null when nothing is selected. Byte-aligned unless
   *  the selection was made over the bits in binary mode. */
  get selectionRange(): BitRange | null {
    if (this.selAnchor === null || this.selAnchor === this.selFocus) return null;
    return {
      startBit: Math.min(this.selAnchor, this.selFocus),
      endBit: Math.max(this.selAnchor, this.selFocus),
    };
  }

  /**
   * Select a run of bits and put the cursor at its start, for the views that
   * pick out a stretch of the file rather than a place in it. An empty run
   * clears the selection, so one call covers both.
   */
  selectRange(startBit: number, endBit: number, cursorBit?: number): void {
    // The cursor lands at the front of the run unless the caller says
    // otherwise. A selection dragged out in the text view has its caret at the
    // end it is being dragged from, and moving it to the front would collapse
    // the selection the next time it grew.
    this.setBitCursor(cursorBit ?? startBit, { select: "keep" });
    this.setSelection(endBit > startBit ? startBit : null, endBit);
    this.render();
  }

  /** Drop the selection and leave the cursor where it is. */
  clearSelection(): void {
    if (this.selAnchor === null) return;
    this.setSelection(null, 0);
    this.render();
  }

  /** The one place selection state is written, so the callback fires once per
   *  real change rather than once per key handler that touched it. */
  private setSelection(anchor: number | null, focus: number): void {
    const was = this.selectionRange;
    // Clamped here rather than at each caller: a drag into the blank cells
    // right of the last byte, or into a row below the end of a short file,
    // lands past the end, and the core panics on a delete that runs past it.
    const cap = (b: number): number => Math.max(0, Math.min(this.doc.lengthBits, b));
    this.selAnchor = anchor === null ? null : cap(anchor);
    this.selFocus = cap(focus);
    const now = this.selectionRange;
    if (was?.startBit !== now?.startBit || was?.endBit !== now?.endBit) this.onSelectionChange(now);
  }

  /** Called with the cursor already moved, so an anchor extends to where it
   *  now is. */
  private applySelect(s: Select | undefined): void {
    if (s === "keep") return;
    if (s === undefined || s === "clear") return void this.setSelection(null, 0);
    this.setSelection(s.anchor, this.cursorState.bitOffset);
  }

  /**
   * Finish a cursor move: bring it on screen, tell the rest of the app, and
   * draw the rows once at the end.
   *
   * Telling the app comes first because it answers. The field the cursor
   * landed in comes back as a highlight, panels ask to be laid out again, and
   * every one of those asks for a redraw; drawing before the answers arrive
   * draws a screenful nobody sees, since the browser paints once when the
   * event is over either way.
   */
  private settle(): void {
    this.scrollCursorIntoView();
    this.settling = true;
    try {
      this.onCursorChange(this.cursorState);
    } finally {
      this.settling = false;
    }
    this.render();
    this.revealCursor();
  }

  /** Draw with the cursor on screen, for a change that moved nothing but may
   *  have changed what fits. */
  private showCursor(): void {
    this.scrollCursorIntoView();
    this.render();
    this.revealCursor();
  }

  /**
   * Bring the cursor's cell onto the screen by where it was drawn, not by its
   * row number. Rows are not all one height, so a row inside the count that
   * fits can still sit below the bottom edge. The view moves down by the rows
   * the shortfall is worth and looks again: the row that becomes the top one
   * takes on the chips carried from above it, and can grow.
   *
   * Not during a drag that is selecting: that scrolls at its own pace, and a
   * second hand on the view would double it.
   */
  private revealCursor(): void {
    if (this.selDrag !== null) return;
    const bpr = this.bytesPerRow;
    const cursorRow = Math.floor(this.cursor / bpr);
    for (let pass = 0; pass < 6; pass++) {
      const cell = this.parts[cursorRow - this.topRow]?.hexCells[this.cursor - cursorRow * bpr];
      if (cursorRow < this.topRow || cell === undefined) return;
      const deficit = cell.getBoundingClientRect().bottom - this.rowsEl.getBoundingClientRect().bottom;
      if (deficit <= 0.5) return;
      const next = Math.min(cursorRow, this.maxTopRow, this.topRow + Math.max(1, Math.ceil(deficit / this.rowHeight)));
      if (next === this.topRow) return;
      this.topRow = next;
      this.render();
    }
  }

  /** How many rows are wholly on screen, measured. At least one. */
  private rowsInView(): number {
    const bottom = this.rowsEl.getBoundingClientRect().bottom;
    let n = 0;
    for (const row of this.rowEls) {
      if (row.getBoundingClientRect().bottom > bottom + 0.5) break;
      n++;
    }
    return Math.max(1, n);
  }

  /** Scroll down a screenful. Returns how many rows the cursor should move. */
  private pageDown(): number {
    const rows = this.rowsInView();
    const from = this.topRow;
    this.topRow = Math.min(this.maxTopRow, from + rows);
    this.lastPage = { from, to: this.topRow, rows };
    return rows;
  }

  /**
   * Scroll up a screenful: the row above the old top row becomes the bottom
   * one. Returns how many rows the cursor should move, which is how far the
   * view went, so the cursor keeps its place on screen.
   */
  private pageUp(): number {
    const from = this.topRow;
    const back = this.lastPage;
    this.lastPage = null;
    if (back !== null && back.to === from) {
      this.topRow = back.from;
      return back.rows;
    }
    if (from === 0) return this.rowsInView();
    const target = from - 1;
    this.topRow = Math.max(0, target - this.visibleRows + 1);
    this.render();
    // The rows above the old top were not on screen, so how many of them fit
    // is found by drawing them and looking, the same way the cursor is.
    for (let pass = 0; pass < 6; pass++) {
      const row = this.rowEls[target - this.topRow];
      if (row === undefined) break;
      const deficit = row.getBoundingClientRect().bottom - this.rowsEl.getBoundingClientRect().bottom;
      if (deficit <= 0.5) break;
      const next = Math.min(target, this.topRow + Math.max(1, Math.ceil(deficit / this.rowHeight)));
      if (next === this.topRow) break;
      this.topRow = next;
      this.render();
    }
    return from - this.topRow;
  }

  /** Move the cursor to an absolute bit. Bit 0 is the top bit of byte 0. */
  setBitCursor(bitOffset: number, opts: { pane?: Pane; select?: Select } = {}): void {
    if (opts.pane) this.pane = opts.pane;
    const at = Math.max(0, Math.min(this.doc.lengthBits, Math.floor(bitOffset)));
    this.cursor = Math.floor(at / 8);
    this.bit = at % 8;
    this.nibble = 0;
    this.applySelect(opts.select);
    this.settle();
  }

  setCursor(offset: number, opts: { pane?: Pane; nibble?: 0 | 1; bit?: number; select?: Select } = {}): void {
    const max = this.doc.lengthBytes; // one past the end is a valid insert position
    this.cursor = Math.max(0, Math.min(max, Math.floor(offset)));
    this.nibble = opts.nibble ?? 0;
    this.bit = Math.max(0, Math.min(7, opts.bit ?? 0));
    if (opts.pane) this.pane = opts.pane;
    this.applySelect(opts.select);
    this.settle();
  }

  /** A message about something the user just asked for, which goes away on its
   *  own. */
  private say(text: string): void {
    this.notice.textContent = text;
    this.notice.hidden = false;
    clearTimeout(this.noticeTimer);
    this.noticeTimer = window.setTimeout(() => {
      this.notice.hidden = true;
    }, NOTICE_MS);
  }

  /** Called when the user dismisses the field highlight with Escape. */
  onHighlightClear: () => void = () => {};

  /** One run, several, or nothing. Runs may be in any order and need not
   *  touch. */
  /**
   * Mark the stretch that answers the other tab's cursor, or clear it.
   *
   * Only one, because there is one cursor over there. `onLinkedPick` fires when
   * the reader clicks it.
   */
  setLinkedRange(startBit: number | null, endBit = 0): void {
    const next = startBit === null ? null : { startBit, endBit };
    const same =
      (next === null && this.linked === null) ||
      (next !== null &&
        this.linked !== null &&
        next.startBit === this.linked.startBit &&
        next.endBit === this.linked.endBit);
    if (same) return;
    this.linked = next;
    this.render();
  }

  /** The stretch the other tab is looking at, or null. */
  get linkedRange(): BitRange | null {
    return this.linked;
  }

  /** The linked mark was clicked: the reader is asking to follow it. */
  onLinkedPick: (startBit: number, endBit: number) => void = () => {};

  setHighlight(range: BitRange | readonly BitRange[] | null): void {
    this.highlight = range === null ? [] : Array.isArray(range) ? range : [range as BitRange];
    this.render();
  }

  scrollTo(row: number): void {
    const want = Math.max(0, Math.floor(row));
    // Drawing the end of the file is what finds out how much further than a
    // screenful of rows the view has to go for the last row to be whole, so
    // a scroll that asked for more than it was given asks again.
    for (let pass = 0; pass < 3; pass++) {
      const next = Math.min(this.maxTopRow, want);
      if (pass > 0 && next === this.topRow) break;
      this.topRow = next;
      this.render();
    }
  }

  private scrollCursorIntoView(): void {
    const row = Math.floor(this.cursor / this.bytesPerRow);
    if (row < this.topRow) this.topRow = row;
    else if (row >= this.topRow + this.visibleRows) this.topRow = row - this.visibleRows + 1;
    this.topRow = Math.max(0, Math.min(this.maxTopRow, this.topRow));
  }

  private onWheel(e: WheelEvent): void {
    e.preventDefault();
    this.stopGlide();
    const rows = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? e.deltaY : e.deltaY / this.rowHeight;
    this.scrollTo(this.topRow + (rows > 0 ? Math.max(1, Math.round(rows)) : Math.min(-1, Math.round(rows))));
  }

  private onPointerDown(e: PointerEvent): void {
    this.el.focus();
    this.stopGlide();
    if (e.pointerType === "touch") {
      this.dragging = { startY: e.clientY, startRow: this.topRow, lastY: e.clientY, lastT: e.timeStamp, velocity: 0 };
      this.rowsEl.setPointerCapture(e.pointerId);
      return;
    }
    if (e.button !== 0) return;
    const hit = this.hitAt(e.clientX, e.clientY);
    if (hit === null) return;
    e.preventDefault();
    // A click on the mark the other tab put here follows it there, rather than
    // moving this view's cursor to a byte the reader was only pointing at.
    // By the byte, the same rule the mark is drawn by: a step may start and end
    // inside a byte, and a reader clicking a byte that is outlined means that
    // byte, not the particular bit of it their pointer landed on.
    const link = this.linked;
    const clicked = Math.floor(hit.bit / 8);
    if (link !== null && !e.shiftKey && clicked * 8 < link.endBit && (clicked + 1) * 8 > link.startBit) {
      this.onLinkedPick(link.startBit, link.endBit);
      return;
    }
    if (e.shiftKey) {
      // Shift+click extends the selection there is, or starts one from where
      // the cursor already is.
      const anchor = this.selAnchor ?? this.cursorState.bitOffset;
      this.setSelection(anchor, hit.bit >= anchor ? hit.bit + hit.unit : hit.bit);
      this.setCursor(Math.floor(hit.bit / 8), { pane: hit.pane, bit: hit.bit % 8, select: "keep" });
      return;
    }
    // No drag follows most presses, and a press that stays put is a click,
    // which clears the selection. That is what `setCursor` does by default.
    this.setCursor(Math.floor(hit.bit / 8), { pane: hit.pane, bit: hit.bit % 8 });
    this.selDrag = { pane: hit.pane, anchor: hit.bit, unit: hit.unit, x: e.clientX, y: e.clientY, raf: null };
    this.rowsEl.setPointerCapture(e.pointerId);
  }

  private onPointerMove(e: PointerEvent): void {
    if (this.selDrag !== null) {
      this.selDrag.x = e.clientX;
      this.selDrag.y = e.clientY;
      this.dragExtend();
      return;
    }
    if (!this.dragging) return;
    const dt = e.timeStamp - this.dragging.lastT;
    if (dt > 0) {
      // Smoothed, because the last sample before a finger lifts is often a
      // stumble, and on its own it would decide the whole throw.
      const v = (this.dragging.lastY - e.clientY) / this.rowHeight / dt;
      this.dragging.velocity = this.dragging.velocity === 0 ? v : this.dragging.velocity * 0.7 + v * 0.3;
      this.dragging.lastY = e.clientY;
      this.dragging.lastT = e.timeStamp;
    }
    this.scrollTo(this.dragging.startRow + (this.dragging.startY - e.clientY) / this.rowHeight);
  }

  private onPointerUp(e: PointerEvent): void {
    if (this.selDrag !== null) {
      this.stopAutoScroll();
      this.selDrag = null;
      if (this.rowsEl.hasPointerCapture(e.pointerId)) this.rowsEl.releasePointerCapture(e.pointerId);
      return;
    }
    if (!this.dragging) return;
    const { startY, startRow, lastT, velocity } = this.dragging;
    this.dragging = null;
    if (Math.abs(startY - e.clientY) <= 6) return void this.clickCell(e.target);
    // A finger that came to rest before lifting was placing the view, not
    // throwing it, however fast it was moving a moment earlier.
    if (e.type === "pointerup" && e.timeStamp - lastT < 80 && Math.abs(velocity) > GLIDE_STOP)
      this.startGlide(startRow + (startY - e.clientY) / this.rowHeight, velocity);
  }

  /** Keep scrolling after the finger lifts, slowing to a stop. A file is long
   *  and a screen is short, and without a throw every screenful costs a drag. */
  private startGlide(pos: number, velocity: number): void {
    let last = -1;
    const step = (now: number): void => {
      // A frame the browser skipped still happened; a frame it took its time
      // over should not fling the view a page further, hence the ceiling.
      const dt = last < 0 ? 16 : Math.min(now - last, 64);
      last = now;
      pos += velocity * dt;
      velocity *= Math.pow(GLIDE_DECAY, dt);
      const stopped = pos < 0 || pos > this.maxTopRow || Math.abs(velocity) < GLIDE_STOP;
      this.scrollTo(pos);
      this.glide = stopped ? null : requestAnimationFrame(step);
    };
    this.glide = requestAnimationFrame(step);
  }

  private stopGlide(): void {
    if (this.glide !== null) cancelAnimationFrame(this.glide);
    this.glide = null;
  }

  private clickCell(target: EventTarget | null): void {
    if (!(target instanceof HTMLElement)) return;
    const off = target.dataset["off"];
    const pane = target.dataset["pane"];
    if (off === undefined || (pane !== "hex" && pane !== "ascii")) return;
    const bit = target.dataset["bit"];
    this.setCursor(Number(off), { pane, bit: bit === undefined ? 0 : Number(bit) });
  }

  /**
   * The bit the pointer is over and how much of the file the cell under it
   * stands for.
   *
   * Read from the element at the point rather than from the event's target,
   * because a captured drag reports the capturing element for every move. A
   * given `pane` also pins the x into that column, which is both what keeps a
   * drag in the column it started in and what stops it falling into the
   * address gutter or the field chips.
   */
  private hitAt(x: number, y: number, pane?: Pane): { pane: Pane; bit: number; unit: number } | null {
    let cx = x;
    if (pane !== undefined) {
      const which = pane === "ascii" ? ".hv-ascii" : this.mode === "binary" ? ".hv-bits" : ".hv-hex";
      const col = this.rowEls[0]?.querySelector(which);
      if (col instanceof HTMLElement) {
        const r = col.getBoundingClientRect();
        cx = Math.min(r.right - 1, Math.max(r.left + 1, x));
      }
    }
    const at = document.elementFromPoint(cx, y);
    if (!(at instanceof HTMLElement)) return null;
    // The field column is part of a row but is not part of the file: pressing a
    // chip picks that field, and moving the cursor to the row first would undo
    // what the press was for.
    if (at.closest(".hv-note") !== null) return null;
    // A heading is not part of the file either: it names the part that starts
    // under it, and pressing it goes there. A drag across it is another
    // matter: that reads as the byte the heading sits before.
    const head = at.closest<HTMLElement>(".hv-headings");
    if (head !== null) {
      if (pane === undefined) return null;
      const off = Number(head.dataset["segOff"]);
      if (Number.isFinite(off)) return { pane: pane ?? this.pane, bit: off * 8, unit: 8 };
    }
    const cell = at.closest<HTMLElement>("[data-off]");
    if (cell !== null) {
      const p = cell.dataset["pane"];
      const off = Number(cell.dataset["off"]);
      const bit = cell.dataset["bit"];
      if ((p !== "hex" && p !== "ascii") || !Number.isFinite(off)) return null;
      if (bit === undefined) return { pane: pane ?? p, bit: off * 8, unit: 8 };
      return { pane: pane ?? p, bit: off * 8 + Number(bit), unit: 1 };
    }
    // A cell held open and empty, on the far side of a cut row. It has no byte
    // of its own, but it sits under the column that names one: the byte its
    // place in the row stands for, which is the one the reader is pointing at.
    const col = at.parentElement;
    const row0 = at.closest<HTMLElement>(".hv-row");
    if (col !== null && row0 !== null && /\bhv-(?:hex|bits|ascii)\b/.test(col.className)) {
      const idx = this.rowEls.indexOf(row0);
      const i = Array.prototype.indexOf.call(col.children, at);
      if (idx >= 0 && i >= 0) {
        const off = (this.topRow + idx) * this.bytesPerRow + i;
        if (off <= this.doc.lengthBytes) return { pane: pane ?? this.pane, bit: off * 8, unit: 8 };
      }
    }
    // Rows past the end of the file have no cells in them, so the row itself is
    // all there is to go on.
    const row = at.closest<HTMLElement>(".hv-row");
    if (row === null) return null;
    const idx = this.rowEls.indexOf(row);
    if (idx < 0) return null;
    const off = Math.min(this.doc.lengthBytes, (this.topRow + idx) * this.bytesPerRow);
    return { pane: pane ?? this.pane, bit: off * 8, unit: 8 };
  }

  /**
   * Carry the selection to wherever the pointer is, scrolling first when it has
   * left the grid.
   *
   * The two have to happen together: the view is virtually scrolled, so the
   * rows under a pointer held past the edge change as it moves, and a scroll
   * that did not re-read the position would stop extending.
   *
   * Both ends move so that the byte pressed on and the byte under the pointer
   * are always both in, which is what a hex editor's drag means and what a
   * text editor's caret-between-characters model does not give.
   */
  private dragExtend(): void {
    const d = this.selDrag;
    if (d === null) return;
    const r = this.rowsEl.getBoundingClientRect();
    const before = this.topRow;
    const above = d.y < r.top;
    const below = d.y > r.bottom;
    if (above || below) {
      const over = above ? r.top - d.y : d.y - r.bottom;
      const rows = Math.min(8, 1 + Math.floor(over / 24));
      this.topRow = Math.max(0, Math.min(this.maxTopRow, this.topRow + (above ? -rows : rows)));
      if (d.raf === null) {
        d.raf = requestAnimationFrame(() => {
          if (this.selDrag === null) return;
          this.selDrag.raf = null;
          this.dragExtend();
        });
      }
    } else {
      this.stopAutoScroll();
    }
    // Pinned to the last row rather than the bottom edge: the rows do not
    // always fill the space, and a point in the slack under them is nowhere.
    const last = this.rowsEl.lastElementChild?.getBoundingClientRect().bottom ?? r.bottom;
    const y = Math.min(r.bottom - 1, last - 1, Math.max(r.top + 1, d.y));
    const hit = this.hitAt(d.x, y, d.pane);
    const anchor = hit === null ? 0 : hit.bit >= d.anchor ? d.anchor : d.anchor + d.unit;
    const focus = hit === null ? 0 : hit.bit >= d.anchor ? hit.bit + hit.unit : hit.bit;
    if (hit === null || (this.selAnchor === anchor && this.selFocus === focus)) {
      if (this.topRow !== before) this.render();
      return;
    }
    this.setSelection(anchor, focus);
    this.setCursor(Math.floor(hit.bit / 8), { pane: d.pane, bit: hit.bit % 8, select: "keep" });
  }

  private stopAutoScroll(): void {
    const d = this.selDrag;
    if (d !== null && d.raf !== null) {
      cancelAnimationFrame(d.raf);
      d.raf = null;
    }
  }

  private onTrackDown(e: PointerEvent): void {
    this.stopGlide();
    this.track.setPointerCapture(e.pointerId);
    this.onTrackMove(e);
  }
  private onTrackMove(e: PointerEvent): void {
    if (!this.track.hasPointerCapture(e.pointerId)) return;
    const r = this.track.getBoundingClientRect();
    const thumbH = this.thumb.offsetHeight;
    const frac = (e.clientY - r.top - thumbH / 2) / Math.max(1, r.height - thumbH);
    // The bottom of the track is the end of the file, however many rows past
    // the usual last top row that turns out to be.
    this.scrollTo(frac >= 1 ? Infinity : Math.max(0, frac) * this.maxTopRow);
  }
  private onTrackUp(e: PointerEvent): void {
    if (this.track.hasPointerCapture(e.pointerId)) this.track.releasePointerCapture(e.pointerId);
  }

  // ----- editing -----

  private onKey(e: KeyboardEvent): void {
    this.stopGlide();
    const bpr = this.bytesPerRow;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "z" && !e.shiftKey) return void (e.preventDefault(), this.doc.undo());
    if (mod && (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey)))
      return void (e.preventDefault(), this.doc.redo());

    // Shift turns every move into an extension. The anchor is where the cursor
    // was when the first shifted key was pressed, so extending and then
    // shrinking again ends with nothing selected.
    const sel: Select = e.shiftKey ? { anchor: this.selAnchor ?? this.cursorState.bitOffset } : "clear";

    if (mod && e.key.toLowerCase() === "a") {
      e.preventDefault();
      // The cursor stays where it is. Jumping to the end of a four gigabyte
      // file to say that all of it is selected loses the reader's place.
      this.setSelection(0, this.doc.lengthBits);
      this.render();
      return;
    }
    if (mod && e.key.toLowerCase() === "c") {
      e.preventDefault();
      void this.copySelection();
      return;
    }
    if (mod && (e.key === "Home" || e.key === "End")) {
      // Ctrl+Home and Ctrl+End are the ends of the file. Plain Home and End
      // used to mean that with shift held, which took shift away from the one
      // thing it means everywhere else, so the file ends moved onto Ctrl.
      e.preventDefault();
      return this.setCursor(e.key === "Home" ? 0 : this.doc.lengthBytes, { select: sel });
    }
    if (mod) return;

    const bitMode = this.mode === "binary" && this.pane === "hex";
    if ((e.key === "Delete" || e.key === "Backspace") && this.selectionRange !== null) {
      e.preventDefault();
      this.deleteSelection();
      return;
    }
    switch (e.key) {
      case "ArrowLeft":
        e.preventDefault();
        if (bitMode) return this.setBitCursor(this.cursorState.bitOffset - 1, { select: sel });
        if (this.pane === "hex" && this.nibble === 1) return this.setCursor(this.cursor, { nibble: 0, select: sel });
        return this.setCursor(this.cursor - 1, { select: sel });
      case "ArrowRight":
        e.preventDefault();
        if (bitMode) return this.setBitCursor(this.cursorState.bitOffset + 1, { select: sel });
        return this.setCursor(this.cursor + 1, { select: sel });
      case "ArrowUp":
        e.preventDefault();
        return this.setCursor(this.cursor - bpr, { select: sel });
      case "ArrowDown":
        e.preventDefault();
        return this.setCursor(this.cursor + bpr, { select: sel });
      case "PageUp": {
        e.preventDefault();
        const rows = this.pageUp();
        return this.setCursor(this.cursor - rows * bpr, { select: sel });
      }
      case "PageDown": {
        e.preventDefault();
        const rows = this.pageDown();
        return this.setCursor(this.cursor + rows * bpr, { select: sel });
      }
      case "Home":
        e.preventDefault();
        return this.setCursor(this.cursor - (this.cursor % bpr), { select: sel });
      case "End":
        e.preventDefault();
        return this.setCursor(this.cursor - (this.cursor % bpr) + bpr - 1, { select: sel });
      case "Tab":
        e.preventDefault();
        return this.setCursor(this.cursor, { pane: this.pane === "hex" ? "ascii" : "hex", select: "keep" });
      case "Insert":
        e.preventDefault();
        this.insertMode = !this.insertMode;
        this.render();
        return this.onCursorChange(this.cursorState);
      case "Delete":
        e.preventDefault();
        if (bitMode) {
          const at = this.cursorState.bitOffset;
          if (at < this.doc.lengthBits) this.doc.deleteBits(at, 1);
          return this.setBitCursor(at);
        }
        if (this.cursor < this.doc.lengthBytes) this.doc.delete(this.cursor, 1);
        return this.setCursor(this.cursor);
      case "Backspace":
        e.preventDefault();
        if (bitMode) {
          const at = this.cursorState.bitOffset;
          if (at > 0) {
            this.doc.deleteBits(at - 1, 1);
            this.setBitCursor(at - 1);
          }
          return;
        }
        if (this.cursor > 0) {
          this.doc.delete(this.cursor - 1, 1);
          this.setCursor(this.cursor - 1);
        }
        return;
      case "Escape":
        // One thing per press, the newest first: the selection the user just
        // made, then the field the cursor is in.
        if (this.selectionRange !== null) {
          this.clearSelection();
          return;
        }
        // Move first, then drop the highlight: the cursor event can pick a new
        // field, and Escape's job is to leave nothing highlighted.
        this.setCursor(this.cursor, { nibble: 0 });
        if (this.highlight.length > 0) {
          this.setHighlight(null);
          this.onHighlightClear();
        }
        return;
    }

    if (e.key.length !== 1 || e.altKey) return;
    e.preventDefault();
    // Whether the key is one this pane takes is settled before anything is
    // deleted: a stray letter in the bytes column must not take a selection
    // with it.
    const usable = bitMode
      ? e.key === "0" || e.key === "1"
      : this.pane === "hex"
        ? !Number.isNaN(parseInt(e.key, 16))
        : e.key.charCodeAt(0) <= 0xff;
    if (!usable) return;
    // Typing over a selection replaces it, and the delete and the first digit
    // are one thing the user did, so they undo together.
    const replacing = this.selectionRange !== null;
    if (replacing) {
      this.doc.beginBatch();
      this.deleteSelection();
    }
    if (bitMode) this.typeBit(e.key, replacing);
    else if (this.pane === "hex") this.typeHex(e.key, replacing);
    else this.typeAscii(e.key, replacing);
    if (replacing) this.doc.endBatch();
  }

  /**
   * Remove the selected bits as one undo step and leave the cursor where they
   * were. A byte-aligned run is one byte-level delete, since the piece table
   * splits pieces rather than moving bytes.
   */
  private deleteSelection(): boolean {
    const sel = this.selectionRange;
    if (sel === null) return false;
    const bits = sel.endBit - sel.startBit;
    if (sel.startBit % 8 === 0 && bits % 8 === 0) this.doc.delete(sel.startBit / 8, bits / 8);
    else this.doc.deleteBits(sel.startBit, bits);
    this.setBitCursor(sel.startBit);
    return true;
  }

  /** `insert` is what typing over a selection wants whatever the mode: the
   *  bytes that would be overwritten are the ones after the deleted range. */
  private typeBit(ch: string, insert = false): void {
    if (ch !== "0" && ch !== "1") return;
    const at = this.cursorState.bitOffset;
    const data = Uint8Array.of(ch === "1" ? 0x80 : 0);
    if (insert || this.insertMode || at >= this.doc.lengthBits) this.doc.insertBits(at, data, 1);
    else this.doc.overwriteBits(at, data, 1);
    this.setBitCursor(at + 1);
  }

  private currentByte(): number {
    return this.doc.read(this.cursor, 1).bytes[0] ?? 0;
  }

  private typeHex(ch: string, insert = false): void {
    const v = parseInt(ch, 16);
    if (Number.isNaN(v)) return;
    const atEnd = this.cursor >= this.doc.lengthBytes;
    if (this.nibble === 0) {
      if (insert || this.insertMode || atEnd) {
        this.doc.insert(this.cursor, Uint8Array.of(v << 4));
      } else {
        this.doc.overwrite(this.cursor, Uint8Array.of((v << 4) | (this.currentByte() & 0x0f)));
      }
      this.setCursor(this.cursor, { nibble: 1 });
    } else {
      this.doc.amendOverwrite(this.cursor, Uint8Array.of((this.currentByte() & 0xf0) | v));
      this.setCursor(this.cursor + 1, { nibble: 0 });
    }
  }

  private typeAscii(ch: string, insert = false): void {
    const code = ch.charCodeAt(0);
    if (code > 0xff) return;
    const atEnd = this.cursor >= this.doc.lengthBytes;
    if (insert || this.insertMode || atEnd) this.doc.insert(this.cursor, Uint8Array.of(code));
    else this.doc.overwrite(this.cursor, Uint8Array.of(code));
    this.setCursor(this.cursor + 1);
  }

  /**
   * Put the selection on the clipboard: hex pairs from the bytes column, the
   * text as it is shown from the text column. Hex pairs are what the search
   * bar's hex box takes, so a copy from here pastes straight into a search.
   *
   * The size is checked before the bytes are asked for. Fetching a gigabyte in
   * order to refuse it afterwards is the wait this limit exists to prevent.
   */
  private async copySelection(): Promise<void> {
    const sel = this.selectionRange;
    if (sel === null) return;
    const start = Math.floor(sel.startBit / 8);
    const n = Math.ceil(sel.endBit / 8) - start;
    if (n > COPY_LIMIT_BYTES) return this.say(COPY_TOO_BIG(n));
    await this.doc.ensureRange(start, n);
    const { bytes, complete } = this.doc.read(start, n);
    if (!complete) return this.say(COPY_PENDING);
    const asText = this.pane === "ascii";
    // The text column's own reading, byte for byte, so what is copied is what
    // is on screen rather than a decode it never showed.
    const text = asText ? Array.from(bytes, asciiGlyph).join("") : Array.from(bytes, (b) => HEX[b] ?? "").join(" ");
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      return this.say(COPY_FAILED);
    }
    this.say(COPY_DONE(n, asText));
  }

  // ----- rendering -----

  /**
   * Which bits of byte `off` the highlight covers, as [from, to) runs within
   * 0..8, in order and not touching. Empty where the byte is not covered.
   *
   * A run of no bits is kept rather than dropped: a field of no length still
   * has a place, and marking the byte it sits in front of would say it covers
   * that byte, which it does not.
   */
  private highlightBits(off: number): Run[] {
    const out: Run[] = [];
    for (const h of this.highlight) {
      const from = Math.max(h.startBit, off * 8) - off * 8;
      const to = Math.min(h.endBit, off * 8 + 8) - off * 8;
      if (to < from || from > 8 || to < 0) continue;
      // An empty run belongs to the byte it starts in, and to that byte only,
      // so the one past the end of a previous byte is not counted twice.
      if (to === from && (from === 8 || h.endBit !== h.startBit)) continue;
      out.push({ from, to });
    }
    if (out.length < 2) return out;
    out.sort((a, b) => a.from - b.from || a.to - b.to);
    const merged: Run[] = [];
    for (const r of out) {
      const last = merged[merged.length - 1];
      // Two empty runs at the same place are one mark, not two.
      if (last !== undefined && r.from <= last.to) last.to = Math.max(last.to, r.to);
      else merged.push(r);
    }
    return merged;
  }

  /** Mark part of a byte in hex mode: a bar under the bits the field covers,
   *  one length of bar per run, or a tick where a run has no bits. */
  private markBits(el: HTMLElement, runs: readonly Run[]): string {
    // The cell is 3ch wide: half a character of padding, two digits, half again.
    const pad = 100 / 6;
    const step = (100 - 2 * pad) / 8;
    const stops: string[] = [];
    let at = 0;
    for (const r of runs) {
      // A run of no bits still shows, as a mark a fraction of a bit wide, so
      // that a field of no length is visible where it sits.
      const from = pad + r.from * step;
      const to = pad + Math.max(r.to, r.from + 0.15) * step;
      stops.push(`transparent ${at}%`, `transparent ${from}%`, `var(--accent) ${from}%`, `var(--accent) ${to}%`);
      at = to;
    }
    if (stops.length === 0) return "";
    stops.push(`transparent ${at}%`, "transparent 100%");
    el.style.backgroundImage = `linear-gradient(to right, ${stops.join(", ")})`;
    // The class goes back to the caller, which writes every class this cell
    // wants in one go.
    return " hv-hlbits";
  }

  /**
   * The part of byte `off` the selection covers, as one [from, to) run within
   * 0..8, or null.
   *
   * Asked per byte on screen, so a selection of a whole four gigabyte file
   * costs what a selection of one row costs.
   */
  private selectionBits(sel: BitRange, off: number): Run | null {
    const from = Math.max(sel.startBit, off * 8) - off * 8;
    const to = Math.min(sel.endBit, off * 8 + 8) - off * 8;
    return to > from && from < 8 && to > 0 ? { from, to } : null;
  }

  /** The eight bits of one byte, split into spans only where that is needed. */
  private fillBits(cell: HTMLElement, byte: number | null, off: number, hl: readonly Run[], sel: Run | null): void {
    const text = byte === null ? "········" : byte.toString(2).padStart(8, "0");
    if (byte === null) cell.classList.add("hv-pending");
    const onCursor = off === this.cursor;
    const whole = covers(hl, 0, 8);
    const selClass = this.pane === "hex" ? "hv-sel" : "hv-sel-weak";
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
      if (onCursor && k === this.bit) {
        s.classList.add("hv-cur", this.pane === "hex" ? "hv-focus" : "hv-dim");
        if (this.insertMode) s.classList.add("hv-ins");
      }
      cell.append(s);
    }
  }

  /** Where the spans on screen land: which one covers each byte, and which
   *  start on each row. A field is named on the row it starts on; one that
   *  started above the view is named on the first row, so nothing on screen is
   *  left unexplained. */
  private placeSpans(start: number, windowBytes: number, bpr: number): Placed {
    const { spans, more, error: trouble } = this.spansForView(start, windowBytes);
    const byteSpan = new Int32Array(windowBytes).fill(-1);
    const byRow: Chip[][] = Array.from({ length: this.visibleRows }, () => []);
    for (const [i, s] of spans.entries()) {
      const from = Math.floor(s.offset_bits / 8);
      const to = Math.ceil((s.offset_bits + s.size_bits) / 8);
      for (let b = Math.max(from, start); b < Math.min(to, start + windowBytes); b++) {
        byteSpan[b - start] = i;
      }
      const row = from < start ? 0 : Math.floor((from - start) / bpr);
      if (row >= 0 && row < this.visibleRows && to > start) {
        const chips = byRow[row] as Chip[];
        const prev = chips[chips.length - 1];
        // Elements of one list, one after another on the row, are one chip
        // saying how many: `[0]`, `[1]`, `[2]` say less than `cell_pointers
        // 3 values`. A chip carried from above the view stays its own, since
        // its arrow is about where it started.
        if (prev !== undefined && !prev.carried && from >= start && foldable(s) && foldable(prev.span) && sameList(prev.span, s)) {
          if (prev.run.length === 0) prev.run.push(prev.span);
          prev.run.push(s);
        } else {
          chips.push({ span: s, carried: from < start, run: [] });
        }
      }
    }
    return { spans, more, trouble, byteSpan, byRow };
  }

  /**
   * The headings that fall on each row on screen. The sections are sorted by
   * offset and a file of a hundred thousand pages has a heading for each, so
   * the first one on screen is found by bisection and the rest read off in
   * order.
   */
  private headingsByRow(start: number, windowBytes: number, bpr: number): OutlineHeading[][] {
    const byRow: OutlineHeading[][] = [];
    const secs = this.sections;
    const fromBit = start * 8;
    const toBit = (start + windowBytes) * 8;
    let lo = 0;
    let hi = secs.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if ((secs[mid] as OutlineHeading).offsetBits < fromBit) lo = mid + 1;
      else hi = mid;
    }
    for (let i = lo; i < secs.length; i++) {
      const h = secs[i] as OutlineHeading;
      if (h.offsetBits >= toBit) break;
      const row = Math.floor((Math.floor(h.offsetBits / 8) - start) / bpr);
      (byRow[row] ??= []).push(h);
    }
    return byRow;
  }

  /** The heading lines for the parts that start at one place: for each, its
   *  colour, name, address range, size and share of the file, as the listing
   *  gives them. Pressing one goes to the part's first byte. `at` is the byte
   *  the block sits before, which is what a drag across it reads as. */
  private headingBlock(heads: readonly OutlineHeading[], fileBits: number, at: number): HTMLElement {
    const block = document.createElement("div");
    block.className = "hv-headings";
    block.dataset["segOff"] = String(at);
    for (const h of heads) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = `hv-heading hv-heading-${h.level}`;
      if (h.level === 0) {
        const swatch = document.createElement("span");
        swatch.className = "hv-swatch";
        swatch.style.background = h.color;
        b.append(swatch);
      }
      const name = headingName(h, fileBits);
      const nameEl = document.createElement("b");
      nameEl.className = "hv-heading-name";
      nameEl.textContent = name;
      const range = document.createElement("span");
      range.className = "hv-heading-range";
      range.textContent = rangeText(h.offsetBits, h.sizeBits);
      const size = document.createElement("span");
      size.className = "hv-heading-size";
      const share = shareText(h.sizeBits, fileBits);
      size.textContent = `${formatBytes(h.sizeBits / 8)}${share === "" ? "" : ` · ${share}`}`;
      b.append(nameEl, range, size);
      b.title = HEADING_TIP(name);
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        this.setBitCursor(h.offsetBits, { pane: "hex" });
        // An empty path is the whole file, which is not what a heading over
        // a run of fields at its front is for.
        if (h.path.length > 0) this.onPickField(h.path);
      });
      block.append(b);
    }
    return block;
  }

  /** Spans for the rows on screen. A pending reply leaves the column empty
   *  for one frame; the fetched chunks trigger another render. */
  private spansForView(start: number, count: number): { spans: Span[]; more: boolean; error: string | null } {
    const key = `${start}:${count}:${this.doc.template ?? ""}`;
    if (this.spanCache?.key === key) return this.spanCache;
    const max = Math.min(SPAN_LIMIT, count * 8);
    const r = this.doc.spans(start * 8, (start + count) * 8, max);
    // Pending: the bytes are on their way and another render follows. Error:
    // the template cannot read what is here, usually after an edit that
    // changed a length, and an empty column would not say that.
    // Nothing to annotate yet, whether the bytes are on their way or the
    // structure is still being worked out. Both come back on their own.
    if (r.status === "pending" || r.status === "working") return { spans: [], more: false, error: null };
    if (r.status === "error") return { spans: [], more: false, error: r.message };
    this.spanCache = { key, spans: r.node, more: r.node.length >= max, error: null };
    return this.spanCache;
  }

  /** What a chip says. A run of list elements is named for the list and says
   *  how many; a structure that reads on one line is the whole chip, since
   *  `[47]` is the element's number in a repeat and says nothing, and the
   *  line says everything. */
  private chipText(c: Chip): ChipText {
    const s = c.span;
    if (c.run.length > 0) return { name: listName(s), detail: runDetail(c.run.length) };
    if (s.gap) return { name: GAP_LABEL, detail: chipDetail(s) };
    if (s.line !== null) return { name: s.line, detail: "" };
    return { name: s.name, detail: chipDetail(s) };
  }

  /** One entry in the annotation column, coloured to match its bytes. */
  private chip(c: Chip, text: ChipText): HTMLElement {
    const s = c.span;
    const { name, detail } = text;
    const el = document.createElement("button");
    el.type = "button";
    el.className = "hv-chip";
    if (s.gap) el.classList.add("hv-chip-gap");
    else el.classList.add(fieldClass(s.kind));
    if (c.carried) el.classList.add("hv-chip-carried");
    const nameEl = document.createElement("b");
    nameEl.textContent = name;
    el.append(nameEl);
    if (detail !== "") {
      const v = document.createElement("span");
      v.className = "hv-chip-val";
      v.textContent = detail;
      el.append(v);
    }
    const path = [...s.trail, s.name].join(" ");
    if (c.run.length > 0) {
      // The first few, by number and value, so the reader can see what kind
      // of thing the run is without opening it.
      const first = c.run.slice(0, 6).map((e) => `${e.name} ${chipDetail(e)}`);
      if (c.run.length > first.length) first.push("\u2026");
      el.title = `${s.trail.join(" ")} \u00b7 ${s.type} \u00b7 ${detail}: ${first.join(", ")}`;
    } else if (s.gap) {
      el.title = `No field covers these ${detail}. Inside: ${path}`;
    } else if (c.carried) {
      // The arrow says "this began further up", which a screen reader cannot
      // see and a first-time reader should not have to work out.
      el.title = `Starts above the visible rows: ${path}, ${detail}`;
      el.setAttribute("aria-label", `starts above: ${name}, ${detail}`);
    } else {
      el.title = `${path} \u00b7 ${s.type}`;
    }
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      if (!s.gap) this.onPickField(s.path);
    });
    if (s.gap) el.disabled = true;
    return el;
  }

  /**
   * How wide the chips' own text is drawn, read off a chip that has been. A
   * chip's name is bold sans and its value mono, so counting characters at
   * one width was wrong for both, and wrong by enough to predict three lines
   * where the browser drew four.
   *
   * Null until a chip exists to read a font from; the caller keeps the
   * character count until then and draws once more when this arrives.
   */
  private readChipFonts(): ChipMeasure | null {
    const nameEl = this.rowsEl.querySelector(".hv-chip > b") as HTMLElement | null;
    if (nameEl === null) return null;
    const valEl = this.rowsEl.querySelector(".hv-chip-val") as HTMLElement | null;
    const ctx = document.createElement("canvas").getContext("2d");
    if (ctx === null) return null;
    // Built from the longhands: what `font` computes to is not something every
    // browser will hand back whole.
    const font = (el: HTMLElement): string => {
      const s = getComputedStyle(el);
      return `${s.fontStyle} ${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;
    };
    const nameFont = font(nameEl);
    const valFont = valEl === null ? nameFont : font(valEl);
    const width = (f: string, s: string): number => {
      ctx.font = f;
      return ctx.measureText(s).width;
    };
    return { name: (s) => width(nameFont, s), value: (s) => width(valFont, s) };
  }

  render(): void {
    // A move draws once, when it has finished moving.
    if (this.settling) return;
    this.fitRows();
    const bpr = this.bytesPerRow;
    const len = this.doc.lengthBytes;
    const addrWidth = Math.max(8, len.toString(16).length);
    const start = this.topRow * bpr;
    const windowBytes = this.visibleRows * bpr;
    const { bytes, complete } = this.doc.read(start, windowBytes);
    const binary = this.mode === "binary";
    const fields = this.showsFields;
    const showText = this.showsText;
    // Below the bytes once the side column has been squeezed too narrow to
    // hold a chip. Until it has been measured the chips go beside the bytes,
    // which is what the measurement is taken from.
    const below = fields && this.arrangement === "below";
    // Full rows grow to hold every chip; condensed ones stop at three lines
    // and count the rest.
    const maxLines = this.isCondensed ? CHIP_LINES : Infinity;
    const templated = this.doc.template !== null;
    const selection = this.selectionRange;

    const { spans, more, trouble, byteSpan, byRow } =
      fields && templated ? this.placeSpans(start, windowBytes, bpr) : NO_SPANS(windowBytes);

    const columns = document.createElement("span");
    columns.textContent =
      " ".repeat(addrWidth) +
      "  " +
      Array.from({ length: bpr }, (_, i) => (binary ? (HEX[i] ?? "").padEnd(8) : HEX[i])).join(" ");
    this.header.replaceChildren(columns);
    if (showText) {
      // Nothing to label, but the width has to be held so the heading over the
      // fields lands over the fields.
      const gap = document.createElement("span");
      gap.className = "hv-ascii";
      gap.textContent = " ".repeat(bpr);
      this.header.append(gap);
    }
    // Nothing to head when the chips are below the bytes: the header sits over
    // the bytes, and the fields no longer do.
    if (fields && !below) {
      const title = document.createElement("span");
      title.className = "hv-note hv-head-note";
      title.textContent = "Fields";
      this.header.append(title);
    }

    this.fitParts(bpr, binary, showText, fields, below);
    const headsByRow = this.headingsByRow(start, windowBytes, bpr);
    // What each row will be tall once the browser has laid it out, from what
    // was put in it: its lines of chips and the headings above it. Zero for a
    // row past the end of the file.
    const heights: number[] = [];
    for (let r = 0; r < this.rowEls.length; r++) {
      const row = this.rowEls[r];
      const parts = this.parts[r];
      if (!row || parts === undefined) continue;
      const rowStart = start + r * bpr;
      if (rowStart > len) {
        if (!parts.blank) {
          row.replaceChildren();
          parts.blank = true;
          parts.layoutKey = "";
        }
        heights.push(0);
        continue;
      }
      parts.blank = false;
      const heads = headsByRow[r] ?? [];
      // Where each heading goes, as a position in the row. A part that starts
      // part-way along cuts the row there, so the heading sits between the
      // bytes before it and the bytes after. Condensed readings keep every
      // heading above the row: they are the readings that trade room for rows.
      const headsAt = new Map<number, OutlineHeading[]>();
      for (const h of heads) {
        const at = this.isCondensed ? 0 : Math.min(bpr - 1, Math.max(0, Math.floor(h.offsetBits / 8) - rowStart));
        (headsAt.get(at) ?? (headsAt.set(at, []), headsAt.get(at) as OutlineHeading[])).push(h);
      }
      const segs = [...new Set([0, ...headsAt.keys()])].sort((a, b) => a - b);
      // The share of the file changes with its length, so the key does too.
      const layoutKey = `${segs.join(",")}#${heads.map((h) => h.key).join("|")}@${len}`;
      if (layoutKey !== parts.layoutKey) {
        this.layOutRow(row, parts, rowStart, segs, headsAt, len * 8, addrWidth);
        parts.layoutKey = layoutKey;
        // Cells that changed line have to be told which byte they draw again.
        parts.start = -1;
      }
      let height = this.rowHeight * segs.length;
      for (const h of heads) height += this.sizes.heading[h.level];
      const addr = (parts.lines[0] as LineParts).addr;
      setText(addr, rowStart.toString(16).padStart(addrWidth, "0"));
      // Which bytes a row stands for only changes when the view moves. A
      // cursor key leaves every address where it was, and writing them all
      // again would be the largest part of the redraw it causes.
      const moved = parts.start !== rowStart;
      parts.start = rowStart;
      for (let i = 0; i < bpr; i++) {
        const off = rowStart + i;
        const h = parts.hexCells[i] as HTMLElement;
        const a = parts.textCells[i] as HTMLElement;
        // What each cell is, gathered as a string and written only if it is
        // not what the cell already says. Most of a redraw changes nothing —
        // a cursor key moves a mark two cells — and a class written back
        // unchanged still costs the browser the styling of that cell.
        let hc = "";
        let ac = "";
        if (h.style.backgroundImage !== "") h.style.backgroundImage = "";
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
        // The linked stretch is marked by the byte, not by the bit: it stands
        // for a place in another document, and half a byte of one is not a
        // finer answer, only a smaller one.
        const link = this.linked;
        const linked = link !== null && off * 8 < link.endBit && (off + 1) * 8 > link.startBit;
        const hl = selection === null ? this.highlightBits(off) : [];
        const sb = selection === null ? null : this.selectionBits(selection, off);
        let text = binary ? "        " : "  ";
        if (off < len) {
          const b = bytes[off - start] ?? 0;
          setText(a, complete ? asciiGlyph(b) : " ");
          if (complete && !(b >= 0x20 && b < 0x7f)) ac += " hv-np";
          if (!binary) {
            text = complete ? HEX[b] ?? "" : "··";
            if (!complete) hc += " hv-pending";
          }
        } else {
          setText(a, " ");
          if (off === len) hc += " hv-end";
        }
        if (!binary || off >= len) setText(h, text);

        const si = fields && off >= start && off < start + windowBytes ? byteSpan[off - start] ?? -1 : -1;
        if (si >= 0) {
          const s = spans[si];
          if (s !== undefined && !s.gap) {
            hc += ` hv-tint ${fieldClass(s.kind)}`;
            if (off === Math.floor(s.offset_bits / 8)) hc += " hv-field-start";
          }
        }
        if (hl.length > 0) {
          // The text column cannot show part of a byte, so a partly covered
          // byte is marked more faintly there than a fully covered one, and a
          // run of no bits is not marked there at all: one character standing
          // for a whole byte cannot say "between two of these".
          const whole = covers(hl, 0, 8);
          const any = hl.some((r) => r.to > r.from);
          if (any) ac += whole ? " hv-hl" : " hv-hl-weak";
          if (!binary && off < len) {
            if (whole) hc += " hv-hl";
            else hc += this.markBits(h, hl);
          }
        }
        if (sb !== null) {
          // A byte only partly selected is marked weakly in both columns: two
          // hex digits and one text character each stand for the whole byte,
          // and a full mark would say the whole byte is in.
          const whole = sb.from <= 0 && sb.to >= 8;
          if (!binary && off < len) hc += whole && this.pane === "hex" ? " hv-sel" : " hv-sel-weak";
          ac += whole && this.pane === "ascii" ? " hv-sel" : " hv-sel-weak";
        }
        if (linked && link !== null) {
          hc += " hv-linked";
          ac += " hv-linked";
          // The ends of the run get the ends of the outline, so a mark that
          // runs off a row still reads as one stretch rather than as a box per
          // byte.
          if (off * 8 <= link.startBit) {
            hc += " hv-linked-first";
            ac += " hv-linked-first";
          }
          if ((off + 1) * 8 >= link.endBit) {
            hc += " hv-linked-last";
            ac += " hv-linked-last";
          }
        }
        if (off === this.cursor) {
          // In binary the bits carry the cursor, except past the end of the
          // file where there are no bits to carry it.
          if (!binary || off >= len) {
            hc += this.pane === "hex" ? " hv-cur hv-focus" : " hv-cur hv-dim";
            if (!binary && this.pane === "hex" && this.nibble === 1) hc += " hv-nib1";
            if (this.insertMode) hc += " hv-ins";
          }
          ac += this.pane === "ascii" ? " hv-cur hv-focus" : " hv-cur hv-dim";
          if (this.insertMode) ac += " hv-ins";
        }
        // The bits inside a cell carry their own marks, so in binary the cell
        // has only what `fillBits` puts on it.
        if (h.className !== hc) h.className = hc;
        if (binary && off < len) this.fillBits(h, complete ? bytes[off - start] ?? 0 : null, off, hl, sb);
        if (a.className !== ac) a.className = ac;
      }
      if (fields) {
        for (let j = 0; j < segs.length; j++) (parts.lines[j] as LineParts).note.replaceChildren();
        const firstNote = (parts.lines[0] as LineParts).note;
        if (!templated || trouble !== null) {
          if (r === 0) {
            const none = document.createElement("span");
            none.className = "hv-chip hv-chip-gap hv-chip-wide";
            none.textContent = trouble ?? NO_TEMPLATE;
            if (trouble !== null) none.title = trouble;
            firstNote.append(none);
          }
          heights.push(height);
          continue;
        }
        // A cut row's chips go beside the bytes they name. A run of list
        // elements folded into one chip goes with the first of them, since
        // that is the byte the chip's arrow points at.
        const buckets: Chip[][] = segs.map(() => []);
        for (const c of byRow[r] ?? []) {
          let j = 0;
          if (!c.carried) {
            const at = Math.floor(c.span.offset_bits / 8) - rowStart;
            while (j + 1 < segs.length && (segs[j + 1] as number) <= at) j++;
          }
          (buckets[j] as Chip[]).push(c);
        }
        const measure = this.chipFonts ?? GUESS_TEXT;
        for (let j = 0; j < segs.length; j++) {
          const note = (parts.lines[j] as LineParts).note;
          const entries = buckets[j] as Chip[];
          const texts = entries.map((c) => this.chipText(c));
          const { shown, lines } = chipLayout(
            texts.map((t, i) => chipWidth(carriedName(t.name, entries[i]), t.detail, measure)),
            this.noteWidth,
            maxLines,
          );
          // Beside the bytes the chips share their line's height with the
          // cells, so the line is the taller of the two. Below them the chips
          // are their own block and their lines add to it.
          height += below ? lines * this.sizes.chipLine : Math.max(0, lines * this.sizes.chipLine - this.rowHeight);
          for (let i = 0; i < shown; i++) note.append(this.chip(entries[i] as Chip, texts[i] as ChipText));
          if (shown < entries.length) {
            const rest = document.createElement("span");
            rest.className = "hv-chip hv-chip-gap hv-chip-rest";
            const left = entries.slice(shown);
            rest.textContent = `+${left.length}`;
            const named = left.slice(0, 8).map((c) => this.chipText(c).name);
            if (left.length > named.length) named.push("\u2026");
            rest.title = `${left.length} more ${left.length === 1 ? "field starts" : "fields start"} on this row: ${named.join(", ")}`;
            note.append(rest);
          }
        }
        if (more && r === this.rowEls.length - 1) {
          const rest = document.createElement("span");
          rest.className = "hv-chip hv-chip-gap";
          rest.textContent = "more fields below";
          rest.title = `The field column shows up to ${SPAN_LIMIT} fields at a time. Scroll down to see the rest.`;
          (parts.lines[segs.length - 1] as LineParts).note.append(rest);
        }
      }
      heights.push(height);
    }

    // Everything the browser has to be asked, asked together: the widths and
    // the fonts the next layout is worked out from, and the heights this one
    // actually came to. One forced layout for the lot, at the end of the draw,
    // rather than one per row.
    const fonts = fields && this.chipFonts === null ? this.readChipFonts() : null;
    let widened = false;
    if (this.metrics === null || this.arrangement === "unknown") {
      const noteEl = fields ? (this.rowEls[0]?.querySelector(".hv-note") as HTMLElement | null) : null;
      // `clientWidth` counts the note's own left padding, which no chip can be
      // drawn in.
      const pad = noteEl === null ? 0 : parseFloat(getComputedStyle(noteEl).paddingLeft) || 0;
      const w = noteEl === null ? 0 : Math.max(0, noteEl.clientWidth - pad);
      // The note existing is what says the width has been measured. A width of
      // zero is an answer — the column has been squeezed away entirely — and
      // waiting for a wider one would mean measuring again on every draw.
      if (fields && this.arrangement === "unknown" && noteEl !== null) {
        // A side column this narrow shows a sliver of one chip and cuts the
        // rest off, so the chips go under the bytes instead, where the whole
        // row is theirs.
        this.arrangement = w < HexView.NOTE_MIN ? "below" : "side";
        // The width just measured belongs to the side column; a note below the
        // bytes has to be measured again, in the place it will be drawn.
        if (this.arrangement === "below") widened = true;
      }
      this.metrics = { noteWidth: w, trackH: this.track.clientHeight };
      // One redraw when the measured width first disagrees with the guess, so
      // the count of what did not fit is right rather than nearly right.
      if (fields && w > 0 && Math.abs(w - this.noteWidth) > 4) widened = true;
      if (fields && !below) this.noteWidth = w;
    }
    // A note below the bytes is as wide as the row, whatever the side column
    // was measured at.
    if (below) {
      const rowW = this.rowEls[0]?.clientWidth ?? 0;
      if (rowW > 0 && Math.abs(rowW - this.noteWidth) > 4) {
        this.noteWidth = rowW;
        widened = true;
      }
    }
    if (fonts !== null) {
      // Set before the redraw, so a font that measures the same cannot send
      // the draw round again.
      this.chipFonts = fonts;
      widened = true;
    }
    if (widened) {
      this.render();
      return;
    }

    // How tall each row actually came out. The prediction above decides how
    // many lines of chips a row holds and what it counts as left over; what
    // the view scrolls by has to be what the browser drew, or a row taller
    // than it was reckoned to be spills over the one below it. A row past the
    // end of the file is still a box with a minimum height, so the prediction
    // is what says which rows are there at all.
    const real = heights.map((h, i) => (h === 0 ? 0 : (this.rowEls[i]?.offsetHeight ?? h)));

    // Which rows are on screen, by the heights measured above: a row whose
    // top is inside the view is on screen, whatever of it is cut off below.
    let onScreen = 0;
    let y = 0;
    for (const h of real) {
      if (h === 0) break;
      if (y < this.viewH) onScreen++;
      y += h;
    }
    // When the end of the file is drawn, how many rows further than usual the
    // view has to go for the last row to be wholly on screen. Counted back
    // from the last row while the rows still fit.
    if (this.topRow + this.rowEls.length >= this.totalRows) {
      const lastIdx = Math.min(real.length - 1, this.totalRows - 1 - this.topRow);
      let sum = 0;
      let first = lastIdx;
      for (let i = lastIdx; i >= 0; i--) {
        sum += real[i] ?? 0;
        if (sum > this.viewH) break;
        first = i;
      }
      this.endSlack = Math.max(0, this.topRow + first - (this.totalRows - this.visibleRows));
    }

    // Scrollbar thumb: position is the fraction of rows above the viewport.
    const trackH = this.metrics.trackH;
    const thumbH = Math.max(24, Math.round((this.visibleRows / this.totalRows) * trackH));
    const top = this.maxTopRow === 0 ? 0 : Math.round((this.topRow / this.maxTopRow) * (trackH - thumbH));
    this.thumb.style.height = `${thumbH}px`;
    this.thumb.style.transform = `translateY(${top}px)`;
    this.onViewport({ startBit: start * 8, endBit: Math.min(len, start + onScreen * bpr) * 8 });
  }
}

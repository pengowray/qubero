// Virtualised address + hex + ascii view.
//
// The browser cannot host a scroll container billions of pixels tall, so this view
// does not use native scrolling for the document. It keeps a place in the file —
// a top row and how far into that row the top edge falls — and renders only the
// rows that fit, with its own scrollbar mapped pixel <-> file offset.
//
// Rows are not all one height: a heading above a part, the extra line a heading
// inside a row cuts it into, and chips that wrap all make a row taller. So the
// view scrolls in pixels over `RowHeights`, a ledger of what each row is worth,
// and a finger that moves twenty pixels moves the bytes twenty pixels whatever
// it is passing over.

import type { Doc, Span } from "./doc.js";
import type { OutlineHeading, Viewport } from "./outline.js";
import { NO_TEMPLATE } from "./strings.js";
import { CHIP_LINES, GUESS_TEXT, type ChipMeasure } from "./chipfit.js";
import { pinnedNoteKey, placeChips, planRowChips, rowNoteKey, type Chip, type ChipBlock } from "./chipplan.js";
import {
  asciiGlyph,
  cellDraw,
  covers,
  highlightBits,
  selectionBits,
  setText,
  type Run,
} from "./hexcell.js";
import { fillNote, fillPlain, newChip, readChipFonts, SPAN_LIMIT, type ChipEl } from "./hexchips.js";
import { cutRow, fillHeadings, headingsByRow } from "./hexheadings.js";
import { RowHeights, type StructuralExtra } from "./rowheights.js";

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

/** How many bytes one copy may carry. A selection can be the whole file, and
 *  the bytes have to be fetched and turned into a string before the clipboard
 *  sees them, so past some size the honest answer is no. */
const COPY_LIMIT_BYTES = 256 * 1024;

/** How long a message stays up before it goes away on its own. */
const NOTICE_MS = 5000;

/** What is left of a throw's speed after a millisecond: about half of it every
 *  350ms, so a hard flick covers tens of rows and a gentle one a handful. */
const GLIDE_DECAY = 0.998;
/** Pixels per millisecond below which a throw is over. Five pixels a second is
 *  not scrolling. */
const GLIDE_STOP = 0.08;

const COPY_TOO_BIG = (bytes: number): string =>
  `Selection too large to copy: ${Math.round(bytes / 1024).toLocaleString()} KB, limit ${COPY_LIMIT_BYTES / 1024} KB.`;
const COPY_PENDING = "Selection is still loading. Try again in a moment.";
const COPY_FAILED = "Couldn't copy to the clipboard.";
const COPY_DONE = (bytes: number, asText: boolean): string =>
  `Copied ${bytes.toLocaleString()} ${bytes === 1 ? "byte" : "bytes"} as ${asText ? "text" : "hex"}.`;

/** What a cursor move does to the selection. An anchor extends from that bit,
 *  which is the one thing a plain "keep" cannot say. */
type Select = "keep" | "clear" | { readonly anchor: number };

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

/**
 * What one draw is drawing: the shape the view is in, the bytes and the spans
 * for the rows on screen, and where the headings fall.
 *
 * Read once at the top of `render` and handed down, so that the steps of a
 * draw agree about what they are drawing however many of them there are, and
 * so that none of them asks the document for the same answer twice.
 */
type Frame = {
  readonly bpr: number;
  readonly len: number;
  readonly addrWidth: number;
  /** The first byte on screen, and how many the rows hold between them. */
  readonly start: number;
  readonly windowBytes: number;
  readonly bytes: Uint8Array;
  /** False while some of those bytes are still on their way. */
  readonly complete: boolean;
  readonly binary: boolean;
  readonly fields: boolean;
  readonly showText: boolean;
  /** True when the chips are drawn under the bytes rather than beside them. */
  readonly below: boolean;
  readonly templated: boolean;
  /** How many lines of chips a row may hold before the rest is counted. */
  readonly maxLines: number;
  readonly selection: BitRange | null;
  readonly spans: Span[];
  /** True when the column stopped short of naming every field on screen. */
  readonly more: boolean;
  readonly trouble: string | null;
  readonly byteSpan: Int32Array;
  readonly byRow: Chip[][];
  readonly headsByRow: OutlineHeading[][];
  /** What the top row carries in from above, put here by the row that finds
   *  it and read by the strip pinned over the rows. */
  pinned: ChipBlock | null;
};

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

export class HexView {
  readonly el: HTMLElement;
  private readonly header: HTMLElement;
  private readonly rowsEl: HTMLElement;
  /** The rows themselves, inside `rowsEl` and shifted up by `topPx` so the top
   *  row can be partly above the edge. `rowsEl` clips it. */
  private readonly rowsInner: HTMLElement;
  /** The chips for fields that began above the visible rows, pinned over the
   *  top edge of the rows rather than drawn inside the top one. In the flow it
   *  made the top row a line of chips taller than the same row is anywhere
   *  else, so every row below it jumped as a long field scrolled past. Only in
   *  the "below" arrangement; hidden when nothing is carried. */
  private readonly pinned: HTMLElement;
  /** What the pinned strip last said, so it is filled again only when it would
   *  say something else. */
  private pinnedKey = "";
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
    /** What the row's chips last said, so they are built again only when they
     *  would say something else. Cleared whenever the lines they live in are. */
    noteKey: string;
  }[] = [];
  private partsShape = "";
  /** What `fitParts` last built the lines for, so a line added mid-draw for a
   *  row that had to be cut is built the same way. */
  private lineShape = { bpr: 16, binary: false, showText: true, fields: false, below: false };

  private topRow = 0;
  /** How far into the top row the top edge of the view falls, in pixels. Always
   *  less than that row's height: `render` normalises it after measuring. */
  private topPx = 0;
  /** What each row is worth in pixels, so a scroll can be one. */
  private readonly ledger = new RowHeights();
  /** How many times in a row a draw has moved the top row while keeping its
   *  place, so a file whose rows keep changing height cannot draw forever. */
  private renormPasses = 0;
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
  private dragging: { startY: number; startPx: number; lastY: number; lastT: number; velocity: number } | null = null;
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
  private metrics: { readonly noteWidth: number; readonly noteLeft: number; readonly trackH: number } | null = null;
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
    this.rowsInner = document.createElement("div");
    this.rowsInner.className = "hv-rows-inner";
    this.pinned = document.createElement("span");
    this.pinned.className = "hv-note hv-note-pinned hv-empty";
    this.rowsEl.append(this.rowsInner, this.pinned);
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
      // An insert or a delete moves every later byte onto a different row, so
      // what those rows were measured at belongs to bytes that are no longer
      // there. The headings move too, and arrive again through `setSections`.
      this.ledger.setRows(this.totalRows);
      this.ledger.clearMeasured();
      this.render();
    });
  }

  // ----- geometry -----

  private get totalRows(): number {
    return Math.max(1, Math.ceil((this.doc.lengthBytes + 1) / this.bytesPerRow));
  }
  /** Where the top edge of the view sits, in pixels from the top of the file. */
  private get scrollY(): number {
    return this.ledger.heightBefore(this.topRow) + this.topPx;
  }
  /** The furthest down the view may go: far enough for the last row to be
   *  whole, and no further. */
  private get maxScrollY(): number {
    return Math.max(0, this.ledger.totalHeight() - this.viewH);
  }

  /**
   * What the headings add to the rows they fall on, worked out before anything
   * is drawn: the height of each heading line, plus one more line of cells per
   * place a heading cuts a row part-way along. Condensed readings put every
   * heading above the row, so they cut nothing.
   *
   * The sections are in order of offset, so the rows come out in order too and
   * one pass builds the lot.
   */
  private rebuildStructural(): void {
    this.ledger.setBase(this.rowHeight);
    this.ledger.setRows(this.totalRows);
    const bpr = this.bytesPerRow;
    const condensed = this.isCondensed;
    const out: StructuralExtra[] = [];
    let row = -1;
    let extra = 0;
    let cuts = new Set<number>();
    for (const h of this.sections) {
      const byte = Math.floor(h.offsetBits / 8);
      const r = Math.floor(byte / bpr);
      if (r !== row) {
        if (extra > 0) out.push({ row, extra });
        row = r;
        extra = 0;
        cuts = new Set<number>();
      }
      extra += this.sizes.heading[h.level] ?? this.sizes.heading[1] ?? this.rowHeight;
      if (!condensed) {
        const at = Math.min(bpr - 1, Math.max(0, byte - r * bpr));
        // Position zero is the row's own start, not a cut in it.
        if (at > 0 && !cuts.has(at)) {
          cuts.add(at);
          extra += this.rowHeight;
        }
      }
    }
    if (extra > 0) out.push({ row, extra });
    this.ledger.setStructural(out);
  }

  /** Pick the field a chip stands for. Held as one function for the life of
   *  the view, since every chip keeps it. */
  private readonly pickField = (path: readonly number[]): void => {
    this.onPickField(path);
  };

  /** Go to the first byte of the part a heading names. Held as one function
   *  for the life of the view, since every heading line keeps it. */
  private readonly pressHeading = (h: OutlineHeading): void => {
    this.setBitCursor(h.offsetBits, { pane: "hex" });
    // An empty path is the whole file, which is not what a heading over
    // a run of fields at its front is for.
    if (h.path.length > 0) this.onPickField(h.path);
  };

  /** Take the parts of the file to draw headings for. Headings above the
   *  cursor's row push it down, so it is brought back on screen if that
   *  pushed it off; a cursor that was already off screen is left there. */
  setSections(sections: readonly OutlineHeading[]): void {
    this.sections = sections;
    this.rebuildStructural();
    // The measurements stand: what is kept is what a row's chips wrapped to,
    // over and above the headings it was reckoned to carry, and a heading
    // arriving or leaving changes the reckoning rather than the wrapping.
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
    // Every measured height was taken at the old shape, so none of them stand.
    this.ledger.clearMeasured();
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
    // Draw where the view was, then pull it back if that turns out to be past
    // the end: the total is an estimate until the rows around here have been
    // measured, and clamping by the old one would jog the view while the
    // reader is only resizing the window.
    this.topRow = Math.min(this.topRow, Math.max(0, this.totalRows - 1));
    this.render();
    if (this.scrollY > this.maxScrollY) this.scrollToY(this.maxScrollY);
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
    // One more than fills the space: the top row is usually cut off by the
    // scroll position, and without the spare there would be a strip of nothing
    // at the bottom for however much of it is above the edge.
    const fit = Math.max(1, Math.ceil(this.viewH / this.rowHeight) + 1);
    if (fit !== this.visibleRows) {
      this.visibleRows = fit;
      this.topRow = Math.min(this.topRow, Math.max(0, this.totalRows - 1));
    }
    // The heading sizes and the base height have just been read, and both are
    // what a row's known height is built from.
    this.rebuildStructural();
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
        noteKey: "",
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
    // Every line the row has ever needed, in order, whether or not this
    // drawing uses it. A row that stops being cut hides its second line rather
    // than dropping it, so the list of things in the row only ever grows and a
    // scroll never takes an element out from under a finger.
    const kids: HTMLElement[] = [];
    for (const [j, lp] of parts.lines.entries()) {
      const on = j < segs.length;
      const at = on ? (segs[j] as number) : 0;
      // Always in place, empty when no part starts here, so that a heading
      // arriving or leaving writes into a block that is already there.
      fillHeadings(lp.head, on ? (heads.get(at) ?? []) : [], fileBits, rowStart + at, this.pressHeading);
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

  private ensureRowEls(): void {
    while (this.rowEls.length < this.visibleRows) {
      const r = document.createElement("div");
      r.className = "hv-row";
      r.setAttribute("role", "row");
      this.rowsInner.append(r);
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
   * the shortfall is worth and looks again.
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
      const was = this.scrollY;
      this.scrollToY(was + deficit);
      if (this.scrollY === was) return;
    }
  }

  /** How far a page key moves, in pixels: everything on screen but nothing
   *  that is not, which for one screenful is the screen itself. */
  private pageStep(): number {
    return Math.max(this.rowHeight, this.viewH);
  }

  /** Scroll down a screenful. Returns how many rows the cursor should move, so
   *  it keeps its place on screen. */
  private pageDown(): number {
    const from = this.topRow;
    this.scrollToY(this.scrollY + this.pageStep());
    const moved = this.topRow - from;
    return moved > 0 ? moved : Math.max(1, Math.floor(this.viewH / this.rowHeight));
  }

  /** Scroll up a screenful. Returns how many rows the cursor should move. */
  private pageUp(): number {
    const from = this.topRow;
    this.scrollToY(this.scrollY - this.pageStep());
    const moved = from - this.topRow;
    return moved > 0 ? moved : Math.max(1, Math.floor(this.viewH / this.rowHeight));
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

  /** Put a row at the top of the view, its first pixel against the top edge. */
  scrollTo(row: number): void {
    this.scrollToY(this.ledger.heightBefore(Math.max(0, Math.floor(row))));
  }

  /**
   * Put the view at a pixel and draw it there.
   *
   * Rounded, because a row drawn at half a pixel is a row of blurred text. The
   * clamp is by the total the ledger holds, which is an estimate for the rows
   * that have never been drawn; near the end of the file drawing it is what
   * corrects the estimate, so a scroll that landed there asks once more.
   */
  private scrollToY(y: number): void {
    for (let pass = 0; pass < 3; pass++) {
      const want = Math.round(Math.max(0, Math.min(this.maxScrollY, y)));
      const at = this.ledger.rowAtY(want);
      if (pass > 0 && at.row === this.topRow && at.offsetPx === this.topPx) break;
      this.topRow = at.row;
      this.topPx = at.offsetPx;
      this.render();
      // Away from the end the total does not move enough to be worth a second
      // draw, and asking again on every wheel tick would double the cost of
      // scrolling.
      if (this.topRow + this.rowEls.length < this.totalRows) break;
    }
  }

  private scrollCursorIntoView(): void {
    const row = Math.floor(this.cursor / this.bytesPerRow);
    const top = this.ledger.heightBefore(row);
    const bottom = top + this.ledger.heightOf(row);
    const y = this.scrollY;
    // Above the top edge — which includes a cursor in the row the edge cuts
    // through, so a step up onto it brings the whole row down.
    if (top < y) {
      this.topRow = row;
      this.topPx = 0;
      return;
    }
    if (bottom <= y + this.viewH) return;
    const at = this.ledger.rowAtY(Math.round(Math.max(0, Math.min(this.maxScrollY, bottom - this.viewH))));
    this.topRow = at.row;
    this.topPx = at.offsetPx;
  }

  private onWheel(e: WheelEvent): void {
    e.preventDefault();
    this.stopGlide();
    const px =
      e.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? e.deltaY * this.rowHeight
        : e.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? e.deltaY * this.viewH
          : e.deltaY;
    this.scrollToY(this.scrollY + px);
  }

  private onPointerDown(e: PointerEvent): void {
    this.el.focus();
    this.stopGlide();
    if (e.pointerType === "touch") {
      this.dragging = { startY: e.clientY, startPx: this.scrollY, lastY: e.clientY, lastT: e.timeStamp, velocity: 0 };
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
      const v = (this.dragging.lastY - e.clientY) / dt;
      this.dragging.velocity = this.dragging.velocity === 0 ? v : this.dragging.velocity * 0.7 + v * 0.3;
      this.dragging.lastY = e.clientY;
      this.dragging.lastT = e.timeStamp;
    }
    this.scrollToY(this.dragging.startPx + (this.dragging.startY - e.clientY));
  }

  private onPointerUp(e: PointerEvent): void {
    if (this.selDrag !== null) {
      this.stopAutoScroll();
      this.selDrag = null;
      if (this.rowsEl.hasPointerCapture(e.pointerId)) this.rowsEl.releasePointerCapture(e.pointerId);
      return;
    }
    if (!this.dragging) return;
    const { startY, startPx, lastT, velocity } = this.dragging;
    this.dragging = null;
    if (Math.abs(startY - e.clientY) <= 6) return void this.clickCell(e.target);
    // A finger that came to rest before lifting was placing the view, not
    // throwing it, however fast it was moving a moment earlier.
    if (e.type === "pointerup" && e.timeStamp - lastT < 80 && Math.abs(velocity) > GLIDE_STOP)
      this.startGlide(startPx + (startY - e.clientY), velocity);
  }

  /** Keep scrolling after the finger lifts, slowing to a stop. A file is long
   *  and a screen is short, and without a throw every screenful costs a drag.
   *  `pos` is where the view is in pixels, `velocity` pixels per millisecond. */
  private startGlide(pos: number, velocity: number): void {
    let last = -1;
    const step = (now: number): void => {
      // A frame the browser skipped still happened; a frame it took its time
      // over should not fling the view a page further, hence the ceiling.
      const dt = last < 0 ? 16 : Math.min(now - last, 64);
      last = now;
      pos += velocity * dt;
      velocity *= Math.pow(GLIDE_DECAY, dt);
      const stopped = pos < 0 || pos > this.maxScrollY || Math.abs(velocity) < GLIDE_STOP;
      this.scrollToY(pos);
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
    const before = this.scrollY;
    const above = d.y < r.top;
    const below = d.y > r.bottom;
    if (above || below) {
      const over = above ? r.top - d.y : d.y - r.bottom;
      const step = Math.min(8, 1 + Math.floor(over / 24)) * this.rowHeight;
      const at = this.ledger.rowAtY(Math.max(0, Math.min(this.maxScrollY, before + (above ? -step : step))));
      this.topRow = at.row;
      this.topPx = at.offsetPx;
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
    const last = this.rowsInner.lastElementChild?.getBoundingClientRect().bottom ?? r.bottom;
    const y = Math.min(r.bottom - 1, last - 1, Math.max(r.top + 1, d.y));
    const hit = this.hitAt(d.x, y, d.pane);
    const anchor = hit === null ? 0 : hit.bit >= d.anchor ? d.anchor : d.anchor + d.unit;
    const focus = hit === null ? 0 : hit.bit >= d.anchor ? hit.bit + hit.unit : hit.bit;
    if (hit === null || (this.selAnchor === anchor && this.selFocus === focus)) {
      if (this.scrollY !== before) this.render();
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
    // The bottom of the track is the end of the file.
    // Infinity rather than the limit as it stands: the end of the file is
    // where the track's bottom means, and how far that is is only settled once
    // the last rows have been drawn and measured.
    this.scrollToY(frac >= 1 ? Infinity : Math.max(0, frac) * this.maxScrollY);
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
    const { byteSpan, byRow } = placeChips(spans, start, windowBytes, bpr, this.visibleRows);
    return { spans, more, trouble, byteSpan, byRow };
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

  /**
   * Everything one draw needs, gathered once: the shape the view is in, the
   * bytes and the spans for the rows on screen, and where the headings fall.
   *
   * Held together so a row can be drawn from it without asking the document
   * anything of its own.
   */
  private frame(): Frame {
    const bpr = this.bytesPerRow;
    const len = this.doc.lengthBytes;
    const start = this.topRow * bpr;
    const windowBytes = this.visibleRows * bpr;
    const { bytes, complete } = this.doc.read(start, windowBytes);
    const fields = this.showsFields;
    const templated = this.doc.template !== null;
    const { spans, more, trouble, byteSpan, byRow } =
      fields && templated ? this.placeSpans(start, windowBytes, bpr) : NO_SPANS(windowBytes);
    return {
      bpr,
      len,
      addrWidth: Math.max(8, len.toString(16).length),
      start,
      windowBytes,
      bytes,
      complete,
      binary: this.mode === "binary",
      fields,
      showText: this.showsText,
      // Below the bytes once the side column has been squeezed too narrow to
      // hold a chip. Until it has been measured the chips go beside the bytes,
      // which is what the measurement is taken from.
      below: fields && this.arrangement === "below",
      templated,
      // Full rows grow to hold every chip; condensed ones stop at three lines
      // and count the rest.
      maxLines: this.isCondensed ? CHIP_LINES : Infinity,
      selection: this.selectionRange,
      spans,
      more,
      trouble,
      byteSpan,
      byRow,
      headsByRow: headingsByRow(this.sections, start, windowBytes, bpr),
      pinned: null,
    };
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
      }
      return 0;
    }
    parts.blank = false;
    const heads = f.headsByRow[r] ?? [];
    const { headsAt, segs } = cutRow(heads, rowStart, bpr, this.isCondensed);
    // The share of the file changes with its length, so the key does too.
    const layoutKey = `${segs.join(",")}#${heads.map((h) => h.key).join("|")}@${len}`;
    if (layoutKey !== parts.layoutKey) {
      this.layOutRow(row, parts, rowStart, segs, headsAt, len * 8, f.addrWidth);
      parts.layoutKey = layoutKey;
      parts.noteKey = "";
      // Cells that changed line have to be told which byte they draw again.
      parts.start = -1;
    }
    let height = this.rowHeight * segs.length;
    for (const h of heads) height += this.sizes.heading[h.level];
    const addr = (parts.lines[0] as LineParts).addr;
    setText(addr, rowStart.toString(16).padStart(f.addrWidth, "0"));
    // Which bytes a row stands for only changes when the view moves. A
    // cursor key leaves every address where it was, and writing them all
    // again would be the largest part of the redraw it causes.
    const moved = parts.start !== rowStart;
    parts.start = rowStart;
    this.drawCells(parts, rowStart, moved, f);
    if (f.fields) height += this.drawNotes(r, parts, rowStart, segs, f);
    return height;
  }

  /** Write one row's bytes, and their text, into the cells they are drawn
   *  in. */
  private drawCells(parts: HexView["parts"][number], rowStart: number, moved: boolean, f: Frame): void {
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
      const hl = selection === null ? highlightBits(this.highlight, off) : [];
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
        link: this.linked,
        cursor: this.cursor,
        pane: this.pane,
        nibble: this.nibble,
        insertMode: this.insertMode,
      });
      if (h.style.backgroundImage !== draw.bits) h.style.backgroundImage = draw.bits;
      setText(a, draw.asciiText);
      if (draw.hexText !== null) setText(h, draw.hexText);
      // The bits inside a cell carry their own marks, so in binary the cell
      // has only what `fillBits` puts on it.
      if (h.className !== draw.hex) h.className = draw.hex;
      if (binary && off < len) this.fillBits(h, complete ? bytes[off - start] ?? 0 : null, off, hl, sb);
      if (a.className !== draw.ascii) a.className = draw.ascii;
    }
  }

  /** Put a row's chips in the blocks beside or below its bytes, and say what
   *  they add to its height. What the top row carries goes on the frame, for
   *  the strip pinned over the rows. */
  private drawNotes(
    r: number,
    parts: HexView["parts"][number],
    rowStart: number,
    segs: readonly number[],
    f: Frame,
  ): number {
    const firstNote = (parts.lines[0] as LineParts).note;
    if (!f.templated || f.trouble !== null) {
      const key = `!${r === 0 ? (f.trouble ?? NO_TEMPLATE) : ""}`;
      if (key !== parts.noteKey) {
        parts.noteKey = key;
        for (let j = 1; j < segs.length; j++) (parts.lines[j] as LineParts).note.replaceChildren();
        const say = r === 0 ? (f.trouble ?? NO_TEMPLATE) : null;
        while (firstNote.childElementCount > (say === null ? 0 : 1)) firstNote.lastElementChild?.remove();
        if (say !== null) {
          if (firstNote.childElementCount === 0) firstNote.append(newChip(this.pickField));
          fillPlain(firstNote.firstElementChild as ChipEl, "hv-chip-wide", say, f.trouble ?? "");
        }
      }
      return 0;
    }
    // What each block of chips will say, worked out before any of it is
    // written, and where each goes on a row a heading has cut. See
    // `chipplan.ts`, which holds the reasoning and the arithmetic.
    const planned = planRowChips({
      chips: f.byRow[r] ?? [],
      segs,
      rowStart,
      top: r === 0,
      noteWidth: this.noteWidth,
      maxLines: f.maxLines,
      measure: this.chipFonts ?? GUESS_TEXT,
      below: f.below,
      rowHeight: this.rowHeight,
      chipLine: this.sizes.chipLine,
    });
    if (planned.pinned !== null) f.pinned = planned.pinned;
    const trailer = f.more && r === this.rowEls.length - 1;
    const key = rowNoteKey(planned.blocks, trailer);
    if (key !== parts.noteKey) {
      parts.noteKey = key;
      for (const [j, b] of planned.blocks.entries()) {
        fillNote((parts.lines[j] as LineParts).note, b, false, trailer && j === segs.length - 1, this.pickField);
      }
    }
    return planned.extraHeight;
  }

  /** The strip over the top edge, filled in place: it is inside `.hv-rows`,
   *  which is where a touch drag is captured, so it is emptied and hidden
   *  rather than taken away. */
  private drawPinned(f: Frame): void {
    const pinnedKey = pinnedNoteKey(f.pinned);
    if (pinnedKey !== this.pinnedKey) {
      this.pinnedKey = pinnedKey;
      fillNote(this.pinned, f.pinned, true, false, this.pickField);
    }
  }

  /**
   * Everything the browser has to be asked, asked together: the widths and
   * the fonts the next layout is worked out from. One forced layout for the
   * lot, at the end of the draw, rather than one per row.
   *
   * `widened` says the rows were drawn against a column width that has since
   * changed, so the caller draws them again.
   */
  private measure(f: Frame): { widened: boolean; trackH: number } {
    const fields = f.fields;
    const below = f.below;
    const fonts = fields && this.chipFonts === null ? readChipFonts(this.rowsEl) : null;
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
      // Where the side column starts, so the pinned strip can sit over it
      // rather than over the whole row. Read in the same forced layout as the
      // width above.
      const noteLeft =
        noteEl === null ? 0 : noteEl.getBoundingClientRect().left - this.rowsEl.getBoundingClientRect().left;
      this.metrics = { noteWidth: w, noteLeft, trackH: this.track.clientHeight };
      // One redraw when the measured width first disagrees with the guess, so
      // the count of what did not fit is right rather than nearly right.
      if (fields && w > 0 && Math.abs(w - this.noteWidth) > 4) widened = true;
      if (fields && !below) this.noteWidth = w;
    }
    // Beside the bytes the strip stands over the column it replaces a line of,
    // not over the row: the bytes underneath stay readable, and the chips keep
    // the column's hairline and indent. Placed after the measurement above, so
    // the draw that first finds where the column is puts the strip there too.
    const side = fields && !below;
    if (this.pinned.classList.contains("hv-note-pinned-side") !== side)
      this.pinned.classList.toggle("hv-note-pinned-side", side);
    const pinLeft = side ? `${this.metrics.noteLeft}px` : "";
    if (this.pinned.style.left !== pinLeft) this.pinned.style.left = pinLeft;
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
    return { widened, trackH: this.metrics.trackH };
  }

  /**
   * Take what the rows actually came out at into the ledger, and say whether
   * the top row had to move for it.
   *
   * The view does not move for it: a row that turned out taller or shorter
   * than expected changes the total, and the thumb may shift a pixel for that,
   * but the bytes the reader is looking at stay where they are. Every row: a
   * row is drawn the same height wherever it falls, now that the fields
   * carried down from above the view are named by the strip pinned over the
   * rows rather than inside the top one, beside the bytes as well as below
   * them.
   */
  private settleHeights(real: readonly number[]): boolean {
    for (const [i, h] of real.entries()) {
      if (h > 0) this.ledger.measure(this.topRow + i, h);
    }
    this.ledger.trim(this.topRow);

    // The top row may now be shorter than the offset into it, which is the same
    // place in the file said a different way. Saying it the other way costs a
    // second draw, since the row elements would be standing for the wrong rows.
    let moved = false;
    while (this.topPx >= this.ledger.heightOf(this.topRow) && this.topRow + 1 < this.totalRows) {
      this.topPx -= this.ledger.heightOf(this.topRow);
      this.topRow++;
      moved = true;
    }
    if (this.topRow + 1 >= this.totalRows) this.topPx = Math.min(this.topPx, this.ledger.heightOf(this.topRow));
    return moved;
  }

  /** Put the rows at the scroll position, size the scrollbar thumb, and say
   *  what stretch of the file is on screen. */
  private finish(real: readonly number[], trackH: number, f: Frame): void {
    this.rowsInner.style.transform = this.topPx === 0 ? "" : `translateY(${-this.topPx}px)`;

    // Which rows are on screen, by the heights measured above: a row whose
    // top is inside the view is on screen, whatever of it is cut off below.
    // The first one starts above the edge by however much of it is hidden.
    let onScreen = 0;
    let y = -this.topPx;
    for (const h of real) {
      if (h === 0) break;
      if (y < this.viewH) onScreen++;
      y += h;
    }

    // Scrollbar thumb: as long a share of the track as the screen is of the
    // file, and as far down it as the view is through the file.
    const total = Math.max(1, this.ledger.totalHeight());
    const thumbH = Math.min(trackH, Math.max(24, Math.round((this.viewH / total) * trackH)));
    const limit = this.maxScrollY;
    const top = limit === 0 ? 0 : Math.round((Math.min(this.scrollY, limit) / limit) * (trackH - thumbH));
    this.thumb.style.height = `${thumbH}px`;
    this.thumb.style.transform = `translateY(${top}px)`;
    this.onViewport({ startBit: f.start * 8, endBit: Math.min(f.len, f.start + onScreen * f.bpr) * 8 });
  }

  render(): void {
    // A move draws once, when it has finished moving.
    if (this.settling) return;
    this.fitRows();
    const f = this.frame();
    this.drawHeader(f);
    this.fitParts(f.bpr, f.binary, f.showText, f.fields, f.below);
    // What each row will be tall once the browser has laid it out, from what
    // was put in it: its lines of chips and the headings above it. Zero for a
    // row past the end of the file.
    const heights: number[] = [];
    for (let r = 0; r < this.rowEls.length; r++) {
      const h = this.drawRow(r, f);
      if (h !== null) heights.push(h);
    }
    this.drawPinned(f);
    const { widened, trackH } = this.measure(f);
    if (widened) {
      // Those rows were drawn against a column width that has just changed, so
      // how tall they came out says nothing about how tall they will be.
      this.ledger.clearMeasured();
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
    if (this.settleHeights(real) && this.renormPasses < 3) {
      this.renormPasses++;
      this.render();
      this.renormPasses--;
      return;
    }
    this.finish(real, trackH, f);
  }
}

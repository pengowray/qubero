// The file as the text it is.
//
// The hex grid answers "what is at this address" and the listing answers "how
// is this file put together". Neither reads a file the way it was written to
// be read, and plenty of files were: a log, a manifest, a terminal captured to
// disk, a hex dump somebody pasted. This is the view for those.
//
// It scrolls by line, over an index of where the file's lines start that is
// built in the background from the front of the file. Where the index has
// reached, a scrollbar position is a line number and a line number is a byte,
// both exactly; past it, the position is an estimate from the average line so
// far, and the estimate is replaced rather than nudged as the index grows.
// The gutter carries each line's offset, and its line number once the index
// can say what that is. See `lineindex.ts`.
//
// The lines themselves are kept too, in a cache keyed by the byte each starts
// at, filled a few hundred at a time ahead of and behind the screen. Scrolling
// back over text already read asks the file for nothing.
//
// Three things a text file does not say about itself are shown rather than
// smoothed over, because in a hex editor they are the interesting part:
// which line ending each line used, which bytes did not fit the encoding, and
// where the escape sequences are. A capture of a coloured terminal is not
// noise to be hidden; it is what is in the file.

import { formatOffset } from "./doc.js";
import {
  CODEPAGE_A_DEFAULT,
  CODEPAGE_A_KEY,
  CODEPAGE_B_DEFAULT,
  CODEPAGE_B_KEY,
  CODEPAGES_A,
  CODEPAGES_B,
  storedChoice,
} from "./encodings.js";
import type { Doc, TextLine, TextReading } from "./doc.js";
import { el } from "./dom.js";
import { RowHeights } from "./rowheights.js";
import { LineIndex } from "./lineindex.js";
import type { Endings } from "./lineindex.js";
import { moveRows, needsPaint, paintWindow } from "./paintwindow.js";
import { TEXTVIEW } from "./strings.js";

/** Height of one row, which must match `--tv-row` in the stylesheet: rows are
 *  placed by arithmetic on it. */
const ROW = 20;
/** How tall the scrolling canvas is allowed to get. Browsers stop honouring an
 *  element's height past a few tens of millions of pixels, so past this the
 *  canvas is scaled down and a pixel of scrollbar is worth more than a row.
 *  Firefox's limit is the lowest, about 17.9 million, and a canvas past it
 *  draws nothing at all rather than less. */
const MAX_CANVAS = 16_000_000;
/** Wrapped text includes the entire bounded chunk returned by the core. */
const MAX_CHARS = 4096;
/** How much of a selection one copy carries. A selection can be the whole
 *  file, and the clipboard is not where a gigabyte belongs. */
const COPY_LIMIT = 1 << 20;
/** Lines read in one go, so a screenful is one trip through the core and the
 *  next screenful in either direction is already in hand when it is wanted. */
const BATCH = 300;
/** Decoded lines kept. Enough that scrolling around inside a large log never
 *  asks twice, and bounded so that a day of scrolling does not grow forever. */
const CACHE_LINES = 50_000;
/** Bound decoded text as well as line count. Long lines otherwise retain
 * hundreds of megabytes, plus their lazily built character/byte maps. */
const CACHE_CHARS = 2 * 1024 * 1024;
/** Small scans yield between reads so background work cannot monopolise UI. */
const INDEX_STEP = 256 * 1024;
/** Files no larger than this are indexed to their end. A multi-gigabyte image
 *  should not be read cover to cover merely because its text tab was opened. */
const FULL_INDEX_LIMIT = 16 * 1024 * 1024;
/** Enough of a giant file to establish a useful line-length estimate. Jumps
 *  elsewhere leave local index segments, so nearby movement stays exact. */
const GIANT_INDEX_HEAD = 1024 * 1024;
/** The longest a line is before the core cuts it, which must match `MAX_LINE`
 *  in `crates/core/src/textview.rs`. */
const MAX_LINE = 4096;
/** How much is indexed around a jump to somewhere the background pass has not
 *  reached, so that scrolling on from there is answered locally. */
const PROBE = 1 * 1024 * 1024;

/** Line endings named the way the core names them. */
const ENDINGS = new Set(["LF", "CRLF", "CR", "no ending", "cut"]);

export class TextView {
  readonly el: HTMLElement;
  private readonly gutter: HTMLElement;
  private readonly rows: HTMLElement;
  private readonly view: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly scroll: HTMLElement;

  /** Byte offset of the first drawn line. */
  private top = 0;
  private wrap: "off" | "line" | "word" = "off";
  private readonly heights = new RowHeights();
  private layoutWidth = 0;
  private clipChars = MAX_CHARS;
  /** Native scrollTop changes before the scroll event/RAF is delivered.
   * Keep the last mapping we wrote so layout cannot overwrite that input. */
  private wrappedScrollTop = 0;
  private wrappedScrollSpan = 1;
  private wrappedPixelSpan = 1;
  private scrollRevision = 0;
  private requestedByte: number | null = null;

  private get displayLines(): number {
    return Math.max(this.index.totalLines, this.topLine + this.lines.length);
  }

  private captureWrappedScroll(): void {
    if (this.wrap === "off" || Math.abs(this.scroll.scrollTop - this.wrappedScrollTop) < 0.5) return;
    const y = this.scroll.scrollTop / this.wrappedPixelSpan * this.wrappedScrollSpan;
    const pos = this.heights.rowAtY(y);
    this.viewLine = pos.row;
    this.viewOffset = pos.offsetPx;
    this.wrappedScrollTop = this.scroll.scrollTop;
    this.scrollRevision++;
    if (this.drawing) this.pending = true;
  }

  async setWrap(mode: "off" | "line" | "word"): Promise<void> {
    this.captureWrappedScroll();
    this.wrap = mode;
    this.wrappedScrollTop = this.scroll.scrollTop;
    this.el.dataset.wrap = mode;
    this.heights.clearMeasured();
    this.rowCache.clear();
    this.viewOffset = 0;
    await this.draw();
    if (mode === "off") {
      this.scroll.scrollTop = this.lineY(this.viewLine);
      this.placeBlock();
    } else this.syncWrappedPosition();
  }

  private wrappedSpan(): number {
    this.heights.setRows(this.displayLines);
    return Math.max(1, this.heights.totalHeight() - this.scroll.clientHeight);
  }

  private syncWrappedPosition(): void {
    if (this.wrap === "off") return;
    this.captureWrappedScroll();
    this.canvasWas = this.canvasHeight();
    this.canvas.style.height = `${this.canvasWas}px`;
    const y = Math.min(this.wrappedSpan(), this.heights.heightBefore(this.viewLine) + this.viewOffset);
    this.scroll.scrollTop = y / this.wrappedSpan() * this.span();
    this.wrappedScrollTop = this.scroll.scrollTop;
    this.wrappedScrollSpan = this.wrappedSpan();
    this.wrappedPixelSpan = this.span();
    this.placeBlock();
  }

  private scrollWrapped(pixels: number): void {
    this.captureWrappedScroll();
    const y = Math.max(0, Math.min(this.wrappedSpan(), this.heights.heightBefore(this.viewLine) + this.viewOffset + pixels));
    const pos = this.heights.rowAtY(y);
    this.viewLine = pos.row;
    this.viewOffset = pos.offsetPx;
    this.syncWrappedPosition();
    if (this.needsPaint(this.viewLine)) void this.draw();
  }
  /** Line number of the first drawn line, and of the first line inside the
   *  viewport. They differ by the overscan, except at the front of the file. */
  private topLine = 0;
  private viewLine = 0;
  /** What is drawn now. */
  private lines: readonly TextLine[] = [];
  private reading: TextReading = { encoding: "UTF-8", mark: 0, guessed: true, unit: 1 };
  /** An encoding the reader chose, or "" to let the file decide. */
  private chosen = "";
  /** The byte the cursor is on, so the character holding it can be marked. */
  private cursor = 0;
  /** The ending the file mostly uses, decided from the index and not from the
   *  screen, so a line using another can be marked as the odd one out and the
   *  mark does not change under a scroll. */
  private usualEnding = "";
  private drawing = false;
  private pending = false;
  /** Whether the reading has been asked of the file at all: the one above is a
   *  stand-in until the first draw, and is held rather than asked again after,
   *  since settling it is a trip through the core on every redraw otherwise. */
  private readingSettled = false;
  /** True while a movement key is extending a selection rather than moving
   *  away from one. */
  private extending = false;

  /** Where the file's lines start. */
  private index: LineIndex;
  /** Lines already decoded, by the byte each starts at. A Map is its own
   *  least-recently-used list: re-reading a line moves it to the back. */
  private cache = new Map<number, TextLine>();
  private cacheChars = 0;
  /** How many times the core has been asked for lines, which is the number the
   *  whole of this is about. Read by the tests and by hand in the console. */
  fetches = 0;

  /** Called when the reader picks a character, with the byte it starts at. */
  onPick: (at: number) => void = () => {};
  /** Called when the reading is settled, so the toolbar can say what it is,
   *  with the line endings counted over everything indexed so far. */
  onReading: (r: TextReading, endings: Endings) => void = () => {};
  /** Called when the file changed, so the rest of the page catches up. */
  onEdit: () => void = () => {};
  /** Called when the encoding has no room for a character that was typed. */
  onRefuse: (char: string, encoding: string) => void = () => {};
  /** Called with something to tell the reader, which the toolbar shows. */
  onMessage: (text: string) => void = () => {};
  /** Called when the reader selects a stretch, in bytes. An empty stretch
   *  clears the selection, so one callback covers both. */
  onSelect: (startByte: number, endByte: number, caretByte: number) => void = () => {};

  /** The selected bytes, as the rest of the app has them. Held rather than
   *  owned: the hex view is where a selection lives, and this renders what it
   *  says and writes back through it, so the two can never drift apart. */
  private selection: { start: number; end: number } | null = null;
  /** Where a keyboard or pointer selection is being extended from. */
  private anchor: number | null = null;
  private dragging = false;

  constructor(private doc: Doc) {
    this.index = new LineIndex(doc.lengthBytes);
    this.gutter = el("div", { className: "tv-gutter" });
    this.rows = el("div", { className: "tv-rows" });
    this.view = el("div", { className: "tv-view" }, this.gutter, this.rows);
    this.canvas = el("div", { className: "tv-canvas" }, this.view);
    this.scroll = el("div", { className: "tv-scroll", tabIndex: 0 }, this.canvas);
    this.scroll.setAttribute("role", "region");
    this.scroll.setAttribute("aria-label", TEXTVIEW.regionLabel);
    this.el = el("div", { className: "textview" }, this.scroll);

    this.scroll.addEventListener("scroll", () => this.onScroll(), { passive: true });
    this.scroll.addEventListener("wheel", (e) => this.onWheel(e), { passive: false });
    this.scroll.addEventListener("keydown", (e) => this.onKey(e));
    this.rows.addEventListener("pointerdown", (e) => this.onPointerDown(e));
    this.rows.addEventListener("pointermove", (e) => this.onPointerMove(e));
    this.rows.addEventListener("click", (e) => this.onClick(e));
    const stop = (): void => {
      this.dragging = false;
    };
    window.addEventListener("pointerup", stop);
    // A finger that turned out to be a scroll leaves no pointerup, and without
    // this the view would go on treating every move as a drag.
    window.addEventListener("pointercancel", stop);
    new ResizeObserver(() => {
      void this.draw();
    }).observe(this.scroll);
  }

  /** How many rows fit. */
  private visible(): number {
    return Math.max(1, Math.floor(this.scroll.clientHeight / ROW));
  }

  /** The encoding in use, or "" when the file decided. */
  get encoding(): string {
    return this.chosen;
  }

  /** Read the file as this encoding instead. "" hands it back to the file.
   *  Where the lines are is a fact about a reading of the file rather than
   *  about the file, so the index and the decoded lines both go. */
  async setEncoding(name: string): Promise<void> {
    this.chosen = name;
    this.forget();
    await this.draw(true);
  }

  /** Throw away everything read about the file's text. */
  private forget(): void {
    this.index = new LineIndex(this.doc.lengthBytes, this.reading.mark);
    this.cache.clear();
    this.cacheChars = 0;
    this.heights.clearMeasured();
    this.usualEnding = "";
    this.anchorLine = 0;
    this.anchorAt = 0;
    this.startIndexing();
  }

  relayout(): void {
    void this.draw(true);
  }

  // ---- the index ------------------------------------------------------

  private idle = 0;

  /** Where the next background scan may begin. Giant files keep a measured
   *  head and local segments instead of turning opening Text into a full-file
   *  read. */
  private indexFrom(): number | null {
    const from = this.index.gap;
    const stop = this.doc.lengthBytes <= FULL_INDEX_LIMIT ? this.doc.lengthBytes : GIANT_INDEX_HEAD;
    return from !== null && from < stop ? from : null;
  }

  /** Keep the index growing from wherever it has reached, in the time the
   *  browser has nothing else to do with. */
  private startIndexing(): void {
    if (this.idle !== 0 || this.indexFrom() === null) return;
    // Idle time, but with a deadline: a browser hands a tab nobody is looking
    // at no idle time at all, and a file opened in a tab left in the
    // background should still be indexed by the time it is looked at.
    const soon = (f: () => void): number =>
      typeof requestIdleCallback === "function" ? requestIdleCallback(() => f(), { timeout: 250 }) : window.setTimeout(f, 16);
    this.idle = soon(() => {
      this.idle = 0;
      void this.indexPass().then(() => {
        if (this.indexFrom() !== null) this.startIndexing();
      });
    });
  }

  /** As much of the index as one turn of idle time is worth. A pass is nearly
   *  all waiting for the file rather than scanning it, so stopping after one
   *  step would leave the reading of a hundred megabytes paced by the
   *  scheduler rather than by the disk. */
  private async indexPass(): Promise<void> {
    const until = performance.now() + 8;
    do {
      await this.indexStep();
    } while (this.indexFrom() !== null && performance.now() < until);
  }

  /** One pass of the background index. */
  private async indexStep(): Promise<void> {
    const from = this.indexFrom();
    if (from === null) return;
    const got = await this.doc.textIndex(this.chosen, from, from + INDEX_STEP);
    if (got.starts.length === 0) return;
    this.captureWrappedScroll();
    this.index.add(got.starts, got.next, { lf: got.lf, cr: got.cr, crlf: got.crlf });
    this.settleEnding();
    // top/topLine are being resolved across awaits. They do not describe
    // the painted rows until the draw commits; correcting them now can turn
    // an old window into the anchor for a newer scrollbar jump.
    if (this.drawing) {
      this.pending = true;
      return;
    }
    // The line on screen was a line number the index had to guess at, and the
    // index has now counted its way to it. Take the correction on the line
    // number and the scrollbar together: the canvas is as tall as the file has
    // lines, and the file has just turned out to have a different number of
    // them, so both ends of the mapping moved. Nothing moves on screen.
    const known = this.index.lineAt(this.top);
    if (known !== null && known !== this.topLine) {
      this.heights.clearMeasured();
      this.viewLine += known - this.topLine;
      this.topLine = known;
      this.anchorLine = known;
      this.anchorAt = this.top;
      // The gutter can say which lines these are now, which it could not a
      // moment ago.
      this.render();
    }
    this.reanchor();
    this.onReading(this.reading, this.index.endings);
  }

  /** Take in the line starts a window of text just told us, so that reading
   *  the text and knowing where it is are never two separate trips. */
  private note(lines: readonly TextLine[], next: number): void {
    if (lines.length === 0) return;
    const starts = new Float64Array(lines.length);
    let lf = 0;
    let cr = 0;
    let crlf = 0;
    for (let i = 0; i < lines.length; i++) {
      const l = lines[i];
      if (l === undefined) continue;
      starts[i] = l.at;
      if (l.ending === "LF") lf++;
      else if (l.ending === "CR") cr++;
      else if (l.ending === "CRLF") crlf++;
    }
    this.index.add(starts, next, { lf, cr, crlf });
  }

  /** Which ending the file uses, from the index rather than from the screen.
   *  Nothing is marked while the index has seen no endings at all. */
  private settleEnding(): void {
    const e = this.index.endings;
    const most = Math.max(e.lf, e.crlf, e.cr);
    this.usualEnding = most === 0 ? "" : e.lf === most ? "LF" : e.crlf === most ? "CRLF" : "CR";
  }

  // ---- the canvas -----------------------------------------------------

  /** How tall the canvas is: one row per line the file has, or per line it is
   *  estimated to have, scaled down where that is taller than a browser will
   *  honour. */
  private canvasHeight(): number {
    this.heights.setRows(this.displayLines);
    const want = this.wrap === "off" ? this.displayLines * ROW : this.heights.totalHeight();
    return Math.max(this.scroll.clientHeight + 1, Math.min(MAX_CANVAS, want));
  }

  /** How far the scrollbar travels. */
  private span(height = this.canvasHeight()): number {
    return Math.max(1, height - this.scroll.clientHeight);
  }

  /** The last line the viewport can start on, which is what the bottom of the
   *  scrollbar has to mean. Mapping through this rather than through pixels
   *  per line is what keeps the end of the file reachable once the canvas has
   *  been scaled down: a screenful is worth more than a screen of canvas then,
   *  and the last screenful would otherwise sit past the scrollbar's floor. */
  private lastTop(): number {
    return Math.max(1, this.displayLines - this.visible());
  }

  /** Where a line sits on the scrollbar. */
  private lineY(n: number, height = this.canvasHeight()): number {
    if (this.wrap !== "off") return Math.min(1, this.heights.heightBefore(n) / this.wrappedSpan()) * this.span(height);
    return Math.round(Math.min(1, n / this.lastTop()) * this.span(height));
  }

  /** The line the scrollbar is pointing at. */
  private lineAtY(y: number): number {
    if (this.wrap !== "off") return this.heights.rowAtY(y / this.span() * this.wrappedSpan()).row;
    return Math.max(0, Math.round((y / this.span()) * this.lastTop()));
  }

  /** Put the scrollbar on the line the reader is looking at, and the block of
   *  drawn rows with it, so that nothing moves on screen. This is the whole of
   *  what used to be a drifting estimate nudged back into place when the
   *  scrolling stopped: the index says where the line is, and the only reason
   *  to move anything is that the index learned something. */
  private reanchor(): void {
    // Nothing moved unless the canvas is a different height than the last
    // time it was set: the mapping from line to pixel only changes with it.
    // Touching the scrollbar for no reason was what made the text flicker: a
    // wheel in flight sits a fraction of a row past the line, and putting the
    // scrollbar back on the line every index pass threw that fraction away,
    // several times a second, under the reader's hand.
    if (this.wrap !== "off") { this.syncWrappedPosition(); return; }
    const height = this.canvasHeight();
    const was = this.canvasWas;
    this.canvasWas = height;
    if (was === height) return;
    // The fraction of a row the view is past the line, kept across the
    // change in mapping so the text on screen stays exactly where it is.
    const frac = was === -1 ? 0 : this.scroll.scrollTop - this.lineY(this.viewLine, was);
    this.canvas.style.height = `${height}px`;
    const y = Math.max(0, Math.min(this.lineY(this.viewLine) + frac, height - this.scroll.clientHeight));
    if (Math.abs(this.scroll.scrollTop - y) >= 1) this.scroll.scrollTop = y;
    this.placeBlock();
  }

  /** The canvas height last written, so a reanchor can tell whether anything
   *  changed and what the old mapping was. */
  private canvasWas = -1;

  /** Put the block of drawn rows where the line it starts with belongs.
   *
   *  The rows sit inside the scrolled canvas rather than being moved to meet
   *  it, which is the whole of the flicker fix: a transform recomputed from a
   *  scroll event runs on the main thread while a flick runs on the
   *  compositor, so the two are never in step and the text slides against the
   *  scrollport and snaps back. On an uncapped canvas this placement is
   *  constant while the viewport moves, so the compositor carries the rows by
   *  itself. On a capped canvas it also accounts for compressed scrollbar
   *  pixels: one scrollbar pixel can cross several full-height rows there. */
  private placeBlock(): void {
    const y = this.wrap === "off"
      ? this.lineY(this.viewLine) - (this.viewLine - this.topLine) * ROW - this.viewOffset
      : this.scroll.scrollTop - this.viewOffset - (this.heights.heightBefore(this.viewLine) - this.heights.heightBefore(this.topLine));
    this.view.style.transform = `translateY(${y}px)`;
  }

  private scrollFrame = 0;

  /** Runway on either side of the rows currently in the DOM. */
  private paintRunway = 0;
  /** Pixels already scrolled into `viewLine` when a capped canvas is moved by
   *  wheel rows rather than by its heavily compressed native pixels. */
  private viewOffset = 0;

  /** Whether the browser-height ceiling has compressed rows on the canvas. */
  private compressed(): boolean {
    return this.index.totalLines * ROW > MAX_CANVAS;
  }

  /** Whether scrolling has come near enough to a painted edge to refill it. */
  private needsPaint(line: number): boolean {
    if (this.lines.length === 0) return true;
    const last = this.lines[this.lines.length - 1];
    const atEnd = last !== undefined && last.at + last.len >= this.doc.lengthBytes;
    if (this.wrap !== "off") {
      const y = this.heights.heightBefore(line) + this.viewOffset;
      const above = y - this.heights.heightBefore(this.topLine);
      const below = this.heights.heightBefore(this.topLine + this.lines.length) - y - this.scroll.clientHeight;
      return above < 0 || (!atEnd && below < this.scroll.clientHeight)
        || (this.topLine > 0 && above < this.scroll.clientHeight);
    }
    return needsPaint(
      line,
      this.visible(),
      this.topLine,
      this.lines.length,
      this.paintRunway,
      this.topLine === 0,
      atEnd,
    );
  }

  private onScroll(): void {
    this.captureWrappedScroll();
    // The compositor scrolls through rows already in the DOM. JavaScript only
    // recentres that painted window when half its runway has been consumed;
    // ordinary half-page and page movements therefore need no draw at all.
    if (this.scrollFrame !== 0) return;
    this.scrollFrame = requestAnimationFrame(() => {
      this.scrollFrame = 0;
      if (this.wrap !== "off") {
        this.captureWrappedScroll();
        this.placeBlock();
        if (this.needsPaint(this.viewLine)) void this.draw();
        return;
      }
      const want = this.lineAtY(this.scroll.scrollTop);
      if (want === this.viewLine) return;
      this.viewLine = want;
      this.viewOffset = 0;
      // This is the same transform on an ordinary canvas. Once the canvas is
      // capped it keeps the full-height rows aligned with a compressed
      // scrollbar without replacing any of them.
      this.placeBlock();
      if (this.needsPaint(want)) void this.draw();
    });
  }

  /**
   * A native wheel over a capped canvas is not a text scroll: a few hundred
   * pixels can mean thousands or millions of lines and instantly outrun any
   * useful painted window. Move by the text's real row height instead. The
   * native scrollbar remains globally mapped, so dragging its thumb is still
   * a deliberate jump to anywhere in the file.
   */
  private onWheel(e: WheelEvent): void {
    if (e.ctrlKey || e.metaKey) return;
    if (this.wrap !== "off") {
      if (this.heights.totalHeight() <= MAX_CANVAS || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
      e.preventDefault();
      this.scrollWrapped(e.deltaY * (e.deltaMode === 1 ? ROW : e.deltaMode === 2 ? this.scroll.clientHeight : 1));
      return;
    }
    if (!this.compressed() || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    e.preventDefault();
    const unit = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? ROW : e.deltaMode === WheelEvent.DOM_DELTA_PAGE ? this.scroll.clientHeight : 1;
    const next = moveRows(this.viewLine, this.viewOffset, e.deltaY * unit, ROW, this.lastTop());
    if (next.line === this.viewLine && next.offset === this.viewOffset) return;
    this.viewLine = next.line;
    this.viewOffset = next.offset;
    this.scroll.scrollTop = this.lineY(this.viewLine);
    this.placeBlock();
    if (this.needsPaint(this.viewLine)) void this.draw();
  }

  /** Go to a line, scrollbar and all. `render` also refreshes row decoration,
   *  used when arriving there changed the caret rather than only the scroll. */
  private async goto(n: number, render = false): Promise<void> {
    this.captureWrappedScroll();
    this.viewLine = Math.max(0, Math.min(n, Math.max(0, this.index.totalLines - 1)));
    this.viewOffset = 0;
    const repaint = this.needsPaint(this.viewLine);
    if (this.wrap !== "off") this.syncWrappedPosition();
    else this.scroll.scrollTop = Math.max(0, Math.min(this.lineY(this.viewLine), this.canvasHeight() - this.scroll.clientHeight));
    this.placeBlock();
    if (repaint || render) await this.draw();
  }

  /** Where the first drawn line starts, for the line the viewport is on.
   *
   *  Inside the index this is a lookup and cannot be wrong. Past it, the line
   *  number is an estimate, so the byte it works out to is walked back to a
   *  real line start and the file around it is indexed, which is what makes
   *  scrolling on from there exact even though arriving was a guess. */
  private async topByte(first: number): Promise<number> {
    this.topLine = first;
    const known = this.index.byteOfLine(first);
    if (known !== null) return this.anchoredAt(first, Math.min(known, this.doc.lengthBytes));
    // Past the index's reach, but perhaps inside a segment a jump left behind.
    // A segment cannot say what line number it is at, but it can say what the
    // line after this one is, and stepping through it is what makes scrolling
    // in un-indexed ground exact rather than a guess per row.
    const near = this.index.place(this.anchorAt);
    if (near !== null && near.segment.firstLine === null) {
      const step = near.segment.starts[near.index + (first - this.anchorLine)];
      if (step !== undefined) return this.anchoredAt(first, step);
    }
    const b = await this.doc.textBack(this.chosen, this.index.guessByteOfLine(first), 0);
    const got = await this.doc.textIndex(this.chosen, b.start, b.start + PROBE);
    if (got.starts.length > 0) this.index.add(got.starts, got.next, { lf: got.lf, cr: got.cr, crlf: got.crlf });
    return this.anchoredAt(first, b.start);
  }

  /** The line the top of the screen was last resolved to, so that the next row
   *  along can be stepped to rather than guessed at. */
  private anchorLine = 0;
  private anchorAt = 0;

  private anchoredAt(line: number, at: number): number {
    this.anchorLine = line;
    this.anchorAt = at;
    return at;
  }

  // ---- the lines ------------------------------------------------------

  /** The next `want` lines from `at`, out of the cache where they are in it
   *  and out of the file in batches where they are not. */
  private async linesFrom(at: number, want: number): Promise<TextLine[]> {
    const out: TextLine[] = [];
    let here = at;
    while (out.length < want && here < this.doc.lengthBytes) {
      const hit = this.cache.get(here);
      if (hit !== undefined) {
        // Touched, so the least recently used is the one that goes.
        this.cache.delete(here);
        this.cache.set(here, hit);
        out.push(hit);
        here += hit.len;
        if (hit.len === 0) break;
        continue;
      }
      const w = await this.doc.textWindow(this.chosen, here, Math.max(want - out.length, BATCH));
      this.fetches++;
      if (w.lines.length === 0) break;
      this.note(w.lines, w.next);
      for (const l of w.lines) this.keep(l);
      for (const l of w.lines) {
        if (out.length >= want) break;
        out.push(l);
        here = l.at + l.len;
      }
    }
    return out;
  }

  private keep(line: TextLine): void {
    this.cacheChars -= this.cache.get(line.at)?.text.length ?? 0;
    this.cache.delete(line.at);
    this.cache.set(line.at, line);
    this.cacheChars += line.text.length;
    while (this.cache.size > CACHE_LINES || this.cacheChars > CACHE_CHARS) {
      const oldest = this.cache.keys().next();
      if (oldest.done === true) break;
      this.cacheChars -= this.cache.get(oldest.value)?.text.length ?? 0;
      this.cache.delete(oldest.value);
    }
  }

  /** Read the lines just past the screen, so that a scroll of a page in either
   *  direction is answered from the cache. */
  private prefetch(): void {
    const last = this.lines[this.lines.length - 1];
    if (last === undefined) return;
    const ahead = last.at + last.len;
    if (ahead < this.doc.lengthBytes && !this.cache.has(ahead)) void this.linesFrom(ahead, BATCH);
    const first = this.lines[0];
    if (first === undefined || first.at <= 0) return;
    // The line above is in the index nearly always, and asking the core to
    // walk back to it would be a trip through the file on every draw. Where it
    // is already decoded there is nothing behind the screen left to read.
    const above = this.index.place(first.at - 1)?.at;
    if (above !== undefined && this.cache.has(above)) return;
    void this.doc.textBack(this.chosen, first.at, BATCH).then(async (b) => {
      if (b.back < first.at && !this.cache.has(b.back)) await this.linesFrom(b.back, BATCH);
    });
  }

  /** Draw what fits, reading only that. */
  async draw(force = false): Promise<void> {
    if (this.drawing) {
      this.pending = true;
      return;
    }
    this.drawing = true;
    try {
      do {
        this.pending = false;
        this.captureWrappedScroll();
        const revision = this.scrollRevision;
        const requested = this.requestedByte;
        if (force || !this.readingSettled || this.reading.encoding === "") {
          const was = this.reading.mark;
          this.reading = await this.doc.textReading(this.chosen);
          this.readingSettled = true;
          if (this.reading.mark !== was) this.index = new LineIndex(this.doc.lengthBytes, this.reading.mark);
        }
        this.index.setLength(this.doc.lengthBytes);
        this.startIndexing();
        const window = this.wrap === "off" ? paintWindow(this.viewLine, this.visible()) : {
          first: this.heights.rowAtY(Math.max(0, this.heights.heightBefore(this.viewLine) - this.scroll.clientHeight * 2)).row,
          count: this.visible() * 5,
          runway: this.visible() * 2,
        };
        const want = window.count;
        this.paintRunway = window.runway;
        if (requested !== null) {
          // Explicit byte navigation is exact, even where scrollbar positions
          // are estimates. Resolve a bounded window around the requested byte.
          const back = await this.doc.textBack(this.chosen, requested, Math.floor(want / 2));
          this.top = back.back;
          this.topLine = this.index.lineAt(this.top) ?? this.index.guessLineAt(this.top);
          this.anchoredAt(this.topLine, this.top);
        } else this.top = await this.topByte(window.first);
        let got = await this.linesFrom(this.top, want + (requested === null ? 0 : 2));
        // A screen that came up short ran out of file. Back up so that the
        // last screenful of a file is a full screen rather than one line at
        // the bottom of an empty one.
        if (requested === null && got.length < want && this.top > 0) {
          const oldTop = this.top;
          const b = await this.doc.textBack(this.chosen, this.top, want - got.length);
          if (b.back < this.top) {
            this.top = b.back;
            got = await this.linesFrom(b.back, want);
            const backed = got.findIndex((line) => line.at === oldTop);
            this.topLine = this.index.lineAt(b.back) ?? Math.max(0, this.topLine - Math.max(0, backed));
            // An estimated jump may have named lines beyond the real end.
            // Clamp it to the last full viewport without pulling a valid last
            // screen upward merely because its lower runway met the file end.
            if (this.wrap === "off") this.viewLine = Math.min(this.viewLine, this.topLine + Math.max(0, got.length - this.visible()));
          }
        }
        this.captureWrappedScroll();
        if (revision !== this.scrollRevision || requested !== this.requestedByte) {
          this.pending = true;
          continue;
        }
        const last = got[got.length - 1];
        const eof = last === undefined ? this.doc.lengthBytes === 0 : last.at + last.len === this.doc.lengthBytes;
        if (eof && (last === undefined || this.textEnd(last) < this.doc.lengthBytes)) {
          got.push({ at: this.doc.lengthBytes, len: 0, ending: "no ending", text: "", escapes: [], lossy: false });
        }
        const toEnd = requested === this.doc.lengthBytes && requested !== null;
        if (toEnd) {
          // Detached tail segments have exact bytes but estimated row numbers.
          // Attach their last row to the end of the estimated scrollbar.
          this.topLine = this.index.lineAt(this.top) ?? Math.max(0, this.index.totalLines - got.length);
          this.anchoredAt(this.topLine, this.top);
        }
        this.lines = got;
        if (requested !== null) {
          const i = got.findIndex(l => requested >= l.at && requested <= this.textEnd(l));
          if (i >= 0) this.viewLine = Math.max(this.topLine, this.topLine + i - Math.floor(this.visible() / 3));
          this.viewOffset = 0;
          this.requestedByte = null;
        }
        if (this.usualEnding === "") this.settleEnding();
        if (this.wrap === "off") {
          this.canvasWas = this.canvasHeight();
          this.canvas.style.height = `${this.canvasWas}px`;
          if (requested !== null) this.scroll.scrollTop = this.lineY(this.viewLine);
        }
        this.placeBlock();
        this.render();
        if (toEnd) {
          if (this.wrap === "off") {
            this.viewLine = Math.max(0, this.topLine + got.length - this.visible());
            this.scroll.scrollTop = this.lineY(this.viewLine);
            this.placeBlock();
          } else {
            const y = Math.max(0, this.heights.heightBefore(this.topLine + got.length) - this.scroll.clientHeight);
            const pos = this.heights.rowAtY(y);
            this.viewLine = pos.row;
            this.viewOffset = pos.offsetPx;
            this.syncWrappedPosition();
          }
        }
        this.onReading(this.reading, this.index.endings);
        this.prefetch();
      } while (this.pending);
    } finally {
      this.drawing = false;
    }
  }

  /** Show the line holding this byte, and mark the character it falls in. */
  async setByte(at: number): Promise<void> {
    at = Math.max(0, Math.min(at, this.doc.lengthBytes));
    this.cursor = at;
    // Drawn is not the same as on screen: a caret in the overscan has to be
    // scrolled to like any other.
    const i = this.lines.findIndex((l) => at >= l.at && (at < l.at + l.len || at === this.doc.lengthBytes && at === this.textEnd(l)));
    const onScreen = this.wrap === "off"
      ? i >= this.viewLine - this.topLine && i < this.viewLine - this.topLine + this.visible()
      : i >= 0 && this.heights.heightBefore(this.topLine + i + 1) > this.heights.heightBefore(this.viewLine) + this.viewOffset
        && this.heights.heightBefore(this.topLine + i) < this.heights.heightBefore(this.viewLine) + this.viewOffset + this.scroll.clientHeight;
    if (!onScreen) {
      this.requestedByte = at;
      await this.draw();
      this.revealCaret();
      return;
    }
    await this.draw();
    this.revealCaret();
  }

  private revealCaret(): void {
    if (this.wrap === "off") return;
    const caret = this.rows.querySelector<HTMLElement>(".is-cursor") ?? this.rows.querySelector<HTMLElement>(".tv-caret");
    if (caret === null) return;
    const rect = caret.getBoundingClientRect();
    const viewport = this.scroll.getBoundingClientRect();
    if (rect.top < viewport.top) this.scrollWrapped(rect.top - viewport.top);
    else if (rect.bottom > viewport.bottom) this.scrollWrapped(rect.bottom - viewport.bottom);
  }

  /** Show the selection the rest of the app is holding. */
  setSelection(startByte: number | null, endByte: number): void {
    this.selection = startByte === null || endByte <= startByte ? null : { start: startByte, end: endByte };
    if (this.selection === null) this.anchor = null;
    this.render();
  }

  /** Where a line's text stops, which is where its ending starts. */
  private textEnd(line: TextLine): number {
    const u = this.reading.unit;
    const ending = line.ending === "CRLF" ? 2 * u : line.ending === "LF" || line.ending === "CR" ? u : 0;
    return line.at + line.len - ending;
  }

  /** The line on screen holding the caret, and where it is in that list. */
  private caretLine(): { line: TextLine; index: number } | null {
    const index = this.lines.findIndex((l) => this.cursor >= l.at && this.cursor <= this.textEnd(l));
    const line = this.lines[index];
    return line === undefined ? null : { line, index };
  }

  private onKey(e: KeyboardEvent): void {
    const page = this.visible() - 1;
    const scroll = (n: number): void => {
      e.preventDefault();
      void this.scrollLines(n);
    };
    const move = (f: () => Promise<void>): void => {
      e.preventDefault();
      void f();
    };
    if (e.ctrlKey || e.metaKey) {
      if (e.key === "Home" || e.key === "End") {
        e.preventDefault();
        this.extending = e.shiftKey;
        if (e.shiftKey) this.anchor ??= this.cursor;
        else this.anchor = null;
        void this.moveFileEdge(e.key === "End" ? this.doc.lengthBytes : 0);
        return;
      }
      if (e.key === "c" || e.key === "C") {
        e.preventDefault();
        void this.copySelection();
      }
      return;
    }
    if (e.altKey) return;
    // Shift with a movement key extends a selection from wherever the caret
    // was when the first of them was pressed. Anything else drops it, which is
    // what every other text view does.
    const extending = e.shiftKey && SELECTING.has(e.key);
    if (extending && this.anchor === null) this.anchor = this.cursor;
    else if (!extending && !e.shiftKey) this.anchor = null;
    this.extending = extending;
    switch (e.key) {
      case "ArrowRight":
        return move(() => this.moveChar(1));
      case "ArrowLeft":
        return move(() => this.moveChar(-1));
      case "ArrowDown":
        return move(() => this.moveLine(1));
      case "ArrowUp":
        return move(() => this.moveLine(-1));
      case "PageDown":
        return scroll(page);
      case "PageUp":
        return scroll(-page);
      case "Home":
        return move(() => this.moveToLineEdge("start"));
      case "End":
        return move(() => this.moveToLineEdge("end"));
      case "Backspace":
        return move(() => this.erase(-1));
      case "Delete":
        return move(() => this.erase(1));
      case "Enter":
        return move(() => this.insert(endingText(this.usualEnding)));
      default:
    }
    // One printable character. A key name longer than a character is a key and
    // not something to type: "Shift", "F5", "Escape".
    if ([...e.key].length === 1) move(() => this.insert(e.key));
  }

  private async moveFileEdge(at: number): Promise<void> {
    // Always resolve the edge, including when the final logical line is
    // already drawn but its wrapped tail is below the viewport.
    this.requestedByte = at;
    this.cursor = at;
    await this.draw();
    if (this.extending) this.extendTo(at);
    else {
      this.selection = null;
      this.onPick(at);
    }
    this.render();
  }

  /** Move the caret a character left or right, following it onto the line
   *  above or below when it runs off the end of this one. */
  private async moveChar(dir: 1 | -1): Promise<void> {
    const here = this.caretLine();
    if (here === null) return;
    const cells = this.charsOf(here.line);
    if (dir === 1) {
      const next = cells.find((c) => c.at >= this.cursor);
      if (next !== undefined && next.at === this.cursor) return this.place(this.cursor + next.width);
      return this.place(here.line.at + here.line.len);
    }
    const prev = [...cells].reverse().find((c) => c.at < this.cursor);
    if (prev !== undefined) return this.place(prev.at);
    if (here.line.at === 0) return;
    const b = await this.doc.textBack(this.chosen, here.line.at - 1, 0);
    await this.place(b.start);
    const above = this.caretLine();
    if (above !== null) await this.place(this.textEnd(above.line));
  }

  /** Move the caret a line up or down, keeping the character it was on. */
  private async moveLine(dir: 1 | -1): Promise<void> {
    if (this.wrap !== "off") {
      const caret = this.rows.querySelector<HTMLElement>(".is-cursor") ?? this.rows.querySelector<HTMLElement>(".tv-caret");
      if (caret !== null) {
        const rect = caret.getBoundingClientRect();
        const port = this.scroll.getBoundingClientRect();
        let y = rect.top + 1 + dir * ROW;
        if (y < port.top || y >= port.bottom) {
          this.scrollWrapped(dir * ROW);
          y -= dir * ROW;
        }
        const at = this.byteUnder(new MouseEvent("mousemove", { clientX: rect.left + 1, clientY: y }));
        if (at !== null && at !== this.cursor) return this.place(at);
      }
    }
    const here = this.caretLine();
    if (here === null) return;
    const column = this.charsOf(here.line).filter((c) => c.at < this.cursor).length;
    const target = this.lines[here.index + dir];
    if (target === undefined) {
      await this.scrollLines(dir);
      const after = this.caretLine();
      if (after === null) return;
      const next = this.lines[after.index + dir];
      if (next === undefined) return;
      return this.place(this.columnAt(next, column));
    }
    return this.place(this.columnAt(target, column));
  }

  /** The byte a column lands on in a line, clamped to its end. */
  private columnAt(line: TextLine, column: number): number {
    const cells = this.charsOf(line);
    return cells[column]?.at ?? this.textEnd(line);
  }

  private async moveToLineEdge(edge: "start" | "end"): Promise<void> {
    const here = this.caretLine();
    if (here === null) return;
    await this.place(edge === "start" ? here.line.at : this.textEnd(here.line));
  }

  /** Put the caret on a byte and tell the rest of the app, which is what moves
   *  every other view: the caret is the cursor, not a second position. */
  private async place(at: number): Promise<void> {
    const clamped = Math.max(0, Math.min(at, this.doc.lengthBytes));
    this.cursor = clamped;
    // While a selection is being extended the cursor moves with it, so it is
    // the selection that carries the caret. Announcing the caret on its own as
    // well would clear the selection, which is what putting the cursor
    // somewhere means everywhere else.
    if (this.extending) this.extendTo(clamped);
    else {
      if (this.selection !== null) this.selection = null;
      this.onPick(clamped);
    }
    await this.draw();
    this.revealCaret();
  }

  /** Type text in at the caret, over whatever is selected. */
  private async insert(text: string): Promise<void> {
    const got = this.doc.encodeText(this.chosen, this.reading.encoding, text);
    if (got.refused !== "") {
      this.onRefuse(got.refused, this.chosen === "" ? this.reading.encoding : this.chosen);
      return;
    }
    const bytes = Uint8Array.from(got.bytes);
    // What was selected goes and what was typed takes its place, in one write
    // and so in one undo step. The selection is replaced byte for byte, since
    // it may have been made over the bytes elsewhere and half a character is
    // still the bytes somebody picked.
    const sel = this.selRange();
    const at = sel === null ? this.cursor : sel.start;
    const plain = sel === null && this.leavesLinesAlone(text, bytes.length);
    this.write(() => this.doc.replaceAt(at, sel === null ? 0 : sel.end - sel.start, bytes));
    if (sel !== null) {
      this.selection = null;
      this.anchor = null;
      this.onSelect(at, at, at);
    }
    await this.after(at + bytes.length, at, plain ? bytes.length : null);
  }

  /**
   * Whether writing `text` at the caret leaves every line ending where it is,
   * so that the index can move the lines after it along instead of reading
   * them again.
   *
   * Three ways it might not. The text itself may be an ending. The caret may
   * be on a line the core cut for being too long, where every cut after it is
   * at a fixed distance from the line's real start and all of them move. And
   * the line may cross that limit by growing, which makes a cut that was not
   * there before. Anything else is a letter typed into a line.
   */
  private leavesLinesAlone(text: string, grew: number): boolean {
    if (/[\r\n]/.test(text)) return false;
    const here = this.caretLine();
    if (here === null || here.line.ending === "cut") return false;
    return this.textEnd(here.line) - here.line.at + grew <= MAX_LINE;
  }

  /** Take out the character before the caret, or the one after it. */
  private async erase(dir: 1 | -1): Promise<void> {
    const gone = this.removeSelection();
    if (gone !== null) return this.after(gone, gone, null);
    const here = this.caretLine();
    if (here === null) return;
    const cells = this.charsOf(here.line);
    if (dir === -1) {
      const prev = [...cells].reverse().find((c) => c.at < this.cursor);
      if (prev !== undefined) {
        const plain = here.line.ending !== "cut";
        this.write(() => this.doc.replaceAt(prev.at, prev.width, new Uint8Array()));
        return this.after(prev.at, prev.at, plain ? -prev.width : null);
      }
      // At the front of a line, what is behind the caret is the line ending
      // above it, however many bytes that turned out to be.
      if (here.line.at === 0) return;
      const b = await this.doc.textBack(this.chosen, here.line.at - 1, 0);
      const above = this.lines.find((l) => l.at === b.start);
      const end = above === undefined ? here.line.at - 1 : this.textEnd(above);
      this.write(() => this.doc.replaceAt(end, here.line.at - end, new Uint8Array()));
      return this.after(end, end, null);
    }
    const next = cells.find((c) => c.at === this.cursor);
    if (next !== undefined) {
      const plain = here.line.ending !== "cut";
      this.write(() => this.doc.replaceAt(next.at, next.width, new Uint8Array()));
      return this.after(next.at, next.at, plain ? -next.width : null);
    }
    // At the end of a line, what is in front of the caret is its ending.
    const ending = here.line.at + here.line.len - this.cursor;
    if (ending > 0) this.write(() => this.doc.replaceAt(this.cursor, ending, new Uint8Array()));
    return this.after(this.cursor, this.cursor, null);
  }

  /** After an edit: the caret lands where the change left it, the index gives
   *  back what the change made untrue, and every other view is told.
   *
   *  `delta` is how much longer the file got when the edit is known to have
   *  left every line ending alone, and null when it might not have. A typed
   *  letter moves the lines after it along; anything that could make or unmake
   *  an ending has them read again. */
  private async after(caret: number, at: number, delta: number | null): Promise<void> {
    this.cursor = Math.max(0, Math.min(caret, this.doc.lengthBytes));
    const from = this.index.place(at)?.at ?? at;
    for (const key of [...this.cache.keys()]) if (key >= from) {
      this.cacheChars -= this.cache.get(key)?.text.length ?? 0;
      this.cache.delete(key);
    }
    if (delta === null) this.index.dropFrom(at);
    else this.index.shiftFrom(at, delta);
    this.index.setLength(this.doc.lengthBytes);
    this.heights.clearMeasured();
    this.startIndexing();
    await this.draw(true);
    this.onPick(this.cursor);
    this.onEdit();
  }

  private async scrollLines(n: number): Promise<void> {
    if (n === 0) return;
    if (this.wrap !== "off") { this.scrollWrapped(n * ROW); return; }
    await this.goto(this.viewLine + n);
  }

  /** The byte a pointer is over, or null when it is not over a character. */
  private byteUnder(e: MouseEvent): number | null {
    // Resolve at use time: a queued drag event can refer to a span replaced
    // by the previous frame's selection decoration.
    const target = document.elementFromPoint(e.clientX, e.clientY) ?? e.target;
    if (!(target instanceof Element)) return null;

    // Runs keep the DOM small; ask the browser which insertion point inside
    // the run the pointer is nearest to, then turn that character back into
    // the byte address the editor owns. Offsets are UTF-16 code units, while
    // `charsOf` is Unicode code points, so count rather than using it raw.
    type CaretPoint = { readonly offsetNode: Node; readonly offset: number };
    type PointDocument = Document & {
      caretPositionFromPoint?: (x: number, y: number) => CaretPoint | null;
      caretRangeFromPoint?: (x: number, y: number) => Range | null;
    };
    const doc = document as PointDocument;
    const position = doc.caretPositionFromPoint?.(e.clientX, e.clientY);
    const range = position === undefined ? doc.caretRangeFromPoint?.(e.clientX, e.clientY) : undefined;
    const node = position?.offsetNode ?? range?.startContainer;
    const offset = position?.offset ?? range?.startOffset;
    const element = node instanceof Element ? node : node?.parentElement;
    const run = element?.closest<HTMLElement>(".tv-text[data-cell]") ?? target.closest<HTMLElement>(".tv-text[data-cell]");
    const row = run?.closest<HTMLElement>(".tv-row[data-line-at]");
    if (run === null || row === undefined || row === null || !this.rows.contains(row)) return null;
    const lineAt = Number(row.dataset.lineAt);
    const first = Number(run.dataset.cell);
    const line = this.lines.find(candidate => candidate.at === lineAt);
    if (!Number.isFinite(first) || line === undefined) return null;
    if (node === undefined || offset === undefined || !run.contains(node)) return null;
    const into =
      node === run
        ? offset === 0
          ? 0
          : [...(run.textContent ?? "")].length
        : [...(node.textContent ?? "").slice(0, offset)].length;
    const cells = this.charsOf(line);
    const cell = cells[first + into];
    return cell?.at ?? this.textEnd(line);
  }

  /** How the caret was last put somewhere, so a touch is not answered twice. */
  private lastPointer = "mouse";

  private onPointerDown(e: PointerEvent): void {
    this.lastPointer = e.pointerType;
    // A finger on the text is a scroll until it turns out not to be. Taking it
    // here would move the caret on every flick and, worse, would run a drag
    // selection through the whole of the momentum: a redraw per pointermove,
    // over a view that is meanwhile trying to scroll. What is left of the
    // touch, if the view did not scroll, arrives as a click.
    if (e.pointerType === "touch") return;
    const at = this.byteUnder(e);
    if (at === null) return;
    e.preventDefault();
    this.scroll.focus();
    if (e.shiftKey) {
      this.anchor ??= this.cursor;
      this.cursor = at;
      this.extendTo(at);
      this.render();
      return;
    }
    this.dragging = true;
    this.anchor = at;
    this.cursor = at;
    this.selection = null;
    this.onSelect(at, at, at);
    this.onPick(at);
    this.render();
  }

  /** A tap that did not turn into a scroll, which is where a touch puts the
   *  caret. A mouse has already done it on pointerdown. */
  private onClick(e: MouseEvent): void {
    if (this.lastPointer !== "touch") return;
    const at = this.byteUnder(e);
    if (at === null) return;
    this.scroll.focus();
    this.anchor = at;
    this.cursor = at;
    this.selection = null;
    this.onSelect(at, at, at);
    this.onPick(at);
    this.render();
  }

  private moveFrame = 0;
  private lastMove: PointerEvent | null = null;

  private onPointerMove(e: PointerEvent): void {
    if (!this.dragging) return;
    // One redraw per frame, from wherever the pointer is by then. Pointer
    // moves come faster than frames, and each redraw walks the window.
    this.lastMove = e;
    if (this.moveFrame !== 0) return;
    this.moveFrame = requestAnimationFrame(() => {
      this.moveFrame = 0;
      const ev = this.lastMove;
      if (ev === null || !this.dragging) return;
      const at = this.byteUnder(ev);
      if (at === null) return;
      this.cursor = at;
      this.extendTo(at);
      this.render();
    });
  }

  /** The rows as last drawn, keyed by everything that shaped them, so a redraw
   *  that changes nothing about a row keeps its element rather than building a
   *  thousand spans again. A scroll keeps every row that stayed on screen; a
   *  drag rebuilds only the rows the selection moved through. */
  private rowCache = new Map<string, HTMLElement>();
  private gutterCache = new Map<string, HTMLElement>();
  private lineIds = new WeakMap<TextLine, number>();
  private nextLineId = 0;

  /** Everything a row's appearance depends on. */
  private rowKey(line: TextLine): string {
    let id = this.lineIds.get(line);
    if (id === undefined) {
      id = ++this.nextLineId;
      this.lineIds.set(line, id);
    }
    const end = line.at + line.len;
    const sel = this.selection;
    const a = sel === null ? 0 : Math.max(sel.start, line.at);
    const b = sel === null ? 0 : Math.min(sel.end, end);
    const selPart = b > a ? `${a}:${b}` : "";
    const cur = this.cursor >= line.at && this.cursor <= end ? this.cursor : -1;
    const S = "\u0000";
    return id + S + cur + S + selPart + S + this.usualEnding + S + this.reading.encoding;
  }

  private render(): void {
    this.captureWrappedScroll();
    // Measure once before mutations; resizing invalidates both the clipping
    // budget and all wrap heights, but never the decoded text or byte index.
    const width = this.scroll.clientWidth;
    if (width <= 0) return;
    if (width !== this.layoutWidth) {
      this.layoutWidth = width;
      this.heights.clearMeasured();
      this.rowCache.clear();
      const probe = document.createElement("canvas").getContext("2d");
      if (probe !== null) {
        const style = getComputedStyle(this.rows);
        probe.font = `${style.fontSize} ${getComputedStyle(this.gutter).fontFamily}`;
        this.clipChars = Math.max(16, Math.ceil(width / Math.max(1, probe.measureText("0").width)) + 8);
      }
    }
    const keptRows = new Map<string, HTMLElement>();
    const keptGutter = new Map<string, HTMLElement>();
    const gutter: HTMLElement[] = [];
    const rows: HTMLElement[] = [];
    for (let i = 0; i < this.lines.length; i++) {
      const line = this.lines[i];
      if (line === undefined) continue;
      // The line number is there once the index has reached this far, and the
      // offset always: the offset is what this view is for, and the number is
      // what a reader of a log came looking for.
      const n = this.index.lineAt(line.at);
      const gkey = `${line.at}\u0000${n ?? ""}`;
      const g =
        this.gutterCache.get(gkey) ??
        el(
          "div",
          { className: "tv-off" },
          ...(n === null ? [] : [el("span", { className: "tv-lineno", textContent: String(n + 1) })]),
          el("span", { className: "tv-at", textContent: formatOffset(line.at * 8) }),
        );
      keptGutter.set(gkey, g);
      gutter.push(g);
      const key = this.rowKey(line);
      const r = this.rowCache.get(key) ?? this.row(line);
      keptRows.set(key, r);
      rows.push(r);
    }
    this.rowCache = keptRows;
    this.gutterCache = keptGutter;
    replace(this.gutter, gutter);
    replace(this.rows, rows);
    if (this.wrap !== "off") {
      // Read all heights together before writing the gutter: one layout pass.
      const real = rows.map(row => row.offsetHeight);
      real.forEach((h, i) => {
        this.heights.measure(this.topLine + i, h);
        const g = gutter[i];
        if (g !== undefined) g.style.height = `${h}px`;
      });
      this.heights.trim(this.viewLine);
      this.syncWrappedPosition();
    } else {
      for (const g of gutter) g.style.height = "";
    }
  }

  /** Where each character of a line sits and how many bytes it takes. The
   *  write path asks this as much as the drawing does: what backspace removes
   *  is the width of the character before the caret, not one byte. */
  private charsOf(line: TextLine): { char: string; at: number; width: number }[] {
    const key = `${this.reading.encoding}\u0000${this.reading.unit}`;
    if (key !== this.charCacheKey) {
      this.charCache = new WeakMap();
      this.charCacheKey = key;
    }
    const hit = this.charCache.get(line);
    if (hit !== undefined) return hit;
    const out: { char: string; at: number; width: number }[] = [];
    let at = line.at;
    for (const c of line.text) {
      const width = charWidth(c, this.reading.encoding, this.reading.unit);
      out.push({ char: c, at, width });
      at += width;
    }
    this.charCache.set(line, out);
    return out;
  }

  /** The cells of each line, held for as long as the line is. A line lives in
   *  the cache now, so this lasts as long as the line does rather than as long
   *  as one window; keying the map itself on the reading is what invalidates
   *  it when the encoding changes. */
  private charCache = new WeakMap<TextLine, { char: string; at: number; width: number }[]>();
  private charCacheKey = "";

  private row(line: TextLine): HTMLElement {
    const row = el("div", { className: "tv-row" });
    row.dataset.lineAt = String(line.at);
    if (line.lossy) row.classList.add("is-lossy");
    const cells = this.charsOf(line);
    const escapes = escapeMask(cells.length, line.escapes);
    const sel = this.selection;
    const caretIndex = this.wrap === "off" ? cells.findIndex(c => this.cursor >= c.at && this.cursor < c.at + c.width) : -1;
    const column = this.cursor === this.textEnd(line) ? cells.length : caretIndex;
    // Keep a far-away byte reachable in No wrap without constructing the
    // offscreen prefix. The ellipsis makes the shifted excerpt explicit.
    const first = this.wrap === "off" && column >= this.clipChars - 16 ? Math.max(0, column - Math.floor(this.clipChars / 2)) : 0;
    const shown = Math.min(cells.length, first + (this.wrap === "off" ? this.clipChars : MAX_CHARS));
    if (first > 0) {
      row.append(el("span", { className: "tv-more", textContent: "…", title: "Earlier text is outside this excerpt. Enable wrapping to show the complete line." }));
    }
    // Adjacent characters with the same appearance share one span. A plain
    // long line is one node rather than two thousand; `byteUnder` uses the
    // browser's caret hit test to recover an exact character inside the run.
    let runText = "";
    let runClass = "";
    let runStart = 0;
    const flush = (): void => {
      if (runText === "") return;
      const span = el("span", { className: `tv-text${runClass}`, textContent: runText });
      span.dataset.cell = String(runStart);
      row.append(span);
      runText = "";
    };
    for (let i = first; i < shown; i++) {
      const cell = cells[i];
      if (cell === undefined) continue;
      const { char: c, at, width } = cell;
      if (this.cursor === at && this.wrap === "off") {
        flush();
        row.append(el("span", { className: "tv-caret" }));
      }
      const picture = control(c);
      let classes = escapes[i] === true ? " tv-esc" : picture === null ? "" : " tv-ctl";
      // The cursor may sit inside a character rather than on its front, which
      // is what a selection made over the bytes elsewhere can do.
      if (this.cursor >= at && this.cursor < at + width) classes += " is-cursor";
      if (sel !== null && at < sel.end && at + width > sel.start) classes += " is-sel";
      if (classes !== runClass) {
        flush();
        runClass = classes;
        runStart = i;
      } else if (runText === "") {
        runStart = i;
      }
      runText += picture ?? c;
    }
    flush();
    const end = this.textEnd(line);
    if (this.cursor === end) row.append(el("span", { className: "tv-caret" }));
    // A line ending inside the selection is shown as a selected space at the
    // end of the line, so a selection across lines does not look like it stops
    // at each of them.
    if (sel !== null && end < sel.end && line.at + line.len > sel.start && line.ending !== "no ending" && line.ending !== "cut") {
      row.append(el("span", { className: "tv-text is-sel tv-eol", textContent: " " }));
    }
    if (cells.length > shown) row.append(el("span", { className: "tv-more", textContent: TEXTVIEW.lineClipped }));
    if (line.ending === "cut") row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineCut }));
    else if (ENDINGS.has(line.ending) && line.ending !== this.usualEnding && this.usualEnding !== "") {
      row.append(el("span", { className: "tv-mark", textContent: line.ending }));
    }
    if (line.lossy) row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineLossy(this.reading.encoding) }));
    return row;
  }

  /** The selection as bytes, or null. */
  private selRange(): { start: number; end: number } | null {
    return this.selection;
  }

  /** Put the selection where a move left it, and tell the rest of the app. */
  private extendTo(at: number): void {
    if (this.anchor === null) return;
    const start = Math.min(this.anchor, at);
    const end = Math.max(this.anchor, at);
    this.selection = end > start ? { start, end } : null;
    this.onSelect(start, end, at);
  }

  /** One change to the file, however many writes it takes.
   *
   *  A replacement that changes length is a delete and an insert underneath,
   *  so without this a keystroke over a selection would take two presses of
   *  undo to put back and the file would sit in a state nobody typed. */
  private write(f: () => void): void {
    this.doc.beginBatch();
    try {
      f();
    } finally {
      this.doc.endBatch();
    }
  }

  /** Take out whatever is selected, leaving the caret where it was. */
  private removeSelection(): number | null {
    const sel = this.selRange();
    if (sel === null) return null;
    this.write(() => this.doc.replaceAt(sel.start, sel.end - sel.start, new Uint8Array()));
    this.selection = null;
    this.anchor = null;
    this.onSelect(sel.start, sel.start, sel.start);
    return sel.start;
  }

  /** The selected text, for the clipboard. Read from the document rather than
   *  from the screen, so a selection reaching past what is drawn copies whole
   *  rather than copying what happens to be visible. */
  private async copySelection(): Promise<void> {
    const sel = this.selRange();
    if (sel === null) return;
    const len = Math.min(sel.end - sel.start, COPY_LIMIT);
    const first = this.chosen === "" ? this.reading.encoding : this.chosen;
    // The reading wanted here is the file's own, which comes back first. The
    // two code page slots still have to be named, and are the reader's own
    // choices so that a page picked in the panel is the one a copy uses.
    const got = this.doc.selectionText(
      sel.start,
      len,
      first,
      storedChoice(CODEPAGE_A_KEY, CODEPAGES_A, CODEPAGE_A_DEFAULT),
      storedChoice(CODEPAGE_B_KEY, CODEPAGES_B, CODEPAGE_B_DEFAULT),
    );
    const text = got?.readings[0]?.text;
    if (text === undefined) return;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      this.onMessage(TEXTVIEW.copyFailed);
    }
  }
}

/** Put a list of children in a parent, moving what is already there rather
 *  than tearing the lot down: nearly every row of a scroll is a row that was
 *  on screen a moment ago, and the ones entering and leaving are the only two
 *  handfuls worth touching. */
function replace(parent: HTMLElement, want: readonly HTMLElement[]): void {
  const keep = new Set<Element>(want);
  for (const child of [...parent.children]) if (!keep.has(child)) child.remove();
  for (let i = 0; i < want.length; i++) {
    const node = want[i];
    if (node === undefined) continue;
    if (parent.children[i] !== node) parent.insertBefore(node, parent.children[i] ?? null);
  }
}

/** A string with its control characters shown as their pictures. */
export function withPictures(text: string): string {
  return [...text].map((c) => control(c) ?? c).join("");
}

/** Movement keys that extend a selection when Shift is down. */
const SELECTING = new Set(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"]);

/** The characters a line ending is made of, so Enter writes what the rest of
 *  the file writes. A file that has not settled on one gets a line feed, which
 *  is what a file with no endings at all would be given by anything else. */
function endingText(usual: string): string {
  return usual === "CRLF" ? "\r\n" : usual === "CR" ? "\r" : "\n";
}

/** Which characters of a line are inside an escape sequence. */
function escapeMask(len: number, flat: readonly number[]): boolean[] {
  const mask = new Array<boolean>(len).fill(false);
  for (let i = 0; i + 1 < flat.length; i += 2) {
    const start = flat[i] ?? 0;
    const n = flat[i + 1] ?? 0;
    for (let j = start; j < start + n && j < len; j++) mask[j] = true;
  }
  return mask;
}

/** The picture Unicode gives a control character, so a stray byte is visible
 *  rather than moving the text around. Shared with the inspector's readings of
 *  a selection: a selected line feed shown as a line feed is a row that looks
 *  empty. */
export function control(c: string): string | null {
  const code = c.codePointAt(0) ?? 0;
  if (code < 0x20) return String.fromCodePoint(0x2400 + code);
  if (code === 0x7f) return "\u{2421}";
  return null;
}

/** How many bytes one character takes in an encoding. This is a write path as
 *  much as a display one: a character above the basic plane is two UTF-16 code
 *  units, so backspacing over one has to take four bytes and not two. */
function charWidth(c: string, encoding: string, unit: number): number {
  const code = c.codePointAt(0) ?? 0;
  if (encoding === "UTF-8") return code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  if (unit === 2) return code < 0x10000 ? 2 : 4;
  return 1;
}

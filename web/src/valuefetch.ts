// What the field column asks the core for, and what it keeps while it waits.
//
// Two questions, one for each half of the column: which fields are on screen
// (`spansForView`) and what the elements of a folded run read as
// (`runsForView`). Both are answered over the wire from the core, both can
// come back later than the frame that asked, and both have the same rule:
// **the last answer stands until the next one is ready**. Scrolling one step
// through a program is what that is for. The reply for the new window takes
// several goes, and blanking the column for every one of them makes the whole
// column flicker off for as long as the reading takes.
//
// Nothing here draws. The view asks, gets values back, and places them: where
// the spans land on the rows is `chipplan.ts`, and what shape the values take
// is `valuelayout.ts`. Keeping the waiting here is what lets the view's draw
// path be a function of what it was handed.

import type { Cell, Doc, Span } from "./doc.js";
import { SPAN_LIMIT } from "./hexchips.js";
import type { RunCells } from "./valuetable.js";

/** How many elements of one run the value table reads at a time. A screenful
 *  of 24-bit samples is a few hundred; the rest of the limit is the margin
 *  either side that lets a scroll of a row cost nothing. */
const VALUE_LIMIT = 2000;

/** What one element of a run is, from what the run is: `i16 le[]` holds an
 *  `i16 le`. The core writes the brackets; nothing else is taken off. */
const elementType = (type: string): string => type.replace(/\[\]$/, "");

/** What one ask for spans came back with. `error` is the template failing to
 *  read what is here, which an empty column would not say. */
export type SpanAnswer = { spans: Span[]; more: boolean; error: string | null };

export class ValueFetch {
  /** Ask the core again for whatever is on screen by then, and draw with it. */
  constructor(
    private readonly doc: Doc,
    private readonly redraw: () => void,
  ) {}

  /** The last answered spans, and the stretch of file they were asked for.
   *  Kept past the view moving: while the next answer is still being worked
   *  out, the rows these still cover keep their chips. */
  private spanCache: { key: string; from: number; to: number; spans: Span[]; more: boolean; error: string | null } | null =
    null;
  /** Whether this draw is the one that asks the core for spans, rather than
   *  the one that gets the bytes on screen first. See `spansForView`. */
  private spansNow = false;
  /** The frame already booked to ask for spans, so that a burst of draws books
   *  one. Cleared when it runs. */
  private spansSoon: number | null = null;
  /** The window already asked about once this way. A second draw on the same
   *  window asks outright rather than booking another frame, so a reply that
   *  needs several goes still gets them. */
  private spansAsked: string | null = null;
  /** The elements of the runs on screen, kept between draws and read again
   *  only when the view has left what was fetched. A scroll of one row must
   *  not cost a screenful of elements. Keyed by the run's path; emptied
   *  whenever the document or the template changes. */
  private readonly cellCache = new Map<string, { from: number; to: number; cells: Cell[] }>();
  /** The widest text each run has shown, kept past the cells themselves. Which
   *  layout a run gets is decided from the widest value on screen, and a value
   *  a digit longer scrolling into view would otherwise take every row of that
   *  run from the aligned layout to the uniform one and back again. Kept as
   *  the text rather than a width, so it survives a change of font. */
  private readonly runWidest = new Map<string, string>();

  /** Forget the elements, keep what the runs have been. Turning the column on
   *  and off must not let a run take the aligned layout back after a wider
   *  value has already been seen in it. */
  forgetCells(): void {
    this.cellCache.clear();
  }

  /** Forget everything: the bytes themselves have changed, so a run is not the
   *  run it was and nothing read off the old ones stands. */
  forgetAll(): void {
    this.spanCache = null;
    this.spansAsked = null;
    this.cellCache.clear();
    this.runWidest.clear();
  }

  /** Forget the spans alone, for a change of shape that leaves the bytes as
   *  they were. */
  forgetSpans(): void {
    this.spanCache = null;
    this.spansAsked = null;
  }

  /** Spans for the rows on screen.
   *
   *  An answer that is not ready yet leaves the last one on screen for the
   *  rows it still covers, and only the rows past it empty. */
  spansForView(start: number, count: number): SpanAnswer {
    const key = `${start}:${count}:${this.doc.template ?? ""}`;
    if (this.spanCache?.key === key) return this.spanCache;
    // The core's answer for a window nobody has asked about yet can take
    // longer than a frame: inside a compressed stream it has bits to decode
    // before it can say what is there. The bytes do not wait for it. A draw
    // that lands on an unanswered window puts the hex up with the chips the
    // last answer left, and books the next frame to ask. That frame asks about
    // wherever the view is by then, so a second wheel step before the first
    // answer lands replaces the question rather than queueing behind it.
    if (!this.spansNow && this.spanCache !== null && this.spansAsked !== key) {
      this.askForSpansSoon();
      const kept = this.spanCache;
      const overlaps = kept.from < start + count && start < kept.to;
      return overlaps ? { spans: kept.spans, more: kept.more, error: null } : { spans: [], more: false, error: null };
    }
    this.spansNow = false;
    this.spansAsked = key;
    const max = Math.min(SPAN_LIMIT, count * 8);
    const r = this.doc.spans(start * 8, (start + count) * 8, max);
    // Pending: the bytes are on their way. Working: the structure is still
    // being worked out. Both come back on their own, and until they do the
    // last answer stands wherever it reaches. `placeChips` draws only the
    // spans that fall on screen, so the rows past it are left empty.
    if (r.status === "pending" || r.status === "working") {
      const kept = this.spanCache;
      if (kept === null || kept.to <= start || start + count <= kept.from) {
        return { spans: [], more: false, error: null };
      }
      return { spans: kept.spans, more: kept.more, error: null };
    }
    // The template cannot read what is here, usually after an edit that
    // changed a length, and an empty column would not say that.
    if (r.status === "error") return { spans: [], more: false, error: r.message };
    this.spanCache = {
      key,
      from: start,
      to: start + count,
      spans: r.node,
      more: r.node.length >= max,
      error: null,
    };
    return this.spanCache;
  }

  /** Book the next frame to ask the core for the spans of whatever is on
   *  screen then, and draw again with them. One booking at a time. */
  private askForSpansSoon(): void {
    if (this.spansSoon !== null) return;
    this.spansSoon = requestAnimationFrame(() => {
      this.spansSoon = null;
      this.spansNow = true;
      try {
        this.redraw();
      } finally {
        this.spansNow = false;
      }
    });
  }

  /**
   * The elements of one folded run over a stretch of it, from the cache where
   * the cache reaches and from the core where it does not.
   *
   * A screenful either side is read at a time, so scrolling a row at a time
   * does not ask again for what is already here; an answer that is not ready
   * yet leaves the last table on screen, the way the spans do. What is kept
   * reaches past the window on purpose, and the rows it falls outside drop it.
   */
  private runCells(s: Span, fromBit: number, toBit: number): Cell[] | null {
    const key = s.path.join(",");
    const had = this.cellCache.get(key);
    if (had !== undefined && had.from <= fromBit && had.to >= toBit) return had.cells;
    const margin = toBit - fromBit;
    const end = s.offset_bits + s.size_bits;
    const from = Math.max(s.offset_bits, fromBit - margin);
    const to = Math.min(end, toBit + margin);
    const r = this.doc.runCells(s.path, from, to, VALUE_LIMIT);
    // Still on its way: what was read last time stands until it arrives, so
    // the table does not blink off for every step of a scroll.
    if (r.status !== "ok") return had?.cells ?? null;
    const cells = r.node;
    // The limit may have stopped the answer short of what was asked for, and
    // what is cached has to say what it really covers or the next draw would
    // take the missing tail for an empty stretch of file.
    const last = cells[cells.length - 1];
    const reached = cells.length < VALUE_LIMIT || last === undefined ? to : last.offset_bits + last.size_bits;
    // One file can hold many runs, and every one scrolled past would otherwise
    // be kept for the life of the view.
    if (this.cellCache.size > 16) this.cellCache.clear();
    this.cellCache.set(key, { from, to: reached, cells });
    return cells;
  }

  /**
   * The runs whose values are drawn on this screenful: the ones the core
   * folded, `body 72,000 values`, read through `runCells`.
   *
   * Not the runs the view folds, where a handful of a list's elements sit
   * together on a row and become one chip. Those are already on screen as
   * spans and would cost nothing to draw — but whether they fold at all
   * depends on what the top row carries in from above, so the same row would
   * be one height at the top of the screen and another below it, which is the
   * one thing a row's height may never depend on.
   */
  runsForView(spans: readonly Span[], start: number, windowBytes: number): RunCells[] {
    const fromBit = start * 8;
    const toBit = (start + windowBytes) * 8;
    const out: RunCells[] = [];
    for (const s of spans) {
      if (s.count <= 0 || s.gap) continue;
      const end = s.offset_bits + s.size_bits;
      if (end <= fromBit || s.offset_bits >= toBit) continue;
      const cells = this.runCells(s, Math.max(fromBit, s.offset_bits), Math.min(toBit, end));
      if (cells === null || cells.length === 0) continue;
      // `unit` is the format's own word for what a run holds: a deflate block
      // codes symbols, and a symbol's cell reads as one. The type a cell says
      // is the element's, not the run's: a cell of `body` holds an `i16 le`,
      // and the run is the `i16 le[]`.
      out.push({
        path: s.path,
        name: s.name,
        type: elementType(s.type),
        symbol: s.unit === "symbol",
        widest: this.widestOf(s.path.join(","), cells),
        cells,
      });
    }
    return out;
  }

  /** The widest text a run has shown, this screenful's cells included. */
  private widestOf(key: string, cells: readonly Cell[]): string {
    let widest = this.runWidest.get(key) ?? "";
    for (const c of cells) {
      if (c.label.length > widest.length) widest = c.label;
    }
    this.runWidest.set(key, widest);
    return widest;
  }
}

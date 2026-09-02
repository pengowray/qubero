// The listing as one scrolling report: the file's parts as headings, their
// fields as rows, and the bytes nothing claimed as rows of their own.
//
// `flatten` says what to draw and in what order; this says how tall each thing
// is and puts the ones on screen in the document. Everything else stays out of
// it, so a file whose tree runs to millions of nodes costs a screenful of DOM.
// What one item looks like is `listingdraw.ts`: this file is the layout, the
// input and the selection, and hands that one an item and a `DrawContext`.
//
// Scrolling counts items, not bits. The old listing scrolls the file itself
// because `spans` is windowed by bit range and cannot say how many rows a file
// has; a flattened tree is a list with a length, and a list scrolls by index.

import type { Doc } from "./doc.js";
import type { FieldPick } from "./doc.js";
import { emptyState, flatten, PAGE, refold } from "./flatten.js";
import type { FlatOptions, Item, ListingState, TreeSource, Window } from "./flatten.js";
import { sectionColor } from "./fieldstyle.js";
import { markStrip } from "./bytestrip.js";
import { cardKind, watchCard } from "./contentcard.js";
import { markMap } from "./filemap.js";
import { checkGap } from "./gapcheck.js";
import { isRecordList } from "./records.js";
import { jpegCardKind } from "./jpegcards.js";
import { drawItem, el, headingTitle, holdsSelection, isSelected, itemOpens } from "./listingdraw.js";
import type { DrawContext, Selected } from "./listingdraw.js";
import type { GapVerdict } from "./gapcheck.js";
import type { MapSegment } from "./filemap.js";
import type { OutlineHeading, Viewport } from "./outline.js";
import { NO_TEMPLATE_HINT, NO_TEMPLATE_MATCH, REPORT } from "./strings.js";

/** Row heights, which must match `--rp-*` in the stylesheet: the tops of every
 *  item are a running total of these, and a row that draws taller than it was
 *  measured at would slide out from under its own place. */
const HEIGHT = { section: 54, part: 36, row: 22, strip: 96, card: 400 } as const;
/** Items drawn above and below the window, so a wheel notch has somewhere to
 *  go before the next paint. */
const OVERSCAN = 6;
/** How long a hidden listing waits after the file's last change before
 *  walking the tree for the views that are showing. */
const HIDDEN_WALK_MS = 300;

/** What an item is worth before it has been drawn. A byte strip's chips wrap
 *  to the width they are given, so its real height is only known once it is
 *  in the document; this is the guess the first layout uses, and `measured`
 *  replaces it. */
function heightOf(item: Item): number {
  if (item.kind === "bytes" || item.kind === "record" || item.kind === "formatcard") return HEIGHT.strip;
  if (item.kind === "card") return HEIGHT.card;
  if (item.kind !== "heading") return HEIGHT.row;
  return item.level === 0 ? HEIGHT.section : HEIGHT.part;
}

/** The kinds whose height is what they turn out to be rather than what their
 *  kind says. */
function isMeasured(item: Item): boolean {
  return item.kind === "bytes" || item.kind === "record" || item.kind === "card" || item.kind === "formatcard";
}

export class ListingReport {
  readonly el: HTMLElement;
  /** The bar above the rows saying what the top of the window is inside. */
  private readonly crumbs: HTMLElement;
  private readonly scroller: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly doc: Doc;
  private readonly src: TreeSource;
  private state: ListingState = emptyState;
  private items: readonly Item[] = [];
  /** The rows standing in the document right now, by key. Scrolling a listing
   *  moves the window by an item or two, and drawing all thirty again costs
   *  the browser a fresh layout of every one of them. A row whose key is still
   *  in the window is the same row: it is left where it is and only told its
   *  new top. Anything that changes what a row *says* clears this, so a kept
   *  row is never a stale one. */
  private mounted = new Map<string, HTMLElement>();
  /** The items by key, for the redraws that start from a row on screen rather
   *  than from a place in the list. */
  private byKey = new Map<string, Item>();
  /** Where each item starts, in pixels, and one past the last: `tops[i + 1]`
   *  is always there. */
  private tops: number[] = [0];
  private drawn: { from: number; to: number } | null = null;
  /** The file's top-level parts, which every strip of the map is drawn from.
   *  Worked out once per flatten so that every strip has the same geometry. */
  private segments: readonly MapSegment[] = [];
  /** Heights of items whose real one had to be measured, by key. Only byte
   *  strips are in here; everything else is the height its kind says. */
  private measured = new Map<string, number>();
  /** Width the strips were measured at. Chips wrap, so a narrower view makes
   *  every one of them taller and the measurements have to go. */
  private measuredAt = 0;
  /** True while a measurement's own redraw is running. */
  private measuring = false;
  /** Whether the file's first bytes matched a template. */
  private matched = true;
  /** What each gap turned out to hold. Checking one reads up to 64 KiB and can
   *  ask the file for chunks; without this that happens on every scroll tick
   *  for every gap on screen. Thrown away when the file changes, since that is
   *  when an answer can stop being true. */
  private verdicts = new Map<string, GapVerdict>();
  private frame = 0;
  private hiddenTimer = 0;
  /** True when the file changed while this was hidden, so the rows on screen
   *  are of a file that has moved on. Walked again when it comes back. */
  private stale = false;
  /** What is selected, as the bits it covers rather than as the row showing
   *  it. The same field appears as a row, as a line of a record table, as a
   *  column of an open byte strip and as a mark on every file map; all four
   *  are answered by the range, and only one of them by a row. */
  private selected: Selected | null = null;
  /** True while a selection this view made is being sent out, so the cursor
   *  move it causes does not come back and undo the scroll position. */
  private picking = false;
  /** The key of the row standing in for a selection with no row of its own,
   *  worked out once a paint rather than once a row. */
  private nearest: string | null | undefined = undefined;
  /** The rows and record tables as nested stretches of the file, so that the
   *  one holding the selection is found by bisection and a walk up its
   *  containers rather than by reading every item. Built with the layout,
   *  since that is when the list changes; null when the items did not come out
   *  in address order, which the walk means them to and which this cannot
   *  assume of a template that placed a field out of line. */
  private nesting: Nesting | null = null;
  /** The item the keyboard is on, by key so that it survives a re-flatten.
   *  Null until an arrow key is pressed, which starts it at the top of the
   *  window rather than at the top of the file. */
  private cursor: string | null = null;
  /** Fields opened out into a dump of all their bytes, by the bit they start
   *  at, and where each dump is scrolled to. Neither can live in the strip:
   *  it is built again from nothing every time anything on screen changes. */
  private dumps = new Set<number>();
  /** The long halves of format cards the reader has opened. Outside the
   *  items, since the items are thrown away and built again on every scroll. */
  private cards = new Set<string>();
  private dumpTops = new Map<number, number>();

  onPick: (pick: FieldPick) => void = () => {};
  /** A long list was asked for on its own. The pane is the caller's, since
   *  where it goes on the screen is not this view's business. */
  onOpenList: (path: readonly number[]) => void = () => {};
  /** The parts of the file changed: the tree was walked again. The rail and
   *  the hex view take their headings from `outline()` when this fires. */
  onOutline: (headings: readonly OutlineHeading[]) => void = () => {};
  /** The stretch of the file on screen, after every paint, from the first
   *  drawn item to the end of the last. */
  onViewport: (v: Viewport) => void = () => {};

  /** The headings the listing draws, in file order: the parts of the file as
   *  every surface names them. */
  outline(): OutlineHeading[] {
    const out: OutlineHeading[] = [];
    for (const i of this.items) {
      if (i.kind !== "heading") continue;
      out.push({
        key: i.key,
        section: i.section,
        level: i.level,
        path: i.path,
        name: i.title,
        offsetBits: i.offsetBits,
        sizeBits: i.sizeBits,
        color: sectionColor(i.section),
      });
    }
    return out;
  }

  constructor(doc: Doc) {
    this.doc = doc;
    this.src = {
      node: (path) => doc.templateNode(path),
      children: (path, from, to) => doc.templateChildren(path, from, to),
    };
    this.el = el("div", "report");
    this.crumbs = el("div", "rp-crumbs");
    this.scroller = el("div", "rp-scroll");
    this.scroller.tabIndex = 0;
    this.scroller.setAttribute("role", "tree");
    this.canvas = el("div", "rp-canvas");
    this.scroller.append(this.canvas);
    this.el.append(this.crumbs, this.scroller);
    // The rows are what the keyboard drives, so focus given to the view as a
    // whole lands on them.
    this.el.tabIndex = -1;
    this.el.addEventListener("focus", () => this.scroller.focus());
    this.scroller.addEventListener("scroll", () => this.paint(), { passive: true });
    this.scroller.addEventListener("click", (e) => this.onClick(e));
    this.scroller.addEventListener("keydown", (e) => this.onKey(e));
    new ResizeObserver(() => {
      const width = this.scroller.clientWidth;
      // A hidden view has no width and nothing to measure against; what was
      // measured is kept for when it is back.
      if (width > 0 && width !== this.measuredAt) {
        // Strips wrap to the width and the card scales to it, so every
        // measured height is stale. And rows drawn while this had no width
        // were drawn for no one: the card did not start its decode. Drawing
        // again answers both.
        this.measured.clear();
        this.layout();
      }
      this.relayout();
    }).observe(this.scroller);
    // Chunks arriving turn a pending stretch into rows; so does an edit.
    doc.onChange(() => {
      this.verdicts.clear();
      this.schedule();
    });
    // The card at the top of an image file changes on its own, when the
    // picture has been decoded or the reader has changed its size; it is a
    // different height each time.
    watchCard(doc, (what) => {
      if (what === "size") {
        // The picture in the card that is already up has its size now. The
        // card is measured where it stands; a height that differs from the
        // one it was laid out at is what lays the list out again, and one
        // that does not ends it, which is what stops the picture in the
        // redrawn card asking for the same thing again.
        if (this.drawn !== null) this.remeasure(this.drawn.from, this.drawn.to);
      } else this.refreshCard();
    });
    // A file that never changes after opening still has parts to name.
    this.schedule();
  }

  /** Draw again once the frame is over, however many things asked. Streaming a
   *  file's first megabyte fires `onChange` far faster than a screen redraws.
   *
   *  Only for changes the file makes to itself. Something the reader did takes
   *  effect there and then: waiting a frame to open a structure buys nothing,
   *  and a hidden tab runs no frames at all. */
  private schedule(): void {
    // Nothing is drawn while the hex view has the screen, and walking the tree
    // to draw nothing is not free: a file being streamed fires this once per
    // chunk, and flattening a whole SQLite database on each of them is a cost
    // the reader pays in the view they are actually looking at.
    if (this.el.hidden) {
      this.stale = true;
      // The walk is still owed while this is hidden: the rail and the hex view
      // take the file's parts from it. Once the changes have quietened, so a
      // file being streamed costs one walk and not one per chunk.
      clearTimeout(this.hiddenTimer);
      this.hiddenTimer = window.setTimeout(() => {
        this.hiddenTimer = 0;
        if (!this.el.hidden || !this.stale) return;
        this.stale = false;
        this.rebuild();
      }, HIDDEN_WALK_MS);
      return;
    }
    if (this.frame !== 0) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.rebuild();
    });
  }

  relayout(): void {
    if (this.el.hidden) return;
    // Everything the file did while this was hidden lands in one walk here.
    if (this.stale || this.items.length === 0) {
      this.stale = false;
      this.rebuild();
    } else this.paint();
  }

  /** Walk the tree again and lay the items out, keeping the reader where they
   *  were. The item under the top of the window is the anchor: opening a fold
   *  above it would otherwise push the whole file down under the cursor. */
  private rebuild(): void {
    const anchor = this.anchor();
    this.items = flatten(this.src, this.state, this.flatOpts()).items;
    this.segments = this.items
      .filter((i) => i.kind === "heading" && i.level === 0)
      .map((i) => ({ offsetBits: i.offsetBits, sizeBits: i.sizeBits, color: sectionColor(i.section) }));
    // A strip that is no longer in the list keeps no height.
    const live = new Set(this.items.map((i) => i.key));
    for (const key of [...this.measured.keys()]) if (!live.has(key)) this.measured.delete(key);
    this.layout();
    if (anchor !== null) this.restore(anchor);
    this.paint();
    this.onOutline(this.outline());
    // A stretch still being read comes back as a pending item, so this runs
    // again when its chunks land: `Doc` has already asked for them.
  }

  /** How the tree is flattened. Read afresh each time, since what the file
   *  opens with follows its template, and the template can change. */
  private flatOpts(): FlatOptions {
    return {
      isRecord: (node) => isRecordList(this.doc, node),
      formatCard: (node) => jpegCardKind(this.doc, node),
      card: cardKind(this.doc.template),
      fileBits: this.doc.lengthBits,
    };
  }

  /** Draw the content card again: it has something new to show and is not
   *  the height it was. The reader's place is kept, since the card is above
   *  everything and would otherwise push it all down. */
  private refreshCard(): void {
    const key = this.items.find((i) => i.kind === "card")?.key;
    if (key === undefined) return;
    const node = this.mounted.get(key);
    if (node !== undefined) {
      node.remove();
      this.mounted.delete(key);
    }
    this.measured.delete(key);
    const anchor = this.anchor();
    this.place();
    if (anchor !== null) this.restore(anchor);
    this.paint();
  }

  /**
   * Redraw what a change of selection or of the keyboard's place changes, and
   * no more.
   *
   * Nothing is built again. A row says whether it is the selected one with a
   * class. A heading, an open byte strip and a record table each draw the
   * selection into themselves — as a mark on the file map, as a lit column, as
   * a lit line — and each of the three is told where it goes now: the mark and
   * the column are the only parts of those that move, and a strip read out of
   * the file again to move one class is a strip's whole cost for a class.
   */
  private restyle(): void {
    this.nearest = this.nearestToSelection();
    for (const [key, node] of [...this.mounted]) {
      const item = this.byKey.get(key);
      if (item === undefined) {
        node.remove();
        this.mounted.delete(key);
        continue;
      }
      node.classList.toggle("is-cursor", key === this.cursor);
      if (key === this.cursor) this.scroller.setAttribute("aria-activedescendant", node.id);
      if (item.kind === "heading") {
        const map = node.querySelector<HTMLElement>(".fmap");
        if (map !== null) markMap(map, this.selected);
        continue;
      }
      if (item.kind === "bytes") {
        const strip = node.querySelector<HTMLElement>(".bstrip");
        if (strip !== null) markStrip(strip, this.selected);
        continue;
      }
      if (item.kind === "record" || item.kind === "formatcard") {
        this.markRecord(node);
        continue;
      }
      node.classList.toggle("is-on", item.kind === "row" && (this.isSelected(item.offsetBits, item.sizeBits) || this.nearest === key));
    }
    this.paint();
  }

  /** Which line of a record table holds the selection. The lines carry the
   *  stretch they were read from, so this is the same question `drawRecord`
   *  asked, asked again of the table already on screen. */
  private markRecord(host: HTMLElement): void {
    for (const line of host.querySelectorAll<HTMLElement>("[data-at][data-size]")) {
      const at = Number(line.dataset["at"]);
      const size = Number(line.dataset["size"]);
      line.classList.toggle("is-on", this.holdsSelection(at, size));
    }
  }

  private discard(): void {
    if (this.mounted.size > 0) {
      for (const node of this.mounted.values()) node.remove();
      this.mounted.clear();
    }
    this.nearest = undefined;
  }

  private layout(): void {
    this.discard();
    this.place();
  }

  /** Where each item starts, from the heights their kinds say and the heights
   *  the measured ones turned out to be. Nothing here touches the rows already
   *  standing: a row that still says what it said keeps its place in the
   *  document and is only told its new top. */
  private place(): void {
    this.byKey = new Map(this.items.map((item) => [item.key, item]));
    this.nesting = buildNesting(this.items);
    this.tops = new Array(this.items.length + 1);
    this.tops[0] = 0;
    for (const [i, item] of this.items.entries()) {
      this.tops[i + 1] = (this.tops[i] ?? 0) + (this.measured.get(item.key) ?? heightOf(item));
    }
    this.canvas.style.height = `${this.tops[this.items.length] ?? 0}px`;
    this.drawn = null;
  }

  private anchor(): { readonly key: string; readonly delta: number } | null {
    const i = this.indexAt(this.scroller.scrollTop);
    const item = this.items[i];
    if (item === undefined) return null;
    return { key: item.key, delta: (this.tops[i] ?? 0) - this.scroller.scrollTop };
  }

  private restore(anchor: { readonly key: string; readonly delta: number }): void {
    const i = this.items.findIndex((item) => item.key === anchor.key);
    if (i < 0) return;
    this.scroller.scrollTop = (this.tops[i] ?? 0) - anchor.delta;
  }

  /** The item covering a pixel, by bisection over the running totals. */
  private indexAt(y: number): number {
    let lo = 0;
    let hi = this.items.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if ((this.tops[mid] ?? 0) <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  // ----- drawing -----

  private paint(): void {
    // A file nothing has a template for flattens to nothing. Leaving the last
    // file's rows up would show one file's structure over another's bytes.
    // Read the scroll position once. Everything below this line writes to the
    // document, and asking for it again afterwards makes the browser lay the
    // new rows out on the spot to answer.
    const top = this.scroller.scrollTop;
    const height = this.scroller.clientHeight;
    if (this.items.length === 0) {
      const note = el("p", "rp-empty", this.doc.template === null ? NO_TEMPLATE_HINT : this.matched ? REPORT.reading : NO_TEMPLATE_MATCH);
      this.discard();
      this.canvas.replaceChildren(note);
      this.drawn = null;
      return;
    }
    const from = Math.max(0, this.indexAt(top) - OVERSCAN);
    const to = Math.min(this.items.length, this.indexAt(top + height) + 1 + OVERSCAN);
    if (this.drawn !== null && this.drawn.from === from && this.drawn.to === to) return;
    this.drawn = { from, to };
    // Nothing is standing, so anything in the canvas is left over from the
    // note an empty listing puts there.
    if (this.mounted.size === 0) this.canvas.replaceChildren();
    if (this.nearest === undefined) this.nearest = this.nearestToSelection();
    const fileBits = this.doc.lengthBits;
    const context = this.context();
    const keep = new Set<string>();
    for (let i = from; i < to; i++) {
      const item = this.items[i];
      if (item === undefined) continue;
      keep.add(item.key);
      let node = this.mounted.get(item.key);
      if (node === undefined) {
        node = drawItem(context, item, fileBits);
        node.dataset["key"] = item.key;
        node.id = `rp-${item.key}`;
        // The scroller is the tree and keeps the focus; which item the keyboard
        // is on is said by `aria-activedescendant`, so a screen reader hears the
        // row without the focus ever leaving the one scrolling element.
        node.setAttribute("role", "treeitem");
        const opens = openKeyOf(item);
        if (opens !== null) node.setAttribute("aria-expanded", String(this.state.open.has(opens)));
        if (item.key === this.cursor) node.classList.add("is-cursor");
        this.mounted.set(item.key, node);
        this.canvas.append(node);
      }
      node.style.top = `${this.tops[i] ?? 0}px`;
      if (item.key === this.cursor) this.scroller.setAttribute("aria-activedescendant", node.id);
    }
    for (const [key, node] of this.mounted) {
      if (keep.has(key)) continue;
      node.remove();
      this.mounted.delete(key);
    }
    // Taken from the top of the window rather than from `from`, which reaches
    // a few rows above it and would name the heading before this one.
    this.trail(top);
    this.remeasure(from, to);
    const first = this.items[this.indexAt(top)];
    const last = this.items[Math.min(this.items.length - 1, this.indexAt(top + height))];
    if (first !== undefined && last !== undefined) {
      this.onViewport({ startBit: first.offsetBits, endBit: Math.max(first.offsetBits, last.offsetBits + last.sizeBits) });
    }
  }

  /** Take the height of anything that had to be guessed, and lay out again if
   *  the guess was wrong. Writing back only a changed height is what stops
   *  this and `paint` calling each other for ever. */
  private remeasure(from: number, to: number): void {
    if (this.measuring) return;
    // A hidden view has no width, and chips measured against no width wrap one
    // to a line and come out three times too tall. That height would then be
    // kept and used once the view was shown.
    const width = this.scroller.clientWidth;
    if (width === 0) return;
    this.measuredAt = width;
    let changed = false;
    for (let i = from; i < to; i++) {
      const item = this.items[i];
      if (item === undefined || !isMeasured(item)) continue;
      const node = this.mounted.get(item.key);
      if (node === undefined) continue;
      const height = Math.ceil(node.getBoundingClientRect().height);
      if (height > 0 && this.measured.get(item.key) !== height) {
        this.measured.set(item.key, height);
        changed = true;
      }
    }
    if (!changed) return;
    // Laying out again draws again, which measures again. The heights are
    // written back only when they differ, so the second pass finds nothing to
    // change and it stops; the flag is there in case a strip is ever drawn at
    // a height that depends on where it is.
    this.measuring = true;
    const anchor = this.anchor();
    this.layout();
    if (anchor !== null) this.restore(anchor);
    this.paint();
    this.measuring = false;
  }

  /** What the top of the window is inside, which is the headings it has
   *  scrolled past. Sticky, because a listing scrolled far enough that its
   *  heading is gone stops saying which part of the file it is showing. */
  private trail(scrollTop: number): void {
    const at = this.indexAt(scrollTop);
    const parts: string[] = [this.doc.name];
    const fileBits = this.doc.lengthBits;
    let section: string | null = null;
    let part: string | null = null;
    for (let i = Math.min(at, this.items.length - 1); i >= 0; i--) {
      const item = this.items[i];
      if (item === undefined || item.kind !== "heading") continue;
      const name = headingTitle(item, fileBits);
      if (item.level === 1 && part === null && section === null) part = name;
      if (item.level === 0) {
        section = name;
        break;
      }
    }
    if (section !== null) parts.push(section);
    if (part !== null) parts.push(part);
    this.crumbs.replaceChildren(
      ...parts.flatMap((text, i) => {
        const span = el("span", i === parts.length - 1 ? "rp-crumb is-here" : "rp-crumb", text);
        return i === 0 ? [span] : [el("span", "rp-crumb-sep", "\u203a"), span];
      }),
    );
  }

  /** What the drawing is allowed to see of this view, gathered once per paint.
   *  Everything in it changes between paints and nothing in it is written to
   *  from the other side, so a fresh reading each time is both cheaper than
   *  keeping it in step and harder to get wrong. */
  private context(): DrawContext {
    return {
      doc: this.doc,
      segments: this.segments,
      selected: this.selected,
      nearest: this.nearest,
      bytes: this.state.bytes,
      dumps: this.dumps,
      dumpTops: this.dumpTops,
      cards: this.cards,
      toggleBytes: (key) => this.toggleBytes(key),
      toggleDump: (at) => this.toggleDump(at),
      toggleCard: (key) => this.toggleCard(key),
      verdict: (item) => this.verdict(item),
      shown: this.scroller.clientWidth > 0,
    };
  }

  /** Open the long half of a format card, or put it away. The card is a
   *  different height now and its height is measured rather than declared,
   *  so the measurement goes and the list is laid out again. */
  private toggleCard(key: string): void {
    if (this.cards.has(key)) this.cards.delete(key);
    else this.cards.add(key);
    for (const k of [...this.measured.keys()]) if (k.startsWith("fc:")) this.measured.delete(k);

    this.layout();
    this.paint();
  }

  /** Open one field out into all of its bytes, or put it away. */
  private toggleDump(offsetBits: number): void {
    if (this.dumps.has(offsetBits)) {
      this.dumps.delete(offsetBits);
      this.dumpTops.delete(offsetBits);
    } else this.dumps.add(offsetBits);
    // The strip is taller or shorter now, and its height is measured rather
    // than declared, so the measurement goes and the list is laid out again.
    for (const k of [...this.measured.keys()]) if (k.startsWith("bytes:")) this.measured.delete(k);
    this.layout();
    this.paint();
  }

  private toggleBytes(key: string): void {
    const bytes = new Set(this.state.bytes);
    let open = this.state.open;
    // Found before the state moves. A strip belongs to the item whose key it
    // is, so that item is the one the splice walks again — the same run of the
    // list a fold on it would have replaced.
    const at = this.items.findIndex((item) => item.key === key);
    if (bytes.has(key)) {
      bytes.delete(key);
      this.pruneDumps(key);
    } else {
      bytes.add(key);
      // A strip is part of what its item opens into, not a second expansion
      // beside it: asking a closed item for its bytes opens the item, so a
      // strip never stands over rows that are hidden.
      const target = openTargetOf(key);
      if (target !== null && !open.has(target)) open = new Set([...open, target]);
    }
    this.state = { ...this.state, bytes, open };
    if (at >= 0 && this.splice(at)) return;
    this.rebuild();
  }

  /** The dumps open inside one strip go with it. Without this, a dump closed
   *  along with its strip would be standing open again when the strip came
   *  back, which is state the reader put away coming back on its own. */
  private pruneDumps(bytesKey: string): void {
    const strip = this.items.find((i) => i.key === `bytes:${bytesKey}`);
    if (strip === undefined) return;
    for (const at of [...this.dumps]) {
      if (at >= strip.offsetBits && at < strip.offsetBits + strip.sizeBits) {
        this.dumps.delete(at);
        this.dumpTops.delete(at);
      }
    }
  }

  /** Whether a stretch of bytes is the selected one. Equality, not overlap: a
   *  field sits inside its structure, and lighting everything the selection is
   *  inside would light most of the screen. */
  private isSelected(offsetBits: number, sizeBits: number): boolean {
    return isSelected(this.selected, offsetBits, sizeBits);
  }

  /** Whether a stretch holds the selection. For the one row that is allowed
   *  to say so: the nearest thing on screen that contains it, when the field
   *  itself is not a row of its own. */
  private holdsSelection(offsetBits: number, sizeBits: number): boolean {
    return holdsSelection(this.selected, offsetBits, sizeBits);
  }

  /** The item to light when the selected field has no row of its own: the
   *  smallest one on the list that contains it. A record shows its rows
   *  itself and a long list stops at a page, so the field the cursor is in is
   *  often not a row, and saying nothing at all would lose the reader. */
  private nearestToSelection(): string | null {
    const s = this.selected;
    if (s === null) return null;
    const n = this.nesting;
    if (n === null) {
      let best: { key: string; size: number } | null = null;
      for (const item of this.items) {
        if (item.kind !== "row" && item.kind !== "record" && item.kind !== "formatcard") continue;
        if (!this.holdsSelection(item.offsetBits, item.sizeBits)) continue;
        if (best === null || item.sizeBits < best.size) best = { key: item.key, size: item.sizeBits };
      }
      return best?.key ?? null;
    }
    return holder(n, s.offsetBits, s.offsetBits + s.sizeBits);
  }

  private verdict(item: Extract<Item, { kind: "gap" }>): GapVerdict {
    const known = this.verdicts.get(item.key);
    if (known !== undefined) return known;
    const found = checkGap(this.doc, item.offsetBits, item.sizeBits);
    // A run whose bytes have not arrived is not an answer, so it is not kept:
    // the next draw after they land asks again.
    if (found !== "unread") this.verdicts.set(item.key, found);
    return found;
  }

  // ----- input -----

  private onClick(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof Element)) return;
    const host = target.closest<HTMLElement>(".rp-item");
    const key = host?.dataset["key"];
    if (key === undefined) return;
    const item = this.items.find((i) => i.key === key);
    if (item === undefined) return;
    const reads = target.closest<HTMLElement>("[data-reads]")?.dataset["reads"];
    if (reads !== undefined) {
      this.reveal(reads === "" ? [] : reads.split(".").map(Number));
      return;
    }
    const list = target.closest<HTMLElement>("[data-list]")?.dataset["list"];
    if (list !== undefined) {
      this.onOpenList(list === "" ? [] : list.split(".").map(Number));
      return;
    }
    const wants = target.closest<HTMLElement>("[data-bytes]")?.dataset["bytes"];
    if (wants !== undefined) {
      this.toggleBytes(wants);
      return;
    }
    // A click inside an open strip is for the strip, not for opening whatever
    // the strip is sitting under. The content card answers its own clicks.
    if (item.kind === "bytes" || item.kind === "card" || item.kind === "formatcard") return;
    this.cursor = item.key;
    this.activate(item);
  }

  /** What clicking an item does, which is also what Enter on it does. */
  private activate(item: Item): void {
    if (item.kind === "more") {
      const shown = new Map(this.state.shown);
      const win = item.side === "later" ? { from: item.from, to: item.to + PAGE } : { from: Math.max(0, item.from - PAGE), to: item.to };
      shown.set(pathString(item.path), win);
      this.state = { ...this.state, shown };
      this.rebuild();
      return;
    }
    const openKey = openKeyOf(item);
    if (openKey !== null) this.setOpen(openKey, !this.state.open.has(openKey));
    if (item.kind === "row") this.pick(item);
  }

  /** Say what is selected, both here and to everything else showing it. */
  private pick(item: Extract<Item, { kind: "row" }>): void {
    this.select(item.path, item.offsetBits, item.sizeBits);
    this.picking = true;
    this.onPick({ path: item.path, startBit: item.offsetBits, endBit: item.offsetBits + item.sizeBits });
    this.picking = false;
  }

  private setOpen(key: string, want: boolean): void {
    if (this.state.open.has(key) === want) return;
    // Found before the state moves, since it is this list that the new one is
    // spliced into.
    const at = this.items.findIndex((item) => openKeyOf(item) === key);
    const open = new Set(this.state.open);
    let bytes = this.state.bytes;
    if (want) open.add(key);
    else {
      open.delete(key);
      // Closing an item takes its strip with it: closed, open, and open with
      // bytes are the only states there are.
      const owned = [`h:${key}`, `r:${key}`].filter((k) => bytes.has(k));
      if (owned.length > 0) {
        const next = new Set(bytes);
        for (const k of owned) {
          next.delete(k);
          this.pruneDumps(k);
        }
        bytes = next;
      }
    }
    this.state = { ...this.state, open, bytes };
    if (at >= 0 && this.splice(at)) return;
    this.rebuild();
  }

  /**
   * Redraw the one fold that moved, rather than the file.
   *
   * Opening a structure changes what is under it and leaves everything else
   * where it was, so the rows above keep their places in the document and the
   * scroll position needs no anchor: it was never going to move. False when
   * this item has no fold of its own to walk, which is the caller's cue to
   * walk the whole tree.
   */
  private splice(at: number): boolean {
    const cut = refold(this.src, this.state, this.flatOpts(), this.items, at);
    if (cut === null) return false;
    const top = this.tops[cut.from] ?? 0;
    const was = (this.tops[cut.to] ?? top) - top;
    this.items = [...this.items.slice(0, cut.from), ...cut.items, ...this.items.slice(cut.to)];
    const live = new Set(this.items.map((i) => i.key));
    for (const key of [...this.measured.keys()]) if (!live.has(key)) this.measured.delete(key);
    for (const [key, node] of [...this.mounted]) {
      // The item itself says whether it is open, so it is not the row it was.
      if (live.has(key) && key !== cut.items[0]?.key) continue;
      node.remove();
      this.mounted.delete(key);
    }
    this.place();
    // A fold that moved above the window would otherwise push everything the
    // reader is looking at up or down by however much it grew.
    if (top < this.scroller.scrollTop) {
      const now = (this.tops[cut.from + cut.items.length] ?? top) - top;
      if (now !== was) this.scroller.scrollTop += now - was;
    }
    this.remark();
    this.paint();
    return true;
  }

  /** Tell the rows on screen whether they are the selected one, when what is
   *  selected has not moved but what stands between it and a row of its own
   *  has. */
  private remark(): void {
    const before = this.nearest;
    this.nearest = this.nearestToSelection();
    if (before === this.nearest) return;
    for (const [key, node] of this.mounted) {
      const item = this.byKey.get(key);
      if (item === undefined || item.kind !== "row") continue;
      node.classList.toggle("is-on", this.isSelected(item.offsetBits, item.sizeBits) || this.nearest === key);
    }
  }

  // ----- keyboard -----

  /** Where the keyboard is. The top of the window until the reader moves it,
   *  so the first arrow press starts from what they are looking at rather
   *  than from the top of the file. */
  private cursorAt(): number {
    const i = this.cursor === null ? -1 : this.items.findIndex((item) => item.key === this.cursor);
    return i < 0 ? this.indexAt(this.scroller.scrollTop) : i;
  }

  private onKey(e: KeyboardEvent): void {
    if (this.items.length === 0) return;
    const at = this.cursorAt();
    const item = this.items[at];
    const page = Math.max(1, Math.floor(this.scroller.clientHeight / HEIGHT.row) - 1);
    switch (e.key) {
      case "ArrowDown": this.moveTo(at + 1); break;
      case "ArrowUp": this.moveTo(at - 1); break;
      case "PageDown": this.moveTo(at + page); break;
      case "PageUp": this.moveTo(at - page); break;
      case "Home": this.moveTo(0); break;
      case "End": this.moveTo(this.items.length - 1); break;
      // Sideways opens and closes, as in any tree. A row with nothing to open
      // swallows the key rather than scrolling the view sideways under it.
      case "ArrowRight": if (item !== undefined) this.reopen(item, true); break;
      case "ArrowLeft": if (item !== undefined) this.reopen(item, false); break;
      case "Enter":
      case " ": if (item !== undefined) this.activate(item); break;
      default: return;
    }
    e.preventDefault();
  }

  private reopen(item: Item, want: boolean): void {
    const key = openKeyOf(item);
    if (key !== null) this.setOpen(key, want);
  }

  /** Put the keyboard on an item, bring it on screen, and select it if it is
   *  a field: arrowing down a list is reading it, and everything else showing
   *  the file follows the same selection a click would make. */
  private moveTo(i: number): void {
    const at = Math.max(0, Math.min(this.items.length - 1, i));
    const item = this.items[at];
    if (item === undefined) return;
    this.cursor = item.key;
    const top = this.tops[at] ?? 0;
    const bottom = this.tops[at + 1] ?? top;
    if (top < this.scroller.scrollTop) this.scroller.scrollTop = top;
    else if (bottom > this.scroller.scrollTop + this.scroller.clientHeight) this.scroller.scrollTop = bottom - this.scroller.clientHeight;
    if (item.kind === "row") this.pick(item);
    else this.restyle();
  }

  /** Redraw the window that is already up, for a change that moves nothing. */
  private paintAgain(): void {
    this.discard();
    this.drawn = null;
    this.paint();
  }

  private select(path: readonly number[], offsetBits: number, sizeBits: number): void {
    this.selected = { path, offsetBits, sizeBits };
    this.restyle();
  }

  /**
   * The windows on the long lists between the root and `path`, moved so that
   * each step through one is drawn.
   *
   * A page at a time, aligned to page boundaries, so that arriving at element
   * 19,974 draws elements 19,800 to 20,000 rather than one lone row: the
   * reader clicked bytes in the middle of a run and what they want to see is
   * the run. A window already holding the step is left where it is, so
   * following the cursor along a list does not keep jumping it.
   */
  private framed(path: readonly number[]): Map<string, Window> {
    const shown = new Map(this.state.shown);
    for (let n = 0; n < path.length; n++) {
      const at = pathString(path.slice(0, n));
      const i = path[n] ?? 0;
      const win = shown.get(at) ?? { from: 0, to: PAGE };
      if (i >= win.from && i < win.to) continue;
      const from = Math.floor(i / PAGE) * PAGE;
      shown.set(at, { from, to: from + PAGE });
    }
    return shown;
  }

  /** Bring the field at `path` on screen and select it. */
  reveal(path: readonly number[]): void {
    const node = this.doc.templateNode(path);
    if (node.status !== "ok") return;
    this.selected = { path, offsetBits: node.node.offset_bits, sizeBits: node.node.size_bits };
    const key = `r:${pathString(path)}`;
    let i = this.items.findIndex((item) => item.key === key);
    if (i < 0) {
      // The field is inside something still closed: open every step down to it,
      // and move the window of every long list on the way so that the step
      // through it is one of the elements drawn.
      const open = new Set(this.state.open);
      for (let n = 1; n <= path.length; n++) open.add(pathString(path.slice(0, n)));
      this.state = { ...this.state, open, shown: this.framed(path) };
      this.rebuild();
      i = this.items.findIndex((item) => item.key === key);
    }
    // A part big enough for a heading has no row: a SQLite page reached from
    // a schema row's `rootpage` is a section of the file, and its heading is
    // where the reader is being sent.
    if (i < 0) i = this.items.findIndex((item) => item.key === `h:${pathString(path)}`);
    // The row may not be there at all: a table shows its rows itself, and a
    // list past its first page does not reach that far. Then the nearest thing
    // that holds it is what to go to, since that is what will be lit.
    if (i < 0) {
      this.nearest = this.nearestToSelection();
      i = this.nearest === null ? -1 : this.items.findIndex((item) => item.key === this.nearest);
    }
    if (i < 0) {
      this.restyle();
      return;
    }
    this.scroller.scrollTop = Math.max(0, (this.tops[i] ?? 0) - this.scroller.clientHeight / 3);
    this.restyle();
  }

  /** Follow the cursor: select whatever field covers this bit. */
  setBit(bit: number): void {
    if (this.picking || this.el.hidden) return;
    const at = this.doc.locate(bit);
    if (at.status !== "ok") return;
    // A cursor moved one byte is usually still inside the field it was in, and
    // then the listing has nothing to say that it is not already saying.
    const now = this.selected;
    if (now !== null && now.path.length === at.node.length && now.path.every((p, i) => p === at.node[i])) return;
    this.reveal(at.node);
  }

  /** Whether the file matched a template at all, which decides what an empty
   *  listing has to say for itself. */
  setMatched(matched: boolean): void {
    this.matched = matched;
    this.paintAgain();
  }

  clearSelection(): void {
    this.selected = null;
    this.restyle();
  }
}

function pathString(path: readonly number[]): string {
  return path.join(".");
}

/**
 * The rows and record tables of the list as a forest of nested stretches of
 * the file.
 *
 * `start` is non-decreasing, so the last entry beginning at or before a bit is
 * found by bisection; everything else that could hold that bit is one of its
 * containers, which `parent` chains together. A list of ten thousand names
 * gives a chain a few links long, which is the point: the question is "what is
 * this inside", and the answer never depended on the other nine thousand.
 */
type Nesting = {
  readonly start: Float64Array;
  readonly end: Float64Array;
  readonly parent: Int32Array;
  readonly key: readonly string[];
};

/** Null when the items are not in address order, which leaves the caller its
 *  own reading of the list. The walk lays them out in file order, so this is
 *  about a template that can surprise it rather than about a case that is
 *  expected. */
function buildNesting(items: readonly Item[]): Nesting | null {
  const start: number[] = [];
  const end: number[] = [];
  const parent: number[] = [];
  const key: string[] = [];
  // Indices of the entries the current one is inside, innermost last.
  const inside: number[] = [];
  for (const item of items) {
    if (item.kind !== "row" && item.kind !== "record") continue;
    const from = item.offsetBits;
    const to = from + item.sizeBits;
    if (from < (start[start.length - 1] ?? -Infinity)) return null;
    while (inside.length > 0) {
      const top = inside[inside.length - 1] ?? 0;
      if ((start[top] ?? 0) <= from && (end[top] ?? 0) >= to) break;
      inside.pop();
    }
    parent.push(inside.length === 0 ? -1 : (inside[inside.length - 1] ?? -1));
    inside.push(start.length);
    start.push(from);
    end.push(to);
    key.push(item.key);
  }
  return { start: Float64Array.from(start), end: Float64Array.from(end), parent: Int32Array.from(parent), key };
}

/**
 * The smallest entry covering `[from, to)`, and the earliest of those where
 * several are the same size, which is what reading the list in order gave.
 *
 * A selection of no bits belongs to the entry that begins where it is, not to
 * the one that ended there — the same reading `HexView.highlightBits` gives a
 * run of no bits, and the one thing here that differs from reading the whole
 * list, which had no way to tell the two apart.
 */
function holder(n: Nesting, from: number, to: number): string | null {
  let lo = 0;
  let hi = n.start.length - 1;
  let at = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((n.start[mid] ?? 0) <= from) {
      at = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  let best = -1;
  for (let i = at; i >= 0; i = n.parent[i] ?? -1) {
    if ((n.end[i] ?? 0) >= to) {
      best = i;
      break;
    }
  }
  if (best < 0) return null;
  // A container of the same extent as what it contains is the same stretch of
  // the file said twice, and the list named the outer one first.
  const size = (n.end[best] ?? 0) - (n.start[best] ?? 0);
  for (let i = n.parent[best] ?? -1; i >= 0; i = n.parent[i] ?? -1) {
    if ((n.end[i] ?? 0) - (n.start[i] ?? 0) !== size) break;
    best = i;
  }
  return n.key[best] ?? null;
}

/** The state key a click on this item turns on and off, or null for an item
 *  with nothing inside it. */
function openKeyOf(item: Item): string | null {
  if (item.kind === "heading") return item.node === null || item.level === 0 ? null : pathString(item.path);
  if (item.kind === "row") return itemOpens(item.node) ? pathString(item.path) : null;
  return null;
}

/** The open-state key behind a bytes key, for the items that have one. A run
 *  heading (`h:path[a-b]`) is always open and has no key; a leaf row's or a
 *  record row's path lands in `open` unread, which costs nothing. */
function openTargetOf(bytesKey: string): string | null {
  if (bytesKey.startsWith("h:")) {
    const path = bytesKey.slice(2);
    return path.includes("[") ? null : path;
  }
  if (bytesKey.startsWith("r:")) return bytesKey.slice(2);
  return null;
}



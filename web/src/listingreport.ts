// The listing as one scrolling report: the file's parts as headings, their
// fields as rows, and the bytes nothing claimed as rows of their own.
//
// `flatten` says what to draw and in what order; this says how tall each thing
// is and puts the ones on screen in the document. Everything else stays out of
// it, so a file whose tree runs to millions of nodes costs a screenful of DOM.
//
// Scrolling counts items, not bits. The old listing scrolls the file itself
// because `spans` is windowed by bit range and cannot say how many rows a file
// has; a flattened tree is a list with a length, and a list scrolls by index.

import { formatBytes, formatOffset } from "./doc.js";
import type { Doc, TemplateNode } from "./doc.js";
import type { FieldPick } from "./doc.js";
import { emptyState, flatten, PAGE } from "./flatten.js";
import type { FlatOptions, Item, ListingState, TreeSource, Window } from "./flatten.js";
import { fieldClass, sectionColor } from "./fieldstyle.js";
import { byteStrip } from "./bytestrip.js";
import { fileMap } from "./filemap.js";
import { checkGap } from "./gapcheck.js";
import { isRecordList, recordTable } from "./records.js";
import type { RecordCell } from "./records.js";
import type { GapVerdict } from "./gapcheck.js";
import type { MapSegment } from "./filemap.js";
import { bitSizeText, childWord, countText, GAP_LABEL, NO_TEMPLATE_HINT, NO_TEMPLATE_MATCH, REPORT } from "./strings.js";

/** Row heights, which must match `--rp-*` in the stylesheet: the tops of every
 *  item are a running total of these, and a row that draws taller than it was
 *  measured at would slide out from under its own place. */
const HEIGHT = { section: 40, part: 26, row: 22, strip: 96 } as const;
/** Items drawn above and below the window, so a wheel notch has somewhere to
 *  go before the next paint. */
const OVERSCAN = 6;

/** What an item is worth before it has been drawn. A byte strip's chips wrap
 *  to the width they are given, so its real height is only known once it is
 *  in the document; this is the guess the first layout uses, and `measured`
 *  replaces it. */
function heightOf(item: Item): number {
  if (item.kind === "bytes" || item.kind === "record") return HEIGHT.strip;
  if (item.kind !== "heading") return HEIGHT.row;
  return item.level === 0 ? HEIGHT.section : HEIGHT.part;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** `0x1000 – 0x1fff`, the stretch a heading covers. A part of no bytes has no
 *  range to give, which is what a field placed somewhere else looks like. */
function rangeText(offsetBits: number, sizeBits: number): string {
  if (sizeBits === 0) return formatOffset(offsetBits);
  return `${formatOffset(offsetBits)} – ${formatOffset(offsetBits + sizeBits - 8)}`;
}

/** How much of the file this is, for a part big enough for the answer to mean
 *  anything. Under a per cent, the number says less than the range does. */
function shareText(sizeBits: number, fileBits: number): string {
  if (fileBits <= 0) return "";
  const share = sizeBits / fileBits;
  return share < 0.01 ? REPORT.tinyShare : `${Math.round(share * 100)}%`;
}

/** Where a run of plain fields sits in the file, which is all there is to
 *  name it by. A run at the front is a header; the same fields at the back
 *  are not. */
function runPosition(item: Item, fileBits: number): "start" | "end" | "middle" {
  if (item.offsetBits === 0) return "start";
  if (fileBits > 0 && item.offsetBits + item.sizeBits >= fileBits) return "end";
  return "middle";
}

/** What each answer from `checkGap` is called. */
const GAP_VERDICT = {
  zeros: REPORT.gapZeros,
  something: REPORT.gapNonzero,
  "too-large": REPORT.gapTooLarge,
  unread: REPORT.gapUnread,
} as const;

export class ListingReport {
  readonly el: HTMLElement;
  /** The bar above the rows saying what the top of the window is inside. */
  private readonly crumbs: HTMLElement;
  private readonly scroller: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly doc: Doc;
  private readonly src: TreeSource;
  private readonly opts: FlatOptions;
  private state: ListingState = emptyState;
  private items: readonly Item[] = [];
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
  /** True when the file changed while this was hidden, so the rows on screen
   *  are of a file that has moved on. Walked again when it comes back. */
  private stale = false;
  /** What is selected, as the bits it covers rather than as the row showing
   *  it. The same field appears as a row, as a line of a record table, as a
   *  column of an open byte strip and as a mark on every file map; all four
   *  are answered by the range, and only one of them by a row. */
  private selected: { readonly path: readonly number[]; readonly offsetBits: number; readonly sizeBits: number } | null = null;
  /** True while a selection this view made is being sent out, so the cursor
   *  move it causes does not come back and undo the scroll position. */
  private picking = false;
  /** The key of the row standing in for a selection with no row of its own,
   *  worked out once a paint rather than once a row. */
  private nearest: string | null = null;
  /** The item the keyboard is on, by key so that it survives a re-flatten.
   *  Null until an arrow key is pressed, which starts it at the top of the
   *  window rather than at the top of the file. */
  private cursor: string | null = null;
  /** Fields opened out into a dump of all their bytes, by the bit they start
   *  at, and where each dump is scrolled to. Neither can live in the strip:
   *  it is built again from nothing every time anything on screen changes. */
  private dumps = new Set<number>();
  private dumpTops = new Map<number, number>();

  onPick: (pick: FieldPick) => void = () => {};
  /** A long list was asked for on its own. The pane is the caller's, since
   *  where it goes on the screen is not this view's business. */
  onOpenList: (path: readonly number[]) => void = () => {};

  constructor(doc: Doc) {
    this.doc = doc;
    this.src = {
      node: (path) => doc.templateNode(path),
      children: (path, from, to) => doc.templateChildren(path, from, to),
    };
    this.opts = { isRecord: (node) => isRecordList(doc, node) };
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
      if (this.scroller.clientWidth !== this.measuredAt) this.measured.clear();
      this.relayout();
    }).observe(this.scroller);
    // Chunks arriving turn a pending stretch into rows; so does an edit.
    doc.onChange(() => {
      this.verdicts.clear();
      this.schedule();
    });
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
    this.items = flatten(this.src, this.state, this.opts).items;
    this.segments = this.items
      .filter((i) => i.kind === "heading" && i.level === 0)
      .map((i) => ({ offsetBits: i.offsetBits, sizeBits: i.sizeBits, color: sectionColor(i.section) }));
    // A strip that is no longer in the list keeps no height.
    const live = new Set(this.items.map((i) => i.key));
    for (const key of [...this.measured.keys()]) if (!live.has(key)) this.measured.delete(key);
    this.layout();
    if (anchor !== null) this.restore(anchor);
    this.paint();
    // A stretch still being read comes back as a pending item, so this runs
    // again when its chunks land: `Doc` has already asked for them.
  }

  /** Where every item starts. Anchoring and drawing are the caller's, since
   *  what to hold on to differs between walking the tree again and finding a
   *  strip was not the height it was taken for. */
  private layout(): void {
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
    if (this.items.length === 0) {
      const note = el("p", "rp-empty", this.doc.template === null ? NO_TEMPLATE_HINT : this.matched ? REPORT.reading : NO_TEMPLATE_MATCH);
      this.canvas.replaceChildren(note);
      this.drawn = null;
      return;
    }
    const from = Math.max(0, this.indexAt(this.scroller.scrollTop) - OVERSCAN);
    const to = Math.min(this.items.length, this.indexAt(this.scroller.scrollTop + this.scroller.clientHeight) + 1 + OVERSCAN);
    if (this.drawn !== null && this.drawn.from === from && this.drawn.to === to) return;
    this.drawn = { from, to };
    this.nearest = this.nearestToSelection();
    const fileBits = this.doc.lengthBits;
    const out: HTMLElement[] = [];
    for (let i = from; i < to; i++) {
      const item = this.items[i];
      if (item === undefined) continue;
      const node = this.draw(item, fileBits);
      node.style.top = `${this.tops[i] ?? 0}px`;
      node.dataset["key"] = item.key;
      node.id = `rp-${item.key}`;
      // The scroller is the tree and keeps the focus; which item the keyboard
      // is on is said by `aria-activedescendant`, so a screen reader hears the
      // row without the focus ever leaving the one scrolling element.
      node.setAttribute("role", "treeitem");
      const opens = openKeyOf(item);
      if (opens !== null) node.setAttribute("aria-expanded", String(this.state.open.has(opens)));
      if (item.key === this.cursor) {
        node.classList.add("is-cursor");
        this.scroller.setAttribute("aria-activedescendant", node.id);
      }
      out.push(node);
    }
    this.canvas.replaceChildren(...out);
    // Taken from the top of the window rather than from `from`, which reaches
    // a few rows above it and would name the heading before this one.
    this.trail();
    this.remeasure(from, to);
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
      if (item === undefined || (item.kind !== "bytes" && item.kind !== "record")) continue;
      const node = this.canvas.querySelector<HTMLElement>(`[data-key="${CSS.escape(item.key)}"]`);
      if (node === null) continue;
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
  private trail(): void {
    const at = this.indexAt(this.scroller.scrollTop);
    const parts: string[] = [this.doc.name];
    const fileBits = this.doc.lengthBits;
    let section: string | null = null;
    let part: string | null = null;
    for (let i = Math.min(at, this.items.length - 1); i >= 0; i--) {
      const item = this.items[i];
      if (item === undefined || item.kind !== "heading") continue;
      const name = item.node?.name ?? REPORT.unnamedPart(runPosition(item, fileBits));
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

  private draw(item: Item, fileBits: number): HTMLElement {
    switch (item.kind) {
      case "heading":
        return this.drawHeading(item, fileBits);
      case "row":
        return this.drawRow(item);
      case "gap":
        return this.drawGap(item);
      case "bytes":
        return this.drawStrip(item);
      case "record":
        return this.drawRecord(item);
      case "more":
        return this.drawMore(item);
      case "pending":
        return el("div", "rp-item rp-pending", REPORT.reading);
    }
  }

  private drawHeading(item: Extract<Item, { kind: "heading" }>, fileBits: number): HTMLElement {
    const row = el("div", `rp-item rp-h${item.level}`);
    if (item.level === 0) {
      const swatch = el("span", "rp-swatch");
      swatch.style.background = sectionColor(item.section);
      row.append(swatch);
    }
    row.append(el("b", "rp-name", item.node?.name ?? REPORT.unnamedPart(runPosition(item, fileBits))));
    row.append(el("span", "rp-range", rangeText(item.offsetBits, item.sizeBits)));
    const share = shareText(item.sizeBits, fileBits);
    row.append(el("span", "rp-size", `${formatBytes(item.sizeBits / 8)}${share === "" ? "" : ` · ${share}`}`));
    row.append(this.bytesButton(item.key));
    // Only a list too long to draw: for anything the window already holds
    // whole, a pane of its own would be the same rows somewhere else.
    if (item.node !== null && item.node.child_count > PAGE) row.append(this.listButton(item.path));
    row.append(this.mapFor(item));
    return row;
  }

  /** A structure the format keeps as a table, drawn as one: the format's own
   *  column names, and where each row is written. */
  private drawRecord(item: Extract<Item, { kind: "record" }>): HTMLElement {
    const host = el("div", "rp-item rp-record");
    host.style.paddingLeft = `${8 + item.depth * 12}px`;
    const table = recordTable(this.doc, item.node);
    if (table === null) {
      host.append(el("div", "bs-wait", REPORT.reading));
      return host;
    }
    const grid = document.createElement("table");
    grid.className = "rec";
    const head = document.createElement("tr");
    for (const name of table.columns) head.append(el("th", "", name));
    head.append(el("th", "rec-at", REPORT.storedAt));
    grid.append(head);
    for (const row of table.rows) {
      const tr = document.createElement("tr");
      // A table row is a range, not a field: the selection is usually one
      // column inside it.
      if (this.holdsSelection(row.offsetBits, row.sizeBits)) tr.className = "is-on";
      for (const cell of row.cells) tr.append(this.drawCell(cell));
      const at = el("td", "rec-at");
      // The one way out of the table: the row's own bytes, which is where it
      // was read from and where the reader goes to see how.
      const link = el("button", "rec-link", `${formatOffset(row.offsetBits)} \u00b7 ${formatBytes(row.sizeBits / 8)}`);
      link.type = "button";
      // Back to the fields: the row's own bytes, under the table it came from.
      const rowKey = `r:${row.path.join(".")}`;
      if (this.state.bytes.has(rowKey)) link.classList.add("is-on");
      link.addEventListener("click", (e) => {
        e.stopPropagation();
        this.toggleBytes(rowKey);
      });
      at.append(link);
      tr.append(at);
      grid.append(tr);
    }
    host.append(grid);
    if (table.pending) host.append(el("div", "bs-wait", REPORT.reading));
    return host;
  }

  /** One cell of a record table. A value that names another part of the file
   *  is a link there, which is rule 7's cross-reference: `data-reads` is the
   *  same route the rows' "→ cells" links already take. */
  private drawCell(cell: RecordCell): HTMLElement {
    const td = el("td", fieldClass(cell.kind));
    if (cell.link === undefined) {
      td.textContent = cell.text;
      return td;
    }
    const link = el("button", "rec-link", cell.link.text);
    link.type = "button";
    link.title = cell.link.label;
    link.setAttribute("aria-label", cell.link.label);
    link.dataset["reads"] = pathString(cell.link.path);
    td.append(link);
    return td;
  }

  private drawStrip(item: Extract<Item, { kind: "bytes" }>): HTMLElement {
    const host = el("div", "rp-item rp-strip");
    host.style.paddingLeft = `${8 + item.depth * 12}px`;
    const caption = `${item.name} ${rangeText(item.offsetBits, item.sizeBits)}`;
    host.append(
      byteStrip(this.doc, item.offsetBits, item.sizeBits, caption, this.mapFor(item), () => this.toggleBytes(item.owner), this.selected, {
        open: this.dumps,
        toggle: (at) => this.toggleDump(at),
        scroll: (at) => ({ get: () => this.dumpTops.get(at) ?? 0, set: (top) => this.dumpTops.set(at, top) }),
      }),
    );
    return host;
  }

  private mapFor(item: Item): HTMLElement {
    return fileMap(this.segments, item.offsetBits, item.sizeBits, rangeText(item.offsetBits, item.sizeBits), this.selected);
  }

  /** The control that shows an item's bytes, and takes them away again. */
  private bytesButton(key: string): HTMLElement {
    const on = this.state.bytes.has(key);
    const b = el("button", `rp-bytes${on ? " is-on" : ""}`, REPORT.showBytes);
    b.type = "button";
    b.setAttribute("aria-pressed", String(on));
    b.dataset["bytes"] = key;
    return b;
  }

  /** The way out of a window and into the whole list. It sits on the list's
   *  own heading and on both ends of the drawn window, which is where a reader
   *  finds out the list is longer than what is in front of them. */
  private listButton(path: readonly number[]): HTMLElement {
    const b = el("button", "rp-bytes rp-list", REPORT.paneOpen);
    b.type = "button";
    b.dataset["list"] = pathString(path);
    return b;
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
    if (bytes.has(key)) {
      bytes.delete(key);
      this.pruneDumps(key);
    } else {
      bytes.add(key);
      // A strip is part of what its item opens into, not a second expansion
      // beside it: asking a closed item for its bytes opens the item, so a
      // strip never stands over rows that are hidden.
      const at = openTargetOf(key);
      if (at !== null && !open.has(at)) open = new Set([...open, at]);
    }
    this.state = { ...this.state, bytes, open };
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
    return this.selected !== null && this.selected.offsetBits === offsetBits && this.selected.sizeBits === sizeBits;
  }

  /** Whether a stretch holds the selection. For the one row that is allowed
   *  to say so: the nearest thing on screen that contains it, when the field
   *  itself is not a row of its own. */
  private holdsSelection(offsetBits: number, sizeBits: number): boolean {
    const s = this.selected;
    return s !== null && s.offsetBits >= offsetBits && s.offsetBits + s.sizeBits <= offsetBits + sizeBits;
  }

  /** The item to light when the selected field has no row of its own: the
   *  smallest one on the list that contains it. A record shows its rows
   *  itself and a long list stops at a page, so the field the cursor is in is
   *  often not a row, and saying nothing at all would lose the reader. */
  private nearestToSelection(): string | null {
    const s = this.selected;
    if (s === null) return null;
    let best: { key: string; size: number } | null = null;
    for (const item of this.items) {
      if (item.kind !== "row" && item.kind !== "record") continue;
      if (!this.holdsSelection(item.offsetBits, item.sizeBits)) continue;
      if (best === null || item.sizeBits < best.size) best = { key: item.key, size: item.sizeBits };
    }
    return best?.key ?? null;
  }

  private drawRow(item: Extract<Item, { kind: "row" }>): HTMLElement {
    const n = item.node;
    // A field of no bytes is grey: whether it is a value the template worked
    // out or a list that turned out to be empty, there is nothing of it in the
    // file, and a row the reader can skip should look like one.
    const row = el("div", `rp-item rp-row${n.size_bits === 0 ? " rp-nobytes" : ""}`);
    if (this.isSelected(item.offsetBits, item.sizeBits) || this.nearest === item.key) row.classList.add("is-on");
    row.style.paddingLeft = `${8 + item.depth * 12}px`;
    // A computed value is not written anywhere, so it has no address, and its
    // length says so in words: "0x101a7" and "0 bytes" would be answers to
    // questions this row is not the answer to.
    const written = n.type !== "computed";
    row.append(el("span", "rp-at", written ? formatOffset(n.offset_bits) : ""));
    // A row that opens says so. Without it the only way to find out which
    // rows have anything under them is to click every one of them.
    row.append(el("span", "rp-twist", itemOpens(n) ? (item.open ? "\u25be" : "\u25b8") : ""));
    row.append(el("span", `rp-field ${fieldClass(n.kind)}`, n.name));
    const value = el("span", "rp-value", n.composite ? countText(n.child_count, childWord(n)) : n.value);
    if (item.reads !== null) value.append(this.readsLink(item.reads));
    row.append(value);
    row.append(el("span", "rp-type", n.type));
    row.append(el("span", "rp-size", written ? bitSizeText(n.size_bits) : REPORT.notStored));
    // A toggle that opens a strip of nothing is a dead control.
    if (n.size_bits > 0) row.append(this.bytesButton(item.key));
    return row;
  }

  /** What reads this field, as a link to it. */
  private readsLink(reads: { readonly name: string; readonly path: readonly number[] }): HTMLElement {
    const link = el("button", "rp-reads", REPORT.reads(reads.name));
    link.type = "button";
    link.title = REPORT.readsLabel(reads.name);
    link.setAttribute("aria-label", REPORT.readsLabel(reads.name));
    link.dataset["reads"] = pathString(reads.path);
    return link;
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

  private drawGap(item: Extract<Item, { kind: "gap" }>): HTMLElement {
    const row = el("div", "rp-item rp-row rp-gap");
    row.style.paddingLeft = `${8 + item.depth * 12}px`;
    row.append(el("span", "rp-at", formatOffset(item.offsetBits)));
    row.append(el("span", "rp-twist", ""));
    row.append(el("span", "rp-field", item.unmapped ? GAP_LABEL : REPORT.gap));
    row.append(el("span", "rp-value", GAP_VERDICT[this.verdict(item)]));
    row.append(el("span", "rp-type", ""));
    row.append(el("span", "rp-size", bitSizeText(item.sizeBits)));
    return row;
  }

  private drawMore(item: Extract<Item, { kind: "more" }>): HTMLElement {
    const row = el("div", "rp-item rp-row rp-more");
    row.style.paddingLeft = `${8 + item.depth * 12}px`;
    const reply = this.doc.templateNode(item.path);
    const noun = reply.status === "ok" ? childWord(reply.node) : "item";
    row.append(el("span", "rp-at", ""));
    row.append(el("span", "rp-field", REPORT.more(countText(item.remaining, noun), item.side)));
    row.append(this.listButton(item.path));
    return row;
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
    // the strip is sitting under.
    if (item.kind === "bytes") return;
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
    this.rebuild();
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
    else this.paintAgain();
  }

  /** Redraw the window that is already up, for a change that moves nothing. */
  private paintAgain(): void {
    this.drawn = null;
    this.paint();
  }

  private select(path: readonly number[], offsetBits: number, sizeBits: number): void {
    this.selected = { path, offsetBits, sizeBits };
    this.paintAgain();
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
      this.paintAgain();
      return;
    }
    this.scroller.scrollTop = Math.max(0, (this.tops[i] ?? 0) - this.scroller.clientHeight / 3);
    this.paintAgain();
  }

  /** Follow the cursor: select whatever field covers this bit. */
  setBit(bit: number): void {
    if (this.picking || this.el.hidden) return;
    const at = this.doc.locate(bit);
    if (at.status !== "ok") return;
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
    this.paintAgain();
  }
}

function pathString(path: readonly number[]): string {
  return path.join(".");
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

function itemOpens(node: TemplateNode): boolean {
  return node.composite && node.child_count > 0;
}

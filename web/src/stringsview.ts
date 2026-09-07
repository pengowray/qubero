// The strings in a file that is not a text file.
//
// The text view reads a file that was written to be read. This reads the other
// kind: an executable, a game archive, a firmware image, a save file. Such a
// file is mostly not text, and the text it does hold is the part a reader can
// recognise with no template at all.
//
// The list is built the way `lineindex` builds its index, and for the same
// reason: nothing can say how many strings a file holds without reading all of
// it, and reading all of it is what this editor does not do. So the scan walks
// forward from the front of the file in the browser's idle time and the list
// grows behind it. What has been found is exact; past where the scan has
// reached, nothing is claimed. The status line says where that is.
//
// The rows are one height each, so the row at index `i` is at `i * ROW` and
// what is on screen is division rather than search. That is `listpane`'s
// arithmetic, and it is what makes a list of two hundred thousand strings
// scroll like a list of twenty.

import type { Doc, StringEncoding, StringHit, StringScanOpts } from "./doc.js";
import { formatOffset } from "./format.js";
import { STRINGSVIEW as SV } from "./strings.js";

/** Height of one row, which must match `--sv-row` in the stylesheet: the rows
 *  are placed by arithmetic on it, so a row that drew taller would slide out
 *  from under its own place. */
const ROW = 22;

/** Rows drawn above and below the window, so a wheel notch has somewhere to go
 *  before the next paint. */
const OVERSCAN = 8;

/** How tall the canvas is allowed to get. Browsers stop honouring an element's
 *  height somewhere past a few tens of millions of pixels. */
const MAX_CANVAS = 16_000_000;

/** How many strings one scan call is asked for. Small enough that a stretch of
 *  file that is all text does not arrive as one enormous array, large enough
 *  that a stretch with nothing in it is crossed in few calls. */
const BATCH = 5000;

/** How much of a very long string goes on the row and in its tooltip. A
 *  four-kilobyte base64 blob is not a tooltip. */
const TOOLTIP_TEXT = 1000;

/** How many strings are kept. A file can hold more than a browser should be
 *  asked to hold, and past this the scan stops and says so rather than filling
 *  memory with a list nobody will scroll to the end of. */
export const MAX_HITS = 200_000;

/** The default shortest string reported, in characters. What `strings(1)`
 *  uses, and for the same reason. */
export const MIN_CHARS_DEFAULT = 4;

export const MIN_CHARS_KEY = "qubero.strings.min";
export const ENCODINGS_KEY = "qubero.strings.encodings";

/** The readings on offer, in the order the toggles appear. */
export const ENCODINGS: readonly StringEncoding[] = ["ascii", "utf16le", "utf16be"];

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export class StringsView {
  readonly el: HTMLElement;
  private readonly doc: Doc;
  private readonly scroller: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly status: HTMLElement;
  private readonly progress: HTMLElement;
  private readonly words: HTMLElement;

  /** Every string found so far, in file order. */
  private hits: StringHit[] = [];
  /** Indices into `hits` that the filter lets through, or null for all of
   *  them. Null rather than a copy of every index: the usual case is no
   *  filter, and a list of two hundred thousand numbers standing for "all of
   *  them" is a list nobody needs. */
  private shown: number[] | null = null;
  private filterText = "";

  /** Where the next scan call carries on from. */
  private next = 0;
  /** True once the scan has reached the end of the file or the cap. */
  private done = false;
  /** True while a scan pass is in flight, so two do not run at once. */
  private busy = false;
  /** Bumped whenever what to look for changes, so a pass that was in flight
   *  when the reader changed the settings throws its answer away instead of
   *  appending it to a list it no longer belongs to. */
  private generation = 0;
  private idle = 0;
  /** True once the view has been shown at least once. The scan does not start
   *  until then: a reader who never opens this view never pays for it. */
  private started = false;

  private minChars = MIN_CHARS_DEFAULT;
  private encodings: StringEncoding[] = [...ENCODINGS];

  private drawn: { from: number; to: number } | null = null;
  /** The row the cursor is on, as an index into `hits`. */
  private at = -1;
  /** True while a pick this view made is being sent out, so the cursor move it
   *  causes does not come back and undo the scroll position. */
  private picking = false;

  /** A string was picked: put the cursor on its first byte and select it. The
   *  selection is what makes the panel say what the bytes are, every way they
   *  can be read, which is the check on the reading this view chose. */
  onPick: (at: number, len: number) => void = () => {};
  /** The number in front of a string was picked. It names an address, and an
   *  address on screen is one a reader will want to look at. Its own bytes,
   *  so the panel reads the number out the way it reads out the string. */
  onPickPrefix: (at: number, len: number) => void = () => {};

  constructor(doc: Doc) {
    this.doc = doc;
    this.el = el("div", "stringsview");
    this.el.hidden = true;
    this.el.setAttribute("role", "region");
    this.el.setAttribute("aria-label", SV.regionLabel);
    this.scroller = el("div", "sv-scroll");
    this.scroller.tabIndex = 0;
    this.canvas = el("div", "sv-canvas");
    this.scroller.append(this.canvas);
    this.status = el("div", "sv-status");
    this.status.setAttribute("role", "status");
    // How far the scan has reached, along the top edge of the status row. It
    // goes away when the scan is done: a bar that is always full says nothing.
    this.progress = el("div", "sv-progress");
    this.progress.setAttribute("role", "progressbar");
    this.status.append(this.progress);
    this.words = el("span", "sv-words");
    this.status.append(this.words);
    this.el.append(this.scroller, this.status);
    this.scroller.addEventListener("scroll", () => this.paint(), { passive: true });
    this.scroller.addEventListener("click", (e) => this.onClick(e));
    this.scroller.addEventListener("keydown", (e) => this.onKey(e));
    new ResizeObserver(() => this.paintAgain()).observe(this.scroller);
  }

  // ---- what to look for -------------------------------------------------

  get minimum(): number {
    return this.minChars;
  }

  get reading(): readonly StringEncoding[] {
    return this.encodings;
  }

  /** The shortest string reported. Changing it starts the scan again from the
   *  front: a shorter minimum finds strings that were passed over, and there
   *  is nowhere to put them but where they belong. */
  setMinimum(chars: number): void {
    const want = Math.max(1, Math.min(1024, Math.round(chars)));
    if (want === this.minChars) return;
    this.minChars = want;
    this.restart();
  }

  setReading(encodings: readonly StringEncoding[]): void {
    const want = ENCODINGS.filter((e) => encodings.includes(e));
    if (want.join(",") === this.encodings.join(",")) return;
    this.encodings = want;
    this.restart();
  }

  /** Show only the strings holding this text. The scan is not affected: the
   *  filter is over what has been found, and what has not been found yet
   *  arrives filtered as it comes. */
  setFilter(text: string): void {
    if (text === this.filterText) return;
    this.filterText = text;
    this.refilter();
    this.paintAgain();
    this.tellStatus();
  }

  private opts(): StringScanOpts {
    return { minChars: this.minChars, encodings: this.encodings };
  }

  /** Forget everything found and look again. */
  private restart(): void {
    this.generation += 1;
    // A pass may be waiting on the file right now. It will see the new
    // generation and drop its answer, and it must not be what stops the next
    // one from starting, so the flag it holds is released here rather than
    // where it finishes.
    this.busy = false;
    this.hits = [];
    this.shown = null;
    this.next = 0;
    this.done = false;
    this.at = -1;
    this.scroller.scrollTop = 0;
    this.refilter();
    this.paintAgain();
    this.tellStatus();
    if (this.started) this.startScanning();
  }

  // ---- the scan ---------------------------------------------------------

  /** Run every time this view comes to the front. */
  enter(): void {
    this.started = true;
    this.startScanning();
    this.paintAgain();
    this.tellStatus();
  }

  relayout(): void {
    this.paintAgain();
  }

  /** Keep the scan going in the time the browser has nothing else to do with.
   *
   *  Idle time, but with a deadline, for the reason the text index has one: a
   *  browser hands a tab nobody is looking at no idle time at all. */
  private startScanning(): void {
    if (this.idle !== 0 || this.busy || this.done || this.encodings.length === 0) return;
    const soon = (f: () => void): number =>
      typeof requestIdleCallback === "function" ? requestIdleCallback(() => f(), { timeout: 250 }) : window.setTimeout(f, 16);
    this.idle = soon(() => {
      this.idle = 0;
      void this.pass().then(() => {
        if (!this.done) this.startScanning();
      });
    });
  }

  /** As much of the scan as one turn of idle time is worth.
   *
   *  A pass is mostly waiting for the file rather than scanning it, so one
   *  call a turn leaves the reading of six hundred megabytes paced by the
   *  scheduler rather than by the disk: eight milliseconds of work between
   *  idle callbacks is a few per cent of the time available, and the scan
   *  crawls. Thirty is still short of a frame's worth of jank and gets a large
   *  file read in minutes rather than in half an hour. */
  private async pass(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    const mine = this.generation;
    try {
      const until = performance.now() + 30;
      do {
        const scan = await this.doc.stringsScan(this.next, BATCH, this.opts());
        if (mine !== this.generation) return;
        // No answer and no progress: the chunks it wanted did not come, so
        // there is nothing to do but stop and let the next change wake it.
        if (scan.next === this.next && scan.hits.length === 0) {
          this.done = true;
          break;
        }
        this.next = scan.next;
        this.append(scan.hits);
        if (this.next >= this.doc.lengthBytes || this.hits.length >= MAX_HITS) this.done = true;
      } while (!this.done && performance.now() < until);
    } finally {
      if (mine === this.generation) this.busy = false;
    }
    if (mine !== this.generation) return;
    this.grow();
    this.paint();
    this.tellStatus();
  }

  private append(hits: readonly StringHit[]): void {
    const room = MAX_HITS - this.hits.length;
    const take = hits.length <= room ? hits : hits.slice(0, room);
    const first = this.hits.length;
    for (const h of take) this.hits.push(h);
    if (this.shown !== null) {
      const needle = this.filterText.toLowerCase();
      take.forEach((h, i) => {
        if (h.text.toLowerCase().includes(needle)) this.shown?.push(first + i);
      });
    }
  }

  private refilter(): void {
    if (this.filterText === "") {
      this.shown = null;
      return;
    }
    const needle = this.filterText.toLowerCase();
    const out: number[] = [];
    this.hits.forEach((h, i) => {
      if (h.text.toLowerCase().includes(needle)) out.push(i);
    });
    this.shown = out;
  }

  // ---- drawing ----------------------------------------------------------

  private get rows(): number {
    return this.shown === null ? this.hits.length : this.shown.length;
  }

  /** The hit drawn at row `i`. */
  private hitAt(i: number): StringHit | undefined {
    const index = this.shown === null ? i : this.shown[i];
    return index === undefined ? undefined : this.hits[index];
  }

  /** Index into `hits` of row `i`. */
  private indexAt(i: number): number {
    return this.shown === null ? i : (this.shown[i] ?? -1);
  }

  /** The canvas is as tall as the list is long. Called after each pass, since
   *  that is when the list gets longer. */
  private grow(): void {
    this.canvas.style.height = `${Math.min(MAX_CANVAS, this.rows * ROW)}px`;
  }

  private paintAgain(): void {
    this.drawn = null;
    this.grow();
    this.paint();
  }

  private paint(): void {
    if (this.el.hidden) return;
    const rows = this.rows;
    const first = Math.max(0, Math.floor(this.scroller.scrollTop / ROW) - OVERSCAN);
    const last = Math.min(rows, Math.ceil((this.scroller.scrollTop + this.scroller.clientHeight) / ROW) + OVERSCAN);
    if (this.drawn !== null && this.drawn.from === first && this.drawn.to === last) return;
    this.drawn = { from: first, to: last };
    const out: HTMLElement[] = [];
    for (let i = first; i < last; i++) {
      const hit = this.hitAt(i);
      if (hit === undefined) continue;
      const row = this.drawRow(hit, this.indexAt(i), this.indexAt(i) === this.at);
      row.style.top = `${i * ROW}px`;
      row.dataset["row"] = String(i);
      out.push(row);
    }
    this.canvas.replaceChildren(...out);
  }

  /** One row: where it is, how it is read, what it says, what is unusual about
   *  it, and how long it is. Everything but the text is muted, so a page of
   *  ordinary strings reads as one column and the rows worth stopping at are
   *  the ones with something in the margin. */
  private drawRow(hit: StringHit, index: number, on: boolean): HTMLElement {
    const row = el("div", on ? "sv-row is-on" : "sv-row");
    const at = el("span", "sv-at", formatOffset(hit.at * 8));
    at.title = SV.offsetTitle;
    row.append(at);
    const enc = el("span", hit.enc === "ASCII" ? "sv-enc is-plain" : "sv-enc", hit.enc);
    enc.title = SV.encodingTitle[hit.enc] ?? hit.enc;
    row.append(enc);

    const text = el("span", "sv-text", hit.text.slice(0, TOOLTIP_TEXT));
    text.title = hit.text.slice(0, TOOLTIP_TEXT);
    row.append(text);
    if (hit.terminator > 0) {
      const nul = el("span", "sv-nul", SV.terminatorMark);
      nul.title = SV.terminatorTitle(hit.terminator);
      row.append(nul);
    }

    const notes = el("span", "sv-notes");
    const first = hit.prefix[0];
    if (first !== undefined) {
      // Only the readings that come to the same number are folded into one
      // note: they are the same length written at several widths. A reading
      // that says something else gets said in full.
      const same = hit.prefix.filter(
        (p) => p.value === first.value && p.counts === first.counts && p.with_terminator === first.with_terminator,
      );
      const rest = hit.prefix.filter((p) => !same.includes(p));
      const say = (p: typeof first, also: readonly string[]): HTMLElement => {
        const n = el("button", "sv-note sv-prefix");
        n.type = "button";
        // The hex is its own span so that a pane too narrow for the note
        // drops it whole. Truncating from the right would eat the count
        // first and leave "length prefix", the least specific words here.
        const said = SV.prefix(p.kind, p.bytes, p.value, p.counts, p.with_terminator, p.weak);
        n.append(
          said.before,
          el("span", "sv-hex", said.hex),
          said.after + (also.length > 0 ? SV.prefixAlso(also) : ""),
        );
        n.title = SV.prefixTitle(also.length > 0 ? same : [p], hit.len, hit.enc.startsWith("UTF-16"));
        // The common grade is the quieter one, the way the ASCII tag is
        // quieter than the rest of the encoding column.
        if (p.weak) n.classList.add("is-weak");
        n.dataset["prefix"] = String(p.at);
        n.dataset["prefixLen"] = String(p.bytes.length);
        return n;
      };
      notes.append(say(first, same.slice(1).map((p) => p.kind)));
      for (const p of rest) notes.append(say(p, []));
    }
    if (hit.lone_surrogates) {
      const n = el("span", "sv-note is-odd", SV.loneSurrogate);
      n.title = SV.loneSurrogateTitle;
      notes.append(n);
    }
    if (hit.cut) {
      const n = el("span", "sv-note is-odd", SV.continuesAt(hit.at + hit.len));
      n.title = SV.continuesAtTitle(hit.at + hit.len, hit.len);
      notes.append(n);
    }
    const before = this.hits[index - 1];
    if (before !== undefined && before.cut && before.at + before.len === hit.at) {
      const n = el("span", "sv-note is-odd", SV.continuedFrom(before.at));
      n.title = SV.continuedFromTitle(before.at, before.len);
      notes.append(n);
    }
    if (notes.childElementCount > 0) row.append(notes);

    const len = el("span", "sv-len", SV.length(hit.len));
    len.title = SV.lengthTitle(hit.len, hit.units, hit.chars);
    row.append(len);
    return row;
  }

  private tellStatus(): void {
    const total = this.doc.lengthBytes;
    const scanned = Math.min(this.next, total);
    const capped = this.hits.length >= MAX_HITS;
    if (this.encodings.length === 0) {
      this.words.textContent = SV.statusNoEncodings;
      this.progress.hidden = true;
      return;
    }
    if (this.done && !capped && this.hits.length === 0) {
      this.words.textContent = SV.statusNone(this.minChars);
      this.progress.hidden = true;
      return;
    }
    const count =
      this.filterText === "" ? SV.statusFound(this.hits.length) : SV.statusFiltered(this.rows, this.hits.length);
    const where = capped
      ? SV.statusCapped(scanned, total)
      : this.done
        ? SV.statusWhole
        : SV.statusScanning(scanned, total);
    this.words.textContent = `${count} \u00b7 ${where}`;
    // The bar is for a scan still moving. Once it has stopped, capped or not,
    // the words say where it stopped and a frozen bar only asks whether it is.
    this.progress.hidden = this.done;
    if (!this.done) {
      this.progress.style.width = `${total === 0 ? 100 : (scanned / total) * 100}%`;
      this.progress.setAttribute("aria-label", SV.scanProgressLabel(scanned, total));
    }
  }

  // ---- following the cursor --------------------------------------------

  /** Put the cursor's byte on screen, if a string covers it. A byte with no
   *  string over it moves nothing: most of a binary is not a string, and
   *  scrolling the list every time the hex cursor crossed a byte would make
   *  the list unreadable. */
  setByte(at: number): void {
    if (this.picking || this.el.hidden) return;
    const index = this.find(at);
    if (index < 0) return;
    this.at = index;
    const row = this.rowOf(index);
    if (row >= 0) this.scrollTo(row);
    this.paintAgain();
  }

  /** The string covering a byte, by binary search over the hits, which are in
   *  file order. */
  private find(at: number): number {
    let lo = 0;
    let hi = this.hits.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const h = this.hits[mid];
      if (h === undefined) break;
      if (at < h.at) hi = mid - 1;
      else if (at >= h.at + h.len) lo = mid + 1;
      else return mid;
    }
    return -1;
  }

  /** Which row a hit is drawn on, or -1 when the filter is hiding it. */
  private rowOf(index: number): number {
    if (this.shown === null) return index;
    const row = this.shown.indexOf(index);
    return row;
  }

  private scrollTo(row: number): void {
    const top = row * ROW;
    if (top < this.scroller.scrollTop) this.scroller.scrollTop = top;
    else if (top + ROW > this.scroller.scrollTop + this.scroller.clientHeight) {
      this.scroller.scrollTop = top + ROW - this.scroller.clientHeight;
    }
  }

  // ---- input ------------------------------------------------------------

  private onClick(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof Element)) return;
    // The number in front of a string names an address of its own, so
    // clicking it goes there rather than to the text it counts.
    const button = target.closest<HTMLElement>(".sv-prefix");
    const prefix = button?.dataset["prefix"];
    if (prefix !== undefined) {
      this.onPickPrefix(Number(prefix), Number(button?.dataset["prefixLen"] ?? 0));
      return;
    }
    const row = target.closest<HTMLElement>(".sv-row")?.dataset["row"];
    if (row === undefined) return;
    this.pick(Number(row));
  }

  private onKey(e: KeyboardEvent): void {
    const page = Math.max(1, Math.floor(this.scroller.clientHeight / ROW) - 1);
    const by: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page };
    const move = by[e.key];
    if (move !== undefined) {
      e.preventDefault();
      this.scroller.scrollTop += move * ROW;
      return;
    }
    if (e.key === "Home") {
      e.preventDefault();
      this.scroller.scrollTop = 0;
    } else if (e.key === "End") {
      e.preventDefault();
      this.scroller.scrollTop = this.canvas.clientHeight;
    } else if (e.key === "Enter" || e.key === " ") {
      const row = Math.floor((this.scroller.scrollTop + this.scroller.clientHeight / 2) / ROW);
      e.preventDefault();
      this.pick(row);
    }
  }

  private pick(row: number): void {
    const hit = this.hitAt(row);
    if (hit === undefined) return;
    this.at = this.indexAt(row);
    this.paintAgain();
    this.picking = true;
    this.onPick(hit.at, hit.len);
    this.picking = false;
  }
}

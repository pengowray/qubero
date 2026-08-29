// One long list, on its own, scrolled by index.
//
// The listing draws a window of a long list and says how much is above and
// below it. That is right for a list you are passing through on the way down
// the file, and wrong for one you came to read: a quarter of a million tokens
// is a document in its own right, and paging through it two hundred at a time
// is not reading it.
//
// So this is the other half of the answer, and it is the simplest kind of
// virtual list there is. Every element is one row of the same height, so where
// row `i` sits is `i * ROW` and what is on screen is division rather than
// search: no flattening, no measuring, no prefix sums. Only the rows in the
// window are asked for, which is what makes a list nobody could draw scroll
// like one anybody can.

import { formatOffset } from "./doc.js";
import type { Doc, FieldPick, TemplateNode } from "./doc.js";
import { fieldClass } from "./fieldstyle.js";
import { bitSizeText, childWord, countText, REPORT } from "./strings.js";

/** Height of one row, which must match `--lp-row` in the stylesheet: the rows
 *  are placed by arithmetic on it, so a row that draws taller would slide out
 *  from under its own place. */
const ROW = 22;
/** Rows drawn above and below the window, so a wheel notch has somewhere to go
 *  before the next paint. */
const OVERSCAN = 8;
/** How tall the canvas is allowed to get. Browsers stop honouring an element's
 *  height somewhere past a few tens of millions of pixels, and a list long
 *  enough to hit that would scroll wrong in a way nothing on screen explains.
 *  A quarter of a million rows is five and a half million pixels, so this is
 *  headroom rather than a limit anybody meets. */
const MAX_CANVAS = 30_000_000;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export class ListPane {
  readonly el: HTMLElement;
  private readonly title: HTMLElement;
  private readonly scroller: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly doc: Doc;
  /** The list being shown, or null while the pane is closed. */
  private path: readonly number[] | null = null;
  private count = 0;
  /** The format's word for one element: values, tensors, entries. */
  private unit = "item";
  private name = "";
  /** Elements already read, by index. Cleared whenever the file changes, since
   *  that is when what one of them says can stop being true. */
  private have = new Map<number, TemplateNode>();
  /** True while some row on screen is waiting on bytes. */
  private waiting = false;
  private drawn: { from: number; to: number } | null = null;
  private selected: { readonly offsetBits: number; readonly sizeBits: number } | null = null;
  /** True while a selection this pane made is being sent out, so the cursor
   *  move it causes does not come back and undo the scroll position. */
  private picking = false;
  private frame = 0;

  onPick: (pick: FieldPick) => void = () => {};
  onClose: () => void = () => {};

  constructor(doc: Doc) {
    this.doc = doc;
    this.el = el("div", "listpane");
    this.el.hidden = true;
    this.title = el("b", "lp-title");
    const close = el("button", "lp-close", REPORT.paneClose);
    close.type = "button";
    close.title = REPORT.paneCloseLabel;
    close.setAttribute("aria-label", REPORT.paneCloseLabel);
    close.addEventListener("click", () => this.close());
    this.scroller = el("div", "lp-scroll");
    this.scroller.tabIndex = 0;
    this.canvas = el("div", "lp-canvas");
    this.scroller.append(this.canvas);
    const bar = el("header", "lp-bar");
    bar.append(this.title, close);
    this.el.append(bar, this.scroller);
    this.scroller.addEventListener("scroll", () => this.paint(), { passive: true });
    this.scroller.addEventListener("click", (e) => this.onClick(e));
    this.scroller.addEventListener("keydown", (e) => this.onKey(e));
    new ResizeObserver(() => this.paintAgain()).observe(this.scroller);
    // Chunks arriving turn a waiting row into an element; so does an edit.
    doc.onChange(() => this.schedule());
  }

  /** True while a list is open in the pane. */
  get isOpen(): boolean {
    return this.path !== null;
  }

  /** Show one list. A second call replaces what is there: the pane is a place
   *  to read one list in, not a stack of them. */
  open(path: readonly number[]): void {
    const reply = this.doc.templateNode(path);
    if (reply.status !== "ok") return;
    const node = reply.node;
    this.path = path;
    this.count = node.child_count;
    this.unit = childWord(node);
    this.name = node.name;
    this.have.clear();
    this.el.hidden = false;
    this.scroller.scrollTop = 0;
    this.title.textContent = REPORT.paneTitle(node.name, node.child_count, this.unit);
    this.canvas.style.height = `${Math.min(MAX_CANVAS, this.count * ROW)}px`;
    this.paintAgain();
  }

  close(): void {
    if (this.path === null) return;
    this.path = null;
    this.have.clear();
    this.el.hidden = true;
    this.canvas.replaceChildren();
    this.drawn = null;
    this.onClose();
  }

  /** The list this pane is showing, so the caller can tell whether a path is
   *  in it without knowing how the pane keeps it. */
  holds(path: readonly number[]): boolean {
    const mine = this.path;
    if (mine === null || path.length <= mine.length) return false;
    return mine.every((step, i) => path[i] === step);
  }

  relayout(): void {
    this.paintAgain();
  }

  /** Draw again once the frame is over, however many things asked. Streaming a
   *  file's first megabyte fires `onChange` far faster than a screen redraws. */
  private schedule(): void {
    if (this.frame !== 0 || this.path === null || this.el.hidden) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      // What a row said may have been a guess at bytes that have since
      // arrived, so the answers go rather than being drawn again.
      this.have.clear();
      this.paintAgain();
    });
  }

  private paintAgain(): void {
    this.drawn = null;
    this.paint();
  }

  // ----- drawing -----

  private paint(): void {
    if (this.path === null || this.el.hidden) return;
    const first = Math.max(0, Math.floor(this.scroller.scrollTop / ROW) - OVERSCAN);
    const last = Math.min(this.count, Math.ceil((this.scroller.scrollTop + this.scroller.clientHeight) / ROW) + OVERSCAN);
    if (this.drawn !== null && this.drawn.from === first && this.drawn.to === last) return;
    this.drawn = { from: first, to: last };
    this.fetch(first, last);
    const out: HTMLElement[] = [];
    for (let i = first; i < last; i++) {
      const node = this.have.get(i);
      const row = node === undefined ? this.drawWaiting(i) : this.drawRow(i, node);
      row.style.top = `${i * ROW}px`;
      row.dataset["index"] = String(i);
      out.push(row);
    }
    this.canvas.replaceChildren(...out);
  }

  /** Read the elements on screen that are not read yet. One call for the whole
   *  window: the elements of a list placed by offsets sit apart from one
   *  another, so asking for them one at a time is one wait each. */
  private fetch(from: number, to: number): void {
    if (this.path === null) return;
    let missing = false;
    for (let i = from; i < to; i++) if (!this.have.has(i)) missing = true;
    if (!missing) return;
    const reply = this.doc.templateChildren(this.path, from, to);
    if (reply.status !== "ok") {
      // Not an answer, so nothing is kept: the redraw after the chunks land
      // asks again. `Doc` has already gone for them.
      this.waiting = reply.status === "pending" || reply.status === "working";
      return;
    }
    this.waiting = false;
    reply.node.forEach((node, i) => this.have.set(from + i, node));
  }

  private drawRow(index: number, node: TemplateNode): HTMLElement {
    const row = el("div", "lp-row");
    if (this.isSelected(node.offset_bits, node.size_bits)) row.classList.add("is-on");
    row.append(el("span", "lp-at", formatOffset(node.offset_bits)));
    row.append(el("span", `lp-name ${fieldClass(node.kind)}`, node.name));
    row.append(el("span", "lp-value", node.composite ? countText(node.child_count, childWord(node)) : node.value));
    row.append(el("span", "lp-size", bitSizeText(node.size_bits)));
    return row;
  }

  /** A row whose bytes are not here yet. It keeps its place and its index, so
   *  the list does not jump when the answer arrives in it. */
  private drawWaiting(index: number): HTMLElement {
    const row = el("div", "lp-row lp-waiting");
    row.append(el("span", "lp-at", ""));
    row.append(el("span", "lp-name", `[${index.toLocaleString()}]`));
    row.append(el("span", "lp-value", REPORT.paneWaiting));
    row.append(el("span", "lp-size", ""));
    return row;
  }

  // ----- selection -----

  private isSelected(offsetBits: number, sizeBits: number): boolean {
    const s = this.selected;
    return s !== null && s.offsetBits === offsetBits && s.sizeBits === sizeBits;
  }

  /** Follow the cursor. A list is its own index, so finding the row for a bit
   *  is a step of the path rather than a search. */
  setBit(bit: number): void {
    if (this.picking || this.path === null || this.el.hidden) return;
    const at = this.doc.locate(bit);
    if (at.status !== "ok" || !this.holds(at.node)) return;
    const index = at.node[this.path.length];
    if (index === undefined) return;
    const node = this.doc.templateNode(at.node.slice(0, this.path.length + 1));
    if (node.status !== "ok") return;
    this.selected = { offsetBits: node.node.offset_bits, sizeBits: node.node.size_bits };
    this.scrollTo(index);
    this.paintAgain();
  }

  clearSelection(): void {
    this.selected = null;
    this.paintAgain();
  }

  private scrollTo(index: number): void {
    const top = index * ROW;
    if (top < this.scroller.scrollTop) this.scroller.scrollTop = top;
    else if (top + ROW > this.scroller.scrollTop + this.scroller.clientHeight) {
      this.scroller.scrollTop = top + ROW - this.scroller.clientHeight;
    }
  }

  // ----- input -----

  private onClick(e: MouseEvent): void {
    const target = e.target;
    if (!(target instanceof Element)) return;
    const at = target.closest<HTMLElement>(".lp-row")?.dataset["index"];
    if (at === undefined) return;
    this.pick(Number(at));
  }

  private onKey(e: KeyboardEvent): void {
    if (this.path === null) return;
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
    } else if (e.key === "Escape") {
      e.preventDefault();
      this.close();
    }
  }

  private pick(index: number): void {
    const node = this.have.get(index);
    if (node === undefined || this.path === null) return;
    this.selected = { offsetBits: node.offset_bits, sizeBits: node.size_bits };
    this.paintAgain();
    this.picking = true;
    this.onPick({ path: [...this.path, index], startBit: node.offset_bits, endBit: node.offset_bits + node.size_bits });
    this.picking = false;
  }
}

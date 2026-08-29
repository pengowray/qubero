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
import { emptyState, flatten, PAGE } from "./flatten.js";
import type { Item, ListingState, TreeSource } from "./flatten.js";
import { fieldClass, sectionColor } from "./fieldstyle.js";
import { bitSizeText, childWord, countText, REPORT } from "./strings.js";

/** Row heights, which must match `--rp-*` in the stylesheet: the tops of every
 *  item are a running total of these, and a row that draws taller than it was
 *  measured at would slide out from under its own place. */
const HEIGHT = { section: 40, part: 26, row: 22 } as const;
/** Items drawn above and below the window, so a wheel notch has somewhere to
 *  go before the next paint. */
const OVERSCAN = 6;

function heightOf(item: Item): number {
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

export class ListingReport {
  readonly el: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly doc: Doc;
  private readonly src: TreeSource;
  private state: ListingState = emptyState;
  private items: readonly Item[] = [];
  /** Where each item starts, in pixels, and one past the last: `tops[i + 1]`
   *  is always there. */
  private tops: number[] = [0];
  private drawn: { from: number; to: number } | null = null;
  private frame = 0;
  private selected: string | null = null;

  onPick: (path: readonly number[]) => void = () => {};

  constructor(doc: Doc) {
    this.doc = doc;
    this.src = {
      node: (path) => doc.templateNode(path),
      children: (path, from, to) => doc.templateChildren(path, from, to),
    };
    this.el = el("div", "report");
    this.el.tabIndex = 0;
    this.el.setAttribute("role", "tree");
    this.canvas = el("div", "rp-canvas");
    this.el.append(this.canvas);
    this.el.addEventListener("scroll", () => this.paint(), { passive: true });
    this.el.addEventListener("click", (e) => this.onClick(e));
    new ResizeObserver(() => this.paint()).observe(this.el);
    // Chunks arriving turn a pending stretch into rows; so does an edit.
    doc.onChange(() => this.schedule());
  }

  /** Draw again once the frame is over, however many things asked. Streaming a
   *  file's first megabyte fires `onChange` far faster than a screen redraws.
   *
   *  Only for changes the file makes to itself. Something the reader did takes
   *  effect there and then: waiting a frame to open a fold buys nothing, and
   *  a hidden tab runs no frames at all. */
  private schedule(): void {
    if (this.frame !== 0) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.rebuild();
    });
  }

  relayout(): void {
    if (this.items.length === 0) this.rebuild();
    else this.paint();
  }

  /** Walk the tree again and lay the items out, keeping the reader where they
   *  were. The item under the top of the window is the anchor: opening a fold
   *  above it would otherwise push the whole file down under the cursor. */
  private rebuild(): void {
    const anchor = this.anchor();
    this.items = flatten(this.src, this.state).items;
    this.tops = new Array(this.items.length + 1);
    this.tops[0] = 0;
    for (const [i, item] of this.items.entries()) this.tops[i + 1] = (this.tops[i] ?? 0) + heightOf(item);
    this.canvas.style.height = `${this.tops[this.items.length] ?? 0}px`;
    this.drawn = null;
    if (anchor !== null) this.restore(anchor);
    this.paint();
    // A stretch still being read comes back as a pending item. `Doc` has
    // already asked for the chunks behind it and will say when they land, so
    // there is nothing to do here but draw what there is.
  }

  private anchor(): { readonly key: string; readonly delta: number } | null {
    const i = this.indexAt(this.el.scrollTop);
    const item = this.items[i];
    if (item === undefined) return null;
    return { key: item.key, delta: (this.tops[i] ?? 0) - this.el.scrollTop };
  }

  private restore(anchor: { readonly key: string; readonly delta: number }): void {
    const i = this.items.findIndex((item) => item.key === anchor.key);
    if (i < 0) return;
    this.el.scrollTop = (this.tops[i] ?? 0) - anchor.delta;
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
    if (this.items.length === 0) return;
    const from = Math.max(0, this.indexAt(this.el.scrollTop) - OVERSCAN);
    const to = Math.min(this.items.length, this.indexAt(this.el.scrollTop + this.el.clientHeight) + 1 + OVERSCAN);
    if (this.drawn !== null && this.drawn.from === from && this.drawn.to === to) return;
    this.drawn = { from, to };
    const fileBits = this.doc.lengthBits;
    const out: HTMLElement[] = [];
    for (let i = from; i < to; i++) {
      const item = this.items[i];
      if (item === undefined) continue;
      const node = this.draw(item, fileBits);
      node.style.top = `${this.tops[i] ?? 0}px`;
      node.dataset["key"] = item.key;
      out.push(node);
    }
    this.canvas.replaceChildren(...out);
  }

  private draw(item: Item, fileBits: number): HTMLElement {
    switch (item.kind) {
      case "heading":
        return this.drawHeading(item, fileBits);
      case "row":
        return this.drawRow(item);
      case "gap":
        return this.drawGap(item);
      case "fold":
        return this.drawFold(item);
      case "record":
        return el("div", "rp-item rp-record", item.node.name);
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
    row.append(el("b", "rp-name", item.node?.name ?? REPORT.unnamedPart));
    row.append(el("span", "rp-range", rangeText(item.offsetBits, item.sizeBits)));
    const share = shareText(item.sizeBits, fileBits);
    row.append(el("span", "rp-size", `${formatBytes(item.sizeBits / 8)}${share === "" ? "" : ` · ${share}`}`));
    return row;
  }

  private drawRow(item: Extract<Item, { kind: "row" }>): HTMLElement {
    const n = item.node;
    const row = el("div", "rp-item rp-row");
    if (this.selected === item.key) row.classList.add("is-on");
    row.style.paddingLeft = `${8 + item.depth * 12}px`;
    row.append(el("span", "rp-at", formatOffset(n.offset_bits)));
    row.append(el("span", `rp-field ${fieldClass(n.kind)}`, n.name));
    row.append(el("span", "rp-value", n.composite ? countText(n.child_count, childWord(n)) : n.value));
    row.append(el("span", "rp-type", n.type));
    row.append(el("span", "rp-size", bitSizeText(n.size_bits)));
    return row;
  }

  private drawGap(item: Extract<Item, { kind: "gap" }>): HTMLElement {
    const row = el("div", "rp-item rp-row rp-gap");
    row.style.paddingLeft = `${8 + item.depth * 12}px`;
    row.append(el("span", "rp-at", formatOffset(item.offsetBits)));
    row.append(el("span", "rp-field", REPORT.gap));
    row.append(el("span", "rp-value", ""));
    row.append(el("span", "rp-type", ""));
    row.append(el("span", "rp-size", bitSizeText(item.sizeBits)));
    return row;
  }

  private drawFold(item: Extract<Item, { kind: "fold" }>): HTMLElement {
    const row = el("div", "rp-item rp-row rp-fold");
    row.style.paddingLeft = `${8 + item.depth * 12}px`;
    row.append(el("span", "rp-at", formatOffset(item.offsetBits)));
    row.append(el("span", "rp-twist", item.open ? "▾" : "▸"));
    row.append(el("span", "rp-value", REPORT.fold(item.nodes.length, item.owner?.name ?? null)));
    row.append(el("span", "rp-size", bitSizeText(item.sizeBits)));
    return row;
  }

  private drawMore(item: Extract<Item, { kind: "more" }>): HTMLElement {
    const row = el("div", "rp-item rp-row rp-more");
    row.style.paddingLeft = `${8 + item.depth * 12}px`;
    const reply = this.doc.templateNode(item.path);
    const noun = reply.status === "ok" ? childWord(reply.node) : "item";
    row.append(el("span", "rp-at", ""));
    row.append(el("span", "rp-field", REPORT.more(countText(item.remaining, noun))));
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
    if (item.kind === "more") {
      const at = pathString(item.path);
      const shown = new Map(this.state.shown);
      shown.set(at, item.shown + PAGE);
      this.state = { open: this.state.open, shown };
      this.rebuild();
      return;
    }
    const openKey = openKeyOf(item);
    if (openKey !== null) {
      const open = new Set(this.state.open);
      if (open.has(openKey)) open.delete(openKey);
      else open.add(openKey);
      this.state = { open, shown: this.state.shown };
      this.rebuild();
    }
    if (item.kind === "row") {
      this.selected = item.key;
      this.onPick(item.path);
      this.paintAgain();
    }
  }

  /** Redraw the window that is already up, for a change that moves nothing. */
  private paintAgain(): void {
    this.drawn = null;
    this.paint();
  }

  /** Bring the field at `path` on screen and select it. */
  reveal(path: readonly number[]): void {
    const key = `r:${pathString(path)}`;
    let i = this.items.findIndex((item) => item.key === key);
    if (i < 0) {
      // The field is inside something still closed: open every step down to it.
      const open = new Set(this.state.open);
      for (let n = 1; n <= path.length; n++) open.add(pathString(path.slice(0, n)));
      this.state = { open, shown: this.state.shown };
      this.rebuild();
      i = this.items.findIndex((item) => item.key === key);
      if (i < 0) return;
    }
    this.selected = key;
    this.el.scrollTop = Math.max(0, (this.tops[i] ?? 0) - this.el.clientHeight / 3);
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
  if (item.kind === "fold") return item.key;
  if (item.kind === "heading") return item.node === null || item.level === 0 ? null : pathString(item.path);
  if (item.kind === "row") return itemOpens(item.node) ? pathString(item.path) : null;
  return null;
}

function itemOpens(node: TemplateNode): boolean {
  return node.composite && node.child_count > 0;
}

// One item of the listing as one element.
//
// Split out of `listingreport.ts`, which had grown to hold the layout, the
// input, the selection and the drawing all at once. This half is the drawing
// only: an item goes in, an element comes out, and nothing here knows where in
// the document it lands, how tall it turned out to be, or what the reader does
// to it next. What it does need from the report — the file, the parts of it a
// map is drawn from, what is selected, what is open — arrives as a
// `DrawContext`, read fresh for each paint.

import { formatBytes, formatOffset } from "./doc.js";
import type { Doc, TemplateNode } from "./doc.js";
import { pathKey, PAGE } from "./flatten.js";
import type { Item } from "./flatten.js";
import { fieldClass, sectionColor } from "./fieldstyle.js";
import { byteStrip } from "./bytestrip.js";
import { fileMap } from "./filemap.js";
import { recordTable } from "./records.js";
import type { GapVerdict } from "./gapcheck.js";
import type { MapSegment } from "./filemap.js";
import { bitSizeText, childWord, countText, GAP_LABEL, REPORT } from "./strings.js";

/** What is selected, as the bits it covers rather than as the row showing it. */
export type Selected = { readonly path: readonly number[]; readonly offsetBits: number; readonly sizeBits: number };

/** Everything the drawing needs from the report, and nothing else. */
export type DrawContext = {
  readonly doc: Doc;
  /** The file's top-level parts, which every strip of the map is drawn from. */
  readonly segments: readonly MapSegment[];
  readonly selected: Selected | null;
  /** The key of the row standing in for a selection with no row of its own. */
  readonly nearest: string | null | undefined;
  /** The keys of the items showing their bytes. */
  readonly bytes: ReadonlySet<string>;
  /** Fields opened out into a dump of all their bytes, and where each dump is
   *  scrolled to. Neither can live in the strip, which is built again from
   *  nothing every time anything on screen changes. */
  readonly dumps: Set<number>;
  readonly dumpTops: Map<number, number>;
  readonly toggleBytes: (key: string) => void;
  readonly toggleDump: (offsetBits: number) => void;
  /** What a run of unclaimed bytes turned out to hold. The answer is cached by
   *  the report, since finding it reads the file. */
  readonly verdict: (item: Extract<Item, { kind: "gap" }>) => GapVerdict;
};

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** `0x1000 – 0x1fff`, the stretch a heading covers. A part of no bytes has no
 *  range to give, which is what a field placed somewhere else looks like. */
export function rangeText(offsetBits: number, sizeBits: number): string {
  if (sizeBits === 0) return formatOffset(offsetBits);
  return `${formatOffset(offsetBits)} – ${formatOffset(offsetBits + sizeBits - 8)}`;
}

/** How much of the file this is, for a part big enough for the answer to mean
 *  anything. Under a per cent, the number says less than the range does. */
export function shareText(sizeBits: number, fileBits: number): string {
  if (fileBits <= 0) return "";
  const share = sizeBits / fileBits;
  return share < 0.01 ? REPORT.tinyShare : `${Math.round(share * 100)}%`;
}

/** Where a run of plain fields sits in the file, which is all there is to
 *  name it by. A run at the front is a header; the same fields at the back
 *  are not. */
export function runPosition(item: Item, fileBits: number): "start" | "end" | "middle" {
  if (item.offsetBits === 0) return "start";
  if (fileBits > 0 && item.offsetBits + item.sizeBits >= fileBits) return "end";
  return "middle";
}

/** Whether a stretch of bytes is the selected one. Equality, not overlap: a
 *  field sits inside its structure, and lighting everything the selection is
 *  inside would light most of the screen. */
export function isSelected(sel: Selected | null, offsetBits: number, sizeBits: number): boolean {
  return sel !== null && sel.offsetBits === offsetBits && sel.sizeBits === sizeBits;
}

/** Whether a stretch holds the selection. For the one row that is allowed to
 *  say so: the nearest thing on screen that contains it, when the field itself
 *  is not a row of its own. */
export function holdsSelection(sel: Selected | null, offsetBits: number, sizeBits: number): boolean {
  return sel !== null && sel.offsetBits >= offsetBits && sel.offsetBits + sel.sizeBits <= offsetBits + sizeBits;
}

/** What each answer from `checkGap` is called. */
const GAP_VERDICT = {
  zeros: REPORT.gapZeros,
  something: REPORT.gapNonzero,
  "too-large": REPORT.gapTooLarge,
  unread: REPORT.gapUnread,
} as const;

export function drawItem(c: DrawContext, item: Item, fileBits: number): HTMLElement {
  switch (item.kind) {
    case "heading":
      return drawHeading(c, item, fileBits);
    case "row":
      return drawRow(c, item);
    case "gap":
      return drawGap(c, item);
    case "bytes":
      return drawStrip(c, item);
    case "record":
      return drawRecord(c, item);
    case "more":
      return drawMore(c, item);
    case "pending":
      return el("div", "rp-item rp-pending", REPORT.reading);
  }
}

function drawHeading(c: DrawContext, item: Extract<Item, { kind: "heading" }>, fileBits: number): HTMLElement {
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
  row.append(bytesButton(c, item.key));
  // Only a list too long to draw: for anything the window already holds
  // whole, a pane of its own would be the same rows somewhere else.
  if (item.node !== null && item.node.child_count > PAGE) row.append(listButton(item.path));
  row.append(mapFor(c, item));
  return row;
}

function drawRow(c: DrawContext, item: Extract<Item, { kind: "row" }>): HTMLElement {
  const n = item.node;
  // A field of no bytes is grey: whether it is a value the template worked
  // out or a list that turned out to be empty, there is nothing of it in the
  // file, and a row the reader can skip should look like one.
  const row = el("div", `rp-item rp-row${n.size_bits === 0 ? " rp-nobytes" : ""}`);
  if (isSelected(c.selected, item.offsetBits, item.sizeBits) || c.nearest === item.key) row.classList.add("is-on");
  row.style.paddingLeft = `${8 + item.depth * 12}px`;
  // A computed value is not written anywhere, so it has no address, and its
  // length says so in words: "0x101a7" and "0 bytes" would be answers to
  // questions this row is not the answer to.
  const written = n.type !== "computed";
  row.append(el("span", "rp-at", written ? formatOffset(n.offset_bits) : ""));
  // A row that opens says so. Without it the only way to find out which
  // rows have anything under them is to click every one of them.
  row.append(el("span", "rp-twist", itemOpens(n) ? (item.open ? "▾" : "▸") : ""));
  row.append(el("span", `rp-field ${fieldClass(n.kind)}`, n.name));
  const value = el("span", "rp-value", n.composite ? countText(n.child_count, childWord(n)) : n.value);
  if (item.reads !== null) value.append(readsLink(item.reads));
  row.append(value);
  row.append(el("span", "rp-type", n.type));
  row.append(el("span", "rp-size", written ? bitSizeText(n.size_bits) : REPORT.notStored));
  // A toggle that opens a strip of nothing is a dead control.
  if (n.size_bits > 0) row.append(bytesButton(c, item.key));
  return row;
}

function drawGap(c: DrawContext, item: Extract<Item, { kind: "gap" }>): HTMLElement {
  const row = el("div", "rp-item rp-row rp-gap");
  row.style.paddingLeft = `${8 + item.depth * 12}px`;
  row.append(el("span", "rp-at", formatOffset(item.offsetBits)));
  row.append(el("span", "rp-twist", ""));
  row.append(el("span", "rp-field", item.unmapped ? GAP_LABEL : REPORT.gap));
  row.append(el("span", "rp-value", GAP_VERDICT[c.verdict(item)]));
  row.append(el("span", "rp-type", ""));
  row.append(el("span", "rp-size", bitSizeText(item.sizeBits)));
  return row;
}

/** A structure the format keeps as a table, drawn as one: the format's own
 *  column names, and where each row is written. */
function drawRecord(c: DrawContext, item: Extract<Item, { kind: "record" }>): HTMLElement {
  const host = el("div", "rp-item rp-record");
  host.style.paddingLeft = `${8 + item.depth * 12}px`;
  const table = recordTable(c.doc, item.node);
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
    tr.dataset["at"] = String(row.offsetBits);
    tr.dataset["size"] = String(row.sizeBits);
    if (holdsSelection(c.selected, row.offsetBits, row.sizeBits)) tr.className = "is-on";
    for (const cell of row.cells) tr.append(el("td", fieldClass(cell.kind), cell.text));
    const at = el("td", "rec-at");
    // The one way out of the table: the row's own bytes, which is where it
    // was read from and where the reader goes to see how.
    const link = el("button", "rec-link", `${formatOffset(row.offsetBits)} · ${formatBytes(row.sizeBits / 8)}`);
    link.type = "button";
    // Back to the fields: the row's own bytes, under the table it came from.
    const rowKey = `r:${pathKey(row.path)}`;
    if (c.bytes.has(rowKey)) link.classList.add("is-on");
    link.addEventListener("click", (e) => {
      e.stopPropagation();
      c.toggleBytes(rowKey);
    });
    at.append(link);
    tr.append(at);
    grid.append(tr);
  }
  host.append(grid);
  if (table.pending) host.append(el("div", "bs-wait", REPORT.reading));
  return host;
}

function drawStrip(c: DrawContext, item: Extract<Item, { kind: "bytes" }>): HTMLElement {
  const host = el("div", "rp-item rp-strip");
  host.style.paddingLeft = `${8 + item.depth * 12}px`;
  const caption = `${item.name} ${rangeText(item.offsetBits, item.sizeBits)}`;
  host.append(
    byteStrip(c.doc, item.offsetBits, item.sizeBits, caption, mapFor(c, item), () => c.toggleBytes(item.owner), c.selected, {
      open: c.dumps,
      toggle: (at) => c.toggleDump(at),
      scroll: (at) => ({ get: () => c.dumpTops.get(at) ?? 0, set: (top) => c.dumpTops.set(at, top) }),
    }),
  );
  return host;
}

function drawMore(c: DrawContext, item: Extract<Item, { kind: "more" }>): HTMLElement {
  const row = el("div", "rp-item rp-row rp-more");
  row.style.paddingLeft = `${8 + item.depth * 12}px`;
  const reply = c.doc.templateNode(item.path);
  const noun = reply.status === "ok" ? childWord(reply.node) : "item";
  row.append(el("span", "rp-at", ""));
  row.append(el("span", "rp-field", REPORT.more(countText(item.remaining, noun), item.side)));
  row.append(listButton(item.path));
  return row;
}

function mapFor(c: DrawContext, item: Item): HTMLElement {
  return fileMap(c.segments, item.offsetBits, item.sizeBits, rangeText(item.offsetBits, item.sizeBits), c.selected);
}

/** The control that shows an item's bytes, and takes them away again. */
function bytesButton(c: DrawContext, key: string): HTMLElement {
  const on = c.bytes.has(key);
  const b = el("button", `rp-bytes${on ? " is-on" : ""}`, REPORT.showBytes);
  b.type = "button";
  b.setAttribute("aria-pressed", String(on));
  b.dataset["bytes"] = key;
  return b;
}

/** The way out of a window and into the whole list. It sits on the list's own
 *  heading and on both ends of the drawn window, which is where a reader finds
 *  out the list is longer than what is in front of them. */
function listButton(path: readonly number[]): HTMLElement {
  const b = el("button", "rp-bytes rp-list", REPORT.paneOpen);
  b.type = "button";
  b.dataset["list"] = pathKey(path);
  return b;
}

/** What reads this field, as a link to it. */
function readsLink(reads: { readonly name: string; readonly path: readonly number[] }): HTMLElement {
  const link = el("button", "rp-reads", REPORT.reads(reads.name));
  link.type = "button";
  link.title = REPORT.readsLabel(reads.name);
  link.setAttribute("aria-label", REPORT.readsLabel(reads.name));
  link.dataset["reads"] = pathKey(reads.path);
  return link;
}

/** Whether a row has anything under it to open. */
export function itemOpens(node: TemplateNode): boolean {
  return node.composite && node.child_count > 0;
}

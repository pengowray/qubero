// A JPEG's tables, drawn the way they are used rather than the way they are
// stored.
//
// Read field by field, a quantisation table is sixty-four numbers in an order
// nobody thinks in, a Huffman table is sixteen counts and a run of bytes with
// the codes themselves written nowhere at all, and a frame header is three
// structures whose numbers only mean something together. Every one of those
// is a report the format already contains and the field list cannot show: the
// square, the code set, the picture's shape. So a JPEG segment that holds one
// of them is drawn as a card instead of as its fields.
//
// The same answer as `records.ts` and for the same reason. A registry, not an
// IR: what makes these four worth drawing is not anything declarative about
// their fields but that the reader's question about them is a different shape
// from "what is at this offset". The arithmetic that reorders them lives in
// `jpegtables.ts`, with no document and no DOM in it; this file is the
// looking-up and the drawing.
//
// What every card keeps is the way back to the bytes. A cell of a
// quantisation grid, a row of a component table and a line of a decoded code
// set each carry the path of the field they were read from, so clicking one
// selects that field exactly as clicking a cell of a record table does.

import { formatBytes, formatOffset } from "./doc.js";
import type { Doc, TemplateNode } from "./doc.js";
import type { Item } from "./flatten.js";
import { pathKey } from "./flatten.js";
import type { DrawContext } from "./listingdraw.js";
import { el } from "./listingdraw.js";
import { countRestarts, dezigzag, huffmanCodes, subsampling } from "./jpegtables.js";
import { bitSizeText, countText, JPEG, REPORT } from "./strings.js";

/** The four segments a JPEG has a card for, named by what they hold. */
export type JpegCardKind = "quant" | "huffman" | "frame" | "scan";

/** Past this, the entropy-coded data is not read to count its restart
 *  markers. The count is a footnote on the scan's summary and is not worth
 *  a gigabyte through the browser to get. */
const RESTART_SCAN_LIMIT = 64 * 1024 * 1024;

/**
 * Whether this node is a JPEG segment with a card, and which one.
 *
 * Asked of every heading the listing opens, so it stops at the first thing
 * that rules the node out: nearly every node is not a JPEG segment and the
 * check for that is two string comparisons.
 */
export function jpegCardKind(doc: Doc, node: TemplateNode): JpegCardKind | null {
  if (doc.template !== "jpeg") return null;
  if (node.type !== "Segment" || node.child_count < 2) return null;
  const body = kid(doc, node, 1);
  if (body === null) return null;
  // The scan is the one segment whose body is not a length and its contents:
  // the entropy-coded bits after the header have no count on them anywhere.
  if (body.type === "Scan") return "scan";
  if (body.type !== "Body") return null;
  const contents = kid(doc, body, 1);
  switch (contents?.type) {
    case "QuantTable[]":
      return "quant";
    case "HuffmanTable[]":
      return "huffman";
    case "Frame":
      return "frame";
    default:
      return null;
  }
}

/** One child of a node, or null while its bytes are still on their way. */
function kid(doc: Doc, node: TemplateNode, at: number): TemplateNode | null {
  const reply = doc.templateChildren(node.path, at, at + 1);
  return reply.status === "ok" ? (reply.node[0] ?? null) : null;
}

/** A run of a node's children, or null while they are being read. */
function kids(doc: Doc, node: TemplateNode, count = node.child_count): readonly TemplateNode[] | null {
  if (count === 0) return [];
  const reply = doc.templateChildren(node.path, 0, count);
  return reply.status === "ok" ? reply.node : null;
}

/** What an enumerated field is called, without the raw number the listing's
 *  value column carries: `edit_text` is the name on its own. */
function enumName(node: TemplateNode | null): string {
  return node === null ? "" : node.edit_text;
}

function numberOf(node: TemplateNode | null): number {
  const n = node === null ? Number.NaN : Number(node.value);
  return Number.isFinite(n) ? n : Number.NaN;
}

// ----- drawing -----

export function drawJpegCard(c: DrawContext, item: Extract<Item, { kind: "formatcard" }>): HTMLElement {
  const host = el("div", `rp-item rp-jc jc-${item.card}`);
  const node = item.node;
  const body = kid(c.doc, node, 1);
  if (body === null) {
    host.append(el("div", "bs-wait", JPEG.waiting));
    return host;
  }
  switch (item.card as JpegCardKind) {
    case "scan":
      scanCard(c, host, body);
      break;
    case "quant":
      // The sentence is about what any of these grids means, so it is said
      // once under the segment rather than once under each table.
      if (tablesCard(c, host, body, quantTable)) host.append(el("p", "jc-note", JPEG.quantNote));
      break;
    case "huffman":
      tablesCard(c, host, body, huffmanTable);
      break;
    case "frame":
      frameCard(c, host, body, kid(c.doc, node, 0));
      break;
  }
  return host;
}

/** The segments that hold a list of tables: one block per table, under the
 *  one heading the segment already has. False when the segment's bytes are
 *  not all here and nothing was drawn. */
function tablesCard(
  c: DrawContext,
  host: HTMLElement,
  body: TemplateNode,
  draw: (c: DrawContext, host: HTMLElement, table: TemplateNode) => void,
): boolean {
  const contents = kid(c.doc, body, 1);
  const tables = contents === null ? null : kids(c.doc, contents);
  if (tables === null) {
    host.append(el("div", "bs-wait", JPEG.waiting));
    return false;
  }
  for (const table of tables) draw(c, host, table);
  return tables.length > 0;
}

// ----- quantisation -----

/** One quantisation table as the 8 by 8 square it is applied as. The file
 *  writes it along the diagonals, so nothing about the stored order tells a
 *  reader which corner is which; the square does. */
function quantTable(c: DrawContext, host: HTMLElement, table: TemplateNode): void {
  const precision = kid(c.doc, table, 0);
  const id = kid(c.doc, table, 1);
  const values = kid(c.doc, table, 2);
  const cells = values === null ? null : kids(c.doc, values);
  const block = el("div", "jc-block");
  const numbers = (cells ?? []).map(numberOf).filter((n) => Number.isFinite(n));
  const low = numbers.length === 0 ? 0 : Math.min(...numbers);
  const high = numbers.length === 0 ? 0 : Math.max(...numbers);
  block.append(
    head(JPEG.quantTable(id?.value ?? "", enumName(precision)), numbers.length === 0 ? "" : JPEG.quantRange(String(low), String(high))),
  );
  if (cells === null) {
    block.append(el("div", "bs-wait", JPEG.waiting));
    host.append(block);
    return;
  }
  const grid = el("div", "jc-grid");
  // The tint is against the largest number in this table, so the pattern
  // inside one table reads at a glance; the range on the heading line is what
  // makes two tables comparable, since a tint cannot be read as a number.
  for (const cell of dezigzag(cells)) {
    if (cell === undefined) {
      grid.append(el("span", "jc-cell jc-cell-empty", ""));
      continue;
    }
    const value = numberOf(cell);
    const button = fieldButton(c, cell, "jc-cell", cell.value);
    if (Number.isFinite(value) && high > 0) {
      const share = Math.max(0, Math.min(1, value / high));
      button.style.background = `color-mix(in srgb, var(--fg) ${(5 + 33 * share).toFixed(1)}%, transparent)`;
    }
    grid.append(button);
  }
  block.append(grid);
  host.append(block);
}

// ----- Huffman -----

/** One Huffman table: the shape of it as sixteen bars, and the codes
 *  themselves behind a control, since the codes are long and the shape is
 *  what a reader is usually after. */
function huffmanTable(c: DrawContext, host: HTMLElement, table: TemplateNode): void {
  const cls = kid(c.doc, table, 0);
  const id = kid(c.doc, table, 1);
  const countsNode = kid(c.doc, table, 2);
  const symbolsNode = kid(c.doc, table, 3);
  const countNodes = countsNode === null ? null : kids(c.doc, countsNode);
  const block = el("div", "jc-block");
  const counts = (countNodes ?? []).map(numberOf);
  const total = counts.reduce((a, b) => a + (Number.isFinite(b) ? b : 0), 0);
  block.append(head(JPEG.huffmanTable(enumName(cls).toUpperCase(), id?.value ?? ""), total === 0 ? "" : JPEG.huffmanTotal(countText(total, "code"))));
  if (countNodes === null) {
    block.append(el("div", "bs-wait", JPEG.waiting));
    host.append(block);
    return;
  }
  block.append(el("p", "jc-caption", JPEG.huffmanCounts));
  const bars = el("div", "jc-bars");
  const tallest = Math.max(1, ...counts);
  countNodes.forEach((node, i) => {
    const column = fieldButton(c, node, "jc-bar", "");
    column.title = JPEG.huffmanBar(countText(counts[i] ?? 0, "code"), i + 1);
    const well = el("span", "jc-bar-well");
    const stem = el("span", "jc-bar-stem");
    // A count of nothing still gets a column so the sixteen lengths line up;
    // it just has no bar in it.
    stem.style.height = `${Math.round((100 * (counts[i] ?? 0)) / tallest)}%`;
    well.append(stem);
    column.append(well);
    column.append(el("span", "jc-bar-count", String(counts[i] ?? 0)));
    column.append(el("span", "jc-bar-len", String(i + 1)));
    bars.append(column);
  });
  block.append(bars);
  codes(c, block, table, counts, symbolsNode);
  host.append(block);
}

/** The codes this table stands for, worked out from the counts and the
 *  symbols, behind a control. Nothing in the file holds them. */
function codes(
  c: DrawContext,
  block: HTMLElement,
  table: TemplateNode,
  counts: readonly number[],
  symbolsNode: TemplateNode | null,
): void {
  const key = `${pathKey(table.path)}:codes`;
  const open = c.cards.has(key);
  block.append(disclosure(c, key, open ? JPEG.huffmanHide : JPEG.huffmanShow));
  if (!open || symbolsNode === null) return;
  const read = c.doc.read(symbolsNode.offset_bits / 8, symbolsNode.size_bits / 8);
  if (!read.complete) {
    block.append(el("div", "bs-wait", JPEG.waiting));
    return;
  }
  const built = huffmanCodes(counts, [...read.bytes]);
  if (built === null) {
    block.append(el("p", "jc-note", JPEG.huffmanMismatch));
    return;
  }
  const grid = document.createElement("table");
  grid.className = "rec jc-codes";
  const header = document.createElement("tr");
  for (const name of [JPEG.huffmanCodeColumn, JPEG.huffmanBitsColumn, JPEG.huffmanSymbolColumn]) header.append(el("th", "", name));
  grid.append(header);
  for (const code of built) {
    const tr = document.createElement("tr");
    tr.append(el("td", "jc-bits", code.bits));
    tr.append(el("td", "", String(code.length)));
    tr.append(el("td", "", `0x${code.symbol.toString(16).padStart(2, "0")}`));
    grid.append(tr);
  }
  block.append(grid);
}

// ----- the frame header -----

/** What the picture is: its size, its precision, and how each channel is
 *  sampled. The one card whose facts a reader wants before any other, which
 *  is why the size is the line and the channels are the table under it. */
function frameCard(c: DrawContext, host: HTMLElement, body: TemplateNode, marker: TemplateNode | null): void {
  const frame = kid(c.doc, body, 1);
  const fields = frame === null ? null : kids(c.doc, frame);
  if (fields === null) {
    host.append(el("div", "bs-wait", JPEG.waiting));
    return;
  }
  const [precision, height, width, , components] = fields;
  const channels = components === undefined ? null : kids(c.doc, components);
  const parts: (readonly [string, TemplateNode | undefined])[] = [
    [JPEG.frameSize(width?.value ?? "", height?.value ?? ""), width],
    [JPEG.framePrecision(precision?.value ?? ""), precision],
  ];
  const rows = (channels ?? []).map((channel) => kids(c.doc, channel));
  const sampling = rows.every((r): r is readonly TemplateNode[] => r !== null)
    ? subsampling(rows.map((r) => ({ h: numberOf(r[1] ?? null), v: numberOf(r[2] ?? null) })))
    : null;
  // One channel has no colour to subsample, so it is named rather than given
  // a ratio: "greyscale subsampling" would be a phrase about nothing.
  if (sampling === "greyscale") parts.push([JPEG.frameGreyscale, components]);
  else if (sampling !== null) parts.push([JPEG.frameSubsampling(sampling), components]);
  // Baseline or progressive, which is not in the frame at all: it is which
  // marker opened it. The heading above says the same thing in the format's
  // own abbreviation; this line is where a reader is already reading what the
  // picture is, and the answer belongs with the rest of it.
  const coding = enumName(marker).split(", ")[1];
  if (marker !== null && coding !== undefined) parts.push([coding, marker]);
  const line = el("div", "jc-line");
  for (const [text, node] of parts) {
    line.append(node === undefined ? el("span", "jc-fact", text) : fieldButton(c, node, "jc-fact", text));
  }
  host.append(line);
  if (channels === null) {
    host.append(el("div", "bs-wait", JPEG.waiting));
    return;
  }
  const grid = table([...JPEG.frameColumns]);
  channels.forEach((channel, i) => {
    const f = rows[i];
    if (f === undefined || f === null) return;
    grid.append(row(c, channel, [f[0], f[1], f[3]], [undefined, `${f[1]?.value ?? ""} × ${f[2]?.value ?? ""}`]));
  });
  host.append(grid);
}

// ----- the scan -----

/** The scan, which is nearly the whole file and none of which is decoded.
 *  What can honestly be said about it is on one line; the header that says
 *  which tables each channel reads with is behind a control, because a
 *  reader who has scrolled to the scan is usually looking for the size of it
 *  rather than for its six header fields. */
function scanCard(c: DrawContext, host: HTMLElement, body: TemplateNode): void {
  const header = kid(c.doc, body, 1);
  const entropy = kid(c.doc, body, 2);
  const fields = header === null ? null : kids(c.doc, header);
  const components = fields?.[1] ?? null;
  const channels = components === null ? null : kids(c.doc, components);
  const line = el("div", "jc-line");
  if (channels !== null && components !== null) {
    line.append(fieldButton(c, components, "jc-fact", JPEG.scanComponents(countText(channels.length, "channel"))));
  }
  if (entropy !== null) {
    line.append(fieldButton(c, entropy, "jc-fact", JPEG.scanEntropy(formatBytes(entropy.size_bits / 8))));
    const restarts = restartCount(c.doc, entropy);
    if (restarts === null) line.append(el("span", "jc-fact jc-dim", JPEG.scanRestartsUnread));
    else if (restarts > 0) line.append(el("span", "jc-fact", JPEG.scanRestarts(countText(restarts, "restart marker"))));
  }
  host.append(line);
  const key = `${pathKey(body.path)}:header`;
  const open = c.cards.has(key);
  host.append(disclosure(c, key, open ? JPEG.scanHide : JPEG.scanShow));
  if (!open || fields === null) return;
  const from = fields[2];
  const to = fields[3];
  if (from !== undefined && to !== undefined) {
    const band = el("div", "jc-line");
    band.append(fieldButton(c, from, "jc-fact", JPEG.scanBand(from.value, to.value)));
    host.append(band);
  }
  if (channels === null) {
    host.append(el("div", "bs-wait", JPEG.waiting));
    return;
  }
  const grid = table([...JPEG.scanColumns]);
  for (const channel of channels) {
    const f = kids(c.doc, channel);
    if (f === null) continue;
    grid.append(row(c, channel, [f[0], f[1], f[2]]));
  }
  host.append(grid);
}

/** Restart markers already counted, by the stretch they were counted over.
 *  The scan is most of the file and the listing draws again on every scroll,
 *  so this is counted once per document rather than once per paint. */
const restarts = new WeakMap<Doc, Map<string, number>>();

/** How many restart markers are in this scan, or null while its bytes are
 *  not all here. A read of a stretch that is missing chunks asks for them and
 *  the listing draws again when they land, so null resolves itself. */
function restartCount(doc: Doc, entropy: TemplateNode): number | null {
  const bytes = entropy.size_bits / 8;
  if (bytes <= 0 || bytes > RESTART_SCAN_LIMIT) return null;
  let known = restarts.get(doc);
  if (known === undefined) {
    known = new Map();
    restarts.set(doc, known);
  }
  const key = `${entropy.offset_bits}:${entropy.size_bits}`;
  const had = known.get(key);
  if (had !== undefined) return had;
  const read = doc.read(entropy.offset_bits / 8, bytes);
  if (!read.complete) return null;
  const n = countRestarts(read.bytes);
  known.set(key, n);
  return n;
}

// ----- the pieces every card is built from -----

/** A table's own line inside a card: what it is, and the one fact worth
 *  putting beside the name. */
function head(name: string, aside: string): HTMLElement {
  const line = el("div", "jc-head");
  line.append(el("b", "jc-title", name));
  if (aside !== "") line.append(el("span", "jc-aside", aside));
  return line;
}

function table(columns: readonly string[]): HTMLTableElement {
  const grid = document.createElement("table");
  grid.className = "rec jc-table";
  const header = document.createElement("tr");
  for (const name of columns) header.append(el("th", "", name));
  header.append(el("th", "rec-at", REPORT.storedAt));
  grid.append(header);
  return grid;
}

/** One line of a card's table, standing for one structure in the file: the
 *  same shape a record table's line has, so the same code marks it when the
 *  selection lands inside it. */
function row(c: DrawContext, node: TemplateNode, cells: readonly (TemplateNode | undefined)[], texts: readonly (string | undefined)[] = []): HTMLTableRowElement {
  const tr = document.createElement("tr");
  tr.dataset["at"] = String(node.offset_bits);
  tr.dataset["size"] = String(node.size_bits);
  cells.forEach((field, i) => tr.append(cell(c, field, texts[i])));
  const at = el("td", "rec-at");
  const link = el("button", "rec-link", formatOffset(node.offset_bits));
  link.type = "button";
  link.title = bitSizeText(node.size_bits);
  const key = `r:${pathKey(node.path)}`;
  if (c.bytes.has(key)) link.classList.add("is-on");
  link.addEventListener("click", (e) => {
    e.stopPropagation();
    c.toggleBytes(key);
  });
  at.append(link);
  tr.append(at);
  return tr;
}

/** One cell of a card's table, which is one field of the file. */
function cell(c: DrawContext, node: TemplateNode | undefined, text?: string): HTMLElement {
  const td = el("td", "");
  if (node === undefined) return td;
  td.append(fieldButton(c, node, "jc-value", text ?? node.value));
  return td;
}

/**
 * Anything on a card that stands for one field, as the control that goes to
 * it. `data-reads` is the route every cross-reference in the listing already
 * takes, so a click here selects the field and every other view follows,
 * exactly as clicking a cell of a record table does.
 */
function fieldButton(c: DrawContext, node: TemplateNode, className: string, text: string): HTMLButtonElement {
  const b = el("button", className, text);
  b.type = "button";
  b.dataset["reads"] = pathKey(node.path);
  b.dataset["at"] = String(node.offset_bits);
  b.dataset["size"] = String(node.size_bits);
  b.title = `${node.name} · ${formatOffset(node.offset_bits)}`;
  if (c.selected !== null && c.selected.offsetBits === node.offset_bits && c.selected.sizeBits === node.size_bits) {
    b.classList.add("is-on");
  }
  return b;
}

/** The control that opens the long half of a card and puts it away again.
 *  The card is measured rather than declared, so opening one has to tell the
 *  listing to lay itself out again, which is what the callback is for. */
function disclosure(c: DrawContext, key: string, label: string): HTMLElement {
  const on = c.cards.has(key);
  const b = el("button", `jc-more${on ? " is-on" : ""}`, label);
  b.type = "button";
  b.setAttribute("aria-expanded", String(on));
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    c.toggleCard(key);
  });
  return b;
}

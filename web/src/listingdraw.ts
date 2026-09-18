// One item of the listing as one element.
//
// Split out of `listingreport.ts`, which had grown to hold the layout, the
// input, the selection and the drawing all at once. This half is the drawing
// only: an item goes in, an element comes out, and nothing here knows where in
// the document it lands, how tall it turned out to be, or what the reader does
// to it next. What it does need from the report — the file, the parts of it a
// map is drawn from, what is selected, what is open — arrives as a
// `DrawContext`, read fresh for each paint.

import { formatAddress, formatBytes, formatOffset } from "./doc.ts";
import type { Doc, TemplateNode } from "./doc.ts";
import { address } from "./dom.ts";
import { DUMP_MIN_BYTES, isComputed, pathKey, PAGE } from "./flatten.ts";
import type { Item } from "./flatten.ts";
import { fieldClass, sectionColor } from "./fieldstyle.ts";
import { COLOUR_TYPE, swatch } from "./colour.ts";
import { byteStrip } from "./bytestrip.ts";
import { byteDump } from "./bytedump.ts";
import { drawCard } from "./contentcard.ts";
import { drawJpegCard } from "./jpegcards.ts";
import { fileMap } from "./filemap.ts";
import { recordTable } from "./records.ts";
import { tablePlan } from "./tableplan.ts";
import type { RecordCell } from "./records.ts";
import type { GapVerdict } from "./gapcheck.ts";
import type { MapSegment } from "./filemap.ts";
import { JOINED_WHOLE_CAP_BITS } from "./joinedpart.ts";
import { bitSizeText, childWord, countText, DECODED_PLUS_TITLE, DECODED_REFUSED, DECODED_REFUSED_OTHER, GAP_LABEL, JOINED, LISTING_SWITCH, PROBLEMS, REPORT, UNPACKED, TABLE } from "./strings.ts";

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
  /** The long halves of the format cards the reader has opened, by the key
   *  the card gives them. Kept by the report for the same reason `dumps` is:
   *  a card is built again from nothing every time anything on screen moves,
   *  so nothing the reader opened can live in the element. */
  readonly cards: ReadonlySet<string>;
  readonly toggleCard: (key: string) => void;
  /** Turn the listing's own switch over, by the key the switch item carries. */
  readonly toggleSwitch: (key: string) => void;
  readonly toggleBytes: (key: string) => void;
  readonly toggleDump: (offsetBits: number) => void;
  /** What a run of unclaimed bytes turned out to hold. The answer is cached by
   *  the report, since finding it reads the file. */
  readonly verdict: (item: Extract<Item, { kind: "gap" }>) => GapVerdict;
  /** The path keys of the compressed streams that have a row of their own in
   *  this listing, so what a stream holds does not offer to open it a second
   *  time when the stream is drawn right above. */
  readonly streams: ReadonlySet<string>;
  /** Whether the listing is on screen. A hidden one still draws a screenful
   *  so the other views can have its headings, and work that is only for the
   *  eye, like decoding a picture, waits for this. */
  readonly shown: boolean;
};

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** `@0x1000 – @0x1fff`, the stretch a heading covers. A part of no bytes has no
 *  range to give, which is what a field placed somewhere else looks like. */
export function rangeText(offsetBits: number, sizeBits: number, space = 0): string {
  if (sizeBits === 0) return formatAddress(offsetBits, space);
  return `${formatAddress(offsetBits, space)} – ${formatAddress(offsetBits + sizeBits - 8, space)}`;
}

/** Which address space an item's bytes are counted in. Items with no field of
 *  their own are the file's: a gap is a stretch of the file by definition. */
export function spaceOf(item: Item): number {
  return "node" in item && item.node !== null ? item.node.space : 0;
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

/** What a heading says: the part's title, or for a run of fields with no
 *  field of its own to name it, where the run sits. */
export function headingTitle(item: Extract<Item, { kind: "heading" }>, fileBits: number): string {
  return item.node === null ? REPORT.unnamedPart(runPosition(item, fileBits)) : item.title;
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
      return el("div", "rp-item rp-block rp-pending", REPORT.reading);
    case "card":
      return drawCard(c.doc, item, c.shown);
    case "switch":
      return drawSwitch(c, item);
    case "formatcard":
      return drawFormatCard(c, item);
  }
}

/** The one switch a template offers over its own rows, at the top of the
 *  listing. A real checkbox in a label, so Tab reaches it and Space works it
 *  the way it works every other checkbox; the listing's own keyboard answers
 *  Enter on it as well, since arrowing onto a row and pressing Enter is what
 *  every other item here does. */
function drawSwitch(c: DrawContext, item: Extract<Item, { kind: "switch" }>): HTMLElement {
  const host = el("div", "rp-item rp-switch");
  const words = LISTING_SWITCH[item.switch];
  if (words === undefined) return host;
  const label = el("label", "rp-switch-label");
  label.title = words.title;
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = item.on;
  box.addEventListener("change", () => c.toggleSwitch(item.switch));
  label.append(box, document.createTextNode(words.label));
  host.append(label);
  return host;
}

/** A structure the format keeps in a shape that is not the shape it means,
 *  drawn as that shape. Only JPEG has these; the switch is here rather than
 *  in a registry of one. */
function drawFormatCard(c: DrawContext, item: Extract<Item, { kind: "formatcard" }>): HTMLElement {
  const host = drawJpegCard(c, item);
  host.classList.add("rp-block");
  return host;
}

/** The name and its fold marker, in the one cell that steps in with depth.
 *  The address, value, type and size columns stay where they are at every
 *  depth, so they still read down the page; how far in the name sits is set
 *  by the layout (`--rp-ind`), which knows the heading the row is under. */
function treeCell(twist: string, field: HTMLElement): HTMLElement {
  const cell = el("span", "rp-tree");
  cell.append(el("span", "rp-twist", twist), field);
  return cell;
}

function drawHeading(c: DrawContext, item: Extract<Item, { kind: "heading" }>, fileBits: number): HTMLElement {
  const row = el("div", `rp-item rp-h${item.level}`);
  if (item.level === 0) {
    const swatch = el("span", "rp-swatch");
    swatch.style.background = sectionColor(item.section);
    row.append(swatch);
  }
  row.append(el("b", "rp-name", headingTitle(item, fileBits)));
  // The same mark a hopping row wears in its type column. A heading has no
  // type column to put it in, so it goes beside the title, where the range
  // that follows it is the one a reader would otherwise expect to find inside
  // whatever this sits under. Only on a part that was reached by address.
  if (item.via !== null) row.append(el("span", "rp-via", item.via));
  const space = spaceOf(item);
  row.append(el("span", "rp-range", rangeText(item.offsetBits, item.sizeBits, space)));
  // The byte strip and the file map both show bytes of the file, and a
  // decoded field has none: its bytes came out of a stream and are nowhere in
  // the file to point at. See DESIGN.md, "A stream read as the fields inside
  // it".
  if (space === 0) row.append(bytesButton(c, item.key));
  offerUnpacked(c, row, item, item.node);
  // Only a list too long to draw: for anything the window already holds
  // whole, a pane of its own would be the same rows somewhere else.
  if (item.node !== null && item.node.child_count > PAGE) row.append(listButton(item.path));
  // Beside it, where the list reads as rows under columns: the pane shows a
  // long list one element to a line, and a table shows the same elements as
  // the records they are. The two are different questions about one list, so
  // both are offered rather than one replacing the other.
  offerTable(c, row, item.node);
  // The facts about the part's place in the file sit together at the right:
  // how big it is, how much of the file that is, and where.
  const share = space === 0 ? shareText(item.sizeBits, fileBits) : "";
  row.append(el("span", "rp-size", `${formatBytes(item.sizeBits / 8)}${share === "" ? "" : ` · ${share}`}`));
  if (space === 0) row.append(mapFor(c, item));
  return row;
}

function drawRow(c: DrawContext, item: Extract<Item, { kind: "row" }>): HTMLElement {
  const n = item.node;
  // A field of no bytes is grey: whether it is a value the template worked
  // out or a list that turned out to be empty, there is nothing of it in the
  // file, and a row the reader can skip should look like one.
  const row = el("div", `rp-item rp-row${n.size_bits === 0 ? " rp-nobytes" : ""}${item.open ? " is-open" : ""}`);
  if (isSelected(c.selected, item.offsetBits, item.sizeBits) || c.nearest === item.key) row.classList.add("is-on");
  // A computed value is not written anywhere, so it has no address, and its
  // length says so in words: "@0x101a7" and "0 bytes" would be answers to
  // questions this row is not the answer to.
  const written = !isComputed(n);
  const at = el("span", "rp-at");
  // The leading plus says the address is counted inside a stream, and carries
  // what it counts from on the mark itself rather than on the whole address:
  // the sign is the part that changes meaning, so the sign is what answers for
  // it. This column has no room for a second line saying so, which is why the
  // hover is the only place that fact can live here.
  if (written) at.append(...address(formatAddress(n.offset_bits, n.space), n.joined ? JOINED.plusTitleStream : DECODED_PLUS_TITLE));
  row.append(at);
  // A row that opens says so. Without it the only way to find out which
  // rows have anything under them is to click every one of them.
  const name = el("span", `rp-field ${fieldClass(n.kind)}`, n.name);
  // The format's own words for the field, on hover: a column is too narrow
  // for a sentence, and most fields have none.
  if (n.doc !== undefined) name.title = n.doc;
  row.append(treeCell(itemOpens(n) ? (item.open ? "▾" : "▸") : "", name));
  // A compressed run nothing could open says why where its count would be:
  // "0 fields" is true and tells the reader nothing they can act on.
  //
  // The count is of the rows below, not of the children the structure has: a
  // dict entry whose opcode bytes are hidden shows two rows, and "5 fields"
  // over two of them is the row disagreeing with itself. `shownChildren` is
  // null whenever the two are the same, and whenever the walk had no grounds
  // to say otherwise.
  const said =
    n.refused !== null
      ? (DECODED_REFUSED[n.refused] ?? DECODED_REFUSED_OTHER)
      : n.composite
        ? countText(item.shownChildren ?? n.child_count, childWord(n))
        : n.value;
  const value = el("span", "rp-value", said);
  // A colour written as `ansi256(34)` or `#5769f7` says nothing to read; the
  // square says what it is. Only where the template called the field a colour,
  // so nothing guesses at text that happens to look like one.
  if (n.type === COLOUR_TYPE) {
    const box = swatch(said);
    if (box !== null) value.prepend(box);
  }
  if (item.reads !== null) value.append(readsLink(item.reads));
  markProblem(row, value, n, item.open);
  // A row that stands for a pointer and what it points at says so here, and
  // only here: `at → ObjectHeader` names the thing and says it was reached
  // rather than contained, which is what the step of indent above it cannot
  // be trusted to mean. Every other row is where it was declared and the
  // column is the plain type, so the arrow marks the rows that hop and
  // nothing else.
  row.append(el("span", "rp-type", item.via ?? n.type));
  row.append(el("span", "rp-size", written ? bitSizeText(n.size_bits) : REPORT.notStored));
  // A toggle that opens a strip of nothing is a dead control, and so is one
  // over bytes that are not in the file.
  if (n.size_bits > 0 && n.space === 0) row.append(bytesButton(c, item.key));
  // A ROOT record draws its compressed run as a row rather than as a heading,
  // and it is the same offer there.
  offerUnpacked(c, row, item, n);
  return row;
}


/** The mark on a row whose value is wrong, and the count on a row with wrong
 *  values under it.
 *
 *  A glyph leads the value either way, since one shape in one place is what a
 *  reader learns to scan for. What follows it depends on who says the value is
 *  wrong: the format ruling a value out is a finding, so the reason is on the
 *  row and the value takes the warning colour; Qubero having no name for a
 *  value is not, so the glyph stands alone and the reason is on the row's
 *  hover. Nothing here fills or recolours anything else: colour on these rows
 *  is already spent on what kind of field it is.
 *
 *  The count on a composite says `so far` while the row is closed, because
 *  that is exactly when the children under it are the ones the core happened
 *  to read rather than all of them. An open row has its children below it,
 *  each carrying its own mark. */
function markProblem(row: HTMLElement, value: HTMLElement, n: TemplateNode, open: boolean): void {
  const problem = n.problem;
  if (problem !== undefined) {
    const invalid = problem.tier === "invalid";
    value.prepend(glyph(invalid));
    if (invalid) {
      value.classList.add("is-invalid");
      row.append(value);
      const why = el("span", "rp-problem", problem.text);
      why.title = problem.text;
      row.append(why);
    } else {
      // The words are one hover away rather than on the row: an undefined
      // value is not a finding, and a column of reasons beside every wasm
      // opcode nobody has catalogued would bury the ones that are.
      row.title = problem.text;
      row.append(value);
    }
    return;
  }
  row.append(value);
  const [invalid, undefinedCount] = n.problems_within;
  if (invalid === 0 && undefinedCount === 0) return;
  const count = el("span", "rp-problem rp-within", PROBLEMS.within(invalid, undefinedCount, !open));
  row.append(count);
}

/** The one mark both tiers wear, which only says look here. Hidden from a
 *  screen reader, which is given the words instead. */
function glyph(invalid: boolean): HTMLElement {
  const dot = el("span", `problem-glyph ${invalid ? "is-invalid" : "is-undefined"}`, PROBLEMS.glyph);
  dot.setAttribute("aria-hidden", "true");
  return dot;
}

function drawGap(c: DrawContext, item: Extract<Item, { kind: "gap" }>): HTMLElement {
  const row = el("div", "rp-item rp-row rp-gap");
  row.append(el("span", "rp-at", formatOffset(item.offsetBits)));
  row.append(treeCell("", el("span", "rp-field", item.unmapped ? GAP_LABEL : REPORT.gap)));
  // A gap short enough to read is shown, the way a `reserved` field's bytes
  // are; a longer one gets a word about what is in it, and its dump below.
  row.append(el("span", "rp-value", gapBytes(c, item) ?? GAP_VERDICT[c.verdict(item)]));
  row.append(el("span", "rp-type", ""));
  row.append(el("span", "rp-size", bitSizeText(item.sizeBits)));
  // The one question a gap row cannot answer in a word: what is actually in
  // there. Same control as a field's, and it opens on the same bytes.
  if (item.sizeBits > 0) row.append(bytesButton(c, item.key));
  return row;
}

/** The hex of a gap the row itself can show: whole bytes, no more than the
 *  value column's preview holds. Null for a longer or an unaligned one, and
 *  while the bytes are still arriving. */
function gapBytes(c: DrawContext, item: Extract<Item, { kind: "gap" }>): string | null {
  if (item.offsetBits % 8 !== 0 || item.sizeBits % 8 !== 0 || item.sizeBits > DUMP_MIN_BYTES * 8 || item.sizeBits === 0) return null;
  const { bytes, complete } = c.doc.read(item.offsetBits / 8, item.sizeBits / 8);
  if (!complete) return null;
  return Array.from(bytes.subarray(0, item.sizeBits / 8), (b) => b.toString(16).padStart(2, "0")).join(" ");
}

/** A structure the format keeps as a table, drawn as one: the format's own
 *  column names, and where each row is written. */
function drawRecord(c: DrawContext, item: Extract<Item, { kind: "record" }>): HTMLElement {
  const host = el("div", "rp-item rp-block rp-record");
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
    for (const cell of row.cells) tr.append(drawCell(cell));
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

/** One cell of a record table. A value that names another part of the file
 *  is a link there, which is rule 7's cross-reference: `data-reads` is the
 *  same route the rows' "→ cells" links already take. */
function drawCell(cell: RecordCell): HTMLElement {
  const problem = cell.problem;
  const invalid = problem?.tier === "invalid";
  const td = el("td", `${fieldClass(cell.kind)}${invalid ? " is-invalid" : ""}`);
  if (problem !== undefined) td.title = `${cell.text}\n${problem.text}`;
  if (cell.link === undefined) {
    td.textContent = cell.text;
    if (problem !== undefined) td.prepend(glyph(invalid));
    return td;
  }
  const link = el("button", "rec-link", cell.link.text);
  link.type = "button";
  link.title = cell.link.label;
  link.setAttribute("aria-label", cell.link.label);
  link.dataset["reads"] = pathKey(cell.link.path);
  td.append(link);
  return td;
}

function drawStrip(c: DrawContext, item: Extract<Item, { kind: "bytes" }>): HTMLElement {
  const host = el("div", "rp-item rp-block rp-strip");
  // A gap has no field to take a name from, so the row's own word names it.
  const gap = item.owner.startsWith("gap:");
  const name = gap ? GAP_LABEL : item.name;
  const caption = `${name} ${rangeText(item.offsetBits, item.sizeBits)}`;
  // A strip is a map of the fields in a stretch, and a gap or one opaque
  // field has none: its columns would be one column. What the reader wants
  // there is the bytes, so it opens straight into the dump. It costs nothing
  // however long the stretch is, since only the lines on screen are read.
  if (item.dump && item.offsetBits % 8 === 0 && item.sizeBits % 8 === 0) {
    host.append(dumpOf(c, item, name, caption));
    return host;
  }
  host.append(
    byteStrip(c.doc, item.offsetBits, item.sizeBits, caption, mapFor(c, item), () => c.toggleBytes(item.owner), c.selected, {
      open: c.dumps,
      toggle: (at) => c.toggleDump(at),
      scroll: (at) => ({ get: () => c.dumpTops.get(at) ?? 0, set: (top) => c.dumpTops.set(at, top) }),
    }),
  );
  return host;
}

/** A run of bytes with no fields in it, opened: a gap, or one opaque field.
 *  The same caption and the same way out as a byte strip has, so one control
 *  closes either. */
function dumpOf(c: DrawContext, item: Extract<Item, { kind: "bytes" }>, name: string, caption: string): HTMLElement {
  const strip = el("div", "bstrip bs-dump");
  const cap = el("div", "bs-cap");
  cap.append(el("span", "bs-cap-text", caption), mapFor(c, item));
  const close = el("button", "bs-close", REPORT.hideBytes);
  close.type = "button";
  close.addEventListener("click", (e) => {
    e.stopPropagation();
    c.toggleBytes(item.owner);
  });
  cap.append(close);
  strip.append(cap);
  const at = item.offsetBits;
  strip.append(
    byteDump(c.doc, at / 8, item.sizeBits / 8, name, {
      get: () => c.dumpTops.get(at) ?? 0,
      set: (top) => c.dumpTops.set(at, top),
    }),
  );
  return strip;
}

function drawMore(c: DrawContext, item: Extract<Item, { kind: "more" }>): HTMLElement {
  const row = el("div", "rp-item rp-row rp-more");
  const reply = c.doc.templateNode(item.path);
  const noun = reply.status === "ok" ? childWord(reply.node) : "item";
  row.append(el("span", "rp-at", ""));
  row.append(treeCell("", el("span", "rp-field", REPORT.more(countText(item.remaining, noun), item.side))));
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
/**
 * Offer to open a compressed run as a document of its own, wherever the run is
 * drawn: some templates give the stream a row of its own and fold its contents
 * away, others fold the stream away and show only its contents, and a ROOT
 * record draws it as an ordinary row rather than as a heading.
 *
 * A run that would not open gets nothing: the line saying why is already where
 * its value would be. When both the stream and its contents are on screen, only
 * the stream offers it, so one stream is never two buttons.
 */
function offerUnpacked(c: DrawContext, row: HTMLElement, item: Item, n: TemplateNode | null): void {
  if (n === null) return;
  if (n.decoded && n.refused === null) {
    row.append(unpackedButton(item.path, n.name, false));
    return;
  }
  if (!n.space_root || item.path.length === 0) return;
  // A stream joined from several runs is held whole to be a document of its
  // own, which the core does up to the cap an unpacked run has and refuses
  // past it. The listing still reads a longer one a part at a time, so only
  // the button goes.
  if (n.joined && n.size_bits > JOINED_WHOLE_CAP_BITS) return;
  const stream = item.path.slice(0, -1);
  if (!c.streams.has(pathKey(stream))) row.append(unpackedButton(stream, n.name, n.joined));
}

/** The control that opens a compressed run, or a stream joined from several
 *  runs, as a document of its own. */
function unpackedButton(path: readonly number[], name: string, joined: boolean): HTMLElement {
  const b = el("button", "rp-bytes rp-unpacked", joined ? JOINED.open : UNPACKED.open);
  b.type = "button";
  b.title = UNPACKED.openTitle(name);
  b.dataset["unpacked"] = pathKey(path);
  return b;
}

/** Offer the table, where this node reads as one. The row word comes from the
 *  plan, so a run of samples says samples and a dBase file says records. */
function offerTable(c: DrawContext, row: HTMLElement, node: TemplateNode | null): void {
  if (node === null) return;
  const plan = tablePlan(c.doc, node);
  if (plan === null) return;
  const b = el("button", "rp-bytes rp-table", TABLE.open(plan.rowWord));
  b.type = "button";
  b.title = TABLE.tabTooltip(plan.rowWord, node.name, c.doc.name);
  b.dataset["table"] = pathKey(node.path);
  row.append(b);
}

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

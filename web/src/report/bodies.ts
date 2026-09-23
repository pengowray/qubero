// The three bodies a part can have: a field table for a short structure, a
// record table for a list of records, and a hex strip for bytes with no
// structure. Shared by the parts section and the content section.

import type { Doc, TemplateNode } from "../doc.ts";
import { fieldClass } from "../fieldstyle.ts";
import { formatOffset } from "../format.ts";
import type { TablePlan } from "../tableplan.ts";
import type { ReportData } from "./data.ts";
import { byteRef, dumpRows, pointAt } from "./refs.ts";
import type { ReportHost } from "./section.ts";
import { bitsText, RV } from "./text.ts";

/** Bytes shown inline in a row. */
const INLINE_BYTES = 8;

/** The first bytes of a stretch as hex, `ff d8 ff e0 …`, or dots while they
 *  are still being read. Empty for a stretch that does not start on a byte. */
export function firstBytes(doc: Doc, offsetBits: number, sizeBits: number, space = 0): string {
  if (space !== 0 || offsetBits % 8 !== 0 || sizeBits < 8) return "";
  const n = Math.min(INLINE_BYTES, Math.floor(sizeBits / 8));
  const r = doc.read(offsetBits / 8, n);
  const hex = Array.from(r.bytes, (b) => (r.complete ? b.toString(16).padStart(2, "0") : "··")).join(" ");
  return sizeBits / 8 > n ? `${hex} …` : hex;
}

function cell(tag: "td" | "th", text: string | Node, cls?: string): HTMLTableCellElement {
  const c = document.createElement(tag);
  if (cls !== undefined) c.className = cls;
  c.append(text);
  return c;
}

function head(...names: string[]): HTMLTableSectionElement {
  const thead = document.createElement("thead");
  const tr = document.createElement("tr");
  for (const n of names) tr.append(cell("th", n));
  thead.append(tr);
  return thead;
}

/** A value as the listing writes it, with the wrong-value glyph in front of it
 *  when the core says something is wrong, and the reason on hover. */
function valueCell(n: TemplateNode): HTMLTableCellElement {
  const td = document.createElement("td");
  td.className = `rv-val ${fieldClass(n.kind)}`;
  if (n.problem !== undefined) {
    const mark = document.createElement("span");
    mark.className = `problem-glyph is-${n.problem.tier}`;
    mark.textContent = "●";
    mark.title = n.problem.text;
    td.append(mark, " ");
    td.title = n.problem.text;
  }
  td.append(n.line ?? n.value);
  return td;
}

/** A field's address, as a reference, or its offset in a stream when it is
 *  not a place in the file. */
function atCell(n: TemplateNode): HTMLTableCellElement {
  if (n.space !== 0) return cell("td", `+${formatOffset(n.offset_bits).slice(1)}`, "rv-addr");
  return cell("td", byteRef({ path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits }, n.name), "rv-addr");
}

/** A structure's fields, one row each: name, value, address, size, bytes. */
export function fieldTable(doc: Doc, data: ReportData, fields: readonly TemplateNode[], more: number): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-fields";
  t.append(head(RV.colField, RV.colValue, RV.colAt, RV.colSize, RV.colBytes));
  const body = document.createElement("tbody");
  for (const n of fields) {
    const tr = document.createElement("tr");
    const name = document.createElement("code");
    name.textContent = n.name;
    if (n.doc !== undefined) {
      name.title = n.doc;
      name.className = "rv-hasdoc";
      // Named without the structure it was lifted out of (`body.width`), since
      // the description is of the field.
      data.term(n.name.replace(/^.*\./, ""), n.doc);
    }
    tr.append(cell("td", name), valueCell(n), atCell(n), cell("td", bitsText(n.size_bits), "rv-num"), cell("td", firstBytes(doc, n.offset_bits, n.size_bits, n.space), "rv-hex"));
    body.append(tr);
  }
  t.append(body);
  wrap.append(t);
  if (more > 0) wrap.append(moreLine(more, "field"));
  return wrap;
}

/** Records, one row each: its place in the list, address, size, what it reads
 *  as, and its first bytes. */
export function recordTable(doc: Doc, rows: readonly TemplateNode[], more: number, word: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-records";
  t.append(head(RV.colIndex, RV.colAt, RV.colSize, RV.colReads, RV.colBytes));
  const body = document.createElement("tbody");
  for (const n of rows) {
    const tr = document.createElement("tr");
    const index = n.path[n.path.length - 1] ?? 0;
    const reads = document.createElement("td");
    reads.className = "rv-reads";
    reads.append(n.line ?? n.value);
    if (n.problems_within[0] > 0) reads.classList.add("has-invalid");
    tr.append(cell("td", String(index), "rv-num"), atCell(n), cell("td", bitsText(n.size_bits), "rv-num"), reads, cell("td", firstBytes(doc, n.offset_bits, n.size_bits, n.space), "rv-hex"));
    body.append(tr);
  }
  t.append(body);
  wrap.append(t);
  if (more > 0) wrap.append(moreLine(more, word));
  return wrap;
}

/** The first rows of a table as the table view reads them: its own columns,
 *  from the plan the table view uses. */
export function planTable(plan: TablePlan, rows: number): { el: HTMLElement; complete: boolean } {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-plan";
  const names = [RV.colIndex, ...plan.columns.map((c) => (c.unit === "" ? c.name : `${c.name} (${c.unit})`))];
  t.append(head(...names));
  const body = document.createElement("tbody");
  let complete = true;
  const n = Math.min(rows, plan.count);
  for (let i = 0; i < n; i++) {
    const row = plan.row(i);
    if (row === null) {
      complete = false;
      break;
    }
    const tr = document.createElement("tr");
    const idx = cell("td", String(i), "rv-num");
    pointAt(idx, { path: row.path, startBit: row.offsetBits, endBit: row.offsetBits + row.sizeBits });
    tr.append(idx);
    for (const c of row.cells) {
      const td = cell("td", c.text, `rv-val ${fieldClass(c.kind)}`);
      if (c.problem !== undefined) td.title = c.problem.text;
      tr.append(td);
    }
    body.append(tr);
  }
  t.append(body);
  wrap.append(t);
  return { el: wrap, complete };
}

/** The first bytes of an opaque stretch as hex rows, and what kind of bytes
 *  they are. */
export function hexStrip(doc: Doc, offsetBits: number, sizeBits: number): { el: HTMLElement; complete: boolean } {
  const wrap = document.createElement("div");
  wrap.className = "rv-strip";
  const start = Math.floor(offsetBits / 8);
  const end = Math.ceil((offsetBits + sizeBits) / 8);
  const dump = dumpRows(doc, start, end, 4);
  wrap.append(dump.el);
  const sample = Math.min(end - start, 64 * 1024);
  const r = doc.read(start, sample);
  if (r.complete && sample > 0) {
    let zero = 0;
    let text = 0;
    for (const b of r.bytes) {
      if (b === 0) zero++;
      else if ((b >= 0x20 && b < 0x7f) || b === 9 || b === 10 || b === 13) text++;
    }
    const p = document.createElement("p");
    p.className = "rv-note";
    p.textContent = zero === sample ? RV.allZero(sample) : RV.classesOf(text, zero, sample - text - zero, sample);
    wrap.append(p);
  }
  return { el: wrap, complete: dump.complete && r.complete };
}

function moreLine(n: number, word: string): HTMLElement {
  const p = document.createElement("p");
  p.className = "rv-note";
  p.textContent = RV.moreRows(n, word);
  return p;
}

/** A button that shows a node in the listing, for the rows a table leaves out. */
export function listingButton(host: ReportHost, path: readonly number[]): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "rv-button";
  b.textContent = RV.showInListing;
  b.addEventListener("click", () => host.showInListing(path));
  return b;
}


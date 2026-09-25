// The three bodies a part can have: a field table for a short structure, a
// record table for a list of records, and a hex strip for bytes with no
// structure. Shared by the parts section and the content section.

import type { Doc, TableFact, TemplateNode } from "../doc.ts";
import { fieldClass } from "../fieldstyle.ts";
import { childWord, DECODED_REFUSED, DECODED_REFUSED_OTHER } from "../strings.ts";
import { formatOffset } from "../format.ts";
import { factValue } from "../tablebar.ts";
import type { TablePlan } from "../tableplan.ts";
import type { ReportData } from "./data.ts";
import { byteRef, dumpRows, pointAt } from "./refs.ts";
import type { ReportHost } from "./section.ts";
import { stripIndex } from "./partrules.ts";
import { bitsText, clip, counted, RV } from "./text.ts";

/** Bytes shown inline in a row. */
const INLINE_BYTES = 8;
/** Characters of a record's reading shown in its row; the rest is on hover. */
const READS_CHARS = 160;

/** The first bytes of a stretch as hex, `ff d8 ff e0 …`, or dots while they
 *  are still being read. Empty for a stretch that does not start on a byte. */
export function firstBytes(doc: Doc, offsetBits: number, sizeBits: number, space = 0): string {
  if (space !== 0 || offsetBits % 8 !== 0 || sizeBits < 8) return "";
  const n = Math.min(INLINE_BYTES, Math.floor(sizeBits / 8));
  const r = doc.read(offsetBits / 8, n);
  const hex = Array.from(r.bytes, (b) => (r.complete ? b.toString(16).padStart(2, "0") : "··")).join(" ");
  return sizeBits / 8 > n ? `${hex} …` : hex;
}

/** A cell, with the column's name on it for the narrow layout, where each
 *  row is two lines and the headings are gone (see `report.css`). */
function cell(tag: "td" | "th", text: string | Node, cls?: string, label?: string): HTMLTableCellElement {
  const c = document.createElement(tag);
  if (cls !== undefined) c.className = cls;
  if (label !== undefined) c.dataset.label = label;
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

/**
 * What a field holds, for the value column. A plain value or a structure's
 * one-line reading is itself. A structure with neither is never its bare count
 * of children: a compressed stream says what it is and what it unpacks to, or
 * why it was not unpacked, and anything else says how many of what it holds.
 */
export function valueText(doc: Doc, n: TemplateNode): string {
  if (n.refused !== null) return DECODED_REFUSED[n.refused] ?? DECODED_REFUSED_OTHER;
  if (n.line !== null) return n.line;
  if (!n.composite || n.kind !== "composite") return n.value;
  if (n.decoded) {
    const kids = doc.templateChildren(n.path, 0, Math.min(n.child_count, 8));
    const root = kids.status === "ok" ? kids.node.find((k) => k.space_root) : undefined;
    return root === undefined ? n.type : RV.unpacksTo(n.type, bitsText(root.size_bits));
  }
  return counted(n.child_count, childWord(n));
}

/** A value as the listing writes it, with the wrong-value glyph in front of it
 *  when the core says something is wrong, and the reason on hover. */
function valueCell(doc: Doc, n: TemplateNode): HTMLTableCellElement {
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
  td.append(valueText(doc, n));
  return td;
}

/** A field's address, as a reference, or its offset in a stream when it is
 *  not a place in the file. */
function atCell(n: TemplateNode): HTMLTableCellElement {
  if (n.space !== 0) return cell("td", `+${formatOffset(n.offset_bits).slice(1)}`, "rv-addr", RV.colAt);
  return cell("td", byteRef({ path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits }, n.name), "rv-addr", RV.colAt);
}

/** A structure's fields, one row each: name, value, address, size, bytes. */
export function fieldTable(doc: Doc, data: ReportData, fields: readonly TemplateNode[], more: number): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-fields rv-stack";
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
    tr.append(
      cell("td", name, "rv-cell-name"),
      valueCell(doc, n),
      atCell(n),
      cell("td", bitsText(n.size_bits), "rv-num", RV.colSize),
      cell("td", firstBytes(doc, n.offset_bits, n.size_bits, n.space), "rv-hex", RV.colBytes),
    );
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
  t.className = "rv-table rv-records rv-stack";
  // The records' own names, where the format gives them and they tell the
  // records apart: a ZIP entry's file name, an ELF section's.
  const names = rows.map((n) => stripIndex(n.name));
  const named = names.some((s) => s !== "") && new Set(names).size > 1;
  if (named) t.classList.add("rv-named");
  t.append(named ? head(RV.colIndex, RV.colName, RV.colAt, RV.colSize, RV.colReads, RV.colBytes) : head(RV.colIndex, RV.colAt, RV.colSize, RV.colReads, RV.colBytes));
  const body = document.createElement("tbody");
  for (const [i, n] of rows.entries()) {
    const tr = document.createElement("tr");
    const index = n.path[n.path.length - 1] ?? 0;
    if (named) {
      const code = document.createElement("code");
      code.textContent = names[i] ?? "";
      tr.append(cell("td", String(index), "rv-num", RV.colIndex), cell("td", code, "rv-name rv-cell-name"));
    }
    const reads = document.createElement("td");
    reads.className = "rv-reads";
    // Bytes read as bytes say nothing the next column does not, and a
    // structure with no one-line reading has only its count of fields.
    if (n.line !== null) {
      reads.append(clip(n.line, READS_CHARS));
      if (n.line.length > READS_CHARS) reads.title = n.line;
    }
    else if (n.kind !== "bytes" && n.kind !== "unread" && !n.composite) reads.append(n.value);
    if (n.problems_within[0] > 0) reads.classList.add("has-invalid");
    if (!named) tr.append(cell("td", String(index), "rv-num", RV.colIndex));
    tr.append(atCell(n), cell("td", bitsText(n.size_bits), "rv-num", RV.colSize), reads, cell("td", firstBytes(doc, n.offset_bits, n.size_bits, n.space), "rv-hex", RV.colBytes));
    body.append(tr);
  }
  t.append(body);
  wrap.append(t);
  if (more > 0) wrap.append(moreLine(more, word));
  return wrap;
}

/**
 * A table's facts as the report shows them. Each is named by its own field,
 * without the structures it was read through: `body.sample_rate` is
 * `sample_rate`, unless two facts would then have one name. A number gets its
 * thousands separators, as the table view writes it.
 */
export function factRows(facts: readonly TableFact[]): { readonly name: string; readonly value: string }[] {
  const short = facts.map((f) => f.label.replace(/^.*\./, ""));
  return facts.map((f, i) => {
    const name = short[i] ?? f.label;
    return { name: short.indexOf(name) === short.lastIndexOf(name) ? name : f.label, value: factValue(f.value) };
  });
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


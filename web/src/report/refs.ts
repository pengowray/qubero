// Byte references: every place in the report that names bytes of the file
// points at them.
//
// A reference is any element carrying the `data-rv-start` and `data-rv-end`
// attributes, bits of the file, and optionally `data-rv-path`, the field. The
// view answers every one of them the same way (see `ReportView`): a hover shows
// the bytes, a click puts the cursor there, and a double click goes to the hex
// view. So a section only marks what an element points at and never wires an
// event of its own.

import type { Doc } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { bitsText, RV } from "./text.ts";
import type { Target } from "./section.ts";

/** Mark an element as pointing at a place in the file. */
export function pointAt<E extends Element>(e: E, t: Target, label?: string): E {
  e.setAttribute("data-rv-start", String(t.startBit));
  e.setAttribute("data-rv-end", String(t.endBit));
  if (t.path !== undefined) e.setAttribute("data-rv-path", t.path.join("/"));
  if (label !== undefined) e.setAttribute("data-rv-label", label);
  return e;
}

/** What an element points at, if anything: the nearest marked ancestor. */
export function targetOf(e: Element | null): { el: Element; target: Target; label: string | null } | null {
  const hit = e?.closest("[data-rv-start]") ?? null;
  if (hit === null) return null;
  const start = Number(hit.getAttribute("data-rv-start"));
  const end = Number(hit.getAttribute("data-rv-end"));
  const p = hit.getAttribute("data-rv-path");
  const path = p === null ? undefined : p === "" ? [] : p.split("/").map(Number);
  return { el: hit, target: path === undefined ? { startBit: start, endBit: end } : { path, startBit: start, endBit: end }, label: hit.getAttribute("data-rv-label") };
}

/** A reference written as its address: `@0x1c`, pointing at the bytes. */
export function byteRef(t: Target, label?: string, text?: string): HTMLAnchorElement {
  const a = document.createElement("a");
  a.className = "rv-at";
  a.href = "#";
  a.textContent = text ?? formatOffset(t.startBit);
  return pointAt(a, t, label);
}

/** Rows of 16 bytes over a stretch, as the hex view writes them, for the
 *  hover. At most `rows` rows; the rest is said as a count. Bytes that have
 *  not been read yet are `··`, and reading them is asked for. */
export function dumpRows(doc: Doc, startByte: number, endByte: number, rows: number): { el: HTMLElement; complete: boolean } {
  const pre = document.createElement("div");
  pre.className = "rv-dump";
  const first = Math.floor(startByte / 16) * 16;
  const wanted = Math.min(Math.ceil(endByte / 16) * 16, first + rows * 16, doc.lengthBytes);
  const read = doc.read(first, Math.max(0, wanted - first));
  const digits = Math.max(4, Math.ceil(Math.log2(Math.max(2, endByte)) / 4));
  for (let r = first; r < wanted; r += 16) {
    const line = document.createElement("div");
    const off = document.createElement("span");
    off.className = "rv-dump-off";
    off.textContent = r.toString(16).padStart(digits, "0");
    line.append(off, " ");
    let text = "";
    for (let i = 0; i < 16; i++) {
      const at = r + i;
      const cell = document.createElement("span");
      if (at >= doc.lengthBytes) {
        cell.textContent = "  ";
        text += " ";
      } else {
        const v = read.bytes[at - first] ?? 0;
        cell.textContent = read.complete ? v.toString(16).padStart(2, "0") : "··";
        const inside = at >= startByte && at < endByte;
        cell.className = inside ? "rv-dump-in" : "rv-dump-out";
        if (inside && v === 0) cell.classList.add("rv-dump-zero");
        text += read.complete && v >= 0x20 && v < 0x7f ? String.fromCharCode(v) : "·";
      }
      line.append(cell, i === 7 ? "  " : " ");
    }
    const asc = document.createElement("span");
    asc.className = "rv-dump-asc";
    asc.textContent = text;
    line.append(" ", asc);
    pre.append(line);
  }
  const shown = wanted - first;
  const total = Math.ceil(endByte / 16) * 16 - first;
  if (total > shown) {
    const more = document.createElement("div");
    more.className = "rv-dump-more";
    more.textContent = `… ${bitsText((endByte - Math.max(startByte, wanted)) * 8)} more`;
    pre.append(more);
  }
  return { el: pre, complete: read.complete };
}

/** What the hover over a reference says: the name, where, how much, and the
 *  bytes themselves. */
export function refTip(doc: Doc, t: Target, label: string | null): HTMLElement {
  const box = document.createElement("div");
  if (label !== null && label !== "") {
    const b = document.createElement("b");
    b.textContent = label;
    box.append(b);
  }
  const size = bitsText(t.endBit - t.startBit);
  const where = document.createElement("div");
  where.className = "rv-tip-where";
  where.textContent =
    t.endBit - t.startBit > 8 ? RV.tipRange(formatOffset(t.startBit), formatOffset(t.endBit - 8), size) : RV.tipAt(formatOffset(t.startBit), size);
  box.append(where);
  const startByte = Math.floor(t.startBit / 8);
  const endByte = Math.max(startByte + 1, Math.ceil(t.endBit / 8));
  if (endByte > startByte && startByte < doc.lengthBytes) {
    const dump = dumpRows(doc, startByte, endByte, 4);
    box.append(dump.el);
    if (!dump.complete) box.append(Object.assign(document.createElement("div"), { className: "rv-tip-note", textContent: RV.tipLoading }));
  }
  box.append(Object.assign(document.createElement("div"), { className: "rv-tip-note", textContent: RV.tipClick }));
  return box;
}

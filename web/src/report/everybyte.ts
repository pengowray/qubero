// Section 12: every byte, part by part. The full ledger: each part in file
// order, and each field one level inside it, with where it starts, how big it
// is, and its share of the file. Long parts are cut to their first rows with a
// count; the listing has every field.

import type { TemplateNode } from "../doc.ts";
import { percentText } from "../format.ts";
import { lineColor, lineLabel } from "./bytes.ts";
import type { Ledger } from "./coredata.ts";
import { ledgerLines, runsOf } from "./ledger.ts";
import { ok, type PartsModel, type Unit } from "./model.ts";
import { byteRef } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { bitsText, clip, RV } from "./text.ts";

/** Rows in all, and rows inside one part, before the rest are counted. */
const ROWS = 400;
const PER_PART = 24;

export const everyByteSection: Section = {
  id: "everybyte",
  render(ctx: ReportCtx): Rendered {
    const model = ctx.data.parts();
    if (model === WAIT) return WAIT;
    if (model === null || model.groups.length === 0) return null;
    // The core's ledger, once it is complete; the parts' own reading where the
    // core has none to give.
    const core = ctx.data.core();
    if (core !== null && core.failed === null) {
      if (core.ledger === null || !core.ledger.done) return WAIT;
      return coreEveryByte(ctx, core.ledger, model);
    }
    const fileBits = model.fileBits;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-every";
    const h = document.createElement("h2");
    h.textContent = RV.everyHeading(Math.ceil(fileBits / 8));
    sec.append(h);
    const wrap = document.createElement("div");
    wrap.className = "rv-tablewrap";
    const t = document.createElement("table");
    t.className = "rv-table rv-everytable";
    const thead = document.createElement("thead");
    const hr = document.createElement("tr");
    for (const [text, cls] of [
      [RV.ledgerPart, ""],
      [RV.ledgerStart, "rv-num"],
      [RV.ledgerBytes, "rv-num"],
      [RV.ledgerShare, "rv-num"],
    ] as const) {
      const th = document.createElement("th");
      th.textContent = text;
      if (cls !== "") th.className = cls;
      hr.append(th);
    }
    thead.append(hr);
    t.append(thead);
    const body = document.createElement("tbody");
    let rows = 0;
    let cut = 0;
    const row = (label: Node | string, target: { path?: readonly number[]; startBit: number; endBit: number }, depth: number, cls = ""): void => {
      if (rows >= ROWS) {
        cut++;
        return;
      }
      rows++;
      const tr = document.createElement("tr");
      if (cls !== "") tr.className = cls;
      const name = document.createElement("td");
      name.style.paddingLeft = `${depth * 16}px`;
      name.append(label);
      const at = document.createElement("td");
      at.className = "rv-num";
      at.append(byteRef(target));
      const size = document.createElement("td");
      size.className = "rv-num";
      size.textContent = bitsText(target.endBit - target.startBit);
      const share = document.createElement("td");
      share.className = "rv-num";
      share.textContent = percentText(target.endBit - target.startBit, fileBits);
      tr.append(name, at, size, share);
      body.append(tr);
    };
    for (const g of model.groups) {
      for (const u of g.units) {
        const head = document.createElement("span");
        const sw = document.createElement("span");
        sw.className = "rv-swatch";
        sw.style.background = g.color;
        const label = document.createElement(u.named ? "code" : "span");
        label.textContent = u.label;
        head.append(sw, label);
        row(head, targetOf(u), 0, "rv-grouprow");
        const kids = fieldsOf(ctx, u);
        if (kids === WAIT) return WAIT;
        for (const k of runsOf(u, kids).slice(0, PER_PART)) {
          const code = document.createElement("code");
          code.textContent = k.count > 1 ? RV.runOf(k.name, k.count) : k.name;
          row(code, { path: k.path, startBit: k.startBit, endBit: k.endBit }, 1);
        }
        const n = u.node === null ? u.fields.length : u.node.child_count;
        if (n > PER_PART) {
          const more = document.createElement("span");
          more.className = "rv-muted";
          more.textContent = RV.moreRows(n - PER_PART, u.node?.unit ?? (u.node?.list === true ? "item" : "field"));
          if (rows < ROWS) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 4;
            td.style.paddingLeft = "16px";
            td.append(more);
            tr.append(td);
            body.append(tr);
          }
        }
      }
    }
    t.append(body);
    wrap.append(t);
    sec.append(wrap);
    const cap = document.createElement("p");
    cap.className = "rv-note";
    cap.textContent = cut > 0 ? `${RV.everyCaption} ${RV.everyCut(cut)}` : RV.everyCaption;
    sec.append(cap);
    return sec;
  },
};

/**
 * The core's ledger in full: each line of section 6, and under it what its
 * bytes are, by role, where that is more than one thing. Bytes no field
 * describes and padding are split into zero bytes, other bytes, and bytes the
 * core did not read to find out.
 */
function coreEveryByte(ctx: ReportCtx, ledger: Ledger, model: PartsModel): HTMLElement {
  const fileBits = ledger.file_bits;
  const lines = ledgerLines(ledger.rows);
  const sec = document.createElement("section");
  sec.className = "rv-section rv-every";
  const h = document.createElement("h2");
  h.textContent = RV.everyHeading(Math.ceil(fileBits / 8));
  sec.append(h);
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-everytable rv-stack";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const [text, cls] of [
    [RV.ledgerPart, ""],
    [RV.ledgerStart, "rv-num"],
    [RV.ledgerBytes, "rv-num"],
    [RV.ledgerShare, "rv-num"],
  ] as const) {
    const th = document.createElement("th");
    th.textContent = text;
    if (cls !== "") th.className = cls;
    hr.append(th);
  }
  thead.append(hr);
  t.append(thead);
  const body = document.createElement("tbody");
  let rows = 0;
  let cut = 0;
  const cell = (text: Node | string, cls: string, label: string): HTMLTableCellElement => {
    const td = document.createElement("td");
    td.className = cls;
    if (label !== "") td.dataset.label = label;
    td.append(text);
    return td;
  };
  lines.forEach((l, i) => {
    if (rows >= ROWS) {
      cut++;
      return;
    }
    rows++;
    const tr = document.createElement("tr");
    tr.className = "rv-grouprow";
    const name = document.createElement("span");
    const sw = document.createElement("span");
    sw.className = "rv-swatch";
    sw.style.background = lineColor(model, l, i);
    name.append(sw, ...lineLabel(ctx.doc, l, fileBits));
    const at = byteRef({ ...(l.firstPath.length > 0 ? { path: l.firstPath } : {}), startBit: l.firstOffsetBits, endBit: l.firstOffsetBits + 8 });
    tr.append(cell(name, "rv-cell-name", ""), cell(at, "rv-num", RV.ledgerStart), cell(bitsText(l.bits), "rv-num", RV.ledgerBytes), cell(percentText(l.bits, fileBits), "rv-num", RV.ledgerShare));
    body.append(tr);
    // A line with one role would repeat its own bytes under it, so it says
    // nothing more, except bytes no field describes and padding, which say
    // how many of them are zero.
    const only = l.roles[0];
    const oneRole = l.roles.length === 1 && only?.role !== "gap" && only?.role !== "padding";
    if (!oneRole) {
      for (const r of l.roles) {
        if (rows >= ROWS) {
          cut++;
          break;
        }
        rows++;
        const sub = document.createElement("tr");
        const label = document.createElement("span");
        label.className = "rv-sub";
        label.textContent = RV.role(r.role, r.aligns);
        const detail = r.role === "gap" || r.role === "padding" ? RV.zeroSplit(r.zeroBits, r.bits - r.zeroBits - r.unscannedBits, r.unscannedBits) : "";
        if (detail !== "") {
          const d = document.createElement("span");
          d.className = "rv-muted";
          d.textContent = `: ${detail}`;
          label.append(d);
        }
        sub.append(cell(label, "rv-cell-name", ""), cell("", "rv-num", ""), cell(bitsText(r.bits), "rv-num", RV.ledgerBytes), cell(percentText(r.bits, fileBits), "rv-num", RV.ledgerShare));
        body.append(sub);
      }
    }
    // The fields of a structure taken together are named, since the line's
    // label names only the structure.
    if (l.fields.length > 1 && rows < ROWS) {
      rows++;
      const sub = document.createElement("tr");
      const names = document.createElement("span");
      names.className = "rv-sub rv-muted";
      names.textContent = clip(l.fields.join(", "), FIELD_NAMES_CHARS);
      const td = cell(names, "rv-cell-name", "");
      td.colSpan = 4;
      sub.append(td);
      body.append(sub);
    }
  });
  t.append(body);
  wrap.append(t);
  sec.append(wrap);
  const cap = document.createElement("p");
  cap.className = "rv-note";
  cap.textContent = cut > 0 ? `${RV.coreEveryCaption} ${RV.everyCut(cut)}` : RV.coreEveryCaption;
  sec.append(cap);
  return sec;
}

/** Characters of the field names under a line of plain fields. */
const FIELD_NAMES_CHARS = 240;

function targetOf(u: Unit): { path?: readonly number[]; startBit: number; endBit: number } {
  return { ...(u.node !== null ? { path: u.path } : {}), startBit: u.offsetBits, endBit: u.offsetBits + u.sizeBits };
}

/** The fields one level inside a part, in file order. */
function fieldsOf(ctx: ReportCtx, u: Unit): readonly TemplateNode[] | typeof WAIT {
  if (u.node === null) return u.fields;
  if (!u.node.composite || u.node.child_count === 0) return [];
  const kids = ok(ctx.doc.templateChildren(u.path, 0, Math.min(u.node.child_count, PER_PART)));
  if (kids === WAIT) return WAIT;
  return (kids ?? []).filter((k) => !k.absent && k.size_bits > 0 && k.space === 0);
}

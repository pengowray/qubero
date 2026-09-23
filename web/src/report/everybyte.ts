// Section 12: every byte, part by part. The full ledger: each part in file
// order, and each field one level inside it, with where it starts, how big it
// is, and its share of the file. Long parts are cut to their first rows with a
// count; the listing has every field.

import type { TemplateNode } from "../doc.ts";
import { percentText } from "../format.ts";
import { ok, type Unit } from "./model.ts";
import { byteRef } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { bitsText, RV } from "./text.ts";

/** Rows in all, and rows inside one part, before the rest are counted. */
const ROWS = 400;
const PER_PART = 24;

export const everyByteSection: Section = {
  id: "everybyte",
  render(ctx: ReportCtx): Rendered {
    const model = ctx.data.parts();
    if (model === WAIT) return WAIT;
    if (model === null || model.groups.length === 0) return null;
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
        for (const k of kids.slice(0, PER_PART)) {
          const code = document.createElement("code");
          code.textContent = k.name;
          row(code, { path: k.path, startBit: k.offset_bits, endBit: k.offset_bits + k.size_bits }, 1);
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

// Section 8: the parts, in the order a reader needs them. Parts that other
// parts place or size come before the headers and directories that place
// them, and parts with nothing between them go largest first (see
// `readingOrder`), so a JPEG reads scan, tables, headers.
//
// Each part says where it is (its range and a position bar, so a reader who
// arrives from a link knows where they are), what the template says it is, and
// then shows one of three bodies: a record table for several records, a field
// table for a short structure, or a hex strip for bytes with no structure.

import type { TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { tablePlan } from "../tableplan.ts";
import { fieldTable, hexStrip, listingButton, planTable, recordTable } from "./bodies.ts";
import { ok, type Group, type PartsModel, type Unit } from "./model.ts";
import { positionBar } from "./posbar.ts";
import { byteRef, pointAt } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { bitsText, counted, RV, shareOfFile } from "./text.ts";

/** Parts drawn as sections of their own. The rest are counted. */
const SECTIONS_MAX = 40;
/** Rows of a record table, and fields of a field table, before the count. */
const ROWS = 16;
const FIELDS = 40;

export const partsSection: Section = {
  id: "parts",
  render(ctx: ReportCtx): Rendered {
    const model = ctx.data.parts();
    if (model === WAIT) return WAIT;
    if (model === null) return null;
    const groups = model.order.filter((g) => !g.gap);
    if (groups.length === 0) return null;
    const wrap = document.createElement("div");
    wrap.className = "rv-parts";
    for (const g of groups.slice(0, SECTIONS_MAX)) {
      const sec = partSection(ctx, model, g);
      if (sec === WAIT) return WAIT;
      wrap.append(sec);
    }
    if (groups.length > SECTIONS_MAX) {
      const p = document.createElement("p");
      p.className = "rv-note";
      p.textContent = RV.moreRows(groups.length - SECTIONS_MAX, "part");
      wrap.append(p);
    }
    return wrap;
  },
};

function heading(model: PartsModel, g: Group): HTMLElement {
  const h = document.createElement("h2");
  h.className = "rv-parthead";
  const sw = document.createElement("span");
  sw.className = "rv-swatch";
  sw.style.background = g.color;
  h.append(sw);
  const name = document.createElement(g.named ? "code" : "span");
  name.textContent = g.label;
  const size = bitsText(g.sizeBits);
  const share = shareOfFile(g.sizeBits, model.fileBits);
  const rest = g.units.length > 1 ? `: ${counted(g.units.length, g.unitWord)}, ${size}, ${share}` : `: ${size}, ${share}`;
  h.append(name, rest);
  const first = g.units[0] as Unit;
  pointAt(h, { ...(first.node !== null ? { path: first.path } : {}), startBit: first.offsetBits, endBit: first.offsetBits + first.sizeBits }, g.label);
  return h;
}

function partSection(ctx: ReportCtx, model: PartsModel, g: Group): HTMLElement | typeof WAIT {
  const sec = document.createElement("section");
  sec.className = "rv-section rv-part";
  const first = g.units[0] as Unit;
  const last = g.units[g.units.length - 1] as Unit;
  sec.dataset.rvPartStart = String(first.offsetBits);
  sec.dataset.rvPartEnd = String(last.offsetBits + last.sizeBits);
  sec.append(heading(model, g));
  const where = document.createElement("div");
  where.className = "rv-where";
  where.append(
    byteRef({ startBit: first.offsetBits, endBit: first.offsetBits + first.sizeBits }, g.label),
    RV.rangeTo,
    byteRef({ startBit: Math.max(first.offsetBits, last.offsetBits + last.sizeBits - 8), endBit: last.offsetBits + last.sizeBits }, g.label, formatOffset(Math.max(0, last.offsetBits + last.sizeBits - 8))),
  );
  where.append(positionBar(model.fileBits, g.units, g.color));
  sec.append(where);
  const doc = first.node?.doc;
  if (doc !== undefined) {
    const p = document.createElement("p");
    p.className = "rv-doc";
    p.textContent = doc;
    sec.append(p);
    if (first.node !== null) ctx.data.term(first.node.name, doc);
  }
  const body = bodyOf(ctx, g);
  if (body === WAIT) return WAIT;
  if (body !== null) sec.append(body);
  return sec;
}

/** The body that fits the part: see the file comment. */
function bodyOf(ctx: ReportCtx, g: Group): HTMLElement | null | typeof WAIT {
  const doc = ctx.doc;
  if (g.units.length > 1) {
    const nodes = g.units.map((u) => u.node).filter((n): n is TemplateNode => n !== null);
    const box = document.createElement("div");
    box.append(recordTable(doc, nodes.slice(0, ROWS), Math.max(0, nodes.length - ROWS), g.unitWord));
    return box;
  }
  const u = g.units[0] as Unit;
  if (u.node === null) return u.fields.length > 0 ? fieldTable(doc, ctx.data, u.fields.slice(0, FIELDS), Math.max(0, u.fields.length - FIELDS)) : null;
  const n = u.node;
  if (n.list) {
    const plan = tablePlan(doc, n);
    if (plan !== null && plan.columns.length > 0) {
      const t = planTable(plan, ROWS);
      if (!t.complete) return WAIT;
      const box = document.createElement("div");
      box.append(t.el);
      if (plan.count > ROWS) box.append(moreWithListing(ctx, n, plan.count - ROWS, plan.rowWord));
      return box;
    }
    const kids = ok(doc.templateChildren(n.path, 0, Math.min(n.child_count, ROWS)));
    if (kids === WAIT) return WAIT;
    const box = document.createElement("div");
    box.append(recordTable(doc, kids ?? [], 0, "item"));
    if (n.child_count > ROWS) box.append(moreWithListing(ctx, n, n.child_count - ROWS, n.unit ?? "item"));
    return box;
  }
  if (n.composite && n.child_count > 0) {
    const kids = ok(doc.templateChildren(n.path, 0, Math.min(n.child_count, FIELDS)));
    if (kids === WAIT) return WAIT;
    const shown = (kids ?? []).filter((k) => !k.absent && k.size_bits > 0);
    const opened = openBody(ctx, n, shown);
    if (opened === WAIT) return WAIT;
    return fieldTable(doc, ctx.data, opened, Math.max(0, n.child_count - FIELDS));
  }
  const strip = hexStrip(doc, n.offset_bits, n.size_bits);
  return strip.complete ? strip.el : WAIT;
}

/**
 * A structure of a few fields whose bulk is one structure inside it (a JPEG
 * segment is a marker and a `body`) shows that structure's fields in its
 * place, named `body.width`, so the table says what the segment holds rather
 * than that it has a body.
 */
function openBody(ctx: ReportCtx, n: TemplateNode, kids: readonly TemplateNode[]): readonly TemplateNode[] | typeof WAIT {
  if (kids.length > 4) return kids;
  const i = kids.findIndex((k) => k.composite && !k.list && !k.decoded && !k.inline && k.size_bits * 2 >= n.size_bits && k.child_count > 0 && k.child_count <= FIELDS);
  const body = kids[i];
  if (body === undefined) return kids;
  const inner = ok(ctx.doc.templateChildren(body.path, 0, body.child_count));
  if (inner === WAIT) return WAIT;
  if (inner === null) return kids;
  const named = inner.filter((k) => !k.absent && k.size_bits > 0).map((k) => ({ ...k, name: `${body.name}.${k.name}` }));
  return [...kids.slice(0, i), ...named, ...kids.slice(i + 1)];
}

function moreWithListing(ctx: ReportCtx, n: TemplateNode, more: number, word: string): HTMLElement {
  const p = document.createElement("p");
  p.className = "rv-note";
  p.append(`${RV.moreRows(more, word)} `, listingButton(ctx.host, n.path));
  return p;
}

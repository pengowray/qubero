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
import { entropyDecoded, scanEntropy } from "../jpegcards.ts";
import { childWord } from "../strings.ts";
import { tablePlan } from "../tableplan.ts";
import { fieldTable, hexStrip, listingButton, planTable, recordTable } from "./bodies.ts";
import { nameInPart } from "./bytes.ts";
import { formatCard } from "./cards.ts";
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
/** Parts of one group shown one by one, each in full. */
const EACH_MAX = 4;

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
  const listName = g.units[0]?.list?.name;
  if (g.kind !== null && listName !== undefined) name.append(...nameInPart(g.kind, listName));
  else name.textContent = g.label;
  const size = bitsText(g.sizeBits);
  const share = shareOfFile(g.sizeBits, model.fileBits);
  // A list says how many it holds as well as how big it is: `pages: 73 pages,
  // 37,376 bytes`.
  const only = g.units.length === 1 ? g.units[0]?.node : undefined;
  const count = g.units.length > 1 ? counted(g.units.length, g.unitWord) : only?.list === true ? counted(only.child_count, childWord(only)) : null;
  // The colon stays with the name, and the facts wrap as one piece, so a
  // narrow column never starts a line with the colon.
  const lead = document.createElement("span");
  lead.className = "rv-partname";
  lead.append(name, ":");
  const facts = document.createElement("span");
  facts.className = "rv-partfacts";
  facts.textContent = count !== null ? `${count}, ${size}, ${share}` : `${size}, ${share}`;
  h.append(lead, " ", facts);
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

/** The body that fits the part: see the file comment. A few parts of one kind
 *  (a MIDI file's three tracks) are each shown in full under a heading of
 *  their own; more than that are one row each of a record table. */
function bodyOf(ctx: ReportCtx, g: Group): HTMLElement | null | typeof WAIT {
  const doc = ctx.doc;
  if (g.units.length > 1 && g.units.length <= EACH_MAX) {
    const box = document.createElement("div");
    for (const u of g.units) {
      const h = document.createElement("h3");
      const name = document.createElement(u.named ? "code" : "span");
      name.textContent = u.label;
      h.append(name, `, `, byteRef({ ...(u.node !== null ? { path: u.path } : {}), startBit: u.offsetBits, endBit: u.offsetBits + u.sizeBits }, u.label), `, ${bitsText(u.sizeBits)}`);
      const body = unitBody(ctx, u);
      if (body === WAIT) return WAIT;
      box.append(h);
      if (body !== null) box.append(body);
    }
    return box;
  }
  if (g.units.length > 1) {
    const nodes = g.units.map((u) => u.node).filter((n): n is TemplateNode => n !== null);
    const box = document.createElement("div");
    box.append(recordTable(doc, nodes.slice(0, ROWS), Math.max(0, nodes.length - ROWS), g.unitWord));
    return box;
  }
  return unitBody(ctx, g.units[0] as Unit);
}

/** The body of one part. */
function unitBody(ctx: ReportCtx, u: Unit): HTMLElement | null | typeof WAIT {
  const doc = ctx.doc;
  if (u.node === null) return u.fields.length > 0 ? fieldTable(doc, ctx.data, u.fields.slice(0, FIELDS), Math.max(0, u.fields.length - FIELDS)) : null;
  const n = u.node;
  if (n.list) return listBody(ctx, n);
  if (n.composite && n.child_count > 0) {
    const kids = ok(doc.templateChildren(n.path, 0, Math.min(n.child_count, FIELDS)));
    if (kids === WAIT) return WAIT;
    const shown = (kids ?? []).filter((k) => !k.absent && k.size_bits > 0);
    const opened = openBody(ctx, n, shown);
    if (opened === WAIT) return WAIT;
    const box = document.createElement("div");
    // The listing's card for the node, where it draws one, above its fields.
    const card = formatCard(doc, n);
    if (card !== null) box.append(card, ...decodedBelow(ctx, n));
    box.append(fieldTable(doc, ctx.data, opened.fields, Math.max(0, n.child_count - FIELDS)));
    if (opened.list !== null) {
      const list = listBody(ctx, opened.list);
      if (list === WAIT) return WAIT;
      const cap = document.createElement("p");
      cap.className = "rv-sublead";
      const code = document.createElement("code");
      code.textContent = opened.list.name;
      cap.append(code, `: ${counted(opened.list.child_count, childWord(opened.list))}`);
      box.append(cap, list);
    }
    return box;
  }
  const strip = hexStrip(doc, n.offset_bits, n.size_bits);
  return strip.complete ? strip.el : WAIT;
}

/** A list: the first rows of its table when it reads as one, or else one row
 *  a record. */
function listBody(ctx: ReportCtx, n: TemplateNode): HTMLElement | typeof WAIT {
  const doc = ctx.doc;
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
  box.append(recordTable(doc, kids ?? [], 0, childWord(n)));
  if (n.child_count > ROWS) box.append(moreWithListing(ctx, n, n.child_count - ROWS, childWord(n)));
  return box;
}

/**
 * A structure of a few fields whose bulk is one structure inside it (a JPEG
 * segment is a marker and a `body`) shows that structure's fields in its
 * place, named `body.width`, so the table says what the segment holds rather
 * than that it has a body. Where the bulk is a list (a MIDI track's events),
 * the list is lifted out to be shown as its records under the fields.
 */
function openBody(
  ctx: ReportCtx,
  n: TemplateNode,
  kids: readonly TemplateNode[],
): { readonly fields: readonly TemplateNode[]; readonly list: TemplateNode | null } | typeof WAIT {
  if (kids.length > 4) return { fields: kids, list: null };
  // A structure the listing draws on one row (`inline`) is opened here all the
  // same: the report is about how the bytes are laid out, and the one-row
  // reading is still there in the ledger and the tables.
  const bulk = (k: TemplateNode): boolean => k.composite && !k.decoded && k.size_bits * 2 >= n.size_bits && k.child_count > 0;
  const i = kids.findIndex(bulk);
  const body = kids[i];
  if (body === undefined) return { fields: kids, list: null };
  if (body.list) return { fields: kids.filter((_, j) => j !== i), list: body };
  if (body.child_count > FIELDS) return { fields: kids, list: null };
  const inner = ok(ctx.doc.templateChildren(body.path, 0, body.child_count));
  if (inner === WAIT) return WAIT;
  if (inner === null) return { fields: kids, list: null };
  const real = inner.filter((k) => !k.absent && k.size_bits > 0);
  // The body's own bulk may be a list in turn: a chunk's body holding the
  // samples, a track's body holding its events.
  const innerList = real.find((k) => k.list && k.size_bits * 2 >= n.size_bits && k.child_count > 0) ?? null;
  const named = real.filter((k) => k !== innerList).map((k) => ({ ...k, name: `${body.name}.${k.name}` }));
  return { fields: [...kids.slice(0, i), ...named, ...kids.slice(i + 1)], list: innerList };
}

/**
 * For a JPEG scan the report decodes further down, a line that says so and
 * links to that section, so the card's "decoded" has somewhere to go. The
 * line shows once the section is drawn, and never where it is not: a scan
 * past the ones the section draws, or one whose trace did not come.
 */
function decodedBelow(ctx: ReportCtx, segment: TemplateNode): HTMLElement[] {
  const entropy = scanEntropy(ctx.doc, segment);
  if (entropy === null || !entropyDecoded(entropy)) return [];
  const p = document.createElement("p");
  p.className = "rv-note";
  p.hidden = true;
  const key = entropy.path.join("/");
  ctx.live(() => {
    if (!ctx.data.drawn("jpeg")) return false;
    const target = p.closest(".rv-page")?.querySelector<HTMLElement>(`[data-rv-scan="${key}"]`) ?? null;
    if (target === null) return true;
    const a = document.createElement("a");
    a.href = "#";
    a.className = "rv-jump";
    a.textContent = target.textContent;
    a.addEventListener("click", (e) => {
      e.preventDefault();
      target.scrollIntoView({ behavior: "smooth", block: "start" });
    });
    const [before, after] = RV.scanDecodedBelow("\u0000").split("\u0000");
    p.replaceChildren(before ?? "", a, after ?? "");
    p.hidden = false;
    return true;
  });
  return [p];
}

function moreWithListing(ctx: ReportCtx, n: TemplateNode, more: number, word: string): HTMLElement {
  const p = document.createElement("p");
  p.className = "rv-note";
  p.append(`${RV.moreRows(more, word)} `, listingButton(ctx.host, n.path));
  return p;
}

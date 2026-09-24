// Section 6: where the bytes go. The whole-file map with semantic zoom, then
// the ledger: a stacked bar of the parts and a table with each part's share of
// the file. Hovering a ledger row lights its bytes in the map.
//
// For a file with no template there are no parts, so the ledger is the byte
// scan's own: how much of the file is zero, text, and anything else, counted
// as the scan gets on.

import type { Doc } from "../doc.ts";
import { formatOffset, percentText } from "../format.ts";
import { byteClassColor, sectionColor, UNMAPPED_COLOR } from "../fieldstyle.ts";
import { REPORT } from "../strings.ts";
import type { Ledger } from "./coredata.ts";
import { isGapLine, ledgerLines, type LedgerLine } from "./ledger.ts";
import { groupAt, type Group, type PartsModel } from "./model.ts";
import { runPosition } from "./partrules.ts";
import { byteRef, pointAt } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { bitsText, bytesText, clip, counted, RV } from "./text.ts";
import { stripLegend, ZoomMap, type MapPart } from "./zoommap.ts";

/** Ledger rows before the rest are counted. */
const LEDGER_ROWS = 60;
/** Characters of a part's reading shown in its row; the rest is on hover. */
const WHAT_CHARS = 120;

export const bytesSection: Section = {
  id: "bytes",
  render(ctx: ReportCtx): Rendered {
    const model = ctx.data.parts();
    if (model === WAIT) return WAIT;
    const size = ctx.doc.lengthBytes;
    if (size === 0) return null;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-bytes";
    const h = document.createElement("h2");
    h.textContent = RV.bytesHeading(size);
    sec.append(h);
    const mapParts: MapPart[] = [];
    for (const g of model?.groups ?? []) {
      for (const u of g.units) {
        mapParts.push({
          startBit: u.offsetBits,
          endBit: u.offsetBits + u.sizeBits,
          label: g.units.length > 1 ? g.label : u.label,
          color: g.color,
          ...(u.node !== null ? { path: u.path } : {}),
        });
      }
    }
    const map = new ZoomMap(ctx, mapParts);
    const fig = document.createElement("figure");
    fig.className = "rv-figure";
    const hint = document.createElement("div");
    hint.className = "rv-hint";
    hint.textContent = RV.mapHint;
    const cap = document.createElement("figcaption");
    const lead = document.createElement("b");
    lead.textContent = RV.mapCaptionLead;
    cap.append(lead, ` ${RV.mapCaption}`);
    fig.append(map.el, stripLegend(), hint, cap);
    sec.append(fig);
    ctx.live(() => map.update());
    if (model === null) {
      sec.append(classLedger(ctx));
      return sec;
    }
    // The core's ledger once it has answered, drawn again as the walk gets on;
    // the parts' own ledger where the core has no ledger to give.
    const box = document.createElement("div");
    sec.append(box);
    let drawn: Ledger | null = null;
    const draw = (): boolean => {
      const core = ctx.data.core();
      if (core === null || core.failed !== null) {
        box.replaceChildren(ledger(model, map));
        return true;
      }
      const l = core.ledger;
      if (l === null) return false;
      if (l !== drawn) {
        drawn = l;
        box.replaceChildren(coreLedger(ctx, l, model, map));
      }
      return l.done;
    };
    if (!draw()) ctx.live(draw);
    return sec;
  },
};

/** A colour for each ledger line: the colour its first bytes have on the
 *  map, so a row and the part of the map it is about look alike. */
export function lineColor(model: PartsModel, l: LedgerLine, i: number): string {
  if (isGapLine(l)) return UNMAPPED_COLOR;
  return groupAt(model, l.firstOffsetBits)?.color ?? sectionColor(i);
}

/** What a ledger line is called: its group within its part, the fields of a
 *  structure taken together, or bytes no field describes. A group of more
 *  than one element says how many, where the parts found them: four dht
 *  segments are `dht, huffman tables × 4`. */
export function lineLabel(doc: Doc, l: LedgerLine, fileBits: number, model: PartsModel | null = null): (Node | string)[] {
  if (isGapLine(l)) return [RV.ledgerGapLabel];
  if (l.key === "padding") return [RV.ledgerPaddingLabel];
  const code = (s: string): HTMLElement => {
    const c = document.createElement("code");
    c.textContent = s;
    return c;
  };
  if (l.part === null) {
    const parent = l.parentPath;
    if (parent === null || parent.length === 0) {
      return [REPORT.unnamedPart(runPosition({ offsetBits: l.firstOffsetBits, sizeBits: l.bits }, fileBits))];
    }
    const n = doc.templateNode(parent);
    const name = n.status === "ok" ? n.node.name : "";
    return [RV.fieldsOf, code(name)];
  }
  if (l.group === "") return [code(l.part)];
  const inPart = document.createElement("span");
  inPart.className = "rv-muted";
  inPart.append(` ${RV.inPart} `, code(l.part));
  const g = model === null ? null : groupAt(model, l.firstOffsetBits);
  const n = g !== null && g.label === l.group ? g.units.length : 1;
  return [n > 1 ? RV.runOf(l.group, n) : l.group, inPart];
}

/** Light a line's first field on the map, and zoom there on a click. */
function linkRow(ctx: ReportCtx, tr: HTMLTableRowElement, l: LedgerLine, map: ZoomMap): void {
  const first = (): { offsetBits: number; sizeBits: number } => {
    const n = l.firstPath.length > 0 ? ctx.doc.templateNode(l.firstPath) : null;
    const size = n !== null && n.status === "ok" ? n.node.size_bits : 8;
    return { offsetBits: l.firstOffsetBits, sizeBits: Math.max(8, size) };
  };
  tr.addEventListener("pointerenter", () => map.highlight([first()]));
  tr.addEventListener("pointerleave", () => map.highlight([]));
  tr.addEventListener("click", (e) => {
    if ((e.target as Element).closest("[data-rv-start], a, button") !== null) return;
    const f = first();
    const from = f.offsetBits / 8;
    const pad = Math.max(16, (f.sizeBits / 8) * 0.5);
    map.show(Math.max(0, from - pad), from + f.sizeBits / 8 + pad);
    map.el.scrollIntoView({ behavior: "smooth", block: "center" });
  });
  tr.title = RV.ledgerRowTitle;
  tr.classList.add("rv-zoomrow");
}

/** The core's ledger as a stacked bar and a table, one row per part and
 *  group, in file order. */
function coreLedger(ctx: ReportCtx, ledger: Ledger, model: PartsModel, map: ZoomMap): HTMLElement {
  const lines = ledgerLines(ledger.rows);
  const fileBits = ledger.file_bits;
  const box = document.createElement("figure");
  box.className = "rv-figure rv-ledger";
  const bar = document.createElement("div");
  bar.className = "rv-ledgerbar";
  bar.setAttribute("aria-hidden", "true");
  lines.forEach((l, i) => {
    const seg = document.createElement("span");
    seg.style.flexGrow = String(l.bits);
    seg.style.background = lineColor(model, l, i);
    if (isGapLine(l)) seg.className = "is-gap";
    bar.append(seg);
  });
  if (ledger.reached_bits < fileBits) {
    const rest = document.createElement("span");
    rest.style.flexGrow = String(fileBits - ledger.reached_bits);
    rest.className = "is-unread";
    bar.append(rest);
  }
  box.append(bar);
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-ledgertable rv-stack";
  t.append(tableHead([RV.ledgerPart, ""], [RV.ledgerStart, "rv-num"], [RV.ledgerBytes, "rv-num"], [RV.ledgerShare, ""]));
  const body = document.createElement("tbody");
  const largest = Math.max(1, ...lines.map((l) => l.bits));
  lines.slice(0, LEDGER_ROWS).forEach((l, i) => {
    const tr = document.createElement("tr");
    if (isGapLine(l)) tr.className = "is-gap";
    const name = document.createElement("td");
    name.className = "rv-cell-name";
    const sw = document.createElement("span");
    sw.className = "rv-swatch";
    sw.style.background = lineColor(model, l, i);
    name.append(sw, ...lineLabel(ctx.doc, l, fileBits, model));
    const at = document.createElement("td");
    at.className = "rv-num";
    at.dataset.label = RV.ledgerStart;
    at.append(byteRef({ ...(l.firstPath.length > 0 ? { path: l.firstPath } : {}), startBit: l.firstOffsetBits, endBit: l.firstOffsetBits + 8 }));
    const bytes = document.createElement("td");
    bytes.className = "rv-num";
    bytes.dataset.label = RV.ledgerBytes;
    bytes.textContent = bitsText(l.bits).replace(/ bytes?$/, "");
    const share = document.createElement("td");
    share.className = "rv-share";
    share.dataset.label = RV.ledgerShare;
    const b = document.createElement("span");
    b.className = "rv-sharebar";
    b.style.width = `${Math.max(1, (l.bits / largest) * 80)}px`;
    b.style.background = lineColor(model, l, i);
    share.append(b, ` ${percentText(l.bits, fileBits)}`);
    tr.append(name, at, bytes, share);
    linkRow(ctx, tr, l, map);
    body.append(tr);
  });
  t.append(body);
  wrap.append(t);
  box.append(wrap);
  const cap = document.createElement("figcaption");
  const top = [...lines].sort((a, b) => b.bits - a.bits)[0];
  if (top !== undefined) {
    const lead = document.createElement("b");
    lead.append(RV.largestLead, ...lineLabel(ctx.doc, top, fileBits, model), ` (${percentText(top.bits, fileBits)} ${RV.ofTheFile}).`);
    cap.append(lead, " ");
  }
  cap.append(RV.coreLedgerCaption);
  if (!ledger.done) cap.append(` ${RV.ledgerSoFar(formatOffset(ledger.reached_bits))}`);
  if (lines.length > LEDGER_ROWS) cap.append(` ${RV.ledgerUnlisted(lines.length - LEDGER_ROWS)}`);
  box.append(cap);
  return box;
}

function tableHead(...cols: readonly (readonly [string, string])[]): HTMLTableSectionElement {
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const [text, cls] of cols) {
    const th = document.createElement("th");
    th.textContent = text;
    if (cls !== "") th.className = cls;
    hr.append(th);
  }
  thead.append(hr);
  return thead;
}

/** The parts as a stacked bar and a table, in file order. */
function ledger(model: PartsModel, map: ZoomMap): HTMLElement {
  const box = document.createElement("figure");
  box.className = "rv-figure rv-ledger";
  const bar = document.createElement("div");
  bar.className = "rv-ledgerbar";
  bar.setAttribute("aria-hidden", "true");
  for (const g of model.groups) {
    const seg = document.createElement("span");
    seg.style.flexGrow = String(g.sizeBits);
    seg.style.background = g.color;
    seg.className = g.gap ? "is-gap" : "";
    pointAt(seg, target(g), g.label);
    bar.append(seg);
  }
  box.append(bar);
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-ledgertable";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const [text, cls] of [
    [RV.ledgerPart, ""],
    [RV.ledgerStart, "rv-num"],
    [RV.ledgerBytes, "rv-num"],
    [RV.ledgerShare, ""],
    [RV.ledgerWhat, ""],
  ] as const) {
    const th = document.createElement("th");
    th.textContent = text;
    if (cls !== "") th.className = cls;
    hr.append(th);
  }
  thead.append(hr);
  t.append(thead);
  const body = document.createElement("tbody");
  const largest = Math.max(1, ...model.groups.map((g) => g.sizeBits));
  for (const g of model.groups.slice(0, LEDGER_ROWS)) {
    const tr = document.createElement("tr");
    if (g.gap) tr.className = "is-gap";
    const name = document.createElement("td");
    const sw = document.createElement("span");
    sw.className = "rv-swatch";
    sw.style.background = g.color;
    const label = document.createElement(g.named ? "code" : "span");
    label.textContent = g.units.length > 1 ? `${g.label} × ${g.units.length.toLocaleString()}` : g.label;
    name.append(sw, label);
    const at = document.createElement("td");
    at.className = "rv-num";
    at.append(byteRef(target(g), g.label));
    const bytes = document.createElement("td");
    bytes.className = "rv-num";
    bytes.textContent = bitsText(g.sizeBits).replace(/ bytes?$/, "");
    const share = document.createElement("td");
    share.className = "rv-share";
    const b = document.createElement("span");
    b.className = "rv-sharebar";
    b.style.width = `${Math.max(1, (g.sizeBits / largest) * 80)}px`;
    b.style.background = g.color;
    share.append(b, ` ${percentText(g.sizeBits, model.fileBits)}`);
    const what = document.createElement("td");
    what.className = "rv-what";
    const says = whatItIs(g);
    what.textContent = clip(says, WHAT_CHARS);
    if (says.length > WHAT_CHARS) what.title = says;
    tr.append(name, at, bytes, share, what);
    tr.addEventListener("pointerenter", () => map.highlight(g.units));
    tr.addEventListener("pointerleave", () => map.highlight([]));
    // A click anywhere on the row but its address zooms the map to the part:
    // the ranges the ledger names are the presets the map offers.
    tr.addEventListener("click", (e) => {
      if ((e.target as Element).closest("[data-rv-start]") !== null) return;
      const first = g.units[0];
      const last = g.units[g.units.length - 1];
      if (first === undefined || last === undefined) return;
      const from = first.offsetBits / 8;
      const to = (last.offsetBits + last.sizeBits) / 8;
      const pad = Math.max(4, (to - from) * 0.15);
      map.show(Math.max(0, from - pad), to + pad);
      map.el.scrollIntoView({ behavior: "smooth", block: "center" });
    });
    tr.title = RV.ledgerRowTitle;
    tr.classList.add("rv-zoomrow");
    body.append(tr);
  }
  t.append(body);
  wrap.append(t);
  box.append(wrap);
  const cap = document.createElement("figcaption");
  const top = [...model.groups].sort((a, b) => b.sizeBits - a.sizeBits)[0];
  if (top !== undefined) {
    const lead = document.createElement("b");
    lead.textContent = RV.ledgerCaptionLead(top.label, percentText(top.sizeBits, model.fileBits));
    cap.append(lead, " ");
  }
  cap.append(RV.ledgerCaption);
  if (model.groups.length > LEDGER_ROWS) cap.append(` ${RV.ledgerUnlisted(model.groups.length - LEDGER_ROWS)}`);
  else if (model.unlisted > 0) cap.append(` ${RV.ledgerUnlisted(model.unlisted)}`);
  box.append(cap);
  return box;
}

function target(g: Group): { path?: readonly number[]; startBit: number; endBit: number } {
  const u = g.units[0];
  if (u === undefined) return { startBit: g.offsetBits, endBit: g.offsetBits };
  return { ...(u.node !== null ? { path: u.path } : {}), startBit: u.offsetBits, endBit: u.offsetBits + u.sizeBits };
}

/** One line on what a part is: the template's description, or what its fields
 *  read as, or for bytes no field covers, that. */
function whatItIs(g: Group): string {
  if (g.gap) return g.unexamined ? RV.unexaminedBody : RV.gapBody;
  const u = g.units[0];
  const n = u?.node ?? null;
  if (n?.doc !== undefined) return firstSentence(n.doc);
  if (g.units.length > 1) return counted(g.units.length, g.unitWord);
  if (n !== null) return n.line ?? (n.list ? counted(n.child_count, n.unit ?? "item") : "");
  const fields = u?.fields ?? [];
  return fields.map((f) => f.name).slice(0, 6).join(", ") + (fields.length > 6 ? ", …" : "");
}

function firstSentence(text: string): string {
  const m = /^.*?[.!?](\s|$)/.exec(text);
  return (m?.[0] ?? text).trim();
}

/** For a file with no parts: the byte scan's counts, as the ledger. */
function classLedger(ctx: ReportCtx): HTMLElement {
  const box = document.createElement("figure");
  box.className = "rv-figure rv-ledger";
  const size = ctx.doc.lengthBytes;
  const draw = (): boolean => {
    const scan = ctx.data.scan();
    if (scan === null) return false;
    const read = Math.min(size, scan.read_bytes);
    const other = Math.max(0, read - scan.zero_bytes - scan.text_bytes);
    const rows: [string, number, string][] = [
      [RV.classZero, scan.zero_bytes, byteClassColor(0)],
      [RV.classText, scan.text_bytes, byteClassColor(2)],
      [RV.classOther, other, byteClassColor(3)],
    ];
    if (read < size) rows.push([RV.classUnread, size - read, "transparent"]);
    const t = document.createElement("table");
    t.className = "rv-table rv-ledgertable";
    const body = document.createElement("tbody");
    for (const [label, n, color] of rows) {
      const tr = document.createElement("tr");
      const name = document.createElement("td");
      const sw = document.createElement("span");
      sw.className = color === "transparent" ? "rv-swatch is-unread" : "rv-swatch";
      sw.style.background = color;
      name.append(sw, label);
      const bytes = document.createElement("td");
      bytes.className = "rv-num";
      bytes.textContent = bytesText(n);
      const share = document.createElement("td");
      share.textContent = percentText(n, size);
      tr.append(name, bytes, share);
      body.append(tr);
    }
    t.append(body);
    const cap = document.createElement("figcaption");
    cap.textContent = RV.classLedgerCaption(formatOffset(read * 8), scan.done);
    box.replaceChildren(t, cap);
    return scan.done;
  };
  draw();
  ctx.live(draw);
  return box;
}

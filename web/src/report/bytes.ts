// Section 6: where the bytes go. The whole-file map with semantic zoom, then
// the ledger: a stacked bar of the parts and a table with each part's share of
// the file. Hovering a ledger row lights its bytes in the map.
//
// For a file with no template there are no parts, so the ledger is the byte
// scan's own: how much of the file is zero, text, and anything else, counted
// as the scan gets on.

import { formatOffset, percentText } from "../format.ts";
import type { Group, PartsModel } from "./model.ts";
import { byteRef, pointAt } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { bitsText, bytesText, counted, RV } from "./text.ts";
import { stripLegend, ZoomMap, type MapPart } from "./zoommap.ts";
import { byteClassColor } from "../fieldstyle.ts";

/** Ledger rows before the rest are counted. */
const LEDGER_ROWS = 60;

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
    if (model !== null) sec.append(ledger(model, map));
    else sec.append(classLedger(ctx));
    return sec;
  },
};

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
    what.textContent = whatItIs(g);
    tr.append(name, at, bytes, share, what);
    tr.addEventListener("pointerenter", () => map.highlight(g.units));
    tr.addEventListener("pointerleave", () => map.highlight([]));
    body.append(tr);
  }
  t.append(body);
  wrap.append(t);
  box.append(wrap);
  const cap = document.createElement("figcaption");
  const top = [...model.groups].sort((a, b) => b.sizeBits - a.sizeBits)[0];
  if (top !== undefined) {
    const lead = document.createElement("b");
    lead.textContent = RV.ledgerCaptionLead(top.gap ? RV.gapName : top.label, percentText(top.sizeBits, model.fileBits));
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
  if (g.gap) return RV.gapBody;
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

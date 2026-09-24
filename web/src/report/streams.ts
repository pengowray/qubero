// Section 9: the compressed streams, and what they unpack to.
//
// Each stream gets its stages drawn to scale: the run in the file and the
// bytes it unpacks to. A stream whose codec keeps a trace (deflate) also gets
// the ribbon from its first codes to the bytes they write, from
// `Space::map_out`: a literal is one code for one byte, a copy one code for a
// run of earlier bytes, so its band widens toward them.
//
// Unpacking is a whole decode of the stream, so the report does it on its own
// only for streams up to `OPEN_BYTES`, and only for the first `OPEN_MAX` of
// them. The rest say so and offer to open the stream as a tab, which is the
// same unpacking the listing's button does.

import type { Doc, MapStep, TemplateNode } from "../doc.ts";
import { byteText, formatAddress, formatBytes, formatOffset } from "../format.ts";
import { ok } from "./model.ts";
import { byteRef, pointAt } from "./refs.ts";
import { ribbon, type RibbonData } from "./ribbon.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { stripIndex } from "./partrules.ts";
import { hideTip, tipAt } from "./tip.ts";
import { bytesText, RV } from "./text.ts";
import { walkTemplate } from "./walk.ts";

/** Streams unpacked by the report on its own, the largest it unpacks, and
 *  how much it unpacks in all. */
const OPEN_MAX = 32;
const OPEN_BYTES = 4 * 1024 * 1024;
const OPEN_TOTAL = 16 * 1024 * 1024;
/** Streams listed in the table. */
const ROWS_MAX = 200;
/** Codes drawn in a ribbon, codes read to choose which, and literals shown
 *  before the first copy. */
const STEPS = 48;
const SCAN_STEPS = 400;
const LEAD_STEPS = 12;

type Opened = { readonly node: TemplateNode; readonly label: string; readonly space: Doc | null };

export const streamsSection: Section = {
  id: "streams",
  render(ctx: ReportCtx): Rendered {
    if (ctx.doc.template === null) return null;
    const walk = ctx.data.memo("walk", () => walkTemplate(ctx.doc));
    if (walk === WAIT) return WAIT;
    const streams = walk.streams.filter((s) => s.refused === null);
    if (streams.length === 0 && walk.joins.length === 0) return null;
    const opened: Opened[] = [];
    let unpackedTotal = 0;
    let openedBytes = 0;
    let allOpened = true;
    for (const node of streams.slice(0, ROWS_MAX)) {
      const label = streamLabel(ctx.doc, node);
      if (label === WAIT) return WAIT;
      let space: Doc | null = null;
      const bytes = node.size_bits / 8;
      if (opened.filter((o) => o.space !== null).length < OPEN_MAX && bytes <= OPEN_BYTES && openedBytes + bytes <= OPEN_TOTAL) {
        const r = ctx.doc.openSpace(node.path);
        space = r !== null && !("refused" in r) ? r : null;
        if (space !== null) openedBytes += bytes;
      }
      if (space === null) allOpened = false;
      else unpackedTotal += space.lengthBytes;
      opened.push({ node, label, space });
    }
    if (streams.length > ROWS_MAX) allOpened = false;
    const packed = streams.reduce((n, s) => n + s.size_bits / 8, 0);
    const sec = document.createElement("section");
    sec.className = "rv-section rv-streams";
    const h = document.createElement("h2");
    h.textContent = streams.length === 0 ? RV.joinsHeading(walk.joins.length) : RV.streamsHeading(streams.length, packed, allOpened ? unpackedTotal : null);
    sec.append(h);
    if (streams.length === 0) {
      sec.append(joinList(ctx, walk.joins));
      return sec;
    }
    // One table of every stream, then the figures for one of them: the
    // largest to start with, and whichever row is clicked after that.
    const figure = document.createElement("div");
    const pick = (o: Opened, row: HTMLTableRowElement | null): void => {
      for (const r of sec.querySelectorAll(".rv-streamrow.is-on")) r.classList.remove("is-on");
      row?.classList.add("is-on");
      figure.replaceChildren(streamBlock(ctx, o));
    };
    const table = streamTable(opened, pick);
    sec.append(table.el);
    if (streams.length > ROWS_MAX) {
      const p = document.createElement("p");
      p.className = "rv-note";
      p.textContent = RV.moreStreams(streams.length - ROWS_MAX);
      sec.append(p);
    }
    if (walk.joins.length > 0) sec.append(joinList(ctx, walk.joins));
    sec.append(figure);
    const largest = [...opened].filter((o) => o.space !== null).sort((a, b) => b.node.size_bits - a.node.size_bits)[0] ?? opened[0];
    if (largest !== undefined) pick(largest, table.rows.get(largest) ?? null);
    return sec;
  },
};

/** Every stream in one table: its name, how many bytes it takes in the file
 *  and unpacks to, and the ratio as a bar. A click on a row draws that
 *  stream's figures under the table. */
function streamTable(opened: readonly Opened[], pick: (o: Opened, row: HTMLTableRowElement) => void): { el: HTMLElement; rows: Map<Opened, HTMLTableRowElement> } {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-streamtable rv-stack";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  for (const [text, cls] of [
    [RV.streamName, ""],
    [RV.streamPacked, "rv-num"],
    [RV.streamUnpacked, "rv-num"],
    [RV.streamRatio, ""],
  ] as const) {
    const th = document.createElement("th");
    th.textContent = text;
    if (cls !== "") th.className = cls;
    hr.append(th);
  }
  thead.append(hr);
  t.append(thead);
  const body = document.createElement("tbody");
  const rows = new Map<Opened, HTMLTableRowElement>();
  const ratios = opened.map((o) => (o.space === null || o.node.size_bits === 0 ? 0 : (o.space.lengthBytes * 8) / o.node.size_bits));
  const most = Math.max(1, ...ratios);
  opened.forEach((o, i) => {
    const tr = document.createElement("tr");
    tr.className = "rv-streamrow";
    tr.title = RV.streamRowTitle;
    const name = document.createElement("td");
    name.className = "rv-cell-name";
    const code = document.createElement("code");
    code.textContent = o.label;
    name.append(code, " ", addressOf(o.node, o.label));
    const cell = (text: Node | string, cls: string, label: string): HTMLTableCellElement => {
      const td = document.createElement("td");
      td.className = cls;
      td.dataset.label = label;
      td.append(text);
      return td;
    };
    const ratio = ratios[i] ?? 0;
    const bar = document.createElement("span");
    bar.className = "rv-ratiobar";
    bar.style.width = `${Math.max(1, (ratio / most) * 60)}px`;
    const ratioCell = cell(o.space === null ? RV.streamNotUnpacked : "", "rv-ratio", RV.streamRatio);
    if (o.space !== null) ratioCell.append(bar, ` ${RV.ratio(ratio)}`);
    tr.append(
      name,
      cell(bytesText(o.node.size_bits / 8), "rv-num", RV.streamPacked),
      cell(o.space === null ? "" : bytesText(o.space.lengthBytes), "rv-num", RV.streamUnpacked),
      ratioCell,
    );
    tr.addEventListener("click", (e) => {
      if ((e.target as Element).closest("[data-rv-start]") !== null) return;
      pick(o, tr);
    });
    rows.set(o, tr);
    body.append(tr);
  });
  t.append(body);
  wrap.append(t);
  return { el: wrap, rows };
}

/**
 * Where a stream is: a link to its bytes for one in the file, and for one
 * inside a joined stream its offset there, as plain text, since the joined
 * bytes are at no one place in the file to mark or go to.
 */
function addressOf(n: TemplateNode, label: string): Node {
  if (n.space === 0) return byteRef({ path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits }, label, formatOffset(n.offset_bits));
  const at = document.createElement("span");
  at.className = "rv-muted";
  at.textContent = formatAddress(n.offset_bits, n.space);
  return at;
}

/** The streams joined from pieces in different places, one line each, with a
 *  button to show the joined bytes in the listing. */
function joinList(ctx: ReportCtx, joins: readonly TemplateNode[]): HTMLElement {
  const list = document.createElement("ul");
  list.className = "rv-joins";
  for (const j of joins.slice(0, JOINS_MAX)) {
    const li = document.createElement("li");
    const code = document.createElement("code");
    code.textContent = j.name;
    const b = document.createElement("button");
    b.type = "button";
    b.className = "rv-button";
    b.textContent = RV.showInListing;
    b.addEventListener("click", () => ctx.host.showInListing(j.path));
    li.append(code, RV.joinLine(bytesText(j.size_bits / 8)), " ", b);
    list.append(li);
  }
  if (joins.length > JOINS_MAX) {
    const li = document.createElement("li");
    li.className = "rv-muted";
    li.textContent = RV.moreJoins(joins.length - JOINS_MAX);
    list.append(li);
  }
  return list;
}

/** Joined streams listed, at most. */
const JOINS_MAX = 8;

/** What to call a stream: the element of a list it is inside, by the name the
 *  listing gives it (`word/document.xml`), and its own field name after. */
function streamLabel(doc: Doc, node: TemplateNode): string | typeof WAIT {
  for (let i = node.path.length - 1; i > 0; i--) {
    const up = ok(doc.templateNode(node.path.slice(0, i)));
    if (up === WAIT) return WAIT;
    if (up === null) break;
    if (/^\[\d+\]/.test(up.name)) {
      const bare = stripIndex(up.name);
      if (bare !== "") return bare;
    }
  }
  return node.name;
}

function streamBlock(ctx: ReportCtx, o: Opened): HTMLElement {
  const box = document.createElement("div");
  box.className = "rv-stream";
  if (o.node.space === 0) {
    box.dataset.rvPartStart = String(o.node.offset_bits);
    box.dataset.rvPartEnd = String(o.node.offset_bits + o.node.size_bits);
  }
  const packed = o.node.size_bits / 8;
  const unpacked = o.space?.lengthBytes ?? null;
  const h = document.createElement("h3");
  const name = document.createElement("code");
  name.textContent = o.label;
  const [before, after] = RV.streamHeading("\u0000", packed, o.node.type, unpacked).split("\u0000");
  h.append(before ?? "", name, after ?? "");
  if (o.node.space === 0) pointAt(h, { path: o.node.path, startBit: o.node.offset_bits, endBit: o.node.offset_bits + o.node.size_bits }, o.label);
  box.append(h);
  if (o.space === null) {
    const p = document.createElement("p");
    p.className = "rv-note";
    p.append(`${RV.streamNotOpened(formatBytes(OPEN_BYTES))} `);
    const b = document.createElement("button");
    b.type = "button";
    b.className = "rv-button";
    b.textContent = RV.openUnpacked;
    b.addEventListener("click", () => ctx.host.openUnpacked(o.node.path));
    p.append(b);
    box.append(p);
    return box;
  }
  box.append(stages(packed, o.space.lengthBytes));
  const r = traceRibbon(ctx, o);
  if (r !== null) box.append(r);
  return box;
}

/** One bar per stage, each as wide as its size. */
function stages(packed: number, unpacked: number): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-stages";
  const most = Math.max(1, packed, unpacked);
  for (const [label, n, cls] of [
    [RV.stageCompressed, packed, "is-packed"],
    [RV.stageUnpacked, unpacked, "is-unpacked"],
  ] as const) {
    const row = document.createElement("div");
    row.className = "rv-stage";
    const name = document.createElement("span");
    name.className = "rv-stage-name";
    name.textContent = label;
    const bar = document.createElement("span");
    bar.className = `rv-stage-bar ${cls}`;
    bar.style.width = `${Math.max(0.5, (n / most) * 100)}%`;
    const size = document.createElement("span");
    size.className = "rv-stage-size";
    size.textContent = bytesText(n);
    const track = document.createElement("span");
    track.className = "rv-stage-track";
    track.append(bar);
    row.append(name, track, size);
    fig.append(row);
  }
  const cap = document.createElement("figcaption");
  cap.textContent = RV.stagesCaption(packed > 0 ? `${(unpacked / packed).toFixed(unpacked / packed < 10 ? 1 : 0)} times` : "");
  fig.append(cap);
  return fig;
}

/** The first codes of a traced stream joined to the bytes they write, or null
 *  for a codec that keeps no trace. */
function traceRibbon(ctx: ReportCtx, o: Opened): HTMLElement | null {
  const space = o.space;
  if (space === null) return null;
  const read: MapStep[] = [];
  let out = 0;
  while (read.length < SCAN_STEPS && out < space.lengthBytes) {
    const s = space.mapOut(out);
    if (s === null) break;
    read.push(s);
    out = Math.max(out + 1, s.out_end);
  }
  // The window starts a little before the first copy, so it shows both kinds
  // of code: most streams open with a run of literals, since there is nothing
  // earlier to copy yet.
  const firstCopy = read.findIndex((s) => s.kind === "match");
  const start = firstCopy < 0 ? 0 : Math.max(0, Math.min(firstCopy - LEAD_STEPS, read.length - STEPS));
  const steps = read.slice(start, start + STEPS);
  const first = steps[0];
  const last = steps[steps.length - 1];
  if (first === undefined || last === undefined) return null;
  const outBytes = space.read(first.out_start, last.out_end - first.out_start);
  const textOf = (s: MapStep): string => {
    if (!outBytes.complete) return "";
    let t = "";
    for (let i = s.out_start; i < s.out_end && t.length < 24; i++) {
      const v = outBytes.bytes[i - first.out_start] ?? 0;
      t += v >= 0x20 && v < 0x7f ? String.fromCharCode(v) : "·";
    }
    return t;
  };
  const colour = (s: MapStep): string => (s.kind === "literal" ? "var(--rv-literal)" : s.kind === "match" ? "var(--rv-match)" : "var(--muted)");
  const data: RibbonData = {
    top: {
      from: first.in_start,
      to: last.in_end,
      caption: RV.ribbonTop(last.in_end - first.in_start),
      boxes: steps.map((s) => ({ from: s.in_start, to: s.in_end, color: colour(s) })),
    },
    bottom: {
      from: first.out_start,
      to: last.out_end,
      caption: RV.ribbonBottom(last.out_end - first.out_start),
      boxes: steps.map((s) => ({ from: s.out_start, to: s.out_end, color: colour(s), label: textOf(s) })),
    },
    bands: steps.map((_, i) => ({ top: i, bottom: i })),
  };
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-ribbonfig";
  const svg = ribbon(data, {
    width: 960,
    onHover: (i, e) => {
      const s = i === null ? undefined : steps[i];
      if (s === undefined) {
        hideTip();
        return;
      }
      const box = document.createElement("div");
      const b = document.createElement("b");
      b.textContent =
        s.kind === "literal" ? RV.stepLiteral(byteText(s.value ?? 0)) : s.kind === "match" ? RV.stepMatch(s.len ?? s.out_end - s.out_start, s.dist ?? 0) : RV.stepOther(s.kind);
      const bits = document.createElement("div");
      bits.className = "rv-tip-where";
      bits.textContent = RV.stepBits(s.in_start, s.in_end);
      const bytes = document.createElement("div");
      bytes.className = "rv-tip-where";
      bytes.textContent = RV.stepBytes(s.out_start, s.out_end);
      const text = textOf(s);
      box.append(b, bits, bytes);
      if (text !== "") {
        const t = document.createElement("code");
        t.textContent = text;
        box.append(t);
      }
      tipAt(box, e.clientX, e.clientY);
    },
    onPick: (i) => {
      const s = steps[i];
      if (s === undefined) return;
      ctx.host.pick({ startBit: s.run_offset_bits + s.in_start, endBit: s.run_offset_bits + s.in_end });
    },
  });
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  lead.textContent = RV.ribbonCaptionLead(start, start + steps.length);
  cap.append(lead, ` ${RV.ribbonCaption}`);
  fig.append(svg, cap);
  return fig;
}

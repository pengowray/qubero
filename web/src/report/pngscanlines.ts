// A PNG's scanlines: the passes of an interlaced image, and the filter each
// scanline was written with.
//
// Read off the `png` template's own fields, found by name: the image header for
// the size and the passes, and the blocks of the trace the scanlines were
// unfiltered with, one a scanline, each holding its pass, its row, its filter
// byte and its filtered row. See "A picture's pixels, from the bytes to the
// grid" in docs/DESIGN.md.
//
// What it does not draw yet: how many compressed bits each scanline cost. The
// core joins every inflated byte to the deflate code that wrote it, but does
// not yet share a code's bits out over the bytes it wrote.

import type { Doc, TemplateNode } from "../doc.ts";
import { ok } from "./model.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { RV } from "./text.ts";
import { hideTip, tipAt } from "./tip.ts";
import { adam7Passes, FILTERS, filterCounts, numberOf, passOf, pictureRow, type Scanline } from "./pngpasses.ts";

/** Scanlines read for the strip. A picture taller than this is drawn up to it
 *  and says so. */
const MAX_SCANLINES = 2048;
/** Scanlines asked of the core in one call. */
const BATCH = 256;

type Header = { readonly width: number; readonly height: number; readonly interlaced: boolean };

type Found =
  | { readonly kind: "none" }
  | { readonly kind: "refused"; readonly stage: "inflate" | "unfilter"; readonly why: string }
  | { readonly kind: "read"; readonly header: Header; readonly lines: readonly Scanline[]; readonly total: number };

export const pngScanlinesSection: Section = {
  id: "png-scanlines",
  render(ctx: ReportCtx): Rendered {
    if (ctx.doc.template !== "png") return null;
    const found = ctx.data.memo("png-scanlines", () => readScanlines(ctx.doc));
    if (found === WAIT) return WAIT;
    if (found.kind === "none") return null;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-png";
    const h = document.createElement("h2");
    sec.append(h);
    if (found.kind === "refused") {
      h.textContent = RV.pngScanlinesUnread;
      const p = document.createElement("p");
      p.textContent = found.stage === "inflate" ? RV.pngNotInflated(found.why) : RV.pngNotUnfiltered(found.why);
      sec.append(p);
      return sec;
    }
    const { header, lines, total } = found;
    const counts = filterCounts(lines);
    const most = counts.reduce((best, n, i) => (n > (counts[best] ?? 0) ? i : best), 0);
    h.textContent = RV.pngScanlinesHeading(total, header.interlaced, FILTERS[most] ?? "", counts[most] ?? 0, lines.length === total);
    const intro = document.createElement("p");
    const used = FILTERS.map((name, i) => [name, counts[i] ?? 0] as const)
      .filter(([, n]) => n > 0)
      .sort((a, b) => b[1] - a[1]);
    intro.textContent = `${RV.pngFilterIntro} ${RV.pngFilterCounts(used, lines.length < total ? lines.length : null)}`;
    sec.append(intro);
    if (header.interlaced) {
      const p = document.createElement("p");
      p.textContent = RV.pngAdam7Intro;
      sec.append(p, passesFigure(header, lines));
    }
    sec.append(stripFigure(ctx, header, lines, counts, total));
    return sec;
  },
};

/** Find the scanlines and read every one of them, up to `MAX_SCANLINES`. */
function readScanlines(doc: Doc): Found | typeof WAIT {
  const root = ok(doc.templateNode([]));
  if (root === WAIT) return WAIT;
  if (root === null) return { kind: "none" };
  const header = readHeader(doc, root);
  if (header === WAIT) return WAIT;
  if (header === null) return { kind: "none" };
  // image → the zlib stream it joins → what that inflates to → the scanlines.
  const image = childNamed(doc, root, "image");
  if (image === WAIT) return WAIT;
  if (image === null || image.child_count === 0) return { kind: "none" };
  const zlib = firstChild(doc, image);
  if (zlib === WAIT) return WAIT;
  if (zlib === null) return { kind: "none" };
  const compressed = childNamed(doc, zlib, "compressed");
  if (compressed === WAIT) return WAIT;
  if (compressed === null) return { kind: "none" };
  if (compressed.refused !== null) return { kind: "refused", stage: "inflate", why: compressed.refused };
  const inflated = firstChild(doc, compressed);
  if (inflated === WAIT) return WAIT;
  if (inflated === null) return { kind: "none" };
  const scanlines = childNamed(doc, inflated, "scanlines");
  if (scanlines === WAIT) return WAIT;
  if (scanlines === null) return { kind: "none" };
  if (scanlines.refused !== null) return { kind: "refused", stage: "unfilter", why: scanlines.refused };
  if (scanlines.child_count < 2) return { kind: "none" };
  const blocks = ok(doc.templateNode([...scanlines.path, 1]));
  if (blocks === WAIT) return WAIT;
  if (blocks === null) return { kind: "none" };
  const total = blocks.child_count;
  const want = Math.min(total, MAX_SCANLINES);
  const lines: Scanline[] = [];
  for (let from = 0; from < want; from += BATCH) {
    const got = ok(doc.templateChildren(blocks.path, from, Math.min(want, from + BATCH)));
    if (got === WAIT) return WAIT;
    if (got === null) break;
    for (const block of got) {
      const line = readScanline(doc, block);
      if (line === WAIT) return WAIT;
      if (line !== null) lines.push(line);
    }
  }
  return { kind: "read", header, lines, total };
}

/** A scanline's pass, row and filter, from the fields of its block. */
function readScanline(doc: Doc, block: TemplateNode): Scanline | null | typeof WAIT {
  const kids = ok(doc.templateChildren(block.path, 0, block.child_count));
  if (kids === WAIT) return WAIT;
  if (kids === null) return null;
  const num = (name: string): number | null => {
    const k = kids.find((n) => n.name === name);
    return k === undefined ? null : numberOf(k);
  };
  const filter = num("filter");
  const row = num("row");
  if (filter === null || row === null || filter < 0 || filter > 4) return null;
  return { pass: num("pass"), row, filter, bytes: block.size_bits / 8, path: block.path };
}

/** Width, height and interlacing, from the header chunk: the first chunk, and
 *  one whose `type` reads IHDR. */
function readHeader(doc: Doc, root: TemplateNode): Header | null | typeof WAIT {
  const chunks = childNamed(doc, root, "chunks");
  if (chunks === WAIT) return WAIT;
  if (chunks === null || chunks.child_count === 0) return null;
  const first = firstChild(doc, chunks);
  if (first === WAIT) return WAIT;
  if (first === null) return null;
  const data = childNamed(doc, first, "data");
  if (data === WAIT) return WAIT;
  if (data === null || data.type !== "IHDR") return null;
  const fields = ok(doc.templateChildren(data.path, 0, data.child_count));
  if (fields === WAIT) return WAIT;
  if (fields === null) return null;
  const num = (name: string): number | null => {
    const k = fields.find((n) => n.name === name);
    return k === undefined ? null : numberOf(k);
  };
  const width = num("width");
  const height = num("height");
  if (width === null || height === null) return null;
  return { width, height, interlaced: num("interlace") === 1 };
}


function childNamed(doc: Doc, parent: TemplateNode, name: string): TemplateNode | null | typeof WAIT {
  const kids = ok(doc.templateChildren(parent.path, 0, parent.child_count));
  if (kids === WAIT) return WAIT;
  if (kids === null) return null;
  return kids.find((k) => k.name === name) ?? null;
}

function firstChild(doc: Doc, parent: TemplateNode): TemplateNode | null | typeof WAIT {
  const kids = ok(doc.templateChildren(parent.path, 0, 1));
  if (kids === WAIT) return WAIT;
  return kids?.[0] ?? null;
}

/** The 8 by 8 tile of which pass stores each pixel, beside a table of the
 *  seven passes of this picture. */
function passesFigure(header: Header, lines: readonly Scanline[]): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-png-passes";
  const tile = document.createElement("div");
  tile.className = "rv-png-tile";
  tile.setAttribute("role", "img");
  tile.setAttribute("aria-label", RV.pngTileLabel);
  for (let y = 0; y < 8; y++) {
    for (let x = 0; x < 8; x++) {
      const cell = document.createElement("span");
      const p = passOf(x, y);
      cell.textContent = String(p);
      cell.dataset.pass = String(p);
      tile.append(cell);
    }
  }
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const table = document.createElement("table");
  table.className = "rv-table";
  const head = table.createTHead().insertRow();
  for (const [text, num] of RV.pngPassColumns) {
    const th = document.createElement("th");
    th.textContent = text;
    if (num) th.className = "rv-num";
    head.append(th);
  }
  const body = table.createTBody();
  const passes = adam7Passes(header.width, header.height);
  for (const p of passes) {
    const mine = lines.filter((l) => l.pass === p.pass);
    const row = body.insertRow();
    const stored = p.cols > 0 && p.rows > 0;
    const cells: [string, boolean][] = [
      [String(p.pass), false],
      [stored ? RV.pngPassSize(p.cols, p.rows) : RV.pngPassEmpty, false],
      [(stored ? p.cols * p.rows : 0).toLocaleString(), true],
      [(stored ? p.rows : 0).toLocaleString(), true],
      [mine[0] === undefined ? "" : mine[0].bytes.toLocaleString(), true],
    ];
    for (const [text, num] of cells) {
      const td = row.insertCell();
      td.textContent = text;
      if (num) td.className = "rv-num";
    }
  }
  wrap.append(table);
  const body2 = document.createElement("div");
  body2.className = "rv-png-passbody";
  body2.append(tile, wrap);
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  lead.textContent = RV.pngPassesLead(header.width, header.height);
  cap.append(lead, ` ${RV.pngPassesCaption}`);
  fig.append(body2, cap);
  return fig;
}

/** Every scanline as a mark coloured and lettered by its filter, in stored
 *  order, grouped by pass when the picture is interlaced. */
function stripFigure(ctx: ReportCtx, header: Header, lines: readonly Scanline[], counts: readonly number[], total: number): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-png-strip";
  // A key to the filters this picture uses, and none when it uses one: the
  // heading has already named it.
  const legend = document.createElement("div");
  legend.className = "rv-legend";
  FILTERS.forEach((name, i) => {
    if ((counts[i] ?? 0) === 0) return;
    const item = document.createElement("span");
    item.className = "rv-png-key";
    const sw = document.createElement("span");
    sw.className = "rv-png-swatch";
    sw.dataset.filter = String(i);
    sw.textContent = name.charAt(0);
    item.append(sw, RV.pngLegendItem(name, counts[i] ?? 0));
    legend.append(item);
  });
  if (legend.childElementCount > 1) fig.append(legend);
  const groups = new Map<number | null, Scanline[]>();
  for (const l of lines) {
    const g = groups.get(l.pass);
    if (g === undefined) groups.set(l.pass, [l]);
    else g.push(l);
  }
  for (const [pass, group] of groups) {
    const row = document.createElement("div");
    row.className = "rv-png-group";
    if (pass !== null) {
      const label = document.createElement("span");
      label.className = "rv-png-grouplabel";
      label.textContent = RV.pngPassLabel(pass);
      row.append(label);
    }
    const cells = document.createElement("span");
    cells.className = "rv-png-cells";
    for (const l of group) {
      const cell = document.createElement("button");
      cell.type = "button";
      cell.className = "rv-png-line";
      cell.dataset.filter = String(l.filter);
      cell.textContent = (FILTERS[l.filter] ?? "?").charAt(0);
      cell.setAttribute("aria-label", RV.pngLineTitle(l.pass, l.row, FILTERS[l.filter] ?? ""));
      cell.addEventListener("pointerenter", (e) => tipAt(lineTip(header, l), e.clientX, e.clientY));
      cell.addEventListener("pointerleave", () => hideTip());
      cell.addEventListener("click", () => ctx.host.showInListing(l.path));
      cells.append(cell);
    }
    row.append(cells);
    fig.append(row);
  }
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  lead.textContent = RV.pngStripLead;
  cap.append(lead, ` ${RV.pngStripCaption}`);
  if (lines.length < total) cap.append(` ${RV.pngStripCut(lines.length, total)}`);
  fig.append(cap);
  return fig;
}

function lineTip(header: Header, l: Scanline): HTMLElement {
  const box = document.createElement("div");
  const b = document.createElement("b");
  b.textContent = RV.pngLineTitle(l.pass, l.row, FILTERS[l.filter] ?? "");
  box.append(b);
  if (header.interlaced) {
    const where = document.createElement("div");
    where.className = "rv-tip-where";
    where.textContent = RV.pngLinePictureRow(pictureRow(l));
    box.append(where);
  }
  const size = document.createElement("div");
  size.className = "rv-tip-where";
  size.textContent = RV.pngLineBytes(l.bytes);
  box.append(size);
  return box;
}

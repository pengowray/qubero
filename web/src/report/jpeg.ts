// Section 9, for a JPEG: where each baseline scan's bits go in the picture,
// and one block traced from its codes to its samples.
//
// Drawn from the core's trace of the scan (`Doc.jpegScan`, `Doc.jpegBlock`),
// which is Qubero's own decode: every code with its bits, every block with its
// place, every MCU with its cost. Nothing here decodes the scan again. What is
// worked out here is only what follows from the coefficients by arithmetic,
// which the hand-written report did the same way: multiplying by the
// quantization steps, and the inverse DCT to samples.
//
// Two figures and two tables per scan. The map shades each MCU by its bits,
// over the picture when the file is the JPEG, in the pixel grid
// (`viz/pixelgrid.ts`), which zooms, pans, and can shade one channel's
// blocks instead. The tables
// say which channel and which kind of code the bits went to. The walkthrough
// takes the MCU of median cost, or the one the reader clicks, and shows one of
// its blocks as its codes, then its coefficients in zigzag order and in rows,
// dequantized, and turned into samples.

import "./jpeg.css";
import type { Doc, JpegBlock, JpegCode, JpegScan, TemplateNode } from "../doc.ts";
import { cardState, watchCard } from "../contentcard.ts";
import { drawPixelGrid, gridArea, gridState, type Layers, type PixelGrid, type PixelGridState } from "../viz/pixelgrid.ts";
import { blockBits, indexScan, mcuBits, mcuSize, viewSize } from "../viz/pixelgridmath.ts";
import { ok } from "./model.ts";
import { byteRef } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { hideTip, tipAt } from "./tip.ts";
import { walkTemplate } from "./walk.ts";
import { JPEG, share } from "./jpegtext.ts";

/** Scans given a section each; a file of more is a file of one channel per
 *  scan, and three or four cover it. */
const SCANS_SHOWN = 4;
/** The widest the picture with its map over it is drawn: the width the
 *  picture and the map took when they were drawn side by side. */
const MAP_PX = 904;

export const jpegSection: Section = {
  id: "jpeg",
  render(ctx: ReportCtx): Rendered {
    if (ctx.doc.template === null) return null;
    const walk = ctx.data.memo("walk", () => walkTemplate(ctx.doc));
    if (walk === WAIT) return WAIT;
    const runs = walk.streams.filter((s) => s.type === "jpeg scan" && s.refused === null);
    if (runs.length === 0) return null;
    const scans: { node: TemplateNode; map: JpegScan }[] = [];
    for (const node of runs.slice(0, SCANS_SHOWN)) {
      const map = ctx.data.memo(`jpeg:${node.path.join("/")}`, () => ok(ctx.doc.jpegScan(node.path)));
      if (map === WAIT) return WAIT;
      if (map !== null) scans.push({ node, map });
    }
    if (scans.length === 0) return null;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-jpeg";
    const first = scans[0];
    if (first !== undefined) {
      sec.dataset.rvPartStart = String(first.node.offset_bits);
      sec.dataset.rvPartEnd = String(first.node.offset_bits + first.node.size_bits);
    }
    for (const s of scans) sec.append(...scanBody(ctx, s.node, s.map, scans.length > 1));
    return sec;
  },
};

function scanBody(ctx: ReportCtx, node: TemplateNode, map: JpegScan, several: boolean): HTMLElement[] {
  const out: HTMLElement[] = [];
  const names = map.scan.map((i) => map.channels[i]?.name ?? "");
  const h = document.createElement("h2");
  h.textContent = several ? JPEG.headingOf(names, node.size_bits / 8) : JPEG.heading(node.size_bits / 8);
  // The scan's part section points here, by the path of the scan's data.
  h.dataset.rvScan = node.path.join("/");
  out.push(h);

  const g = mcuSize(map);
  const mcus = map.mcus_across * map.mcus_down;
  const perMcu = mcus === 0 ? 0 : map.block_channel.length / mcus;
  const lede = document.createElement("p");
  if (map.scan.length === 1) {
    lede.textContent = JPEG.ledeOne(names[0] ?? "", map.width, map.height, mcus, map.mcus_across, map.mcus_down);
  } else {
    const parts = map.scan.map((i) => {
      const c = map.channels[i];
      return JPEG.ledePart((c?.h ?? 1) * (c?.v ?? 1), c?.name ?? "");
    });
    lede.textContent = JPEG.ledeInterleaved(map.width, map.height, mcus, map.mcus_across, map.mcus_down, g.mw, g.mh, perMcu, parts);
  }
  if (map.restart_interval > 0) lede.textContent += JPEG.ledeRestart(map.restart_interval);
  out.push(lede);

  // Which MCU the walkthrough starts at: the one of median cost among those
  // whose codes are named, which is a typical block rather than an extreme.
  const firstBlock = (m: number): number => (map.scan.length === 1 ? m : m * perMcu);
  const named = [...Array(mcus).keys()].filter((m) => (map.block_codes[firstBlock(m)] ?? 0) > 0);
  named.sort((a, b) => (map.mcu_bits[a] ?? 0) - (map.mcu_bits[b] ?? 0));
  const median = named[Math.floor(named.length / 2)] ?? 0;

  const walk = document.createElement("div");
  walk.className = "rv-jpeg-walk";
  const choose = (mcu: number, block: number): void => {
    hideTip();
    walk.replaceChildren(...blockWalk(ctx, node, map, mcu, block, mcu === median, choose));
  };
  out.push(mapFigure(ctx, node, map, (mcu, block) => choose(mcu, block ?? firstBlock(mcu))));
  if (map.coarse) {
    const n = map.block_codes.filter((c) => c === 0).length;
    const p = document.createElement("p");
    p.className = "rv-note";
    p.textContent = JPEG.coarse(n, map.step_limit);
    out.push(p);
  }
  out.push(channelTable(map), kindTable(map));
  choose(median, firstBlock(median));
  out.push(walk);
  return out;
}

// ----- the map -----

/** The reader's overlay, channel and zoom on each scan's map, kept while the
 *  report draws its sections again. */
const mapStates = new WeakMap<Doc, Map<string, PixelGridState>>();

function mapState(doc: Doc, node: TemplateNode): PixelGridState {
  let byScan = mapStates.get(doc);
  if (byScan === undefined) {
    byScan = new Map();
    mapStates.set(doc, byScan);
  }
  const key = node.path.join("/");
  let s = byScan.get(key);
  if (s === undefined) {
    s = gridState();
    byScan.set(key, s);
  }
  return s;
}

/**
 * The picture with the bits of each MCU shaded over it, in the pixel grid
 * (`viz/pixelgrid.ts`), which zooms and pans and can show one channel's
 * blocks instead. The picture is the browser's, when the file is this JPEG
 * and the browser has drawn it at the scan's size; without it the shading is
 * drawn alone. A click picks the MCU for the walkthrough below and selects
 * its bits.
 */
function mapFigure(ctx: ReportCtx, node: TemplateNode, map: JpegScan, pick: (mcu: number, block: number | null) => void): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-jpeg-mapfig";
  const holder = document.createElement("div");
  const index = indexScan(map);
  const layers: Layers = { jpeg: { node, scan: map, index }, png: null };
  const state = mapState(ctx.doc, node);
  let grid: PixelGrid | null = null;
  const draw = (image: HTMLImageElement | null): void => {
    grid?.destroy();
    const area = gridArea(map.width, map.height, layers);
    const size = viewSize(area.w, area.h, MAP_PX);
    grid = drawPixelGrid(
      ctx.doc,
      { width: map.width, height: map.height, image, layers },
      {
        ...size,
        state,
        defaultMode: "bits",
        onPick: (hit) => {
          if (hit.mcu === null) return;
          const oneChannel = state.mode === "bits" && state.channel !== null && hit.block !== null;
          pick(hit.mcu, oneChannel ? hit.block : null);
          ctx.host.pick(oneChannel && hit.block !== null ? blockBits(map, index, hit.block) : mcuBits(index, hit.mcu));
        },
      },
    );
    holder.replaceChildren(grid.el);
  };
  // The browser's picture: null when there is none to draw under the map,
  // and undefined while it is still being decoded.
  const picture = (): HTMLImageElement | null | undefined => {
    const s = cardState(ctx.doc);
    if (s === null || s.status === "failed" || s.status === "too-large") return null;
    if (s.status !== "ready") return undefined;
    return s.width === map.width && s.height === map.height ? s.image : null;
  };
  const now = picture();
  draw(now ?? null);
  if (now === undefined) {
    const stop = watchCard(ctx.doc, (what) => {
      if (what !== "state") return;
      const p = picture();
      if (p === undefined) return;
      stop();
      if (p !== null && holder.isConnected) draw(p);
    });
  }
  fig.append(holder);

  const bits = map.mcu_bits;
  const max = bits.reduce((a, b) => Math.max(a, b), 0);
  const cap = document.createElement("figcaption");
  const most = bits.indexOf(max);
  const sorted = [...bits].sort((a, b) => a - b);
  const mid = sorted[Math.floor(sorted.length / 2)] ?? 0;
  const lead = document.createElement("b");
  lead.textContent = JPEG.captionLead(most % map.mcus_across, Math.floor(most / map.mcus_across), max, mid);
  cap.append(lead, ` ${JPEG.caption}`);
  fig.append(cap);
  return fig;
}

// ----- where the bits go -----

function table(head: readonly string[], rows: readonly (readonly (string | Node)[])[], numeric: readonly boolean[]): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const t = document.createElement("table");
  t.className = "rv-table rv-jpeg-table";
  const thead = document.createElement("thead");
  const hr = document.createElement("tr");
  head.forEach((h, i) => {
    const th = document.createElement("th");
    th.textContent = h;
    if (numeric[i] === true) th.className = "rv-num";
    hr.append(th);
  });
  thead.append(hr);
  const tbody = document.createElement("tbody");
  for (const row of rows) {
    const tr = document.createElement("tr");
    row.forEach((cell, i) => {
      const td = document.createElement("td");
      td.append(cell);
      if (numeric[i] === true) td.className = "rv-num";
      tr.append(td);
    });
    tbody.append(tr);
  }
  t.append(thead, tbody);
  wrap.append(t);
  return wrap;
}

function channelTable(map: JpegScan): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure";
  const bits = new Map<number, number>();
  const blocks = new Map<number, number>();
  for (let i = 0; i < map.block_channel.length; i++) {
    const c = map.block_channel[i] ?? 0;
    bits.set(c, (bits.get(c) ?? 0) + (map.block_bits[i] ?? 0));
    blocks.set(c, (blocks.get(c) ?? 0) + 1);
  }
  const allBits = [...bits.values()].reduce((a, b) => a + b, 0);
  const allBlocks = map.block_channel.length;
  const rows = map.scan.map((c) => {
    const b = bits.get(c) ?? 0;
    const n = blocks.get(c) ?? 0;
    return [map.channels[c]?.name ?? "", n.toLocaleString(), b.toLocaleString(), share(b, allBits), n === 0 ? "" : (b / n).toFixed(1)];
  });
  const first = map.scan[0] ?? 0;
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  lead.textContent = JPEG.channelsLead(map.channels[first]?.name ?? "", share(bits.get(first) ?? 0, allBits), share(blocks.get(first) ?? 0, allBlocks));
  cap.append(lead);
  fig.append(table([JPEG.channelCol, JPEG.blocksCol, JPEG.bitsCol, JPEG.shareCol, JPEG.perBlockCol], rows, [false, true, true, true, true]), cap);
  return fig;
}

function kindTable(map: JpegScan): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure";
  const t = map.totals;
  const whole = map.run_bits;
  const order = ["dc_code", "dc_value", "ac_code", "ac_value", "eob", "zrl", "padding", "stuffed", "markers", "unnamed"] as const;
  const rows = order
    .filter((k) => t[k] > 0)
    .map((k) => [JPEG.kind[k] ?? k, t[k].toLocaleString(), share(t[k], whole)]);
  rows.push([JPEG.total, whole.toLocaleString(), "100%"]);
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  lead.textContent = JPEG.kindsLead(share(t.ac_code + t.ac_value, whole));
  cap.append(lead);
  fig.append(table([JPEG.kindCol, JPEG.bitsCol, JPEG.shareCol], rows, [false, true, true]), cap);
  return fig;
}

// ----- one block -----

function blockWalk(
  ctx: ReportCtx,
  node: TemplateNode,
  map: JpegScan,
  mcu: number,
  index: number,
  isMedian: boolean,
  choose: (mcu: number, block: number) => void,
): HTMLElement[] {
  const block = ok(ctx.doc.jpegBlock(node.path, index));
  if (block === null || block === WAIT) return [];
  const name = map.channels[block.channel]?.name ?? "";
  const out: HTMLElement[] = [];
  const named = block.codes.every((c) => c.kind !== "opaque");
  const h = document.createElement("h3");
  h.textContent = JPEG.blockHeading(name, block.x, block.y, named ? block.codes.length : 0);
  out.push(h);

  const where = document.createElement("p");
  const mx = mcu % map.mcus_across;
  const my = Math.floor(mcu / map.mcus_across);
  where.append(`${JPEG.blockWhere(mx, my, isMedian)} `);
  const show = document.createElement("button");
  show.type = "button";
  show.className = "rv-button";
  show.textContent = JPEG.inListing;
  show.addEventListener("click", () => ctx.host.showInListing(block.path));
  where.append(show);
  out.push(where);

  // The other blocks of the same MCU, to step through without going back up
  // to the map.
  const mcus = map.mcus_across * map.mcus_down;
  const per = mcus === 0 ? 1 : map.block_channel.length / mcus;
  if (per > 1) {
    const pickRow = document.createElement("div");
    pickRow.className = "rv-jpeg-pick";
    const label = document.createElement("span");
    label.className = "rv-muted";
    label.textContent = JPEG.blockPick;
    pickRow.append(label);
    for (let i = mcu * per; i < (mcu + 1) * per; i++) {
      const c = map.channels[map.block_channel[i] ?? 0]?.name ?? "";
      const x = map.block_x[i] ?? 0;
      const y = map.block_y[i] ?? 0;
      const b = document.createElement("button");
      b.type = "button";
      b.className = "rv-button";
      b.textContent = JPEG.blockButton(c, x, y);
      b.setAttribute("aria-label", JPEG.blockButtonLabel(c, x, y));
      b.setAttribute("aria-pressed", String(i === index));
      b.addEventListener("click", () => choose(mcu, i));
      pickRow.append(b);
    }
    out.push(pickRow);
  }

  if (!named) {
    const p = document.createElement("p");
    p.className = "rv-note";
    p.textContent = JPEG.unnamedBlock(map.step_limit);
    out.push(p);
    return out;
  }
  out.push(codeStrip(ctx, block), codeTable(block));
  out.push(grids(block, map.channels[block.channel]?.quant ?? null, map.channels[block.channel]?.quant_id ?? 0));
  return out;
}

function says(c: JpegCode): string {
  switch (c.kind) {
    case "dc":
      return JPEG.saysDc(c.value, c.dc);
    case "ac":
      return JPEG.saysAc(c.value, c.k, c.run);
    case "zrl":
      return JPEG.saysZrl(c.k);
    case "eob":
      return JPEG.saysEob(c.k);
    default:
      return JPEG.saysUnnamed;
  }
}

/** A code's bits, with the Huffman code set apart from the value after it. */
function bitsOf(c: JpegCode): HTMLElement {
  const s = document.createElement("span");
  s.className = "rv-jpeg-bits";
  s.title = JPEG.bitsTitle(c.code_bits, c.value_bits);
  const code = document.createElement("span");
  code.className = "rv-jpeg-code";
  code.textContent = c.bits.slice(0, c.code_bits);
  const value = document.createElement("span");
  value.className = "rv-jpeg-value";
  value.textContent = c.bits.slice(c.code_bits);
  s.append(code, value);
  return s;
}

function codeStrip(ctx: ReportCtx, block: JpegBlock): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure";
  const strip = document.createElement("div");
  strip.className = "rv-jpeg-strip";
  for (const c of block.codes) {
    const box = document.createElement("span");
    box.className = `rv-jpeg-box is-${c.kind}`;
    box.append(bitsOf(c));
    box.addEventListener("pointermove", (e) => {
      const t = document.createElement("div");
      const b = document.createElement("b");
      b.textContent = says(c);
      const d = document.createElement("div");
      d.textContent = JPEG.bitsTitle(c.code_bits, c.value_bits);
      t.append(b, d);
      tipAt(t, e.clientX, e.clientY);
    });
    box.addEventListener("pointerleave", () => hideTip());
    box.addEventListener("click", () => ctx.host.pick({ startBit: c.start_bit, endBit: c.end_bit }));
    strip.append(box);
  }
  const bits = block.codes.reduce((n, c) => n + c.code_bits + c.value_bits, 0);
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  lead.textContent = JPEG.stripCaptionLead(block.codes.length, bits);
  cap.append(lead, ` ${JPEG.stripCaption}`);
  fig.append(strip, cap);
  return fig;
}

function codeTable(block: JpegBlock): HTMLElement {
  const rows = block.codes.map((c, i) => {
    const what = document.createElement("span");
    what.textContent = says(c);
    if (c.stuffed) {
      const note = document.createElement("span");
      note.className = "rv-muted";
      note.textContent = ` (${JPEG.stuffedNote})`;
      what.append(note);
    }
    return [String(i + 1), bitsOf(c), what, byteRef({ startBit: c.start_bit, endBit: c.end_bit })];
  });
  return table([JPEG.codeNo, JPEG.codeBits, JPEG.codeSays, JPEG.codeAt], rows, [true, false, false, false]);
}

// ----- the grids -----

/** Where each natural position is in zigzag order: the inverse of the order
 *  the scan writes a block in. */
const ZIGZAG = [
  0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50,
  43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
const ZIG_OF: number[] = (() => {
  const inv = new Array<number>(64).fill(0);
  ZIGZAG.forEach((n, k) => {
    inv[n] = k;
  });
  return inv;
})();

/** The two-dimensional inverse DCT of T.81 A.3.3, in floating point, and the
 *  level shift of 128, rounded and kept to 0 to 255. libjpeg's own IDCT is
 *  an integer one and can differ by one level. */
function idct(f: readonly number[]): number[] {
  const c = (u: number): number => (u === 0 ? Math.SQRT1_2 : 1);
  const out: number[] = [];
  for (let y = 0; y < 8; y++) {
    for (let x = 0; x < 8; x++) {
      let s = 0;
      for (let v = 0; v < 8; v++) {
        for (let u = 0; u < 8; u++) {
          s += c(u) * c(v) * (f[v * 8 + u] ?? 0) * Math.cos(((2 * x + 1) * u * Math.PI) / 16) * Math.cos(((2 * y + 1) * v * Math.PI) / 16);
        }
      }
      out.push(Math.max(0, Math.min(255, Math.round(s / 4 + 128))));
    }
  }
  return out;
}

function grid(label: string, values: readonly number[], kind: "coef" | "steps" | "samples"): HTMLElement {
  const box = document.createElement("div");
  box.className = "rv-jpeg-gridbox";
  const l = document.createElement("div");
  l.className = "rv-jpeg-gridlabel";
  l.textContent = label;
  const g = document.createElement("div");
  g.className = `rv-jpeg-grid is-${kind}`;
  values.forEach((v, n) => {
    const cell = document.createElement("span");
    cell.className = "rv-jpeg-cell";
    cell.textContent = String(v);
    if (kind === "coef" && v === 0) cell.classList.add("is-zero");
    if (kind === "samples") {
      cell.style.background = `rgb(${v}, ${v}, ${v})`;
      cell.style.color = v < 128 ? "#fff" : "#000";
    }
    const row = n >> 3;
    const col = n & 7;
    cell.addEventListener("pointermove", (e) => tipAt(JPEG.cellTip(row, col, ZIG_OF[n] ?? 0, v), e.clientX, e.clientY));
    cell.addEventListener("pointerleave", () => hideTip());
    g.append(cell);
  });
  box.append(l, g);
  return box;
}

function grids(block: JpegBlock, quant: readonly number[] | null, quantId: number): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure";
  // The coefficients in the order the scan wrote them, up to the last one
  // that is not zero; the rest are the EOB's.
  const zz = ZIGZAG.map((n) => block.coefficients[n] ?? 0);
  const zigzag = document.createElement("div");
  zigzag.className = "rv-jpeg-gridbox";
  const zl = document.createElement("div");
  zl.className = "rv-jpeg-gridlabel";
  zl.textContent = JPEG.zigzagLabel;
  const row = document.createElement("div");
  row.className = "rv-jpeg-zigzag";
  zz.forEach((v, k) => {
    const cell = document.createElement("span");
    cell.className = "rv-jpeg-cell";
    if (v === 0) cell.classList.add("is-zero");
    cell.textContent = String(v);
    const n = ZIGZAG[k] ?? 0;
    cell.addEventListener("pointermove", (e) => tipAt(JPEG.cellTip(n >> 3, n & 7, k, v), e.clientX, e.clientY));
    cell.addEventListener("pointerleave", () => hideTip());
    row.append(cell);
  });
  zigzag.append(zl, row);
  fig.append(zigzag);

  const set = document.createElement("div");
  set.className = "rv-jpeg-grids";
  set.append(grid(JPEG.rowsLabel, block.coefficients, "coef"));
  if (quant !== null) {
    const deq = block.coefficients.map((v, n) => v * (quant[n] ?? 0));
    set.append(grid(JPEG.stepsLabel(quantId), quant, "steps"), grid(JPEG.dequantLabel, deq, "coef"), grid(JPEG.samplesLabel, idct(deq), "samples"));
  }
  fig.append(set);
  const cap = document.createElement("figcaption");
  const lead = document.createElement("b");
  lead.textContent = JPEG.gridsLead(block.coefficients.filter((v) => v !== 0).length);
  cap.append(lead, ` ${quant === null ? JPEG.noQuant : JPEG.gridsCaption}`);
  fig.append(cap);
  return fig;
}

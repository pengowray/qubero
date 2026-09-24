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
// beside the picture at the same scale when the file is the JPEG. The tables
// say which channel and which kind of code the bits went to. The walkthrough
// takes the MCU of median cost, or the one the reader clicks, and shows one of
// its blocks as its codes, then its coefficients in zigzag order and in rows,
// dequantized, and turned into samples.

import "./jpeg.css";
import type { JpegBlock, JpegCode, JpegScan, TemplateNode } from "../doc.ts";
import { cardState, watchCard } from "../contentcard.ts";
import { ok } from "./model.ts";
import { byteRef } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { hideTip, tipAt } from "./tip.ts";
import { walkTemplate } from "./walk.ts";
import { JPEG, share } from "./jpegtext.ts";

/** Scans given a section each; a file of more is a file of one channel per
 *  scan, and three or four cover it. */
const SCANS_SHOWN = 4;
/** The widest the map and the picture are drawn, each. */
const PANEL_PX = 440;
/** The largest a small picture is magnified. */
const MAX_ZOOM = 6;
/** Classes of the map's shading, light to dark. */
const CLASSES = 7;
/** The sequential blue ramp of the report's charts, low to high, for each
 *  theme: on a light page the low end is pale, and on a dark one it is dark,
 *  so a cheap MCU recedes into the page either way. */
const RAMP_LIGHT = ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"];
const RAMP_DARK = ["#0d366b", "#184f95", "#256abf", "#3987e5", "#6da7ec", "#9ec5f4", "#cde2fb"];

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

/** Where one MCU is in the grid, and the pixels an MCU covers. */
type Geometry = { readonly mw: number; readonly mh: number };

function geometry(map: JpegScan): Geometry {
  const hmax = Math.max(...map.channels.map((c) => c.h));
  const vmax = Math.max(...map.channels.map((c) => c.v));
  const only = map.scan.length === 1 ? map.channels[map.scan[0] ?? 0] : undefined;
  if (only !== undefined) return { mw: (8 * hmax) / only.h, mh: (8 * vmax) / only.v };
  return { mw: 8 * hmax, mh: 8 * vmax };
}

function scanBody(ctx: ReportCtx, node: TemplateNode, map: JpegScan, several: boolean): HTMLElement[] {
  const out: HTMLElement[] = [];
  const names = map.scan.map((i) => map.channels[i]?.name ?? "");
  const h = document.createElement("h2");
  h.textContent = several ? JPEG.headingOf(names, node.size_bits / 8) : JPEG.heading(node.size_bits / 8);
  out.push(h);

  const g = geometry(map);
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
  out.push(mapFigure(ctx, map, g, (mcu) => choose(mcu, firstBlock(mcu))));
  if (map.coarse) {
    const n = map.block_codes.filter((c) => c === 0).length;
    const p = document.createElement("p");
    p.className = "rv-note";
    p.textContent = JPEG.coarse(n);
    out.push(p);
  }
  out.push(channelTable(map), kindTable(map));
  choose(median, firstBlock(median));
  out.push(walk);
  return out;
}

// ----- the map -----

/** A round step that splits 0 to `max` into `CLASSES` classes: 1, 2, 2.5 or 5
 *  times a power of ten. */
function classStep(max: number): number {
  const raw = Math.max(1, max) / CLASSES;
  const p = 10 ** Math.floor(Math.log10(raw));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * p >= raw) return Math.max(1, Math.ceil(m * p));
  return Math.ceil(10 * p);
}

function ramp(): readonly string[] {
  return matchMedia("(prefers-color-scheme: dark)").matches ? RAMP_DARK : RAMP_LIGHT;
}

function mapFigure(ctx: ReportCtx, map: JpegScan, g: Geometry, pick: (mcu: number) => void): HTMLElement {
  const fig = document.createElement("figure");
  fig.className = "rv-figure rv-jpeg-mapfig";
  const across = map.mcus_across;
  const down = map.mcus_down;
  const codedW = across * g.mw;
  const codedH = down * g.mh;
  let scale = PANEL_PX / codedW;
  if (scale >= 1) scale = Math.min(MAX_ZOOM, Math.floor(scale));
  const cssW = Math.max(1, Math.round(codedW * scale));
  const cssH = Math.max(1, Math.round(codedH * scale));

  const panels = document.createElement("div");
  panels.className = "rv-jpeg-panels";
  // The picture, when the file is this JPEG and the browser has drawn it: at
  // the map's scale, over the same area, so an MCU and the part of the
  // picture it codes are at the same place in each.
  const picture = document.createElement("div");
  picture.className = "rv-jpeg-panel";
  const drawPicture = (): boolean => {
    const s = cardState(ctx.doc);
    if (s === null || s.status === "failed" || s.status === "too-large") {
      picture.remove();
      return true;
    }
    if (s.status !== "ready") return false;
    if (s.width !== map.width || s.height !== map.height) {
      picture.remove();
      return true;
    }
    const label = document.createElement("div");
    label.className = "rv-jpeg-panellabel";
    label.textContent = JPEG.pictureLabel;
    const frame = document.createElement("div");
    frame.className = "rv-jpeg-frame";
    frame.style.width = `${cssW}px`;
    frame.style.height = `${cssH}px`;
    const img = document.createElement("img");
    img.src = s.url;
    img.alt = "";
    img.style.width = `${Math.round(map.width * scale)}px`;
    img.style.height = `${Math.round(map.height * scale)}px`;
    if (scale > 1) img.style.imageRendering = "pixelated";
    frame.append(img);
    picture.replaceChildren(label, frame);
    return true;
  };
  if (!drawPicture()) {
    const stop = watchCard(ctx.doc, (what) => {
      if (what === "state" && drawPicture()) stop();
    });
  }
  panels.append(picture);

  const mapPanel = document.createElement("div");
  mapPanel.className = "rv-jpeg-panel";
  const label = document.createElement("div");
  label.className = "rv-jpeg-panellabel";
  label.textContent = JPEG.mapLabel;
  const canvas = document.createElement("canvas");
  canvas.className = "rv-jpeg-map";
  canvas.width = across;
  canvas.height = down;
  canvas.style.width = `${cssW}px`;
  canvas.style.height = `${cssH}px`;
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", JPEG.mapAlt(across, down));
  const bits = map.mcu_bits;
  const max = bits.reduce((a, b) => Math.max(a, b), 0);
  const step = classStep(max);
  const paint = (): void => {
    const c = canvas.getContext("2d");
    if (c === null) return;
    const img = c.createImageData(across, down);
    const colours = ramp().map(rgb);
    for (let m = 0; m < bits.length; m++) {
      const col = colours[Math.min(CLASSES - 1, Math.floor((bits[m] ?? 0) / step))] ?? [0, 0, 0];
      img.data.set([col[0], col[1], col[2], 255], m * 4);
    }
    c.putImageData(img, 0, 0);
  };
  paint();
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (canvas.isConnected) paint();
  });
  const at = (e: PointerEvent): number | null => {
    const r = canvas.getBoundingClientRect();
    const x = Math.floor(((e.clientX - r.left) / r.width) * across);
    const y = Math.floor(((e.clientY - r.top) / r.height) * down);
    if (x < 0 || y < 0 || x >= across || y >= down) return null;
    return y * across + x;
  };
  const whole = bits.reduce((a, b) => a + b, 0);
  canvas.addEventListener("pointermove", (e) => {
    const m = at(e);
    if (m === null) {
      hideTip();
      return;
    }
    tipAt(mcuTip(map, m, whole), e.clientX, e.clientY);
  });
  canvas.addEventListener("pointerleave", () => hideTip());
  canvas.addEventListener("click", (e) => {
    const m = at(e);
    if (m === null) return;
    pick(m);
    let start = map.run_offset_bits;
    for (let i = 0; i < m; i++) start += bits[i] ?? 0;
    ctx.host.pick({ startBit: start, endBit: start + (bits[m] ?? 0) });
  });
  mapPanel.append(label, canvas, legend(step));
  panels.append(mapPanel);
  fig.append(panels);

  const cap = document.createElement("figcaption");
  const most = bits.indexOf(max);
  const sorted = [...bits].sort((a, b) => a - b);
  const mid = sorted[Math.floor(sorted.length / 2)] ?? 0;
  const lead = document.createElement("b");
  lead.textContent = JPEG.captionLead(most % across, Math.floor(most / across), max, mid);
  cap.append(lead, ` ${JPEG.caption}`);
  fig.append(cap);
  return fig;
}

function rgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function legend(step: number): HTMLElement {
  const l = document.createElement("div");
  l.className = "rv-jpeg-legend";
  const colours = ramp();
  for (let i = 0; i < CLASSES; i++) {
    const item = document.createElement("span");
    item.className = "rv-jpeg-legenditem";
    const sw = document.createElement("span");
    sw.className = "rv-jpeg-swatch";
    sw.style.background = colours[i] ?? "";
    const t = document.createElement("span");
    t.textContent = JPEG.legendClass(i * step, (i + 1) * step - 1);
    item.append(sw, t);
    l.append(item);
  }
  return l;
}

/** The hover over one MCU: its place, its bits, and its blocks' bits by
 *  channel. */
function mcuTip(map: JpegScan, m: number, whole: number): HTMLElement {
  const box = document.createElement("div");
  const b = document.createElement("b");
  b.textContent = JPEG.tipHead(m % map.mcus_across, Math.floor(m / map.mcus_across));
  box.append(b);
  const line = (text: string): void => {
    const d = document.createElement("div");
    d.textContent = text;
    box.append(d);
  };
  line(JPEG.tipBits(map.mcu_bits[m] ?? 0, whole));
  const mcus = map.mcus_across * map.mcus_down;
  const per = mcus === 0 ? 0 : map.block_channel.length / mcus;
  if (per > 1) {
    const by = new Map<number, [number, number]>();
    for (let i = m * per; i < (m + 1) * per; i++) {
      const c = map.block_channel[i] ?? 0;
      const [bits, n] = by.get(c) ?? [0, 0];
      by.set(c, [bits + (map.block_bits[i] ?? 0), n + 1]);
    }
    for (const [c, [bits, n]] of by) line(JPEG.tipChannel(map.channels[c]?.name ?? "", bits, n));
  }
  return box;
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
    p.textContent = JPEG.unnamedBlock;
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

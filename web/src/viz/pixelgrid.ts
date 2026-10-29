// Pixel grid linked to the bytes, with block and pass overlays.
//
// Candidate `pixel-grid-linked` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent E of the first batch (section 5.3). The picture of an image
// file on a canvas that zooms about the pointer and pans under a drag, drawn
// pixel for pixel with no smoothing, so that a pixel can be pointed at, and
// with what made each pixel drawn over it:
// - a JPEG's 8 by 8 blocks and its MCUs as lines, the bits each block or MCU
//   took as a heat layer, one channel at a time or all of them, and the
//   restart intervals as alternating shades;
// - a PNG's Adam7 pass as a tint over each pixel, and the filter of every
//   scanline in a strip down the left, a column per pass.
// Hovering a pixel says where it is, what it holds and what coded it; a click
// selects the bytes that made it (the JPEG MCU, or the block whose bits are
// shown; the PNG pixel's samples), and a field of the JPEG scan selected
// anywhere else is outlined where it lands in the picture. At a zoom where a
// pixel is big enough, each one shows its value.
//
// A PNG's pixels and scanlines are fields of the unfiltered image data, not
// of the file, and the page does not yet tell the shared selection
// (`linked.ts`) about a field outside the file. Until it does, the outline of
// a PNG selection made elsewhere (`light` below) is never drawn.
//
// It replaces three figures that each did part of this: the content card
// (`contentcard.ts`), which showed the picture with no zoom and no way back to
// the bytes; the report's map of bits per MCU (`report/jpeg.ts`), which was
// beside the picture rather than over it and could not zoom; and the report's
// PNG figures (`report/pngscanlines.ts`), which drew the passes and the
// filters apart from the picture and from each other. The content card and
// both report sections draw it; it registers nothing of its own.
//
// What the overlays read is the core's own decode, never a second one done
// here: the JPEG trace (`Doc.jpegScan`, `Doc.jpegBlock`) and the PNG
// scanlines' blocks and unfiltered pixels, found the way the report finds
// them. The picture underneath is the browser's decode, which is why a JPEG
// pixel's colour is labelled as the browser's: the scan stores coefficients,
// not colours. A PNG pixel's samples are read from the template, at their
// stored bit depth.
//
// The arithmetic is `pixelgridmath.ts`, with its tests.

import "./pixelgrid.css";
import type { Doc, JpegScan, TemplateNode } from "../doc.ts";
import { formatAddress, formatOffset } from "../format.ts";
import { VIZ } from "../strings.ts";
import { bitCount } from "../report/jpegtext.ts";
import { ok } from "../report/model.ts";
import { FILTERS, numberOf, pictureRow, type Scanline } from "../report/pngpasses.ts";
import { readScanlines, type Found, type Header } from "../report/pngscanlines.ts";
import { WAIT } from "../report/section.ts";
import { RV } from "../report/text.ts";
import { walkTemplate } from "../report/walk.ts";
import { board, type Board } from "./board.ts";
import { IDENTITY, type View } from "./boardmath.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { goToBits, goToField } from "./pick.ts";
import {
  blockAt,
  blockBits,
  blockSize,
  cellPx,
  codedSize,
  filterColumns,
  HEAT_CLASSES,
  heatClass,
  heatStep,
  hexSample,
  indexScan,
  LABEL_PX,
  LABEL_LINE_PX,
  labelFits,
  magnifyCap,
  mcuAt,
  mcuBits,
  mcuSize,
  mcusInBits,
  pixelAt,
  placePicture,
  restartInterval,
  scanlineKey,
  scanlineOf,
  scanlinePixels,
  VIEW_PAD_PX,
  visiblePixels,
  type Placement,
  type ScanIndex,
} from "./pixelgridmath.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

// ----- what is drawn over the picture -----

export type JpegLayer = { readonly node: TemplateNode; readonly scan: JpegScan; readonly index: ScanIndex };

export type PngLayer = {
  readonly header: Header;
  /** The scanlines read, which is all of them up to the report's limit. */
  readonly lines: readonly Scanline[];
  readonly total: number;
  readonly pixels: TemplateNode | null;
  readonly byKey: ReadonlyMap<string, Scanline>;
  readonly byPath: ReadonlyMap<string, Scanline>;
};

export type Layers = { readonly jpeg: JpegLayer | null; readonly png: PngLayer | null };

const NO_LAYERS: Layers = { jpeg: null, png: null };

const layerCache = new WeakMap<Doc, { readonly at: number; readonly layers: Layers }>();

/**
 * What can be drawn over this document's picture, or `WAIT` while the bytes
 * or the decode it needs are on their way. Kept per document until an edit,
 * since finding a scan walks the template and indexing one sums every block.
 * Only the first JPEG scan is drawn over the picture, and only a baseline
 * one: the core traces no other kind.
 */
export function readLayers(doc: Doc): Layers | typeof WAIT {
  const cached = layerCache.get(doc);
  if (cached !== undefined && cached.at === doc.pieceCount) return cached.layers;
  let layers: Layers = NO_LAYERS;
  if (doc.template === "jpeg") {
    const walk = walkTemplate(doc);
    if (walk === WAIT) return WAIT;
    const node = walk.streams.find((s) => s.type === "jpeg scan" && s.refused === null);
    if (node !== undefined) {
      const scan = ok(doc.jpegScan(node.path));
      if (scan === WAIT) return WAIT;
      if (scan !== null && scan.mcu_bits.length > 0) layers = { jpeg: { node, scan, index: indexScan(scan) }, png: null };
    }
  } else if (doc.template === "png") {
    const found = readScanlines(doc);
    if (found === WAIT) return WAIT;
    if (found.kind === "read") layers = { jpeg: null, png: pngLayer(found) };
  }
  layerCache.set(doc, { at: doc.pieceCount, layers });
  return layers;
}

/** A PNG's scanlines as the grid draws them, indexed by pass and row and by
 *  path. For the report, which has read them already. */
export function pngLayer(found: Extract<Found, { kind: "read" }>): PngLayer {
  const byKey = new Map<string, Scanline>();
  const byPath = new Map<string, Scanline>();
  for (const l of found.lines) {
    byKey.set(scanlineKey(l.pass, l.row), l);
    byPath.set(l.path.join("."), l);
  }
  return { header: found.header, lines: found.lines, total: found.total, pixels: found.pixels, byKey, byPath };
}

export type PixelGridData = {
  readonly width: number;
  readonly height: number;
  /** The browser's decode of the picture, or null where there is none to
   *  draw (the overlays are drawn alone). */
  readonly image: CanvasImageSource | null;
  /** Null while they are still being read. */
  readonly layers: Layers | null;
};

export type PixelGridMode = "picture" | "blocks" | "bits" | "restart" | "passes";

/** What the reader chose, kept by the host so a grid drawn again (the
 *  listing draws its rows again as it scrolls) comes back as it was left. */
export type PixelGridState = {
  mode: PixelGridMode | null;
  /** The channel the bits are shown for, by place in the scan's channels;
   *  null for every channel at once, which is bits per MCU. */
  channel: number | null;
  /** Of the overlay, 0 to 1. */
  opacity: number;
  view: View | null;
};

export function gridState(): PixelGridState {
  return { mode: null, channel: null, opacity: 1, view: null };
}

/** The overlays that make sense for these layers, in the order offered. */
export function modesFor(layers: Layers | null): PixelGridMode[] {
  const out: PixelGridMode[] = ["picture"];
  const j = layers?.jpeg;
  if (j != null) {
    out.push("blocks", "bits");
    if (j.scan.restart_interval > 0 && j.scan.mcu_bits.length > j.scan.restart_interval) out.push("restart");
  }
  if (layers?.png?.header.interlaced === true) out.push("passes");
  return out;
}

/** What a click on the picture landed on. */
export type PixelHit = {
  readonly x: number;
  readonly y: number;
  /** False for the padding a JPEG codes past the picture's edge. */
  readonly inPicture: boolean;
  readonly mcu: number | null;
  /** The block under the pointer of the channel whose bits are shown, in
   *  Bits mode with one channel picked; null otherwise, where a click is on
   *  the whole MCU. */
  readonly block: number | null;
};

export type PixelGridOptions = {
  readonly width: number;
  readonly height: number;
  readonly state: PixelGridState;
  /** The overlay to start on when the reader has not chosen one. */
  readonly defaultMode?: PixelGridMode;
  /** What a click on a pixel does, in place of going to its bytes. */
  readonly onPick?: (hit: PixelHit) => void;
};

export type PixelGrid = { readonly el: HTMLElement; readonly board: Board; destroy(): void };

// ----- sizes -----

/** Each column of the filter strip, and the space round the strip. */
const STRIP_COL_PX = 4;
const STRIP_ONE_PX = 10;
const STRIP_PAD_PX = 3;
/** Lines are drawn round blocks and MCUs only once they are this many screen
 *  pixels across: closer than that, the lines are all there is to see. */
const GRID_MIN_PX = 6;
/** The largest one pixel of the picture may get. */
const MAX_CELL_PX = 96;
/** Values are written in at most this many cells, however large the view. */
const MAX_LABELS = 4000;
/** Outlines drawn at most for a selection; past this the run is outlined as
 *  one box. */
const MAX_OUTLINES = 2000;

/** The pixels the grid draws for a `w` by `h` picture: the picture, or the
 *  whole MCUs a JPEG scan codes, which run past its right and bottom edges.
 *  What a host sizes the view by. */
export function gridArea(w: number, h: number, layers: Layers | null): { w: number; h: number } {
  const j = layers?.jpeg;
  if (j == null) return { w, h };
  const c = codedSize(j.scan);
  return { w: Math.max(w, c.w), h: Math.max(h, c.h) };
}

/** How wide the strip down the left of a PNG is, or 0 for no strip. */
export function stripWidth(layers: Layers | null): number {
  const png = layers?.png;
  if (png == null) return 0;
  const cols = filterColumns(png.header.interlaced);
  return 2 * STRIP_PAD_PX + (cols === 1 ? STRIP_ONE_PX : cols * STRIP_COL_PX);
}

// ----- colours -----

type Palette = {
  readonly seq: readonly string[];
  readonly filter: readonly string[];
  readonly accent: string;
  readonly fg: string;
  readonly bg: string;
  readonly panel: string;
  readonly line: string;
  readonly checkA: string;
  readonly checkB: string;
};

function palette(el: Element): Palette {
  const s = getComputedStyle(el);
  const v = (name: string, fallback: string): string => s.getPropertyValue(name).trim() || fallback;
  return {
    seq: [...Array(HEAT_CLASSES).keys()].map((i) => v(`--viz-seq-${i}`, "#808080")),
    filter: FILTERS.map((_, i) => v(`--viz-filter-${i}`, "#808080")),
    accent: v("--accent", "#2457c5"),
    fg: v("--fg", "#000"),
    bg: v("--bg", "#fff"),
    panel: v("--panel", "#eee"),
    line: v("--line", "#ccc"),
    checkA: v("--viz-check-a", "#fff"),
    checkB: v("--viz-check-b", "#ddd"),
  };
}

type Rgb = readonly [number, number, number];

function rgb(hex: string): [number, number, number] {
  const n = parseInt(hex.replace("#", ""), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** Colour `a` drawn at opacity `alpha` over colour `b`. */
function mix(a: Rgb, b: Rgb, alpha: number): Rgb {
  return [a[0] * alpha + b[0] * (1 - alpha), a[1] * alpha + b[1] * (1 - alpha), a[2] * alpha + b[2] * (1 - alpha)];
}

/** Black or white, whichever has the more contrast with colour `c`, by the
 *  WCAG relative luminance. */
function inkOn(c: Rgb): string {
  const lin = (v: number): number => {
    const x = v / 255;
    return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  };
  const l = 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
  // Against white (1.05 / (l + 0.05)) and black ((l + 0.05) / 0.05), equal
  // where l + 0.05 is the square root of 0.0525.
  return l + 0.05 > Math.sqrt(0.0525) ? "#000" : "#fff";
}

/** The class colour of the pass that stores a pixel: earlier passes darker,
 *  as in the report's tile, since an earlier pass holds fewer pixels. */
function passClass(pass: number): number {
  return Math.max(0, Math.min(HEAT_CLASSES - 1, 7 - pass));
}

function passColour(pal: Palette, pass: number): string {
  return pal.seq[passClass(pass)] ?? "#808080";
}

/** The most bits any one MCU took, or any one block of `channel`, and the
 *  legend's step for that. */
function heatRange(j: JpegLayer, channel: number | null): { step: number; max: number } {
  const s = j.scan;
  let max = 0;
  if (channel === null) for (const b of s.mcu_bits) max = Math.max(max, b);
  else for (let i = 0; i < s.block_channel.length; i++) if (s.block_channel[i] === channel) max = Math.max(max, s.block_bits[i] ?? 0);
  return { step: heatStep(max), max };
}

// ----- small canvases drawn scaled -----

function smallCanvas(w: number, h: number, paint: (img: ImageData) => void): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = Math.max(1, w);
  c.height = Math.max(1, h);
  const ctx = c.getContext("2d");
  if (ctx !== null) {
    const img = ctx.createImageData(c.width, c.height);
    paint(img);
    ctx.putImageData(img, 0, 0);
  }
  return c;
}

/** Bits per block of one channel, one canvas pixel to a block, or bits per
 *  MCU for every channel at once. */
function heatCanvas(j: JpegLayer, channel: number | null, pal: Palette): { canvas: HTMLCanvasElement; step: number; max: number } {
  const s = j.scan;
  const colours = pal.seq.map(rgb);
  const { step, max } = heatRange(j, channel);
  if (channel === null) {
    const canvas = smallCanvas(s.mcus_across, s.mcus_down, (img) => {
      for (let m = 0; m < s.mcu_bits.length; m++) {
        const c = colours[heatClass(s.mcu_bits[m] ?? 0, step)] ?? [0, 0, 0];
        img.data.set([c[0], c[1], c[2], 255], m * 4);
      }
    });
    return { canvas, step, max };
  }
  const g = j.index.grids[channel];
  const canvas = smallCanvas(g?.across ?? 1, g?.down ?? 1, (img) => {
    if (g == null) return;
    for (let k = 0; k < g.index.length; k++) {
      const i = g.index[k] ?? -1;
      if (i < 0) continue;
      const c = colours[heatClass(s.block_bits[i] ?? 0, step)] ?? [0, 0, 0];
      img.data.set([c[0], c[1], c[2], 255], k * 4);
    }
  });
  return { canvas, step, max };
}

/** One canvas pixel to an MCU, shaded by whether its restart interval is an
 *  odd or an even one. */
function restartCanvas(s: JpegScan, pal: Palette): HTMLCanvasElement {
  const light = rgb(pal.seq[1] ?? "#888");
  const dark = rgb(pal.seq[5] ?? "#444");
  return smallCanvas(s.mcus_across, s.mcus_down, (img) => {
    for (let m = 0; m < s.mcu_bits.length; m++) {
      const c = (restartInterval(s, m) ?? 0) % 2 === 0 ? light : dark;
      img.data.set([c[0], c[1], c[2], 255], m * 4);
    }
  });
}

/** Adam7's 8 by 8 tile, each pixel the colour of its pass. Drawn as a
 *  repeating pattern, so a picture of any size costs one tile. */
function passTile(pal: Palette): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = 8;
  c.height = 8;
  const ctx = c.getContext("2d");
  if (ctx !== null) {
    for (let y = 0; y < 8; y++) {
      for (let x = 0; x < 8; x++) {
        ctx.fillStyle = passColour(pal, scanlineOf(true, x, y).pass ?? 7);
        ctx.fillRect(x, y, 1, 1);
      }
    }
  }
  return c;
}

/** Two squares of each grey: the checks behind a picture with transparent
 *  pixels, at a fixed size on screen. */
function checkTile(pal: Palette): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = 16;
  c.height = 16;
  const ctx = c.getContext("2d");
  if (ctx !== null) {
    ctx.fillStyle = pal.checkA;
    ctx.fillRect(0, 0, 16, 16);
    ctx.fillStyle = pal.checkB;
    ctx.fillRect(0, 0, 8, 8);
    ctx.fillRect(8, 8, 8, 8);
  }
  return c;
}

// ----- reading values -----

/** A pixel's stored samples, by name, at the picture's bit depth. */
type Samples = { readonly names: readonly string[]; readonly values: readonly number[]; readonly offsetBits: number; readonly space: number; readonly path: readonly number[]; readonly sizeBits: number };

/** The samples of PNG pixel `i`, from the list of unfiltered pixels; null
 *  while the bytes are on their way or for a list that does not read. */
function pngSamples(doc: Doc, list: TemplateNode, i: number, cache: Map<number, Samples>): Samples | null {
  const hit = cache.get(i);
  if (hit !== undefined) return hit;
  const r = doc.templateChildren(list.path, i, i + 1);
  if (r.status !== "ok") return null;
  const n = r.node[0];
  if (n === undefined) return null;
  let names: string[] = [];
  let values: number[] = [];
  if (n.composite && n.child_count > 0) {
    const k = doc.templateChildren(n.path, 0, n.child_count);
    if (k.status !== "ok") return null;
    names = k.node.map((c) => c.name);
    values = k.node.map((c) => numberOf(c) ?? 0);
  } else {
    names = [n.name];
    values = [numberOf(n) ?? 0];
  }
  const s: Samples = { names, values, offsetBits: n.offset_bits, space: n.space, path: n.path, sizeBits: n.size_bits };
  cache.set(i, s);
  return s;
}

/** Reads the browser's decode back, a region at a time: never the whole
 *  picture at once, which for a large photograph is hundreds of megabytes. */
function readback(image: CanvasImageSource): (x: number, y: number, w: number, h: number) => Uint8ClampedArray | null {
  let ctx: CanvasRenderingContext2D | null = null;
  const c = document.createElement("canvas");
  return (x, y, w, h) => {
    if (w <= 0 || h <= 0) return null;
    if (c.width < w || c.height < h) {
      c.width = Math.max(c.width, w);
      c.height = Math.max(c.height, h);
      ctx = null;
    }
    ctx ??= c.getContext("2d", { willReadFrequently: true });
    if (ctx === null) return null;
    ctx.clearRect(0, 0, w, h);
    ctx.drawImage(image, x, y, w, h, 0, 0, w, h);
    try {
      return ctx.getImageData(0, 0, w, h).data;
    } catch {
      return null;
    }
  };
}

function hex2(n: number): string {
  return n.toString(16).padStart(2, "0");
}

// ----- the drawing -----

type Rect = { readonly x: number; readonly y: number; readonly w: number; readonly h: number };

export function drawPixelGrid(doc: Doc, data: PixelGridData, opts: PixelGridOptions): PixelGrid {
  const width = Math.max(80, Math.round(opts.width));
  const height = Math.max(60, Math.round(opts.height));
  const state = opts.state;
  const layers = data.layers;
  const jpeg = layers?.jpeg ?? null;
  const png = layers?.png ?? null;
  const modes = modesFor(layers);
  if (state.mode === null || !modes.includes(state.mode)) {
    state.mode = opts.defaultMode !== undefined && modes.includes(opts.defaultMode) ? opts.defaultMode : "picture";
  }
  if (jpeg !== null && state.channel !== null && !jpeg.scan.scan.includes(state.channel)) state.channel = null;

  // The picture's own size, and the size the overlays cover: a JPEG codes
  // whole MCUs, which run past the right and bottom edges.
  const picW = data.width;
  const picH = data.height;
  const coded = jpeg === null ? { w: picW, h: picH } : codedSize(jpeg.scan);
  const { w: areaW, h: areaH } = gridArea(picW, picH, layers);
  const strip = stripWidth(layers);
  const left = strip === 0 ? 0 : strip + 4;
  const base = placePicture(areaW, areaH, width - left - 2 * VIEW_PAD_PX, height - 2 * VIEW_PAD_PX, magnifyCap(areaW, areaH));
  // Against the left edge rather than centred: next to the filter strip, so
  // a scanline's mark and its row are read across in one look, and in the
  // same place in the card whatever the card's width.
  const place: Placement = { scale: base.scale, x0: left + VIEW_PAD_PX, y0: base.y0 + VIEW_PAD_PX };

  const fig = document.createElement("figure");
  fig.className = "viz-pixgrid";

  // ----- controls -----
  const bar = document.createElement("div");
  bar.className = "viz-pixgrid-bar";
  const modeRow = document.createElement("div");
  modeRow.className = "viz-pixgrid-modes";
  modeRow.setAttribute("role", "group");
  modeRow.setAttribute("aria-label", VIZ.pixelGrid.modesLabel);
  const modeButtons = new Map<PixelGridMode, HTMLButtonElement>();
  if (modes.length > 1) {
    for (const m of modes) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = VIZ.pixelGrid.modes[m];
      b.addEventListener("click", () => {
        state.mode = m;
        refreshControls();
        paint();
      });
      modeButtons.set(m, b);
      modeRow.append(b);
    }
    bar.append(modeRow);
  }
  // The channel the bits are counted for, in bits-per-block mode.
  const channelField = document.createElement("label");
  channelField.className = "viz-pixgrid-field";
  const channelSelect = document.createElement("select");
  if (jpeg !== null && jpeg.scan.scan.length > 1) {
    const all = document.createElement("option");
    all.value = "";
    all.textContent = VIZ.pixelGrid.allChannels;
    channelSelect.append(all);
    for (const c of jpeg.scan.scan) {
      const o = document.createElement("option");
      o.value = String(c);
      o.textContent = jpeg.scan.channels[c]?.name ?? String(c);
      channelSelect.append(o);
    }
    channelSelect.value = state.channel === null ? "" : String(state.channel);
    channelSelect.addEventListener("change", () => {
      state.channel = channelSelect.value === "" ? null : Number(channelSelect.value);
      heat = null;
      refreshControls();
      paint();
    });
    channelField.append(VIZ.pixelGrid.channel, channelSelect);
    bar.append(channelField);
  }
  const opacityField = document.createElement("label");
  opacityField.className = "viz-pixgrid-field";
  const opacity = document.createElement("input");
  opacity.type = "range";
  opacity.min = "0";
  opacity.max = "100";
  opacity.value = String(Math.round(state.opacity * 100));
  opacity.addEventListener("input", () => {
    state.opacity = Number(opacity.value) / 100;
    paint();
  });
  opacityField.append(VIZ.pixelGrid.opacity, opacity);
  if (modes.length > 1) bar.append(opacityField);
  const zoomRow = document.createElement("div");
  zoomRow.className = "viz-pixgrid-zoom";
  const actual = document.createElement("button");
  actual.type = "button";
  actual.textContent = VIZ.pixelGrid.actualSize;
  const reset = document.createElement("button");
  reset.type = "button";
  reset.textContent = VIZ.resetZoom;
  // Only a picture drawn smaller than it is has an actual size to zoom to.
  if (place.scale < 1) zoomRow.append(actual);
  zoomRow.append(reset);
  bar.append(zoomRow);

  // ----- the canvas -----
  const canvas = document.createElement("canvas");
  canvas.className = "viz-pixgrid-canvas";
  const dpr = typeof devicePixelRatio === "number" && devicePixelRatio > 0 ? devicePixelRatio : 1;
  canvas.width = Math.round(width * dpr);
  canvas.height = Math.round(height * dpr);
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", VIZ.pixelGrid.alt(picW, picH));

  const legend = document.createElement("div");
  legend.className = "viz-pixgrid-legend";
  const note = document.createElement("p");
  note.className = "viz-pixgrid-note";
  fig.append(bar, canvas, legend, note);

  // ----- state of the drawing -----
  let view: View = IDENTITY;
  let pal: Palette | null = null;
  let heat: { canvas: HTMLCanvasElement; step: number; max: number; channel: number | null } | null = null;
  let restart: HTMLCanvasElement | null = null;
  let passes: CanvasPattern | null = null;
  let checks: CanvasPattern | null = null;
  let hover: Rect | null = null;
  let lit: Rect[] = [];
  const samples = new Map<number, Samples>();
  const read = data.image === null ? null : readback(data.image);
  const bus = linkedFor(doc);

  const refreshControls = (): void => {
    for (const [m, b] of modeButtons) b.setAttribute("aria-pressed", String(m === state.mode));
    channelField.hidden = state.mode !== "bits";
    opacityField.hidden = state.mode === "picture" || state.mode === "blocks";
    drawLegend();
  };

  const drawLegend = (): void => {
    const items: (Node | string)[] = [];
    const key = (colour: string, text: string): HTMLElement => {
      const k = document.createElement("span");
      k.className = "viz-pixgrid-key";
      const sw = document.createElement("span");
      sw.className = "viz-pixgrid-swatch";
      sw.style.background = colour;
      k.append(sw, text);
      return k;
    };
    const lead = (text: string): HTMLElement => {
      const s = document.createElement("span");
      s.textContent = text;
      return s;
    };
    // Swatches take the colours as CSS variables, so they are right before
    // the figure is on the page and follow the theme without a redraw.
    if (state.mode === "bits" && jpeg !== null) {
      const h = heatRange(jpeg, state.channel);
      const name = state.channel === null ? null : (jpeg.scan.channels[state.channel]?.name ?? "");
      items.push(lead(name === null ? VIZ.pixelGrid.heatAll : VIZ.pixelGrid.heatChannel(name)));
      for (let i = 0; i < HEAT_CLASSES; i++) {
        const from = i * h.step;
        // The top class takes every count above it; its label ends at the
        // largest count there is.
        const to = i === HEAT_CLASSES - 1 ? Math.max(from, h.max) : (i + 1) * h.step - 1;
        if (from > h.max && i > 0) break;
        items.push(key(`var(--viz-seq-${i})`, VIZ.pixelGrid.heatClass(from, to)));
      }
    } else if (state.mode === "passes") {
      items.push(lead(VIZ.pixelGrid.passesLead));
      for (let pass = 1; pass <= 7; pass++) items.push(key(`var(--viz-seq-${passClass(pass)})`, String(pass)));
    } else if (state.mode === "restart" && jpeg !== null) {
      items.push(lead(VIZ.pixelGrid.restartLead(jpeg.scan.restart_interval)));
    } else if (state.mode === "blocks" && jpeg !== null) {
      const c = gridChannel();
      const { bw, bh } = blockSize(jpeg.scan, c);
      const { mw, mh } = mcuSize(jpeg.scan);
      items.push(lead(bw === mw && bh === mh ? VIZ.pixelGrid.blocksSame(mw, mh) : VIZ.pixelGrid.blocksLead(jpeg.scan.channels[c]?.name ?? "", bw, bh, mw, mh)));
    }
    if (png !== null) {
      const counts = FILTERS.map(() => 0);
      for (const l of png.lines) counts[l.filter] = (counts[l.filter] ?? 0) + 1;
      if (items.length > 0) items.push(document.createElement("br"));
      items.push(lead(VIZ.pixelGrid.filtersLead));
      FILTERS.forEach((name, i) => {
        if ((counts[i] ?? 0) > 0) items.push(key(`var(--viz-filter-${i})`, VIZ.pixelGrid.filterItem(name, counts[i] ?? 0)));
      });
    }
    legend.replaceChildren(...items);
    legend.hidden = items.length === 0;
    // The sentence under the legend says what the strip is, and what was
    // left out of it; or that the overlays are still being read.
    let text = "";
    if (layers === null) text = doc.template === "png" ? VIZ.pixelGrid.readingScanlines : VIZ.pixelGrid.readingScan;
    else if (png !== null) {
      text = png.header.interlaced ? VIZ.pixelGrid.stripPasses : VIZ.pixelGrid.stripOne;
      if (png.lines.length < png.total) text += ` ${VIZ.pixelGrid.stripCut(png.lines.length, png.total)}`;
    }
    note.textContent = text;
    note.hidden = text === "";
  };

  /** The channel whose blocks the grid lines are drawn round: the one whose
   *  bits are shown, or the scan's first. */
  const gridChannel = (): number => (jpeg === null ? 0 : state.mode === "bits" && state.channel !== null ? state.channel : (jpeg.scan.scan[0] ?? 0));

  /** Screen position of picture pixel (x, y), and the size of one pixel. */
  const sx = (x: number): number => (place.x0 + x * place.scale) * view.k + view.x;
  const sy = (y: number): number => (place.y0 + y * place.scale) * view.k + view.y;

  let frame = 0;
  let painted = false;
  const paint = (): void => {
    cancelAnimationFrame(frame);
    // Painted once the figure is on the page, since the colours are the
    // page's variables and a canvas not yet in it has none.
    if (!fig.isConnected) {
      frame = requestAnimationFrame(paint);
      return;
    }
    painted = true;
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    pal ??= palette(fig);
    const p = pal;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.imageSmoothingEnabled = false;
    ctx.fillStyle = p.panel;
    ctx.fillRect(0, 0, width, height);
    const c = cellPx(place, view);
    const ox = sx(0);
    const oy = sy(0);

    // The checks show through transparent pixels; then the picture, one
    // block of colour per pixel at any zoom.
    if (data.image !== null) {
      checks ??= ctx.createPattern(checkTile(p), "repeat");
      if (checks !== null) {
        ctx.fillStyle = checks;
        ctx.fillRect(ox, oy, picW * c, picH * c);
      }
      ctx.drawImage(data.image, ox, oy, picW * c, picH * c);
    }

    // The overlay.
    ctx.globalAlpha = state.opacity;
    if (state.mode === "bits" && jpeg !== null) {
      if (heat === null || heat.channel !== state.channel) heat = { ...heatCanvas(jpeg, state.channel, p), channel: state.channel };
      if (state.channel === null) {
        const { mw, mh } = mcuSize(jpeg.scan);
        ctx.drawImage(heat.canvas, ox, oy, jpeg.scan.mcus_across * mw * c, jpeg.scan.mcus_down * mh * c);
      } else {
        const { bw, bh } = blockSize(jpeg.scan, state.channel);
        ctx.drawImage(heat.canvas, ox, oy, heat.canvas.width * bw * c, heat.canvas.height * bh * c);
      }
    } else if (state.mode === "restart" && jpeg !== null) {
      restart ??= restartCanvas(jpeg.scan, p);
      const { mw, mh } = mcuSize(jpeg.scan);
      ctx.drawImage(restart, ox, oy, jpeg.scan.mcus_across * mw * c, jpeg.scan.mcus_down * mh * c);
    } else if (state.mode === "passes" && png !== null) {
      passes ??= ctx.createPattern(passTile(p), "repeat");
      if (passes !== null) {
        passes.setTransform(new DOMMatrix().translate(ox, oy).scale(c));
        ctx.fillStyle = passes;
        ctx.fillRect(ox, oy, picW * c, picH * c);
      }
    }
    ctx.globalAlpha = 1;

    // Lines round the blocks and the MCUs, once they are far enough apart
    // to see the pixels between them.
    if (jpeg !== null && state.mode !== "picture") {
      const vis = visiblePixels(place, view, width, height, areaW, areaH);
      const lines = (stepX: number, stepY: number, colour: string, lineW: number): void => {
        if (stepX * c < GRID_MIN_PX || stepY * c < GRID_MIN_PX) return;
        // An odd width is centred on a half pixel, an even one on a whole
        // one, so that either covers whole screen pixels and stays sharp.
        const half = lineW % 2 === 1 ? 0.5 : 0;
        ctx.beginPath();
        const top = sy(0);
        const bottom = sy(coded.h);
        const leftX = sx(0);
        const rightX = sx(coded.w);
        for (let x = Math.floor(vis.x0 / stepX) * stepX; x <= Math.min(coded.w, vis.x1); x += stepX) {
          const X = Math.round(sx(x)) + half;
          ctx.moveTo(X, Math.max(0, top));
          ctx.lineTo(X, Math.min(height, bottom));
        }
        for (let y = Math.floor(vis.y0 / stepY) * stepY; y <= Math.min(coded.h, vis.y1); y += stepY) {
          const Y = Math.round(sy(y)) + half;
          ctx.moveTo(Math.max(0, leftX), Y);
          ctx.lineTo(Math.min(width, rightX), Y);
        }
        ctx.strokeStyle = colour;
        ctx.lineWidth = lineW;
        ctx.stroke();
      };
      const { bw, bh } = blockSize(jpeg.scan, gridChannel());
      const { mw, mh } = mcuSize(jpeg.scan);
      // Blocks in thin lines and MCUs in thick ones, as the legend says.
      const split = bw !== mw || bh !== mh;
      ctx.globalAlpha = 0.35;
      if (split) lines(bw, bh, p.fg, 1);
      ctx.globalAlpha = 0.6;
      lines(mw, mh, p.fg, split ? 2 : 1);
      ctx.globalAlpha = 1;
      // The edge of the picture, where the coded padding starts.
      if (coded.w > picW || coded.h > picH) {
        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = p.fg;
        ctx.strokeRect(Math.round(ox) + 0.5, Math.round(oy) + 0.5, Math.round(picW * c), Math.round(picH * c));
        ctx.setLineDash([]);
      }
    }

    // Each pixel's value, where every channel of it fits its cell.
    drawLabels(ctx, c, p);

    // The field selected elsewhere, and what the pointer is on.
    // Each outline has a ring of the page's own colour round it, so it shows
    // on a pixel of any colour, the accent's own included.
    if (lit.length > 0) {
      for (const [colour, w] of [
        [p.bg, 4],
        [p.accent, 2],
      ] as const) {
        ctx.strokeStyle = colour;
        ctx.lineWidth = w;
        for (const r of lit) ctx.strokeRect(sx(r.x), sy(r.y), Math.max(2, r.w * c), Math.max(2, r.h * c));
      }
    }
    if (hover !== null) {
      ctx.strokeStyle = p.bg;
      ctx.lineWidth = 3;
      ctx.strokeRect(sx(hover.x), sy(hover.y), Math.max(2, hover.w * c), Math.max(2, hover.h * c));
      ctx.strokeStyle = p.fg;
      ctx.lineWidth = 1.5;
      ctx.strokeRect(sx(hover.x), sy(hover.y), Math.max(2, hover.w * c), Math.max(2, hover.h * c));
    }

    // The filter strip, over anything panned under it.
    if (png !== null) drawStrip(ctx, c, p);
  };

  /** The overlay's colour at picture pixel (x, y) as the current mode paints
   *  it, or null where nothing is painted over the pixel. */
  let seqRgb: Rgb[] | null = null;
  const overlayAt = (x: number, y: number): Rgb | null => {
    const p = pal;
    if (p === null) return null;
    seqRgb ??= p.seq.map(rgb);
    const seq = seqRgb;
    if (state.mode === "bits" && jpeg !== null && heat !== null) {
      const s = jpeg.scan;
      if (state.channel === null) {
        const m = mcuAt(s, x, y);
        return m === null ? null : (seq[heatClass(s.mcu_bits[m] ?? 0, heat.step)] ?? null);
      }
      const i = blockAt(s, jpeg.index, state.channel, x, y);
      return i === null ? null : (seq[heatClass(s.block_bits[i] ?? 0, heat.step)] ?? null);
    }
    if (state.mode === "restart" && jpeg !== null) {
      const m = mcuAt(jpeg.scan, x, y);
      return m === null ? null : (seq[(restartInterval(jpeg.scan, m) ?? 0) % 2 === 0 ? 1 : 5] ?? null);
    }
    if (state.mode === "passes" && png !== null) return seq[passClass(scanlineOf(true, x, y).pass ?? 7)] ?? null;
    return null;
  };

  const drawLabels = (ctx: CanvasRenderingContext2D, c: number, p: Palette): void => {
    const vis = visiblePixels(place, view, width, height, picW, picH);
    const count = (vis.x1 - vis.x0) * (vis.y1 - vis.y0);
    if (count === 0 || count > MAX_LABELS) return;
    const pngList = png?.pixels ?? null;
    const bits = png?.header.bitDepth ?? 8;
    // How many lines a value takes: the stored samples of a PNG pixel, or the
    // red, green and blue of the browser's decode.
    let channels = 3;
    if (pngList !== null) {
      const first = pngSamples(doc, pngList, vis.y0 * picW + vis.x0, samples);
      if (first === null) return;
      channels = first.values.length;
    }
    const digits = pngList !== null ? hexSample(0, bits).length : 2;
    if (!labelFits(c, channels, digits)) return;
    const decoded = read?.(vis.x0, vis.y0, vis.x1 - vis.x0, vis.y1 - vis.y0) ?? null;
    const across = vis.x1 - vis.x0;
    ctx.font = `${LABEL_PX}px ${getComputedStyle(fig).getPropertyValue("--mono") || "monospace"}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    for (let y = vis.y0; y < vis.y1; y++) {
      for (let x = vis.x0; x < vis.x1; x++) {
        let lines: string[];
        if (pngList !== null) {
          const s = pngSamples(doc, pngList, y * picW + x, samples);
          if (s === null) continue;
          lines = s.values.map((v) => hexSample(v, bits));
        } else if (decoded !== null) {
          const k = ((y - vis.y0) * across + (x - vis.x0)) * 4;
          lines = [hex2(decoded[k] ?? 0), hex2(decoded[k + 1] ?? 0), hex2(decoded[k + 2] ?? 0)];
        } else continue;
        // Dark text on a light cell and light on a dark one, by the colour
        // the cell was painted: the pixel with the overlay mixed over it at
        // the overlay's opacity. A mostly transparent pixel shows the checks,
        // so the page's own text colour, unless the overlay mostly hides them.
        let under: Rgb | null = null;
        if (decoded !== null) {
          const k = ((y - vis.y0) * across + (x - vis.x0)) * 4;
          if ((decoded[k + 3] ?? 255) >= 128) under = [decoded[k] ?? 0, decoded[k + 1] ?? 0, decoded[k + 2] ?? 0];
        }
        const over = state.opacity > 0 ? overlayAt(x, y) : null;
        const cell = over === null ? under : under === null ? (state.opacity >= 0.5 ? over : null) : mix(over, under, state.opacity);
        const ink = cell === null ? p.fg : inkOn(cell);
        const cx = sx(x) + c / 2;
        const top = sy(y) + c / 2 - ((lines.length - 1) * LABEL_LINE_PX) / 2;
        // A value is written whole or not at all: one that would run under
        // the filter strip or off the edge of the view is left to the card.
        const half = (digits * LABEL_PX * 0.6) / 2;
        if (cx - half < (strip === 0 ? 0 : strip + 1) || cx + half > width) continue;
        if (top - LABEL_LINE_PX / 2 < 0 || top + (lines.length - 0.5) * LABEL_LINE_PX > height) continue;
        ctx.fillStyle = ink;
        lines.forEach((t, i) => ctx.fillText(t, cx, top + i * LABEL_LINE_PX));
      }
    }
  };

  const drawStrip = (ctx: CanvasRenderingContext2D, c: number, p: Palette): void => {
    if (png === null) return;
    ctx.fillStyle = p.panel;
    ctx.fillRect(0, 0, strip, height);
    ctx.fillStyle = p.line;
    ctx.fillRect(strip, 0, 1, height);
    const one = !png.header.interlaced;
    const colW = one ? STRIP_ONE_PX : STRIP_COL_PX;
    for (const l of png.lines) {
      const row = pictureRow(l);
      const y0 = sy(row);
      if (y0 > height) continue;
      const y1 = Math.max(y0 + 1, sy(row + 1));
      if (y1 < 0) continue;
      const col = one ? 0 : (l.pass ?? 1) - 1;
      ctx.fillStyle = p.filter[l.filter] ?? p.fg;
      ctx.fillRect(STRIP_PAD_PX + col * colW, y0, colW - 1, y1 - y0);
    }
  };

  // ----- pointing -----

  type Hit = { kind: "pixel"; hit: PixelHit } | { kind: "line"; line: Scanline } | null;

  const hitAt = (e: MouseEvent): Hit => {
    const r = canvas.getBoundingClientRect();
    const px = e.clientX - r.left;
    const py = e.clientY - r.top;
    if (png !== null && px < strip) {
      // The scanline nearest the pointer in the strip's column for that
      // pass, so a one-pixel mark is still easy to point at.
      const cols = filterColumns(png.header.interlaced);
      const colW = cols === 1 ? STRIP_ONE_PX : STRIP_COL_PX;
      const col = Math.max(0, Math.min(cols - 1, Math.floor((px - STRIP_PAD_PX) / colW)));
      const pass = png.header.interlaced ? col + 1 : null;
      const y = Math.floor((py - sy(0)) / cellPx(place, view));
      let best: Scanline | null = null;
      let bestD = Infinity;
      for (const l of png.lines) {
        if (l.pass !== pass) continue;
        const d = Math.abs(sy(pictureRow(l) + 0.5) - py);
        if (d < bestD) {
          best = l;
          bestD = d;
        }
      }
      return best !== null && (bestD <= 12 || Math.abs(pictureRow(best) - y) < 1) ? { kind: "line", line: best } : null;
    }
    const at = pixelAt(place, view, px, py, areaW, areaH);
    if (at === null) return null;
    const inPicture = at.x < picW && at.y < picH;
    if (jpeg === null) return inPicture ? { kind: "pixel", hit: { ...at, inPicture, mcu: null, block: null } } : null;
    const mcu = mcuAt(jpeg.scan, at.x, at.y);
    if (mcu === null) return inPicture ? { kind: "pixel", hit: { ...at, inPicture, mcu: null, block: null } } : null;
    const block = state.channel !== null && state.mode === "bits" ? blockAt(jpeg.scan, jpeg.index, state.channel, at.x, at.y) : null;
    return { kind: "pixel", hit: { ...at, inPicture, mcu, block } };
  };

  /** What the pointer is on, outlined: the block whose bits are shown, the
   *  MCU when every channel's are, or the pixel. */
  const outlineOf = (h: Hit): Rect | null => {
    if (h === null) return null;
    if (h.kind === "line") {
      const s = scanlinePixels(picW, picH, h.line.pass, h.line.row);
      return s === null ? null : { x: 0, y: s.y, w: picW, h: 1 };
    }
    const { x, y } = h.hit;
    if (jpeg !== null && state.mode !== "picture" && h.hit.mcu !== null) {
      if (state.mode === "bits" && state.channel !== null) {
        const { bw, bh } = blockSize(jpeg.scan, state.channel);
        return { x: Math.floor(x / bw) * bw, y: Math.floor(y / bh) * bh, w: bw, h: bh };
      }
      const { mw, mh } = mcuSize(jpeg.scan);
      return { x: Math.floor(x / mw) * mw, y: Math.floor(y / mh) * mh, w: mw, h: mh };
    }
    return { x, y, w: 1, h: 1 };
  };

  const tipFor = (h: Hit): HTMLElement | null => {
    if (h === null) return null;
    if (h.kind === "line") {
      const l = h.line;
      return tipCard(VIZ.pixelGrid.scanline(l.pass, l.row, FILTERS[l.filter] ?? "?"), png?.header.interlaced === true ? [RV.pngLinePictureRow(pictureRow(l))] : [], RV.pngLineBytes(l.bytes));
    }
    const { x, y, inPicture } = h.hit;
    const lines: string[] = [];
    let where = "";
    if (!inPicture) lines.push(VIZ.pixelGrid.paddingNote);
    // What the pixel holds: its stored samples for a PNG, and otherwise the
    // colour the browser drew it in.
    const list = png?.pixels ?? null;
    if (inPicture && list !== null) {
      const s = pngSamples(doc, list, y * picW + x, samples);
      if (s !== null) {
        lines.push(VIZ.pixelGrid.samples(s.names.map((n, i) => `${n} ${s.values[i] ?? 0}`)));
        where = VIZ.pixelGrid.inUnfiltered(formatAddress(s.offsetBits, s.space));
      }
    } else if (inPicture && read !== null) {
      const d = read(x, y, 1, 1);
      if (d !== null) lines.push(VIZ.pixelGrid.decoded(`${hex2(d[0] ?? 0)}${hex2(d[1] ?? 0)}${hex2(d[2] ?? 0)}`));
    }
    if (png !== null && inPicture) {
      const { pass, row } = scanlineOf(png.header.interlaced, x, y);
      const l = png.byKey.get(scanlineKey(pass, row));
      lines.push(VIZ.pixelGrid.scanline(pass, row, l === undefined ? "?" : (FILTERS[l.filter] ?? "?")));
    }
    if (jpeg !== null && h.hit.mcu !== null) {
      const s = jpeg.scan;
      const m = h.hit.mcu;
      const mb = mcuBits(jpeg.index, m);
      lines.push(VIZ.pixelGrid.mcu(m % s.mcus_across, Math.floor(m / s.mcus_across), bitCount(s.mcu_bits[m] ?? 0)));
      // The block of each channel that coded the pixel, so the card says
      // what each channel spent on this place in the picture.
      for (const c of s.scan) {
        const i = blockAt(s, jpeg.index, c, x, y);
        if (i === null) continue;
        lines.push(VIZ.pixelGrid.block(s.channels[c]?.name ?? "", s.block_x[i] ?? 0, s.block_y[i] ?? 0, bitCount(s.block_bits[i] ?? 0)));
      }
      const n = restartInterval(s, m);
      if (n !== null) lines.push(VIZ.pixelGrid.interval(n, Math.ceil(s.mcu_bits.length / s.restart_interval)));
      // Where the bits a click selects start: the block's whose bits are
      // shown, or else the MCU's. Their count is on the line above.
      if (h.hit.block !== null) {
        const c = s.block_channel[h.hit.block] ?? 0;
        where = VIZ.pixelGrid.blockStarts(s.channels[c]?.name ?? "", formatOffset(blockBits(s, jpeg.index, h.hit.block).startBit));
      } else where = VIZ.pixelGrid.mcuStarts(formatOffset(mb.startBit));
    }
    const title = inPicture ? VIZ.pixelGrid.pixel(x, y) : VIZ.pixelGrid.padding(x, y);
    return tipCard(title, lines, where);
  };

  /** The bits the pointer is over, in the file, for the hex view to mark. */
  const hoverBits = (h: Hit): { startBit: number; endBit: number } | null => {
    if (h === null || h.kind !== "pixel" || jpeg === null || h.hit.mcu === null || jpeg.node.space !== 0) return null;
    if (h.hit.block !== null) return blockBits(jpeg.scan, jpeg.index, h.hit.block);
    return mcuBits(jpeg.index, h.hit.mcu);
  };

  const stopTip = hoverTip(canvas, (e) => {
    const h = hitAt(e);
    const next = outlineOf(h);
    if (!sameRect(next, hover)) {
      hover = next;
      paint();
    }
    bus.publish({ hover: hoverBits(h) });
    return tipFor(h);
  });
  const leave = (): void => {
    if (hover !== null) {
      hover = null;
      paint();
    }
    bus.publish({ hover: null });
  };
  canvas.addEventListener("pointerleave", leave);

  const goToPixel = (hit: PixelHit): void => {
    if (jpeg !== null && hit.mcu !== null) {
      // The block whose bits are shown, or else the whole MCU: the bits the
      // hover card's last line counts and the hex view marked under the
      // pointer.
      if (hit.block === null) {
        const bits = mcuBits(jpeg.index, hit.mcu);
        const b = ok(doc.jpegBlock(jpeg.node.path, hit.mcu * jpeg.index.perMcu));
        if (b !== null && b !== WAIT) goToField(doc, { path: b.mcu_path, ...bits });
        else goToBits(doc, [bits]);
        return;
      }
      const b = ok(doc.jpegBlock(jpeg.node.path, hit.block));
      const first = b !== null && b !== WAIT ? b.codes[0] : undefined;
      const last = b !== null && b !== WAIT ? b.codes[b.codes.length - 1] : undefined;
      if (b !== null && b !== WAIT && first !== undefined && last !== undefined) goToField(doc, { path: b.path, startBit: first.start_bit, endBit: last.end_bit });
      else goToBits(doc, [blockBits(jpeg.scan, jpeg.index, hit.block)]);
      return;
    }
    const list = png?.pixels ?? null;
    if (list !== null && hit.inPicture) {
      const s = pngSamples(doc, list, hit.y * picW + hit.x, samples);
      if (s !== null) goToField(doc, { path: s.path, startBit: s.offsetBits, endBit: s.offsetBits + s.sizeBits });
    }
  };

  const click = (e: MouseEvent): void => {
    const h = hitAt(e);
    if (h === null) return;
    e.stopPropagation();
    if (h.kind === "line") {
      const n = ok(doc.templateNode(h.line.path));
      if (n !== null && n !== WAIT) goToField(doc, { path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits });
      return;
    }
    if (opts.onPick !== undefined) opts.onPick(h.hit);
    else goToPixel(h.hit);
  };
  canvas.addEventListener("click", click);
  // A picture with nothing behind it to go to says so by its cursor.
  if (jpeg === null && png?.pixels == null) canvas.style.cursor = "grab";

  // ----- the selection made elsewhere -----

  const light = (): void => {
    const sel = bus.state.selection;
    lit = [];
    if (sel === null) return;
    if (png !== null) {
      const list = png.pixels;
      if (list !== null && startsWith(sel.path, list.path) && sel.path.length > list.path.length) {
        const i = sel.path[list.path.length] ?? 0;
        lit = [{ x: i % picW, y: Math.floor(i / picW), w: 1, h: 1 }];
        return;
      }
      for (const [, l] of png.byPath) {
        if (!startsWith(sel.path, l.path)) continue;
        const s = scanlinePixels(picW, picH, l.pass, l.row);
        if (s === null) return;
        lit = s.dx === 1 ? [{ x: 0, y: s.y, w: picW, h: 1 }] : [...Array(s.count).keys()].map((k) => ({ x: s.x0 + k * s.dx, y: s.y, w: 1, h: 1 }));
        return;
      }
      return;
    }
    if (jpeg === null) return;
    // Bits of the space the scan is read in, from a field that is not the
    // scan itself or anything holding it: a block, a code, a run of the hex.
    const n = ok(doc.templateNode(sel.path));
    if (n === null || n === WAIT || n.space !== jpeg.node.space) return;
    if (startsWith(jpeg.node.path, sel.path)) return;
    const s = jpeg.scan;
    const { from, to } = mcusInBits(jpeg.index, sel.startBit, sel.endBit);
    if (to <= from) return;
    const { mw, mh } = mcuSize(s);
    if (to - from <= 4) {
      // A few MCUs: the blocks inside them that the bits meet.
      for (let m = from; m < to; m++) {
        for (let i = m * jpeg.index.perMcu; i < (m + 1) * jpeg.index.perMcu; i++) {
          const b = blockBits(s, jpeg.index, i);
          if (b.endBit <= sel.startBit || b.startBit >= Math.max(sel.endBit, sel.startBit + 1)) continue;
          const c = s.block_channel[i] ?? 0;
          const { bw, bh } = blockSize(s, c);
          lit.push({ x: (s.block_x[i] ?? 0) * bw, y: (s.block_y[i] ?? 0) * bh, w: bw, h: bh });
        }
      }
      if (lit.length > 0) return;
    }
    if (to - from > MAX_OUTLINES) {
      const y0 = Math.floor(from / s.mcus_across);
      const y1 = Math.floor((to - 1) / s.mcus_across) + 1;
      lit = [{ x: 0, y: y0 * mh, w: s.mcus_across * mw, h: (y1 - y0) * mh }];
      return;
    }
    for (let m = from; m < to; m++) lit.push({ x: (m % s.mcus_across) * mw, y: Math.floor(m / s.mcus_across) * mh, w: mw, h: mh });
  };
  light();
  const stopLinked = subscribeWhileConnected(bus, fig, (_state, changed) => {
    if (changed !== "selection") return;
    light();
    paint();
  });

  // The theme changes the colours, which the canvas has to be told.
  const scheme = typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)") : null;
  const onScheme = (): void => {
    if (!fig.isConnected && painted) {
      scheme?.removeEventListener("change", onScheme);
      return;
    }
    pal = null;
    seqRgb = null;
    heat = null;
    restart = null;
    passes = null;
    checks = null;
    drawLegend();
    paint();
  };
  scheme?.addEventListener("change", onScheme);

  // ----- zoom -----
  const saved = state.view;
  // The board says its starting view as it is made, before the saved one is
  // put back; only what comes after that is the reader's.
  let made = false;
  const zoom = board(canvas, {
    width,
    height,
    content: { x0: place.x0, y0: place.y0, x1: place.x0 + areaW * place.scale, y1: place.y0 + areaH * place.scale },
    maxScale: Math.max(2, MAX_CELL_PX / place.scale),
    onZoom: (v) => {
      view = v;
      if (made) state.view = v;
      reset.disabled = v.k === 1 && v.x === 0 && v.y === 0;
      paint();
    },
  });
  made = true;
  if (saved !== null && (saved.k !== 1 || saved.x !== 0 || saved.y !== 0)) {
    // The view the reader left, as the box of layout it showed.
    zoom.zoomTo({ x0: -saved.x / saved.k, y0: -saved.y / saved.k, x1: (width - saved.x) / saved.k, y1: (height - saved.y) / saved.k });
  }
  reset.addEventListener("click", () => zoom.home(true));
  actual.addEventListener("click", () => {
    const k = 1 / (place.scale * zoom.view().k);
    zoom.zoomBy(k);
  });

  refreshControls();
  paint();
  return {
    el: fig,
    board: zoom,
    destroy: () => {
      cancelAnimationFrame(frame);
      zoom.destroy();
      stopTip();
      stopLinked();
      hideTip();
      canvas.removeEventListener("pointerleave", leave);
      canvas.removeEventListener("click", click);
      scheme?.removeEventListener("change", onScheme);
    },
  };
}

function sameRect(a: Rect | null, b: Rect | null): boolean {
  if (a === null || b === null) return a === b;
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

function startsWith(path: readonly number[], prefix: readonly number[]): boolean {
  return prefix.length <= path.length && prefix.every((s, i) => path[i] === s);
}

/** The pixel grid is drawn by its hosts (the content card and the report's
 *  JPEG and PNG sections) rather than through a registry, so there is
 *  nothing to add here. */
export function register(): void {}

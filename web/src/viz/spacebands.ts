// File offsets against memory addresses, as bands between two columns.
//
// Candidate `file-vs-memory-bands` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent C2 of the first batch (section 5.3). A tab of its own in the
// rail (`overviewtabs.ts`), offered for ELF and PE files. Test samples:
// elf/busybox-x86_64, pe/busybox-w32-x86_64.exe.
//
// What it is for: the listing says where each section header points, one
// header at a time, and nothing on screen says how the loader puts the file
// together in memory. Here the file runs down the left column to scale and
// memory down the right, and a band joins each section's bytes to its
// addresses, coloured by its permissions in memory (executable, writable,
// read-only), with the `rwx` letters on the band where there is room. An
// ELF stretch has the permissions of the LOAD segment that maps it, as the
// loader goes by the program headers; a PE section has its own. A band that
// fans out is alignment slack; a
// `.bss` widens out of nothing on the memory side; the section names and
// debug data have no band, because they are never loaded. The program
// headers that do the loading are brackets beside both columns.
//
// How it is read and explored:
// - The wheel zooms about the pointer and a drag pans, both columns together
//   by default (a band stays a band). With "Zoom both columns together" off,
//   each column has its own zoom, so a 16-byte section can be opened up on
//   one side while the other still shows the whole; the bands then shear,
//   which is why it is not the default. Each half of the drawing is its own
//   `board.ts` surface, and in linked mode one follows the other.
// - Hover shows the one card (`tip.ts`): name, permissions and the program
//   header they are from, the section flags, the file offset and size, the
//   address and size, and what a click selects. A click selects what the
//   card names, even where two stretches share a pixel. Hovering a section lights its bytes in the hex view
//   (`linked.ts`), and the field the cursor is in lights its band.
// - A click on the file side or a band selects the bytes; on the memory side
//   it selects the header entry that put the section there (`pick.ts`).
// - Names sit outside the columns on rows (`placeLabels`), with a leader to
//   what they name, cut with an ellipsis to fit (`fit.ts`), and dropped where
//   no row is free; the hover card always has them. The `rwx` letters on the
//   bands are measured the same way and dropped where they would touch.
// - The wheel and the drag answer only over the columns' height, not over
//   the column names and readouts above and below it: zooming about a point
//   outside the picture would push the picture off the drawing.
//
// The layout arithmetic is `spacebandslayout.ts`, with its tests. This module
// reads the Doc (`readPicture`), draws at the size it is given
// (`drawSpaceBands`) and makes the rail's panel (`register`).

import type { Doc, TemplateNode, TemplateReply } from "../doc.ts";
import { formatBytes, formatOffset } from "../format.ts";
import { bitSizeText, TREEMAP, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { applyY, invertY, type View, IDENTITY } from "./boardmath.ts";
import { canvasMeasure, ellipsise, monoMeasure, type Measure } from "./fit.ts";
import { assignLanes } from "./lanes.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { registerOverviewTab, type OverviewTabPanel } from "./overviewtabs.ts";
import { goToBits, goToField, nearestTarget, type HitTarget } from "./pick.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";
import {
  ACCESS_ORDER,
  axisAt,
  axisY,
  bandAt,
  bandEdges,
  bandPath,
  BRACKET_LANE_PX,
  columnsFor,
  elfPicture,
  enumWord,
  fileAxis,
  memoryAxis,
  pePicture,
  placeLabels,
  regionAtByte,
  sectionCounts,
  type Access,
  type Axis,
  type Band,
  type Columns,
  type Cut,
  type ElfSectionIn,
  type ElfSegmentIn,
  type EntryPick,
  type Picture,
  type Range,
  type Region,
} from "./spacebandslayout.ts";

const S = VIZ.spaceBands;

// ----- reading the Doc -----

/** Most fields read of one structure: a header has a few dozen. */
const MOST_FIELDS = 64;
/** Most sections or segments drawn. A file with more is drawn with these. */
const MOST_ENTRIES = 4096;

type Fields = ReadonlyMap<string, TemplateNode>;

/** A structure's fields by name. By name rather than by place, since a
 *  32-bit ELF or a PE32 puts some of the same fields in another order. */
function fieldsOf(doc: Doc, path: readonly number[]): TemplateReply<Fields> {
  const kids = doc.templateChildren(path, 0, MOST_FIELDS);
  if (kids.status !== "ok") return kids;
  return { status: "ok", node: new Map(kids.node.map((k) => [k.name, k])) };
}

/** The entries of a table, past the one-child wrapper an `at → X[]` field
 *  has: `program_headers` holds the array, and the array the entries. */
function entriesOf(doc: Doc, node: TemplateNode): TemplateReply<readonly TemplateNode[]> {
  let at = node;
  for (let depth = 0; depth < 3 && !at.list && at.child_count === 1; depth++) {
    const one = doc.templateChildren(at.path, 0, 1);
    if (one.status !== "ok") return one;
    const inner = one.node[0];
    if (inner === undefined) break;
    at = inner;
  }
  return doc.templateChildren(at.path, 0, Math.min(at.child_count, MOST_ENTRIES));
}

function pickOf(n: TemplateNode | undefined): EntryPick | null {
  return n === undefined ? null : { path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits };
}

function num(f: Fields, name: string): number {
  const v = Number(f.get(name)?.value ?? Number.NaN);
  return Number.isFinite(v) ? v : 0;
}

/** Which formats this piece reads. */
function readsFormat(doc: Doc): boolean {
  return doc.template === "elf" || doc.template === "pe";
}

class Wait {
  constructor(readonly reply: Exclude<TemplateReply<unknown>, { status: "ok" }>) {}
}

/** The reply's node, or a `Wait` thrown to the caller of `readPicture` with
 *  why it cannot be read yet. One way out of a dozen nested reads. */
function need<T>(reply: TemplateReply<T>): T {
  if (reply.status !== "ok") throw new Wait(reply);
  return reply.node;
}

/** The field `name`, or a `Wait` with `missing`, the reader's words for
 *  what is not there. */
function needField(f: Fields, name: string, missing: string): TemplateNode {
  const n = f.get(name);
  if (n === undefined) throw new Wait({ status: "error", message: missing });
  return n;
}

export type PictureReply = TemplateReply<Picture>;

/** The stretches of the file the Doc holds, read from its headers. */
export function readPicture(doc: Doc): PictureReply {
  try {
    if (doc.template === "elf") return { status: "ok", node: readElf(doc) };
    if (doc.template === "pe") return { status: "ok", node: readPe(doc) };
    return { status: "error", message: S.missing.format };
  } catch (e) {
    if (e instanceof Wait) return e.reply as PictureReply;
    throw e;
  }
}

function readElf(doc: Doc): Picture {
  const root = need(fieldsOf(doc, []));
  const headerNode = needField(root, "header", S.missing.elfHeader);
  const h = need(fieldsOf(doc, headerNode.path));
  const phTable = h.get("program_headers");
  const shTable = h.get("section_headers");
  const bodiesNode = h.get("sections");
  const contents = need(doc.elfContents(0));
  // Section bodies are read on demand and may still be on the way while the
  // headers are already here; a click falls back to the bytes until they are.
  const bodies = bodiesNode === undefined ? null : doc.templateChildren(bodiesNode.path, 0, Math.min(bodiesNode.child_count, MOST_ENTRIES));
  const bodyAt = (i: number, offset: number): EntryPick | null => {
    if (bodies === null || bodies.status !== "ok") return null;
    const b = bodies.node[i];
    return b !== undefined && b.offset_bits === offset * 8 ? pickOf(b) : null;
  };
  const sections: ElfSectionIn[] = contents.sections.map((s, index) => {
    const entry = need(doc.templateNode(s.path));
    const f = need(fieldsOf(doc, s.path));
    return {
      index, name: s.name, type: s.kind, flags: f.get("flags")?.value ?? "", address: s.address, offset: s.offset, size: s.size,
      entry: pickOf(entry), body: bodyAt(index, s.offset),
    };
  });
  const segments: ElfSegmentIn[] = [];
  if (phTable !== undefined) {
    for (const [index, e] of need(entriesOf(doc, phTable)).entries()) {
      const f = need(fieldsOf(doc, e.path));
      segments.push({
        index, type: enumWord(f.get("type")?.value ?? ""), flags: f.get("flags")?.value ?? "",
        offset: num(f, "offset"), vaddr: num(f, "virtual_address"), fileSize: num(f, "file_size"), memSize: num(f, "memory_size"),
        entry: pickOf(e),
      });
    }
  }
  return elfPicture({
    fileBytes: doc.lengthBytes,
    headerSize: num(h, "header_size") || 64,
    headerEntry: { path: [], startBit: 0, endBit: (num(h, "header_size") || 64) * 8 },
    phoff: num(h, "program_header_offset"),
    phnum: num(h, "program_header_count"),
    phentsize: num(h, "program_header_entry_size"),
    phEntry: pickOf(phTable),
    shoff: num(h, "section_header_offset"),
    shnum: num(h, "section_header_count"),
    shentsize: num(h, "section_header_entry_size"),
    shEntry: pickOf(shTable),
    sections,
    segments,
  });
}

function readPe(doc: Doc): Picture {
  const root = need(fieldsOf(doc, []));
  const pe = need(fieldsOf(doc, needField(root, "pe", S.missing.peHeader).path));
  const optional = need(fieldsOf(doc, needField(pe, "optional", S.missing.optionalHeader).path));
  const addresses = need(fieldsOf(doc, needField(optional, "addresses", S.missing.imageBase).path));
  const table = needField(pe, "sections", S.missing.sectionTable);
  const bodiesNode = pe.get("section_data");
  const bodies = bodiesNode === undefined ? null : doc.templateChildren(bodiesNode.path, 0, Math.min(bodiesNode.child_count, MOST_ENTRIES));
  const sections = need(entriesOf(doc, table)).map((e, index) => {
    const f = need(fieldsOf(doc, e.path));
    const rawOffset = num(f, "raw_offset");
    const b = bodies !== null && bodies.status === "ok" ? bodies.node[index] : undefined;
    return {
      index, name: f.get("name")?.value ?? "", virtualSize: num(f, "virtual_size"), virtualAddress: num(f, "virtual_address"),
      rawSize: num(f, "raw_size"), rawOffset, flags: f.get("characteristics")?.value ?? "",
      entry: pickOf(e), body: b !== undefined && b.offset_bits === rawOffset * 8 ? pickOf(b) : null,
    };
  });
  const headersSize = num(addresses, "headers_size");
  return pePicture({
    fileBytes: doc.lengthBytes,
    imageBase: num(addresses, "image_base"),
    headersSize,
    headersEntry: headersSize > 0 ? { path: [], startBit: 0, endBit: Math.min(headersSize, doc.lengthBytes) * 8 } : null,
    imageBaseEntry: pickOf(addresses.get("image_base")),
    sections,
  });
}

// ----- words -----

/** A virtual address. Plain hex, without the `@` a file offset has: the two
 *  columns are different spaces, and `0x400130` of memory is not `@0x400130`
 *  of the file. */
export function addressText(a: number): string {
  return `0x${Math.max(0, Math.round(a)).toString(16)}`;
}

function sizeText(bytes: number): string {
  const exact = bitSizeText(bytes * 8);
  return bytes < 1024 ? exact : `${formatBytes(bytes)} (${exact})`;
}

// ----- drawing -----

const SVG = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

/** The two rows over the columns (their names, then the first address in
 *  view) and the one under them (the last address in view). */
const HEAD_H = 34;
const FOOT_H = 18;
/** A name's size and its row: the listing's small text. */
const NAME_PX = 11;
const ROW_H = 15;
const NAME_MIN_CHARS = 4;
/** The addresses over and under the columns, in the monospace stack. */
const READOUT_PX = 10;
const READOUT_MEASURE = monoMeasure();
/** How thick a band has to be where the letters go for its `rwx` letters. */
const PERMS_MIN_PX = 14;
/** How wide one byte may get at the deepest zoom. */
const MAX_BYTE_PX = 4;

let measure: Measure | null = null;
function measurer(): Measure {
  measure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return measure;
}

function accessClass(a: Access): string {
  return `mmap-${a}`;
}

export type SpaceBandsOptions = {
  readonly width: number;
  readonly height: number;
  /** Whether the wheel zooms both columns at once. */
  readonly linked: boolean;
};

export type SpaceBands = {
  readonly el: SVGSVGElement;
  /** Both columns back to no zoom. */
  home(): void;
  /** Zoom the columns together or each on its own. */
  setLinked(linked: boolean): void;
  /** Whether either column is zoomed. */
  zoomed(): boolean;
  /** Called after every zoom, for the panel's Reset button. */
  onZoom: () => void;
  destroy(): void;
};

/** What the pointer is on. */
type Hit =
  | { readonly kind: "region"; readonly region: Region; readonly side: "file" | "memory" | "band" }
  | { readonly kind: "segment"; readonly region: Region }
  | { readonly kind: "cut"; readonly cut: Cut }
  | { readonly kind: "file-at"; readonly at: number }
  | { readonly kind: "memory-at"; readonly at: number };

/** Draw the picture `width` by `height` pixels. Never measures anything. */
export function drawSpaceBands(doc: Doc, pic: Picture, opts: SpaceBandsOptions): SpaceBands {
  const width = Math.max(120, opts.width);
  const height = Math.max(160, opts.height);
  const top = HEAD_H;
  const bottom = height - FOOT_H;
  const length = bottom - top;

  // Segment brackets in lanes, so two that overlap (a LOAD segment and the
  // RELRO stretch inside it) never share one.
  const fileLanes = assignLanes(pic.segments, (s) => (s.file === null ? { start: 0, end: 0 } : { start: s.file.start, end: s.file.end }));
  const memLanes = assignLanes(pic.segments, (s) => (s.mem === null ? { start: 0, end: 0 } : { start: s.mem.start, end: s.mem.end }));
  const lanes = Math.min(3, Math.max(pic.segments.length > 0 ? 1 : 0, fileLanes.count, memLanes.count));
  const cols: Columns = columnsFor(width, lanes);

  const fAxis = fileAxis(pic.fileBytes, length);
  const memRanges = [...pic.regions, ...pic.segments].flatMap((r) => (r.mem === null ? [] : [r.mem]));
  const mAxis = memoryAxis(memRanges, length);
  const bigLabelsR = cols.namesR.end - cols.namesR.start >= 90;

  const root = svg("svg", { width, height, class: "mmap-svg", role: "img" });
  root.setAttribute("aria-label", S.title);
  const clipId = `mmap-clip-${Math.random().toString(36).slice(2)}`;
  const defs = svg("defs");
  const clip = svg("clipPath", { id: clipId });
  clip.append(svg("rect", { x: 0, y: top, width, height: length }));
  defs.append(clip);
  const plot = svg("g", { "clip-path": `url(#${clipId})`, class: "mmap-plot" });
  const head = svg("g", { class: "mmap-head" });
  // Two surfaces for the wheel and the drag, one per half, drawn over
  // everything so the pictures under them never take the pointer. They
  // cover the columns' height only, so the wheel always zooms about a point
  // of the picture.
  const hitL = svg("rect", { x: 0, y: top, width: cols.middle, height: length, class: "mmap-hit" });
  const hitR = svg("rect", { x: cols.middle, y: top, width: width - cols.middle, height: length, class: "mmap-hit" });
  root.append(defs, plot, head, hitL, hitR);

  let fileView: View = IDENTITY;
  let memView: View = IDENTITY;
  let linked = opts.linked;
  let hovered: string | null = null;
  const bus = linkedFor(doc);
  let selected: string | null = null;
  /** What the hover card shows and where the pointer was, so a click there
   *  selects what the card names. Two stretches can share one pixel at no
   *  zoom, and a click's position is rounded where the pointer's is not. */
  let carded: { x: number; y: number; hit: Hit | null } | null = null;

  const fy = (byte: number): number => applyY(fileView, top + axisY(fAxis, byte));
  const my = (addr: number): number => applyY(memView, top + axisY(mAxis, addr));

  /** What was drawn last, for the hit test. */
  let fileBoxes: HitTarget<Region>[] = [];
  let memBoxes: HitTarget<Region>[] = [];
  let bracketBoxes: HitTarget<Region>[] = [];
  let bands: { band: Band; region: Region }[] = [];
  let namesL: HitTarget<Region>[] = [];
  let namesR: HitTarget<Region | Cut>[] = [];
  let cutBoxes: HitTarget<Cut>[] = [];

  /** Every part of a region carries its key, so lighting one is a class. */
  const state = (r: Region): string => (r.key === selected ? " is-on" : r.key === hovered ? " is-hover" : "");

  const draw = (): void => {
    const parts: SVGElement[] = [];
    fileBoxes = [];
    memBoxes = [];
    bracketBoxes = [];
    bands = [];
    cutBoxes = [];
    // The columns' ground: the whole file, and each mapped stretch of memory.
    const fileTop = fy(0);
    const fileBot = fy(pic.fileBytes);
    parts.push(svg("rect", { x: cols.file.start, y: fileTop, width: cols.file.end - cols.file.start, height: Math.max(1, fileBot - fileTop), class: "mmap-ground" }));
    for (const p of mAxis.pieces) {
      const y0 = applyY(memView, top + p.y0);
      const y1 = applyY(memView, top + p.y1);
      parts.push(svg("rect", { x: cols.memory.start, y: y0, width: cols.memory.end - cols.memory.start, height: Math.max(1, y1 - y0), class: "mmap-ground" }));
    }
    // A cut is a zigzag across the memory column at each of its edges.
    for (const c of mAxis.cuts) {
      const y0 = applyY(memView, top + c.y0);
      const y1 = applyY(memView, top + c.y1);
      cutBoxes.push({ x0: cols.memory.start, x1: cols.memory.end, y0, y1, value: c });
      parts.push(svg("path", { d: zigzag(cols.memory.start - 3, cols.memory.end + 3, y0 + 3) + zigzag(cols.memory.start - 3, cols.memory.end + 3, y1 - 3), class: "mmap-cut" }));
    }

    // Bands first, under the boxes they join.
    for (const r of pic.regions) {
      if (r.mem === null) continue;
      const fileAt = r.file ?? (r.filePoint !== null ? { start: r.filePoint, end: r.filePoint } : null);
      if (fileAt === null) continue;
      const band: Band = { xa: cols.file.end, xb: cols.memory.start, fa: fy(fileAt.start), fb: fy(fileAt.end), ma: my(r.mem.start), mb: my(r.mem.end) };
      if (Math.max(band.fb, band.mb) < top || Math.min(band.fa, band.ma) > bottom) continue;
      bands.push({ band, region: r });
      parts.push(svg("path", { d: bandPath(band), class: `mmap-band ${accessClass(r.access)}${state(r)}`, "data-k": r.key }));
    }
    drawPerms(parts);
    const box = (r: Region, range: Range, x: Range, y: (n: number) => number, into: HitTarget<Region>[]): void => {
      const y0 = y(range.start);
      const y1 = y(range.end);
      if (y1 < top - 2 || y0 > bottom + 2) return;
      into.push({ x0: x.start, x1: x.end, y0, y1, value: r });
      const h = Math.max(1, y1 - y0);
      parts.push(svg("rect", { x: x.start, y: y0, width: x.end - x.start, height: h, class: `mmap-box ${accessClass(r.access)}${h >= 4 ? " has-gap" : ""}${state(r)}`, "data-k": r.key }));
    };
    for (const r of pic.regions) {
      if (r.file !== null) box(r, r.file, cols.file, fy, fileBoxes);
      if (r.mem !== null) box(r, r.mem, cols.memory, my, memBoxes);
    }
    // The segments' brackets, one lane each beside both columns.
    pic.segments.forEach((s, i) => {
      const bracket = (range: Range | null, strip: Range, lane: number, outward: 1 | -1, y: (n: number) => number): void => {
        if (range === null || lane >= lanes) return;
        const y0 = y(range.start);
        const y1 = y(range.end);
        if (y1 < top || y0 > bottom) return;
        // Lane 0 sits against the column; later lanes step outward.
        const x = outward === 1 ? strip.start + 1 + lane * BRACKET_LANE_PX : strip.end - 1 - (lane + 1) * BRACKET_LANE_PX;
        bracketBoxes.push({ x0: x, x1: x + BRACKET_LANE_PX - 1, y0, y1, value: s });
        parts.push(svg("rect", { x, y: y0, width: BRACKET_LANE_PX - 2, height: Math.max(1, y1 - y0), class: `mmap-bracket ${accessClass(s.access)}${state(s)}`, "data-k": s.key }));
      };
      bracket(s.file, cols.stripL, fileLanes.lane[i] ?? 0, -1, fy);
      bracket(s.mem, cols.stripR, memLanes.lane[i] ?? 0, 1, my);
    });
    drawNames(parts);
    plot.replaceChildren(...parts);
    drawHead();
  };

  /** The `rwx` letters across the middle of each band thick enough to hold
   *  them, dropped where they would touch letters already placed. */
  const drawPerms = (parts: SVGElement[]): void => {
    const x = (cols.file.end + cols.memory.start) / 2;
    const placed: { y0: number; y1: number }[] = [];
    for (const { band, region } of bands) {
      if (region.perms === null) continue;
      const w = READOUT_MEASURE(region.perms, READOUT_PX);
      if (w > cols.memory.start - cols.file.end - 8) continue;
      const e = bandEdges(band, x);
      const y0 = Math.max(top, e.top);
      const y1 = Math.min(bottom, e.bottom);
      if (y1 - y0 < PERMS_MIN_PX) continue;
      const y = (y0 + y1) / 2;
      const box = { y0: y - READOUT_PX / 2 - 1, y1: y + READOUT_PX / 2 + 1 };
      if (placed.some((p) => box.y0 < p.y1 && box.y1 > p.y0)) continue;
      placed.push(box);
      const t = svg("text", { x, y, "text-anchor": "middle", "dominant-baseline": "central", class: "mmap-perms" });
      t.textContent = region.perms;
      parts.push(t);
    }
  };

  /** Names on rows beside both columns, each with a leader to its box. */
  const drawNames = (parts: SVGElement[]): void => {
    const visible = (y0: number, y1: number): number => (Math.max(top, y0) + Math.min(bottom, y1)) / 2;
    // File side: every stretch with bytes in the file, the bigger first.
    const left = fileBoxes.filter((b) => b.y1 >= top && b.y0 <= bottom);
    const rowsL = placeLabels(left.map((b) => ({ y: visible(b.y0, b.y1), priority: priorityOf(b) })), ROW_H, top, bottom);
    namesL = [];
    left.forEach((b, i) => {
      const row = rowsL[i];
      if (row === null || row === undefined) return;
      const room = cols.namesL.end - cols.namesL.start;
      const text = ellipsise(b.value.name, room, NAME_PX, measurer(), NAME_MIN_CHARS);
      if (text === null) return;
      const at = visible(b.y0, b.y1);
      namesL.push({ x0: cols.namesL.start, x1: cols.stripL.start, y0: row - ROW_H / 2, y1: row + ROW_H / 2, value: b.value });
      parts.push(svg("path", { d: `M${cols.namesL.end + 2},${row}L${cols.stripL.start - 1},${at}L${cols.file.start},${at}`, class: `mmap-leader${state(b.value)}`, "data-k": b.value.key }));
      const t = svg("text", { x: cols.namesL.end, y: row, "text-anchor": "end", "dominant-baseline": "central", class: `mmap-name${state(b.value)}`, "data-k": b.value.key });
      t.textContent = text;
      parts.push(t);
    });
    // Memory side: the stretches with no bytes in the file (which have no
    // name on the left), every stretch when there is room for its name, and
    // the cuts.
    type Right = { y0: number; y1: number; name: string; value: Region | Cut; priority: number };
    const right: Right[] = [];
    for (const b of memBoxes) {
      if (b.y1 < top || b.y0 > bottom) continue;
      if (!bigLabelsR && b.value.file !== null) continue;
      right.push({ y0: b.y0, y1: b.y1, name: b.value.name, value: b.value, priority: priorityOf(b) + (b.value.file === null ? 1e6 : 0) });
    }
    for (const c of cutBoxes) {
      if (c.y1 < top || c.y0 > bottom) continue;
      right.push({ y0: c.y0, y1: c.y1, name: S.cutLabel(formatBytes(c.value.end - c.value.start)), value: c.value, priority: -1 });
    }
    const rowsR = placeLabels(right.map((r) => ({ y: visible(r.y0, r.y1), priority: r.priority })), ROW_H, top, bottom);
    namesR = [];
    right.forEach((r, i) => {
      const row = rowsR[i];
      if (row === null || row === undefined) return;
      const room = cols.namesR.end - cols.namesR.start;
      const isCut = !("key" in r.value);
      // A cut's size is drawn whole or not at all: `2.00 …` would be a
      // different number.
      const text = ellipsise(r.name, room, NAME_PX, measurer(), isCut ? r.name.length : NAME_MIN_CHARS);
      if (text === null) return;
      const at = visible(r.y0, r.y1);
      const st = isCut ? "" : state(r.value as Region);
      const k = isCut ? "" : (r.value as Region).key;
      namesR.push({ x0: cols.memory.end, x1: cols.namesR.end, y0: row - ROW_H / 2, y1: row + ROW_H / 2, value: r.value });
      parts.push(svg("path", { d: `M${cols.memory.end},${at}L${cols.stripR.end + 1},${at}L${cols.namesR.start - 2},${row}`, class: `mmap-leader${st}`, "data-k": k }));
      const t = svg("text", { x: cols.namesR.start, y: row, "dominant-baseline": "central", class: `mmap-name${isCut ? " is-cut" : ""}${st}`, "data-k": k });
      t.textContent = text;
      parts.push(t);
    });
  };

  /** The columns' names, and the first and last address in view of each. */
  const drawHead = (): void => {
    const parts: SVGElement[] = [];
    const text = (x: number, y: number, s: string, anchor: string, cls: string): void => {
      const t = svg("text", { x, y, "text-anchor": anchor, class: cls });
      t.textContent = s;
      parts.push(t);
    };
    text(cols.file.end, 12, S.fileHead, "end", "mmap-colhead");
    text(cols.memory.start, 12, S.memoryHead, "start", "mmap-colhead");
    const fileFrom = Math.max(0, axisAt(fAxis, invertY(fileView, top) - top).at);
    const fileTo = Math.min(pic.fileBytes, axisAt(fAxis, invertY(fileView, bottom) - top).at);
    const memFrom = axisAt(mAxis, invertY(memView, top) - top).at;
    const memTo = axisAt(mAxis, invertY(memView, bottom) - top).at;
    // The file's run left from its column's right edge. Memory's start at its
    // column's left edge, and are pulled back to end at the drawing's right
    // edge where they would run off it, as far as the file's allow; a
    // readout with no room left is left off.
    const fileReadout = (s: string, y: number): number => {
      const w = READOUT_MEASURE(s, READOUT_PX);
      if (cols.file.end - w < 0) return 0;
      text(cols.file.end, y, s, "end", "mmap-readout");
      return cols.file.end;
    };
    const memReadout = (s: string, y: number, after: number): void => {
      const w = READOUT_MEASURE(s, READOUT_PX);
      const x = Math.min(cols.memory.start, width - w);
      if (x < after + 6) return;
      text(x, y, s, "start", "mmap-readout");
    };
    const topEnd = fileReadout(formatOffset(Math.floor(fileFrom) * 8), 28);
    const footEnd = fileReadout(formatOffset(Math.ceil(fileTo) * 8), height - 5);
    if (mAxis.pieces.length > 0) {
      memReadout(addressText(memFrom), 28, topEnd);
      memReadout(addressText(memTo), height - 5, footEnd);
    }
    head.replaceChildren(...parts);
  };

  // ----- pan and zoom -----

  const maxBytes = Math.max(pic.fileBytes, mAxis.end - mAxis.start);
  const maxScale = Math.max(64, (maxBytes / Math.max(1, length)) * MAX_BYTE_PX);
  const content = { x0: 0, y0: top, x1: width, y1: bottom };
  /** Set while one board is told to follow the other, so the follower's own
   *  zoom event does not send the leader after it in turn. */
  let following = false;
  /** The column the reader zoomed last, which leads when the two are linked
   *  again. */
  let lastSide: "file" | "memory" = "file";
  const result: SpaceBands = {
    el: root,
    home: () => {
      boardL?.home(true);
      if (!linked) boardR?.home(true);
    },
    setLinked: (on) => {
      linked = on;
      if (!on || boardL === null || boardR === null) return;
      if (lastSide === "file") follow(boardR, fileView);
      else follow(boardL, memView);
    },
    zoomed: () => fileView.k !== 1 || fileView.y !== 0 || memView.k !== 1 || memView.y !== 0,
    onZoom: () => {},
    destroy: () => {
      boardL?.destroy();
      boardR?.destroy();
      stopTip();
      stopLinked();
      hideTip();
    },
  };
  /** Put a board at a transform, through the one call `board.ts` has for it:
   *  a box exactly the view's size at that zoom, with nothing to pad. */
  const follow = (b: Board, v: View): void => {
    following = true;
    b.zoomTo({ x0: -v.x / v.k, x1: (width - v.x) / v.k, y0: -v.y / v.k, y1: (height - v.y) / v.k });
    following = false;
  };
  const onZoom = (side: "file" | "memory") => (v: View): void => {
    // What the card showed is no longer under the pointer.
    carded = null;
    if (!following) lastSide = side;
    if (side === "file") fileView = v;
    else memView = v;
    if (linked && !following && boardL !== null && boardR !== null) {
      if (side === "file") follow(boardR, v);
      else follow(boardL, v);
    }
    draw();
    result.onZoom();
  };
  // Made one after the other, and each calls `onZoom` once as it is made,
  // so a board may hear from the other before both exist.
  let boardL: Board | null = null;
  let boardR: Board | null = null;
  boardL = board(hitL, { width, height, maxScale, content, onZoom: onZoom("file") });
  boardR = board(hitR, { width, height, maxScale, content, onZoom: onZoom("memory") });

  // ----- pointer -----

  const pointAt = (e: MouseEvent): { x: number; y: number } => {
    const r = root.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const hitAt = (x: number, y: number): Hit | null => {
    if (y < top || y > bottom) return null;
    const near = <T>(targets: readonly HitTarget<T>[]): T | null => nearestTarget(targets, x, y)?.value ?? null;
    if (x < cols.file.start) {
      // The brackets first, over their own lane; then a name, on its row.
      if (x >= cols.stripL.start - 2) {
        const seg = near(bracketBoxes.filter((b) => b.x1 <= cols.file.start));
        if (seg !== null) return { kind: "segment", region: seg };
      }
      const named = near(namesL.filter((n) => y >= n.y0 && y <= n.y1));
      return named === null ? null : { kind: "region", region: named, side: "file" };
    }
    if (x <= cols.file.end) {
      const r = near(fileBoxes);
      if (r !== null) return { kind: "region", region: r, side: "file" };
      return { kind: "file-at", at: axisAt(fAxis, invertY(fileView, y) - top).at };
    }
    if (x < cols.memory.start) {
      const i = bandAt(bands.map((b) => b.band), x, y);
      const b = i === null ? undefined : bands[i];
      return b === undefined ? null : { kind: "region", region: b.region, side: "band" };
    }
    if (x <= cols.memory.end) {
      const c = cutBoxes.find((k) => y >= k.y0 && y < k.y1);
      if (c !== undefined) return { kind: "cut", cut: c.value };
      const r = near(memBoxes);
      if (r !== null) return { kind: "region", region: r, side: "memory" };
      return mAxis.pieces.length === 0 ? null : { kind: "memory-at", at: axisAt(mAxis, invertY(memView, y) - top).at };
    }
    const seg = near(bracketBoxes.filter((b) => b.x0 >= cols.memory.end));
    if (seg !== null && x < cols.namesR.start) return { kind: "segment", region: seg };
    const named = near(namesR.filter((n) => y >= n.y0 && y <= n.y1));
    if (named === null) return null;
    return "key" in named ? { kind: "region", region: named, side: "memory" } : { kind: "cut", cut: named };
  };

  const card = (hit: Hit): HTMLElement => {
    if (hit.kind === "cut") {
      const c = hit.cut;
      return tipCard(S.unmapped, [S.unmappedRange(addressText(c.start), addressText(c.end), sizeText(c.end - c.start)), S.notToScale]);
    }
    if (hit.kind === "file-at") return tipCard(formatOffset(Math.floor(hit.at) * 8), [S.noSection]);
    if (hit.kind === "memory-at") return tipCard(addressText(hit.at), [S.noSection]);
    const r = hit.region;
    const lines: string[] = [];
    if (r.perms !== null) {
      lines.push(r.grant !== null ? S.permsFrom(r.perms, r.grant.index, r.grant.type.toUpperCase()) : r.kind === "section" && pic.segments.length > 0 ? S.permsFromFlags(r.perms) : S.perms(r.perms));
    }
    if (r.flags !== "") lines.push(S.sectionFlags(r.flags));
    if (r.file !== null) lines.push(S.inFile(formatOffset(r.file.start * 8), sizeText(r.file.end - r.file.start)));
    else if (r.mem !== null) lines.push(S.noFileBytes);
    if (r.mem !== null) {
      const at = addressText(r.mem.start);
      const size = sizeText(r.mem.end - r.mem.start);
      lines.push(r.imageBase !== null ? S.inMemoryRva(at, size, addressText(r.mem.start - r.imageBase)) : S.inMemory(at, size));
    } else lines.push(S.notLoaded);
    const side = hit.kind === "segment" ? "memory" : hit.side;
    const where = side === "memory" && r.entryKind !== null ? S.click[r.entryKind] : r.file !== null ? S.click.bytes : r.entryKind !== null ? S.click[r.entryKind] : "";
    return tipCard(r.name, lines, where);
  };

  /** Light the hovered region's parts by their class, without drawing the
   *  picture again: a file with thousands of sections redraws slowly. */
  const relight = (key: string | null): void => {
    if (key === hovered) return;
    const was = hovered;
    hovered = key;
    for (const k of [was, key]) {
      if (k === null) continue;
      for (const el of plot.querySelectorAll(`[data-k="${CSS.escape(k)}"]`)) el.classList.toggle("is-hover", k === key && k !== selected);
    }
  };

  const stopTip = hoverTip(root, (e) => {
    const { x, y } = pointAt(e);
    const hit = hitAt(x, y);
    carded = { x, y, hit };
    const region = hit !== null && (hit.kind === "region" || hit.kind === "segment") ? hit.region : null;
    bus.publish({ hover: region?.file != null ? { startBit: region.file.start * 8, endBit: region.file.end * 8 } : null });
    relight(region?.key ?? null);
    return hit === null ? null : card(hit);
  });
  root.addEventListener("pointerleave", () => {
    carded = null;
    bus.publish({ hover: null });
    relight(null);
  });
  root.addEventListener("click", (e) => {
    const { x, y } = pointAt(e);
    const hit = carded !== null && Math.abs(carded.x - x) <= 1 && Math.abs(carded.y - y) <= 1 ? carded.hit : hitAt(x, y);
    if (hit === null || (hit.kind !== "region" && hit.kind !== "segment")) return;
    const r = hit.region;
    const side = hit.kind === "segment" ? "memory" : hit.side;
    if (side === "memory" && r.entry !== null) goToField(doc, r.entry);
    else if (r.body !== null && r.body.path.length > 0) goToField(doc, r.body);
    else if (r.file !== null) goToBits(doc, [{ startBit: r.file.start * 8, endBit: r.file.end * 8 }]);
    else if (r.entry !== null) goToField(doc, r.entry);
  });

  // The cursor's field lights its section: the one whose header entry the
  // field is in, or else the one whose bytes it is in.
  const selectFrom = (path: readonly number[] | null, startBit: number | null): string | null => {
    if (path === null || startBit === null) return null;
    const all = [...pic.regions, ...pic.segments];
    const byEntry = all.find((r) => r.entry !== null && r.entryKind !== "loading-header" && r.entry.path.length > 0 && r.entry.path.every((s, i) => path[i] === s));
    if (byEntry !== undefined) return byEntry.key;
    return regionAtByte(pic.regions, Math.floor(startBit / 8))?.key ?? null;
  };
  selected = selectFrom(bus.state.selection?.path ?? null, bus.state.selection?.startBit ?? null);
  const stopLinked = subscribeWhileConnected(bus, root, (s, changed) => {
    if (changed !== "selection") return;
    const next = selectFrom(s.selection?.path ?? null, s.selection?.startBit ?? null);
    if (next === selected) return;
    selected = next;
    draw();
  });

  draw();
  return result;
}

/** Bigger stretches name first; a header table is a fact about the format
 *  and gives way to the sections when they compete for a row. */
function priorityOf(b: HitTarget<Region>): number {
  return (b.y1 - b.y0) + (b.value.kind === "section" ? 1000 : 0);
}

/** A zigzag across from `x0` to `x1` at height `y`: the mark for an axis cut. */
function zigzag(x0: number, x1: number, y: number): string {
  const step = 4;
  let d = `M${x0},${y}`;
  let up = true;
  for (let x = x0 + step; x <= x1 + 1e-9; x += step) {
    d += `L${x},${y + (up ? -2 : 2)}`;
    up = !up;
  }
  return d;
}

// ----- the rail's panel -----

/** The least and most height the drawing takes in the rail. It takes the
 *  room left in the window under it, so its foot is in view without
 *  scrolling the rail, and never less than the least, where the rail
 *  scrolls instead. */
const RAIL_MIN_H = 300;
const RAIL_MAX_H = 900;
/** Room kept under the drawing, in the rail. */
const RAIL_FOOT_PX = 12;

class SpaceBandsPanel implements OverviewTabPanel {
  readonly el: HTMLElement;
  private readonly figure: HTMLElement;
  private readonly summary: HTMLElement;
  private readonly legend: HTMLElement;
  private readonly reset: HTMLButtonElement;
  private readonly linkBox: HTMLInputElement;
  private readonly grow: HTMLButtonElement;
  private width: number;
  private shown = false;
  private big = false;
  private linked = true;
  private pic: Picture | null = null;
  private picLength = -1;
  private drawn: SpaceBands | null = null;
  private retry = 0;
  /** Draws again at the window's new size, while the drawing fills it. */
  private readonly onWindowResize = (): void => {
    if (this.big) this.render();
  };

  constructor(private readonly doc: Doc, width: number) {
    this.width = width;
    this.el = document.createElement("section");
    this.el.className = "mmap-panel";
    this.el.tabIndex = -1;
    const bar = document.createElement("div");
    bar.className = "mmap-bar";
    const title = document.createElement("h3");
    title.textContent = S.title;
    this.reset = document.createElement("button");
    this.reset.type = "button";
    this.reset.className = "viz-reset";
    this.reset.textContent = VIZ.resetZoom;
    this.reset.disabled = true;
    this.reset.addEventListener("click", () => this.drawn?.home());
    this.grow = document.createElement("button");
    this.grow.type = "button";
    this.grow.className = "tmp-grow";
    this.grow.addEventListener("click", () => this.setBig(!this.big));
    bar.append(title, this.reset, this.grow);

    const link = document.createElement("label");
    link.className = "mmap-link";
    link.title = S.linkedTitle;
    this.linkBox = document.createElement("input");
    this.linkBox.type = "checkbox";
    this.linkBox.checked = true;
    this.linkBox.addEventListener("change", () => {
      this.linked = this.linkBox.checked;
      this.drawn?.setLinked(this.linked);
    });
    link.append(this.linkBox, document.createTextNode(` ${S.linked}`));

    this.summary = document.createElement("p");
    this.summary.className = "mmap-summary";
    this.legend = document.createElement("div");
    this.legend.className = "mmap-legend";
    this.figure = document.createElement("div");
    this.figure.className = "mmap-figure";
    this.el.append(bar, this.summary, this.legend, link, this.figure);
    this.setGrowLabel();
    this.el.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && this.big) {
        e.stopPropagation();
        this.setBig(false);
      }
    });
  }

  setShown(shown: boolean): void {
    this.shown = shown;
    if (shown) this.refresh();
    else if (this.big) this.setBig(false);
  }

  resize(width: number): void {
    if (Math.abs(width - this.width) < 1) return;
    this.width = width;
    if (!this.big) this.render();
  }

  pump(): void {
    if (this.shown && this.pic === null) this.refresh();
  }

  /** Read the headers again if they were not all here, or the file grew. */
  private refresh(): void {
    if (this.pic !== null && this.picLength === this.doc.lengthBytes) return;
    const reply = readPicture(this.doc);
    if (reply.status === "ok") {
      this.pic = reply.node;
      this.picLength = this.doc.lengthBytes;
      this.render();
      return;
    }
    this.drawn?.destroy();
    this.drawn = null;
    this.legend.replaceChildren();
    this.figure.replaceChildren();
    this.summary.textContent = reply.status === "error" ? S.failed(reply.message) : S.reading;
    // Headers still on the way: ask again shortly, as well as on the rail's
    // own nudges, since a file that has stopped arriving sends none.
    if (reply.status !== "error") {
      clearTimeout(this.retry);
      this.retry = window.setTimeout(() => this.pump(), 250);
    }
  }

  private render(): void {
    const pic = this.pic;
    if (pic === null) return;
    const counts = sectionCounts(pic.regions);
    this.summary.replaceChildren(S.summary(counts.loaded, counts.fileOnly.map((r) => r.name), counts.memOnly.map((r) => r.name)));
    const memRanges = [...pic.regions, ...pic.segments].flatMap((r) => (r.mem === null ? [] : [r.mem]));
    const cuts = memoryAxis(memRanges, 1000).cuts;
    if (cuts.length > 0) {
      const note = document.createElement("span");
      note.className = "mmap-note";
      note.textContent = S.cutNote(formatBytes(cuts.reduce((n, c) => n + (c.end - c.start), 0)));
      this.summary.append(document.createElement("br"), note);
    }
    // The key: only the colours this picture uses, in a fixed order.
    const used = new Set([...pic.regions, ...pic.segments].map((r) => r.access));
    this.legend.replaceChildren(
      ...ACCESS_ORDER.filter((a) => used.has(a)).map((a) => {
        const item = document.createElement("span");
        item.className = "mmap-key";
        const swatch = document.createElement("i");
        swatch.className = `mmap-swatch ${accessClass(a)}`;
        item.append(swatch, S.access[a]);
        return item;
      }),
    );
    let width = this.width;
    const wanted = pic.regions.length * ROW_H * 1.6 + HEAD_H + FOOT_H;
    // The figure is in the page by now, with the summary and key over it.
    const room = this.figure.isConnected ? window.innerHeight - this.figure.getBoundingClientRect().top - RAIL_FOOT_PX : RAIL_MAX_H;
    let height = Math.max(RAIL_MIN_H, Math.min(RAIL_MAX_H, wanted, room));
    if (this.big) {
      width = Math.floor(this.figure.clientWidth);
      height = Math.floor(this.figure.clientHeight);
    }
    this.drawn?.destroy();
    const drawn = drawSpaceBands(this.doc, pic, { width, height, linked: this.linked });
    drawn.onZoom = () => {
      this.reset.disabled = !drawn.zoomed();
    };
    this.reset.disabled = true;
    this.drawn = drawn;
    this.figure.replaceChildren(drawn.el);
  }

  private setBig(big: boolean): void {
    this.big = big;
    // Listened for only while big, so a panel the rail has let go of keeps
    // no listener on the window.
    if (big) window.addEventListener("resize", this.onWindowResize);
    else window.removeEventListener("resize", this.onWindowResize);
    this.el.classList.toggle("mmap-big", big);
    this.setGrowLabel();
    if (big) this.el.focus();
    this.render();
  }

  private setGrowLabel(): void {
    this.grow.textContent = this.big ? TREEMAP.shrinkIcon : TREEMAP.growIcon;
    this.grow.title = this.big ? TREEMAP.shrink : TREEMAP.grow;
    this.grow.setAttribute("aria-label", this.grow.title);
  }
}

/** Add the Memory map tab to the rail, for ELF and PE files. */
export function register(): void {
  registerOverviewTab({
    id: "memory-map",
    label: S.tab,
    title: S.tabTitle,
    offers: readsFormat,
    mount: (doc, width) => new SpaceBandsPanel(doc, width),
  });
}

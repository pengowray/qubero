// The arithmetic half of `spacebands.ts`: which stretches of an executable
// are in the file, which are in memory, where each is drawn on its column,
// and where the names go so that none lands on another.
//
// An executable is stored one way and run another. The loader copies each
// section's bytes from a file offset to a virtual address, pads it out to the
// alignment it asks for, fills a `.bss` with zeros out of nothing, and leaves
// the section name table and the debug sections behind in the file. The
// picture is the one the ELF specification draws as linking view against
// execution view: the file down one column, memory down the other, and a
// band from each section's bytes to its addresses. This module works out the
// stretches (`Region`) from the numbers the format's headers state, and the
// two columns' scales, without a page or a `Doc` behind it, so the tests can
// run it on the numbers of the real samples.
//
// Three choices are made here and not in the drawing:
// - The file column is the file, to scale, top to bottom: every byte of it is
//   somewhere, so nothing is left out.
// - The memory column is cut where nothing is mapped for a long way
//   (`memoryAxis`). An ELF's two LOAD segments are two megabytes apart and
//   hold a megabyte between them; drawn to one scale, two thirds of the
//   column would be empty and the data segment a hairline. The cut is a fixed
//   height, marked on the drawing and stated in words, so the reader is told
//   the column is not to one scale rather than left to assume it is.
// - Names are put on fixed rows (`placeLabels`) with a leader line to what
//   they name, in the order of what they name, so that two leaders never
//   cross and two names never share a row. A name with no free row near its
//   section is dropped, and the hover card still has it.

/** A stretch of bytes or of addresses, end exclusive. */
export type Range = { readonly start: number; readonly end: number };

/** What the loader lets a running program do with a stretch, as the colours
 *  of the drawing: run it, write it, only read it, or nothing, since it is
 *  never loaded. Write and execute together is its own colour, because it is
 *  the unusual one a reader is looking for. */
export type Access = "exec" | "write-exec" | "write" | "read" | "unloaded";

export const ACCESS_ORDER: readonly Access[] = ["exec", "write-exec", "write", "read", "unloaded"];

/** A field of the template, for a click to select. */
export type EntryPick = { readonly path: readonly number[]; readonly startBit: number; readonly endBit: number };

export type RegionKind = "section" | "headers" | "segment";

/** The kinds of header entry a click on the memory side can select. */
export type EntryKind = "section-header" | "section-entry" | "program-header" | "loading-header" | "image-base";

export type Region = {
  /** Unique within one picture, stable across redraws. */
  readonly key: string;
  readonly name: string;
  readonly kind: RegionKind;
  /** Where its bytes are in the file, or null when it has none there. */
  readonly file: Range | null;
  /** For a section with no bytes in the file (ELF `NOBITS`), where the file
   *  says it would be; its band widens from this point. */
  readonly filePoint: number | null;
  /** Where it is loaded, as virtual addresses, or null when it is not. */
  readonly mem: Range | null;
  /** For a PE, the address the loader adds the relative ones to. */
  readonly imageBase: number | null;
  readonly access: Access;
  /** The flags as the format states them, for the hover card. */
  readonly flags: string;
  /** The header entry that states where it goes: a section header, a
   *  program header. Null for a stretch no one entry describes. */
  readonly entry: EntryPick | null;
  /** What `entry` is, for the hover card to say where a click goes. */
  readonly entryKind: EntryKind | null;
  /** The field holding its bytes, when the template has one. */
  readonly body: EntryPick | null;
};

/** What a flags field's words say about access. ELF writes `alloc, write,
 *  execute`; a program header `execute, read`; PE `cnt code, mem execute,
 *  mem read`. A word is matched whole, so `executable` in some other flag
 *  name does not count as `execute`. */
export function accessOf(flags: string, loaded: boolean): Access {
  if (!loaded) return "unloaded";
  const words = new Set(flags.toLowerCase().split(/[^a-z]+/).filter(Boolean));
  const x = words.has("execute");
  const w = words.has("write");
  if (x && w) return "write-exec";
  if (x) return "exec";
  if (w) return "write";
  return "read";
}

/** A word of a flags or type field's value, as `"load (1)"` reads `load`. */
export function enumWord(value: string): string {
  const m = /^\s*([^()]*?)\s*(\(|$)/.exec(value);
  return (m?.[1] ?? value).trim();
}

// ----- ELF -----

export type ElfSectionIn = {
  readonly index: number;
  readonly name: string;
  /** `sh_type`: 8 is `NOBITS`, which has no bytes in the file. */
  readonly type: number;
  readonly flags: string;
  readonly address: number;
  readonly offset: number;
  readonly size: number;
  readonly entry: EntryPick | null;
  readonly body: EntryPick | null;
};

export type ElfSegmentIn = {
  readonly index: number;
  /** The type's word: `load`, `dynamic`, `gnu_relro`. */
  readonly type: string;
  readonly flags: string;
  readonly offset: number;
  readonly vaddr: number;
  readonly fileSize: number;
  readonly memSize: number;
  readonly entry: EntryPick | null;
};

export type ElfIn = {
  readonly fileBytes: number;
  readonly headerSize: number;
  readonly headerEntry: EntryPick | null;
  readonly phoff: number;
  readonly phnum: number;
  readonly phentsize: number;
  readonly phEntry: EntryPick | null;
  readonly shoff: number;
  readonly shnum: number;
  readonly shentsize: number;
  readonly shEntry: EntryPick | null;
  readonly sections: readonly ElfSectionIn[];
  readonly segments: readonly ElfSegmentIn[];
};

const NOBITS = 8;
const SHF_ALLOC = "alloc";

/** Where a stretch of the file lands in memory, by the LOAD segment that
 *  holds it, or null when none does. */
function loadedAt(segments: readonly ElfSegmentIn[], file: Range): { mem: Range; flags: string; entry: EntryPick | null } | null {
  for (const s of segments) {
    if (s.type !== "load") continue;
    if (file.start >= s.offset && file.end <= s.offset + s.fileSize) {
      const at = s.vaddr + (file.start - s.offset);
      return { mem: { start: at, end: at + (file.end - file.start) }, flags: s.flags, entry: s.entry };
    }
  }
  return null;
}

export type Picture = {
  readonly fileBytes: number;
  /** Sections and headers, in file order and then memory order. */
  readonly regions: readonly Region[];
  /** Program headers: drawn as brackets beside both columns. */
  readonly segments: readonly Region[];
};

/** The stretches of an ELF file, from its headers' numbers. */
export function elfPicture(elf: ElfIn): Picture {
  const regions: Region[] = [];
  const clip = (r: Range): Range | null => {
    const start = Math.max(0, Math.min(r.start, elf.fileBytes));
    const end = Math.max(start, Math.min(r.end, elf.fileBytes));
    return end > start ? { start, end } : null;
  };
  // A header's own field is its body; the entry that places it in memory is
  // the program header of the LOAD segment it is inside.
  const header = (key: string, name: string, range: Range, body: EntryPick | null): void => {
    const file = clip(range);
    if (file === null) return;
    const at = loadedAt(elf.segments, file);
    const entry = at?.entry ?? null;
    regions.push({
      key, name, kind: "headers", file, filePoint: null, mem: at?.mem ?? null, imageBase: null,
      access: accessOf(at?.flags ?? "", at !== null), flags: "", entry,
      entryKind: entry !== null ? "loading-header" : null, body,
    });
  };
  header("elf-header", "ELF header", { start: 0, end: elf.headerSize }, elf.headerEntry);
  if (elf.phnum > 0) header("ph-table", "Program header table", { start: elf.phoff, end: elf.phoff + elf.phnum * elf.phentsize }, elf.phEntry);
  if (elf.shnum > 0) header("sh-table", "Section header table", { start: elf.shoff, end: elf.shoff + elf.shnum * elf.shentsize }, elf.shEntry);

  for (const s of elf.sections) {
    // Section 0 is the null entry every ELF starts its table with.
    if (s.type === 0 || (s.index === 0 && s.size === 0)) continue;
    const nobits = s.type === NOBITS;
    const alloc = new Set(s.flags.toLowerCase().split(/[^a-z]+/)).has(SHF_ALLOC);
    const mem = alloc && s.size > 0 ? { start: s.address, end: s.address + s.size } : null;
    const file = nobits || s.size === 0 ? null : clip({ start: s.offset, end: s.offset + s.size });
    if (file === null && mem === null) continue;
    regions.push({
      key: `s${s.index}`, name: s.name === "" ? `Section ${s.index}` : s.name, kind: "section",
      file, filePoint: nobits ? s.offset : null, mem, imageBase: null,
      access: accessOf(s.flags, mem !== null), flags: s.flags, entry: s.entry,
      entryKind: s.entry !== null ? "section-header" : null, body: s.body,
    });
  }
  const segments: Region[] = [];
  for (const s of elf.segments) {
    const file = s.fileSize > 0 ? clip({ start: s.offset, end: s.offset + s.fileSize }) : null;
    const mem = s.memSize > 0 && s.type === "load" ? { start: s.vaddr, end: s.vaddr + s.memSize } : null;
    // A segment with neither (`gnu_stack` states only flags) has nothing to
    // draw; its flags are in the listing.
    if (file === null && mem === null) continue;
    segments.push({
      key: `p${s.index}`, name: `${s.type} ${s.index + 1}`, kind: "segment", file, filePoint: null, mem, imageBase: null,
      access: accessOf(s.flags, mem !== null || s.type !== "load"), flags: s.flags, entry: s.entry,
      entryKind: s.entry !== null ? "program-header" : null, body: null,
    });
  }
  return { fileBytes: elf.fileBytes, regions: sortRegions(regions), segments };
}

// ----- PE -----

export type PeSectionIn = {
  readonly index: number;
  readonly name: string;
  readonly virtualSize: number;
  /** Relative to the image base, as the section table states it. */
  readonly virtualAddress: number;
  readonly rawSize: number;
  readonly rawOffset: number;
  readonly flags: string;
  readonly entry: EntryPick | null;
  readonly body: EntryPick | null;
};

export type PeIn = {
  readonly fileBytes: number;
  readonly imageBase: number;
  readonly headersSize: number;
  /** The headers' own stretch, for a click on their file side. */
  readonly headersEntry: EntryPick | null;
  /** The field stating the image base, which is where the headers load. */
  readonly imageBaseEntry: EntryPick | null;
  readonly sections: readonly PeSectionIn[];
};

/** The stretches of a PE file, from its section table. */
export function pePicture(pe: PeIn): Picture {
  const regions: Region[] = [];
  const base = pe.imageBase;
  if (pe.headersSize > 0) {
    const end = Math.min(pe.headersSize, pe.fileBytes);
    regions.push({
      key: "pe-headers", name: "Headers", kind: "headers", file: { start: 0, end }, filePoint: null,
      mem: { start: base, end: base + pe.headersSize }, imageBase: base, access: "read", flags: "",
      entry: pe.imageBaseEntry, entryKind: pe.imageBaseEntry !== null ? "image-base" : null, body: pe.headersEntry,
    });
  }
  let fileEnd = pe.headersSize;
  for (const s of pe.sections) {
    // A virtual size of zero means the raw size, by the PE specification.
    const memSize = s.virtualSize > 0 ? s.virtualSize : s.rawSize;
    const rawEnd = Math.min(s.rawOffset + s.rawSize, pe.fileBytes);
    const file = s.rawSize > 0 && s.rawOffset > 0 && rawEnd > s.rawOffset ? { start: s.rawOffset, end: rawEnd } : null;
    const mem = memSize > 0 ? { start: base + s.virtualAddress, end: base + s.virtualAddress + memSize } : null;
    if (file !== null) fileEnd = Math.max(fileEnd, file.end);
    if (file === null && mem === null) continue;
    regions.push({
      key: `s${s.index}`, name: s.name === "" ? `Section ${s.index + 1}` : s.name, kind: "section",
      file, filePoint: null, mem, imageBase: base, access: accessOf(s.flags, mem !== null), flags: s.flags,
      entry: s.entry, entryKind: s.entry !== null ? "section-entry" : null, body: s.body,
    });
  }
  // Bytes past the last section (a signature, an installer's payload) are in
  // the file and never loaded.
  if (fileEnd < pe.fileBytes) {
    regions.push({
      key: "pe-overlay", name: "Data after the last section", kind: "headers", file: { start: fileEnd, end: pe.fileBytes },
      filePoint: null, mem: null, imageBase: null, access: "unloaded", flags: "", entry: null, entryKind: null, body: null,
    });
  }
  return { fileBytes: pe.fileBytes, regions: sortRegions(regions), segments: [] };
}

/** File order, and the stretches with no file bytes after, in memory order. */
function sortRegions(regions: Region[]): Region[] {
  const where = (r: Region): number => r.file?.start ?? r.filePoint ?? Number.POSITIVE_INFINITY;
  return regions.sort((a, b) => where(a) - where(b) || (a.mem?.start ?? 0) - (b.mem?.start ?? 0));
}

// ----- the two axes -----

/** One stretch of an axis drawn to scale, from `y0` to `y1` in layout pixels. */
export type Piece = { readonly start: number; readonly end: number; readonly y0: number; readonly y1: number };

/** A stretch cut out of an axis: addresses `start` to `end` are drawn as the
 *  fixed height `y0` to `y1`, not to scale. */
export type Cut = Piece;

export type Axis = {
  readonly pieces: readonly Piece[];
  readonly cuts: readonly Cut[];
  /** The layout height the axis is drawn over. */
  readonly length: number;
  readonly start: number;
  readonly end: number;
};

/** The file axis: the whole file, to scale. */
export function fileAxis(fileBytes: number, length: number): Axis {
  const end = Math.max(1, fileBytes);
  return { pieces: [{ start: 0, end, y0: 0, y1: length }], cuts: [], length, start: 0, end };
}

export type MemoryAxisOptions = {
  /** The height a cut is drawn at. */
  readonly cutPx: number;
  /** The least height a piece is drawn at, so the data segment of an ELF,
   *  a hundredth the size of its code, is still something to point at. */
  readonly minPiecePx: number;
  /** A gap is cut when it is longer than this share of the mapped bytes. */
  readonly cutShare: number;
};

export const MEMORY_AXIS: MemoryAxisOptions = { cutPx: 18, minPiecePx: 28, cutShare: 0.25 };

/**
 * The memory axis over `ranges`: the stretches that are mapped, merged where
 * they touch or the gap between them is short, drawn to scale; and each long
 * gap cut down to `cutPx`. Short gaps (the alignment between PE sections, a
 * page between ELF segments) stay to scale, since they are the slack the
 * picture is meant to show. A gap counts as long when it is more than
 * `cutShare` of all the mapped bytes together.
 */
export function memoryAxis(ranges: readonly Range[], length: number, opts: MemoryAxisOptions = MEMORY_AXIS): Axis {
  const sorted = ranges.filter((r) => r.end > r.start).sort((a, b) => a.start - b.start);
  if (sorted.length === 0) return { pieces: [], cuts: [], length, start: 0, end: 1 };
  // Merge what overlaps or touches.
  const runs: { start: number; end: number }[] = [];
  for (const r of sorted) {
    const last = runs[runs.length - 1];
    if (last !== undefined && r.start <= last.end) last.end = Math.max(last.end, r.end);
    else runs.push({ start: r.start, end: r.end });
  }
  const mapped = runs.reduce((n, r) => n + (r.end - r.start), 0);
  // Join runs across the gaps short enough to keep to scale.
  const spans: { start: number; end: number }[] = [];
  for (const r of runs) {
    const last = spans[spans.length - 1];
    if (last !== undefined && r.start - last.end <= mapped * opts.cutShare) last.end = r.end;
    else spans.push({ ...r });
  }
  const cutCount = spans.length - 1;
  const room = Math.max(1, length - cutCount * opts.cutPx);
  const total = spans.reduce((n, s) => n + (s.end - s.start), 0);
  // Each span its share of the room, raised to the least height where that
  // leaves room for the rest; the raise comes out of the spans above it.
  const min = Math.min(opts.minPiecePx, room / spans.length);
  let heights = spans.map((s) => (room * (s.end - s.start)) / total);
  const small = heights.map((h) => h < min);
  const bigTotal = spans.reduce((n, s, i) => n + (small[i] === true ? 0 : s.end - s.start), 0);
  const bigRoom = room - small.filter(Boolean).length * min;
  if (small.some(Boolean) && bigTotal > 0) heights = spans.map((s, i) => (small[i] === true ? min : (bigRoom * (s.end - s.start)) / bigTotal));
  const pieces: Piece[] = [];
  const cuts: Cut[] = [];
  let y = 0;
  spans.forEach((s, i) => {
    const h = heights[i] as number;
    pieces.push({ start: s.start, end: s.end, y0: y, y1: y + h });
    y += h;
    const next = spans[i + 1];
    if (next !== undefined) {
      cuts.push({ start: s.end, end: next.start, y0: y, y1: y + opts.cutPx });
      y += opts.cutPx;
    }
  });
  const first = spans[0] as { start: number };
  const last = spans[spans.length - 1] as { end: number };
  return { pieces, cuts, length, start: first.start, end: last.end };
}

/** Where an address is drawn on an axis, in layout pixels. Inside a cut it
 *  is placed in proportion across the cut's height, so the axis is still one
 *  line from top to bottom. */
export function axisY(axis: Axis, at: number): number {
  const all = [...axis.pieces, ...axis.cuts].sort((a, b) => a.start - b.start);
  if (all.length === 0) return 0;
  const first = all[0] as Piece;
  if (at <= first.start) return first.y0;
  for (const p of all) {
    if (at <= p.end) {
      const span = p.end - p.start;
      return span === 0 ? p.y0 : p.y0 + ((at - p.start) / span) * (p.y1 - p.y0);
    }
  }
  return (all[all.length - 1] as Piece).y1;
}

/** Which address is drawn at a layout height, and whether it is in a cut. */
export function axisAt(axis: Axis, y: number): { readonly at: number; readonly cut: Cut | null } {
  for (const c of axis.cuts) if (y >= c.y0 && y < c.y1) return { at: c.start, cut: c };
  for (const p of axis.pieces) {
    if (y >= p.y0 && y <= p.y1) {
      const h = p.y1 - p.y0;
      return { at: h === 0 ? p.start : p.start + ((y - p.y0) / h) * (p.end - p.start), cut: null };
    }
  }
  const first = axis.pieces[0];
  const last = axis.pieces[axis.pieces.length - 1];
  if (first === undefined || last === undefined) return { at: 0, cut: null };
  return { at: y < first.y0 ? first.start : last.end, cut: null };
}

// ----- across -----

/** Where everything goes across the drawing, left to right: the file side's
 *  names, the lane of segment brackets beside the file column, the file
 *  column, the gap the bands cross, the memory column, its bracket lane, and
 *  the memory side's names. All in pixels from the left edge. */
export type Columns = {
  readonly namesL: Range;
  readonly stripL: Range;
  readonly file: Range;
  readonly memory: Range;
  readonly stripR: Range;
  readonly namesR: Range;
  /** Where the two halves meet, for the two zoom surfaces. */
  readonly middle: number;
};

/** Room between a name and its column, for the leader line. */
const LEADER_PX = 8;
/** One lane of segment brackets. */
export const BRACKET_LANE_PX = 5;

/**
 * The columns for a drawing `width` wide with `lanes` lanes of segment
 * brackets. At the rail's width (about 256 pixels) the bands get some 70
 * pixels and each side's names what is left; over the whole window the
 * columns widen and the gap is held to a width where a band's slope still
 * reads, with the whole centred.
 */
export function columnsFor(width: number, lanes: number): Columns {
  const strip = lanes > 0 ? lanes * BRACKET_LANE_PX + 2 : 0;
  const col = Math.round(Math.max(16, Math.min(64, width * 0.07)));
  let namesL = Math.round(Math.max(44, Math.min(220, width * 0.28)));
  let namesR = Math.round(Math.max(32, Math.min(180, width * 0.16)));
  const fixed = (): number => namesL + namesR + 2 * (LEADER_PX + strip + col);
  // Narrower than the least it can be drawn at: the names give way first.
  const minGap = 32;
  if (width - fixed() < minGap) {
    const over = minGap - (width - fixed());
    const cutL = Math.min(namesL - 24, Math.ceil((over * namesL) / (namesL + namesR)));
    namesL -= Math.max(0, cutL);
    namesR -= Math.max(0, Math.min(namesR - 16, over - cutL));
  }
  const gap = Math.max(minGap, Math.min(480, width - fixed()));
  const total = fixed() + gap;
  let x = Math.max(0, (width - total) / 2);
  const take = (w: number): Range => {
    const r = { start: x, end: x + w };
    x += w;
    return r;
  };
  const nl = take(namesL);
  x += LEADER_PX;
  const sl = take(strip);
  const file = take(col);
  x += gap;
  const memory = take(col);
  const sr = take(strip);
  x += LEADER_PX;
  const nr = take(namesR);
  return { namesL: nl, stripL: sl, file, memory, stripR: sr, namesR: nr, middle: (file.end + memory.start) / 2 };
}

// ----- names on rows -----

/** A name that wants to be drawn level with `y`; the larger `priority` goes
 *  first when two want one row. */
export type LabelWant = { readonly y: number; readonly priority: number };

/**
 * A row for each name, between `top` and `bottom`, rows `rowH` apart: the
 * centre of the row it is drawn on, or null where it is dropped.
 *
 * The most important names choose first, each taking the free row nearest
 * its own height within `maxShift` rows. Then the rows chosen are handed out
 * again in the order of the heights wanted, so the names run in the same
 * order as what they name and no two leader lines cross. A name whose height
 * is off the drawing is dropped: it would point at nothing on screen.
 */
export function placeLabels(wants: readonly LabelWant[], rowH: number, top: number, bottom: number, maxShift = 4): (number | null)[] {
  const out = new Array<number | null>(wants.length).fill(null);
  const rows = Math.floor((bottom - top) / rowH);
  if (rows <= 0) return out;
  const taken = new Array<boolean>(rows).fill(false);
  const order = wants.map((_, i) => i).sort((a, b) => (wants[b] as LabelWant).priority - (wants[a] as LabelWant).priority || a - b);
  const chosen: { i: number; row: number }[] = [];
  for (const i of order) {
    const w = wants[i] as LabelWant;
    if (w.y < top || w.y > bottom) continue;
    const ideal = Math.max(0, Math.min(rows - 1, Math.floor((w.y - top) / rowH)));
    for (let d = 0; d <= maxShift; d++) {
      const below = ideal + d;
      const above = ideal - d;
      const row = below < rows && !taken[below] ? below : above >= 0 && !taken[above] ? above : -1;
      if (row < 0) continue;
      taken[row] = true;
      chosen.push({ i, row });
      break;
    }
  }
  const byWant = [...chosen].sort((a, b) => (wants[a.i] as LabelWant).y - (wants[b.i] as LabelWant).y || a.i - b.i);
  const slots = chosen.map((c) => c.row).sort((a, b) => a - b);
  byWant.forEach((c, n) => {
    out[c.i] = top + ((slots[n] as number) + 0.5) * rowH;
  });
  return out;
}

// ----- bands -----

/** The S-curve a band's edge follows across the gap, flat at both ends so a
 *  band leaves each column level. */
export function smooth(u: number): number {
  const t = Math.max(0, Math.min(1, u));
  return t * t * (3 - 2 * t);
}

/** A band between the file column's edge at `xa` and the memory column's at
 *  `xb`: the file side runs `fa` to `fb`, the memory side `ma` to `mb`. */
export type Band = { readonly xa: number; readonly xb: number; readonly fa: number; readonly fb: number; readonly ma: number; readonly mb: number };

/** The band's top and bottom edge at `x`. */
export function bandEdges(b: Band, x: number): { readonly top: number; readonly bottom: number } {
  const u = b.xb === b.xa ? 0 : (x - b.xa) / (b.xb - b.xa);
  const s = smooth(u);
  return { top: b.fa + (b.ma - b.fa) * s, bottom: b.fb + (b.mb - b.fb) * s };
}

/** The band's outline as an SVG path, in `steps` straight pieces each way:
 *  the same curve `bandEdges` hit-tests, so what is drawn is what is picked. */
export function bandPath(b: Band, steps = 16, minThick = 1): string {
  const top: string[] = [];
  const bottom: string[] = [];
  for (let i = 0; i <= steps; i++) {
    const x = b.xa + ((b.xb - b.xa) * i) / steps;
    const e = bandEdges(b, x);
    // A band from a sliver still shows as a line.
    const mid = (e.top + e.bottom) / 2;
    const half = Math.max(minThick / 2, (e.bottom - e.top) / 2);
    top.push(`${x.toFixed(2)},${(mid - half).toFixed(2)}`);
    bottom.push(`${x.toFixed(2)},${(mid + half).toFixed(2)}`);
  }
  return `M${top.join("L")}L${bottom.reverse().join("L")}Z`;
}

/**
 * The band a pointer at (`x`, `y`) is on, by index: the thinnest one it is
 * inside at that `x`, and otherwise the nearest within `reach` pixels, so a
 * hairline band from an eight-byte section can still be pointed at. Null off
 * the gap between the columns, or with nothing near.
 */
export function bandAt(bands: readonly Band[], x: number, y: number, reach = 6): number | null {
  let best: number | null = null;
  let bestThick = Number.POSITIVE_INFINITY;
  let near: number | null = null;
  let nearDist = Number.POSITIVE_INFINITY;
  bands.forEach((b, i) => {
    if (x < Math.min(b.xa, b.xb) || x > Math.max(b.xa, b.xb)) return;
    const e = bandEdges(b, x);
    const thick = e.bottom - e.top;
    if (y >= e.top && y <= e.bottom) {
      if (thick < bestThick) {
        best = i;
        bestThick = thick;
      }
      return;
    }
    const d = y < e.top ? e.top - y : y - e.bottom;
    if (d <= reach && d < nearDist) {
      near = i;
      nearDist = d;
    }
  });
  return best ?? near;
}

/** The region whose file bytes hold `byte`, the smallest when several do
 *  (a section inside the headers' stretch), or null. */
export function regionAtByte(regions: readonly Region[], byte: number): Region | null {
  let best: Region | null = null;
  for (const r of regions) {
    if (r.file === null || byte < r.file.start || byte >= r.file.end) continue;
    if (best === null || (best.file !== null && r.file.end - r.file.start < best.file.end - best.file.start)) best = r;
  }
  return best;
}

/** How many sections the picture has, and how many of them are loaded, in
 *  the file only, or in memory only: the sentence over the drawing. */
export function sectionCounts(regions: readonly Region[]): { total: number; loaded: number; fileOnly: Region[]; memOnly: Region[] } {
  const sections = regions.filter((r) => r.kind === "section");
  return {
    total: sections.length,
    loaded: sections.filter((r) => r.mem !== null).length,
    fileOnly: sections.filter((r) => r.mem === null),
    memOnly: sections.filter((r) => r.file === null && r.mem !== null),
  };
}

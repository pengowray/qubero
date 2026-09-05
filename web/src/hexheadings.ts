// The lines that name the parts of the file, drawn between the bytes of the
// grid: which rows they fall on, where along a row each one cuts it, and how a
// block of them is written.
import { formatBytes } from "./doc.js";
import type { OutlineHeading } from "./outline.js";
import { REPORT } from "./strings.js";
import { rangeText, shareText } from "./listingdraw.js";
import { setText } from "./hexcell.js";

/** A heading line, with the part it names kept on it. */
export type HeadEl = HTMLButtonElement & { _head?: OutlineHeading | undefined };

/** What a heading calls a part with no name of its own: the listing's word
 *  for a run of fields at the front, the back or the middle of the file. */
export function headingName(h: OutlineHeading, fileBits: number): string {
  if (h.name !== "") return h.name;
  const where = h.offsetBits === 0 ? "start" : fileBits > 0 && h.offsetBits + h.sizeBits >= fileBits ? "end" : "middle";
  return REPORT.unnamedPart(where);
}

/** What the stylesheet says a heading line is tall, by the heading's level:
 *  one pair for a heading with space above it and one for the heading that
 *  has nothing above it to be spaced away from. */
export type HeadingSizes = {
  readonly heading: readonly [number, number];
  readonly headingFirst: readonly [number, number];
};

/**
 * How tall a heading line is.
 *
 * Every heading has space above it, so that a part of the file is divided
 * from the one before rather than butted up against it: every heading but the
 * one for the part that starts at the front of the file, which has nothing
 * above it. Keyed on where the part is in the file, never on where the row
 * happens to fall on screen, since a row's height must not depend on whether
 * it is the top one. `fallback` is a row's own height, for a level the
 * stylesheet says nothing about.
 */
export function headingHeight(h: OutlineHeading, sizes: HeadingSizes, fallback: number): number {
  const pair = h.offsetBits === 0 ? sizes.headingFirst : sizes.heading;
  return pair[h.level] ?? pair[1] ?? fallback;
}

/** What pressing a heading does, for a reader who cannot guess. */
export const HEADING_TIP = (name: string): string => `Move the cursor to the first byte of ${name}`;

/**
 * The headings that fall on each row on screen. The sections are sorted by
 * offset and a file of a hundred thousand pages has a heading for each, so
 * the first one on screen is found by bisection and the rest read off in
 * order.
 */
export function headingsByRow(
  sections: readonly OutlineHeading[],
  start: number,
  windowBytes: number,
  bpr: number,
): OutlineHeading[][] {
  const byRow: OutlineHeading[][] = [];
  const fromBit = start * 8;
  const toBit = (start + windowBytes) * 8;
  let lo = 0;
  let hi = sections.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((sections[mid] as OutlineHeading).offsetBits < fromBit) lo = mid + 1;
    else hi = mid;
  }
  for (let i = lo; i < sections.length; i++) {
    const h = sections[i] as OutlineHeading;
    if (h.offsetBits >= toBit) break;
    const row = Math.floor((Math.floor(h.offsetBits / 8) - start) / bpr);
    (byRow[row] ??= []).push(h);
  }
  return byRow;
}

/**
 * How one row on screen is put together: where a heading cuts it, what stands
 * over each of the pieces that makes, and how tall that is.
 *
 * The three lists run in step, one entry per piece. Everything that has to
 * know the shape of a row reads it from here rather than working it out
 * again: which lines the row is drawn in, how tall it will come out, and how
 * far a scroll has to travel to take a piece off the top of the screen.
 */
export type RowPieces = {
  /** The byte the row starts at. */
  readonly rowStart: number;
  /** The pieces the row is drawn in, as the position each starts at. Always
   *  begins with 0, and is just `[0]` on a row nothing cuts. */
  readonly segs: readonly number[];
  /** The headings drawn over each piece. Empty for a piece nothing starts
   *  at, which is every piece of most rows. */
  readonly heads: readonly (readonly OutlineHeading[])[];
  /** How tall each of those blocks of headings comes to. */
  readonly headHeights: readonly number[];
};

/**
 * Work out that shape.
 *
 * A part that starts part-way along a row cuts it there, so the heading sits
 * between the bytes before it and the bytes after. Condensed readings keep
 * every heading above the row: they are the readings that trade room for
 * rows.
 */
export function rowPieces(
  heads: readonly OutlineHeading[],
  rowStart: number,
  bpr: number,
  condensed: boolean,
  sizes: HeadingSizes,
  rowHeight: number,
): RowPieces {
  const at = new Map<number, OutlineHeading[]>();
  for (const h of heads) {
    const pos = condensed ? 0 : Math.min(bpr - 1, Math.max(0, Math.floor(h.offsetBits / 8) - rowStart));
    const had = at.get(pos);
    if (had === undefined) at.set(pos, [h]);
    else had.push(h);
  }
  const segs = [...new Set([0, ...at.keys()])].sort((a, b) => a - b);
  const per = segs.map((pos) => at.get(pos) ?? []);
  return {
    rowStart,
    segs,
    heads: per,
    headHeights: per.map((hs) => hs.reduce((n, h) => n + headingHeight(h, sizes, rowHeight), 0)),
  };
}

/** An empty heading line. Like a chip, it keeps its click handler and the
 *  part it names, so drawing it again does not mean making it again. */
export function newHeading(onPress: (h: OutlineHeading) => void): HeadEl {
  const b = document.createElement("button") as HeadEl;
  b.type = "button";
  const swatch = document.createElement("span");
  swatch.className = "hv-swatch";
  const nameEl = document.createElement("b");
  nameEl.className = "hv-heading-name";
  const range = document.createElement("span");
  range.className = "hv-heading-range";
  const size = document.createElement("span");
  size.className = "hv-heading-size";
  b.append(swatch, nameEl, range, size);
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    const h = b._head;
    if (h === undefined) return;
    onPress(h);
  });
  return b;
}

/**
 * Fill a block with the heading lines for the parts that start at one place:
 * for each, its colour, name, address range, size and share of the file, as
 * the listing gives them. Pressing one goes to the part's first byte. `at` is
 * the byte the block sits before, which is what a drag across it reads as.
 *
 * The block and its lines are written over rather than built again, so that a
 * scroll does not take an element out from under a finger resting on it,
 * which the browser reads as the touch being called off.
 */
export function fillHeadings(
  block: HTMLElement,
  heads: readonly OutlineHeading[],
  fileBits: number,
  at: number,
  onPress: (h: OutlineHeading) => void,
): void {
  const off = String(at);
  if (block.dataset["segOff"] !== off) block.dataset["segOff"] = off;
  // The block stays in the row whether or not a part starts here; with
  // nothing in it, it is not a gap above the row.
  if (block.hidden !== (heads.length === 0)) block.hidden = heads.length === 0;
  while (block.childElementCount < heads.length) block.append(newHeading(onPress));
  // Hidden rather than taken away, for the same reason the chips are.
  for (let i = 0; i < block.childElementCount; i++) {
    const b = block.children[i] as HTMLElement;
    if (b.hidden !== i >= heads.length) b.hidden = i >= heads.length;
  }
  for (const [i, h] of heads.entries()) {
    const b = block.children[i] as HeadEl;
    b._head = h;
    // Every heading has space above it but the one for the part that starts at
    // the front of the file, which has nothing above it to be divided from.
    // The class says which, so a row's height never depends on where the row
    // falls on screen.
    const cls = `hv-heading hv-heading-${h.level}${h.offsetBits === 0 ? " hv-heading-first" : ""}`;
    if (b.className !== cls) b.className = cls;
    const swatch = b.firstElementChild as HTMLElement;
    swatch.hidden = h.level !== 0;
    if (h.level === 0 && swatch.style.background !== h.color) swatch.style.background = h.color;
    const name = headingName(h, fileBits);
    setText(b.children[1] as HTMLElement, name);
    setText(b.children[2] as HTMLElement, rangeText(h.offsetBits, h.sizeBits));
    const share = shareText(h.sizeBits, fileBits);
    setText(b.children[3] as HTMLElement, `${formatBytes(h.sizeBits / 8)}${share === "" ? "" : ` · ${share}`}`);
    const tip = HEADING_TIP(name);
    if (b.title !== tip) b.title = tip;
  }
}

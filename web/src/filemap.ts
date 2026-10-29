// The same small picture of the whole file, drawn again wherever a range needs
// placing: on a heading, in a byte strip's caption, on the rail, under a part
// of the report.
//
// The geometry is fixed. Every strip has the same segments in the same places,
// whatever it is drawn beside, so the only thing that moves between one strip
// and the next is which part is lit. That is what makes it readable at a
// glance: the reader learns the shape of the file once, and every later strip
// is the same shape with a different piece bright. A bar showing each row's
// own percentage would be a different picture every time and would say only
// how big the row is, which the row already says.
//
// The strip is live. Pointing at it names the part under the pointer, with
// its range and its share of the file, in the one hover card (`viz/tip.ts`);
// clicking goes to that part. A two-pixel part is found from twelve pixels
// either side, by the hit test every piece uses. How the click reaches the
// part depends on what the strip was given:
// - with a `Doc`, the strip goes there itself (`viz/pick.ts`), and the part
//   under the pointer is lit in the hex view through the shared hover;
// - without one, as on the listing's headings, the strip carries the part
//   under the pointer in the `data-reads`, `data-at` and `data-size`
//   attributes, which the listing already follows for every field button.
//
// The parts are cut into cells first, in file order, so parts handed over out
// of order or nested inside one another still make a map (`viz/mapcells.ts`).
//
// The report's position bar (`report/posbar.ts` until this) is the same strip
// with several stretches lit at once (`lit`): a group of parts scattered
// through the file is one strip with each of them bright.
//
// A field inside a compressed stream is at an offset of the bytes the stream
// unpacks to, which is nowhere in the file. Its strip has a second ring
// (`ring`): the file strip with the stream's packed bytes lit, and under it a
// strip of the unpacked stream with the field lit.

import type { Doc } from "./doc.ts";
import type { TrailPlace } from "./trail.ts";
import { formatAddress } from "./format.ts";
import { bitSizeText, VIZ } from "./strings.ts";
import {
  bitInCell,
  cellAt,
  cellSpans,
  cellStretches,
  cellsThatFit,
  foldCells,
  litSpans,
  MAP_WIDTH_PX,
  mapCells,
  partAt,
  segmentWidths,
  type MapCell,
  type Stretch,
} from "./viz/mapcells.ts";
import { linkedFor } from "./viz/linked.ts";
import { goToBits, goToField } from "./viz/pick.ts";
import { hideTip, tipAt, tipCard } from "./viz/tip.ts";

export { segmentWidths } from "./viz/mapcells.ts";

/** One division of the file: a stretch of bytes and the colour it goes by,
 *  and, where the caller has them, what it is called and the field it is, so
 *  the strip can name it on hover and go to it on a click. */
export type MapSegment = {
  readonly offsetBits: number;
  readonly sizeBits: number;
  readonly color: string;
  readonly name?: string;
  readonly path?: readonly number[];
};

/** What is selected, marked over the top of what the strip is about. */
export type MapMark = Stretch | null;

/** The unpacked stream a field is in, drawn as a second strip under the
 *  file's. The file strip lights `packed`, where the stream's bytes are in
 *  the file; the ring lights `lit`, the field, in the stream's own offsets. */
export type MapRing = {
  /** The stream's parts, in its own offsets. Empty draws it as one part. */
  readonly segments: readonly MapSegment[];
  readonly lengthBits: number;
  /** Which address space the stream is, as `TemplateNode.space` counts. */
  readonly space: number;
  readonly packed: Stretch;
  readonly lit: Stretch;
  /** What the stream is called, for the hover card. */
  readonly name: string;
};

export type MapOptions = {
  /** How wide the strip is drawn, in CSS pixels. 96 when not given. */
  readonly width?: number;
  /** Several stretches lit at once, in place of the one range. */
  readonly lit?: readonly Stretch[];
  /** The whole length of the file, so bytes after the last part are drawn. */
  readonly lengthBits?: number;
  /** Go to the part clicked, and light the part pointed at in the hex view. */
  readonly doc?: Doc;
  readonly ring?: MapRing | null;
};

/** The unlit ground. Dark enough that a lit sliver reads as lit, in either
 *  theme, without carrying a meaning of its own. */
const GROUND = "color-mix(in srgb, var(--fg) 14%, var(--bg))";
/** Bytes between parts that no part covers: paler than the ground, so a hole
 *  in the listing's parts reads as a hole and not as an unlit part. */
const UNCOVERED = "color-mix(in srgb, var(--fg) 6%, var(--bg))";

function paint(color: string, spans: readonly (readonly [number, number])[]): string {
  if (spans.length === 0) return GROUND;
  const first = spans[0] as readonly [number, number];
  if (spans.length === 1 && first[0] <= 0 && first[1] >= 1) return color;
  const stops: string[] = [];
  for (const [a, b] of spans) {
    const from = `${(a * 100).toFixed(2)}%`;
    const to = `${(b * 100).toFixed(2)}%`;
    stops.push(`${GROUND} ${from}`, `${color} ${from}`, `${color} ${to}`, `${GROUND} ${to}`);
  }
  return `linear-gradient(to right, ${stops.join(", ")})`;
}

/** What one drawn strip stands on: its parts, its cells and where each cell
 *  is drawn. Kept so a strip can be told what is selected now without being
 *  drawn again, and so the pointer can be answered without measuring. */
type Drawn = {
  readonly segments: readonly MapSegment[];
  readonly cells: readonly MapCell[];
  readonly spans: ReturnType<typeof cellSpans>;
  /** Which address space the strip's offsets count in; 0 is the file. */
  readonly space: number;
  /** The bits the shares on the hover card are of. */
  readonly totalBits: number;
};

const drawnOver = new WeakMap<HTMLElement, Drawn>();

/** The cells of a set of parts, worked out once per set: the listing draws a
 *  strip on every heading from the same array, and a paint redraws them all. */
const cellsOf = new WeakMap<readonly MapSegment[], Map<string, MapCell[]>>();

function cellsFor(segments: readonly MapSegment[], width: number, lengthBits: number | undefined): MapCell[] {
  let byWidth = cellsOf.get(segments);
  if (byWidth === undefined) {
    byWidth = new Map();
    cellsOf.set(segments, byWidth);
  }
  const key = `${width}:${lengthBits ?? ""}`;
  let cells = byWidth.get(key);
  if (cells === undefined) {
    cells = foldCells(mapCells(segments, lengthBits === undefined ? undefined : 0, lengthBits), cellsThatFit(width));
    byWidth.set(key, cells);
  }
  return cells;
}

function markCell(cell: HTMLElement, at: readonly (readonly [number, number])[]): void {
  const pins = Array.from(cell.children).filter((c): c is HTMLElement => c instanceof HTMLElement && c.classList.contains("fmap-mark"));
  at.forEach(([a, b], i) => {
    const pin = pins[i] ?? cell.appendChild(document.createElement("b"));
    pin.className = "fmap-mark";
    pin.style.insetInlineStart = `${(a * 100).toFixed(2)}%`;
    pin.style.width = `${((b - a) * 100).toFixed(2)}%`;
  });
  for (const extra of pins.slice(at.length)) extra.remove();
}

/** Move the mark on a strip that is already up. */
export function markMap(strip: HTMLElement, mark: MapMark): void {
  const drawn = drawnOver.get(strip);
  if (drawn === undefined) return;
  drawn.cells.forEach((c, i) => {
    const cell = strip.children[i];
    if (!(cell instanceof HTMLElement)) return;
    markCell(cell, mark === null ? [] : litSpans(c, [mark]));
  });
}

/**
 * One strip, with the bytes from `offsetBits` for `sizeBits` lit (or the
 * stretches of `opts.lit`), and `mark` drawn over the top.
 *
 * `title` says what the strip is about, in the words the rest of its row
 * uses, for a screen reader; the hover card names the part under the pointer.
 */
export function fileMap(
  segments: readonly MapSegment[],
  offsetBits: number,
  sizeBits: number,
  title: string,
  mark?: MapMark,
  opts: MapOptions = {},
): HTMLElement {
  const ring = opts.ring ?? null;
  const lit = ring !== null ? [ring.packed] : (opts.lit ?? [{ offsetBits, sizeBits }]);
  const strip = drawStrip(segments, lit, mark ?? null, opts.width ?? MAP_WIDTH_PX, opts.lengthBits, 0, title, opts.doc);
  if (ring === null) return strip;
  // The file strip, then the stream's: the first `.fmap` in the stack is the
  // file's, which is the one callers mark the selection on.
  const stack = document.createElement("span");
  stack.className = "fmap-stack";
  const inner = drawStrip(ring.segments.length > 0 ? ring.segments : [{ offsetBits: 0, sizeBits: ring.lengthBits, color: "var(--accent)", name: ring.name }], [ring.lit], null, opts.width ?? MAP_WIDTH_PX, ring.lengthBits, ring.space, title, opts.doc);
  inner.classList.add("fmap-ring");
  stack.append(strip, inner);
  return stack;
}

function drawStrip(
  segments: readonly MapSegment[],
  lit: readonly Stretch[],
  mark: MapMark,
  width: number,
  lengthBits: number | undefined,
  space: number,
  title: string,
  doc: Doc | undefined,
): HTMLElement {
  const strip = document.createElement("span");
  strip.className = "fmap";
  strip.setAttribute("role", "img");
  strip.setAttribute("aria-label", title);
  if (width !== MAP_WIDTH_PX) strip.style.width = `${width}px`;
  const cells = cellsFor(segments, width, lengthBits);
  const widths = segmentWidths(cellStretches(cells), width);
  cells.forEach((c, i) => {
    const cell = document.createElement("i");
    cell.style.flex = `0 0 ${(widths[i] ?? 0).toFixed(2)}px`;
    const color = c.owner < 0 ? null : (segments[c.owner] as MapSegment).color;
    cell.style.background = color === null ? UNCOVERED : paint(color, litSpans(c, lit));
    // What is selected, over the top of what this strip is about. Drawn rather
    // than blended in, so a selection inside the lit part still reads: the two
    // answer different questions and must not average into one colour.
    if (mark !== null) markCell(cell, litSpans(c, [mark]));
    strip.append(cell);
  });
  const first = cells[0];
  const last = cells[cells.length - 1];
  const totalBits = lengthBits ?? (first !== undefined && last !== undefined ? last.end - first.start : 0);
  const drawn: Drawn = { segments, cells, spans: cellSpans(widths), space, totalBits };
  drawnOver.set(strip, drawn);
  live(strip, drawn, doc);
  return strip;
}

/** The part under a pointer event, by the hit test: its index in the
 *  segments, or -1 for bytes no part covers, and the bit under the pointer. */
function hitOf(strip: HTMLElement, drawn: Drawn, e: MouseEvent): { part: number; bit: number } | null {
  const r = strip.getBoundingClientRect();
  // The strip may be drawn wider or narrower than it was laid out for (the
  // rail stretches its strip to the rail): the pointer is scaled back.
  const laidOut = drawn.spans.length === 0 ? 0 : (drawn.spans[drawn.spans.length - 1] as { end: number }).end;
  const x = r.width > 0 && laidOut > 0 ? ((e.clientX - r.left) / r.width) * laidOut : 0;
  const i = cellAt(drawn.spans, x);
  if (i === null) return null;
  const cell = drawn.cells[i] as MapCell;
  const bit = bitInCell(cell, drawn.spans[i] as { start: number; end: number }, x);
  return { part: partAt(drawn.segments, cell, bit), bit };
}

function rangeOf(s: Stretch, space: number): string {
  if (s.sizeBits <= 0) return formatAddress(s.offsetBits, space);
  return `${formatAddress(s.offsetBits, space)} – ${formatAddress(s.offsetBits + s.sizeBits - 8, space)}`;
}

/** A share of the whole, said the way the listing says it. */
export function shareWords(sizeBits: number, totalBits: number): string {
  if (totalBits <= 0) return "";
  const share = sizeBits / totalBits;
  return share > 0 && share < 0.01 ? "<1%" : `${Math.round(share * 100)}%`;
}

/** The stretch of bytes no part covers around `bit`, for the hover card. */
function holeAround(drawn: Drawn, bit: number): Stretch {
  const cell = drawn.cells.find((c) => bit >= c.start && bit < c.end);
  return cell === undefined ? { offsetBits: bit, sizeBits: 0 } : { offsetBits: cell.start, sizeBits: cell.end - cell.start };
}

function cardFor(drawn: Drawn, hit: { part: number; bit: number }): HTMLElement {
  const words = VIZ.locator;
  const seg = hit.part >= 0 ? (drawn.segments[hit.part] as MapSegment) : null;
  const s: Stretch = seg ?? holeAround(drawn, hit.bit);
  const share = shareWords(s.sizeBits, drawn.totalBits);
  const title = seg === null ? words.notInAnyPart : (seg.name ?? rangeOf(s, drawn.space));
  const lines = [`${rangeOf(s, drawn.space)} · ${bitSizeText(s.sizeBits)}`];
  if (share !== "") lines.push(drawn.space === 0 ? words.shareOfFile(share) : words.shareOfStream(share));
  return tipCard(title, lines);
}

/** Answer the pointer: the card, the shared hover, and the click. */
function live(strip: HTMLElement, drawn: Drawn, doc: Doc | undefined): void {
  const canGo = doc !== undefined || drawn.segments.some((s) => s.path !== undefined);
  if (canGo) strip.classList.add("fmap-live");
  /** Put the part under the pointer where a host that follows the click
   *  contract will find it. */
  const carry = (hit: { part: number; bit: number } | null): void => {
    const seg = hit === null || hit.part < 0 ? null : (drawn.segments[hit.part] as MapSegment);
    if (seg === null || seg.path === undefined) {
      strip.removeAttribute("data-reads");
      strip.removeAttribute("data-at");
      strip.removeAttribute("data-size");
      return;
    }
    strip.setAttribute("data-reads", seg.path.join("."));
    strip.setAttribute("data-at", String(seg.offsetBits));
    strip.setAttribute("data-size", String(seg.sizeBits));
  };
  strip.addEventListener("pointermove", (e) => {
    const hit = hitOf(strip, drawn, e);
    carry(hit);
    if (e.buttons !== 0 || hit === null) {
      hideTip();
      return;
    }
    tipAt(cardFor(drawn, hit), e.clientX, e.clientY);
    // The hex view marks the published hover, which is in the file's offsets.
    if (doc !== undefined && drawn.space === 0) {
      const seg = hit.part >= 0 ? (drawn.segments[hit.part] as MapSegment) : holeAround(drawn, hit.bit);
      linkedFor(doc).publish({ hover: { startBit: seg.offsetBits, endBit: seg.offsetBits + seg.sizeBits } });
    }
  });
  strip.addEventListener("pointerdown", (e) => {
    hideTip();
    carry(hitOf(strip, drawn, e));
  });
  strip.addEventListener("pointerleave", () => {
    hideTip();
    if (doc !== undefined) linkedFor(doc).publish({ hover: null });
  });
  if (doc === undefined) return;
  strip.addEventListener("click", (e) => {
    const hit = hitOf(strip, drawn, e);
    if (hit === null) return;
    e.stopPropagation();
    hideTip();
    const seg = hit.part >= 0 ? (drawn.segments[hit.part] as MapSegment) : null;
    if (seg !== null && seg.path !== undefined) {
      goToField(doc, { path: seg.path, startBit: seg.offsetBits, endBit: seg.offsetBits + seg.sizeBits });
      return;
    }
    // A part with no field to name it, or bytes between parts, in the file:
    // the cursor goes to the bytes themselves. A stream's bytes are not the
    // file's, and a part of one with no path has nowhere to send the cursor.
    if (drawn.space !== 0) return;
    const s = seg ?? holeAround(drawn, hit.bit);
    goToBits(doc, [{ startBit: s.offsetBits, endBit: s.offsetBits + s.sizeBits }]);
  });
}

/**
 * The small bar under a crumb of the inspector's trail: where that crumb's
 * field sits inside the crumb before it (`TrailPlace`, from `trail.ts`). The
 * crumb's own words say what it is; the card, shown while the pointer is on
 * the crumb, says where, in bytes and as a share of the crumb before. The
 * crumb is the click: it already goes to its field.
 */
export function crumbBar(crumb: HTMLElement, label: string, place: TrailPlace): HTMLElement {
  const bar = document.createElement("span");
  bar.className = "crumb-bar";
  bar.setAttribute("aria-hidden", "true");
  const lit = document.createElement("i");
  lit.style.insetInlineStart = `${(place.at[0] * 100).toFixed(2)}%`;
  lit.style.width = `${((place.at[1] - place.at[0]) * 100).toFixed(2)}%`;
  bar.append(lit);
  const card = (): HTMLElement => {
    const where: Stretch = { offsetBits: place.offsetBits, sizeBits: place.sizeBits };
    return tipCard(label, [`${rangeOf(where, place.space)} · ${bitSizeText(place.sizeBits)}`, VIZ.locator.shareOf(shareWords(place.sizeBits, place.parentBits), place.parent)]);
  };
  crumb.classList.add("has-bar");
  crumb.append(bar);
  crumb.addEventListener("pointermove", (e) => tipAt(card(), e.clientX, e.clientY));
  crumb.addEventListener("pointerleave", () => hideTip());
  crumb.addEventListener("pointerdown", () => hideTip());
  return crumb;
}

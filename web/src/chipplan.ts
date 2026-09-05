// What the annotation column says for the rows on screen, worked out before
// anything is drawn.
//
// A redraw usually wants the very chips that are already there, and building
// them again costs a button apiece; worse, it destroys the element a finger may
// be resting on, which the browser reads as the touch being taken away and
// cancels the drag that is scrolling the view. So the plan is settled here, as
// values, and the view writes it into the elements it already has. The keys
// below say when even that is not needed.
//
// Nothing in this file touches the document, which is what lets it be tested
// without one.

// `.ts` rather than the `.js` the rest of `src` writes: the tests run this
// file under `node --test`, which strips the types but does not rewrite a
// `.js` specifier back to the file it came from.
import type { Span } from "./doc.ts";
import { GAP_LABEL } from "./strings.ts";
import { chipDetail, chipLayout, chipWidth, runDetail, type ChipMeasure } from "./chipfit.ts";

/** A span named on a row, whether it started above the view, and the elements
 *  of its list it stands for when a run of them is drawn as one chip: empty
 *  for a chip that is one field. */
export type Chip = { span: Span; carried: boolean; run: Span[] };

/** What a chip says: the name in bold and the value after it. */
export type ChipText = { readonly name: string; readonly detail: string };

/** The chips one block of the column holds, and how many of them fit. What is
 *  past `shown` is counted rather than drawn. */
export type ChipBlock = { entries: Chip[]; texts: ChipText[]; shown: number };

/** The name a list gives its elements. */
const ELEMENT = /^\[\d+\]$/;

/** Whether a span is an element of a list that reads as one of many, so that
 *  a run of its siblings on one row can be one chip. Text is not: each string
 *  is worth reading. Nor is a structure that reads on one line, for the same
 *  reason, or a run the core has already folded. */
export function foldable(s: Span): boolean {
  return !s.gap && s.count === 0 && s.line === null && s.kind !== "str" && ELEMENT.test(s.name);
}

/** Whether two spans are elements of the same list, read the same way. */
export function sameList(a: Span, b: Span): boolean {
  return a.type === b.type && a.trail.length === b.trail.length && a.trail.every((t, i) => t === b.trail[i]);
}

/** The list an element belongs to, by name. */
export function listName(s: Span): string {
  return s.trail[s.trail.length - 1] ?? s.name;
}

/** The name as it is actually drawn. A chip for a field that began above the
 *  view is marked with an arrow by `.hv-chip-carried`, which is a CSS
 *  `::before` and so invisible to measuring the name's own text. Without it
 *  every carried chip is measured a character and a half short. */
export function carriedName(name: string, c: Chip | undefined): string {
  return c?.carried === true ? `↑ ${name}` : name;
}

/** What a chip says when it is drawn above the bytes it names, after its own
 *  value: the field started further up and runs on through this row. */
export function continuedDetail(detail: string): string {
  return detail === "" ? "continued" : `${detail} · continued`;
}

/** What a chip says. A run of list elements is named for the list and says
 *  how many; a structure that reads on one line is the whole chip, since
 *  `[47]` is the element's number in a repeat and says nothing, and the
 *  line says everything. */
export function chipText(c: Chip): ChipText {
  const s = c.span;
  if (c.run.length > 0) return { name: listName(s), detail: runDetail(c.run.length) };
  if (s.gap) return { name: GAP_LABEL, detail: chipDetail(s) };
  if (s.line !== null) return { name: s.line, detail: "" };
  return { name: s.name, detail: chipDetail(s) };
}

/** Where the spans on screen land: which one covers each byte, and which are
 *  named on each row. */
export type Placement = {
  /** The span covering each byte of the window, by index into `spans`, or -1. */
  readonly byteSpan: Int32Array;
  /** The chips named on each row on screen. */
  readonly byRow: Chip[][];
};

/**
 * Which span covers each byte, and which are named on each row. A field is
 * named on the row it starts on; one that started above the view is named on
 * the first row, so nothing on screen is left unexplained.
 */
export function placeChips(
  spans: readonly Span[],
  start: number,
  windowBytes: number,
  bpr: number,
  rows: number,
): Placement {
  const byteSpan = new Int32Array(windowBytes).fill(-1);
  const byRow: Chip[][] = Array.from({ length: rows }, () => []);
  for (const [i, s] of spans.entries()) {
    const from = Math.floor(s.offset_bits / 8);
    const to = Math.ceil((s.offset_bits + s.size_bits) / 8);
    for (let b = Math.max(from, start); b < Math.min(to, start + windowBytes); b++) {
      byteSpan[b - start] = i;
    }
    const row = from < start ? 0 : Math.floor((from - start) / bpr);
    if (row >= 0 && row < rows && to > start) {
      const chips = byRow[row] as Chip[];
      const prev = chips[chips.length - 1];
      // Elements of one list, one after another on the row, are one chip
      // saying how many: `[0]`, `[1]`, `[2]` say less than `cell_pointers
      // 3 values`. A chip carried from above the view stays its own, since
      // its arrow is about where it started.
      if (prev !== undefined && !prev.carried && from >= start && foldable(s) && foldable(prev.span) && sameList(prev.span, s)) {
        if (prev.run.length === 0) prev.run.push(prev.span);
        prev.run.push(s);
      } else {
        chips.push({ span: s, carried: from < start, run: [] });
      }
    }
  }
  return { byteSpan, byRow };
}

/**
 * Which piece of a cut row each chip belongs to. A cut row's chips go beside
 * the bytes they name. A run of list elements folded into one chip goes with
 * the first of them, since that is the byte the chip's arrow points at.
 */
export function bucketChips(chips: readonly Chip[], segs: readonly number[], rowStart: number): Chip[][] {
  const buckets: Chip[][] = segs.map(() => []);
  for (const c of chips) {
    let j = 0;
    if (!c.carried) {
      const at = Math.floor(c.span.offset_bits / 8) - rowStart;
      while (j + 1 < segs.length && (segs[j + 1] as number) <= at) j++;
    }
    (buckets[j] as Chip[]).push(c);
  }
  return buckets;
}

/** What one row's chips come to. */
export type RowChipPlan = {
  /** One block per piece of the row. */
  readonly blocks: ChipBlock[];
  /** The carried chips for the strip pinned over the rows, on the top row
   *  only; null on every other row, which carries nothing. */
  readonly pinned: ChipBlock | null;
  /** What the chips add to the row's height, over and above its lines of
   *  cells and the headings on it. */
  readonly extraHeight: number;
  /** How tall each block's chips are on their own, in pixels. The row's own
   *  height is not taken off here: a block that also holds a table of values
   *  is taller than its chips, and only the caller knows by how much. */
  readonly chipHeights: number[];
};

export type RowChipOpts = {
  readonly chips: readonly Chip[];
  /** Where the row is cut, as positions in it. Always starts with 0. */
  readonly segs: readonly number[];
  readonly rowStart: number;
  /** True for the top row, the only one that can carry a chip in from above. */
  readonly top: boolean;
  readonly noteWidth: number;
  readonly maxLines: number;
  readonly measure: ChipMeasure;
  /** True when the chips are drawn under the bytes rather than beside them. */
  readonly below: boolean;
  readonly rowHeight: number;
  readonly chipLine: number;
  /** How far the top row is scrolled up past the top edge. Zero on every
   *  other row, and on a top row sitting square against the edge. */
  readonly topPx?: number;
  /** How tall the headings above each piece of the row are. A heading is
   *  drawn over the piece it cuts the row before, so it is part of what a
   *  scroll has to travel to take that piece off the screen. */
  readonly headHeights?: readonly number[];
  /** How tall the table of a folded run's values is. It hangs under the
   *  chips of the row's first piece, so it too holds that piece on screen. */
  readonly valsHeight?: number;
};

/**
 * The first byte of the row that is still on screen.
 *
 * A heading cuts a row into pieces drawn one under another, so scrolling the
 * top row up past the edge takes its pieces away one at a time: the bytes
 * before the cut go first, and the bytes after it are what is left at the top
 * of the screen. `topPx` past the bottom of a piece means that piece is gone.
 *
 * `heights` are the pieces' own lines, `heads` what stands above each, both
 * in the order they are drawn.
 */
export function firstVisibleByte(
  rowStart: number,
  segs: readonly number[],
  heads: readonly number[],
  heights: readonly number[],
  topPx: number,
): number {
  let y = 0;
  for (let j = 0; j < segs.length; j++) {
    y += heads[j] ?? 0;
    const bottom = y + (heights[j] ?? 0);
    if (bottom > topPx) return rowStart + (segs[j] as number);
    y = bottom;
  }
  return rowStart + (segs[segs.length - 1] ?? 0);
}

/** The byte one past the last a span covers. */
const spanEnd = (s: Span): number => Math.ceil((s.offset_bits + s.size_bits) / 8);

/** Lay out one row's chips: which block each goes in, how many of each block
 *  fit, and what the lot adds to the row's height. */
export function planRowChips(o: RowChipOpts): RowChipPlan {
  const buckets = bucketChips(o.chips, o.segs, o.rowStart);
  // A field carried down from above the view is named by the strip
  // pinned over the top of the rows, not by the top row itself: a chip
  // under the top row would sit between the bytes it covers and the ones
  // after them, and a chip inside it would make that row taller than the
  // same row is anywhere else, so every row below it jumped a chip line
  // as the top row changed. Only the top row can carry anything, so this
  // is the one place a row's height would depend on where it fell.
  const carried = o.top ? (buckets[0] as Chip[]).filter((c) => c.carried) : [];
  if (o.top) buckets[0] = (buckets[0] as Chip[]).filter((c) => !c.carried);

  /** One block's chips, and how many lines of the column they take. */
  const layOut = (entries: Chip[]): { block: ChipBlock; lines: number } => {
    const texts = entries.map((c) => chipText(c));
    const { shown, lines } = chipLayout(
      texts.map((t, i) => chipWidth(carriedName(t.name, entries[i]), t.detail, o.measure)),
      o.noteWidth,
      o.maxLines,
    );
    return { block: { entries, texts, shown }, lines };
  };

  const blocks: ChipBlock[] = [];
  const chipHeights: number[] = [];
  for (let j = 0; j < o.segs.length; j++) {
    const { block, lines } = layOut(buckets[j] as Chip[]);
    chipHeights.push(lines * o.chipLine);
    blocks.push(block);
  }

  let pinned: ChipBlock | null = null;
  if (o.top) {
    // Which of the carried fields reach a byte that is actually on screen.
    // A row the reader has scrolled halfway up has lost the pieces above the
    // edge, and a field whose last byte was in one of them is not continuing
    // on to anything: saying so over bytes it does not cover reads as a field
    // starting where the next one does.
    const first = firstVisibleByte(
      o.rowStart,
      o.segs,
      o.segs.map((_, j) => (o.headHeights ?? [])[j] ?? 0),
      chipHeights.map((h, j) => lineHeight(o, h + (j === 0 ? (o.valsHeight ?? 0) : 0))),
      o.topPx ?? 0,
    );
    const reaching = carried.filter((c) => spanEnd(c.span) > first);
    const texts = reaching.map((c) => chipText(c));
    const { shown } = chipLayout(
      texts.map((t, i) => chipWidth(carriedName(t.name, reaching[i]), continuedDetail(t.detail), o.measure)),
      o.noteWidth,
      o.maxLines,
    );
    pinned = { entries: reaching, texts, shown };
    // More carried fields than a capped block can hold: the rest are
    // named below the bytes rather than dropped.
    if (shown < reaching.length) {
      const { block, lines } = layOut([...reaching.slice(shown), ...(buckets[0] as Chip[])]);
      blocks[0] = block;
      chipHeights[0] = lines * o.chipLine;
    }
  }

  // Beside the bytes the chips share their line's height with the cells, so
  // the line is the taller of the two. Below them the chips are their own
  // block and their lines add to it. The table of values is the caller's to
  // add: only it knows how tall the table came out.
  const extraHeight = chipHeights.reduce((n, h) => n + (o.below ? h : Math.max(0, h - o.rowHeight)), 0);
  return { blocks, pinned, extraHeight, chipHeights };
}

/** How tall one piece of a row is drawn, from what its block holds: beside
 *  the bytes the block shares the row's own line, below them it hangs under
 *  it. */
function lineHeight(o: RowChipOpts, blockHeight: number): number {
  return o.below ? o.rowHeight + blockHeight : Math.max(o.rowHeight, blockHeight);
}

/** What a row's chips say, as one string, so they are written again only when
 *  they would say something else. */
export function rowNoteKey(blocks: readonly ChipBlock[], trailer: boolean): string {
  const block = (b: ChipBlock | null): string =>
    b === null
      ? ""
      : // The first field that did not fit is named in the key too: the
        // count of them can stay the same while the names the "+N" chip
        // lists in its tooltip change.
        `${b.shown}/${b.entries.length}/${b.texts[b.shown]?.name ?? ""}~` +
        b.texts
          .slice(0, b.shown)
          .map((t, i) => `${b.entries[i]?.carried === true ? "^" : ""}${t.name}${t.detail}`)
          .join("");
  return `${trailer ? "+" : ""}${blocks.map(block).join("")}`;
}

/** The same, for the strip pinned over the top of the rows. Every chip in it
 *  is carried, so nothing marks which are. */
export function pinnedNoteKey(b: ChipBlock | null): string {
  return b === null
    ? ""
    : `${b.shown}/${b.entries.length}~` +
        b.texts
          .slice(0, b.shown)
          .map((t) => `${t.name}${t.detail}`)
          .join("");
}

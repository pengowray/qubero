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
};

/** Lay out one row's chips: which block each goes in, how many of each block
 *  fit, and what the lot adds to the row's height. */
export function planRowChips(o: RowChipOpts): RowChipPlan {
  const buckets = bucketChips(o.chips, o.segs, o.rowStart);
  let pinned: ChipBlock | null = null;
  // A field carried down from above the view is named by the strip
  // pinned over the top of the rows, not by the top row itself: a chip
  // under the top row would sit between the bytes it covers and the ones
  // after them, and a chip inside it would make that row taller than the
  // same row is anywhere else, so every row below it jumped a chip line
  // as the top row changed. Only the top row can carry anything, so this
  // is the one place a row's height would depend on where it fell.
  if (o.top) {
    const carried = (buckets[0] as Chip[]).filter((c) => c.carried);
    buckets[0] = (buckets[0] as Chip[]).filter((c) => !c.carried);
    const texts = carried.map((c) => chipText(c));
    const { shown } = chipLayout(
      texts.map((t, i) => chipWidth(carriedName(t.name, carried[i]), continuedDetail(t.detail), o.measure)),
      o.noteWidth,
      o.maxLines,
    );
    pinned = { entries: carried, texts, shown };
    // More carried fields than a capped block can hold: the rest are
    // named below the bytes rather than dropped.
    if (shown < carried.length) buckets[0] = [...carried.slice(shown), ...(buckets[0] as Chip[])];
  }
  const blocks: ChipBlock[] = [];
  const chipHeights: number[] = [];
  let extraHeight = 0;
  for (let j = 0; j < o.segs.length; j++) {
    const entries = buckets[j] as Chip[];
    const texts = entries.map((c) => chipText(c));
    const { shown, lines } = chipLayout(
      texts.map((t, i) => chipWidth(carriedName(t.name, entries[i]), t.detail, o.measure)),
      o.noteWidth,
      o.maxLines,
    );
    // Beside the bytes the chips share their line's height with the
    // cells, so the line is the taller of the two. Below them the chips
    // are their own block and their lines add to it.
    extraHeight += o.below ? lines * o.chipLine : Math.max(0, lines * o.chipLine - o.rowHeight);
    chipHeights.push(lines * o.chipLine);
    blocks.push({ entries, texts, shown });
  }
  return { blocks, pinned, extraHeight, chipHeights };
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

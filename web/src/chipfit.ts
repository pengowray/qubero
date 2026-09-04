/**
 * What a chip in the annotation column says, and how a row's chips lay out
 * in it: how many fit on how many lines, and how many are left over.
 *
 * The layout has to be worked out before any chip is drawn, so that what is
 * left over can be counted rather than quietly cut off, and so that the row's
 * height is known before the browser lays it out. That means measuring text
 * without a layout. The constants below are the padding, borders and gaps the
 * stylesheet draws around that text, and want changing with it; the text
 * itself is measured by the caller, which is the one that can see the fonts.
 */

// `.ts` on the strings import for the same reason `chipplan.ts` does it: the
// tests run this file under `node --test`, which strips types but does not
// rewrite a `.js` specifier back to the file it came from.
import type { Span } from "./doc.js";
import { bitSizeText, countText } from "./strings.ts";

/** Longest value shown on a chip before it is cut short. */
const CHIP_VALUE = 32;
/** Rough width of a character in the chip font, for working out how many chips
 *  fit before any of them are drawn. Only a stand-in: the name is bold sans
 *  and the value is mono, so one number cannot be right for both. The view
 *  measures the real fonts and passes the answer in; this is what is used
 *  before there is a chip on screen to read a font from. */
const CHIP_CHAR = 6.7;
/** Padding either side of a chip's text (`padding: 0 6px`) and the coloured
 *  edge (`border-left: 3px`). Mirrors `.hv-chip` in style.css. */
const CHIP_CHROME = 6 + 6 + 3;
/** The gap between a chip's name and its value (`gap: 6px` in `.hv-chip`). */
const CHIP_INNER = 6;
/** The gap between two chips side by side (`column-gap: 4px` on `.hv-note`). */
const CHIP_GAP = 4;
/** Room kept for the `+3` that counts what did not fit, so the count itself is
 *  never what pushes a chip off the row. */
const CHIP_REST = 44;
/** Width to assume before the column has been measured once. */
const CHIP_COLUMN_GUESS = 320;
/** How many lines of one row its chips may take before the rest is counted
 *  rather than shown. */
export const CHIP_LINES = 3;

/** What a chip says for a run of fields shown as one entry. The same words
 *  whether the core folded the run or the view did. `unit` is the format's own
 *  word for what it holds, where it has one: a deflate block coded symbols,
 *  and calling them values says less. */
export function runDetail(count: number, unit: string | null = null): string {
  return countText(count, unit ?? "value");
}

/** What a chip says after the name. A run of numbers says how many; raw bytes
 *  say how many, since the bytes themselves are already on the left. */
export function chipDetail(s: Span): string {
  if (s.count > 0) return runDetail(s.count, s.unit);
  if (s.gap || s.kind === "bytes" || s.value === "") return bitSizeText(s.size_bits);
  return s.value.length > CHIP_VALUE ? `${s.value.slice(0, CHIP_VALUE)}…` : s.value;
}

/**
 * How the two runs of text on a chip are measured. The name is drawn bold
 * sans and the value mono, so they are measured apart. The view builds one of
 * these from a canvas and the chips' own computed fonts; `GUESS_TEXT` stands
 * in until there is a chip on screen to read a font from.
 */
export type ChipMeasure = {
  readonly name: (s: string) => number;
  readonly value: (s: string) => number;
};

/** Character counting, for before the real fonts are known. */
export const GUESS_TEXT: ChipMeasure = {
  name: (s) => s.length * CHIP_CHAR,
  value: (s) => s.length * CHIP_CHAR,
};

/** How wide a chip saying `name` and `detail` will be drawn, near enough to
 *  choose by. Rounded up, since a fraction under the column's own whole
 *  pixels is what makes the difference between three lines and four. */
export function chipWidth(name: string, detail: string, text: ChipMeasure = GUESS_TEXT): number {
  const inner = detail === "" ? 0 : CHIP_INNER + text.value(detail);
  return Math.ceil(CHIP_CHROME + text.name(name) + inner);
}

export type ChipLayout = {
  /** How many of the chips are drawn. */
  readonly shown: number;
  /** How many lines they take. Zero only when there are no chips. */
  readonly lines: number;
};

/**
 * Lay a row's chips, given their widths, into a column `width` pixels across
 * and at most `maxLines` lines tall. A chip that does not fit beside the last
 * one starts the next line; once the last line is full, what is left is
 * counted rather than drawn, and room is kept on that line for the count.
 *
 * The first chip on a line is always drawn, even in a column too narrow for
 * it: a row where a field starts says so even when it cannot say what, since
 * the alternative is a row that looks empty when it is not.
 *
 * `width` of zero means the column has not been measured yet and a guess
 * stands in; the caller redraws once the real width disagrees with it.
 *
 * `maxLines` of `Infinity` means the row grows to hold every chip: nothing is
 * ever left over, so nothing is ever counted.
 */
export function chipLayout(widths: readonly number[], width: number, maxLines = CHIP_LINES): ChipLayout {
  const column = width || CHIP_COLUMN_GUESS;
  let room = column;
  let lines = widths.length > 0 ? 1 : 0;
  let onLine = 0;
  let shown = 0;
  for (const [i, w] of widths.entries()) {
    const last = i === widths.length - 1;
    // The last chip needs no room kept after it, since there is nothing left
    // for a `+N` to count.
    const keep = lines === maxLines && !last ? CHIP_REST : 0;
    // A chip after the first on its line brings the gap between them with it.
    const need = onLine > 0 ? w + CHIP_GAP : w;
    if (onLine > 0 && need > room - keep) {
      if (lines === maxLines) break;
      lines += 1;
      room = column - w;
      onLine = 1;
      shown += 1;
      continue;
    }
    room -= need;
    onLine += 1;
    shown += 1;
  }
  return { shown, lines };
}

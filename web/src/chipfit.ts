/**
 * What a chip in the annotation column says, and how a row's chips lay out
 * in it: how many fit on how many lines, and how many are left over.
 *
 * The layout has to be worked out before any chip is drawn, so that what is
 * left over can be counted rather than quietly cut off, and so that the row's
 * height is known before the browser lays it out. That means measuring text
 * without a layout, which is what the width constants below are for: they are
 * the chip font's own numbers and want changing with it.
 */

import type { Span } from "./doc.js";

/** Longest value shown on a chip before it is cut short. */
const CHIP_VALUE = 32;
/** Rough width of a character in the chip font, for working out how many chips
 *  fit before any of them are drawn. */
const CHIP_CHAR = 6.7;
/** Padding, border and gap around a chip's text. */
const CHIP_CHROME = 20;
/** The most characters a chip is drawn with, from the stylesheet's
 *  `max-width: 26ch`. A longer name is cut short there, so measuring it in
 *  full would send a chip that fits to the next line. */
const CHIP_MAX_CHARS = 26;
/** Room kept for the `+3` that counts what did not fit, so the count itself is
 *  never what pushes a chip off the row. */
const CHIP_REST = 44;
/** Width to assume before the column has been measured once. */
const CHIP_COLUMN_GUESS = 320;
/** How many lines of one row its chips may take before the rest is counted
 *  rather than shown. */
export const CHIP_LINES = 3;

/** What a chip says for a run of fields shown as one entry. The same words
 *  whether the core folded the run or the view did. */
export function runDetail(count: number): string {
  return `${count.toLocaleString()} values`;
}

/** What a chip says after the name. A run of numbers says how many; raw bytes
 *  say how many, since the bytes themselves are already on the left. */
export function chipDetail(s: Span): string {
  if (s.count > 0) return runDetail(s.count);
  if (s.gap || s.kind === "bytes") {
    return s.size_bits % 8 === 0
      ? `${(s.size_bits / 8).toLocaleString()} bytes`
      : `${s.size_bits.toLocaleString()} bits`;
  }
  return s.value.length > CHIP_VALUE ? `${s.value.slice(0, CHIP_VALUE)}…` : s.value;
}

/** How wide a chip saying `name` and `detail` will be drawn, near enough to
 *  choose by. */
export function chipWidth(name: string, detail: string): number {
  const chars = Math.min(CHIP_MAX_CHARS, name.length + detail.length + (detail === "" ? 0 : 1));
  return CHIP_CHROME + chars * CHIP_CHAR;
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
    if (onLine > 0 && w > room - keep) {
      if (lines === maxLines) break;
      lines += 1;
      room = column;
      onLine = 0;
    }
    room -= w;
    onLine += 1;
    shown += 1;
  }
  return { shown, lines };
}

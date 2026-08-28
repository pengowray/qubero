/**
 * What a chip in the annotation column says, and how many of them fit on a row.
 *
 * The count has to be worked out before any chip is drawn, so that what is left
 * over can be counted rather than quietly cut off. That means measuring text
 * without a layout, which is what the two width constants below are for: they
 * are the chip font's own numbers and want changing with it.
 */

import type { Span } from "./doc.js";

/** Longest value shown on a chip before it is cut short. */
const CHIP_VALUE = 32;
/** Rough width of a character in the chip font, for working out how many chips
 *  fit before any of them are drawn. */
const CHIP_CHAR = 6.7;
/** Padding, border and gap around a chip's text. */
const CHIP_CHROME = 20;
/** Room kept for the `+3` that counts what did not fit, so the count itself is
 *  never what pushes a chip off the row. */
const CHIP_REST = 44;
/** Width to assume before the column has been measured once. */
const CHIP_COLUMN_GUESS = 320;

/** What a chip says after the name. A run of numbers says how many; raw bytes
 *  say how many, since the bytes themselves are already on the left. */
export function chipDetail(s: Span): string {
  if (s.count > 0) return `${s.count.toLocaleString()} values`;
  if (s.gap || s.kind === "bytes") {
    return s.size_bits % 8 === 0
      ? `${(s.size_bits / 8).toLocaleString()} bytes`
      : `${s.size_bits.toLocaleString()} bits`;
  }
  return s.value.length > CHIP_VALUE ? `${s.value.slice(0, CHIP_VALUE)}…` : s.value;
}

/** How wide a chip for this span will be drawn, near enough to choose by. */
export function chipWidth(s: Span): number {
  return CHIP_CHROME + (s.name.length + chipDetail(s).length + 1) * CHIP_CHAR;
}

/**
 * How many of a row's chips fit in a column `width` pixels across. Zero only
 * when the row has nothing on it: a row where a field starts says so even in a
 * column too narrow to say what, since the alternative is a row that looks
 * empty when it is not.
 *
 * `width` of zero means the column has not been measured yet and a guess
 * stands in; the caller redraws once the real width disagrees with it.
 */
export function chipsThatFit(spans: readonly Span[], width: number): number {
  let room = width || CHIP_COLUMN_GUESS;
  let shown = 0;
  for (const s of spans) {
    const w = chipWidth(s);
    // The last chip on the row needs no room kept after it, since there is
    // nothing left for a `+N` to count.
    if (shown > 0 && w > room - (shown < spans.length - 1 ? CHIP_REST : 0)) break;
    room -= w;
    shown += 1;
  }
  return shown;
}

/**
 * What the Diagram view says about the open file's counts, decided apart from
 * the drawing so each rule can be tested without a page.
 *
 * The drawing is of the format and the counts are of the file, laid over it.
 * Three rules make the overlay honest:
 *
 * - **A count that has not finished is a floor, and says so.** Every badge
 *   gets a `+` until the count is done, including 0 and 1, which a finished
 *   count leaves bare. Nothing is drawn faded as "none of these" until the
 *   count is done: a box nothing has been found for yet is not a box the file
 *   has none of.
 * - **A row's badge is the exception to its box's.** A field of a structure
 *   is there once per structure, so a row counted as often as its box says
 *   nothing the box's badge has not; only a row that differs is badged, and
 *   the eye lands on the optional fields and the cases taken by some.
 * - **One gesture goes to the hex view,** and it is a double click, the
 *   app's convention for anything that switches views. A single click only
 *   marks the row.
 */

import type { CensusState, DiagramCensus } from "./doc.ts";
import { DIAGRAM } from "./strings.ts";

/**
 * How much of a file the Diagram view counts without being asked.
 *
 * A file under `wholeFileUnderBytes` is counted to the end, in the background,
 * a go at a time. Measured natively on 2026-09-14, a count of the whole of a
 * 48 MB EPUB is 275,000 fields in about two seconds and a 200 KB ROOT file
 * 183,000 fields in under three, holding a few thousand nodes at a time; the
 * browser is slower by a small factor, and the page stays usable throughout.
 *
 * A larger file is counted as far as `fieldsBeforeAsking` and then stops, with
 * a button to count the rest. The size is the reader's own measure of what
 * they opened, and a file of gigabytes can hold tens of millions of fields,
 * which is minutes of work nobody asked for by opening a view. The fields
 * number is about what the whole-file count above does in a couple of
 * seconds.
 */
export const AUTO_COUNT = {
  wholeFileUnderBytes: 50 * 1024 * 1024,
  fieldsBeforeAsking: 200_000,
} as const;

/** No limit, as the core's count takes one: the largest u32. */
export const NO_LIMIT = 0xffff_ffff;

/** How many fields to let the count walk, for a file of `fileBytes` bytes and
 *  whether the reader has asked it to carry on. */
export function countLimit(fileBytes: number, keepCounting: boolean): number {
  return keepCounting || fileBytes < AUTO_COUNT.wholeFileUnderBytes ? NO_LIMIT : AUTO_COUNT.fieldsBeforeAsking;
}

/** How long one turn of the page may spend counting before it gives the page
 *  back, in milliseconds. A go of the count is usually a few milliseconds and
 *  can be a hundred, so this asks for as many as fit in a frame or so. */
export const COUNT_TURN_MS = 12;

/** How often the drawing is laid out again while a count is running. A layout
 *  moves every box a badge widens, so once a second is as often as a reader
 *  can follow, and no more often than the layout can afford. */
export const REDRAW_MS = 1000;

/** The three states a reader can tell apart: waiting for bytes is still
 *  counting as far as the drawing is concerned. */
type Phase = "done" | "counting" | "stopped";

function phase(state: CensusState): Phase {
  return state === "done" ? "done" : state === "capped" ? "stopped" : "counting";
}

/** What a box's badge says, or null for no badge. */
export function boxBadge(n: number, state: CensusState): string | null {
  if (state === "done") return n > 1 ? DIAGRAM.count(n) : null;
  return DIAGRAM.countSoFar(n);
}

/**
 * What a row's badge says, or null for no badge: only when the row's count
 * differs from its box's. A finished count of 0 is drawn faded instead.
 */
export function rowBadge(row: number, box: number, state: CensusState): string | null {
  if (row === box) return null;
  if (state === "done") return row === 0 ? null : DIAGRAM.count(row);
  return DIAGRAM.countSoFar(row);
}

/** Whether a box or row is drawn faded as one this file has none of. */
export function isUnused(n: number, state: CensusState): boolean {
  return state === "done" && n === 0;
}

/** The hover for a count of a box or of a field row, in whichever state. */
export function countTitle(n: number, what: string, c: DiagramCensus): string {
  switch (phase(c.state)) {
    case "done":
      return n === 0 ? DIAGRAM.unusedTitle : DIAGRAM.countTitle(n, what);
    case "stopped":
      return n === 0 ? DIAGRAM.noneInFirst(c.walked) : DIAGRAM.countTitleStopped(n, what, c.walked);
    case "counting":
      return n === 0 ? DIAGRAM.noneYet : DIAGRAM.countTitleCounting(n, what);
  }
}

/** The same for a switch's row, which is a case: `value` is the value the
 *  switch reads for it. */
export function caseCountTitle(value: string, n: number, c: DiagramCensus): string {
  switch (phase(c.state)) {
    case "done":
      return n === 0 ? DIAGRAM.unusedTitle : DIAGRAM.caseTitle(value, n);
    case "stopped":
      return n === 0 ? DIAGRAM.noneInFirst(c.walked) : DIAGRAM.caseTitleStopped(value, n, c.walked);
    case "counting":
      return n === 0 ? DIAGRAM.noneYet : DIAGRAM.caseTitleCounting(value, n);
  }
}

/** What the toolbar says about the count, and whether the button that carries
 *  a stopped count on is shown. `started` is true once a count has been asked
 *  for, so the line can say it is counting before the first answer. */
export function countStatus(c: DiagramCensus | null, started: boolean): { readonly text: string; readonly keep: boolean } {
  if (c === null) return { text: started ? DIAGRAM.counting(0) : "", keep: false };
  switch (phase(c.state)) {
    case "done":
      return { text: "", keep: false };
    case "stopped":
      return { text: DIAGRAM.stopped(c.walked), keep: true };
    case "counting":
      return { text: DIAGRAM.counting(c.walked), keep: false };
  }
}

/** The toggle's hover, which owns up to hiding what has not been found yet. */
export function onlyUsedTitle(c: DiagramCensus | null): string {
  return c === null || c.state === "done" ? DIAGRAM.onlyUsedTitle : DIAGRAM.onlyUsedTitle + DIAGRAM.onlyUsedTitleSoFar;
}

/**
 * How long to wait before laying the drawing out again for a count that has
 * just arrived: 0 for now, a number of milliseconds, or null when nothing the
 * reader would see has changed.
 *
 * A finished or stopped count is drawn at once. A running one is drawn at most
 * once every `REDRAW_MS`, and the first of them waits a whole interval from
 * when counting began, so a small file that finishes inside it never flashes
 * a drawing of `×0+` badges on its way to the real one.
 */
export function redrawIn(
  shown: DiagramCensus | null,
  incoming: DiagramCensus | null,
  now: number,
  lastDrawn: number,
  countingSince: number,
): number | null {
  if (incoming === null) return shown === null ? null : 0;
  if (shown !== null && phase(shown.state) === phase(incoming.state) && shown.walked === incoming.walked) return null;
  if (phase(incoming.state) !== "counting") return 0;
  const since = shown === null ? countingSince : lastDrawn;
  return Math.max(0, since + REDRAW_MS - now);
}

/** Where a double click on a row goes. */
export type GoTarget =
  /** A field of the file's first structure, which `main.ts` finds by row. */
  | { readonly kind: "pick" }
  /** The first node the count found for this box or row. */
  | { readonly kind: "path"; readonly path: readonly number[] }
  /** The first one is inside an unpacked stream, which has no offsets in the
   *  file, so there is nowhere to go; the row says why. */
  | { readonly kind: "stream" };

/**
 * Where a double click on a row takes the reader, or null when it does
 * nothing. A row of the first box that the open file has is the field itself,
 * which is exact where the count's first one is only as good as the count;
 * every other row goes to the first one counted.
 */
export function goTarget(pickable: boolean, first: { readonly first_path: readonly number[]; readonly space: number } | undefined): GoTarget | null {
  if (pickable) return { kind: "pick" };
  if (first === undefined) return null;
  return first.space === 0 ? { kind: "path", path: first.first_path } : { kind: "stream" };
}

/** Sentences joined for one hover: a full stop between two, unless the first
 *  already ends in one. Empty ones are left out. */
export function joinTitle(...parts: readonly (string | null | undefined)[]): string {
  let out = "";
  for (const p of parts) {
    if (p === null || p === undefined || p === "") continue;
    if (out === "") out = p;
    else out += /[.!?…]$/.test(out) ? ` ${p}` : `. ${p}`;
  }
  return out;
}

/**
 * When a list too long to show whole is folded behind a "… N more" row.
 *
 * The fold row takes the room of a row, so a fold that hides one row saves
 * nothing and makes the reader click for a line that would have fitted where
 * the fold is drawn: a Diagram box of 25 fields read "… 1 more fields". A
 * fold is only worth drawing when it hides enough to be worth a row of its
 * own.
 */

/**
 * How many rows past the cap are shown rather than folded.
 *
 * Three, because a fold that hides two or three rows still costs a click to
 * save as much room as the fold row itself takes, and a list a few rows over
 * its cap reads at a glance the same as one at it. Past that the fold starts
 * to earn its place.
 */
export const FOLD_SLACK = 3;

/**
 * How many of `total` rows to show when `cap` of them fit before a fold: all of
 * them when folding would hide no more than `FOLD_SLACK`, else `cap`.
 */
export function shownBeforeFold(total: number, cap: number): number {
  return total <= cap + FOLD_SLACK ? total : Math.min(total, cap);
}

/** Whether a list of `total` rows with `cap` before its fold is folded at all. */
export function folds(total: number, cap: number): boolean {
  return shownBeforeFold(total, cap) < total;
}

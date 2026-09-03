// The row window kept in the DOM around the text viewport.
//
// Decoding ahead is not enough to make native scrolling smooth: the browser
// can only composite rows which already exist. Keep eight complete viewports
// painted on either side, and refill only after half of that runway has been
// consumed. The rows themselves are cheap text runs rather than one element
// per character, so this is a small DOM even when the file has long lines.

export type PaintWindow = {
  readonly first: number;
  readonly count: number;
  readonly runway: number;
};

export type RowPosition = {
  readonly line: number;
  readonly offset: number;
};

/** Move a full-height row viewport by pixels, retaining the sub-row part. */
export function moveRows(line: number, offset: number, pixels: number, rowHeight: number, lastTop: number): RowPosition {
  const height = Math.max(1, rowHeight);
  const total = offset + pixels;
  const lines = Math.floor(total / height);
  const next = Math.max(0, Math.min(lastTop, line + lines));
  if (next === 0 && line + lines < 0) return { line: 0, offset: 0 };
  const remainder = total - lines * height;
  if (next === lastTop && (line + lines > lastTop || remainder > 0)) return { line: lastTop, offset: 0 };
  return { line: next, offset: remainder };
}

/** The line range which should exist in the DOM for this viewport. */
export function paintWindow(viewLine: number, visible: number): PaintWindow {
  const rows = Math.max(1, visible);
  const runway = 8 * rows;
  return {
    first: Math.max(0, viewLine - runway),
    count: rows + 2 * runway,
    runway,
  };
}

/**
 * Whether a painted row window should be recentered.
 *
 * The file boundaries count as unlimited runway: there cannot be rows to
 * reveal beyond them. Away from a boundary we repaint while four viewports of
 * runway remain, leaving the browser ample rows to scroll
 * through while that work completes.
 */
export function needsPaint(
  viewLine: number,
  visible: number,
  first: number,
  count: number,
  runway: number,
  atStart: boolean,
  atEnd: boolean,
): boolean {
  const above = viewLine - first;
  const below = first + count - (viewLine + Math.max(1, visible));
  if (above < 0 || below < 0) return true;
  const reserve = Math.max(1, Math.floor(runway / 2));
  return (!atStart && above < reserve) || (!atEnd && below < reserve);
}

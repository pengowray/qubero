// How tall each row of a virtualised view is, so the view can be scrolled by
// pixels rather than by whole rows.
//
// Rows are mostly one height, and the exceptions come from two directions:
//
//  - Structural extras are known before anything is drawn. In the hex view a
//    section heading above a row, and the extra line of cells a row is cut
//    into so a heading can sit between its bytes, are both worked out from the
//    parts of the file. A file can have a hundred thousand of them, so they
//    are held as a sorted array with prefix sums built in one pass and queried
//    by bisection.
//  - Measured extras are only known once the browser has laid a row out: how
//    many lines a row's chips wrapped to. Only rows that have been on screen
//    have one, so they are a sparse map, capped, and their prefix sums are
//    rebuilt lazily when something has changed them.
//
// Nothing here touches the DOM. The listing's `tops[]` and the text view's
// `lineY`/`lineAtY` answer the same three questions this does — where a row
// starts, how tall everything is, which row is at a pixel — and could be moved
// onto this ledger later. They are not wired to it now.

/** A row's extra height above the base, known before it is drawn. */
export type StructuralExtra = { readonly row: number; readonly extra: number };

/** How many measured rows are kept. Past this the ones furthest from where the
 *  reader is looking are dropped: they are an improvement on the estimate, not
 *  a fact the view needs, and a map that grew with every row ever scrolled
 *  past would grow without end. */
const MEASURED_CAP = 4000;

export class RowHeights {
  private base = 20;
  private rows = 0;
  /** Rows with a structural extra, ascending. */
  private structRows: Int32Array = new Int32Array(0);
  private structExtra: Float64Array = new Float64Array(0);
  /** `structSum[i]` is the total structural extra of `structRows[0..i)`. */
  private structSum: Float64Array = new Float64Array(1);
  /** Row -> measured extra, over and above base plus structural. Never
   *  negative: a ledger that could shrink a row below its base would make
   *  `heightBefore` non-monotone, and the bisection in `rowAtY` depends on it
   *  rising. */
  private measured = new Map<number, number>();
  /** The measured rows in ascending order with their prefix sums, rebuilt when
   *  `measuredDirty`. */
  private mRows: Int32Array = new Int32Array(0);
  private mSum: Float64Array = new Float64Array(1);
  private measuredDirty = false;

  /** The height every row has before any extra. */
  setBase(px: number): void {
    if (px > 0 && px !== this.base) this.base = px;
  }

  get baseHeight(): number {
    return this.base;
  }

  /** How many rows there are. */
  setRows(n: number): void {
    this.rows = Math.max(0, Math.floor(n));
  }

  get rowCount(): number {
    return this.rows;
  }

  /** Replace the structural extras. `entries` need not be sorted and entries
   *  for the same row add up. */
  setStructural(entries: readonly StructuralExtra[]): void {
    const kept = entries.filter((e) => e.extra > 0 && e.row >= 0);
    kept.sort((a, b) => a.row - b.row);
    const rows = new Int32Array(kept.length);
    const extra = new Float64Array(kept.length);
    let n = 0;
    for (const e of kept) {
      if (n > 0 && rows[n - 1] === e.row) {
        extra[n - 1] = (extra[n - 1] as number) + e.extra;
        continue;
      }
      rows[n] = e.row;
      extra[n] = e.extra;
      n++;
    }
    this.structRows = rows.subarray(0, n);
    this.structExtra = extra.subarray(0, n);
    const sum = new Float64Array(n + 1);
    for (let i = 0; i < n; i++) sum[i + 1] = (sum[i] as number) + (extra[i] as number);
    this.structSum = sum;
  }

  /** Forget every structural extra. */
  clearStructural(): void {
    this.setStructural([]);
  }

  /** Forget every measured extra: the rows will be measured again as they are
   *  drawn. */
  clearMeasured(): void {
    if (this.measured.size === 0) return;
    this.measured.clear();
    this.measuredDirty = true;
  }

  /** Forget everything but the base height and the row count. */
  clearAll(): void {
    this.clearStructural();
    this.clearMeasured();
  }

  /** Record how tall a row actually came out. The base and its structural
   *  extra are taken off here, so the caller passes the height it measured. */
  measure(row: number, realPx: number): void {
    if (row < 0 || !Number.isFinite(realPx)) return;
    const extra = Math.max(0, realPx - this.base - this.structuralOf(row));
    const had = this.measured.get(row);
    if (had === extra) return;
    if (extra === 0 && had === undefined) return;
    this.measured.set(row, extra);
    this.measuredDirty = true;
  }

  /** Whether a row has been measured. */
  hasMeasured(row: number): boolean {
    return this.measured.has(row);
  }

  /**
   * Drop measured rows down to the cap, keeping the ones nearest `focus`.
   * Called by the view once a draw is over, since dropping them mid-draw would
   * move the ground under the row being drawn.
   *
   * The first and last rows are never dropped. The last one is what says how
   * far down the file goes, so forgetting it would let the end of the scrollbar
   * drift once the reader had been away from it; the first is what a jump back
   * to the top of the file lands on.
   */
  trim(focus: number): void {
    if (this.measured.size <= MEASURED_CAP) return;
    const last = this.rows - 1;
    const rows = [...this.measured.keys()].filter((r) => r !== 0 && r !== last);
    rows.sort((a, b) => Math.abs(a - focus) - Math.abs(b - focus));
    const keep = Math.max(0, MEASURED_CAP - (this.measured.size - rows.length));
    for (let i = keep; i < rows.length; i++) this.measured.delete(rows[i] as number);
    this.measuredDirty = true;
  }

  /** The structural extra of one row. */
  structuralOf(row: number): number {
    const i = lowerBound(this.structRows, row);
    return this.structRows[i] === row ? (this.structExtra[i] as number) : 0;
  }

  /** How tall a row is, as well as the ledger knows. */
  heightOf(row: number): number {
    if (row < 0 || row >= this.rows) return this.base;
    return this.base + this.structuralOf(row) + (this.measured.get(row) ?? 0);
  }

  /** Where a row's top edge sits, in pixels from the top of the file. Rows past
   *  the end are counted at the base height, so the answer keeps rising. */
  heightBefore(row: number): number {
    const r = Math.max(0, Math.floor(row));
    const capped = Math.min(r, this.rows);
    let y = r * this.base;
    y += this.structSum[lowerBound(this.structRows, capped)] as number;
    this.rebuild();
    y += this.mSum[lowerBound(this.mRows, capped)] as number;
    return y;
  }

  /** How tall every row together is. */
  totalHeight(): number {
    return this.heightBefore(this.rows);
  }

  /**
   * The row at a pixel, and how far into it the pixel falls. Clamped to the
   * file: a negative `y` is the top of the first row, and a `y` past the end is
   * the top of the last one.
   */
  rowAtY(y: number): { row: number; offsetPx: number } {
    if (this.rows === 0) return { row: 0, offsetPx: 0 };
    if (y <= 0) return { row: 0, offsetPx: 0 };
    const total = this.totalHeight();
    if (y >= total) {
      const last = this.rows - 1;
      return { row: last, offsetPx: Math.max(0, Math.min(this.heightOf(last), y - this.heightBefore(last))) };
    }
    // `heightBefore` never falls, so the first row whose top is past `y` is
    // found by bisection and the row wanted is the one before it.
    let lo = 0;
    let hi = this.rows;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (this.heightBefore(mid + 1) <= y) lo = mid + 1;
      else hi = mid;
    }
    const row = Math.min(lo, this.rows - 1);
    return { row, offsetPx: y - this.heightBefore(row) };
  }

  private rebuild(): void {
    if (!this.measuredDirty) return;
    this.measuredDirty = false;
    const rows = Int32Array.from(this.measured.keys()).sort();
    const sum = new Float64Array(rows.length + 1);
    for (let i = 0; i < rows.length; i++) {
      sum[i + 1] = (sum[i] as number) + (this.measured.get(rows[i] as number) as number);
    }
    this.mRows = rows;
    this.mSum = sum;
  }
}

/** The first index of `arr` whose value is at least `v`. */
function lowerBound(arr: Int32Array, v: number): number {
  let lo = 0;
  let hi = arr.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((arr[mid] as number) < v) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

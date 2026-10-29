// The arithmetic behind the run sparkline (`sparkline.ts`): which records of
// a list hold the same field, the values read out of them, the min-max
// envelope a long run is drawn as, and the short numbers written at the two
// ends.
//
// A field that repeats once per record (a chunk's length, an event's delta
// time, an entry's size) is one value in each record's rows, and reading
// them one row at a time says nothing about the run. The sparkline is the run
// on one line: every record's value, left to right in file order. A run longer
// than the line is pixels wide is drawn as the smallest and largest value in
// each pixel's stretch of records, which is what keeps a spike in a run of a
// hundred thousand from being averaged away.
//
// Kept apart from the drawing so that `node --test` can run it with no page.

/** One step of the way from a record down to the field: the child's name and
 *  type, which is how the same field is found in another record. Index alone
 *  would find a different field in a record of another kind (a MIDI meta
 *  event and a note event both have four children). */
export type Step = { readonly name: string; readonly type: string };

/** A value written as the core writes a number for editing: decimal, or hex
 *  with `0x`, perhaps with a sign. Null for anything else, which is a field
 *  the sparkline does not draw. */
export function parseNumber(text: string): number | null {
  const t = text.trim().replace(/_/g, "");
  if (t === "") return null;
  if (/^[-+]?0x[0-9a-f]+$/i.test(t)) {
    const neg = t.startsWith("-");
    const v = Number.parseInt(t.replace(/^[-+]/, ""), 16);
    return Number.isFinite(v) ? (neg ? -v : v) : null;
  }
  const v = Number(t);
  return Number.isFinite(v) ? v : null;
}

export type Envelope = {
  /** The smallest and largest value in each bin. */
  readonly min: readonly number[];
  readonly max: readonly number[];
  /** The first record in each bin, so a point on the line goes back to a
   *  record. */
  readonly first: readonly number[];
  /** True when there is a bin per record and min equals max throughout. */
  readonly exact: boolean;
};

/**
 * `values` in at most `bins` bins, each the smallest and largest of the
 * records in it. A run no longer than `bins` is one bin per record.
 */
export function envelope(values: readonly number[], bins: number): Envelope {
  const n = values.length;
  const b = Math.max(1, Math.floor(bins));
  if (n <= b) return { min: [...values], max: [...values], first: values.map((_, i) => i), exact: true };
  const min: number[] = [];
  const max: number[] = [];
  const first: number[] = [];
  for (let j = 0; j < b; j++) {
    const from = Math.floor((j * n) / b);
    const to = Math.max(from + 1, Math.floor(((j + 1) * n) / b));
    let lo = Number.POSITIVE_INFINITY;
    let hi = Number.NEGATIVE_INFINITY;
    for (let i = from; i < to; i++) {
      const v = values[i] as number;
      if (v < lo) lo = v;
      if (v > hi) hi = v;
    }
    min.push(lo);
    max.push(hi);
    first.push(from);
  }
  return { min, max, first, exact: false };
}

/** Which bin of `envelope(values, bins)` record `i` of `n` is in. */
export function binOf(i: number, n: number, bins: number): number {
  const b = Math.max(1, Math.floor(bins));
  if (n <= b) return i;
  return Math.min(b - 1, Math.floor((i * b) / n));
}

/** The records a bin stands for, end exclusive. */
export function binRecords(j: number, n: number, bins: number): { readonly from: number; readonly to: number } {
  const b = Math.max(1, Math.floor(bins));
  if (n <= b) return { from: j, to: j + 1 };
  const from = Math.floor((j * n) / b);
  return { from, to: Math.max(from + 1, Math.floor(((j + 1) * n) / b)) };
}

/** A number short enough for the end of a sparkline: `768`, `12.3k`,
 *  `4.29G`. Three significant figures past a thousand, which says how big
 *  without claiming the exact value, which is in the hover card. */
export function shortNumber(v: number): string {
  const a = Math.abs(v);
  if (a < 1000) {
    if (Number.isInteger(v)) return String(v);
    return String(Number(v.toPrecision(3)));
  }
  const units: readonly [number, string][] = [
    [1e12, "T"],
    [1e9, "G"],
    [1e6, "M"],
    [1e3, "k"],
  ];
  for (const [size, mark] of units) {
    if (a >= size) return `${Number((v / size).toPrecision(3))}${mark}`;
  }
  return String(v);
}

/** Whether a run is worth a line: at least three values, and not all the
 *  same. A field every record agrees on is one value, which its row says. */
export function worthDrawing(values: readonly number[]): boolean {
  if (values.length < 3) return false;
  const v0 = values[0];
  return values.some((v) => v !== v0);
}

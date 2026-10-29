// Lanes for things drawn over one axis: braces under a byte strip, brackets
// over a token stream, the several readings of one run of bytes.
//
// Two of them that overlap in x must never share a y, or one is drawn over
// the other and neither can be read. Nudging the second one down by hand is
// what `hexlinks.ts` and the Diagram view's role words do, and it is the known
// defect: it works for the two the author tried and collides on the third.
// This is the textbook answer instead, interval colouring: sort by where each
// one starts and put each in the first lane whose last occupant has ended.
// For intervals on a line that greedy order is optimal, so the number of lanes
// is the most that overlap at any one x and never more.
//
// DOM-free and in whatever unit the caller draws in. A caller in pixels passes
// a gap, so two braces that touch end to start still get a hair between them
// rather than reading as one long brace.

/** A stretch of the axis, half open: `end` is one past the last unit. A
 *  stretch with `end <= start` is a point, and shares a lane with anything
 *  that starts where it is; give a drawn glyph the width it is drawn at. */
export type Span = { readonly start: number; readonly end: number };

export type Lanes = {
  /** The lane of each item, in the order the items were given. Lane 0 is the
   *  one nearest the axis. */
  readonly lane: readonly number[];
  /** How many lanes were used: the height the caller has to find room for. */
  readonly count: number;
};

/**
 * Put each item in the lowest lane where it overlaps nothing already there.
 *
 * Ties on `start` go longest first, so a brace that encloses another sits
 * nearer the axis and the one inside it steps out: read outward from the
 * bytes, the lanes then go from the whole to its parts, which is how a reader
 * follows a nest of brackets. `gap` is the room that must separate two items
 * in one lane, in the caller's unit.
 */
export function assignLanes<T>(items: readonly T[], span: (item: T) => Span, gap = 0): Lanes {
  const spans = items.map(span);
  const order = spans.map((_, i) => i);
  order.sort((a, b) => {
    const sa = spans[a] as Span;
    const sb = spans[b] as Span;
    if (sa.start !== sb.start) return sa.start - sb.start;
    const la = sa.end - sa.start;
    const lb = sb.end - sb.start;
    if (la !== lb) return lb - la;
    return a - b;
  });
  const lane = new Array<number>(items.length).fill(0);
  /** Where each lane's last item ends. */
  const ends: number[] = [];
  for (const i of order) {
    const s = spans[i] as Span;
    const end = Math.max(s.end, s.start);
    let at = ends.findIndex((e) => e + gap <= s.start);
    if (at < 0) {
      at = ends.length;
      ends.push(end);
    } else ends[at] = end;
    lane[i] = at;
  }
  return { lane, count: ends.length };
}

/** The most items that overlap at any one point: the number of lanes
 *  `assignLanes` will use with no gap, since for intervals on a line the
 *  greedy order is optimal. For a caller deciding whether a figure has room
 *  before it lays anything out. */
export function depthOf(spans: readonly Span[]): number {
  return assignLanes(spans, (s) => s).count;
}

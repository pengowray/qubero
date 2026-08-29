// The same small picture of the whole file, drawn again wherever a range needs
// placing: on a heading, in a byte strip's caption, on the rail.
//
// The geometry is fixed. Every strip has the same segments in the same places,
// whatever it is drawn beside, so the only thing that moves between one strip
// and the next is which part is lit. That is what makes it readable at a
// glance: the reader learns the shape of the file once, and every later strip
// is the same shape with a different piece bright. A bar showing each row's
// own percentage would be a different picture every time and would say only
// how big the row is, which the row already says.

/** One division of the file: a stretch of bytes and the colour it goes by. */
export type MapSegment = {
  readonly offsetBits: number;
  readonly sizeBits: number;
  readonly color: string;
};

/** How wide the strip is, and the least a segment may shrink to. A hundred
 *  bytes of header in a twelve-kilobyte database is under a pixel of ninety-six
 *  honestly drawn, and a part of the file that cannot be seen cannot be lit. */
const WIDTH_PX = 96;
const MIN_SEGMENT_PX = 2;
const GAP_PX = 1;
/** The least of a segment a lit range may occupy. A twenty-byte cell inside a
 *  four-kilobyte page is a thousandth of it; without a floor the strip would
 *  say the range is nowhere. */
const MIN_LIT_FRACTION = 0.06;

/**
 * Widths in pixels for each segment, proportional to its bytes except that
 * none falls below `MIN_SEGMENT_PX`.
 *
 * Raising the small ones has to come from somewhere, so the rest are scaled
 * down to pay for it. A segment already above the floor can be pushed below it
 * that way, so the floor is applied again until it holds everywhere or there
 * is nothing left to take.
 */
export function segmentWidths(segments: readonly MapSegment[], width = WIDTH_PX): number[] {
  const n = segments.length;
  if (n === 0) return [];
  const room = Math.max(0, width - GAP_PX * (n - 1));
  const floor = Math.min(MIN_SEGMENT_PX, room / n);
  const total = segments.reduce((sum, s) => sum + Math.max(0, s.sizeBits), 0);
  if (total <= 0) return segments.map(() => room / n);
  const out = segments.map((s) => (Math.max(0, s.sizeBits) / total) * room);
  for (let pass = 0; pass < n; pass++) {
    const short = out.map((w, i) => (w < floor ? i : -1)).filter((i) => i >= 0);
    if (short.length === 0) break;
    const owed = short.reduce((sum, i) => sum + (floor - (out[i] ?? 0)), 0);
    const spare = out.reduce((sum, w) => sum + Math.max(0, w - floor), 0);
    if (spare <= 0) return out.map(() => room / n);
    const take = Math.min(1, owed / spare);
    for (let i = 0; i < n; i++) {
      const w = out[i] ?? 0;
      out[i] = w < floor ? floor : w - (w - floor) * take;
    }
  }
  return out;
}

/** How much of `segment` the range `[from, to)` covers, as two fractions of
 *  its width. Null when they do not overlap. */
function litSpan(segment: MapSegment, from: number, to: number): readonly [number, number] | null {
  const start = segment.offsetBits;
  const end = start + segment.sizeBits;
  if (segment.sizeBits <= 0 || to <= start || from >= end) return null;
  const a = (Math.max(from, start) - start) / segment.sizeBits;
  const b = (Math.min(to, end) - start) / segment.sizeBits;
  if (b - a >= MIN_LIT_FRACTION) return [a, b];
  // Too thin to see. Widen it about its middle and keep it inside the segment.
  const mid = (a + b) / 2;
  const half = MIN_LIT_FRACTION / 2;
  const lo = Math.max(0, Math.min(1 - MIN_LIT_FRACTION, mid - half));
  return [lo, lo + MIN_LIT_FRACTION];
}

/** The unlit ground. Dark enough that a lit sliver reads as lit, in either
 *  theme, without carrying a meaning of its own. */
const GROUND = "color-mix(in srgb, var(--fg) 14%, var(--bg))";

function paint(segment: MapSegment, span: readonly [number, number] | null): string {
  if (span === null) return GROUND;
  const [a, b] = span;
  if (a <= 0 && b >= 1) return segment.color;
  const from = `${(a * 100).toFixed(2)}%`;
  const to = `${(b * 100).toFixed(2)}%`;
  return `linear-gradient(to right, ${GROUND} ${from}, ${segment.color} ${from}, ${segment.color} ${to}, ${GROUND} ${to})`;
}

/**
 * One strip, with the bytes from `offsetBits` for `sizeBits` lit.
 *
 * `title` is what the strip says when pointed at, which the caller already has
 * in the words the rest of its row uses.
 */
export function fileMap(segments: readonly MapSegment[], offsetBits: number, sizeBits: number, title: string): HTMLElement {
  const strip = document.createElement("span");
  strip.className = "fmap";
  strip.title = title;
  strip.setAttribute("aria-hidden", "true");
  const widths = segmentWidths(segments);
  const to = offsetBits + Math.max(0, sizeBits);
  segments.forEach((segment, i) => {
    const cell = document.createElement("i");
    cell.style.flex = `0 0 ${(widths[i] ?? 0).toFixed(2)}px`;
    cell.style.background = paint(segment, litSpan(segment, offsetBits, to));
    strip.append(cell);
  });
  return strip;
}

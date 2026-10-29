// Whether a name fits where it is wanted, and what to draw when it does not.
//
// The rule every piece follows, first written for the treemap and taken out
// of it so that the rest can share it: measure the text, draw it when it fits
// its box with padding, cut it with an ellipsis when enough of it is left to
// be worth reading, and otherwise leave it off and keep the name in the hover
// card. Text is never shrunk below a readable size to make it fit, and two
// names never share a place: a name that would land on one already placed is
// moved down inside its own box, or dropped.
//
// Measuring is a parameter. The treemap draws in the monospace stack and
// places names by a fixed character width (`monoMeasure`), which is exact for
// that face and needs no canvas; a piece drawing in the sans face measures
// with a canvas (`canvasMeasure`), once per text, and the tests pass a fake.
// Everything else here is arithmetic on rectangles, with no page behind it.

/** How wide `text` is drawn at `size` pixels, in pixels. */
export type Measure = (text: string, size: number) => number;

/** A face where every character is the same width: the monospace stack, at
 *  0.6 of the font size. */
export function monoMeasure(ratio = 0.6): Measure {
  return (text, size) => text.length * size * ratio;
}

/**
 * A canvas measurer for one font family. Widths scale with the size, so each
 * text is measured once at a reference size and scaled after, which keeps a
 * redraw of a thousand names at every zoom step to a thousand map lookups.
 */
export function canvasMeasure(family: string): Measure {
  const REF = 100;
  const cache = new Map<string, number>();
  let ctx: CanvasRenderingContext2D | null = null;
  return (text, size) => {
    let w = cache.get(text);
    if (w === undefined) {
      ctx ??= document.createElement("canvas").getContext("2d");
      if (ctx === null) return text.length * size * 0.6;
      ctx.font = `${REF}px ${family}`;
      w = ctx.measureText(text).width;
      // A bound on memory, not a policy: a piece that names fifty thousand
      // records measures each once per redraw rather than keeping them all.
      if (cache.size > 20_000) cache.clear();
      cache.set(text, w);
    }
    return (w * size) / REF;
  };
}

export type Rect = { x0: number; y0: number; x1: number; y1: number };

export function overlaps(a: Rect, b: Rect): boolean {
  return a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;
}

export type FitOptions = {
  /** The sizes to try, largest first. */
  readonly sizes: readonly number[];
  /** Which of `sizes` to start from: the largest one this label is allowed. */
  readonly start?: number;
  /** A long name is drawn once this many of its characters fit, and cut off
   *  with an ellipsis. A shorter one has to fit whole or not be drawn at all:
   *  `program_header_off…` is still worth reading and `0x8…` is not, because
   *  a cut-off byte value is a different byte value. */
  readonly minChars: number;
  /** Room around the text inside its plate. */
  readonly padX: number;
  readonly padY: number;
  /** Pixels of box needed beyond the plate, so a name two pixels short of
   *  fitting is not drawn and then clipped. */
  readonly slack?: number;
  /** Whether a tall narrow box may have its name run downwards. */
  readonly sideways?: boolean;
  readonly measure: Measure;
};

/** A name that fits: how big, which way up, how wide it wants to be, and how
 *  tall one line of it is. */
export type LabelFit = { size: number; sideways: boolean; wanted: number; lineH: number };

/**
 * The biggest size a name fits a `w` by `h` box at, upright if it can be and
 * turned on its side if it cannot (and `sideways` allows it). Null when it
 * does not fit at any of the sizes.
 */
export function fitLabel(text: string, w: number, h: number, opts: FitOptions): LabelFit | null {
  const slack = opts.slack ?? 0;
  const first = Math.max(0, Math.min(opts.start ?? 0, opts.sizes.length - 1));
  const head = text.slice(0, opts.minChars);
  for (let i = first; i < opts.sizes.length; i++) {
    const size = opts.sizes[i] as number;
    const lineH = size + 2 * opts.padY + 2;
    const need = opts.measure(head, size) + 2 * opts.padX + slack;
    const wanted = opts.measure(text, size) + 2 * opts.padX;
    if (h >= lineH && w >= need) return { size, sideways: false, wanted, lineH };
    if (opts.sideways === true && w >= lineH && h >= need) return { size, sideways: true, wanted, lineH };
  }
  return null;
}

const ELLIPSIS = "…";

/**
 * `text` as it can be drawn in `room` pixels at `size`: whole if it fits, cut
 * with an ellipsis if at least `minChars` characters survive, otherwise null,
 * which means leave it off. A text of `minChars` characters or fewer is never
 * cut, by the rule in `FitOptions.minChars`.
 */
export function ellipsise(text: string, room: number, size: number, measure: Measure, minChars: number): string | null {
  if (measure(text, size) <= room) return text;
  if (text.length <= minChars) return null;
  // The longest prefix that fits with its ellipsis, by halving. Widths grow
  // with the prefix in every face, so the search is sound.
  let lo = minChars;
  let hi = text.length - 1;
  if (measure(text.slice(0, lo) + ELLIPSIS, size) > room) return null;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(text.slice(0, mid) + ELLIPSIS, size) <= room) lo = mid;
    else hi = mid - 1;
  }
  return text.slice(0, lo) + ELLIPSIS;
}

/**
 * Somewhere in its box for a name, given the names already placed.
 *
 * A name wants the top left of its box, and so does the name of every frame
 * around it, so a name that lands on one already down steps a line down (or,
 * running sideways, a column across) and tries again, up to `tries` times. It
 * stays inside its own box: `far` is the box's far edge in the direction it
 * steps, and a name that has walked past it is dropped, since by then it
 * would be pointing at something else.
 */
export function freeRect(want: Rect, placed: readonly Rect[], far: number, sideways = false, tries = 4): Rect | null {
  const height = want.y1 - want.y0;
  const width = want.x1 - want.x0;
  let rect = want;
  for (let n = 0; n < tries; n++) {
    if ((sideways ? rect.x1 : rect.y1) > far) return null;
    const hit = placed.find((p) => overlaps(p, rect));
    if (hit === undefined) return rect;
    rect = sideways ? { ...rect, x0: hit.x1, x1: hit.x1 + width } : { ...rect, y0: hit.y1, y1: hit.y1 + height };
  }
  return null;
}

/** One label on a line: where it would be drawn, and how much it matters. */
export type LineLabel = { readonly start: number; readonly end: number; readonly priority: number };

/**
 * Which labels on one line to draw so that none overlaps another: the most
 * important first, each kept only when it clears every one kept already by
 * `gap`. For names along a strip or ticks along an axis that d3-axis does not
 * draw. Ties in priority go to the earlier label. Returns one flag per label,
 * in the order given.
 */
export function thinLabels(labels: readonly LineLabel[], gap = 0): boolean[] {
  const keep = new Array<boolean>(labels.length).fill(false);
  const order = labels.map((_, i) => i).sort((a, b) => (labels[b] as LineLabel).priority - (labels[a] as LineLabel).priority || a - b);
  /** The kept labels as non-overlapping intervals, sorted by start. */
  const kept: LineLabel[] = [];
  for (const i of order) {
    const l = labels[i] as LineLabel;
    // The first kept label that starts after this one does; the one before it
    // is the only other one that can overlap from the left.
    let lo = 0;
    let hi = kept.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if ((kept[mid] as LineLabel).start < l.start) lo = mid + 1;
      else hi = mid;
    }
    const before = kept[lo - 1];
    const after = kept[lo];
    if (before !== undefined && before.end + gap > l.start) continue;
    if (after !== undefined && l.end + gap > after.start) continue;
    kept.splice(lo, 0, l);
    keep[i] = true;
  }
  return keep;
}

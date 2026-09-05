// Which shape a run's values are drawn in, and how wide the pieces of that
// shape are.
//
// `valuetable.ts` places the cells of one row; this file answers the two
// questions it has to be told the answer to first: which of the three layouts
// a run gets, and what a cell of that layout may be. Both are arithmetic over
// measured text, so both are here rather than in the view, and both are
// settled per screenful rather than per row: a table that changed shape from
// one row to the next would be unreadable, and a row whose height depended on
// what happened to be beside it would break the scroll.
//
// A fourth layout is one more case in `chooseLayout` and one more branch in
// `valuetable.ts`'s placement, and nothing else.
//
// `.ts` on the imports rather than the `.js` the rest of `src` writes: the
// tests run this file under `node --test`, which strips the types but does not
// rewrite a `.js` specifier back to the file it came from.
import type { ChipMeasure } from "./chipfit.ts";
import type { Cell } from "./doc.ts";

// `Cell` is the core's own answer, from `doc.runCells`; nothing here reshapes
// it. Re-exported so the view and the tests have one name for it.
export type { Cell };

/** A run and its elements on screen. The name and type are what a cell's
 *  tooltip says it is; `symbol` marks the steps of a traced block, which are
 *  named rather than numbered. */
export type RunCells = {
  /** The run itself, so a cell picks the element at `[...path, index]`. */
  readonly path: readonly number[];
  readonly name: string;
  readonly type: string;
  readonly symbol: boolean;
  /** The widest text this run has ever shown, whether or not it is on screen
   *  now. Which layout a run gets has to hold still while the reader scrolls:
   *  a window whose samples happen to be four digits would otherwise be
   *  aligned and the next window, holding a five-digit one, uniform, and every
   *  row would change height between them. The floor only ever rises, so a run
   *  gives the aligned layout up at most once. */
  readonly widest: string;
  readonly cells: readonly Cell[];
};

/** Padding either side of a cell's text (`padding: 0 2px` in `.hv-val`), plus
 *  the hairline between two of them. Mirrors style.css, and is kept narrow on
 *  purpose: two bytes of a 16-byte row are 45 pixels, and `-32768` and its
 *  padding have to fit inside them for a run of 16-bit samples to be drawn
 *  over the bytes it is stored in. */
export const VALUE_PAD = 5;
/** The gap between two uniform cells (`gap: 2px` on `.hv-vals-uniform`). */
export const VALUE_GAP = 2;
/** Room kept on the last line for the `+3` that counts what did not fit. */
export const VALUE_REST = 34;
/** The three shapes a row's table takes. Aligned draws every value over the
 *  bits it is stored in; uniform is equal cells wrapped over as many lines as
 *  the row needs; flow is cells of their own natural width, for a run whose
 *  elements are the symbols of a decoder and read as the text they decode
 *  to. */
export type Layout = "aligned" | "uniform" | "flow";
/** Width to assume before the column has been measured once. */
export const COLUMN_GUESS = 320;
/** Pitch of a hex cell to assume before one has been measured. */
const PITCH_GUESS = 22;

/**
 * How wide a cell of this kind and size has to be, whatever value it happens
 * to hold: the longest text the type can produce, as a string of digits to
 * measure.
 *
 * This is what keeps the layout still while the reader scrolls. Measuring the
 * values that happen to be on screen means a window of four-digit samples is
 * laid out one way and the next window, holding a five-digit one, another —
 * and every row between them changes height. A `u16` is five digits wide
 * whether it holds 7 or 65,535.
 *
 * Empty for the kinds whose text has no width the type can promise: a float, a
 * decoder's symbol, a record's one-line reading. Those fall back to the widest
 * text the run has shown.
 */
export function typeDigits(kind: string, sizeBits: number): string {
  if (sizeBits <= 0 || sizeBits > 64) return "";
  if (kind === "uint" || kind === "unset") return "8".repeat(Math.ceil(Math.log10(2 ** Math.min(sizeBits, 53))));
  if (kind === "int") return `-${"8".repeat(Math.ceil(Math.log10(2 ** Math.min(sizeBits - 1, 53))))}`;
  return "";
}

/** How wide one cell's text may be drawn: what its type can produce where that
 *  is known, and what it says where it is not. */
export function textWidth(c: Cell, measure: ChipMeasure): number {
  const digits = typeDigits(c.kind, c.size_bits);
  return measure.value(digits === "" ? c.label : digits);
}

export type FitOpts = {
  readonly bpr: number;
  /** Width of the annotation column, measured from the last frame. */
  readonly noteWidth: number;
  /** Width of one byte of the hex column, so a byte of the table can have the
   *  pitch of a byte of the bytes. */
  readonly hexPitch: number;
  readonly measure: ChipMeasure;
};

/**
 * The bits of the widest piece an element is left in once the row edges have
 * cut it: the whole element when no edge does, else the larger of its pieces.
 * That piece is the cell its text is drawn in (see `valuetable.ts`), so it is
 * the width the text has to fit. Rows begin at multiples of their own width.
 */
export function widestPieceBits(c: Cell, rowBits: number): number {
  const start = c.offset_bits;
  const end = start + c.size_bits;
  let best = 0;
  for (let at = Math.floor(start / rowBits) * rowBits; at < end; at += rowBits) {
    best = Math.max(best, Math.min(end, at + rowBits) - Math.max(start, at));
  }
  return best;
}

/**
 * How many pixels a bit of the aligned grid has to be for every value in
 * these runs to fit the piece it is drawn in: a bit of a hex cell, so that a
 * byte of the table sits under a byte of the bytes, or wider where a value
 * needs more than that. A run of 32-bit samples that starts two bytes into a
 * row is cut in half by every row edge, and nine digits do not fit two bytes
 * at the hex pitch; they do fit two bytes at half again as much, and a table
 * a little wider than the bytes still reads as the bytes' own table. Infinity
 * for a run that cannot be aligned at all: one with no cells, or one the core
 * says is not stored in one contiguous stretch of bits.
 */
export function alignedBit(runs: readonly RunCells[], o: FitOpts): number {
  const columns = o.bpr * 8;
  let need = (o.hexPitch > 0 ? o.hexPitch : PITCH_GUESS) / 8;
  let any = false;
  for (const run of runs) {
    const floor = o.measure.value(run.widest);
    for (const c of run.cells) {
      if (!c.contiguous) return Infinity;
      any = true;
      const text = Math.max(floor, textWidth(c, o.measure)) + VALUE_PAD;
      need = Math.max(need, text / widestPieceBits(c, columns));
    }
  }
  return any ? need : Infinity;
}

/** How wide the aligned table is drawn on this screenful: a byte of it has the
 *  pitch of a hex cell, or as much more as the widest value of the runs that
 *  are aligned needs, and never more than the column. One width for the
 *  screenful, since every row's table shares the column's byte positions. */
export function alignedWidth(runs: readonly RunCells[], o: FitOpts): number {
  const columns = o.bpr * 8;
  const column = o.noteWidth || COLUMN_GUESS;
  const aligned = runs.filter((r) => alignedFits([r], o));
  const bit = aligned.length > 0 ? alignedBit(aligned, o) : (o.hexPitch > 0 ? o.hexPitch : PITCH_GUESS) / 8;
  return Math.min(column, columns * bit);
}

/**
 * Whether a run's cells can be drawn over the bits they are stored in.
 *
 * Aligned puts every value over the bits it is stored in, which is the whole
 * point of the table, but only while the values fit the width their bits are
 * worth. A six-bit code holding `-13` has eight pixels to say it in, and a
 * cell that cannot say what it holds says nothing at all, so those runs go to
 * the uniform layout instead. So does any run the core marks as not stored in
 * one contiguous stretch of bits: there is no one place to draw those over.
 *
 * The element is measured whole, not as the piece of it left on a row it
 * straddles: the element at the end of a row is cut by the row edge, not by
 * the layout. `alignedWidth` widens the grid for those pieces where the
 * column has the room; where it has not, the piece lets its text out over the
 * end of the table (`.hv-val-cut`), which is still the value beside its bytes.
 */
export function alignedFits(runs: readonly RunCells[], o: FitOpts): boolean {
  const columns = o.bpr * 8;
  const bit = Math.min(o.noteWidth || COLUMN_GUESS, columns * ((o.hexPitch > 0 ? o.hexPitch : PITCH_GUESS) / 8)) / columns;
  let narrowest = Infinity;
  let widest = 0;
  let any = false;
  for (const run of runs) {
    widest = Math.max(widest, o.measure.value(run.widest));
    for (const c of run.cells) {
      if (!c.contiguous) return false;
      any = true;
      narrowest = Math.min(narrowest, Math.min(c.size_bits, columns) * bit);
      widest = Math.max(widest, textWidth(c, o.measure));
    }
  }
  return any && narrowest >= widest + VALUE_PAD;
}

/** How wide a uniform cell is: the widest text on screen and its padding. */
export function uniformWidth(runs: readonly RunCells[], measure: ChipMeasure): number {
  let widest = 0;
  for (const run of runs) {
    widest = Math.max(widest, measure.value(run.widest));
    for (const c of run.cells) widest = Math.max(widest, textWidth(c, measure));
  }
  return Math.ceil(widest + VALUE_PAD);
}

/**
 * The layout one run gets.
 *
 * A run of a decoder's symbols flows whatever it would otherwise do: its cells
 * read as the bytes the block produces, so they take the width of what they
 * say and a row of them reads as a line of the file being unpacked. Read off
 * the cells' own `kind` rather than the run's `symbol`, which comes from the
 * span's unit and is a different answer to a different question.
 */
export function chooseLayout(run: RunCells, o: FitOpts): Layout {
  if (run.cells.some((c) => c.kind === "symbol")) return "flow";
  return alignedFits([run], o) ? "aligned" : "uniform";
}

/**
 * The layout a row takes when more than one run reaches it.
 *
 * Aligned and flow both draw a cell somewhere only that run can be drawn, so
 * two runs that disagree have nothing in common but the uniform layout, which
 * is the one every run can be drawn in. A row no run reaches is uniform and
 * empty.
 */
export function rowLayout(layouts: readonly Layout[]): Layout {
  const first = layouts[0];
  if (first === undefined) return "uniform";
  return layouts.every((l) => l === first) ? first : "uniform";
}

/**
 * Pack flow cells into lines, each cell as wide as its own text.
 *
 * The same greedy wrap the browser will do, worked out here because the row's
 * height has to be known before the row is laid out. Every line takes at least
 * one cell, so a label wider than the whole column takes a line of its own
 * rather than never fitting.
 */
export function wrapFlow<T extends { readonly width: number }>(
  cells: readonly T[],
  column: number,
  maxLines: number,
): { kept: T[]; lines: number; rest: number } {
  let lines = 1;
  let used = 0;
  for (const [i, c] of cells.entries()) {
    const room = lines === maxLines ? column - VALUE_REST : column;
    const next = used === 0 ? c.width : used + VALUE_GAP + c.width;
    if (used > 0 && next > room) {
      if (lines === maxLines) return { kept: cells.slice(0, i), lines, rest: cells.length - i };
      lines++;
      used = c.width;
    } else {
      used = next;
    }
  }
  return { kept: [...cells], lines, rest: 0 };
}

/**
 * How many uniform cells fit on a line, and how many the last line gives up to
 * the `+N` that counts what is left.
 */
export function uniformFit(column: number, width: number): { perLine: number; lastLine: number } {
  const w = Math.max(1, width);
  return {
    perLine: Math.max(1, Math.floor((column + VALUE_GAP) / (w + VALUE_GAP))),
    lastLine: Math.max(1, Math.floor((column + VALUE_GAP - VALUE_REST) / (w + VALUE_GAP))),
  };
}

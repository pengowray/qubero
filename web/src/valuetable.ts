// The table of values the field column draws beside every row a folded run
// covers: one cell per element whose bits fall on the row, in the order of the
// bytes, so the reader can go from a byte to its value and back.
//
// A folded run is one chip — `body 72,000 values` — and the rows under that
// chip used to say nothing at all. This is what they say instead. Nothing here
// touches the document: the layout is settled as values, before a cell is
// drawn, so a row's height is known before the browser lays it out and so this
// file can be tested without one.
//
// `.ts` on the imports rather than the `.js` the rest of `src` writes: the
// tests run this file under `node --test`, which strips the types but does not
// rewrite a `.js` specifier back to the file it came from.
import type { ChipMeasure } from "./chipfit.ts";

/** One element of a run, as the core reads it. The shape `doc.runCells`
 *  answers in; `runcellsshim.ts` builds the same thing out of
 *  `templateChildren` until that lands. */
export type Cell = {
  /** The element's number in its run: the last step of its path. */
  readonly index: number;
  readonly offset_bits: number;
  readonly size_bits: number;
  /** What the listing would say on a shared row, or the symbol's name for a
   *  traced block. Never reformatted here. */
  readonly text: string;
  readonly kind: string;
  /** False when the element's bits are not one contiguous run, which is what
   *  sends the whole run to the uniform layout. */
  readonly contiguous: boolean;
};

/** A run and its elements on screen. The name and type are what a cell's
 *  tooltip says it is; `symbol` marks the steps of a traced block, which are
 *  named rather than numbered. */
export type RunCells = {
  readonly name: string;
  readonly type: string;
  readonly symbol: boolean;
  readonly cells: readonly Cell[];
};

/** Padding either side of a cell's text (`padding: 0 4px` in `.hv-val`), plus
 *  the hairline between two of them. Mirrors style.css. */
export const VALUE_PAD = 9;
/** The gap between two uniform cells (`gap: 2px` on `.hv-vals-uniform`). */
export const VALUE_GAP = 2;
/** Room kept on the last line for the `+3` that counts what did not fit. */
export const VALUE_REST = 34;
/** Width to assume before the column has been measured once. */
const COLUMN_GUESS = 320;
/** Pitch of a hex cell to assume before one has been measured. */
const PITCH_GUESS = 22;

/** Whether a value is a number, and so is read from its right-hand end. */
export function numeric(kind: string): boolean {
  return kind === "uint" || kind === "int" || kind === "float" || kind === "unset";
}

/** One cell as it is drawn: where it sits, what it says, and whether it is the
 *  tail of an element that began on the row above. */
export type PlacedCell = {
  readonly index: number;
  /** Empty on a continuation cell: the element's text is on the row it
   *  started on. */
  readonly text: string;
  readonly kind: string;
  readonly numeric: boolean;
  readonly continued: boolean;
  /** The run the element belongs to, for the tooltip. */
  readonly run: string;
  readonly type: string;
  readonly symbol: boolean;
  readonly sizeBits: number;
  /** Grid columns in the aligned layout: bit columns of the row, 1-based, so
   *  a row of `bpr` bytes runs from 1 to `bpr * 8 + 1`. Both are 0 in the
   *  uniform layout, which has no grid. */
  readonly from: number;
  readonly to: number;
};

/** What one row's table comes to. */
export type RowValues = {
  readonly layout: "aligned" | "uniform";
  readonly cells: PlacedCell[];
  /** Uniform only: how wide every cell is drawn, in pixels. */
  readonly cellWidth: number;
  /** How many lines the table takes. Zero when the row has no values. */
  readonly lines: number;
  /** How many elements the lines could not hold, counted on a `+N` cell. */
  readonly rest: number;
  /** What the table adds to the row's height. */
  readonly height: number;
  /** What the table says, as one string, so the cells are written again only
   *  when they would say something else. */
  readonly key: string;
};

/** A row with no values at all. */
export const NO_VALUES: RowValues = {
  layout: "uniform",
  cells: [],
  cellWidth: 0,
  lines: 0,
  rest: 0,
  height: 0,
  key: "",
};

export type FitOpts = {
  readonly bpr: number;
  /** Width of the annotation column, measured from the last frame. */
  readonly noteWidth: number;
  /** Width of one byte of the hex column, so a byte of the table can have the
   *  pitch of a byte of the bytes. */
  readonly hexPitch: number;
  readonly measure: ChipMeasure;
};

/** How wide the aligned table is drawn: a byte of it has the pitch of a hex
 *  cell where the column is wide enough for that, and the whole table shrinks
 *  proportionally where it is not. */
export function alignedWidth(o: FitOpts): number {
  const pitch = o.hexPitch > 0 ? o.hexPitch : PITCH_GUESS;
  return Math.min(o.noteWidth || COLUMN_GUESS, o.bpr * pitch);
}

/**
 * Which layout a run's cells get, decided once per run per screenful.
 *
 * Aligned puts every value over the bits it is stored in, which is the whole
 * point of the table — but only while the values fit the width their bits are
 * worth. A six-bit code holding `-13` has eight pixels to say it in, and a
 * cell that cannot say what it holds says nothing at all, so those runs go to
 * the uniform layout instead. So does any run the core marks as not stored in
 * one contiguous stretch of bits: there is no one place to draw those over.
 *
 * The narrowest cell is measured from the element's own size, not from the
 * piece of it left on a row it straddles: the element at the end of a row is
 * cut by the row edge, not by the layout, and letting that decide would send
 * every run with a straddling element to the uniform layout.
 */
export function alignedFits(runs: readonly RunCells[], o: FitOpts): boolean {
  const columns = o.bpr * 8;
  const bit = alignedWidth(o) / columns;
  let narrowest = Infinity;
  let widest = 0;
  let any = false;
  for (const run of runs) {
    for (const c of run.cells) {
      if (!c.contiguous) return false;
      any = true;
      narrowest = Math.min(narrowest, Math.min(c.size_bits, columns) * bit);
      widest = Math.max(widest, o.measure.value(c.text));
    }
  }
  return any && narrowest >= widest + VALUE_PAD;
}

/** How wide a uniform cell is: the widest text on screen and its padding. */
export function uniformWidth(runs: readonly RunCells[], measure: ChipMeasure): number {
  let widest = 0;
  for (const run of runs) {
    for (const c of run.cells) widest = Math.max(widest, measure.value(c.text));
  }
  return Math.ceil(widest + VALUE_PAD);
}

export type RowValueOpts = {
  /** The runs on screen, with the cells that overlap this row among them. */
  readonly runs: readonly RunCells[];
  readonly rowStart: number;
  readonly bpr: number;
  readonly layout: "aligned" | "uniform";
  /** From `uniformWidth`, so every row of a screenful draws the same width. */
  readonly cellWidth: number;
  readonly noteWidth: number;
  /** How many lines the table may take before the rest is counted. */
  readonly maxLines: number;
  /** The pitch of one line of values, gap included, so `lines * valLine` is
   *  what the table adds to the row. */
  readonly valLine: number;
};

/**
 * Lay one row's values out.
 *
 * Aligned is one line whatever it holds: every cell is over its own bits, and
 * an element that straddles the row edge has its text on the row it starts on
 * and a continuation cell — same tint, no text — on the next.
 *
 * Uniform wraps into as many lines as the row needs, in the order of the
 * bytes, and an element belongs to the row its first bit falls on so that no
 * value is drawn twice.
 */
export function planRowValues(o: RowValueOpts): RowValues {
  const rowFrom = o.rowStart * 8;
  const rowTo = rowFrom + o.bpr * 8;
  const cells: PlacedCell[] = [];
  for (const run of o.runs) {
    for (const c of run.cells) {
      const end = c.offset_bits + c.size_bits;
      const started = c.offset_bits >= rowFrom && c.offset_bits < rowTo;
      // Aligned draws a cell wherever the element's bits reach; uniform draws
      // it once, on the row it starts on.
      if (o.layout === "uniform" ? !started : end <= rowFrom || c.offset_bits >= rowTo) continue;
      const continued = c.offset_bits < rowFrom;
      cells.push({
        index: c.index,
        text: continued ? "" : c.text,
        kind: c.kind,
        numeric: numeric(c.kind),
        continued,
        run: run.name,
        type: run.type,
        symbol: run.symbol,
        sizeBits: c.size_bits,
        from: o.layout === "aligned" ? Math.max(rowFrom, c.offset_bits) - rowFrom + 1 : 0,
        to: o.layout === "aligned" ? Math.min(rowTo, end) - rowFrom + 1 : 0,
      });
    }
  }
  cells.sort((a, b) => a.from - b.from || a.index - b.index);
  if (cells.length === 0) return NO_VALUES;
  if (o.layout === "aligned") {
    return finish("aligned", cells, 0, 1, 0, o.valLine);
  }
  const width = Math.max(1, o.cellWidth);
  const column = o.noteWidth || COLUMN_GUESS;
  const perLine = Math.max(1, Math.floor((column + VALUE_GAP) / (width + VALUE_GAP)));
  const want = Math.ceil(cells.length / perLine);
  if (want <= o.maxLines) return finish("uniform", cells, width, want, 0, o.valLine);
  // Past the cap the last line keeps room for the count of what is left.
  const room = Math.max(1, Math.floor((column + VALUE_GAP - VALUE_REST) / (width + VALUE_GAP)));
  const shown = perLine * (o.maxLines - 1) + room;
  const kept = cells.slice(0, Math.min(cells.length - 1, shown));
  return finish("uniform", kept, width, o.maxLines, cells.length - kept.length, o.valLine);
}

function finish(
  layout: "aligned" | "uniform",
  cells: PlacedCell[],
  cellWidth: number,
  lines: number,
  rest: number,
  valLine: number,
): RowValues {
  const key =
    `${layout[0] ?? ""}${lines}/${rest}/${Math.round(cellWidth)}~` +
    cells.map((c) => (c.continued ? `^${c.index}` : `${c.index}=${c.text}`)).join(",");
  return { layout, cells, cellWidth, lines, rest, height: lines * valLine, key };
}

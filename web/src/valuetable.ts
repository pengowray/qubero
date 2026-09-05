// Where each value of a folded run sits on the row it falls on.
//
// A folded run is one chip — `body 72,000 values` — and the rows under that
// chip used to say nothing at all. This is what they say instead: one cell per
// element whose bits fall on the row, in the order of the bytes, so the reader
// can go from a byte to its value and back.
//
// This file places the cells. `valuelayout.ts` decides which of the three
// shapes they are placed in and how wide the pieces of that shape are; the
// view hands the answer in as `layout`. Nothing here touches the document: the
// layout is settled as values, before a cell is drawn, so a row's height is
// known before the browser lays it out and so this file can be tested without
// one.
//
// `.ts` on the imports rather than the `.js` the rest of `src` writes: the
// tests run this file under `node --test`, which strips the types but does not
// rewrite a `.js` specifier back to the file it came from.
import type { ChipMeasure } from "./chipfit.ts";
import {
  COLUMN_GUESS,
  uniformFit,
  VALUE_PAD,
  wrapFlow,
  type Cell,
  type Layout,
  type RunCells,
} from "./valuelayout.ts";

// One import site for the view and the tests: what a run's values come to is
// this file's subject, and which shape they take is next door.
export {
  alignedFits,
  alignedWidth,
  chooseLayout,
  rowLayout,
  typeDigits,
  uniformWidth,
  VALUE_GAP,
  VALUE_PAD,
  VALUE_REST,
  type Cell,
  type FitOpts,
  type Layout,
  type RunCells,
} from "./valuelayout.ts";

/** Whether a value is a number, and so is read from its right-hand end. */
export function numeric(kind: string): boolean {
  return kind === "uint" || kind === "int" || kind === "float" || kind === "unset";
}

/** One cell as it is drawn: where it sits, what it says, and whether it is a
 *  piece of an element whose text is on another row. */
export type PlacedCell = {
  readonly index: number;
  /** Empty on the piece of a straddling element that does not carry the text.
   *  What is drawn, which for a symbol is shorter than what the tooltip
   *  says. */
  readonly text: string;
  /** What the element reads as in full, for the tooltip. */
  readonly tip: string;
  readonly kind: string;
  readonly numeric: boolean;
  /** Set on the empty piece of a straddling element: which way the piece
   *  carrying the text lies. Null on a cell that says what it holds. */
  readonly carried: "above" | "below" | null;
  /** True when the piece carrying the text is narrower than the whole element,
   *  so the cell is narrower than the value needs. Its text is let out of the
   *  cell rather than cut off. */
  readonly cut: boolean;
  /** Flow only: how wide the cell is drawn, in pixels. Zero in the other two
   *  layouts, which have a width of their own. */
  readonly width: number;
  /** The run the element belongs to: its name for the tooltip, its path for
   *  the pick. */
  readonly path: readonly number[];
  readonly run: string;
  readonly type: string;
  readonly symbol: boolean;
  /** A symbol that is not one byte of the stream's output: a match, the end of
   *  a block. Those are where the copying happens, and are tinted apart from
   *  the literals around them. */
  readonly copy: boolean;
  readonly sizeBits: number;
  /** The element's own bits, absolute, so the cursor can be found in them
   *  without reading anything back off the document. */
  readonly startBit: number;
  readonly endBit: number;
  /** Grid columns in the aligned layout: bit columns of the row, 1-based, so
   *  a row of `bpr` bytes runs from 1 to `bpr * 8 + 1`. Both are 0 in the
   *  uniform layout, which has no grid. */
  readonly from: number;
  readonly to: number;
};

/**
 * Whether a symbol is a copy rather than a byte the block spelled out.
 *
 * Read off the text, which is the core's and is one of a fixed handful of
 * English phrases; `kind` is `symbol` for every step of a trace and says
 * nothing about which. A literal is the only step that is one byte of output,
 * so everything else is where the copying happens.
 */
export function symbolCopies(kind: string, text: string): boolean {
  return kind === "symbol" && !text.startsWith("literal ");
}

/** What one row's table comes to. */
export type RowValues = {
  readonly layout: Layout;
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

export type RowValueOpts = {
  /** The runs on screen, with the cells that overlap this row among them. */
  readonly runs: readonly RunCells[];
  readonly rowStart: number;
  readonly bpr: number;
  readonly layout: Layout;
  /** Flow only: how wide a cell's own text is. Aligned reads it too, to know
   *  whether a cut value fits the piece it is drawn on. */
  readonly measure?: ChipMeasure;
  /** Aligned only: how many pixels one bit of the grid is drawn as, from
   *  `alignedWidth`. Zero when unknown, which takes every cut value to be too
   *  wide for its piece. */
  readonly bitWidth?: number;
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
 * Which row of the ones an element straddles carries its text, as an offset in
 * rows from the row the element starts on.
 *
 * The wider piece: a 24-bit sample whose first byte is the last byte of a row
 * has seven eighths of itself on the row below, and putting the number on the
 * one-byte sliver leaves it hanging off the end of the table over nothing. The
 * piece that is most of the value is the piece a reader looks at for it. Ties
 * go to the piece the element starts on, so a value split evenly reads where
 * it begins.
 */
function widestPiece(startBit: number, endBit: number, rowFrom: number, rowBits: number): number {
  // Where the row containing the element's first bit begins. Rows are a fixed
  // pitch apart, so this is worked out from the row in hand without knowing
  // which row that is.
  const firstRow = rowFrom + Math.floor((startBit - rowFrom) / rowBits) * rowBits;
  let best = 0;
  let bestWidth = 0;
  for (let r = 0, at = firstRow; at < endBit; r++, at += rowBits) {
    const width = Math.min(endBit, at + rowBits) - Math.max(startBit, at);
    if (width > bestWidth) {
      best = r;
      bestWidth = width;
    }
  }
  return best;
}

/**
 * Lay one row's values out.
 *
 * Aligned is one line whatever it holds: every cell is over its own bits, and
 * an element the row edge cuts has its text on whichever of its two pieces is
 * wider, with the other drawn as an empty cell in the same tint.
 *
 * Uniform wraps equal cells into as many lines as the row needs, and flow
 * wraps cells of their own width. Both are in the order of the bytes, and in
 * both an element belongs to the row its first bit falls on so that no value
 * is drawn twice.
 */
export function planRowValues(o: RowValueOpts): RowValues {
  const rowBits = o.bpr * 8;
  const rowFrom = o.rowStart * 8;
  const rowTo = rowFrom + rowBits;
  const aligned = o.layout === "aligned";
  const cells: PlacedCell[] = [];
  for (const run of o.runs) {
    for (const c of run.cells) {
      const end = c.offset_bits + c.size_bits;
      const started = c.offset_bits >= rowFrom && c.offset_bits < rowTo;
      // Aligned draws a cell wherever the element's bits reach; the other two
      // draw it once, on the row it starts on.
      if (!aligned ? !started : end <= rowFrom || c.offset_bits >= rowTo) continue;
      // Which piece says what the element is, and so which of the others are
      // empty and which way they look for it.
      const straddles = aligned && (c.offset_bits < rowFrom || end > rowTo);
      const carrier = straddles ? widestPiece(c.offset_bits, end, rowFrom, rowBits) : 0;
      const mine = straddles ? pieceIndex(c.offset_bits, rowFrom, rowBits) : 0;
      const carried = !straddles || mine === carrier ? null : mine > carrier ? "above" : "below";
      cells.push({
        index: c.index,
        text: carried === null ? c.label : "",
        tip: c.text,
        kind: c.kind,
        numeric: numeric(c.kind),
        carried,
        // The piece the text is on is narrower than the element, so the value
        // may not fit the cell it is in; it is let out only when it does not,
        // since a value that fits its piece is read like any other cell.
        cut:
          straddles &&
          carried === null &&
          (o.measure === undefined ||
            (o.bitWidth ?? 0) <= 0 ||
            o.measure.value(c.label) + VALUE_PAD > (Math.min(rowTo, end) - Math.max(rowFrom, c.offset_bits)) * (o.bitWidth ?? 0)),
        width: o.layout === "flow" ? Math.ceil((o.measure?.value(c.label) ?? 0) + VALUE_PAD) : 0,
        path: run.path,
        run: run.name,
        type: run.type,
        symbol: run.symbol,
        copy: symbolCopies(c.kind, c.text),
        sizeBits: c.size_bits,
        startBit: c.offset_bits,
        endBit: end,
        from: aligned ? Math.max(rowFrom, c.offset_bits) - rowFrom + 1 : 0,
        to: aligned ? Math.min(rowTo, end) - rowFrom + 1 : 0,
      });
    }
  }
  // In the order of the bytes, which is the order the table is read in, and
  // which is the only order two runs sharing a row can be put in.
  cells.sort((a, b) => a.startBit - b.startBit || a.index - b.index);
  if (cells.length === 0) return NO_VALUES;
  if (aligned) return finish("aligned", cells, 0, 1, 0, o.valLine);
  const column = o.noteWidth || COLUMN_GUESS;
  if (o.layout === "flow") {
    const wrapped = wrapFlow(cells, column, o.maxLines);
    return finish("flow", wrapped.kept, 0, wrapped.lines, wrapped.rest, o.valLine);
  }
  const width = Math.max(1, o.cellWidth);
  const { perLine, lastLine } = uniformFit(column, width);
  const want = Math.ceil(cells.length / perLine);
  if (want <= o.maxLines) return finish("uniform", cells, width, want, 0, o.valLine);
  // Past the cap the last line keeps room for the count of what is left.
  const shown = perLine * (o.maxLines - 1) + lastLine;
  const kept = cells.slice(0, Math.min(cells.length - 1, shown));
  return finish("uniform", kept, width, o.maxLines, cells.length - kept.length, o.valLine);
}

/** Which piece of a straddling element this row holds, counting from the row
 *  the element starts on. */
function pieceIndex(startBit: number, rowFrom: number, rowBits: number): number {
  return -Math.floor((startBit - rowFrom) / rowBits);
}

function finish(
  layout: Layout,
  cells: PlacedCell[],
  cellWidth: number,
  lines: number,
  rest: number,
  valLine: number,
): RowValues {
  const key =
    `${layout[0] ?? ""}${lines}/${rest}/${Math.round(cellWidth)}~` +
    cells.map((c) => (c.carried === null ? `${c.index}=${c.text}` : `${c.carried === "above" ? "^" : "v"}${c.index}`)).join(",");
  return { layout, cells, cellWidth, lines, rest, height: lines * valLine, key };
}

// The arithmetic of a PNG's scanlines, apart from drawing them: Adam7's seven
// passes, which pass stores a pixel, and how many scanlines use each filter.
// The core does the same arithmetic in `codec/scanlines.rs`; this is what the
// report's figure needs of it, for a picture whose size it already has.

import type { TemplateNode } from "../doc.ts";

/** RFC 2083's five filter types, by the number a filter byte holds. */
export const FILTERS = ["None", "Sub", "Up", "Average", "Paeth"] as const;

/** Adam7's seven passes over each 8 by 8 tile: the column and row a pass
 *  starts at, and how far it steps across and down. */
const ADAM7 = [
  [0, 0, 8, 8],
  [4, 0, 8, 8],
  [0, 4, 4, 8],
  [2, 0, 4, 4],
  [0, 2, 2, 4],
  [1, 0, 2, 2],
  [0, 1, 1, 2],
] as const;

export type PassShape = {
  /** 1 to 7. */
  readonly pass: number;
  readonly x0: number;
  readonly y0: number;
  readonly dx: number;
  readonly dy: number;
  /** Pixels across and down. Either can be 0 in a small picture, and then the
   *  pass stores nothing, not even filter bytes. */
  readonly cols: number;
  readonly rows: number;
};

/** The seven passes of a `width` by `height` picture. */
export function adam7Passes(width: number, height: number): PassShape[] {
  return ADAM7.map(([x0, y0, dx, dy], i) => ({
    pass: i + 1,
    x0,
    y0,
    dx,
    dy,
    cols: width > x0 ? Math.ceil((width - x0) / dx) : 0,
    rows: height > y0 ? Math.ceil((height - y0) / dy) : 0,
  }));
}

/** The 8 by 8 tile of RFC 2083's figure: which pass stores each pixel. */
const TILE = [
  [1, 6, 4, 6, 2, 6, 4, 6],
  [7, 7, 7, 7, 7, 7, 7, 7],
  [5, 6, 5, 6, 5, 6, 5, 6],
  [7, 7, 7, 7, 7, 7, 7, 7],
  [3, 6, 4, 6, 3, 6, 4, 6],
  [7, 7, 7, 7, 7, 7, 7, 7],
  [5, 6, 5, 6, 5, 6, 5, 6],
  [7, 7, 7, 7, 7, 7, 7, 7],
] as const;

/** Which pass, 1 to 7, stores the pixel at column `x` and row `y`. */
export function passOf(x: number, y: number): number {
  return TILE[y % 8]?.[x % 8] ?? 0;
}

/** One scanline, as the trace of unfiltering it says. */
export type Scanline = {
  /** 1 to 7 in an interlaced picture, and null in one that is not. */
  readonly pass: number | null;
  /** The row within its pass, from 0, which is the row of the picture when
   *  the picture is not interlaced. */
  readonly row: number;
  /** 0 to 4, an index into `FILTERS`. */
  readonly filter: number;
  /** Its bytes, filter byte included. */
  readonly bytes: number;
  readonly path: readonly number[];
};

/** How many scanlines use each filter, indexed as `FILTERS`. */
export function filterCounts(lines: readonly Scanline[]): number[] {
  const n = FILTERS.map(() => 0);
  for (const l of lines) n[l.filter] = (n[l.filter] ?? 0) + 1;
  return n;
}

/** The image row a scanline is, from its pass and its row within the pass. */
export function pictureRow(line: Scanline): number {
  if (line.pass === null) return line.row;
  const p = ADAM7[line.pass - 1];
  return p === undefined ? line.row : p[1] + line.row * p[3];
}

/** The number a field holds. An enum's value reads as its name with the
 *  number after it in brackets, `adam7 (1)`, and the number is what is
 *  wanted. */
export function numberOf(n: Pick<TemplateNode, "kind" | "value" | "edit_text">): number | null {
  if (n.kind === "enum") {
    const m = /\((0x[0-9a-f]+|-?\d+)\)$/i.exec(n.value);
    return m?.[1] === undefined ? null : Number(m[1]);
  }
  const v = Number(n.edit_text);
  return n.edit_text !== "" && Number.isFinite(v) ? v : null;
}

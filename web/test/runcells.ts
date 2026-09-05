// Fixtures shared by the two value-table test files: a run of fixed-stride
// elements, and a text measurement whose widths are countable by eye.
//
// Not a `*.test.ts`, so `node --test test/*.test.ts` does not try to run it.

import type { ChipMeasure } from "../src/chipfit.ts";
import type { Cell, RunCells } from "../src/valuelayout.ts";

/** Seven pixels a character, near enough to the mono a cell is drawn in, so a
 *  width in these files is a count of characters at that pitch. */
export const CHAR: ChipMeasure = { name: (s) => s.length * 7, value: (s) => s.length * 7 };

/** A run of fixed-stride elements starting at byte 0. */
export function run(o: {
  name?: string;
  type?: string;
  kind?: string;
  stride: number;
  from: number;
  to: number;
  text?: (i: number) => string;
  /** What the cell shows, where that is not what its tooltip says. */
  label?: (i: number) => string;
  contiguous?: boolean;
  symbol?: boolean;
  at?: number;
}): RunCells {
  const cells: Cell[] = [];
  const text = o.text ?? ((n: number) => String(n));
  for (let i = o.from; i < o.to; i++) {
    cells.push({
      index: i,
      offset_bits: (o.at ?? 0) + i * o.stride,
      size_bits: o.stride,
      text: text(i),
      label: (o.label ?? text)(i),
      kind: o.kind ?? "int",
      contiguous: o.contiguous ?? true,
    });
  }
  return {
    path: [3],
    name: o.name ?? "body",
    type: o.type ?? "i24 le",
    symbol: o.symbol ?? false,
    widest: "",
    cells,
  };
}

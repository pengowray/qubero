// Fixtures shared by the two value-table test files: a run of fixed-stride
// elements, and a text measurement whose widths are countable by eye.
//
// Not a `*.test.ts`, so `node --test test/*.test.ts` does not try to run it.

import type { ChipMeasure } from "../src/chipfit.ts";
import type { Cell, RunCells } from "../src/valuelayout.ts";

/** Seven pixels a character, near enough to the mono a cell is drawn in, so a
 *  width in these files is a count of characters at that pitch. */
export const CHAR: ChipMeasure = { name: (s) => s.length * 7, value: (s) => s.length * 7 };

/**
 * A run of packed blocks as the core sends one: per block a `scale` cell for
 * the block's `d`, then one `int` cell per weight at the bits it is stored in,
 * everything in the order of the bytes.
 *
 * The shape rather than the arithmetic of a real `q4_0`: what the layout has
 * to cope with is one float among a couple of dozen small integers, which this
 * has.
 */
export function quantRun(o: {
  blocks: number;
  /** Weights in one block. */
  weights: number;
  /** Bits one weight is stored in. */
  bits: number;
  scaleBits?: number;
  scale?: string;
  contiguous?: boolean;
}): RunCells {
  const scaleBits = o.scaleBits ?? 16;
  const scale = o.scale ?? "0.004108";
  const stride = scaleBits + o.weights * o.bits;
  const cells: Cell[] = [];
  for (let b = 0; b < o.blocks; b++) {
    const at = b * stride;
    cells.push({
      index: b,
      offset_bits: at,
      size_bits: scaleBits,
      text: `d \u{b7} ${scale}`,
      label: scale,
      kind: "scale",
      contiguous: true,
    });
    for (let w = 0; w < o.weights; w++) {
      const q = (w % 16) - 8;
      cells.push({
        index: b,
        offset_bits: at + scaleBits + w * o.bits,
        size_bits: o.bits,
        text: `weight ${w} \u{b7} stored ${q} \u{b7} value 0.004108`,
        label: String(q),
        kind: "int",
        contiguous: o.contiguous ?? true,
      });
    }
  }
  return { path: [6, 0, 3], name: "blocks", type: "Q4_0", symbol: false, widest: "", cells };
}

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

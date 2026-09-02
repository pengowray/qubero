// The arithmetic behind the JPEG cards, with nothing of the browser in it.
//
// Three things a JPEG stores in a shape that is not the shape it means:
// a quantisation table is written along the diagonals and read as a square,
// a Huffman table is written as sixteen counts and read as a set of codes,
// and the sampling factors are written per channel and read as one ratio.
// All three are pure functions of numbers, so they live here and are tested
// without a document, a template or a DOM.

/**
 * Where the coefficient written at each position of a zigzag run belongs in
 * the 8 by 8 block: `ZIGZAG[z]` is the row-major index of the value stored
 * `z`th. Built by walking the diagonals rather than written out, since
 * sixty-four hand-typed numbers are sixty-four chances to mistype one.
 */
export const ZIGZAG: readonly number[] = buildZigzag();

function buildZigzag(): number[] {
  const out: number[] = [];
  let row = 0;
  let col = 0;
  for (let z = 0; z < 64; z++) {
    out.push(row * 8 + col);
    // The diagonals alternate direction: up and to the right, then down and
    // to the left, turning at whichever edge is reached first.
    if ((row + col) % 2 === 0) {
      if (col === 7) row += 1;
      else if (row === 0) col += 1;
      else {
        row -= 1;
        col += 1;
      }
    } else {
      if (row === 7) col += 1;
      else if (col === 0) row += 1;
      else {
        row += 1;
        col -= 1;
      }
    }
  }
  return out;
}

/**
 * A run of sixty-four values in the order the file writes them, put back into
 * the order they are read in: index `row * 8 + col`.
 *
 * A short run leaves holes rather than shifting everything after them, so a
 * truncated table draws in the right places with the missing cells empty.
 */
export function dezigzag<T>(values: readonly T[]): (T | undefined)[] {
  const out: (T | undefined)[] = new Array<T | undefined>(64).fill(undefined);
  for (let z = 0; z < Math.min(64, values.length); z++) {
    const at = ZIGZAG[z];
    if (at !== undefined) out[at] = values[z];
  }
  return out;
}

/** One entry of a decoded Huffman table: the bits a decoder matches, and the
 *  symbol it hands back when they match. */
export type HuffmanCode = {
  /** The code as bits, most significant first: `010`. Its length is the
   *  number of bits, which is the whole point of the table. */
  readonly bits: string;
  readonly length: number;
  readonly symbol: number;
};

/**
 * The codes a JPEG Huffman table stands for. Nothing in the file holds them:
 * the sixteen counts and the run of symbols are enough to rebuild every one,
 * which is the procedure in Annex C of the specification. Codes are assigned
 * in order, shortest first, and each step to a longer length shifts the
 * running code left by one.
 *
 * Null when the counts and the symbols disagree about how many there are, so
 * that a table read wrong is drawn as no table rather than as a plausible
 * wrong one.
 */
export function huffmanCodes(counts: readonly number[], symbols: readonly number[]): HuffmanCode[] | null {
  if (counts.length !== 16) return null;
  let total = 0;
  for (const c of counts) {
    if (!Number.isInteger(c) || c < 0) return null;
    total += c;
  }
  if (total !== symbols.length) return null;
  const out: HuffmanCode[] = [];
  let code = 0;
  let next = 0;
  for (let length = 1; length <= 16; length++) {
    const n = counts[length - 1] ?? 0;
    for (let i = 0; i < n; i++) {
      const symbol = symbols[next];
      if (symbol === undefined) return null;
      next += 1;
      out.push({ bits: code.toString(2).padStart(length, "0"), length, symbol });
      code += 1;
    }
    code <<= 1;
  }
  return out;
}

/** One channel's sampling factors, as the frame header writes them. */
export type Sampling = { readonly h: number; readonly v: number };

/**
 * The subsampling as it is spoken: `4:2:0` and the rest.
 *
 * Only for the shapes the notation was made for, which is three channels
 * whose second and third are sampled once per block. Anything else (four
 * channels, chroma sampled more finely than luma, a JPEG nobody expected)
 * has no J:a:b name, and inventing one would be worse than the per-channel
 * factors the table already shows, so it answers null and the card says
 * nothing.
 */
export function subsampling(components: readonly Sampling[]): string | null {
  const [y, cb, cr] = components;
  if (components.length === 1) return "greyscale";
  if (components.length !== 3 || y === undefined || cb === undefined || cr === undefined) return null;
  if (cb.h !== 1 || cb.v !== 1 || cr.h !== 1 || cr.v !== 1) return null;
  const names: Record<string, string> = {
    "1,1": "4:4:4",
    "1,2": "4:4:0",
    "2,1": "4:2:2",
    "2,2": "4:2:0",
    "4,1": "4:1:1",
    "4,2": "4:1:0",
  };
  return names[`${y.h},${y.v}`] ?? null;
}

/**
 * How many restart markers are in a stretch of entropy-coded data.
 *
 * Inside a scan an 0xff is written with a zero after it, so the only 0xff
 * pairs that mean anything are the eight restarts. Counted rather than
 * decoded: the bits themselves are not read here or anywhere else.
 */
export function countRestarts(bytes: Uint8Array): number {
  let n = 0;
  for (let i = 0; i + 1 < bytes.length; i++) {
    if (bytes[i] !== 0xff) continue;
    const next = bytes[i + 1] ?? 0;
    if (next >= 0xd0 && next <= 0xd7) {
      n += 1;
      i += 1;
    }
  }
  return n;
}

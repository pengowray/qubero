// Which stored byte each bit of a Huffman code came out of, and where the code
// falls in two.
//
// A deflate code is a run of bits at an arbitrary bit offset, so the bytes it
// was packed into have nothing to do with the symbols it carries: a nine-bit
// literal can start four bits into one byte and finish five bits into the
// next, and a match's length code can hand over to its distance code in the
// middle of a third. The panel draws both boundaries, and they are two
// different questions, so they are answered by two functions here.
//
// Split from the renderer for the reason `dom.ts`'s `addressParts` is: the
// arithmetic is the part that can be wrong, and it can be checked without a
// document. Nothing here reads the file. The bits are already a string, the
// offset is already on the node, and a byte address is a division.

/** One clause of the caption under the bit string: how many of the code's bits
 *  came out of one stored byte, and which byte that was. */
export type ByteRun = {
  /** The byte's own address, in the space the field is in. */
  readonly byte: number;
  readonly bits: number;
};

/**
 * The code's bits grouped by the byte they were packed into, in reading order.
 *
 * Bit `i` of a code that begins at `offsetBits` sits in byte
 * `(offsetBits + i) / 8`, rounded down, so the first run is however much of
 * the first byte is left and every run after it is a whole byte until the last.
 * A code inside a single byte is one run, which is what makes the caption read
 * `9 bits from 0x69` there and list three bytes where it spans three.
 *
 * Counted per byte and not per drawn cell: where a match hands over from its
 * length code to its distance code inside one byte, that byte is still one
 * byte and the caption names it once. The cells below split there; this does
 * not.
 */
export function byteRuns(offsetBits: number, count: number): ByteRun[] {
  const out: ByteRun[] = [];
  let at = 0;
  while (at < count) {
    const byte = Math.floor((offsetBits + at) / 8);
    const end = Math.min((byte + 1) * 8 - offsetBits, count);
    out.push({ byte, bits: end - at });
    at = end;
  }
  return out;
}

/** One drawn run of digits: the byte whose fill it takes, the digits
 *  themselves, and whether the code's two parts meet in front of it. */
export type BitCell = {
  readonly byte: number;
  readonly text: string;
  /** True on the first cell of a match's distance code, which is where the
   *  gap goes. Never on the first cell of all, since a gap at the front of the
   *  string would mark a boundary there is nothing on the other side of. */
  readonly gap: boolean;
};

/**
 * The bits cut into the runs the panel draws: at every byte boundary, and at
 * the one place a match's length code gives way to its distance code.
 *
 * Two boundaries, two channels. A byte boundary is drawn as a change of fill,
 * because whitespace already means "byte boundary" everywhere else in this app
 * and a gap here would read backwards. The boundary between the two codes is
 * drawn as a gap, because it is the boundary a reader is being told about.
 *
 * Which is why a cut is made at `split` even in the middle of a byte, and why
 * the cell after it keeps the byte it came from: the two cells either side of
 * that gap are the same byte, take the same fill, and read as one band with a
 * gap in it rather than as two bands. A split that lands on a byte boundary
 * needs no cut of its own and gets none; the byte's own cut is already there
 * and the gap goes on the cell that follows.
 *
 * `split` is a count of bits from the front of the code, or null where there
 * is no second part: a literal, the end mark, and a match whose widths the
 * caller could not make add up to the string it was given. A boundary drawn in
 * the wrong place is worse than no boundary at all, so the caller passing null
 * is how that is said.
 */
export function bitCells(offsetBits: number, bits: string, split: number | null): BitCell[] {
  const cut = split !== null && split > 0 && split < bits.length ? split : null;
  const out: BitCell[] = [];
  let at = 0;
  while (at < bits.length) {
    const byte = Math.floor((offsetBits + at) / 8);
    let end = Math.min((byte + 1) * 8 - offsetBits, bits.length);
    if (cut !== null && cut > at && cut < end) end = cut;
    out.push({ byte, text: bits.slice(at, end), gap: at === cut });
    at = end;
  }
  return out;
}

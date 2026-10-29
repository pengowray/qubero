// A field's bits in the order they are stored against the order they are
// read, as the arithmetic behind the bit-order ribbon (`viz/bitpack.ts`).
//
// Deflate fills each byte from its low bit, so the first bit of a code is bit
// 0 of its byte, the rightmost digit of the byte written out. A Huffman code
// is then read high bit first, the order the stream gives its bits, while the
// extra bits that follow it are a number stored low bit first. Written out as
// numbers, the Huffman code comes out mirrored against the byte and the extra
// bits come out in the byte's own order. Any other field packed from the low
// end of each byte (a GIF code, a FLAC subframe header, the length fields of
// a deflate block's head) is the second kind: one part, a number stored low
// bit first, whose first byte holds its lowest digits.
//
// The figure draws, along the top, every byte the field touches written high
// bit first as a hex editor shows it; along the bottom, each part written as
// the number it is read as; and a band from each bit to where it lands. This
// module says where every bit goes in the two rows, with no document and no
// DOM, so the tests can check it.
//
// It also says where the bits would go if they were read the wrong way, high
// bit first straight off the hex (`reading: "hex"`), which is the reading a
// reader brings to it and the figure can show beside the right one.

/** One part of a field in the order it is read: a Huffman code, or the
 *  extra bits after it, or the whole of a field that is one number. */
export type BitPart = {
  readonly bits: number;
  /** True for a number stored low bit first, such as extra bits. */
  readonly lowFirst: boolean;
  readonly label: string;
};

export type BitOrderLayout = {
  readonly firstByte: number;
  readonly byteCount: number;
  /** For each bit of the field, in the order it is read: its place along the
   *  top, counting the bytes' digits written high bit first. */
  readonly top: readonly number[];
  /** For each bit of the field: its place along the bottom, counting the
   *  parts' digits written as numbers, high digit first. */
  readonly bottom: readonly number[];
  /** Which part each bit of the field belongs to. */
  readonly partOf: readonly number[];
};

/** How the bottom row reads the bits: the way the format reads them, or the
 *  way the hex view shows them, high bit first left to right. */
export type BitReading = "format" | "hex";

/**
 * Where each bit of a field sits in the two orders. `offsetBits` counts the
 * stream's own way, low bit of each byte first, which is how the core places
 * a field read low bit first: bit `i` of the field is bit `(offsetBits + i) %
 * 8` of byte `(offsetBits + i) / 8`, counted from the low end.
 *
 * With `reading: "hex"`, each part's digits are its bits in the order they sit
 * along the top, so no band crosses inside a part: the number a reader gets
 * by copying the digits off the hex view.
 */
export function bitOrderLayout(offsetBits: number, count: number, parts: readonly BitPart[], reading: BitReading = "format"): BitOrderLayout {
  const firstByte = Math.floor(offsetBits / 8);
  const lastByte = Math.floor((offsetBits + Math.max(1, count) - 1) / 8);
  const top: number[] = [];
  const bottom: number[] = [];
  const partOf: number[] = [];
  const partStarts: number[] = [];
  let partStart = 0;
  let inPart = 0;
  let part = 0;
  for (let i = 0; i < count; i++) {
    const bit = offsetBits + i;
    const byte = Math.floor(bit / 8);
    const low = bit % 8;
    top.push((byte - firstByte) * 8 + (7 - low));
    while (part < parts.length && inPart >= (parts[part]?.bits ?? 0)) {
      partStart += parts[part]?.bits ?? 0;
      inPart = 0;
      part++;
    }
    const p = parts[part];
    const width = p?.bits ?? count;
    bottom.push(partStart + (p?.lowFirst === true ? width - 1 - inPart : inPart));
    partOf.push(Math.min(part, Math.max(0, parts.length - 1)));
    partStarts.push(partStart);
    inPart++;
  }
  if (reading === "hex") {
    // Each part's bits ranked by where they sit along the top.
    const byPart = new Map<number, number[]>();
    partOf.forEach((p, i) => byPart.set(p, [...(byPart.get(p) ?? []), i]));
    for (const bits of byPart.values()) {
      const ranked = [...bits].sort((a, b) => (top[a] ?? 0) - (top[b] ?? 0));
      ranked.forEach((i, rank) => {
        bottom[i] = (partStarts[i] ?? 0) + rank;
      });
    }
  }
  return { firstByte, byteCount: lastByte - firstByte + 1, top, bottom, partOf };
}

/** A field's digits in the order it reads them, from the bytes it touches
 *  (`bytes[0]` is byte `layout.firstByte`): what a deflate code's text is,
 *  worked out for a field that has no such text. */
export function readOrderDigits(bytes: ArrayLike<number>, offsetBits: number, count: number): string {
  const first = Math.floor(offsetBits / 8);
  let out = "";
  for (let i = 0; i < count; i++) {
    const bit = offsetBits + i;
    out += ((bytes[Math.floor(bit / 8) - first] ?? 0) >> (bit % 8)) & 1 ? "1" : "0";
  }
  return out;
}

/** The value of each part, from the digits along the bottom: what the part
 *  reads as, high digit first. `digits[i]` is bit `i` of the field. */
export function partValues(layout: BitOrderLayout, digits: string, parts: readonly BitPart[]): number[] {
  const values = parts.map(() => 0);
  const base: number[] = [];
  let at = 0;
  for (const p of parts) {
    base.push(at);
    at += p.bits;
  }
  layout.bottom.forEach((slot, i) => {
    const p = layout.partOf[i] ?? 0;
    const width = parts[p]?.bits ?? 0;
    const place = width - 1 - (slot - (base[p] ?? 0));
    if (digits[i] === "1") values[p] = (values[p] ?? 0) + 2 ** place;
  });
  return values;
}

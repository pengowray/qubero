// The arithmetic behind the bit-scale pieces, with no document and no DOM:
// where each bit of a word goes on a bit ruler (`bitruler.ts`), and how the
// payload bits of a variable-length number gather into the number
// (`varintfigure.ts`). The bit-order ribbon's own arithmetic is older and
// lives in `bitorder.ts`.
//
// Two numberings meet here, and every function says which it takes:
// - A file bit, as the hex view and the linked bus count it: bit 0 is the
//   top bit of byte 0, so bit `byte * 8 + (7 - low)` is the bit `low` places
//   up from the bottom of `byte`.
// - A bit of a value, counted from its low end: bit 3 of a flags word is the
//   one with weight 8, wherever the word's byte order put it.
//
// A ruler draws a word's bytes in file order, because that is the order the
// hex view shows them in and the reader is matching one to the other. Inside
// each byte it draws the bits high bit first (as the hex view does) or low bit
// first (as a stream packed from the bottom of each byte reads them, so that a
// field crossing a byte boundary is one unbroken box).

/** Which way a ruler draws the bits inside each byte. */
export type CellOrder = "high-first" | "low-first";

/** One cell of a ruler: one bit of the file. */
export type RulerCell = {
  /** Where it is drawn: which row, and which column of that row. */
  readonly row: number;
  readonly col: number;
  /** The byte it is in, as an address. */
  readonly byte: number;
  /** Which bit of that byte, counted from the low end, 0 to 7. */
  readonly low: number;
  /** The same bit in the hex view's numbering. */
  readonly bit: number;
};

/** The cells of `byteCount` bytes from `firstByte`, `perRow` to a row. */
export function rulerCells(firstByte: number, byteCount: number, perRow: number, order: CellOrder): RulerCell[] {
  const cells: RulerCell[] = [];
  const per = Math.max(1, perRow);
  for (let k = 0; k < byteCount * 8; k++) {
    const byte = firstByte + Math.floor(k / 8);
    const pos = k % 8;
    const low = order === "high-first" ? 7 - pos : pos;
    cells.push({ row: Math.floor(k / per), col: k % per, byte, low, bit: byte * 8 + (7 - low) });
  }
  return cells;
}

/** A stretch of neighbouring cells in one row, `[col0, col1)`. */
export type CellRun = { readonly row: number; readonly col0: number; readonly col1: number };

/** The cells `has` picks out, as runs: a field drawn as boxes, one box per
 *  stretch of neighbours, broken where the row wraps or where the byte order
 *  puts its bits apart. Runs come in drawing order. */
export function runsOf(cells: readonly RulerCell[], has: (bit: number) => boolean): CellRun[] {
  const runs: { row: number; col0: number; col1: number }[] = [];
  for (const c of cells) {
    if (!has(c.bit)) continue;
    const last = runs[runs.length - 1];
    if (last !== undefined && last.row === c.row && last.col1 === c.col) last.col1++;
    else runs.push({ row: c.row, col0: c.col, col1: c.col + 1 });
  }
  return runs;
}

/** The widest row of 32, 16 or 8 bits that fits `px` at `cellPx` a bit, but
 *  never wider than the word: a two-byte word is one row of 16 even where 32
 *  would fit. Eight when not even that fits, since a byte is never split. */
export function rowBitsFor(byteCount: number, px: number, cellPx: number): 8 | 16 | 32 {
  const most = byteCount * 8;
  for (const w of [32, 16] as const) if (w <= most && w * cellPx <= px) return w;
  return 8;
}

/** Which bit of a value, counted from its low end, sits at bit `low` of byte
 *  `index` of a word `byteCount` bytes long. */
export function valueBit(index: number, low: number, byteCount: number, endian: "le" | "be"): number {
  return (endian === "le" ? index : byteCount - 1 - index) * 8 + low;
}

/** The byte order a flags word was read in, worked out from its bytes and the
 *  value the core read from them: little-endian when that reading gives the
 *  value, big-endian when that one does, null when neither does (the drawing
 *  would then put names on the wrong bits). A one-byte word is either. */
export function wordEndian(bytes: ArrayLike<number>, value: number): "le" | "be" | null {
  let le = 0;
  let be = 0;
  for (let i = 0; i < bytes.length; i++) {
    le += (bytes[i] ?? 0) * 2 ** (8 * i);
    be = be * 256 + (bytes[i] ?? 0);
  }
  if (le === value) return "le";
  if (be === value) return "be";
  return null;
}

/** The file bits of a field, value bit 0 first, in the hex view's numbering.
 *  A field read low bit first (`lsb_first`) fills each byte from the bottom
 *  up; any other field's bits are its offset onwards, high bit first, so its
 *  value bit 0 is the last of them. */
export function fieldValueBits(offsetBits: number, sizeBits: number, lsbFirst: boolean): number[] {
  const out: number[] = [];
  for (let i = 0; i < sizeBits; i++) {
    if (lsbFirst) {
      const at = offsetBits + i;
      out.push(Math.floor(at / 8) * 8 + (7 - (at % 8)));
    } else out.push(offsetBits + sizeBits - 1 - i);
  }
  return out;
}

/** The bytes a field touches, as `[first, count]`. */
export function bytesOf(bits: readonly number[]): readonly [number, number] {
  if (bits.length === 0) return [0, 0];
  const lo = Math.min(...bits.map((b) => Math.floor(b / 8)));
  const hi = Math.max(...bits.map((b) => Math.floor(b / 8)));
  return [lo, hi - lo + 1];
}

// ----- variable-length numbers -----

/** What a decoder does with one run of a varint's bits; `Doc.BitRole`. */
export type GroupRole = "more" | "stop" | "width" | "payload";
export type VarintGroup = { readonly bits: string; readonly role: GroupRole };

/**
 * How the payload groups of a varint join into the number, from the stored
 * order of the groups:
 * - `low-first`: the first group stored is the low end. LEB128, so also the
 *   zigzag and signed forms built on it.
 * - `high-first`: the first group stored is the high end. MIDI's VLQ,
 *   SQLite's varint and EBML.
 * - `first-high-rest-low`: 7z's NUMBER, whose first byte holds the top of
 *   the value and whose later bytes are the rest, lowest first.
 */
export type GatherOrder = "low-first" | "high-first" | "first-high-rest-low";

/** One stored bit of a varint. */
export type StoredBit = {
  /** Which byte of the field, from 0, and which bit of it, high bit first. */
  readonly byte: number;
  readonly slot: number;
  readonly digit: 0 | 1;
  readonly role: GroupRole;
  /** Which of the groups it is in, in stored order. */
  readonly group: number;
  /** Where it lands in the number, counted from the number's high end. Null
   *  for a framing bit, which is not part of the value. */
  readonly place: number | null;
};

export type Gather = {
  readonly stored: readonly StoredBit[];
  /** The groups that make the number, most significant first, as indices
   *  into the stored groups. */
  readonly joined: readonly number[];
  /** How many bits the number has. */
  readonly width: number;
  /** The number the bits make, read as unsigned. */
  readonly value: bigint;
  /** For each group in `joined`, how far it is shifted up: the place of its
   *  lowest bit, counted from the number's low end. */
  readonly shifts: readonly number[];
};

/** How a group counts: payload always, and EBML's length marker as well for
 *  an element ID, which keeps the marker as part of the number. */
function isValue(role: GroupRole, markerIsValue: boolean): boolean {
  return role === "payload" || (markerIsValue && role === "width");
}

/** Gather the groups in `order`. */
export function gather(groups: readonly VarintGroup[], order: GatherOrder, markerIsValue = false): Gather {
  const valueGroups = groups.map((g, i) => ({ g, i })).filter(({ g }) => isValue(g.role, markerIsValue)).map(({ i }) => i);
  let joined: number[];
  if (order === "high-first") joined = valueGroups;
  else if (order === "low-first") joined = [...valueGroups].reverse();
  else joined = valueGroups.length === 0 ? [] : [valueGroups[0] as number, ...valueGroups.slice(1).reverse()];
  const startOf = new Map<number, number>();
  let width = 0;
  for (const i of joined) {
    startOf.set(i, width);
    width += groups[i]?.bits.length ?? 0;
  }
  const shifts = joined.map((i) => width - (startOf.get(i) ?? 0) - (groups[i]?.bits.length ?? 0));
  const stored: StoredBit[] = [];
  let at = 0;
  let digits = "";
  for (const i of joined) digits += groups[i]?.bits ?? "";
  groups.forEach((g, gi) => {
    const start = startOf.get(gi);
    for (let k = 0; k < g.bits.length; k++) {
      stored.push({
        byte: Math.floor(at / 8),
        slot: at % 8,
        digit: g.bits[k] === "1" ? 1 : 0,
        role: g.role,
        group: gi,
        place: start === undefined ? null : start + k,
      });
      at++;
    }
  });
  const value = digits === "" ? 0n : BigInt(`0b${digits}`);
  return { stored, joined, width, value, shifts };
}

/** What the unsigned number reads as, by how the type says to take it. */
export type Reading = "unsigned" | "signed" | "zigzag";

/** The number `value`, `width` bits wide, read the given way: two's
 *  complement for signed LEB128, and zigzag's fold of the sign into the
 *  lowest bit for protobuf's `sint`. */
export function readAs(value: bigint, width: number, reading: Reading): bigint {
  if (reading === "unsigned" || width === 0) return value;
  if (reading === "signed") return value >= 1n << BigInt(width - 1) ? value - (1n << BigInt(width)) : value;
  return (value >> 1n) ^ -(value & 1n);
}

/** The order and the reading a type name implies. A template's own name for
 *  a varint type falls through to null, and the caller tries every order
 *  against the value. */
export function orderFor(type: string, rule: string): { readonly order: GatherOrder; readonly reading: Reading } | null {
  const t = type.toLowerCase();
  if (rule === "sevenzip_number" || t === "7z number") return { order: "first-high-rest-low", reading: "unsigned" };
  if (rule === "ebml_size" || rule === "ebml_id") return { order: "high-first", reading: "unsigned" };
  if (t === "leb128" || t === "uleb128") return { order: "low-first", reading: "unsigned" };
  if (t === "sleb128") return { order: "low-first", reading: "signed" };
  if (t === "zigzag") return { order: "low-first", reading: "zigzag" };
  if (t === "vlq" || t === "varint" || t === "sqlite varint" || rule === "sqlite_ninth") return { order: "high-first", reading: "unsigned" };
  return null;
}

/** The whole number a field's value text starts with, or null for text that
 *  is not one: a name, or nothing. */
export function leadingInteger(text: string): bigint | null {
  const m = /^\s*(-?\d+)/.exec(text.replace(/[−]/g, "-").replace(/[,_  ]/g, ""));
  if (m === null) return null;
  try {
    return BigInt(m[1] as string);
  } catch {
    return null;
  }
}

/**
 * The gather that gives the value the field says it has. The type's own
 * order is tried first and every other one after it, each read the type's
 * way and then every other way; the first that gives `valueText` wins. Null
 * when none does, or the value is not a number to check against: bands drawn
 * to the wrong place in the number are worse than none.
 */
export function checkedGather(
  groups: readonly VarintGroup[],
  type: string,
  rule: string,
  valueText: string,
): { readonly gather: Gather; readonly order: GatherOrder; readonly reading: Reading } | null {
  const want = leadingInteger(valueText);
  if (want === null) return null;
  const hint = orderFor(type, rule);
  const orders: GatherOrder[] = ["low-first", "high-first", "first-high-rest-low"];
  const readings: Reading[] = ["unsigned", "signed", "zigzag"];
  if (hint !== null) {
    orders.sort((a, b) => (a === hint.order ? -1 : b === hint.order ? 1 : 0));
    readings.sort((a, b) => (a === hint.reading ? -1 : b === hint.reading ? 1 : 0));
  }
  const markerIsValue = rule === "ebml_id";
  for (const order of orders) {
    const g = gather(groups, order, markerIsValue);
    for (const reading of readings) if (readAs(g.value, g.width, reading) === want) return { gather: g, order, reading };
  }
  return null;
}

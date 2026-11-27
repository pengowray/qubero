// Numbers as a reader reads them: an address, and a size.
//
// Their own module because the copy in `strings.ts` needs them and `doc.ts`
// needs the copy, and a cycle between the two is a cycle nobody wants to think
// about at load time. They depend on nothing, which is what makes them the
// half to move.

/**
 * The mark that says a number is a place in a file and not a value.
 *
 * `0x` alone said both. A treemap box called `0xa60` sat beside boxes called
 * `Zeros` and `Data`, and `Written at +0x1c` sat two rows under a literal
 * byte of `0x4d`, with nothing but the reader's memory to tell the two apart.
 *
 * `@` over the other conventions because the others assert a base. `$A60` and
 * `A60h` both mean hex and would have to be unpicked the day addresses can be
 * shown in decimal; `@` says only "a place", which stays true in any base.
 * `[0x..]` was out because this app already writes array indices that way, so
 * `records[0]` and an address would have looked like the same kind of thing.
 * Colour was tried and pulled: it shouted, and an address is not an alarm.
 *
 * It goes in front of everything, the `+` of a stream address included, so
 * every address on screen starts with the same character and the eye can
 * stop reading after one.
 */
export const ADDRESS_MARK = "@";

/**
 * An address with nothing in front of it, for the callers that put the mark
 * back themselves because something else has to go between.
 *
 * Sub-byte offsets carry the bit as `+3b`, since a field that starts inside a
 * byte has no plain address.
 */
export function offsetDigits(bits: number): string {
  const byte = Math.floor(bits / 8);
  const rem = bits % 8;
  return `${byteDigits(byte)}${rem === 0 ? "" : `+${rem}b`}`;
}

/** The base addresses are written in, and field values that are offsets with
 *  them. The reader picks it in Settings; `main.ts` sets it at start. */
export type AddressBase = "hex" | "dec";

let addressBase: AddressBase = "hex";

export function setAddressBase(base: AddressBase): void {
  addressBase = base;
}

export function getAddressBase(): AddressBase {
  return addressBase;
}

/** A whole-byte position in the chosen base: `0x1f` or `31`. */
export function byteDigits(byte: number | bigint): string {
  return addressBase === "hex" ? `0x${byte.toString(16)}` : byte.toString(10);
}

/**
 * The value of a field the template uses as a position, written the way
 * addresses are: `0x40` where addresses are hex. Null where the plain value
 * should stand: the field is not a position, not a whole number of zero or
 * more, or addresses are decimal, which is how the value is written anyway.
 * A signed offset is a position too: a CDF writes its offsets as `Int64`, and
 * the -1 that means "nowhere" stays decimal. `value` is the text the field
 * is shown as, where the caller has it apart from `edit_text`.
 *
 * Worked out from `edit_text`, the decimal digits the editor starts from, so
 * a 64-bit offset keeps every digit.
 */
export function positionValue(node: { readonly locates?: boolean; readonly kind: string; readonly edit_text: string; readonly value?: string }): string | null {
  if (addressBase !== "hex" || !node.locates) return null;
  // An enum is a number with names for some of its values: an HDF5 address
  // names only the value that means "undefined". A value with a name keeps
  // the name, so only one that is shown as its bare digits is rewritten.
  // The core writes a value that has no name as `75668 (unknown)`.
  const named = node.kind === "enum" && node.value !== undefined && node.value !== node.edit_text && node.value !== `${node.edit_text} (unknown)`;
  if ((node.kind !== "uint" && node.kind !== "int" && node.kind !== "enum") || named) return null;
  const digits = node.edit_text;
  if (!/^\d+$/.test(digits)) return null;
  return `0x${BigInt(digits).toString(16)}`;
}

/**
 * An address, marked: `@0x40`, or `@0x69+7b` where it falls inside a byte.
 *
 * Lowercase hex, as the gutter writes it. The gutter itself is the one place
 * that leaves the mark off: a column of nothing but addresses, in the spot
 * every hex editor puts them, says what it is by being there.
 */
export function formatOffset(bits: number): string {
  return `${ADDRESS_MARK}${offsetDigits(bits)}`;
}

/**
 * An address, in whatever space it belongs to. A field of the file gets the
 * plain address; one inside a decoded stream gets a `+` as well, because
 * `0x1c` of a stream and `0x1c` of the file are different bytes and a reader
 * comparing the listing against the hex view has to be able to tell.
 *
 * The mark stays in front of the `+`: `@+0x1c`, not `+@0x1c`. Every address
 * on screen begins with the same character, and what the address is counted
 * from is the next question, not the first one.
 */
export function formatAddress(bits: number, space: number): string {
  return space === 0 ? formatOffset(bits) : `${ADDRESS_MARK}+${offsetDigits(bits)}`;
}

/** A size, in the units a reader would say it in. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let x = n / 1024;
  let i = 0;
  while (x >= 1024 && i < units.length - 1) {
    x /= 1024;
    i++;
  }
  return `${x < 10 ? x.toFixed(2) : x < 100 ? x.toFixed(1) : Math.round(x)} ${units[i]}`;
}

/** A limit, which is usually a whole number of the largest unit it reaches
 *  and is then written without decimals: `32 MiB`, where `formatBytes`
 *  writes `32.0 MiB`. Any other size is written as `formatBytes` writes
 *  it. */
export function formatLimit(n: number): string {
  const units: readonly (readonly [string, number])[] = [["GiB", 2 ** 30], ["MiB", 2 ** 20], ["KiB", 2 ** 10]];
  const unit = units.find(([, size]) => n >= size);
  return unit !== undefined && n % unit[1] === 0 ? `${n / unit[1]} ${unit[0]}` : formatBytes(n);
}

/** A share of the whole: `48%`, or `<1%` rather than a zero that reads as
 *  nothing at all. Here rather than in the panel that first wanted it, since
 *  the treemap asks the same question about the same numbers. */
export function percentText(part: number, whole: number): string {
  if (whole === 0) return "0%";
  const p = (part / whole) * 100;
  if (p > 0 && p < 1) return "<1%";
  if (p < 100 && p > 99) return ">99%";
  return `${Math.round(p)}%`;
}

/** A byte as the hex gutter writes it, with its character where it has one. */
export function byteText(v: number): string {
  const hex = `0x${v.toString(16).padStart(2, "0")}`;
  return v >= 0x20 && v < 0x7f ? `${hex} ${String.fromCharCode(v)}` : hex;
}

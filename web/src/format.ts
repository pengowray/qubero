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
  return `0x${byte.toString(16)}${rem === 0 ? "" : `+${rem}b`}`;
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

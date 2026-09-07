// Numbers as a reader reads them: an address, and a size.
//
// Their own module because the copy in `strings.ts` needs them and `doc.ts`
// needs the copy, and a cycle between the two is a cycle nobody wants to think
// about at load time. They depend on nothing, which is what makes them the
// half to move.

/**
 * An address, as the gutter writes it. Sub-byte offsets carry the bit as
 * `+3b`, since a field that starts inside a byte has no plain address.
 */
export function formatOffset(bits: number): string {
  const byte = Math.floor(bits / 8);
  const rem = bits % 8;
  return `0x${byte.toString(16)}${rem === 0 ? "" : `+${rem}b`}`;
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

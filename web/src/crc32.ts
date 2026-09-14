/**
 * CRC-32, the sum a ZIP entry's header carries, carried on over pieces of a
 * file as they are read. Its own module so the worker that sums a large folder
 * off the main thread loads nothing else.
 */

/** CRC-32 tables for slicing by eight: table `k` is the CRC of a byte followed
 *  by `k` zero bytes. */
const TABLES: Uint32Array[] = (() => {
  const t0 = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t0[n] = c >>> 0;
  }
  const tables = [t0];
  for (let k = 1; k < 8; k++) {
    const prev = tables[k - 1] as Uint32Array;
    const t = new Uint32Array(256);
    for (let n = 0; n < 256; n++) t[n] = ((prev[n] as number) >>> 8) ^ (t0[(prev[n] as number) & 0xff] as number);
    tables.push(t);
  }
  return tables;
})();

/** Carry a CRC-32 on over more bytes. Start from 0; the result is the CRC of
 *  everything handed in so far. */
export function crc32Update(crc: number, bytes: Uint8Array): number {
  const [t0, t1, t2, t3, t4, t5, t6, t7] = TABLES as [Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array, Uint32Array];
  let c = ~crc >>> 0;
  let i = 0;
  const n = bytes.length;
  for (; i + 8 <= n; i += 8) {
    c ^= (bytes[i] as number) | ((bytes[i + 1] as number) << 8) | ((bytes[i + 2] as number) << 16) | ((bytes[i + 3] as number) << 24);
    c =
      (t7[c & 0xff] as number) ^
      (t6[(c >>> 8) & 0xff] as number) ^
      (t5[(c >>> 16) & 0xff] as number) ^
      (t4[c >>> 24] as number) ^
      (t3[bytes[i + 4] as number] as number) ^
      (t2[bytes[i + 5] as number] as number) ^
      (t1[bytes[i + 6] as number] as number) ^
      (t0[bytes[i + 7] as number] as number);
  }
  for (; i < n; i++) c = (t0[(c ^ (bytes[i] as number)) & 0xff] as number) ^ (c >>> 8);
  return ~c >>> 0;
}

/** The archive was not finished, or its sums not taken: the signal said stop. */
export class Stopped extends Error {
  constructor() {
    super("stopped");
  }
}

/** The CRC-32 of a file's bytes, read a piece at a time. `read` is told how
 *  many more bytes have been read after each piece. */
export async function crcOf(file: Blob, read: (bytes: number) => void, signal?: AbortSignal): Promise<number> {
  const reader = file.stream().getReader();
  let crc = 0;
  for (;;) {
    if (signal?.aborted === true) {
      await reader.cancel();
      throw new Stopped();
    }
    const { done, value } = await reader.read();
    if (done) return crc;
    crc = crc32Update(crc, value);
    read(value.length);
  }
}

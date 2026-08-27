/** Integrity helpers. Checks are requested work: parsing a field must never
 * force a multi-gigabyte file through a checksum as the cursor passes it. */

let crcTable: Uint32Array | null = null;

function table(): Uint32Array {
  if (crcTable !== null) return crcTable;
  const made = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = (c & 1) === 0 ? c >>> 1 : 0xedb88320 ^ (c >>> 1);
    made[n] = c >>> 0;
  }
  crcTable = made;
  return made;
}

/** The CRC-32 used by PNG, ZIP, and gzip (IEEE polynomial). */
export function crc32(bytes: Uint8Array): number {
  const t = table();
  let crc = 0xffffffff;
  for (const byte of bytes) crc = t[(crc ^ byte) & 0xff]! ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

export function hex32(value: number): string {
  return `0x${(value >>> 0).toString(16).padStart(8, "0")}`;
}

export async function sha256(bytes: Uint8Array): Promise<string> {
  // Copy into an ArrayBuffer-backed view: TS 5.9 correctly remembers that an
  // arbitrary Uint8Array may instead wrap SharedArrayBuffer, which WebCrypto
  // and Blob do not accept.
  const digest = await crypto.subtle.digest("SHA-256", Uint8Array.from(bytes));
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

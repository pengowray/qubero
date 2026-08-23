// Dev-only: a fake file of any size with deterministic content, so large-file
// behaviour can be exercised without a real multi-gigabyte file on disk.

import type { ByteSource } from "./doc.js";

export function syntheticFile(size: number): ByteSource {
  return {
    size,
    name: `synthetic-${size}`,
    slice(start, end) {
      return {
        arrayBuffer: async () => {
          const out = new Uint8Array(end - start);
          for (let i = 0; i < out.length; i++) {
            const off = start + i;
            // Row-ish pattern: low byte of offset, with the address bytes every 16.
            out[i] = off % 16 < 8 ? (off >>> ((off % 8) * 4)) & 0xff : off & 0xff;
          }
          await new Promise((r) => setTimeout(r, 5)); // pretend to be a disk
          return out.buffer;
        },
      };
    },
  };
}

export function parseSize(s: string): number | null {
  const m = /^(\d+(?:\.\d+)?)\s*([kmgt]?)b?$/i.exec(s.trim());
  if (!m) return null;
  const mult = { "": 1, k: 1024, m: 1024 ** 2, g: 1024 ** 3, t: 1024 ** 4 }[m[2]?.toLowerCase() ?? ""];
  return mult === undefined ? null : Math.floor(Number(m[1]) * mult);
}

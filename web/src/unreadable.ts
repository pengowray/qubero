// Bytes of a tab that cannot be made: a part of a joined stream that will not
// unpack, which reads as zeroes. The views that draw bytes mark these rather
// than drawing the zeroes, and the ones that read a value from them say why
// instead. Kept apart from the views so the arithmetic can be tested without
// a document. See `Unreadable` in `doc.ts`.

import type { Unreadable } from "./doc.ts";

/** Whether `u` covers any of bytes `at..at + len`. False for no stretch. */
export function reaches(u: { readonly at: number; readonly len: number } | null | undefined, at: number, len: number): boolean {
  return u != null && len > 0 && at < u.at + u.len && at + len > u.at;
}

/** The first of `list` that covers any of bytes `at..at + len`. */
export function stretchAt(list: readonly Unreadable[], at: number, len: number): Unreadable | undefined {
  return list.find((u) => reaches(u, at, len));
}

/** `bytes`, read from byte `at`, with every byte inside `u` set to `fill`:
 *  what a view that writes bytes as text writes in their place. */
export function masked(bytes: Uint8Array, at: number, u: { readonly at: number; readonly len: number } | null | undefined, fill: number): Uint8Array {
  if (u == null) return bytes;
  return bytes.map((b, i) => (reaches(u, at + i, 1) ? fill : b));
}

/** The error a view gives for a read whose bytes cannot be made, or null for
 *  one that read. `why` names the part: `blocks[2].compressed unpacks to
 *  65280 bytes, not the 65281 the file says`. */
export function unreadableError(read: { readonly unreadable?: Unreadable | null }): Error | null {
  return read.unreadable == null ? null : new Error(`Can't read these bytes: ${read.unreadable.why}`);
}

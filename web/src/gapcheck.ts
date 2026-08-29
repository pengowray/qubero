// What is actually in the bytes no field covers.
//
// Rule 1 asks for every byte to be accounted for, and a row that names a run
// without saying anything about it invites the reader to assume it is empty.
// So the run is read and looked at, rather than described from its position.
//
// The whole-file scan behind the overview cannot answer this: its buckets are
// sized to the file, so on anything large one bucket is wider than the run
// being asked about. The focus scan can, but it holds one block at a time and
// the overview panel is already using it; two views taking turns on one slot
// would restart each other's work. Reading the bytes is cheaper than either
// for the sizes worth reading, and honest about the sizes that are not.

import type { Doc } from "./doc.js";

/** The most that is read to answer the question. Past this the answer is that
 *  nobody looked: a gap can be the greater part of a file, and pulling it
 *  through memory to prove it is empty would cost more than the answer. */
export const CHECK_LIMIT_BYTES = 64 * 1024;

/** Two ways of having no answer, which are not the same: one is settled and
 *  one is waiting. A run past the cap will never be checked; a run whose bytes
 *  have not arrived will be, when they do. */
export type GapVerdict = "zeros" | "something" | "too-large" | "unread";

/**
 * Whether a run of bytes is all zero, something else, or unanswered.
 *
 * `zeros` is only ever returned after reading the whole run. Reading the first
 * part of a long one and finding zeros says nothing about the rest, and a
 * verdict that claims otherwise is worse than no verdict at all.
 */
export function checkGap(doc: Doc, offsetBits: number, sizeBits: number): GapVerdict {
  if (sizeBits <= 0 || offsetBits % 8 !== 0 || sizeBits % 8 !== 0) return "too-large";
  const bytes = sizeBits / 8;
  if (bytes > CHECK_LIMIT_BYTES) return "too-large";
  const { bytes: data, complete } = doc.read(offsetBits / 8, bytes);
  if (!complete) return "unread";
  for (const b of data) if (b !== 0) return "something";
  return "zeros";
}

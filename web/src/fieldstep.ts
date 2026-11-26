/**
 * Where Alt+↑ and Alt+↓ take the cursor in the hex view.
 *
 * A field of 32 KiB is hundreds of rows, and the arrow keys move one row at a
 * time. These two move by fields instead: Alt+↓ goes to the start of the next
 * field, and Alt+↑ goes to the start of the field the cursor is in, or to the
 * start of the one before it when the cursor is already at a start. That is
 * how Ctrl+↑ moves by paragraphs in a word processor, so pressing it twice in
 * the middle of a field reaches the start of that field and then the previous
 * one.
 *
 * "The field" is the deepest one at the cursor: the same field the inspector
 * shows and the chip beside the bytes names. With Shift held the same two
 * keys step by the list or structure around it; see `stepDepth`.
 *
 * Kept apart from the view so it can be tested without a page: the view hands
 * in a function that finds the deepest field covering a bit.
 */

/**
 * How deep Alt+Shift+↑ and Alt+Shift+↓ step, as a path length, for the field
 * at `path`: one level up from it. For an element of a list that is the list,
 * so a million-element list is stepped over in one press; for a field of a
 * PNG chunk it is the chunk, so the keys go from chunk to chunk. Never the
 * whole file, which there is nothing before or after.
 *
 * The nearest list around the field was tried first and went too far: from
 * the CRC of one PNG chunk it stepped over every chunk at once.
 */
export function stepDepth(path: readonly number[]): number {
  return Math.max(1, path.length - 1);
}

/** The deepest field covering a bit: where it starts and where it ends, in
 *  bits. Null where no field covers it, or where the answer is not known yet. */
export type FieldAt = (bit: number) => { readonly path: readonly number[]; readonly start: number; readonly end: number } | null;

/** The field to go to, or null when there is nowhere to go. */
export function fieldStep(at: FieldAt, cursorBit: number, dir: "up" | "down", lengthBits: number): { readonly path: readonly number[]; readonly start: number } | null {
  const here = at(cursorBit);
  if (here === null) return null;
  if (dir === "up") {
    if (cursorBit > here.start) return { path: here.path, start: here.start };
    if (here.start === 0) return null;
    const before = at(here.start - 1);
    return before === null ? null : { path: before.path, start: before.start };
  }
  // A field of no bits ends where it starts; step past it rather than find it
  // again.
  const end = Math.max(here.end, here.start + 1);
  if (end >= lengthBits) return null;
  const next = at(end);
  return next === null ? null : { path: next.path, start: next.start };
}

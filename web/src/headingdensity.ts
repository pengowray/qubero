// How much structure a run of siblings is worth drawing.
//
// A heading is a division of the file: a band of its own, a colour, a place in
// the rail, a line across the hex view. That is right for three SQLite pages
// and absurd for two hundred pickle opcodes a byte each, and the listing had
// one rule for both — every composite child at depth 1 became a heading, at
// any size.
//
// Two things settle it, in this order:
//
// 1. What the template said. `inline` already means "one row, not one row per
//    field", and a structure that has said so should be believed by every
//    surface rather than only by the listing.
// 2. What the shapes say, judged over the whole run rather than element by
//    element. Per-element would give a ragged list: a pickle's 24-byte
//    SHORT_BINUNICODE would earn a band while the MEMOIZE either side of it
//    would not, which reads worse than giving every one of them a band.
//
// A view may override both, which is why this is a module of its own rather
// than a rule buried in `flatten`: see `density` in `FlatOptions`, and
// `records.ts` for the same argument about record tables. Changing how a
// format is drawn should not mean changing what its template says the bytes
// are.

import type { TemplateNode } from "./doc.js";

/** How a run of a structure's children is drawn. */
export type Density =
  /** Each composite among them is a part of the file, with a heading of its
   *  own. What a SQLite database's pages are. */
  | "headings"
  /** They are rows, whatever is inside them. What a pickle's opcodes are:
   *  each is a code and an operand, and a band naming one byte says less than
   *  the row it displaces. */
  | "rows";

/** The size at which a structure stops being a value and starts being a part
 *  of the file. One line of the hex view at its usual width: a structure a
 *  reader can take in on one row is a row.
 *
 *  Not a share of the parent, which a small file makes nonsense of: one byte
 *  is half of a two-byte file and is still one byte. */
export const HEADING_MIN_BYTES = 16;

/** How big the run is, taken from the middle of it so that a single outlier
 *  does not decide it: a pickle frame is mostly one-byte opcodes with a few
 *  long strings among them, and the strings should not buy bands for the
 *  opcodes.
 *
 *  A structure of no bytes is left out rather than counted as small. A field
 *  that points somewhere else costs nothing where it is declared and its
 *  contents are elsewhere entirely: an ELF's program headers are a nought-byte
 *  node and a hundred and twenty-eight bytes of segments. Null when every one
 *  of them is like that, since then the run has said nothing about its size. */
function middleBytes(nodes: readonly TemplateNode[]): number | null {
  const sizes = nodes.map((n) => n.size_bits / 8).filter((b) => b > 0).sort((a, b) => a - b);
  return sizes[sizes.length >> 1] ?? null;
}

/**
 * How to draw a run of siblings, from the shapes alone. `nodes` is the run as
 * it is drawn, which may be a page of a longer list; that is the right sample,
 * since a list is drawn the same way the whole way down.
 *
 * A run with no composites in it has nothing to decide and answers `headings`,
 * which is what the caller does with a run of plain fields anyway.
 */
export function densityOf(nodes: readonly TemplateNode[]): Density {
  const composites = nodes.filter((n) => n.composite && n.child_count > 0);
  if (composites.length === 0) return "headings";
  // The template's own word, where it has one: a structure declared inline is
  // a row, and one of them in the run settles the run.
  if (composites.some((n) => n.inline)) return "rows";
  const middle = middleBytes(composites);
  return middle !== null && middle < HEADING_MIN_BYTES ? "rows" : "headings";
}

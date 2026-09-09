/**
 * What a structure holds, for a panel that has room for one line and a short
 * list rather than a whole tree.
 *
 * A composite has no value of its own, so the inspector used to say how many
 * children it had and stop there. That is the right answer for a page of a
 * database and the wrong one for the two commonest shapes in any format: a
 * length beside the string it sizes, which is one value written in two fields,
 * and a handful of numbers in a row, which is one value written in four. Both
 * fit in the box the value would have gone in, so both go there.
 *
 * Which child is the payload is not decided by its name. The template already
 * says which sibling a field settles the length, count, type or place of, in
 * `consumed_by`; a structure whose every other child points at one child is a
 * structure with one value in it and machinery around it.
 */

import type { TemplateNode } from "./doc.ts";

/** How many children the inspector reads at a time. Enough that a header
 *  arrives whole rather than as a teaser, and few enough that a list of a
 *  quarter of a million tokens does not come back at all. */
export const CHILD_PAGE = 12;

/** Most items joined into a one-line preview, and the longest that line gets.
 *  Past either the box would be showing a fraction of a value while looking
 *  like the whole of it, which the count it replaced never did. */
export const PREVIEW_ITEMS = 8;
const PREVIEW_CHARS = 48;

/** The one value a structure was written to carry, when it has one. */
export type Inside =
  /** A length and the text it sizes. The node is the payload child, so the
   *  bytes are read by its path and not by arithmetic on the parent's. */
  | { readonly kind: "payload"; readonly node: TemplateNode }
  /** A short row of scalars, already joined. */
  | { readonly kind: "row"; readonly text: string };

/**
 * `kids` is what `templateChildren` returned, which is the front of the
 * structure and not always all of it. Both shapes here are short by
 * definition, so a structure whose children did not all fit has neither.
 */
export function insideValue(node: TemplateNode, kids: readonly TemplateNode[]): Inside | null {
  if (!node.composite || kids.length === 0 || kids.length !== node.child_count) return null;
  const payload = onePayload(kids);
  if (payload !== null && (payload.kind === "str" || payload.kind === "bytes")) return { kind: "payload", node: payload };
  const row = scalarRow(node, kids);
  return row === null ? null : { kind: "row", text: row };
}

/** The child every other child exists to place. Null where the children are
 *  several values rather than one, which is most structures. */
function onePayload(kids: readonly TemplateNode[]): TemplateNode | null {
  if (kids.length < 2) return null;
  let at = -1;
  for (let i = 0; i < kids.length; i++) {
    if (kids[i]?.consumed_by !== null) continue;
    // Two children nothing places are two values, and picking one of them to
    // stand for the structure would be picking which half to hide.
    if (at !== -1) return null;
    at = i;
  }
  if (at === -1) return null;
  if (!kids.every((k, i) => i === at || k.consumed_by === at)) return null;
  return kids[at] ?? null;
}

/** A handful of scalars of one kind, as the reader would write them: a
 *  tensor's two dimensions, the four numbers of a version. A list only, and
 *  written with the brackets its type already carries: `{count, offset}` is
 *  two fields with a job each, and `[2, 64]` would call them an array. */
function scalarRow(node: TemplateNode, kids: readonly TemplateNode[]): string | null {
  if (!node.type.endsWith("[]")) return null;
  if (kids.length < 2 || kids.length > PREVIEW_ITEMS) return null;
  const first = kids[0];
  if (first === undefined) return null;
  // Nothing in a row places anything else in it. A list of elements is peers;
  // a structure that only looks like one is not.
  if (kids.some((k) => k.consumed_by !== null)) return null;
  if (kids.some((k) => k.composite || k.kind !== first.kind || k.value === "")) return null;
  if (first.kind !== "uint" && first.kind !== "int" && first.kind !== "float" && first.kind !== "enum") return null;
  const text = kids.map((k) => k.value).join(", ");
  // Bracketed, so a row of numbers reads as the array literal it is rather
  // than as one value with commas in it.
  return text.length > PREVIEW_CHARS ? null : `[${text}]`;
}

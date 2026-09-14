// The steps from the root down to a field, as the inspector's trail names them.
// Kept apart from `inspector.ts` so which steps fold into one can be read and
// tested without a document or a page.

import type { TemplateNode } from "./doc.ts";

/** What the trail reads of a node. */
export type TrailNode = Pick<TemplateNode, "name" | "type" | "composite" | "list" | "size_bits" | "child_count">;

/** One crumb: what it says, the field it goes to, and whether that field is
 *  the one the trail was asked for. */
export type TrailItem = { readonly label: string; readonly path: readonly number[]; readonly here: boolean };

/**
 * Every step from the root down, each one selectable. A list and the element
 * taken from it are one crumb, `boxes[0]`, because two crumbs for one step
 * doubles the length of a deep path without saying more.
 *
 * `node` answers for a prefix of `path`, or null for one that has not been
 * read, which is a crumb of its own saying so.
 */
export function trailItems(path: readonly number[], node: (path: readonly number[]) => TrailNode | null): TrailItem[] {
  const items: TrailItem[] = [];
  for (let i = 0; i <= path.length; i++) {
    const n = node(path.slice(0, i));
    if (n === null) {
      items.push({ label: "?", path: path.slice(0, i), here: i === path.length });
      continue;
    }
    // A field that reads its contents somewhere else and the thing it points
    // at are one step, the way the listing draws them as one row: the
    // pointer's type names what is there and says it was reached by address,
    // and the crumb goes to the target, which is where the bytes are.
    // Pointers nest, and the outermost type already reads `at → at → X`,
    // so the whole run of them is the one step. See `hop` in `flatten`.
    let at = n;
    let past = i;
    while (past < path.length && path[past] === 0 && at.size_bits === 0 && at.composite && at.child_count === 1) {
      const next = node(path.slice(0, past + 1));
      if (next === null || next.name !== at.name) break;
      at = next;
      past += 1;
    }
    if (past > i) {
      items.push({ label: `${n.name} (${n.type})`, path: path.slice(0, past), here: past === path.length });
      i = past;
      continue;
    }
    if (n.list && i < path.length) {
      // Fold the element index into the list's own name.
      const to = path.slice(0, i + 1);
      items.push({ label: `${n.name}[${path[i]}]`, path: to, here: i + 1 === path.length });
      i += 1;
      continue;
    }
    // A struct field is often called `body`; its type says what it holds.
    const label = n.composite && n.type !== n.name ? `${n.name} (${n.type})` : n.name;
    const previous = items[items.length - 1];
    if (previous !== undefined && previous.label === label) {
      // Repeated `object`/`body` wrappers are one logical step. Keep the
      // deepest target so following the crumb still reaches the useful one.
      items[items.length - 1] = { label, path: path.slice(0, i), here: i === path.length };
    } else {
      items.push({ label, path: path.slice(0, i), here: i === path.length });
    }
  }
  return items;
}

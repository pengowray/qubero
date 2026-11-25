// Which one of several a table is: a GUPPI raw file has a table of samples
// for every channel of every block, all called `samples`, and the bar, the tab
// and the export's file name say which block and which channel this one is. Kept apart from
// `tablebar.ts` so the rule can be read and tested without a document.

import type { Doc, TemplateNode } from "./doc.ts";

/** One list the table sits in: the list's field name (`blocks`), the index of
 *  the element that holds the table (from 0), and how many elements the list has. */
export type TablePlace = { readonly list: string; readonly index: number; readonly of: number };

/**
 * The lists the field at `path` sits in, outermost first, the order they are
 * laid out in the file: `blocks[1]`, then `channels[2]`. Indexed from 0, as
 * the inspector and the table's `#` column are. Only a list whose elements
 * the template gives a name to counts; one of plain items says nothing a
 * reader could use. `nodeAt` answers null for a field that does not read.
 */
export function tablePlaces(path: readonly number[], nodeAt: (path: readonly number[]) => TemplateNode | null): TablePlace[] {
  const out: TablePlace[] = [];
  for (let j = path.length - 1; j >= 1; j--) {
    const index = path[j - 1] as number;
    const list = nodeAt(path.slice(0, j - 1));
    if (list === null || !list.list || list.unit === undefined || list.unit === "") continue;
    out.unshift({ list: list.name, index, of: list.child_count });
  }
  return out;
}

/** The same for a field of `doc`. */
export function docPlaces(doc: Doc, path: readonly number[]): TablePlace[] {
  return tablePlaces(path, (at) => {
    const n = doc.templateNode(at);
    return n.status === "ok" ? n.node : null;
  });
}

// Which one of several a table is: a GUPPI raw file has a table of samples
// for every channel of every block, all called `samples`, and the bar, the tab
// and the export's file name say which block and which channel this one is. Kept apart from
// `tablebar.ts` so the rule can be read and tested without a document.

import type { TemplateNode } from "./doc.ts";

/** One list the table sits in: what one of its elements is called, which of
 *  them holds the table (from 1), and how many there are. */
export type TablePlace = { readonly word: string; readonly n: number; readonly of: number };

/**
 * The lists the field at `path` sits in, outermost first, the order they are
 * laid out in the file: `block 2 of 128`, then `channel 3 of 64`. Numbered
 * from 1, as the table's own columns are. Only a list whose elements
 * the template gives a name to counts; one of plain items says nothing a
 * reader could use. `nodeAt` answers null for a field that does not read.
 */
export function tablePlaces(path: readonly number[], nodeAt: (path: readonly number[]) => TemplateNode | null): TablePlace[] {
  const out: TablePlace[] = [];
  for (let j = path.length - 1; j >= 1; j--) {
    const index = path[j - 1] as number;
    const list = nodeAt(path.slice(0, j - 1));
    if (list === null || !list.list || list.unit === undefined || list.unit === "") continue;
    out.unshift({ word: list.unit, n: index + 1, of: list.child_count });
  }
  return out;
}

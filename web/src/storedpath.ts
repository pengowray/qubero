// A name the core gives twice: the way the format names it, and the path the
// file stores it at. The inspector shows the first, and the second only when
// the reader has asked for paths as stored. Kept apart from `inspector.ts` so
// that the rules can be read and tested without a document.
//
// A Parquet footer is Thrift, which keeps a struct as a list of (id, value)
// entries: `meta_data.data_page_offset` is stored at
// `fields[id = 3].value.fields[7].value`. Most formats have no such steps, and
// for them nothing here ever draws anything.

import { STORED_PATHS } from "./strings.ts";

/** Where the switch is remembered, for every file and every visit. */
export const STORED_PATHS_KEY = "qubero.storedpaths";

/** The muted line under a name or a formula: a word saying what the text is,
 *  and the text itself. */
export type StoredLine = { readonly word: string; readonly text: string };

/**
 * The line under a name, or null for none. None when paths as stored are not
 * shown, and none when the file stores the name the way it is written, which
 * is nearly always.
 *
 * `subject` is the name the line is about, for a row that does not start with
 * that name: `@+0x6 in unpacked blocks[3].compressed` leads with an address,
 * and an offset can be stored at a path too.
 */
export function storedLine(stored: string | null, shown: boolean, subject: string | null = null): StoredLine | null {
  if (!shown || stored === null) return null;
  return { word: subject === null ? STORED_PATHS.storedAs : STORED_PATHS.subjectStoredAs(subject), text: stored };
}

/** The line under a formula written with the short names: the expression as
 *  the template has it, or null for none. */
export function templateLine(template: string | null, shown: boolean): StoredLine | null {
  if (!shown || template === null) return null;
  return { word: STORED_PATHS.inTemplate, text: template };
}

/**
 * Which stored path a property row's clause carries, or null.
 *
 * Once per row. A row with working under it names the same field again there,
 * and that row carries the line, so the clause does not: with the row folded
 * the clause stays short, since the stored path is working. A row with no
 * working has nowhere else to put it.
 */
export function clauseStored(one: { readonly stored: string | null } | null, hasWorking: boolean): string | null {
  return hasWorking || one === null ? null : one.stored;
}

/** Whether anything the panel names is stored another way, which is when the
 *  switch is worth showing: a switch that changes nothing is not offered. */
export function anyStored(
  origins: readonly { readonly stored: string | null }[],
  relations: readonly { readonly template: string | null }[],
): boolean {
  return origins.some((o) => o.stored !== null) || relations.some((r) => r.template !== null);
}

type StorageLike = { getItem(key: string): string | null; setItem(key: string, value: string): void };

/** Whether the reader last asked for paths as stored. Off where storage is
 *  missing or refuses, which is the panel as it reads with no switch at all. */
export function readShown(storage: StorageLike | null): boolean {
  try {
    return storage?.getItem(STORED_PATHS_KEY) === "1";
  } catch {
    return false;
  }
}

/** Remember the switch. A storage that refuses forgets it at the next visit,
 *  and nothing else goes wrong. */
export function writeShown(storage: StorageLike | null, shown: boolean): void {
  try {
    storage?.setItem(STORED_PATHS_KEY, shown ? "1" : "0");
  } catch {
    // Nothing to do: the switch still holds for this visit.
  }
}

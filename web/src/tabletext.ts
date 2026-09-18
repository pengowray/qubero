// The table as text: what each line of it says, cell by cell.
//
// The view draws a table and a copy of it writes one, and the two have to
// agree on what the columns are and what order they come in, or a reader
// pastes something other than what they were looking at. So the order is
// written down once, here, with no DOM in it: a line is the row's number, its
// name where the rows have names, its time where they are spaced in time, its
// values, and where it is stored when the reader has asked for that.
//
// That is the table the way it means: a record to a line. A table drawn turned
// is the same lines read the other way, column for column, and `turned` is
// all that takes. Nothing about a turned table is worked out separately, which
// is what keeps the two from drifting apart.

import { formatOffset } from "./format.ts";
import { bitSizeText, TABLE } from "./strings.ts";
import { timeText, type TableRow } from "./tableplan.ts";

/** Which columns go around the values: the ones that are about the row rather
 *  than in it. The row's number is always there. */
export type Lead = {
  /** The rows have names of their own. */
  readonly named: boolean;
  /** Rows a second, or null for rows that are not spaced in time. */
  readonly rate: number | null;
  /** The reader has asked where each row is stored. */
  readonly addresses: boolean;
};

/** Some of the value columns, half-open. A turned table's selection is a run
 *  of these rather than a run of records. */
export type FieldRange = { readonly from: number; readonly to: number };

/** The heading line. */
export function headerCells(headings: readonly string[], lead: Lead, fields?: FieldRange): string[] {
  const out: string[] = [TABLE.index];
  if (lead.named) out.push(TABLE.rowName);
  if (lead.rate !== null) out.push(TABLE.time);
  out.push(...(fields === undefined ? headings : headings.slice(fields.from, fields.to)));
  if (lead.addresses) out.push(TABLE.storedAt, TABLE.size);
  return out;
}

/** The line for record `i`. `width` is how many value columns the table has,
 *  so a record that came back short still fills its line. */
export function recordCells(i: number, row: TableRow, width: number, lead: Lead, fields?: FieldRange): string[] {
  const out: string[] = [String(i)];
  if (lead.named) out.push(row.name ?? "");
  if (lead.rate !== null) out.push(timeText(i, lead.rate));
  const from = fields?.from ?? 0;
  const to = fields?.to ?? width;
  for (let c = from; c < to; c++) out.push(row.cells[c]?.text ?? "");
  if (lead.addresses) out.push(formatOffset(row.offsetBits), bitSizeText(row.sizeBits));
  return out;
}

/** How many columns come before the values. */
export function leadBefore(lead: Lead): number {
  return 1 + (lead.named ? 1 : 0) + (lead.rate !== null ? 1 : 0);
}

/** The same lines with rows and columns changed over: line `j` of the answer
 *  is cell `j` of every line given. The lines are taken to be as long as the
 *  first of them. */
export function turned(lines: readonly (readonly string[])[]): string[][] {
  const width = lines[0]?.length ?? 0;
  return Array.from({ length: width }, (_, j) => lines.map((line) => line[j] ?? ""));
}

/**
 * The lines of a table drawn turned, each one a label and then a cell for
 * every record.
 *
 * The lines about the records come first, all of them: number, name, time,
 * and where each is stored. In a table the right way up the addresses are the
 * last columns; turned, they sit with the other headings at the top, where
 * they stay in view, so that is the order they are written in. Then a line
 * for each of `fields`.
 */
export function turnedLines(headings: readonly string[], records: readonly TableRow[], lead: Lead, fields: FieldRange): string[][] {
  const width = headings.length;
  const all = turned([headerCells(headings, lead), ...records.map((row, i) => recordCells(i, row, width, lead))]);
  const before = leadBefore(lead);
  return [...all.slice(0, before), ...all.slice(before + width), ...all.slice(before + fields.from, before + fields.to)];
}

/** What each of those first lines is, in the order `turnedLines` writes them,
 *  so the view can colour a line of addresses as addresses. */
export function leadKinds(lead: Lead): ("index" | "name" | "time" | "at" | "size")[] {
  const out: ("index" | "name" | "time" | "at" | "size")[] = ["index"];
  if (lead.named) out.push("name");
  if (lead.rate !== null) out.push("time");
  if (lead.addresses) out.push("at", "size");
  return out;
}

/** One line of tab-separated text. A tab or a line break inside a value would
 *  be read as the end of the cell or of the line, so they are written the way
 *  a string literal writes them; a backslash is doubled so the two cannot be
 *  confused coming back. */
export function tsvLine(cells: readonly string[]): string {
  return cells.map((cell) => (/[\t\n\r\\]/.test(cell) ? cell.replace(/\\/g, "\\\\").replace(/\t/g, "\\t").replace(/\n/g, "\\n").replace(/\r/g, "\\r") : cell)).join("\t");
}

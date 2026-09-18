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
import { isNumberKind, timeText, type TableRow } from "./tableplan.ts";

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
  for (const heading of fields === undefined ? headings : headings.slice(fields.from, fields.to)) out.push(heading);
  if (lead.addresses) out.push(TABLE.storedAt, TABLE.size);
  return out;
}

/** One cell of a line: what it says, and whether what it says is a number.
 *  Text formats write the two alike; JSON does not. */
export type Part = { readonly text: string; readonly numeric: boolean };

/** The line for record `i`. `width` is how many value columns the table has,
 *  so a record that came back short still fills its line. */
export function recordParts(i: number, row: TableRow, width: number, lead: Lead, fields?: FieldRange): Part[] {
  const out: Part[] = [{ text: String(i), numeric: true }];
  if (lead.named) out.push({ text: row.name ?? "", numeric: false });
  if (lead.rate !== null) out.push({ text: timeText(i, lead.rate), numeric: true });
  const from = fields?.from ?? 0;
  const to = fields?.to ?? width;
  for (let c = from; c < to; c++) {
    const cell = row.cells[c];
    out.push({ text: cell?.text ?? "", numeric: cell !== undefined && isNumberKind(cell.kind) });
  }
  if (lead.addresses) out.push({ text: formatOffset(row.offsetBits), numeric: false }, { text: bitSizeText(row.sizeBits), numeric: false });
  return out;
}

/** The same line as text alone. */
export function recordCells(i: number, row: TableRow, width: number, lead: Lead, fields?: FieldRange): string[] {
  return recordParts(i, row, width, lead, fields).map((part) => part.text);
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

/** One line of comma-separated text, as RFC 4180 has it: a value holding a
 *  comma, a quote or a line break goes in quotes, with its own quotes doubled.
 *  So does one that starts or ends with a space, which some readers would
 *  otherwise trim. */
export function csvLine(cells: readonly string[]): string {
  return cells.map((cell) => (/[",\r\n]|^\s|\s$/.test(cell) ? `"${cell.replace(/"/g, '""')}"` : cell)).join(",");
}

/** A number the way JSON writes one. Anything else a numeric cell can say
 *  (`NaN`, `1,204`, `0x1f`) is not one, and goes out as a string. */
const JSON_NUMBER = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?$/;

/**
 * One line as a JSON object, keyed by the headings.
 *
 * A numeric cell is written as the number it shows, digit for digit: the text
 * is put in the file as it is rather than going through a double, so a 64-bit
 * count past 2^53 comes out whole.
 */
export function jsonObject(keys: readonly string[], parts: readonly Part[]): string {
  const members = parts.map((part, at) => {
    const value = part.numeric && JSON_NUMBER.test(part.text) ? part.text : JSON.stringify(part.text);
    return `${JSON.stringify(keys[at] ?? "")}: ${value}`;
  });
  return `{${members.join(", ")}}`;
}

/** Headings made fit to be keys: a second column of the same name would
 *  overwrite the first in an object, so it is numbered, `TOTAL (2)`. */
export function uniqueKeys(headings: readonly string[]): string[] {
  const seen = new Map<string, number>();
  return headings.map((heading) => {
    const n = (seen.get(heading) ?? 0) + 1;
    seen.set(heading, n);
    return n === 1 ? heading : `${heading} (${n})`;
  });
}

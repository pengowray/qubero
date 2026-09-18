// A table written out as a file: CSV, TSV or JSON.
//
// The file is the table the way it means, a record to a row, whichever way
// round the view happens to be drawing it. That is what a script or a
// spreadsheet import expects of a table, and it is what keeps an export the
// same file tomorrow when the reader has flipped the view in between. A reader
// who wants the file the way the screen has it can ask, for the two formats
// where that means something; JSON has no way round, only objects.
//
// No DOM here. The text comes out in pieces, a few thousand rows at a time,
// so whoever is asking can let the page breathe between them, say how far it
// has got, and stop. A run of samples is millions of rows, and a loop that
// built all of it before giving anything back would hold the tab still for as
// long as it took. `tableexportpanel.ts` is the half with the form in it.

import type { TableRow } from "./tableplan.ts";
import { csvLine, headerCells, jsonObject, leadKinds, recordCells, recordParts, tsvLine, turnedLines, uniqueKeys, type FieldRange, type Lead } from "./tabletext.ts";

export const FORMATS = ["csv", "tsv", "json"] as const;
export type ExportFormat = (typeof FORMATS)[number];

/** What each format is called, its file extension, and what a browser should
 *  be told the file is. */
export const FORMAT_FACTS: Record<ExportFormat, { readonly label: string; readonly mime: string }> = {
  csv: { label: "CSV", mime: "text/csv" },
  tsv: { label: "TSV", mime: "text/tab-separated-values" },
  json: { label: "JSON", mime: "application/json" },
};

export type ExportJob = {
  readonly format: ExportFormat;
  /** The value columns' headings, all of them. */
  readonly headings: readonly string[];
  readonly lead: Lead;
  /** How many records the table has. */
  readonly count: number;
  /** Which records to write, half-open. */
  readonly records: { readonly from: number; readonly to: number };
  /** Which value columns to write; all of them when absent. */
  readonly fields?: FieldRange;
  /** Write it turned, a record to a column. Not for JSON. */
  readonly asShown: boolean;
};

/** A piece of the file, and how many rows of the table have gone into the
 *  file so far. */
export type ExportPiece = { readonly text: string; readonly done: number };

/** How many rows go into one piece. */
const PIECE = 1000;

/** What a selection in the view is a selection of, as an export sees it. The
 *  right way up the rows on screen are records; turned, they are fields. */
export function scopeOf(turned: boolean, count: number, selected: { readonly from: number; readonly to: number } | null): Pick<ExportJob, "records" | "fields"> {
  if (selected === null) return { records: { from: 0, to: count } };
  return turned ? { records: { from: 0, to: count }, fields: selected } : { records: selected };
}

/** Whether a job is written turned: asked for, and in a format that can be. */
export function writtenTurned(job: Pick<ExportJob, "format" | "asShown">): boolean {
  return job.asShown && job.format !== "json";
}

/** How many rows and columns the file will have, heading row not counted. For
 *  JSON the rows are objects and the columns their keys. */
export function exportSize(job: ExportJob): { readonly rows: number; readonly columns: number } {
  const fields = job.fields === undefined ? job.headings.length : job.fields.to - job.fields.from;
  const records = job.records.to - job.records.from;
  const about = leadKinds(job.lead).length;
  if (writtenTurned(job)) return { rows: about - 1 + fields, columns: 1 + job.count };
  return { rows: records, columns: about + fields };
}

/**
 * The file, a piece at a time.
 *
 * `row` answers with a record once it has been read, however long that takes,
 * and throws when it cannot be. Everything is written in the one order
 * `tabletext.ts` keeps, so a file has the columns the screen has.
 */
export async function* exportText(job: ExportJob, row: (i: number) => Promise<TableRow>): AsyncGenerator<ExportPiece> {
  const width = job.headings.length;
  if (writtenTurned(job)) {
    // Turned, every line has a cell from every record, so they are all read
    // before the first line can be written. Only a table short enough to be
    // drawn turned is ever asked for this way.
    const records: TableRow[] = [];
    for (let i = 0; i < job.count; i++) records.push(await row(i));
    const lines = turnedLines(job.headings, records, job.lead, job.fields ?? { from: 0, to: width });
    yield { text: lines.map((line) => lineOf(job.format, line)).join(""), done: lines.length - 1 };
    return;
  }
  const head = headerCells(job.headings, job.lead, job.fields);
  const keys = uniqueKeys(head);
  let text = job.format === "json" ? "[\n" : lineOf(job.format, head);
  for (let i = job.records.from; i < job.records.to; i++) {
    const record = await row(i);
    if (job.format === "json") text += `  ${jsonObject(keys, recordParts(i, record, width, job.lead, job.fields))}${i + 1 < job.records.to ? "," : ""}\n`;
    else text += lineOf(job.format, recordCells(i, record, width, job.lead, job.fields));
    const done = i + 1 - job.records.from;
    if (done % PIECE === 0) {
      yield { text, done };
      text = "";
    }
  }
  if (job.format === "json") text += "]\n";
  yield { text, done: job.records.to - job.records.from };
}

/** One line with its ending. CSV ends its lines the way RFC 4180 says. */
function lineOf(format: ExportFormat, cells: readonly string[]): string {
  return format === "csv" ? `${csvLine(cells)}\r\n` : `${tsvLine(cells)}\n`;
}

/** A name for the file: the file the table came from, what the table is of,
 *  and the format, with whatever a file name cannot hold taken out. */
export function exportName(file: string, table: string, format: ExportFormat): string {
  const clean = (s: string): string => s.replace(/[\\/:*?"<>|\x00-\x1f]+/g, "_").trim();
  return `${clean(file)}.${clean(table)}.${format}`;
}

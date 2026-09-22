// An export is read by programs, which forgive nothing: a comma inside a
// value splits a column, a number written as a string sorts wrong, and a file
// that comes out turned because the screen was is a different file from the
// one the same table gave yesterday.

import { test } from "node:test";
import assert from "node:assert/strict";

import { exportName, exportSize, exportText, scopeOf, type ExportJob } from "../src/tableexport.ts";
import type { TableRow } from "../src/tableplan.ts";
import { csvLine, jsonObject, uniqueKeys, type Lead } from "../src/tabletext.ts";

const PLAIN: Lead = { named: false, rate: null, addresses: false };

function row(cells: readonly (readonly [string, string])[], o: Partial<TableRow> = {}): TableRow {
  return { cells: cells.map(([text, kind]) => ({ text, kind })), offsetBits: 0, sizeBits: 0, path: [], ...o };
}

const RECORDS: readonly TableRow[] = [
  row([["Alpha", "string"], ["12.50", "float"]]),
  row([["Café, large", "string"], ["3", "uint"]]),
];

function job(o: Partial<ExportJob>): ExportJob {
  return { format: "csv", headings: ["ITEM", "TOTAL"], lead: PLAIN, count: RECORDS.length, records: { from: 0, to: RECORDS.length }, asShown: false, ...o };
}

async function written(j: ExportJob, records: readonly TableRow[] = RECORDS): Promise<string> {
  let text = "";
  for await (const piece of exportText(j, async (i) => records[i] ?? assert.fail(`no record ${i}`))) text += piece.text;
  return text;
}

test("CSV quotes what would otherwise split a cell", () => {
  assert.equal(csvLine(["a", "b"]), "a,b");
  assert.equal(csvLine(["a,b", 'say "hi"', "two\nlines", " padded"]), '"a,b","say ""hi""","two\nlines"," padded"');
});

test("a CSV file is a heading row and a row a record", async () => {
  assert.equal(await written(job({})), '#,ITEM,TOTAL\r\n0,Alpha,12.50\r\n1,"Café, large",3\r\n');
});

test("a TSV file is the same rows with tabs", async () => {
  assert.equal(await written(job({ format: "tsv" })), "#\tITEM\tTOTAL\n0\tAlpha\t12.50\n1\tCafé, large\t3\n");
});

test("JSON writes numbers as numbers and leaves the rest as strings", async () => {
  const text = await written(job({ format: "json" }));
  assert.deepEqual(JSON.parse(text), [
    { "#": 0, ITEM: "Alpha", TOTAL: 12.5 },
    { "#": 1, ITEM: "Café, large", TOTAL: 3 },
  ]);
});

test("a number JSON cannot write goes out as a string, and a big one keeps its digits", () => {
  const parts = [
    { text: "NaN", numeric: true },
    { text: "18446744073709551615", numeric: true },
    { text: "007", numeric: true },
    { text: "12", numeric: false },
  ];
  assert.equal(jsonObject(["a", "b", "c", "d"], parts), '{"a": "NaN", "b": 18446744073709551615, "c": "007", "d": "12"}');
});

test("two columns of one name do not overwrite each other", () => {
  assert.deepEqual(uniqueKeys(["x", "y", "x", "x"]), ["x", "y", "x (2)", "x (3)"]);
});

test("some of the records can be written, numbered as they are in the table", async () => {
  assert.equal(await written(job({ records: { from: 1, to: 2 } })), '#,ITEM,TOTAL\r\n1,"Café, large",3\r\n');
});

test("a file is a record to a row however the table is drawn, unless asked", async () => {
  const strip = [row([["1", "float"], ["2", "float"], ["3", "float"]]), row([["4", "float"], ["5", "float"], ["6", "float"]])];
  const base = job({ headings: ["0", "1", "2"], count: 2, records: { from: 0, to: 2 } });
  assert.equal(await written(base, strip), "#,0,1,2\r\n0,1,2,3\r\n1,4,5,6\r\n");
  assert.equal(await written({ ...base, asShown: true }, strip), "#,0,1\r\n0,1,4\r\n1,2,5\r\n2,3,6\r\n");
  // JSON has no way round.
  assert.equal(await written({ ...base, format: "json", asShown: true }, strip), await written({ ...base, format: "json" }, strip));
});

test("a selection in a turned table is a selection of columns", () => {
  assert.deepEqual(scopeOf(false, 10, { from: 2, to: 5 }), { records: { from: 2, to: 5 } });
  assert.deepEqual(scopeOf(true, 10, { from: 2, to: 5 }), { records: { from: 0, to: 10 }, fields: { from: 2, to: 5 } });
  assert.deepEqual(scopeOf(true, 10, null), { records: { from: 0, to: 10 } });
});

test("the size said beforehand is the size written", async () => {
  const strip = [row([["1", "float"], ["2", "float"], ["3", "float"]]), row([["4", "float"], ["5", "float"], ["6", "float"]])];
  for (const asShown of [false, true]) {
    const j = job({ headings: ["0", "1", "2"], count: 2, records: { from: 0, to: 2 }, fields: { from: 1, to: 3 }, asShown });
    const lines = (await written(j, strip)).trimEnd().split("\r\n");
    assert.deepEqual({ rows: lines.length - 1, columns: lines[0]?.split(",").length }, exportSize(j));
  }
});

test("a long table comes out in pieces, and the pieces are the whole of it", async () => {
  const many = Array.from({ length: 4500 }, (_, i) => row([[String(i), "uint"]]));
  const j = job({ headings: ["v"], count: many.length, records: { from: 0, to: many.length } });
  const done: number[] = [];
  let text = "";
  for await (const piece of exportText(j, async (i) => many[i] ?? assert.fail())) {
    done.push(piece.done);
    text += piece.text;
  }
  assert.deepEqual(done, [1000, 2000, 3000, 4000, 4500]);
  assert.equal(text.split("\r\n").length, 4502);
});

test("the file is named for where the table came from", () => {
  assert.equal(exportName("big_endian.tdms", "values", "csv"), "big_endian.tdms.values.csv");
  assert.equal(exportName("a.bin", "/'Measured Data'/'Phase'", "json"), "a.bin._'Measured Data'_'Phase'.json");
});

test("a file is the values, with no address columns however the table is shown", async () => {
  // The reader has the address columns on screen. A file of the table is its
  // values, so the columns that say where each row is stored are left out,
  // and the size line agrees with what is written.
  const shown = job({ lead: { named: false, rate: null, addresses: true } });
  assert.equal(await written(shown), await written(job({})));
  assert.deepEqual(exportSize(shown), exportSize(job({})));
  assert.equal(await written(job({ ...shown, format: "tsv", asShown: true })), await written(job({ format: "tsv", asShown: true })));
});

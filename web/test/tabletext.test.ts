// A copy of a table has to say what the screen says, in the order the screen
// says it, whichever way round the table is drawn. These pin the order down,
// and the one rule that makes a turned table trustworthy: it is the same
// cells as the table the right way up, read the other way.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { TableRow } from "../src/tableplan.ts";
import { headerCells, leadBefore, leadKinds, recordCells, tsvLine, turned, turnedLines, type Lead } from "../src/tabletext.ts";

const PLAIN: Lead = { named: false, rate: null, addresses: false };

function row(texts: readonly string[], o: Partial<TableRow> = {}): TableRow {
  return { cells: texts.map((text) => ({ text, kind: "float" })), offsetBits: 0, sizeBits: 0, path: [], ...o };
}

test("a line is the row's number and then its values", () => {
  assert.deepEqual(headerCells(["left", "right"], PLAIN), ["#", "left", "right"]);
  assert.deepEqual(recordCells(7, row(["0.5", "0.25"]), 2, PLAIN), ["7", "0.5", "0.25"]);
});

test("name, time and addresses go around the values in a fixed order", () => {
  const lead: Lead = { named: true, rate: 10, addresses: true };
  assert.deepEqual(headerCells(["v"], lead), ["#", "name", "time (s)", "v", "Stored at", "Size"]);
  const cells = recordCells(5, row(["9"], { name: "Phase", offsetBits: 0x80, sizeBits: 64 }), 1, lead);
  assert.equal(cells.length, 6);
  assert.deepEqual(cells.slice(0, 4), ["5", "Phase", (0.5).toLocaleString(undefined, { minimumFractionDigits: 2 }), "9"]);
  assert.equal(leadBefore(lead), 3);
  assert.deepEqual(leadKinds(lead), ["index", "name", "time", "at", "size"]);
});

test("a record that came back short still fills its line", () => {
  assert.deepEqual(recordCells(0, row(["1"]), 3, PLAIN), ["0", "1", "", ""]);
});

test("some of the columns can be asked for", () => {
  assert.deepEqual(headerCells(["a", "b", "c"], PLAIN, { from: 1, to: 3 }), ["#", "b", "c"]);
  assert.deepEqual(recordCells(0, row(["1", "2", "3"]), 3, PLAIN, { from: 1, to: 3 }), ["0", "2", "3"]);
});

test("turning changes rows for columns and turning again changes them back", () => {
  const lines = [
    ["#", "a", "b"],
    ["0", "1", "2"],
  ];
  assert.deepEqual(turned(lines), [
    ["#", "0"],
    ["a", "1"],
    ["b", "2"],
  ]);
  assert.deepEqual(turned(turned(lines)), lines);
});

test("a turned table is headings about the records, then a line a field", () => {
  const records = [row(["1", "2", "3"], { name: "Amplitude" }), row(["4", "5", "6"], { name: "Phase" })];
  const lead: Lead = { named: true, rate: null, addresses: false };
  assert.deepEqual(turnedLines(["[0]", "[1]", "[2]"], records, lead, { from: 1, to: 3 }), [
    ["#", "0", "1"],
    ["name", "Amplitude", "Phase"],
    ["[1]", "2", "5"],
    ["[2]", "3", "6"],
  ]);
});

test("turned, where a record is stored is a heading line and not the last one", () => {
  const records = [row(["1"], { offsetBits: 0, sizeBits: 8 })];
  const lines = turnedLines(["v"], records, { named: false, rate: null, addresses: true }, { from: 0, to: 1 });
  assert.deepEqual(
    lines.map((line) => line[0]),
    ["#", "Stored at", "Size", "v"],
  );
});

test("a tab or a line break in a value does not end the cell", () => {
  assert.equal(tsvLine(["a", "b"]), "a\tb");
  assert.equal(tsvLine(["a\tb", "c\nd"]), "a\\tb\tc\\nd");
  assert.equal(tsvLine(["C:\\temp"]), "C:\\\\temp");
});

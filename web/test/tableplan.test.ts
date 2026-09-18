// A table's columns come from whatever the file happened to say, so the parts
// that name them and count them are the parts worth pinning down: a column
// named wrong puts one field's values under another field's heading, and a
// time column with too few decimals prints the same instant for forty-four
// thousand rows.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { TableShape, TemplateNode } from "../src/doc.ts";
import {
  cellOf,
  columnNameOf,
  FIT_MAX,
  FIT_MIN,
  fitCell,
  fitOf,
  flatFields,
  flatNames,
  indexWidth,
  rowCount,
  rowRange,
  shapeColumns,
  timeDigits,
  timeText,
  timeWidth,
  uniformColumns,
} from "../src/tableplan.ts";
import { PROBLEMS } from "../src/strings.ts";

/** A node as the core sends one, with the few fields this file reads set and
 *  the rest at something harmless. */
function node(o: Partial<TemplateNode> & { name: string }): TemplateNode {
  return {
    path: [],
    type: "u8",
    offset_bits: 0,
    size_bits: 8,
    value: "",
    edit_text: "",
    kind: "uint",
    problems_within: [0, 0],
    child_count: 0,
    composite: false,
    list: false,
    editable: false,
    inline: false,
    value_bytes: 1,
    value_offset_bits: 0,
    read_at: null,
    read_as: null,
    consumed_by: null,
    machinery: null,
    contents: false,
    framed: false,
    space: 0,
    refused: null,
    decoded: false,
    space_root: false,
    joined: false,
    absent: false,
    line: null,
    ...o,
  };
}

function shape(o: Partial<TableShape> = {}): TableShape {
  return { columns: null, names: [], units: [], column_word: null, row_word: null, rate: null, facts: [], ...o };
}

test("elements are grouped into rows by the column count", () => {
  assert.deepEqual(rowRange(0, 2, 10), { from: 0, to: 2 });
  assert.deepEqual(rowRange(3, 2, 10), { from: 6, to: 8 });
  assert.equal(rowCount(10, 2), 5);
  assert.equal(rowCount(10, 1), 10);
});

test("a list that does not divide evenly ends in a short row rather than past its end", () => {
  assert.equal(rowCount(7, 2), 4);
  assert.deepEqual(rowRange(3, 2, 7), { from: 6, to: 7 });
});

test("names are used when there are as many of them as columns", () => {
  const s = shape({ names: ["left", "right"], units: ["", ""], column_word: "channel" });
  assert.deepEqual(shapeColumns(s, 2), [
    { name: "left", unit: "" },
    { name: "right", unit: "" },
  ]);
});

test("names that do not fit the row are dropped for numbers", () => {
  // Two names and six channels means the file disagrees with the template.
  // Numbering says nothing untrue; `left` over channel 3 would.
  const s = shape({ names: ["left", "right"], column_word: "channel" });
  assert.deepEqual(
    shapeColumns(s, 3).map((c) => c.name),
    ["channel 1", "channel 2", "channel 3"],
  );
});

test("a shape with no word for a column still names its columns", () => {
  assert.deepEqual(
    shapeColumns(shape(), 2).map((c) => c.name),
    ["column 1", "column 2"],
  );
});

test("units ride along with the names, and only when they fit", () => {
  assert.deepEqual(shapeColumns(shape({ names: ["f"], units: ["Hz"] }), 1), [{ name: "f", unit: "Hz" }]);
  assert.deepEqual(shapeColumns(shape({ names: ["f"], units: ["Hz", "dB"] }), 1), [{ name: "f", unit: "" }]);
});

test("an element named by the file keeps the name and drops the index", () => {
  assert.equal(columnNameOf("[0] NAME"), "NAME");
  assert.equal(columnNameOf("[12] BIRTH DATE"), "BIRTH DATE");
  // Nothing but an index is still what the column has to be called.
  assert.equal(columnNameOf("[3]"), "[3]");
  assert.equal(columnNameOf("deleted"), "deleted");
});

test("a nested list is expanded one level into the row", () => {
  const values = node({ name: "values", list: true, composite: true, child_count: 2 });
  const record = [node({ name: "deleted" }), values];
  const inner = [node({ name: "[0] NAME" }), node({ name: "[1] AGE" })];
  const flat = flatFields(record, (child) => (child === values ? inner : null));
  assert.deepEqual(flatNames(flat ?? []), ["deleted", "NAME", "AGE"]);
});

test("a nested list still being read makes the whole row wait", () => {
  const values = node({ name: "values", list: true, composite: true, child_count: 2 });
  assert.equal(flatFields([values], () => null), null);
});

test("an empty nested list is a cell rather than nothing", () => {
  const values = node({ name: "values", list: true, composite: true, child_count: 0 });
  const flat = flatFields([node({ name: "deleted" }), values], () => null);
  assert.deepEqual(flatNames(flat ?? []), ["deleted", "values"]);
});

test("the same names in the same order are a table", () => {
  assert.deepEqual(uniformColumns([["a", "b"], ["a", "b"]]), ["a", "b"]);
});

test("elements that disagree are not a table", () => {
  assert.equal(uniformColumns([["a", "b"], ["a", "c"]]), null);
  assert.equal(uniformColumns([["a", "b"], ["a", "b", "c"]]), null);
});

test("one column is a list, and one element is not enough to see a pattern in", () => {
  assert.equal(uniformColumns([["a"], ["a"]]), null);
  assert.equal(uniformColumns([["a", "b"]]), null);
  assert.equal(uniformColumns([]), null);
});

test("the time column carries enough decimals for neighbouring rows to differ", () => {
  assert.equal(timeDigits(44100), 6);
  assert.equal(timeDigits(8000), 5);
  assert.equal(timeDigits(1), 1);
  // Past nine the digits say more than the number they came from knows.
  assert.equal(timeDigits(1e12), 9);
  assert.notEqual(timeText(1, 44100), timeText(2, 44100));
});

test("a rate of nothing asks for no decimals rather than for infinitely many", () => {
  assert.equal(timeDigits(0), 0);
  assert.equal(timeDigits(-1), 0);
});

test("a cell says the value, and a field of fields says how many", () => {
  assert.deepEqual(cellOf(node({ name: "age", value: "37" })), { text: "37", kind: "uint" });
  assert.deepEqual(
    cellOf(node({ name: "values", composite: true, list: true, child_count: 4, kind: "composite" })),
    { text: "4 items", kind: "composite" },
  );
});

test("a cell carries what is wrong with its value, so the table can mark it", () => {
  const bad = cellOf(node({ name: "sample", value: "NaN", kind: "float", problem: { tier: "undefined", text: "Not a number (quiet NaN)" } }));
  assert.deepEqual(bad, { text: "NaN", kind: "float", problem: { tier: "undefined", text: "Not a number (quiet NaN)" } });
  // A field with nothing wrong carries no key at all, which is what the rest
  // of the view tests for.
  assert.equal("problem" in cellOf(node({ name: "age", value: "37" })), false);
});

test("a column heading counts both tiers, and says so far while rows are unread", () => {
  assert.equal(PROBLEMS.column(2, 0, false), "\u00b7 2 invalid");
  assert.equal(PROBLEMS.column(0, 5, false), "\u00b7 5 undefined");
  assert.equal(PROBLEMS.column(2, 5, false), "\u00b7 2 invalid, 5 undefined");
  assert.equal(PROBLEMS.column(2, 0, true), "\u00b7 2 invalid so far");
  // Nothing wrong is nothing on the heading, not a zero.
  assert.equal(PROBLEMS.column(0, 0, true), "");
  assert.equal(PROBLEMS.column(1234, 0, false), "\u00b7 1,234 invalid");
});

test("a column starts as wide as its heading and only ever widens, up to the cap", () => {
  let fit = fitOf("f");
  assert.equal(fit.width, FIT_MIN);
  fit = fitCell(fit, { text: "12345678", kind: "int" });
  assert.equal(fit.width, 8);
  // A shorter value leaves the column where it is: a settled table does not
  // shift under a scroll.
  assert.equal(fitCell(fit, { text: "1", kind: "int" }).width, 8);
  assert.equal(fitCell(fit, { text: "x".repeat(200), kind: "str" }).width, FIT_MAX);
  assert.equal(fitOf("a heading longer than any cap allows for a column").width, FIT_MAX);
});

test("the side a column sits on is decided by its first value and kept", () => {
  const blank = fitOf("v");
  assert.equal(blank.numeric, null);
  const numbers = fitCell(blank, { text: "-3708", kind: "int" });
  assert.equal(numbers.numeric, true);
  assert.equal(fitCell(numbers, { text: "abc", kind: "str" }).numeric, true);
  const text = fitCell(blank, { text: "NAME", kind: "str" });
  assert.equal(text.numeric, false);
  assert.equal(fitCell(text, { text: "7", kind: "uint" }).numeric, false);
  // Nothing changed, so the same fit comes back and the caller can tell.
  assert.equal(fitCell(numbers, { text: "1", kind: "int" }), numbers);
  assert.equal(fitCell(blank, undefined), blank);
});

test("the row number and time columns are as wide as their last row", () => {
  assert.equal(indexWidth(400), 3);
  assert.equal(indexWidth(26_000_000), "25,999,999".length);
  assert.equal(indexWidth(0), 3);
  // `time (s)` is the floor: at 8 kHz the last of 400 rows is `0.04988`.
  assert.equal(timeWidth(400, 8000), "time (s)".length);
  assert.equal(timeWidth(26_000_000, 44100), timeText(25_999_999, 44100).length);
});

// The BUFR panel's wording, from the numbers the core hands over.
//
// Every number comes from the core, so what is left to get wrong here is which
// sentence a value gets: a measurement, a code table, text, a missing value,
// and each of those compressed or not, with a difference width of zero or
// more. And which label a value that is about another element carries.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { BufrCursor, BufrInfo, BufrValue } from "../src/doc.ts";
import { aboutText, bufrNote, code6, cursorLines, showingLine, subsetHead, tablesLine, valueLabel, valueText } from "../src/bufrpanel.ts";

function value(over: Partial<BufrValue> = {}): BufrValue {
  return { code: 12101, role: "element", name: "Temperature/air temperature", text: "272.1", unit: "K", missing: false, about: "", ...over };
}

function cursor(over: Partial<BufrCursor> = {}): BufrCursor {
  return {
    index: 3,
    value: value(),
    bit: 194,
    width: 16,
    scale: 2,
    reference: 0,
    numeric: true,
    packed: 27210,
    base: null,
    increment_width: null,
    across: [],
    ...over,
  };
}

function bufr(over: Partial<BufrInfo> = {}): BufrInfo {
  return {
    kind: "bufr",
    problem: "",
    edition: 3,
    master_table_version: 13,
    tables_version: 13,
    subsets: 1,
    compressed: false,
    steps: [],
    descriptors: [],
    descriptors_total: 0,
    subset: 0,
    values: [value()],
    values_start: 0,
    values_total: 1,
    cursor: null,
    ...over,
  };
}

test("the note counts subsets and says when they are compressed", () => {
  assert.equal(bufrNote(bufr()), "1 subset");
  assert.equal(bufrNote(bufr({ subsets: 128, compressed: true })), "128 subsets, compressed");
  // A message whose header would not read has nothing to count.
  assert.equal(bufrNote(bufr({ edition: 0 })), "");
});

test("the tables line names the version used where it is not the one section 1 names", () => {
  assert.equal(tablesLine(bufr()), "Edition 3, WMO tables version 13");
  assert.equal(tablesLine(bufr({ edition: 4, master_table_version: 20, tables_version: 46 })), "Edition 4, WMO tables version 20 (not bundled: read with version 46 instead)");
  // A message whose header would not read has no edition to name.
  assert.equal(tablesLine(bufr({ edition: 0 })), null);
});

test("a value reads with its unit, and a missing one says so", () => {
  assert.equal(valueText(value()), "272.1 K");
  assert.equal(valueText(value({ unit: "", text: "20" })), "20");
  assert.equal(valueText(value({ missing: true, text: "" })), "missing");
  assert.equal(code6(1001), "001001");
});

test("a value about another element is named by what it is, with the element beside it", () => {
  const quality = value({ code: 33007, role: "quality", name: "Per cent confidence", about: "001001 WMO block number" });
  assert.equal(valueLabel(quality), "033007 Per cent confidence");
  assert.equal(aboutText(quality), "for 001001 WMO block number");
  const associated = value({ role: "associated", name: "Associated field", about: "001001 WMO block number" });
  assert.equal(valueLabel(associated), "Associated field");
  assert.equal(aboutText(associated), "of 001001 WMO block number");
  const marker = value({ code: 225255, role: "marker", name: "Difference statistic", about: "010004 Pressure" });
  assert.equal(valueLabel(marker), "225255 Difference statistic");
  assert.equal(aboutText(marker), "for 010004 Pressure");
  assert.equal(aboutText(value()), null);
});

test("the subset subhead names the subset only when there are several", () => {
  assert.equal(subsetHead(bufr()), "Values");
  assert.equal(subsetHead(bufr({ subsets: 7, subset: 2 })), "Values of subset 3 of 7");
});

test("a cut list says which values it shows", () => {
  assert.equal(showingLine(bufr()), null);
  const values = Array.from({ length: 1000 }, () => value());
  assert.equal(showingLine(bufr({ values, values_start: 531, values_total: 1531 })), "Showing values 532 to 1,531 of 1,531.");
});

test("an uncompressed measurement shows its arithmetic and where its bits are", () => {
  assert.deepEqual(cursorLines(cursor()), ["(27210 + 0) ÷ 10^2 = 272.1, from 16 bits at bit 194 of section 4."]);
  // A negative reference and no scale.
  const pressure = cursor({ scale: 0, reference: -2048, packed: 2100, value: value({ text: "52" }) });
  assert.deepEqual(cursorLines(pressure), ["(2100 − 2048) = 52, from 16 bits at bit 194 of section 4."]);
  // Missing, and a code table, whose number is the value as written.
  assert.deepEqual(cursorLines(cursor({ packed: null, value: value({ missing: true, text: "" }) })), [
    "16 bits at bit 194 of section 4: all ones, which means missing.",
  ]);
  assert.deepEqual(cursorLines(cursor({ numeric: false, width: 6, packed: 20 })), ["6 bits at bit 194 of section 4."]);
});

test("a compressed value says its smallest packed number and this subset's difference", () => {
  const c = cursor({ base: 27000, increment_width: 8, packed: 27210 });
  assert.deepEqual(cursorLines(c), [
    "Compressed, from bit 194 of section 4: the smallest packed number 27000 in 16 bits, a 6-bit width, then a difference of 8 bits for each subset; this subset's is 210.",
    "(27000 + 210 + 0) ÷ 10^2 = 272.1",
  ]);
  // A difference of all ones.
  assert.deepEqual(cursorLines(cursor({ base: 27000, increment_width: 8, packed: null, value: value({ missing: true, text: "" }) })), [
    "Missing: this subset's difference of 8 bits is all ones. The smallest packed number is read from 16 bits at bit 194 of section 4.",
  ]);
  // A difference width of zero: the same in every subset.
  assert.deepEqual(cursorLines(cursor({ base: 27210, increment_width: 0 })), [
    "Every subset has the same value: no differences are written.",
    "(27210 + 0) ÷ 10^2 = 272.1, from 16 bits at bit 194 of section 4.",
  ]);
  // Compressed text counts characters and has no packed number.
  const text = cursor({ numeric: false, width: 160, base: null, packed: null, increment_width: 20, value: value({ text: "Praha-Ruzyne", unit: "" }) });
  assert.deepEqual(cursorLines(text), [
    "Compressed: a 6-bit length, then 20 characters of text for each subset.",
    "160 bits at bit 194 of section 4.",
  ]);
});

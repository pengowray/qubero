// How the strings view's status line counts the strings a template reads as
// numbers, code or packed data.

import { test } from "node:test";
import assert from "node:assert/strict";

import { STRINGSVIEW as SV } from "../src/strings.ts";

test("one kind of hidden string is not counted twice", () => {
  assert.equal(SV.statusHidden(8727, { numbers: 0, code: 8727, packed: 0 }), "8,727 hidden (machine code)");
});

test("several kinds of hidden string are counted each", () => {
  assert.equal(SV.statusHidden(31, { numbers: 27, code: 4, packed: 0 }), "31 hidden (27 numeric data, 4 machine code)");
});

test("strings left in the list say which kind they are inside", () => {
  assert.equal(SV.statusInside({ numbers: 0, code: 0, packed: 134 }), "(134 in compressed data)");
  assert.equal(SV.statusInside({ numbers: 4, code: 2, packed: 3 }), "(4 in numeric data, 2 in machine code, 3 in compressed data)");
});

test("a list with every string hidden says none are shown", () => {
  assert.equal(SV.statusShown(0), "No strings shown");
  assert.equal(SV.statusShown(8108), "8,108 strings shown");
});

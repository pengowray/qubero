// How the strings view's status line counts the strings a template reads as
// numbers, code or packed data, and how it names the packed data.

import { test } from "node:test";
import assert from "node:assert/strict";

import { STRINGSVIEW as SV } from "../src/strings.ts";

test("one kind of hidden string is not counted twice", () => {
  assert.equal(SV.statusHidden(8727, { numbers: 0, code: 8727, packed: {} }), "8,727 hidden (machine code)");
});

test("several kinds of hidden string are counted each", () => {
  assert.equal(SV.statusHidden(31, { numbers: 27, code: 4, packed: {} }), "31 hidden (27 numeric data, 4 machine code)");
});

test("strings left in the list say which kind they are inside", () => {
  assert.equal(SV.statusInside({ numbers: 0, code: 0, packed: { compression: 134 } }), "(134 in compressed data)");
  assert.equal(SV.statusInside({ numbers: 4, code: 2, packed: { compression: 3 } }), "(4 in numeric data, 2 in machine code, 3 in compressed data)");
});

test("packed data is named by what its codec does", () => {
  assert.equal(SV.statusHidden(812, { numbers: 0, code: 0, packed: { scrambling: 812 } }), "812 hidden (scrambled data)");
  assert.equal(SV.statusInside({ numbers: 0, code: 0, packed: { scrambling: 9, compression: 3 } }), "(3 in compressed data, 9 in scrambled data)");
  const title = SV.insideTitle("packed", "image", "tascam scramble", 0x20, 1024, "scrambling");
  assert.match(title, /reads as scrambled data \(tascam scramble, /);
  assert.match(title, /Scrambled data often contains printable bytes by chance\. Click to open the unscrambled data/);
  // A string from a core that sent no kind keeps the words for compression.
  assert.match(SV.insideTitle("packed", "data", "deflate", 0, 10), /reads as compressed data .* Compressed data often .* open the unpacked data/);
});

test("a string inside base64 is chance because of the word it spells, not its printable bytes", () => {
  const title = SV.insideTitle("packed", "body", "base64", 0x340, 30, "encoding");
  assert.match(title, /reads as encoded data \(base64, /);
  assert.match(title, /Encoded data is printable throughout, so a word found in it is most likely there by chance\. Click to open the decoded data/);
  assert.doesNotMatch(title, /printable bytes by chance/);
});

test("a list with every string hidden says none are shown", () => {
  assert.equal(SV.statusShown(0), "No strings shown");
  assert.equal(SV.statusShown(8108), "8,108 strings shown");
});

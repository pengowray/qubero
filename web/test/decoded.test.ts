// What one code of a compressed block reads as, and the heading over a column
// of them.
//
// A code and the thing it encodes are two levels, and the panel used to show
// them as one: nine bits of Huffman code, and beside them a `kind` and a
// `value` of no width that are what the block's table says those bits mean.
// The wording that keeps the two apart is what is pinned down here, because it
// is the whole of the change: the numbers all come from the core, and every
// mistake left to make is in which of them a row claims to be showing.

import { test } from "node:test";
import assert from "node:assert/strict";

import { chipsHead, DECODED, PROPERTIES, VALUES } from "../src/strings.ts";

test("a literal names the symbol, the character and the byte", () => {
  // The character for reading the stream, the number for checking it against
  // the hex view, and the symbol number in front of both, which is the one
  // thing neither of the other two can be mistaken for.
  assert.equal(DECODED.literal(77, 0x4d), "77 = literal 'M' (0x4d)");
});

test("a byte with no glyph is its number alone", () => {
  // Empty quotes say nothing, and a newline between them says less.
  assert.equal(DECODED.literal(10, 0x0a), "10 = literal 0x0a");
  assert.equal(DECODED.literal(0, 0x00), "0 = literal 0x00");
});

test("the end mark is written out, since it is neither a byte nor a length", () => {
  assert.equal(DECODED.endOfBlock(256), "256 = end of block");
});

test("a match says what it copies before it says how it was written", () => {
  assert.equal(DECODED.copies(3, 4), "3 bytes back 4");
  assert.equal(DECODED.copies(1, 12), "1 byte back 12");
  assert.equal(DECODED.length(257, 3), "symbol 257 = length 3");
  assert.equal(DECODED.distance(3, 4), "symbol 3 = distance 4");
});

test("the two copies that read as mistakes say what they are", () => {
  // Deflate's run-length idiom: not a copy at all, but the byte before
  // repeated. `34 bytes back 1` on its own reads as a fault in the file.
  assert.equal(DECODED.repeated(34), "one byte, repeated 34 times");
  // And a copy whose source is shorter than its length, which is reading bytes
  // the same step is still writing.
  assert.equal(DECODED.overlap(4), "overlapping copy: the last 4 bytes, repeated");
});

test("a code with nothing after it says so rather than saying nothing", () => {
  // An absent clause would read as a row with nothing more to say, and what
  // the reader is checking is that no other bits were read there.
  assert.equal(DECODED.width(8, 0, 3, 0), "8-bit code, no extra bits");
  assert.equal(DECODED.width(5, 0, 4, 0), "5-bit code, no extra bits");
});

test("a code with extra bits carries the arithmetic, since the row above has only the answer", () => {
  assert.equal(DECODED.width(7, 1, 11, 1), "7-bit code, then 1 extra bit: 11 + 1");
  assert.equal(DECODED.width(9, 3, 67, 5), "9-bit code, then 3 extra bits: 67 + 5");
});

test("the four widths of a match account for the length row", () => {
  // The reason the widths are shown at all: a reader who adds 8 + 0 + 5 + 0
  // and finds the Length row saying 13 bits has checked the whole account of
  // the step. The clauses are what they add.
  const parts = [
    { code: 8, extra: 0 },
    { code: 5, extra: 0 },
  ];
  const clauses = parts.map((p) => DECODED.width(p.code, p.extra, 0, 0));
  assert.deepEqual(clauses, ["8-bit code, no extra bits", "5-bit code, no extra bits"]);
  assert.equal(parts.reduce((n, p) => n + p.code + p.extra, 0), 13);
});

test("where the bytes went is one address for one byte and two for a run", () => {
  // The leading plus is what says these count from the front of the unpacked
  // stream rather than from the front of the file.
  assert.equal(DECODED.wrote("+0x0", null, "1 byte"), "+0x0 · 1 byte");
  assert.equal(DECODED.wrote("+0x1c", "+0x1e", "3 bytes"), "+0x1c to +0x1e · 3 bytes");
});

test("a code's tooltip counts its place in the run, not its symbol", () => {
  // The collision the whole change exists to undo: `symbol 0 · literal 'M'`
  // said "symbol 0" about a thing that is alphabet symbol 77. The brackets are
  // the form every other value cell uses, so the number cannot be read as
  // anything but a position.
  //
  // The run is named from what it holds, because the entry the column folded
  // is the whole block and index 46 of a block is a row of its code table.
  assert.equal(VALUES.code("code", 0, "literal 'M'", 9), "codes[0] · literal 'M' · 9 bits");
  assert.equal(VALUES.code("code", 12, "match 3 back 4", 13), "codes[12] · match 3 back 4 · 13 bits");
  assert.equal(VALUES.code("code", 4, "end of block", 1), "codes[4] · end of block · 1 bit");
});

test("the column beside the bytes is headed by what is in it", () => {
  const codes = { gap: false, unit: "code" };
  const field = { gap: false, unit: null };
  const gap = { gap: true, unit: null };
  assert.equal(chipsHead([codes, codes]), "Codes");
  assert.equal(chipsHead([field, field]), "Fields");
  // No one word for what a mixed column holds, so the general one stands.
  assert.equal(chipsHead([codes, field]), "Fields");
  // Bytes nothing describes are not fields either, and a gap in the middle of
  // a run must not flip the heading back and forth as the reader scrolls.
  assert.equal(chipsHead([gap, codes, gap]), "Codes");
  assert.equal(chipsHead([]), "Fields");
  assert.equal(chipsHead([gap]), "Fields");
  // Any other word the format has for what it holds, in the same shape.
  assert.equal(chipsHead([{ gap: false, unit: "entry" }]), "Entries");
});

test("a first code is first of the block, in the reader's word for one of them", () => {
  assert.equal(PROPERTIES.placed.first({ parent: "dynamic block", child: "code" }), "first code of dynamic block");
  // Everything else keeps the words it had: a structure's children are fields.
  assert.equal(PROPERTIES.placed.first({ parent: "header", child: "field" }), "first field of header");
  assert.equal(PROPERTIES.placed.first({}), "first field of its parent");
});

test("a step of a trace is placed by the step in front, not by the decoder", () => {
  // The fallback, now that the codes themselves say `after literal 'Z'`. The
  // steps of a trace tile, so this is a fact about the file and not an appeal
  // to the program that read it.
  assert.equal(PROPERTIES.placed.trace(), "after the previous step");
  assert.equal(PROPERTIES.placed.follows({ field: "literal 'Z'" }), "after literal 'Z'");
});

test("a width a table in this file gave names the row that gave it", () => {
  assert.equal(PROPERTIES.sized.table({ field: "code length for symbol 77" }), "from code length for symbol 77");
  assert.equal(
    PROPERTIES.sized.table({ field: "code length for distance symbol 3" }),
    "from code length for distance symbol 3",
  );
  // With no row to name, the table is still the answer.
  assert.equal(PROPERTIES.sized.table({}), "from this block's code lengths");
  // And a fixed-Huffman block's codes are the opposite answer: RFC 1951 chose
  // the widths and nothing in the file could have chosen otherwise.
  assert.equal(PROPERTIES.sized.fixed(), "fixed by the format");
});

test("a match's length is settled by its codes, not by bytes of its own", () => {
  // `decoded from its own bytes` is the varint's clause and a match has no
  // bytes of its own: it is four runs of bits spread across whichever bytes
  // they fall in.
  assert.equal(PROPERTIES.sizedMatch(), "each code says how many extra bits follow it");
  assert.equal(PROPERTIES.sized.encoded(), "decoded from its own bytes");
});

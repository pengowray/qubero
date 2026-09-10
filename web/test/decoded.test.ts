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
import { addressParts } from "../src/dom.ts";

test("a literal's byte and its symbol are two rows, not one equation", () => {
  // For a literal the symbol number and the byte are the same number in two
  // bases, since symbols 0 to 255 are the bytes themselves. One row saying
  // both stated one fact twice and put an equals sign between the halves.
  assert.equal(DECODED.literalByte(0x4d), "0x4d 'M'");
  assert.equal(DECODED.symbolNumber(77), "77");
});

test("a byte with no glyph is its number alone", () => {
  // Empty quotes say nothing, and a newline between them says less. Hex leads
  // either way, so the number sits in the same place whether a glyph follows.
  assert.equal(DECODED.literalByte(0x0a), "0x0a");
  assert.equal(DECODED.literalByte(0x00), "0x00");
});

test("the end mark says what it produced, so the missing output row is an answer", () => {
  assert.equal(DECODED.endWrites, "writes no bytes");
  assert.equal(DECODED.symbolNumber(256), "256");
});

test("a match says what it copies before it says how it was written", () => {
  assert.equal(DECODED.copies(3, 4), "3 bytes back 4");
  assert.equal(DECODED.copies(1, 12), "1 byte back 12");
  // The value beside a label is the thing the label names. `Length code` gets
  // the symbol; what that symbol came to is the Copies row above.
  assert.equal(DECODED.codeSymbol(257), "symbol 257");
  assert.equal(DECODED.codeSymbol(3), "symbol 3");
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
  assert.equal(DECODED.codeBits(8, 0), "8-bit code, no extra bits");
  assert.equal(DECODED.codeBits(5, 0), "5-bit code, no extra bits");
});

test("the widths and the arithmetic are separate lines, since they answer different rows", () => {
  // One line ends at the bit counts, which add to the Length row two sections
  // down. The other is the decoded value, which adds to the Copies row at the
  // top. Sharing a line, `then 1 extra bit: 11 + 1` read as four bit counts.
  assert.equal(DECODED.codeBits(7, 1), "7-bit code, then 1 extra bit");
  assert.equal(DECODED.lengthSum(11, 1, 12), "length 11 + 1 = 12");
  assert.equal(DECODED.codeBits(9, 3), "9-bit code, then 3 extra bits");
  assert.equal(DECODED.distanceSum(67, 5, 72), "distance 67 + 5 = 72");
});

test("the four widths of a match account for the length row", () => {
  // The reason the widths are shown at all: a reader who adds 8 + 0 + 5 + 0
  // and finds the Length row saying 13 bits has checked the whole account of
  // the step. The clauses are what they add.
  const parts = [
    { code: 8, extra: 0 },
    { code: 5, extra: 0 },
  ];
  const clauses = parts.map((p) => DECODED.codeBits(p.code, p.extra));
  assert.deepEqual(clauses, ["8-bit code, no extra bits", "5-bit code, no extra bits"]);
  assert.equal(parts.reduce((n, p) => n + p.code + p.extra, 0), 13);
});

test("where the bytes went is one address for one byte and two for a run", () => {
  // The leading plus is what says these count from the front of the unpacked
  // stream rather than from the front of the file, and `dom.ts`'s `address`
  // hangs what it counts from on the mark itself.
  assert.equal(DECODED.wrote("+0x0", null), "+0x0");
  assert.equal(DECODED.wrote("+0x1c", "+0x1e"), "+0x1c to +0x1e");
});

test("the row saying where the bytes went names a place, not a value", () => {
  // `Unpacks to +0x0` had two readings: what the code unpacks to, which the
  // row two above already answered, or where its output was written, which is
  // the one meant. Only the second survives this label.
  assert.equal(DECODED.writtenLabel, "Written at");
  assert.equal(DECODED.endWrites, "writes no bytes");
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
  assert.equal(PROPERTIES.sizedMatch(), "total length of its two codes and their extra bits");
  assert.equal(PROPERTIES.sized.encoded(), "decoded from its own bytes");
});

test("only the plus that begins an address says what it counts from", () => {
  // `formatOffset` writes a sub-byte address as `0x69+7b`, where the plus
  // means seven bits past the byte. It is a third meaning of the same glyph,
  // and putting the stream answer on it would be a wrong answer on a mark
  // that promises a right one.
  const marks = (text: string): number => addressParts(text).filter((p) => p.mark).length;
  assert.equal(marks("0x69+7b"), 0);
  assert.equal(marks("+0x1c"), 1);
  // Both ends of a range count from the same place, so both are marked.
  assert.equal(marks("+0x1c to +0x1e"), 2);
  // And a stream address that is itself sub-byte marks only its front.
  assert.equal(marks("+0x1c+3b"), 1);
  // The pieces still join back into what they came from.
  for (const t of ["0x69+7b", "+0x1c", "+0x1c to +0x1e", "+0x1c+3b", "0x40"]) {
    assert.equal(addressParts(t).map((p) => p.text).join(""), t);
  }
});

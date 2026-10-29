// The arithmetic behind the bit-scale pieces: the bit ruler's cells and the
// varint figure's gather. See web/src/viz/bitlayout.ts. The bit-order
// ribbon's own arithmetic is in bitorder.test.ts.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  bytesOf,
  checkedGather,
  fieldValueBits,
  gather,
  leadingInteger,
  orderFor,
  readAs,
  rowBitsFor,
  rulerCells,
  runsOf,
  valueBit,
  wordEndian,
  type VarintGroup,
} from "../src/viz/bitlayout.ts";
import { bitOrderLayout, partValues, readOrderDigits } from "../src/bitorder.ts";

// ----- the ruler -----

test("cells run in file order, each byte high bit first as the hex view shows it", () => {
  const cells = rulerCells(6, 2, 16, "high-first");
  assert.equal(cells.length, 16);
  assert.deepEqual(cells.slice(0, 3).map((c) => [c.byte, c.low, c.bit]), [[6, 7, 48], [6, 6, 49], [6, 5, 50]]);
  assert.deepEqual(cells.slice(8, 9).map((c) => [c.byte, c.low, c.bit]), [[7, 7, 56]]);
  assert.ok(cells.every((c) => c.row === 0));
});

test("low bit first turns each byte round and keeps the bytes in file order", () => {
  const cells = rulerCells(0, 2, 16, "low-first");
  assert.deepEqual(cells.slice(0, 8).map((c) => c.low), [0, 1, 2, 3, 4, 5, 6, 7]);
  // The hex view's bit 7 of byte 0 (its lowest) is the first cell.
  assert.equal(cells[0]?.bit, 7);
  assert.equal(cells[8]?.byte, 1);
});

test("a word wider than a row wraps at the row's width", () => {
  const cells = rulerCells(0, 4, 16, "high-first");
  assert.deepEqual([cells[15]?.row, cells[15]?.col, cells[16]?.row, cells[16]?.col], [0, 15, 1, 0]);
});

test("a field read low bit first across a byte boundary is one box low bit first and two high bit first", () => {
  // Five bits from bit 5 of byte 0 (counted from the bottom): bits 5, 6, 7
  // of byte 0 and bits 0, 1 of byte 1.
  const bits = new Set(fieldValueBits(5, 5, true));
  assert.deepEqual([...bits].sort((a, b) => a - b), [0, 1, 2, 14, 15]);
  const low = runsOf(rulerCells(0, 2, 16, "low-first"), (b) => bits.has(b));
  assert.deepEqual(low, [{ row: 0, col0: 5, col1: 10 }]);
  const high = runsOf(rulerCells(0, 2, 16, "high-first"), (b) => bits.has(b));
  assert.deepEqual(high, [{ row: 0, col0: 0, col1: 3 }, { row: 0, col0: 14, col1: 16 }]);
});

test("a field read high bit first has its value bit 0 at its far end", () => {
  assert.deepEqual(fieldValueBits(12, 3, false), [14, 13, 12]);
  assert.deepEqual(bytesOf(fieldValueBits(12, 6, false)), [1, 2]);
});

test("rows are as wide as fit, never wider than the word, and never less than a byte", () => {
  assert.equal(rowBitsFor(4, 32 * 14, 14), 32);
  assert.equal(rowBitsFor(4, 260, 14), 16);
  assert.equal(rowBitsFor(2, 1000, 14), 16);
  assert.equal(rowBitsFor(4, 50, 14), 8);
});

test("a flags word's byte order comes from its bytes and its value", () => {
  // ZIP general-purpose flags 8 are stored 08 00.
  assert.equal(wordEndian([0x08, 0x00], 8), "le");
  assert.equal(wordEndian([0x00, 0x08], 8), "be");
  assert.equal(wordEndian([0x08, 0x01], 8), null);
  // PE Characteristics 0x022e, little-endian.
  assert.equal(wordEndian([0x2e, 0x02], 558), "le");
  // Bit 3 of a little-endian word is bit 3 of its first byte; of a
  // big-endian two-byte word, bit 3 of its second.
  assert.equal(valueBit(0, 3, 2, "le"), 3);
  assert.equal(valueBit(1, 3, 2, "be"), 3);
  assert.equal(valueBit(1, 1, 2, "le"), 9);
});

// ----- the varint gather -----

/** wasm's section size 187, stored `bb 01`. */
const LEB_187: VarintGroup[] = [
  { bits: "1", role: "more" },
  { bits: "0111011", role: "payload" },
  { bits: "0", role: "stop" },
  { bits: "0000001", role: "payload" },
];

test("LEB128 joins its groups lowest first: bb 01 is 187", () => {
  const g = gather(LEB_187, "low-first");
  assert.equal(g.value, 187n);
  assert.equal(g.width, 14);
  assert.deepEqual(g.joined, [3, 1]);
  assert.deepEqual(g.shifts, [7, 0]);
  // The first byte's value bits land at the low end of the number, places
  // 7 to 13 counted from its high end; the framing bits nowhere.
  assert.equal(g.stored[0]?.place, null);
  assert.deepEqual(g.stored.slice(1, 8).map((b) => b.place), [7, 8, 9, 10, 11, 12, 13]);
  assert.deepEqual(g.stored.slice(9, 16).map((b) => b.place), [0, 1, 2, 3, 4, 5, 6]);
  assert.deepEqual([g.stored[9]?.byte, g.stored[9]?.slot], [1, 1]);
});

test("VLQ joins in stored order, so the same bits are a different number", () => {
  const g = gather(LEB_187, "high-first");
  assert.equal(g.value, (0b0111011n << 7n) | 1n);
  assert.deepEqual(g.joined, [1, 3]);
});

test("7z's first byte is the top of the value and the rest follow lowest first", () => {
  const groups: VarintGroup[] = [
    { bits: "110", role: "width" },
    { bits: "00001", role: "payload" },
    { bits: "00000010", role: "payload" },
    { bits: "00000011", role: "payload" },
  ];
  const g = gather(groups, "first-high-rest-low");
  assert.deepEqual(g.joined, [1, 3, 2]);
  assert.equal(g.value, (1n << 16n) | (3n << 8n) | 2n);
});

test("an EBML ID keeps its length marker as part of the number", () => {
  const groups: VarintGroup[] = [
    { bits: "01", role: "width" },
    { bits: "000010", role: "payload" },
    { bits: "10000110", role: "payload" },
  ];
  assert.equal(gather(groups, "high-first", true).value, 0x4286n);
  assert.equal(gather(groups, "high-first", false).value, 0x0286n);
});

test("signed and zigzag readings", () => {
  assert.equal(readAs(0x7fn, 7, "signed"), -1n);
  assert.equal(readAs(0x3fn, 7, "signed"), 63n);
  assert.equal(readAs(187n, 14, "zigzag"), -94n);
  assert.equal(readAs(4n, 7, "zigzag"), 2n);
});

test("the join is checked against the value, and a mismatch draws no bands", () => {
  const ok = checkedGather(LEB_187, "leb128", "high_bit", "187");
  assert.equal(ok?.order, "low-first");
  assert.equal(ok?.reading, "unsigned");
  // A name the template gave the type: every order is tried.
  assert.equal(checkedGather(LEB_187, "u_size", "high_bit", "187")?.order, "low-first");
  assert.equal(checkedGather(LEB_187, "vlq", "high_bit", String((0b0111011 << 7) | 1))?.order, "high-first");
  assert.equal(checkedGather(LEB_187, "leb128", "high_bit", "12345"), null);
  assert.equal(checkedGather(LEB_187, "leb128", "high_bit", "a name"), null);
  assert.equal(checkedGather([{ bits: "0", role: "stop" }, { bits: "1111111", role: "payload" }], "sleb128", "high_bit", "-1")?.reading, "signed");
});

test("type names give the join order", () => {
  assert.equal(orderFor("leb128", "high_bit")?.order, "low-first");
  assert.equal(orderFor("zigzag", "high_bit")?.reading, "zigzag");
  assert.equal(orderFor("varint", "sqlite_ninth")?.order, "high-first");
  assert.equal(orderFor("EBML size", "ebml_size")?.order, "high-first");
  assert.equal(orderFor("u_size", "high_bit"), null);
  assert.equal(leadingInteger("1,024 bytes"), 1024n);
  assert.equal(leadingInteger("−69"), -69n);
});

// ----- the bit-order ribbon's wrong reading -----

test("read high bit first, each part's bits come in the order the hex view shows them", () => {
  // A 9-bit number stored low bit first from bit 5 of byte 10.
  const parts = [{ bits: 9, lowFirst: true, label: "n" }];
  const right = bitOrderLayout(85, 9, parts);
  const hex = bitOrderLayout(85, 9, parts, "hex");
  assert.deepEqual(hex.top, right.top);
  // Along the top the bits sit at 2, 1, 0 (byte 10) and 15 to 10 (byte 11);
  // ranked, bit 2 of the field comes first.
  assert.deepEqual(hex.bottom, [2, 1, 0, 8, 7, 6, 5, 4, 3]);
  // Bytes 0xe0 0x3f: the field is 1, 1, 1 then 1, 1, 1, 1, 1, 1 from the
  // bottom of each byte; all ones either way.
  const digits = readOrderDigits([0xe0, 0x3f], 85, 9);
  assert.equal(digits, "111111111");
  assert.deepEqual(partValues(right, digits, parts), [511]);
});

test("a number across two bytes reads differently straight off the hex", () => {
  // Four bits from bit 6 of byte 0: two from the top of byte 0 (both 1), two
  // from the bottom of byte 1 (0 then 1, low bit first).
  const parts = [{ bits: 4, lowFirst: true, label: "n" }];
  const digits = readOrderDigits([0xc0, 0x02], 6, 4);
  assert.equal(digits, "1101");
  // Low bit first: 1 + 2 + 8 = 11.
  assert.deepEqual(partValues(bitOrderLayout(6, 4, parts), digits, parts), [11]);
  // Straight off the hex: byte 0's two bits then byte 1's two, 11 then 10.
  assert.deepEqual(partValues(bitOrderLayout(6, 4, parts, "hex"), digits, parts), [0b1110]);
});

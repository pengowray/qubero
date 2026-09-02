// The two reorderings a JPEG card has to get right. Getting either wrong puts
// a real number in the wrong place, which is worse than showing nothing: a
// quantisation grid with its diagonals transposed still looks like a
// quantisation grid, and a Huffman code off by one bit still looks like a code.

import { test } from "node:test";
import assert from "node:assert/strict";

import { ZIGZAG, countRestarts, dezigzag, huffmanCodes, subsampling } from "../src/jpegtables.ts";

test("the zigzag order visits all sixty-four positions once", () => {
  assert.equal(ZIGZAG.length, 64);
  assert.deepEqual([...ZIGZAG].sort((a, b) => a - b), Array.from({ length: 64 }, (_, i) => i));
});

test("the zigzag turns at the corners", () => {
  // The first steps: across, down the diagonal, and back up it.
  assert.deepEqual(ZIGZAG.slice(0, 6), [0, 1, 8, 16, 9, 2]);
  // The direct current is first and the highest frequency is last.
  assert.equal(ZIGZAG[0], 0);
  assert.equal(ZIGZAG[63], 63);
  // The second-to-last is the other neighbour of the bottom-right corner.
  assert.deepEqual(ZIGZAG.slice(61, 64), [55, 62, 63]);
});

test("de-zigzagging puts each value where the zigzag says it goes", () => {
  const stored = Array.from({ length: 64 }, (_, z) => z);
  const natural = dezigzag(stored);
  for (let z = 0; z < 64; z++) assert.equal(natural[ZIGZAG[z] ?? -1], z);
});

test("a table cut short leaves holes rather than shifting the rest", () => {
  const natural = dezigzag([10, 20, 30]);
  assert.equal(natural[0], 10);
  assert.equal(natural[1], 20);
  assert.equal(natural[8], 30);
  assert.equal(natural[16], undefined);
  assert.equal(natural.length, 64);
});

test("the codes are rebuilt from the counts, shortest first", () => {
  // The table the template's own test file carries: one code of one bit is
  // absent, one of two bits, two of three.
  const counts = [0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
  assert.deepEqual(huffmanCodes(counts, [5, 6, 7]), [
    { bits: "00", length: 2, symbol: 5 },
    { bits: "010", length: 3, symbol: 6 },
    { bits: "011", length: 3, symbol: 7 },
  ]);
});

test("a full first length uses every code of that length", () => {
  const counts = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
  assert.deepEqual(huffmanCodes(counts, [0xaa, 0xbb]), [
    { bits: "0", length: 1, symbol: 0xaa },
    { bits: "1", length: 1, symbol: 0xbb },
  ]);
});

test("a longer code carries on from where the shorter ones stopped", () => {
  const counts = [1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
  assert.deepEqual(huffmanCodes(counts, [1, 2, 3]), [
    { bits: "0", length: 1, symbol: 1 },
    { bits: "10", length: 2, symbol: 2 },
    { bits: "110", length: 3, symbol: 3 },
  ]);
});

test("counts and symbols that disagree are no table at all", () => {
  const counts = [0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
  assert.equal(huffmanCodes(counts, [5, 6]), null);
  assert.equal(huffmanCodes(counts, [5, 6, 7, 8]), null);
  assert.equal(huffmanCodes([1, 2], [0, 1, 2]), null);
});

test("an empty table is empty rather than broken", () => {
  assert.deepEqual(huffmanCodes(new Array<number>(16).fill(0), []), []);
});

test("the subsampling is named only where the notation applies", () => {
  const one = { h: 1, v: 1 };
  assert.equal(subsampling([{ h: 2, v: 2 }, one, one]), "4:2:0");
  assert.equal(subsampling([{ h: 2, v: 1 }, one, one]), "4:2:2");
  assert.equal(subsampling([one, one, one]), "4:4:4");
  assert.equal(subsampling([{ h: 4, v: 1 }, one, one]), "4:1:1");
  assert.equal(subsampling([one]), "greyscale");
  // Chroma sampled more finely than luma has no name in the notation.
  assert.equal(subsampling([one, { h: 2, v: 2 }, one]), null);
  assert.equal(subsampling([one, one, one, one]), null);
});

test("restart markers are counted and stuffed bytes are not", () => {
  assert.equal(countRestarts(new Uint8Array([0xaa, 0xff, 0x00, 0xbb])), 0);
  assert.equal(countRestarts(new Uint8Array([0xff, 0xd0, 0xff, 0xd7, 0xff, 0x00])), 2);
  // An 0xff at the very end has nothing after it to tell it apart.
  assert.equal(countRestarts(new Uint8Array([0x12, 0xff])), 0);
  // The byte after a restart is not itself the start of another.
  assert.equal(countRestarts(new Uint8Array([0xff, 0xff, 0xd1])), 1);
});

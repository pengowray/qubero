// The gap check reads bytes and says what it found. What it must never do is
// say a run is empty because it did not look at it.

import { test } from "node:test";
import assert from "node:assert/strict";

import { checkGap, CHECK_LIMIT_BYTES } from "../src/gapcheck.ts";
import type { Doc } from "../src/doc.ts";

/** Just enough of `Doc` to answer a read. `missing` stands for bytes the file
 *  has not handed over yet. */
function doc(bytes: Uint8Array, missing = false): Doc {
  return {
    read: (at: number, len: number) => ({ bytes: bytes.subarray(at, at + len), complete: !missing }),
  } as unknown as Doc;
}

test("a run of zeros is said to be zeros, having been read", () => {
  assert.equal(checkGap(doc(new Uint8Array(64)), 0, 64 * 8), "zeros");
});

test("one byte that is not zero is enough to say so", () => {
  const b = new Uint8Array(64);
  b[63] = 1;
  assert.equal(checkGap(doc(b), 0, 64 * 8), "something");
});

test("bytes that have not arrived are not guessed at", () => {
  assert.equal(checkGap(doc(new Uint8Array(64), true), 0, 64 * 8), "unchecked");
});

test("a run too long to read is not read", () => {
  const big = (CHECK_LIMIT_BYTES + 1) * 8;
  assert.equal(checkGap(doc(new Uint8Array(0)), 0, big), "unchecked");
});

test("a run that does not fill whole bytes is left alone", () => {
  assert.equal(checkGap(doc(new Uint8Array(8)), 3, 8), "unchecked");
  assert.equal(checkGap(doc(new Uint8Array(8)), 0, 0), "unchecked");
});

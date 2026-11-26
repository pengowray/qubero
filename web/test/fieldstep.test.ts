// Where Alt+↑ and Alt+↓ go, on a file of three fields: a 2-byte magic, a
// 32 KiB system area, and a 4-byte count.

import { test } from "node:test";
import assert from "node:assert/strict";

import { fieldStep, stepDepth, type FieldAt } from "../src/fieldstep.ts";

test("the outer step is one level up from the field", () => {
  // A field of chunk 7 steps by chunk; an element of list 2 steps over the list.
  assert.equal(stepDepth([1, 7, 3]), 2);
  assert.equal(stepDepth([2, 900]), 1);
  // A field directly under the root steps as itself.
  assert.equal(stepDepth([4]), 1);
});

const fields = [
  { path: [0], start: 0, end: 16 },
  { path: [1], start: 16, end: 16 + 32768 * 8 },
  { path: [2], start: 16 + 32768 * 8, end: 16 + 32772 * 8 },
];
const length = fields[2]!.end;
const at: FieldAt = (bit) => fields.find((f) => bit >= f.start && bit < f.end) ?? null;

test("up from the middle of a field goes to its start", () => {
  assert.deepEqual(fieldStep(at, 4000 * 8, "up", length), { path: [1], start: 16 });
});

test("up from the start of a field goes to the start of the one before", () => {
  assert.deepEqual(fieldStep(at, 16, "up", length), { path: [0], start: 0 });
});

test("up at the start of the file goes nowhere", () => {
  assert.equal(fieldStep(at, 0, "up", length), null);
});

test("down goes to the start of the next field, from anywhere in this one", () => {
  assert.deepEqual(fieldStep(at, 16, "down", length), { path: [2], start: 16 + 32768 * 8 });
  assert.deepEqual(fieldStep(at, 4000 * 8, "down", length), { path: [2], start: 16 + 32768 * 8 });
});

test("down in the last field goes nowhere", () => {
  assert.equal(fieldStep(at, 16 + 32769 * 8, "down", length), null);
});

test("no field under the cursor, or not read yet, goes nowhere", () => {
  assert.equal(fieldStep(() => null, 100, "down", length), null);
});

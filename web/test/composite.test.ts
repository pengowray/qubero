// The two shapes a structure can be read as one value: a length beside the
// string it sizes, and a short row of scalars. Everything else keeps its count.

import { test } from "node:test";
import assert from "node:assert/strict";

import { insideValue } from "../src/composite.ts";
import type { TemplateNode } from "../src/doc.ts";

/** Enough of a node for the rules under test. The rest of `TemplateNode` is
 *  not read here, and writing it out per kid would bury what each case says. */
function node(over: Partial<TemplateNode>): TemplateNode {
  return { name: "f", type: "u8", kind: "uint", value: "0", composite: false, child_count: 0, consumed_by: null, ...over } as TemplateNode;
}

function struct(kids: readonly TemplateNode[], over: Partial<TemplateNode> = {}): TemplateNode {
  return node({ composite: true, kind: "composite", type: "Thing", child_count: kids.length, ...over });
}

/** The same, typed as the list it is: only a list reads as a row of values. */
function list(kids: readonly TemplateNode[], over: Partial<TemplateNode> = {}): TemplateNode {
  return struct(kids, { type: "u64 le[]", ...over });
}

test("a length and the string it sizes read as the string", () => {
  const kids = [node({ name: "len", value: "5", consumed_by: 1 }), node({ name: "text", kind: "str", value: "llama" })];
  assert.deepEqual(insideValue(struct(kids), kids), { kind: "payload", node: kids[1] });
});

test("bytes count as a payload, since a run of them is still one value", () => {
  const kids = [node({ name: "len", value: "2", consumed_by: 1 }), node({ name: "data", kind: "bytes", value: "00 01" })];
  assert.equal(insideValue(struct(kids), kids)?.kind, "payload");
});

test("two children nothing places are two values, not one", () => {
  const kids = [node({ name: "a", value: "1" }), node({ name: "b", kind: "str", value: "x" })];
  assert.equal(insideValue(struct(kids), kids), null);
});

test("a count beside the offset it places is not an array of two", () => {
  // The payload has to be text or bytes, and a field that places another is
  // not a peer of it, so neither reading applies.
  const kids = [node({ name: "n", value: "2", consumed_by: 1 }), node({ name: "at", value: "64" })];
  assert.equal(insideValue(list(kids), kids), null);
});

test("a short row of numbers reads as the array it is", () => {
  const kids = [node({ name: "[0]", value: "4096" }), node({ name: "[1]", value: "32000" })];
  assert.deepEqual(insideValue(list(kids), kids), { kind: "row", text: "[4096, 32000]" });
});

test("two peers that are not a list keep their count", () => {
  const kids = [node({ name: "width", value: "640" }), node({ name: "height", value: "480" })];
  assert.equal(insideValue(struct(kids), kids), null);
});

test("numbers of different kinds are not a row", () => {
  const kids = [node({ name: "[0]", value: "1" }), node({ name: "[1]", kind: "str", value: "x" })];
  assert.equal(insideValue(list(kids), kids), null);
});

test("more than a handful of items keeps its count", () => {
  const kids = Array.from({ length: 9 }, (_, i) => node({ name: `[${i}]`, value: String(i) }));
  assert.equal(insideValue(list(kids), kids), null);
});

test("a row too long for one line keeps its count", () => {
  const kids = Array.from({ length: 6 }, (_, i) => node({ name: `[${i}]`, value: `1234567${i}` }));
  assert.equal(insideValue(list(kids), kids), null);
});

test("a structure whose children did not all arrive says nothing about them", () => {
  const kids = [node({ name: "len", value: "5", consumed_by: 1 }), node({ name: "text", kind: "str", value: "llama" })];
  assert.equal(insideValue(struct(kids, { child_count: 40 }), kids), null);
});

// The decoder sections of the template text: the index leaves them out of the
// template's own structure, and a node a decoder laid down is marked on its
// kind's line. The text is the core's own gzip and zip snapshots.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { indexTemplate, kindLineAt, tokenizeLine, type KindLine } from "../src/templateindex.ts";
import { fieldLines } from "../src/templatejump.ts";

const snapshot = (name: string): string => readFileSync(new URL(`../../crates/core/tests/snapshots/template_text/${name}.txt`, import.meta.url), "utf8");
const GZIP = snapshot("gzip");
const ZIP = snapshot("zip");

/** The line, counted from 0, of the first line of `text` that is `s` once trimmed, after line `from`. */
const lineOf = (text: string, s: string, from = 0): number => text.split("\n").findIndex((l, i) => i >= from && l.trim() === s);

test("a decoder section is not part of the template's own structure", () => {
  const idx = indexTemplate(GZIP);
  const head = lineOf(GZIP, "decoder deflate {");
  assert.ok(head > 0);
  assert.deepEqual(idx.decoders, [[head, GZIP.replace(/\n$/, "").split("\n").length - 1]]);
  // No structure opened inside it: not `blocks`, `block`, `codes` or a `Symbol`.
  assert.ok(idx.blocks.every((b) => b.open < head), JSON.stringify(idx.blocks.map((b) => b.name)));
  assert.equal(idx.byName.get("Symbol"), undefined);
  assert.ok([...idx.stmtEnd.keys()].every((l) => l < head));
  // A kind's line is coloured as a field is and cannot be clicked.
  const literal = lineOf(GZIP, "literal: repeat(any number) codebits Huffman code (fixed or table)");
  assert.equal(idx.kinds[literal], "kind");
  assert.equal(fieldLines(idx)[literal], -1);
  assert.equal(tokenizeLine(GZIP.split("\n")[literal] ?? "", "kind")[1]?.cls, "field");
  assert.equal(tokenizeLine("decoder deflate {", "plain")[0]?.cls, "kw");
  // The root's fields are found as before.
  assert.deepEqual([...(idx.root?.fields.keys() ?? [])], ["members", "decoded"]);
  assert.ok((idx.root?.end ?? head) < head);
});

test("two sections are both left out, and a type's span stops before them", () => {
  const idx = indexTemplate(ZIP);
  const deflate = lineOf(ZIP, "decoder deflate {");
  const lzma = lineOf(ZIP, "decoder lzma for xz {");
  assert.ok(deflate > 0 && lzma > deflate);
  assert.equal(idx.decoders.length, 2);
  assert.deepEqual(idx.decoders.map((d) => d[0]), [deflate, lzma]);
  for (const [, z] of idx.typeSpans) assert.ok(z < deflate);
  // Every structure ends before the sections, the last one included.
  assert.ok(idx.blocks.length > 0 && idx.blocks.every((b) => b.end < deflate));
});

const LINES: KindLine[] = [
  { kind: "lz4.blocks", parent: "", line: 10 },
  { kind: "lz4.block", parent: "lz4.blocks", line: 11 },
  { kind: "lz4.token", parent: "lz4.block", line: 13 },
  { kind: "lz4.codes", parent: "lz4.block", line: 20 },
  { kind: "lz4.token", parent: "lz4.codes", line: 21 },
];

test("the deepest node with a kind is marked on its kind's line, under its parent's kind", () => {
  // file, the decoded field, its blocks, a block, its codes, a token.
  const codes = [undefined, undefined, "lz4.blocks", "lz4.block", "lz4.codes", "lz4.token"];
  assert.equal(kindLineAt(LINES, codes), 21);
  assert.equal(kindLineAt(LINES, [undefined, undefined, "lz4.blocks", "lz4.block", "lz4.token"]), 13);
  // A field of a token's record has no kind; its token decides.
  assert.equal(kindLineAt(LINES, [...codes, undefined]), 21);
  // A parent the lines do not list falls back to the kind's first line.
  assert.equal(kindLineAt(LINES, ["lz4.other", "lz4.token"]), 13);
  assert.equal(kindLineAt(LINES, [undefined, "lz4.blocks"]), 10);
});

test("no kind, or a kind the text has no line for, marks nothing", () => {
  assert.equal(kindLineAt(LINES, [undefined, undefined]), null);
  assert.equal(kindLineAt(LINES, []), null);
  assert.equal(kindLineAt(LINES, [undefined, "deflate.literal"]), null);
  assert.equal(kindLineAt([], ["lz4.token"]), null);
});

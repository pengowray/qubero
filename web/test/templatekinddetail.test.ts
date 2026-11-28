// Which decoder kind a line of the template text belongs to, for the strip
// under the Template tab's text. The text here is made up, since the core's
// own text for records is changing; only its indentation matters.

import { test } from "node:test";
import assert from "node:assert/strict";

import { kindOwnerAt } from "../src/templatepanel.ts";
import type { KindLine } from "../src/templateindex.ts";

const TEXT = `struct File {
  decoded: u8
}

enum SymbolKind {
  0 = literal, 1 = match
}

decoder deflate {
  blocks: traced blocks {
    block: repeat(any number) traced block {
      bfinal: width(fixed) computed enum DeflateFinal {
        0 = more blocks follow, 1 = the last block
      }
      hlit: computed
      codes: traced symbols {
        stored_data: Symbol {
          kind: computed 3 enum SymbolKind
          length: computed
        }
        literal: repeat(any number) codebits Huffman code (fixed or table)
      }
    }
  }
}`;
const LINES = TEXT.split("\n");
const at = (s: string): number => LINES.findIndex((l) => l.trim() === s);
const HEAD = at("decoder deflate {");
const DECODERS: [number, number][] = [[HEAD, LINES.length - 1]];
const names = ["blocks", "block", "bfinal", "hlit", "codes", "stored_data", "literal"];
const KINDS: KindLine[] = names.map((n) => ({ kind: `deflate.${n}`, parent: "", line: LINES.findIndex((l) => l.trimStart().startsWith(`${n}:`)) }));
const owner = (line: number): string | undefined => KINDS[kindOwnerAt(LINES, DECODERS, KINDS, line)]?.kind;

test("a kind's own line is that kind", () => {
  for (const k of KINDS) assert.equal(owner(k.line), k.kind);
});

test("a record's lines and its closing brace belong to the record", () => {
  const rec = at("stored_data: Symbol {");
  assert.equal(owner(rec + 1), "deflate.stored_data");
  assert.equal(owner(rec + 2), "deflate.stored_data");
  assert.equal(owner(rec + 3), "deflate.stored_data");
  assert.equal(LINES[rec + 3]?.trim(), "}");
});

test("an enum's cases belong to the kind whose line opens them", () => {
  assert.equal(owner(at("0 = more blocks follow, 1 = the last block")), "deflate.bfinal");
  assert.equal(owner(at("0 = more blocks follow, 1 = the last block") + 1), "deflate.bfinal");
});

test("a row's closing brace belongs to the row, not to its last child", () => {
  const lit = at("literal: repeat(any number) codebits Huffman code (fixed or table)");
  assert.equal(owner(lit + 1), "deflate.codes");
  assert.equal(owner(lit + 2), "deflate.block");
  assert.equal(owner(lit + 3), "deflate.blocks");
});

test("lines outside a decoder section, and the section's own braces, belong to none", () => {
  assert.equal(owner(HEAD), undefined);
  assert.equal(owner(LINES.length - 1), undefined);
  assert.equal(owner(at("0 = literal, 1 = match")), undefined);
  assert.equal(owner(at("decoded: u8")), undefined);
});

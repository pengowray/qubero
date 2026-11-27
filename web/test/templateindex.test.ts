// Reading the template text back: which lines are fields, which structures
// hold them, and which line a node at the cursor is declared on. The excerpts
// are cut from what the core writes for ELF and GGUF, wrapped lines included.

import { test } from "node:test";
import assert from "node:assert/strict";

import { findLine, indexTemplate, tokenizeLine, type ChainNode } from "../src/templateindex.ts";

const GGUF = `template gguf

root GGUF {
  magic: magic "GGUF"
  metadata_count: u64le
  metadata: Metadata[metadata_count]
}

type Metadata (named by key, contents value) {
  key: String (contents text) {
    len: u64le
    text: text[len] utf8
  }
  value_type: u32le enum GgufType {
    0 = uint8, 1 = int8, 2 = uint16, 3 = int16, 4 = uint32, 5 = int32, 6 = float32, 7 = bool,
    8 = string, 9 = array, 10 = uint64, 11 = int64, 12 = float64
  }
  value: Value
}

type Value = switch value_type {
  0 => u8be
  8 => String (contents text) {
    len: u64le
    text: text[len] utf8
  }
  _ => bytes[0]
}
`;

// Two arms that both write \`ELFHeader\`, told apart by the width of \`entry\`,
// and a structure whose opening brace is on a wrapped line.
const ELF = `template elf

root ELF {
  class: u8be enum Class {1 = 32-bit, 2 = 64-bit}
  // Reserved: the rest of the 16-byte identification, unused since the first ELF and written as
      zeros.
  header: switch class {
    1 => ELFHeader {
      entry: u32le
      program_header_offset: u32le
      program_headers: at(program_header_offset from file) ProgramHeader
          (named by type, unit segment, line type then file_size worded "{} bytes
          in the file") {
        type: u32le
        file_size: u32le
      }[2]
    }
    2 => ELFHeader {
      entry: u64le
      program_header_offset: u64le
      program_headers: at(program_header_offset from file) ProgramHeader
          (named by type, unit segment, line type then file_size worded "{} bytes
          in the file") {
        type: u32le
        file_size: u64le
      }[2]
    }
  }
}
`;

// ELF as it reads when each word size writes its big-endian arm as one line
// pointing at the little-endian one, and a section switch with an arm the
// same as another.
const SHORT = `template elf

root ELF {
  class: u8be enum Class {1 = 32-bit, 2 = 64-bit}
  data: u8be
  header: switch class {
    1 => switch data {
      1 => ELFHeader {
        tag: u8be
        entry: u32le
        program_headers: at(entry from file) ProgramHeader (unit segment) {
          type: u32le
          file_size: u32le
        }[2]
      }
      2 => same as case 1, but big-endian
      _ => bytes[0]
    }
    2 => switch data {
      1 => ELFHeader {
        tag: u8be
        entry: u64le
        program_headers: at(entry from file) ProgramHeader (unit segment) {
          type: u32le
          file_size: u64le
        }[2]
      }
      2 => same as case 1, but big-endian
      _ => bytes[0]
    }
    _ => bytes[0]
  }
  table: switch data {
    2 => Symbol {
      value: u32le
    }
    11 => same as case 2
    "a => b" => same as case 2
    _ => bytes[0]
  }
}
`;

const line = (text: string, n: number | null): string | null => (n === null ? null : (text.split("\n")[n] ?? null));
const node = (name: string, type: string, list = false): ChainNode => ({ name, type, list });

test("fields are found inside the structure that declares them, not in nested ones", () => {
  const idx = indexTemplate(GGUF);
  const meta = idx.byName.get("Metadata")?.[0];
  assert.ok(meta);
  assert.deepEqual([...meta.fields.keys()], ["key", "value_type", "value"]);
  assert.equal(idx.byName.get("String")?.length, 2);
  assert.equal(idx.root?.name, "GGUF");
});

test("an enum's values and a comment's carried-on line are not fields", () => {
  const idx = indexTemplate(ELF);
  const lines = ELF.split("\n");
  const at = (s: string): number => lines.findIndex((l) => l.includes(s));
  const gguf = indexTemplate(GGUF);
  const enumLine = GGUF.split("\n").findIndex((l) => l.includes("0 = uint8"));
  assert.ok(enumLine > 0);
  assert.equal(gguf.kinds[enumLine], "plain");
  assert.equal(idx.kinds[at("// Reserved")], "comment");
  assert.equal(idx.kinds[at("zeros.")], "comment");
  assert.equal(idx.kinds[at("class: u8be")], "field");
  assert.deepEqual([...(idx.root?.fields.keys() ?? [])], ["class", "header"]);
});

test("a structure opened on a wrapped line is named by the word before its brackets", () => {
  const idx = indexTemplate(ELF);
  const ph = idx.byName.get("ProgramHeader");
  assert.equal(ph?.length, 2);
  assert.deepEqual([...(ph?.[0]?.fields.keys() ?? [])], ["type", "file_size"]);
  // The name is marked where it is written, on the first line of the statement.
  const open = ph?.[0]?.open ?? -1;
  const [a, z] = idx.decls[open]?.[0] ?? [0, 0];
  assert.equal(ELF.split("\n")[open]?.slice(a, z), "ProgramHeader");
});

test("a field below the root is found by its parent's type", () => {
  const idx = indexTemplate(GGUF);
  const chain = [node("file", "GGUF"), node("metadata", "Metadata[]", true), node("[0] general.name", "Metadata"), node("key", "String"), node("len", "u64 le")];
  assert.equal(line(GGUF, findLine(idx, chain, () => null)), "    len: u64le");
});

test("a structure written inside a named type is the one that type reaches", () => {
  const idx = indexTemplate(GGUF);
  const chain = [node("file", "GGUF"), node("metadata", "Metadata[]", true), node("[0] a", "Metadata"), node("value", "String"), node("text", "utf8[]")];
  const n = findLine(idx, chain, () => null);
  assert.equal(line(GGUF, n), "    text: text[len] utf8");
  // The one in `type Value`, not the one in `Metadata.key`.
  assert.ok(n !== null && n > GGUF.split("\n").findIndex((l) => l.startsWith("type Value")));
});

test("a list's elements and an element's own elements mark the field the list is", () => {
  const idx = indexTemplate(GGUF);
  const chain = [node("file", "GGUF"), node("metadata", "Metadata[]", true), node("[3] x", "Metadata")];
  assert.equal(line(GGUF, findLine(idx, chain, () => null)), "  metadata: Metadata[metadata_count]");
});

test("two structures of one name are told apart by their children's types", () => {
  const idx = indexTemplate(ELF);
  const kids = [node("entry", "u64 le"), node("program_header_offset", "u64 le"), node("program_headers", "at → ProgramHeader[]")];
  const chain = [node("file", "ELF"), node("header", "ELFHeader"), node("entry", "u64 le")];
  const n = findLine(idx, chain, (depth) => (depth === 1 ? kids : null));
  assert.equal(line(ELF, n), "      entry: u64le");
  // Through `at`, the placing node and the placed list share a name, and the
  // element's fields are those of the structure in the chosen arm.
  const deep = [
    node("file", "ELF"),
    node("header", "ELFHeader"),
    node("program_headers", "at → ProgramHeader[]"),
    node("program_headers", "ProgramHeader[]", true),
    node("[0] load", "ProgramHeader"),
    node("file_size", "u64 le"),
  ];
  const m = findLine(idx, deep, (depth) => (depth === 1 ? kids : null));
  assert.equal(line(ELF, m), "        file_size: u64le");
  assert.ok(m !== null && m > (n ?? 0));
});

test("when the children cannot choose, the enclosing field's line is kept", () => {
  const idx = indexTemplate(ELF);
  const chain = [node("file", "ELF"), node("header", "ELFHeader"), node("entry", "u64 le")];
  assert.equal(line(ELF, findLine(idx, chain, () => null)), "  header: switch class {");
  assert.equal(line(ELF, findLine(idx, chain, () => [node("nothing", "u8")])), "  header: switch class {");
});

test("a node no line declares finds nothing", () => {
  const idx = indexTemplate(GGUF);
  assert.equal(findLine(idx, [node("file", "GGUF"), node("nowhere", "u8")], () => null), null);
  assert.equal(findLine(idx, [], () => null), null);
});

test("the tokenizer colours a field's name, declared names and the notation's words", () => {
  const segs = tokenizeLine("      program_headers: at(program_header_offset from file) ProgramHeader", "field", [[59, 72]]);
  assert.deepEqual(
    segs.filter((s) => s.cls !== "").map((s) => [s.cls, s.text]),
    [
      ["field", "program_headers"],
      ["kw", "at"],
      ["kw", "from"],
      ["decl", "ProgramHeader"],
    ],
  );
  assert.equal(segs.map((s) => s.text).join(""), "      program_headers: at(program_header_offset from file) ProgramHeader");
});

test("the tokenizer leaves field names alone where they are only words", () => {
  // `chain` without a bracket is a field read in an expression; `from` inside
  // quotes is text; `type` is a keyword only at the start of a line.
  const segs = tokenizeLine('    x: u8 enum E {0 = chain, 1 = "from"} type', "plain");
  assert.deepEqual(
    segs.filter((s) => s.cls !== "").map((s) => [s.cls, s.text]),
    [["kw", "enum"]],
  );
  const top = tokenizeLine("type Metadata (named by key) {", "plain", [[5, 13]]);
  assert.deepEqual(
    top.filter((s) => s.cls !== "").map((s) => [s.cls, s.text]),
    [
      ["kw", "type"],
      ["decl", "Metadata"],
    ],
  );
  assert.deepEqual(tokenizeLine("  // a note", "comment"), [{ text: "  // a note", cls: "comment" }]);
});

test("a same-as line knows the arm it points at and the byte order it names", () => {
  const idx = indexTemplate(SHORT);
  const text = (n: number | null): string | null => line(SHORT, n);
  assert.deepEqual(
    idx.sameAs.map((s) => [text(s.line)?.trim(), text(s.target)?.trim(), s.order]),
    [
      ["2 => same as case 1, but big-endian", "1 => ELFHeader {", "big"],
      ["2 => same as case 1, but big-endian", "1 => ELFHeader {", "big"],
      ["11 => same as case 2", "2 => Symbol {", null],
      ['"a => b" => same as case 2', "2 => Symbol {", null],
    ],
  );
  // The two `1 => ELFHeader` targets are the two written-out arms, in order.
  const [a, b] = idx.sameAs;
  assert.ok(a !== undefined && b !== undefined && a.target !== null && b.target !== null && a.target < b.target);
});

test("a field read by a big-endian arm written as one line lands in the arm it points at", () => {
  const idx = indexTemplate(SHORT);
  const kids64 = [node("tag", "u8 be"), node("entry", "u64 be"), node("program_headers", "at → ProgramHeader[]")];
  const chain = [node("file", "ELF"), node("header", "ELFHeader"), node("entry", "u64 be")];
  const n = findLine(idx, chain, (depth) => (depth === 1 ? kids64 : null));
  assert.equal(line(SHORT, n), "        entry: u64le");
  // In the 64-bit arm, not the 32-bit one.
  assert.ok(n !== null && n > SHORT.split("\n").findIndex((l) => l.includes("entry: u32le")));
  // A 32-bit big-endian file lands in the 32-bit arm.
  const kids32 = [node("tag", "u8 be"), node("entry", "u32 be"), node("program_headers", "at → ProgramHeader[]")];
  const m = findLine(idx, [node("file", "ELF"), node("header", "ELFHeader"), node("entry", "u32 be")], (depth) => (depth === 1 ? kids32 : null));
  assert.equal(line(SHORT, m), "        entry: u32le");
  // And deeper, through the placed list.
  const deep = [
    node("file", "ELF"),
    node("header", "ELFHeader"),
    node("program_headers", "at → ProgramHeader[]"),
    node("program_headers", "ProgramHeader[]", true),
    node("[0] load", "ProgramHeader"),
    node("file_size", "u64 be"),
  ];
  const d = findLine(idx, deep, (depth) => (depth === 1 ? kids64 : null));
  assert.equal(line(SHORT, d), "          file_size: u64le");
  assert.ok(d !== null && n !== null && d > n);
});

test("a little-endian file still lands in the arm written for it", () => {
  const idx = indexTemplate(SHORT);
  const kids = [node("tag", "u8 be"), node("entry", "u64 le"), node("program_headers", "at → ProgramHeader[]")];
  const n = findLine(idx, [node("file", "ELF"), node("header", "ELFHeader"), node("entry", "u64 le")], (depth) => (depth === 1 ? kids : null));
  assert.equal(line(SHORT, n), "        entry: u64le");
  assert.ok(n !== null && n > SHORT.split("\n").findIndex((l) => l.includes("entry: u32le")));
});

test("a field read by an arm the same as another lands in that other arm", () => {
  const idx = indexTemplate(SHORT);
  const n = findLine(idx, [node("file", "ELF"), node("table", "Symbol"), node("value", "u32 le")], () => null);
  assert.equal(line(SHORT, n), "      value: u32le");
});

// A switch whose arms share `same as` lines, one of them folded because its
// keys do not fit on one line.
const GROUPED = `template macho

root MachO {
  kind: u32be
  command: switch kind {
    29 => Signature {
      offset: u32le
    }
    30, 38, 41 => same as case 29
    2 => u8be
    12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 31, 32, 33, 34, 35, 36,
        37, 39, 40 => same as case 29, but big-endian
    "a, b", 'fmt ' => same as case 29
    _ => bytes[0]
  }
}
`;

test("a line of several keys is one same-as line, folded or not", () => {
  const idx = indexTemplate(GROUPED);
  const text = (n: number | null): string | null => line(GROUPED, n);
  assert.deepEqual(
    idx.sameAs.map((s) => [text(s.line)?.trim().slice(0, 10), text(s.target)?.trim(), s.order]),
    [
      ["30, 38, 41", "29 => Signature {", null],
      ["12, 13, 14", "29 => Signature {", "big"],
      ['"a, b", \'f', "29 => Signature {", null],
    ],
  );
  // The folded line's second half is not a statement of its own.
  const folded = GROUPED.split("\n").findIndex((l) => l.trimStart().startsWith("37, 39"));
  assert.equal(idx.stmtEnd.get(folded), undefined);
});

test("a field read through any key of a shared line is found in the arm it names", () => {
  const idx = indexTemplate(GROUPED);
  const n = findLine(idx, [node("file", "MachO"), node("command", "Signature"), node("offset", "u32 be")], () => null);
  assert.equal(line(GROUPED, n), "      offset: u32le");
});

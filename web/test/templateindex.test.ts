// Reading the template text back: which lines are fields, which structures
// hold them, and which line a node at the cursor is declared on. The excerpts
// are cut from what the core writes for ELF and GGUF, wrapped lines included.

import { test } from "node:test";
import assert from "node:assert/strict";

import { findLine, findLineAt, indexTemplate, readAs, tokenizeLine, type ChainNode } from "../src/templateindex.ts";

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

// A TIFF inside a PNG, with the big-endian directory written as one line that
// names the little-endian one, which comes after it.
const TYPED = `template png

root PNG {
  tiff: switch order {
    'II' => TIFF {
      ifd: at(8 from window) tiff.Ifd.le
    }
    'MM' => same as case 'II', but big-endian
    _ => bytes[0]
  }
}

type tiff.Ifd.be = same as type tiff.Ifd.le, but big-endian

type tiff.Ifd.le = Ifd (contents entries) {
  count: u16le
  entries: Entry {
    tag: u16le
  }[count]
}
`;

test("a same-as type line points at the type it names, written below it", () => {
  const idx = indexTemplate(TYPED);
  const text = (n: number | null): string | null => line(TYPED, n);
  assert.deepEqual(
    idx.sameAs.map((s) => [text(s.line), text(s.target), s.order]),
    [
      ["    'MM' => same as case 'II', but big-endian", "    'II' => TIFF {", "big"],
      ["type tiff.Ifd.be = same as type tiff.Ifd.le, but big-endian", "type tiff.Ifd.le = Ifd (contents entries) {", "big"],
    ],
  );
  assert.equal(idx.alias.get("tiff.Ifd.be"), "tiff.Ifd.le");
});

test("a field inside a type written as a same-as line is found in the type it names", () => {
  const idx = indexTemplate(TYPED);
  const chain = [
    node("file", "PNG"),
    node("tiff", "TIFF"),
    node("ifd", "at → tiff.Ifd.be"),
    node("ifd", "Ifd"),
    node("entries", "Entry[]", true),
    node("[0] width", "Entry"),
    node("tag", "u16 be"),
  ];
  assert.equal(line(TYPED, findLine(idx, chain, () => null)), "    tag: u16le");
  assert.equal(line(TYPED, findLine(idx, chain.slice(0, 4).concat(node("count", "u16 be")), () => null)), "  count: u16le");
});

// Mach-O, whose little-endian files are read by an arm that names the
// big-endian one.
const MACHO = `template macho

root MachO

type MachO = switch peek(u32be) {
  0xfeedfacf => MachOFile {
    magic: u32be
    cpu_type: u32be enum CpuType {
      0x07 = x86
    }
    level: u8be
  }
  0xcffaedfe => same as case 0xfeedfacf, but little-endian
  _ => Other {
    magic: u32be
  }
}
`;

test("a field read in the other byte order from its line says which", () => {
  const idx = indexTemplate(MACHO);
  const at = (s: string): number => MACHO.split("\n").findIndex((l) => l.includes(s));
  const none = (): never[] => [];
  const magic = at("magic: u32be");
  assert.equal(readAs(idx, magic, { name: "magic", type: "u32 le" }, none), "little");
  // A big-endian file reads the line as it is written.
  assert.equal(readAs(idx, magic, { name: "magic", type: "u32 be" }, none), null);
  // An enumeration's type names no byte order, so the other fields say it.
  const kids = [
    { name: "magic", type: "u32 le" },
    { name: "cpu_type", type: "CpuType" },
    { name: "level", type: "u8" },
  ];
  assert.equal(readAs(idx, at("cpu_type: u32be"), { name: "cpu_type", type: "CpuType" }, () => kids), "little");
  // One byte has no byte order to differ in.
  assert.equal(readAs(idx, at("level: u8be"), { name: "level", type: "u8" }, () => kids), null);
  // A line no `but little-endian` line points at is read as written.
  const other = MACHO.split("\n").findIndex((l, i) => i > at("_ => Other") && l.includes("magic: u32be"));
  assert.equal(readAs(idx, other, { name: "magic", type: "u32 le" }, none), null);
});

test("a field in a type written out for its counterpart says the counterpart's byte order", () => {
  const idx = indexTemplate(TYPED);
  const tag = TYPED.split("\n").findIndex((l) => l.includes("tag: u16le"));
  assert.equal(readAs(idx, tag, { name: "tag", type: "u16 be" }, () => []), "big");
  assert.equal(readAs(idx, tag, { name: "tag", type: "u16 le" }, () => []), null);
  // findLineAt says which node of the chain the line is for.
  const chain = [node("file", "PNG"), node("tiff", "TIFF"), node("ifd", "at → tiff.Ifd.be"), node("ifd", "Ifd"), node("count", "u16 be")];
  const found = findLineAt(idx, chain, () => null);
  assert.equal(found?.depth, 4);
  assert.equal(line(TYPED, found?.line ?? null), "  count: u16le");
});

test("a field read through any key of a shared line is found in the arm it names", () => {
  const idx = indexTemplate(GROUPED);
  const n = findLine(idx, [node("file", "MachO"), node("command", "Signature"), node("offset", "u32 be")], () => null);
  assert.equal(line(GROUPED, n), "      offset: u32le");
});

// Arms that each write a structure of one name with the same fields, so only
// the value of the field the switch reads can tell them apart. The comments
// keep the arms from being written the same.
const ARMS = `template arms

root T {
  kind: u16be
  name: text[4] latin1
  body: switch kind {
    1 => Body {
      // one
      size: u8be
    }
    0x20 => Body {
      // thirty-two
      size: u8be
    }
    30, 38 => same as case 1
    _ => Body {
      // any other
      size: u8be
    }
  }
  named: match name {
    "abcd" => Named {
      // abcd
      v: u8be
    }
    'fmt ' => Named {
      // fmt
      v: u8be
    }
    _ => Named {
      // any other
      v: u8be
    }
  }
}
`;

test("a field inside an arm is found in the arm the switch's field chose", () => {
  const idx = indexTemplate(ARMS);
  const lines = ARMS.split("\n");
  const after = (s: string, field: string): number => {
    const from = lines.findIndex((l) => l.includes(s));
    return lines.findIndex((l, i) => i > from && l.includes(field));
  };
  const at = (kind: Partial<{ value: string; edit: string }>, name: string, leaf: "size" | "v"): number | null => {
    const kids = [
      { name: "kind", type: "u16 be", ...kind },
      { name: "name", type: "latin1", value: name },
      { name: "body", type: "Body" },
      { name: "named", type: "Named" },
    ];
    const chain = [node("file", "T"), leaf === "size" ? node("body", "Body") : node("named", "Named"), node(leaf, "u8")];
    return findLine(idx, chain, (depth) => (depth === 0 ? kids : null));
  };
  // A plain key, a hex key, a key on a shared line and the default arm.
  assert.equal(at({ value: "1" }, "", "size"), after("1 => Body", "size:"));
  assert.equal(at({ value: "32" }, "", "size"), after("0x20 => Body", "size:"));
  assert.equal(at({ value: "38" }, "", "size"), after("1 => Body", "size:"));
  assert.equal(at({ value: "7" }, "", "size"), after("_ => Body", "size:"));
  // An enumeration's value ends with its number; its edit text is the label.
  assert.equal(at({ value: "big (0x20)", edit: "big" }, "", "size"), after("0x20 => Body", "size:"));
  // A label that is a number itself is not the value's number.
  assert.equal(at({ value: "1 (0x20)", edit: "1" }, "", "size"), after("0x20 => Body", "size:"));
  // Quoted keys, spaces inside the quotes included.
  assert.equal(at({}, "abcd", "v"), after('"abcd" => Named', "v:"));
  assert.equal(at({}, "fmt ", "v"), after("'fmt ' => Named", "v:"));
  // An enumeration of four-character codes: its number is the key's bytes,
  // read either way round.
  assert.equal(at({}, "fmt, format (0x666d7420)", "v"), after("'fmt ' => Named", "v:"));
  assert.equal(at({}, "fmt, format (0x20746d66)", "v"), after("'fmt ' => Named", "v:"));
  // Compared as numbers, no key matching is certain, so the default arm.
  assert.equal(at({}, "zzzz, other (0x7a7a7a7a)", "v"), after("_ => Named", "v:"));
});

// The field a switch reads is in an enclosing structure, and a switch nested
// in another arm reads a field of the same name that is not that one.
const NESTED = `template nested

root N {
  kind: u8be
  entry: Entry {
    type: u8be
    value: switch kind {
      1 => Thing {
        type: u8be
        inner: switch type {
          1 => Leaf {
            x: u8be
          }
        }
      }
      2 => Leaf {
        // two
        x: u8be
      }
    }
  }
}
`;

test("a switch's field may be in an enclosing structure; a switch elsewhere is not compared", () => {
  const idx = indexTemplate(NESTED);
  const lines = NESTED.split("\n");
  const two = lines.findIndex((l, i) => i > lines.findIndex((m) => m.includes("// two")) && l.includes("x: u8be"));
  const kids = (depth: number): { name: string; type: string; value?: string }[] | null =>
    depth === 0 ? [{ name: "kind", type: "u8", value: "2" }, { name: "entry", type: "Entry" }] : depth === 1 ? [{ name: "type", type: "u8", value: "1" }, { name: "value", type: "Leaf" }] : null;
  assert.equal(findLine(idx, [node("file", "N"), node("entry", "Entry"), node("value", "Leaf"), node("x", "u8")], kids), two);
});

test("an arm is not chosen when the value cannot be compared with every key", () => {
  const idx = indexTemplate(ARMS);
  const kids = (kind: string, name: string): { name: string; type: string; value: string }[] => [
    { name: "kind", type: "u16 be", value: kind },
    { name: "name", type: "latin1", value: name },
  ];
  const size = [node("file", "T"), node("body", "Body"), node("size", "u8")];
  const v = [node("file", "T"), node("named", "Named"), node("v", "u8")];
  // A value with no number in it says nothing about numbered keys.
  assert.equal(line(ARMS, findLine(idx, size, (d) => (d === 0 ? kids("several", "") : null))), "  body: switch kind {");
  // Text that matches no quoted key may be text shown cut short, so it does
  // not choose the default arm either.
  assert.equal(line(ARMS, findLine(idx, v, (d) => (d === 0 ? kids("1", "fmt") : null))), "  named: match name {");
  assert.equal(line(ARMS, findLine(idx, v, (d) => (d === 0 ? kids("1", "zzzz") : null))), "  named: match name {");
});

// Two arms whose structures have the same fields at the first level, told
// apart one level further down, and two arms that open differently named
// structures in a field of the same name.
const DEEP = `template deep

root D {
  kind: u8be
  body: switch kind {
    1 => Body {
      head: Head {
        a: u8be
      }
    }
    2 => Body {
      head: Head {
        b: u8be
      }
    }
  }
  segment: switch kind {
    1 => Segment {
      length: u16be
      contents: sized(length - 2) App0 {
        x: u8be
      }
    }
    2 => Segment {
      length: u16be
      contents: sized(length - 2) App2 {
        x: u8be
      }
    }
  }
}
`;

test("same-named structures with the same fields are told apart further down", () => {
  const idx = indexTemplate(DEEP);
  const b = DEEP.split("\n").findIndex((l) => l.includes("b: u8be"));
  const chain = [node("file", "D"), node("body", "Body"), node("head", "Head"), node("b", "u8")];
  assert.equal(findLine(idx, chain, (depth) => (depth === 1 ? [node("head", "Head")] : null)), b);
  // And with no children known at all.
  assert.equal(findLine(idx, chain, () => null), b);
  // A field that opens a structure named as the child's type is.
  const seg = [node("file", "D"), node("segment", "Segment"), node("contents", "App2")];
  const segKids = (depth: number): ChainNode[] | null => (depth === 1 ? [node("length", "u16 be"), node("contents", "App2")] : null);
  assert.equal(line(DEEP, findLine(idx, seg, segKids)), "      contents: sized(length - 2) App2 {");
});

// Two arms that write the same structure word for word, and two that do not.
const TWICE = `template twice

root W {
  kind: u8be
  table: switch kind {
    2 => Symbol (unit symbol) {
      value: u32le
    }
    11 => Symbol (unit symbol) {
      value: u32le
    }
  }
  other: switch kind {
    2 => Symbol (unit symbol) {
      value: u32le
    }
    11 => Symbol (unit symbol) {
      value: u32be
    }
  }
}
`;

test("structures written the same are one structure; different ones are a real tie", () => {
  const idx = indexTemplate(TWICE);
  const same = findLine(idx, [node("file", "W"), node("table", "Symbol"), node("value", "u32 le")], () => null);
  assert.equal(same, TWICE.split("\n").findIndex((l) => l.includes("value: u32le")));
  // Not the same text: neither the children nor anything further down can
  // choose, so the enclosing field's line is kept.
  const tie = [node("file", "W"), node("other", "Symbol"), node("value", "u32")];
  assert.equal(line(TWICE, findLine(idx, tie, () => null)), "  other: switch kind {");
  assert.equal(line(TWICE, findLine(idx, tie, (d) => (d === 1 ? [node("value", "u32")] : null))), "  other: switch kind {");
});

// HDF5's addresses, whose value of all ones reads as the format's word for it.
const NAMED_UNSET = `template hdf5

root HDF5 {
  superblock: Superblock {
    base_address: u64le unset 18446744073709551615 "undefined"
    superblock_extension_address: u64le unset 18446744073709551615 "undefined"
  }
}
`;

test("a field whose unset value has a name is found, and the name is not a keyword", () => {
  const idx = indexTemplate(NAMED_UNSET);
  const chain = [node("file", "HDF5"), node("superblock", "Superblock"), node("superblock_extension_address", "u64 le")];
  assert.equal(line(NAMED_UNSET, findLine(idx, chain, () => null)), '    superblock_extension_address: u64le unset 18446744073709551615 "undefined"');
  const segs = tokenizeLine('    base_address: u64le unset 18446744073709551615 "undefined"', "field");
  assert.deepEqual(
    segs.filter((s) => s.cls !== "").map((s) => [s.cls, s.text]),
    [
      ["field", "base_address"],
      ["kw", "unset"],
    ],
  );
});

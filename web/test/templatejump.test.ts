// Clicking a field's line in the Template tab: which node of the file the
// search lands on, and why it lands on none. The trees are made by hand in the
// shape the core gives them: a list's elements are called `[0] ...`, a field
// placed by `at(...)` is a node holding a node of the same name, and a field
// `named by` another is called `body __TEXT`.

import { test } from "node:test";
import assert from "node:assert/strict";

import { indexTemplate } from "../src/templateindex.ts";
import { fieldLines, jumpTarget, missReason, routesTo, type JumpNode, type JumpTree } from "../src/templatejump.ts";

type Fake = { name: string; type: string; list?: boolean; absent?: boolean; value?: string; edit?: string; kids?: Fake[] };

/** A tree for `jumpTarget` made from nested objects. `asked` counts the
 *  nodes it was asked for. */
function fakeTree(root: Fake): JumpTree & { asked: number } {
  const at = (path: readonly number[]): Fake | null => {
    let n: Fake | undefined = root;
    for (const k of path) n = n?.kids?.[k];
    return n ?? null;
  };
  const one = (n: Fake): JumpNode => ({ name: n.name, type: n.type, list: n.list ?? false, childCount: n.kids?.length ?? 0, absent: n.absent ?? false, ...(n.value === undefined ? {} : { value: n.value }), ...(n.edit === undefined ? {} : { edit: n.edit }) });
  const t = {
    asked: 0,
    node: (path: readonly number[]) => {
      t.asked++;
      const n = at(path);
      return n === null ? null : one(n);
    },
    children: (path: readonly number[], from: number, to: number) => {
      const n = at(path);
      const ks = (n?.kids ?? []).slice(from, to);
      t.asked += ks.length;
      return n === null ? null : ks.map(one);
    },
    kids: (path: readonly number[]) => (at(path)?.kids ?? []).map((k) => ({ name: k.name, type: k.type, ...(k.value === undefined ? {} : { value: k.value }) })),
  };
  return t;
}

const lineOf = (text: string, s: string, after = 0): number => text.split("\n").findIndex((l, i) => i >= after && l.trim() === s);

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
    0 = uint8, 8 = string
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

/** One metadata entry; `string` says which arm of `Value` it takes. */
const entry = (k: number, string: boolean): Fake => ({
  name: `[${k}] key${k}`,
  type: "Metadata",
  kids: [
    { name: "key", type: "String", kids: [{ name: "len", type: "u64 le" }, { name: "text", type: "utf8" }] },
    { name: "value_type", type: "u32 le", value: string ? "string (8)" : "uint8 (0)" },
    string ? { name: "value", type: "String", kids: [{ name: "len", type: "u64 le" }, { name: "text", type: "utf8" }] } : { name: "value", type: "u8" },
  ],
});

const gguf = (entries: Fake[]): Fake => ({
  name: "file",
  type: "GGUF",
  kids: [
    { name: "magic", type: "magic" },
    { name: "metadata_count", type: "u64 le" },
    { name: "metadata", type: "Metadata[]", list: true, kids: entries },
  ],
});

test("a field's routes run from the root's field through every type that leads to it", () => {
  const idx = indexTemplate(GGUF);
  const len = lineOf(GGUF, "len: u64le");
  const routes = routesTo(idx, len, "GGUF");
  assert.deepEqual(routes, [[lineOf(GGUF, "metadata: Metadata[metadata_count]"), lineOf(GGUF, "key: String (contents text) {"), len]]);
  // The `len` in `type Value` is reached through `value: Value`.
  const len2 = lineOf(GGUF, "len: u64le", len + 1);
  assert.deepEqual(routesTo(idx, len2, "GGUF"), [[lineOf(GGUF, "metadata: Metadata[metadata_count]"), lineOf(GGUF, "value: Value"), len2]]);
});

test("a click goes to the first element that has the field, or to the cursor's element", () => {
  const idx = indexTemplate(GGUF);
  const tree = fakeTree(gguf([entry(0, false), entry(1, true), entry(2, true)]));
  const len = lineOf(GGUF, "len: u64le");
  assert.deepEqual(jumpTarget(idx, len, tree, null), { kind: "found", path: [2, 0, 0, 0], line: len });
  assert.deepEqual(jumpTarget(idx, len, tree, [2, 2, 0]), { kind: "found", path: [2, 2, 0, 0], line: len });
  // The `len` in `Value`'s string arm: element 0 takes the other arm.
  const len2 = lineOf(GGUF, "len: u64le", len + 1);
  assert.deepEqual(jumpTarget(idx, len2, tree, null), { kind: "found", path: [2, 1, 2, 0], line: len2 });
  assert.deepEqual(jumpTarget(idx, len2, tree, [2, 2, 1]), { kind: "found", path: [2, 2, 2, 0], line: len2 });
  // The cursor's element takes the other arm, so the first one that does not.
  assert.deepEqual(jumpTarget(idx, len2, tree, [2, 0, 1]), { kind: "found", path: [2, 1, 2, 0], line: len2 });
});

test("a field no element has is not in the file; past the bound the search says how far it got", () => {
  const idx = indexTemplate(GGUF);
  const len2 = lineOf(GGUF, "len: u64le", lineOf(GGUF, "len: u64le") + 1);
  assert.deepEqual(jumpTarget(idx, len2, fakeTree(gguf([entry(0, false), entry(1, false)])), null), { kind: "absent" });
  assert.deepEqual(jumpTarget(idx, len2, fakeTree(gguf([])), null), { kind: "absent" });
  // 300 entries, only the last one a string.
  const many = Array.from({ length: 300 }, (_, k) => entry(k, k === 299));
  const tree = fakeTree(gguf(many));
  assert.deepEqual(jumpTarget(idx, len2, tree, null, 100), { kind: "gaveUp", tried: 100, list: "metadata" });
  // Not every element is read on the way.
  assert.ok(tree.asked < 2000, `asked for ${tree.asked} nodes`);
  // With the cursor in it, that entry is tried first.
  assert.deepEqual(jumpTarget(idx, len2, tree, [2, 299], 100), { kind: "found", path: [2, 299, 2, 0], line: len2 });
  assert.deepEqual(jumpTarget(idx, len2, tree, null, 300), { kind: "found", path: [2, 299, 2, 0], line: len2 });
});

const SAME = `template t

root T {
  kind: u8be
  opt: optional(when kind == 9) u8be
  body: switch kind {
    1 => Body (named by size) {
      size: u8be
      at_end: at(size from file) Tail {
        v: u8be
      }
    }
    2 => same as case 1
    3 => Other {
      size: u16be
    }
  }
}
`;

const same = (kind: string): Fake => ({
  name: "file",
  type: "T",
  kids: [
    { name: "kind", type: "u8", value: kind },
    { name: "opt", type: "u8", absent: true },
    kind === "3"
      ? { name: "body", type: "Other", kids: [{ name: "size", type: "u16 be" }] }
      : {
          name: "body 7",
          type: "Body",
          kids: [
            { name: "size", type: "u8", value: "7" },
            { name: "at_end", type: "at → Tail", kids: [{ name: "at_end", type: "Tail", kids: [{ name: "v", type: "u8" }] }] },
          ],
        },
  ],
});

test("an arm written as `same as case` leads to the arm it names, through a named node and a placed one", () => {
  const idx = indexTemplate(SAME);
  const size = lineOf(SAME, "size: u8be");
  const v = lineOf(SAME, "v: u8be");
  for (const kind of ["1", "2"]) {
    assert.deepEqual(jumpTarget(idx, size, fakeTree(same(kind)), null), { kind: "found", path: [2, 0], line: size });
    assert.deepEqual(jumpTarget(idx, v, fakeTree(same(kind)), null), { kind: "found", path: [2, 1, 0, 0], line: v });
  }
  // The other arm has a `size` too, but it is not this line's.
  assert.deepEqual(jumpTarget(idx, size, fakeTree(same("3")), null), { kind: "absent" });
  assert.deepEqual(jumpTarget(idx, lineOf(SAME, "size: u16be"), fakeTree(same("3")), null), { kind: "found", path: [2, 0], line: lineOf(SAME, "size: u16be") });
});

test("an optional field the file left out is not in the file", () => {
  const idx = indexTemplate(SAME);
  assert.deepEqual(jumpTarget(idx, lineOf(SAME, "opt: optional(when kind == 9) u8be"), fakeTree(same("1")), null), { kind: "absent" });
});

test("a list of some other structure is passed over without counting against the bound", () => {
  const idx = indexTemplate(GGUF);
  const len2 = lineOf(GGUF, "len: u64le", lineOf(GGUF, "len: u64le") + 1);
  // Elements that are all one other type: taken to be all that type.
  const other = Array.from({ length: 500 }, (_, k): Fake => ({ name: `[${k}]`, type: "Insn" }));
  assert.deepEqual(jumpTarget(idx, len2, fakeTree(gguf(other)), null, 100), { kind: "absent" });
});

test("a tree not read that far yet is neither found nor absent", () => {
  const idx = indexTemplate(GGUF);
  const tree = fakeTree(gguf([entry(0, true)]));
  const waiting: JumpTree = { ...tree, children: (path, from, to) => (path.length === 1 ? "pending" : tree.children(path, from, to)) };
  assert.deepEqual(jumpTarget(idx, lineOf(GGUF, "len: u64le"), waiting, null), { kind: "pending" });
});

test("every line of a wrapped field statement leads to its first line; other lines lead nowhere", () => {
  const text = `template t

root T {
  long: optional(when a == 1 and b == 2 and c == 3)
      u8be
  e: u8be enum E {
    0 = zero
  }
}
`;
  const idx = indexTemplate(text);
  const at = fieldLines(idx);
  const long = lineOf(text, "long: optional(when a == 1 and b == 2 and c == 3)");
  assert.equal(at[long], long);
  assert.equal(at[long + 1], long);
  assert.equal(at[lineOf(text, "e: u8be enum E {")], lineOf(text, "e: u8be enum E {"));
  assert.equal(at[lineOf(text, "0 = zero")], -1);
  assert.equal(at[lineOf(text, "root T {")], -1);
});

test("a field in an arm the file does not take says what the switch read instead", () => {
  const idx = indexTemplate(SAME);
  const size = lineOf(SAME, "size: u8be");
  // Read once: the value, as the keys write it.
  assert.deepEqual(missReason(idx, size, fakeTree(same("3"))), { kind: "is", on: "kind", value: "3" });
  assert.deepEqual(missReason(idx, lineOf(SAME, "v: u8be"), fakeTree(same("3"))), { kind: "is", on: "kind", value: "3" });
  // A value no arm has, in a switch with no `_` arm.
  assert.deepEqual(missReason(idx, lineOf(SAME, "size: u16be"), fakeTree(same("7"))), { kind: "is", on: "kind", value: "7" });
  // Not in an arm: no reason to give.
  assert.equal(missReason(idx, lineOf(SAME, "opt: optional(when kind == 9) u8be"), fakeTree(same("1"))), null);
  // Read many times in a list: the arm's keys, through a `type X = switch`.
  const g = indexTemplate(GGUF);
  const len2 = lineOf(GGUF, "len: u64le", lineOf(GGUF, "len: u64le") + 1);
  assert.deepEqual(missReason(g, len2, fakeTree(gguf([entry(0, false), entry(1, false)]))), { kind: "none", on: "value_type", keys: ["8"] });
  // A list read only in part may hold the key further on.
  const many = Array.from({ length: 300 }, (_, k) => entry(k, false));
  assert.equal(missReason(g, len2, fakeTree(gguf(many)), 100), null);
});

const MACHO = `template t

root M {
  cputype: u32le enum CpuType {
    0x7 = x86, 0x1000007 = x86_64
  }
  body: switch cputype {
    0x7 => Small {
      a: u8be
    }
    _ => Big {
      b: u8be
    }
  }
  commands: Command[2]
}

type Command {
  cmd: u32le
  data: switch cmd {
    0x19, 0x1A => Segment {
      flavor: u8be
      inner: switch flavor {
        1 => One {
          x: u8be
        }
        2 => same as case 1
        3 => Three {
          y: u8be
        }
      }
    }
    _ => bytes[0]
  }
  tail: switch peek(u8be) {
    0 => Zero {
      z: u8be
    }
  }
}
`;

const macho = (cpu: string, cmds: Fake[], edit?: string): Fake => ({
  name: "file",
  type: "M",
  kids: [
    { name: "cputype", type: "u32 le", value: cpu, ...(edit === undefined ? {} : { edit }) },
    cpu === "x86 (0x7)" ? { name: "body", type: "Small", kids: [{ name: "a", type: "u8" }] } : { name: "body", type: "Big", kids: [{ name: "b", type: "u8" }] },
    { name: "commands", type: "Command[]", list: true, kids: cmds },
  ],
});

const command = (k: number, cmd: string, flavor?: string): Fake => ({
  name: `[${k}]`,
  type: "Command",
  kids: [
    { name: "cmd", type: "u32 le", value: cmd },
    flavor === undefined
      ? { name: "data", type: "bytes" }
      : {
          name: "data",
          type: "Segment",
          kids: [
            { name: "flavor", type: "u8", value: flavor },
            { name: "inner", type: flavor === "3" ? "Three" : "One", kids: [{ name: flavor === "3" ? "y" : "x", type: "u8" }] },
          ],
        },
  ],
});

test("the value is written in the base and case of the switch's keys", () => {
  const idx = indexTemplate(MACHO);
  const a = lineOf(MACHO, "a: u8be");
  assert.deepEqual(missReason(idx, a, fakeTree(macho("x86_64 (0x1000007)", []))), { kind: "is", on: "cputype", value: "0x1000007" });
  // An unnamed enum value is shown with its number in front.
  assert.deepEqual(missReason(idx, a, fakeTree(macho("75668 (unrecognised)", [], "75668"))), { kind: "is", on: "cputype", value: "0x12794" });
  // The default arm, when another arm was taken.
  assert.deepEqual(missReason(idx, lineOf(MACHO, "b: u8be"), fakeTree(macho("x86 (0x7)", []))), { kind: "is", on: "cputype", value: "0x7" });
  // Small keys in decimal and large ones in hex: written as the nearest key is.
  const MIXED = `template t

root M {
  cpu_type: u32le
  body: switch cpu_type {
    7 => Small {
      a: u8be
    }
    0x0100000c => Arm {
      b: u8be
    }
  }
}
`;
  const mixed = indexTemplate(MIXED);
  const cpu = (value: string): Fake => ({ name: "file", type: "M", kids: [{ name: "cpu_type", type: "u32 le", value }] });
  assert.deepEqual(missReason(mixed, lineOf(MIXED, "a: u8be"), fakeTree(cpu("16777223"))), { kind: "is", on: "cpu_type", value: "0x01000007" });
  assert.deepEqual(missReason(mixed, lineOf(MIXED, "b: u8be"), fakeTree(cpu("12"))), { kind: "is", on: "cpu_type", value: "12" });
  // Keys in a list: every key leading to the arm, as written.
  const flavor = lineOf(MACHO, "flavor: u8be");
  const tree = fakeTree(macho("x86 (0x7)", [command(0, "2"), command(1, "50")]));
  assert.deepEqual(missReason(idx, flavor, tree), { kind: "none", on: "cmd", keys: ["0x19", "0x1A"] });
});

test("the outermost arm no read takes is the reason; an arm some read takes is passed through", () => {
  const idx = indexTemplate(MACHO);
  const x = lineOf(MACHO, "x: u8be");
  const y = lineOf(MACHO, "y: u8be");
  // No segment at all: the outer arm is the reason.
  assert.deepEqual(missReason(idx, x, fakeTree(macho("x86 (0x7)", [command(0, "2"), command(1, "3")]))), { kind: "none", on: "cmd", keys: ["0x19", "0x1A"] });
  // A segment of flavor 3: the inner arm is, with the arm `same as` it.
  assert.deepEqual(missReason(idx, x, fakeTree(macho("x86 (0x7)", [command(0, "25", "3"), command(1, "3")]))), { kind: "none", on: "flavor", keys: ["1", "2"] });
  // Every arm taken somewhere: no reason to give.
  assert.equal(missReason(idx, y, fakeTree(macho("x86 (0x7)", [command(0, "25", "3"), command(1, "26", "1")]))), null);
  // A switch on an expression gives no reason.
  assert.equal(missReason(idx, lineOf(MACHO, "z: u8be"), fakeTree(macho("x86 (0x7)", [command(0, "2")]))), null);
});

const CHUNKS = `template t

root P {
  first: text[4] ascii
  head: switch first {
    'IHDR' => Head {
      h: u8be
    }
    _ => bytes[0]
  }
  chunks: Chunk[2]
}

type Chunk {
  type: text[4] ascii
  data: switch type {
    'IHDR' => IHDR {
      width: u32be
    }
    'tEXt' => tEXt {
      keyword: u8be
    }
    _ => bytes[0]
  }
}
`;

test("text keys: a whole quoted value no arm names still says which arm it is not", () => {
  const idx = indexTemplate(CHUNKS);
  const chunk = (k: number, type: string): Fake => ({ name: `[${k}] ${type}`, type: "Chunk", kids: [{ name: "type", type: "ascii", value: type }, { name: "data", type: "bytes" }] });
  const file = (first: string, types: string[]): Fake => ({
    name: "file",
    type: "P",
    kids: [{ name: "first", type: "ascii", value: first }, { name: "head", type: "bytes" }, { name: "chunks", type: "Chunk[]", list: true, kids: types.map((t, k) => chunk(k, t)) }],
  });
  const keyword = lineOf(CHUNKS, "keyword: u8be");
  assert.deepEqual(missReason(idx, keyword, fakeTree(file('"IEND"', ["IDAT", "IEND"]))), { kind: "none", on: "type", keys: ["'tEXt'"] });
  // Read once, the value is written in the keys' quotes.
  assert.deepEqual(missReason(idx, lineOf(CHUNKS, "h: u8be"), fakeTree(file('"IEND"', []))), { kind: "is", on: "first", value: "'IEND'" });
  // A value shown cut short may be the key.
  assert.equal(missReason(idx, lineOf(CHUNKS, "h: u8be"), fakeTree(file('"IE…"', []))), null);
});

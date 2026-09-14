// What the strip plan makes of a diagram, on the shapes the picture is really
// about: a run, an optional field, a choice, a choice of choices, a type read
// twice, and a type with more fields than fit.
//
// The plan is the half of the strip mode that can be checked without a page:
// which types get a strip, in what order, at what depth, and which box opens
// onto which. The other half is measurement, and only a browser can do that.
//
// The fixtures are written by hand rather than read out of a format, because
// the point is the shapes, and a real format has one of them buried in forty
// fields. What each stands for is said above it.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { DiagramBox, DiagramEdge, DiagramRow, TemplateDiagram } from "../src/doc.ts";
import { plan } from "../src/strips.ts";

function row(name: string, type_text: string, over: Partial<DiagramRow> = {}): DiagramRow {
  return { name, type_text, size_text: "", pos_text: "", list: false, kind: "number", ...over };
}

function box(name: string, rows: DiagramRow[], over: Partial<DiagramBox> = {}): DiagramBox {
  return { name, path: name, key: `k:${name}`, kind: "seq", rows, ...over };
}

function made(types: DiagramBox[], edges: DiagramEdge[]): TemplateDiagram {
  return { types, edges, omitted: 0 };
}

const all = (): boolean => true;

test("a run is the first of it, a band, and the last", () => {
  const d = made([box("PNG", [row("chunks", "Chunk[]", { size_text: "until type = 'IEND'", list: true })])], []);
  const items = plan(d, 24, all).strips[0]?.items ?? [];
  assert.deepEqual(
    items.map((i) => i.kind),
    ["first", "band", "last"],
  );
  // How many there are goes under the first, which is where a reader looking
  // for it looks. The last box says only that it is the last.
  assert.equal(items[0]?.size, "until type = 'IEND'");
  assert.equal(items[2]?.size, "");
});

test("a run is whatever the core says is a list, not whatever ends in brackets", () => {
  // A list placed by descriptors, whose type is written with an arrow, and a
  // run of bytes, whose type is written with brackets.
  const d = made(
    [box("Heap", [row("arrays", "descriptors → u8[]", { list: true }), row("data", "bytes[]", { size_text: "len bytes" })])],
    [],
  );
  const items = plan(d, 24, all).strips[0]?.items ?? [];
  assert.deepEqual(
    items.map((i) => `${i.kind} ${i.name}`),
    ["first arrays", "band ", "last arrays", "field data"],
  );
});

test("a field something decides is optional, whatever its type is called", () => {
  // The type prints as its own name here, the way a named type does, so the
  // only thing saying the field may not be there is the condition edge.
  const d = made(
    [box("Header", [row("flags", "u8"), row("extra", "Extra")])],
    [{ from: [0, 0], to: 0, to_row: 1, role: "condition", label: "flags & 4" }],
  );
  const items = plan(d, 24, all).strips[0]?.items ?? [];
  assert.equal(items[0]?.optional, false);
  assert.equal(items[1]?.optional, true);
  // What decides it, in the template's own words, under the name.
  assert.equal(items[1]?.size, "flags & 4");
});

test("a choice is one box with its cases in it, and a strip for each case", () => {
  const d = made(
    [
      box("Segment", [row("body", "switch on marker")]),
      box("switch on marker", [row("0xffc0", "Frame"), row("0xffc4", "Huffman")], { kind: "switch" }),
      box("Frame", [row("precision", "u8")]),
      box("Huffman", [row("class", "u8")]),
    ],
    [
      { from: [0, 0], to: 1, role: "type", label: "" },
      { from: [1, 0], to: 2, role: "case", label: "" },
      { from: [1, 1], to: 3, role: "case", label: "" },
    ],
  );
  const p = plan(d, 24, all);
  const item = p.strips[0]?.items[0];
  assert.deepEqual(item?.cases, ["0xffc0 → Frame", "0xffc4 → Huffman"]);
  // The switch itself gets no strip: a row of nothing but case labels between
  // a field and what it holds is a row that says nothing.
  assert.deepEqual(
    p.strips.map((s) => s.name),
    ["Segment", "Frame", "Huffman"],
  );
  assert.deepEqual(
    p.strips.map((s) => s.depth),
    [0, 1, 1],
  );
  assert.deepEqual(item?.links, [
    { strip: 1, reference: false },
    { strip: 2, reference: false },
  ]);
});

test("a type read twice is drawn once, and the second use is a mention of it", () => {
  const d = made(
    [box("Root", [row("a", "Part"), row("b", "Part")]), box("Part", [row("n", "u8")])],
    [
      { from: [0, 0], to: 1, role: "type", label: "" },
      { from: [0, 1], to: 1, role: "type", label: "" },
    ],
  );
  const p = plan(d, 24, all);
  assert.equal(p.strips.length, 2);
  assert.deepEqual(p.strips[0]?.items[0]?.links, [{ strip: 1, reference: false }]);
  assert.deepEqual(p.strips[0]?.items[1]?.links, [{ strip: 1, reference: true }]);
});

test("a long type is folded, and the end of it is still on the page", () => {
  const rows = Array.from({ length: 10 }, (_, i) => row(`f${i}`, "u8"));
  const items = plan(made([box("Wide", rows)], []), 4, all).strips[0]?.items ?? [];
  assert.deepEqual(
    items.map((i) => i.name),
    ["f0", "f1", "f2", "6", "f9"],
  );
  assert.equal(items[3]?.kind, "more");
});

test("a box the file has nothing of is left out when the toggle says so", () => {
  const d = made(
    [box("Root", [row("a", "Part")]), box("Part", [row("n", "u8")])],
    [{ from: [0, 0], to: 1, role: "type", label: "" }],
  );
  const p = plan(d, 24, (i) => i !== 1);
  assert.deepEqual(
    p.strips.map((s) => s.name),
    ["Root"],
  );
  assert.deepEqual(p.strips[0]?.items[0]?.links, []);
});

test("a choice of choices names the whole question, not the first half of it", () => {
  // ELF's shape: the class picks a choice on the endianness, which picks the
  // header. Drawn as written that is a row holding the word `1` and another
  // below it holding `2`; what a reader is after is the headers at the end.
  const d = made(
    [
      box("ELF", [row("header", "switch on class")]),
      box("switch on class", [row("1", "switch on data"), row("2", "switch on data")], { kind: "switch" }),
      box("switch on data", [row("1", "ELFHeader"), row("2", "ELFHeader")], { kind: "switch", key: "k:data32" }),
      box("switch on data", [row("1", "ELFHeader"), row("2", "ELFHeader")], { kind: "switch", key: "k:data64" }),
      box("ELFHeader", [row("entry", "u32 le")], { key: "k:h32le" }),
      box("ELFHeader", [row("entry", "u32 be")], { key: "k:h32be" }),
      box("ELFHeader", [row("entry", "u64 le")], { key: "k:h64le" }),
      box("ELFHeader", [row("entry", "u64 be")], { key: "k:h64be" }),
    ],
    [
      { from: [0, 0], to: 1, role: "type", label: "" },
      { from: [1, 0], to: 2, role: "case", label: "" },
      { from: [1, 1], to: 3, role: "case", label: "" },
      { from: [2, 0], to: 4, role: "case", label: "" },
      { from: [2, 1], to: 5, role: "case", label: "" },
      { from: [3, 0], to: 6, role: "case", label: "" },
      { from: [3, 1], to: 7, role: "case", label: "" },
    ],
  );
  const p = plan(d, 24, all);
  assert.deepEqual(p.strips[0]?.items[0]?.cases, [
    "1 · 1 → ELFHeader",
    "1 · 2 → ELFHeader",
    "2 · 1 → ELFHeader",
    "2 · 2 → ELFHeader",
  ]);
  // Four headers under the one box, and no strip for either question.
  assert.deepEqual(
    p.strips.map((s) => s.name),
    ["ELF", "ELFHeader", "ELFHeader", "ELFHeader", "ELFHeader"],
  );
  assert.deepEqual(
    p.strips.map((s) => s.depth),
    [0, 1, 1, 1, 1],
  );
});

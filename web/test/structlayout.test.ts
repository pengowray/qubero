// The struct layout figure's arithmetic (`src/viz/structlayoutmath.ts`): which
// children are members, the gaps between them and what each gap is called,
// and how members wrap onto rows. The shapes are the ones the test samples
// have: the ELF header with its fields reached by offset, a 56-byte program
// header, a CD sector with 2,048 bytes of data inside a structure of its own.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  buildRecord,
  COLLAPSE_ROWS,
  defaultRowWidth,
  fitKey,
  middleCut,
  coverGaps,
  namedPadding,
  misaligned,
  naturalAlign,
  offsetAlign,
  placedElsewhere,
  rowLabels,
  summarise,
  wrapRows,
  type Gathered,
  type NodeLike,
} from "../src/viz/structlayoutmath.ts";
import { monoMeasure } from "../src/viz/fit.ts";

let next = 0;
function n(name: string, kind: string, offsetBytes: number, sizeBytes: number, extra: Partial<NodeLike> = {}): NodeLike {
  return {
    path: [next++],
    name,
    type: kind === "composite" ? "S" : "u32 le",
    kind,
    value: "",
    offset_bits: offsetBytes * 8,
    size_bits: sizeBytes * 8,
    composite: kind === "composite",
    list: false,
    inline: false,
    child_count: 0,
    read_at: null,
    ...extra,
  };
}
const leaf = (node: NodeLike): Gathered => ({ node, kids: null });

const METRICS = { laneH: 20, breakH: 12, titleH: 18, blockGap: 6 };

test("the ELF header is its own 48 bytes, not the fields it reaches by offset", () => {
  // Fourteen fields from @0x10, then three `at →` pointers of no bytes and an
  // `offsets →` list as big as the file, all of which the core counts in the
  // header's size.
  const kids: Gathered[] = [
    leaf(n("type", "enum", 16, 2)),
    leaf(n("machine", "enum", 18, 2)),
    leaf(n("version", "uint", 20, 4)),
    leaf(n("entry", "uint", 24, 8)),
    leaf(n("program_header_offset", "uint", 32, 8)),
    leaf(n("section_header_offset", "uint", 40, 8)),
    leaf(n("flags", "uint", 48, 4)),
    leaf(n("header_size", "uint", 52, 2)),
    leaf(n("program_header_entry_size", "uint", 54, 2)),
    leaf(n("program_header_count", "uint", 56, 2)),
    leaf(n("section_header_entry_size", "uint", 58, 2)),
    leaf(n("section_header_count", "uint", 60, 2)),
    leaf(n("section_name_table", "uint", 62, 2)),
    leaf(n("section_name_base", "composite", 64, 0, { type: "at → u64 le", child_count: 1, read_at: 9049024 })),
    leaf(n("program_headers", "composite", 64, 0, { type: "at → ProgramHeader[]", child_count: 1, read_at: 512 })),
    leaf(n("sections", "composite", 64, 1131104, { type: "offsets → switch", list: true, child_count: 10 })),
  ];
  const header = n("header", "composite", 16, 1131152, { child_count: kids.length });
  const r = buildRecord(header, kids);
  assert.equal(r.startBit, 16 * 8);
  assert.equal(r.endBit, 64 * 8);
  assert.equal(r.placed.map((p) => p.name).join(","), "section_name_base,program_headers,sections");
  const s = summarise(r);
  assert.deepEqual(s, { bytes: 48, fields: 13, paddingBytes: 0, unreadBytes: 0, otherBytes: 0, gaps: 0, misaligned: 0 });
  // Eight bytes a row: six rows, and `entry` fills the second one whole.
  const rows = wrapRows([r], 8, METRICS, false);
  assert.equal(rows.lines.length, 6);
  const second = rows.lines[1];
  assert.ok(second !== undefined && second.kind === "row");
  assert.equal(second.cells.length, 1);
  assert.equal(second.cells[0]?.piece.kind === "field" && second.cells[0].piece.node.name, "entry");
});

test("pointers are told apart by their type or by where they were read", () => {
  assert.equal(placedElsewhere(n("a", "composite", 0, 0, { type: "at → X", read_at: 8 })), true);
  assert.equal(placedElsewhere(n("b", "composite", 0, 100, { type: "chain → X", list: true })), true);
  assert.equal(placedElsewhere(n("c", "composite", 0, 100, { type: "descriptors → X", list: true })), true);
  assert.equal(placedElsewhere(n("d", "uint", 0, 4)), false);
  assert.equal(placedElsewhere(n("e", "composite", 0, 4, { type: "attributes" })), false);
});

test("a gap before an aligned number is padding, any other gap is unread", () => {
  // u8 at +0, u32 at +4: three bytes a compiler would leave. Then a u16 at
  // +8 and a u32 at +16: six bytes that align nothing (+10 is not a multiple
  // of 4 short of +16 by less than 4), so unread.
  const s = n("s", "composite", 100, 20, { child_count: 4 });
  const r = buildRecord(s, [leaf(n("tag", "uint", 100, 1)), leaf(n("len", "uint", 104, 4)), leaf(n("kind", "uint", 108, 2)), leaf(n("crc", "uint", 116, 4))]);
  const gaps = r.pieces.filter((p) => p.kind !== "field");
  assert.deepEqual(
    gaps.map((g) => [g.kind, (g.startBit - r.startBit) / 8, (g.endBit - g.startBit) / 8]),
    [
      ["padding", 1, 3],
      ["unread", 10, 6],
    ],
  );
  const sum = summarise(r);
  assert.equal(sum.paddingBytes, 3);
  assert.equal(sum.unreadBytes, 6);
  assert.equal(sum.gaps, 2);
});

test("a gap at the end is padding when it rounds the record up to its widest member", () => {
  // u64 then u8, in a 16-byte record: seven bytes of tail padding.
  const s = n("s", "composite", 0, 16, { child_count: 2 });
  const r = buildRecord(s, [leaf(n("a", "uint", 0, 8)), leaf(n("b", "uint", 8, 1))]);
  const tail = r.pieces[r.pieces.length - 1];
  assert.ok(tail !== undefined && tail.kind === "padding");
  assert.equal((tail.endBit - tail.startBit) / 8, 7);
});

test("alignment is inferred from whole-byte numbers only", () => {
  const num = (kind: string, type: string, bits: number) => ({ kind, type, size_bits: bits });
  assert.equal(naturalAlign(num("uint", "u32 le", 32)), 4);
  assert.equal(naturalAlign(num("float", "f64 be", 64)), 8);
  assert.equal(naturalAlign(num("enum", "ObjectType", 16)), 2);
  assert.equal(naturalAlign(num("uint", "u24 le", 24)), null);
  assert.equal(naturalAlign(num("bytes", "bytes[]", 32)), null);
  assert.equal(naturalAlign(num("uint", "u8", 8)), null);
  // cpio's header fields are eight hex digits: text, with nothing to align.
  assert.equal(naturalAlign(num("int", "base-16 digits", 64)), null);
  assert.equal(offsetAlign(0), 8);
  assert.equal(offsetAlign(12), 4);
  assert.equal(offsetAlign(6), 2);
  assert.equal(misaligned(num("uint", "u64 le", 64), 12 * 8), true);
  assert.equal(misaligned(num("uint", "u64 le", 64), 16 * 8), false);
  assert.equal(misaligned(num("str", "ascii[]", 32), 3 * 8), false);
});

test("a structure inside the record is opened, and its long field folds into a break", () => {
  // A CD sector: sync, address and mode, then a body holding 2,048 bytes of
  // data and the error codes after it.
  const body: Gathered = {
    node: n("body", "composite", 16, 2336, { child_count: 5 }),
    kids: [
      leaf(n("data", "bytes", 16, 2048)),
      leaf(n("edc", "uint", 2064, 4)),
      leaf(n("reserved", "bytes", 2068, 8)),
      leaf(n("ecc_p", "bytes", 2076, 172)),
      leaf(n("ecc_q", "bytes", 2248, 104)),
    ],
  };
  const sector = n("[0]", "composite", 0, 2352, { child_count: 4 });
  const r = buildRecord(sector, [leaf(n("sync", "magic", 0, 12)), leaf(n("address", "bytes", 12, 3)), leaf(n("mode", "enum", 15, 1)), body]);
  assert.equal(r.endBit - r.startBit, 2352 * 8);
  assert.equal(r.groups.length, 1);
  assert.equal(r.groups[0]?.node.name, "body");
  assert.equal(summarise(r).fields, 8);
  assert.equal(defaultRowWidth(2352), 16);
  const rows = wrapRows([r], 16, METRICS, false);
  const breaks = rows.lines.filter((l) => l.kind === "break");
  // data's 128 rows are its first, a break for 126, and its last; ecc_p's
  // 172 bytes from +0x81c start partway into a row and fill nine whole rows
  // after it, which fold too; ecc_q's 104 bytes fill only five whole rows
  // after its first part, and fold as well.
  assert.equal(breaks.length, 3);
  const first = breaks[0];
  assert.ok(first !== undefined && first.kind === "break");
  assert.equal(first.rows, 126);
  assert.equal(first.piece.kind === "field" && first.piece.node.name, "data");
  // 147 rows of bytes in all, drawn as far fewer lines.
  const drawn = rows.lines.filter((l) => l.kind === "row").length;
  const folded = breaks.reduce((sum, b) => sum + (b.kind === "break" ? b.rows : 0), 0);
  assert.equal(drawn + folded, 147);
  assert.ok(rows.lines.length < 20);
});

test("a run shorter than the fold threshold stays as rows", () => {
  const s = n("s", "composite", 0, 8 * (COLLAPSE_ROWS - 1), { child_count: 1 });
  const r = buildRecord(s, [leaf(n("blob", "bytes", 0, 8 * (COLLAPSE_ROWS - 1)))]);
  const rows = wrapRows([r], 8, METRICS, false);
  assert.equal(rows.lines.every((l) => l.kind === "row"), true);
  assert.equal(rows.lines.length, COLLAPSE_ROWS - 1);
});

test("a u64 at 4 bytes a row takes two cells, and overlapping members take two lanes", () => {
  const s = n("s", "composite", 0, 8, { child_count: 2 });
  const r = buildRecord(s, [leaf(n("whole", "uint", 0, 8)), leaf(n("low", "uint", 0, 4))]);
  assert.equal(r.lanes, 2);
  const rows = wrapRows([r], 4, METRICS, false);
  assert.equal(rows.lines.length, 2);
  assert.equal(rows.lines[0]?.h, 2 * METRICS.laneH);
  const cellsOfWhole = rows.lines.flatMap((l) => (l.kind === "row" ? l.cells.filter((c) => c.piece.kind === "field" && c.piece.node.name === "whole") : []));
  assert.equal(cellsOfWhole.length, 2);
});

test("a list draws one titled block per element, each counted from its own start", () => {
  const elements = [0, 1, 2].map((i) => {
    const e = n(`[${i}]`, "composite", 64 + i * 56, 56, { child_count: 2 });
    return buildRecord(e, [leaf(n("type", "uint", 64 + i * 56, 4)), leaf(n("rest", "bytes", 68 + i * 56, 52))]);
  });
  const rows = wrapRows(elements, 8, METRICS, true);
  const titles = rows.lines.filter((l) => l.kind === "title");
  assert.equal(titles.length, 3);
  const firstRows = rows.lines.filter((l) => l.kind === "row" && l.rel === 0);
  assert.equal(firstRows.length, 3);
  // Seven rows each: `type` and the start of `rest`, then six rows wholly
  // `rest`, which fold to their first, a break for four, and their last.
  const perBlock = rows.lines.filter((l) => l.block === 1 && l.kind !== "title");
  assert.deepEqual(
    perBlock.map((l) => (l.kind === "break" ? `break ${l.rows}` : l.kind)),
    ["row", "row", "break 4", "row"],
  );
});

test("a name that does not fit is keyed by number, and the key fits its room", () => {
  const measure = monoMeasure();
  const s = n("s", "composite", 0, 8, { child_count: 3 });
  const r = buildRecord(s, [leaf(n("a_long_name", "uint", 0, 4)), leaf(n("b", "uint", 4, 2)), leaf(n("tiny_but_named", "uint", 6, 2))]);
  const rows = wrapRows([r], 8, METRICS, false);
  const line = rows.lines[0];
  assert.ok(line !== undefined && line.kind === "row");
  let number = 0;
  const numbers = new Map<number, number>();
  // 8 bytes over 120 pixels: 60 for the first, 30 for each of the others.
  const { labels, key } = rowLabels(
    line.cells,
    (c) => (c.x1 - c.x0) * 120,
    () => true,
    (p) => ({ full: p.kind === "field" ? p.node.name : "gap", short: null }),
    (i) => {
      if (!numbers.has(i)) numbers.set(i, ++number);
      return numbers.get(i) as number;
    },
    { size: 10, pad: 3, minChars: 4, measure },
  );
  const texts = line.cells.map((c) => labels.get(c)?.text ?? null);
  // 54 pixels inside the padding: eight characters and the ellipsis, four
  // from each end.
  assert.deepEqual(texts, ["a_lo…name", "b", "1"]);
  // A number is handed out only when it fits, so one that would not fit
  // leaves no hole in the count.
  let handed = 0;
  const squeezed = rowLabels(
    line.cells,
    (c) => (c.x1 - c.x0) * 120,
    () => true,
    (p) => ({ full: p.kind === "field" ? p.node.name : "gap", short: null }),
    (_i, fits) => (fits(String(handed + 1)) ? ++handed : null),
    { size: 10, pad: 3, minChars: 4, measure: (t, size) => (/^\d+$/.test(t) ? 1000 : measure(t, size)) },
  );
  assert.equal(handed, 0);
  assert.deepEqual(
    squeezed.key.map((k) => k.text),
    ["tiny_but_named"],
  );
  assert.deepEqual(
    key.map((k) => k.text),
    ["1 tiny_but_named"],
  );
  const more = (k: number): string => `+${k}`;
  const entries = [
    { text: "1 alpha", index: 0 },
    { text: "2 beta", index: 1 },
    { text: "3 gamma", index: 2 },
  ];
  // Each entry is 6 or 7 characters at 6 pixels: 42, 36 and 42, with 6 between.
  assert.deepEqual(fitKey(entries, 200, 6, 10, measure, more), { shown: entries, rest: 0 });
  const tight = fitKey(entries, 100, 6, 10, measure, more);
  assert.deepEqual(
    tight.shown.map((e) => e.index),
    [0],
  );
  assert.equal(tight.rest, 2);
});

test("names are cut in the middle, and two cut alike are keyed instead", () => {
  const measure = monoMeasure();
  // 60 pixels at 6 a character: ten characters, nine kept and the ellipsis.
  assert.equal(middleCut("linker_version_major", 60, 10, measure, 3), "linke…ajor");
  assert.equal(middleCut("short", 60, 10, measure, 3), "short");
  assert.equal(middleCut("abcdefgh", 12, 10, measure, 3), null);
  const s = n("s", "composite", 0, 4, { child_count: 2 });
  const r = buildRecord(s, [leaf(n("version_major_a", "uint", 0, 2)), leaf(n("version_major_b", "uint", 2, 2))]);
  const rows = wrapRows([r], 4, METRICS, false);
  const line = rows.lines[0];
  assert.ok(line !== undefined && line.kind === "row");
  let number = 0;
  // 66 pixels a cell, 58 inside the padding: the two cut to different texts,
  // so both stay in their boxes.
  const wide = rowLabels(line.cells, (c) => (c.x1 - c.x0) * 132, () => true, (p) => ({ full: p.kind === "field" ? p.node.name : "", short: null }), () => ++number, { size: 10, pad: 4, minChars: 3, measure });
  assert.deepEqual(
    line.cells.map((c) => wide.labels.get(c)?.keyed),
    [false, false],
  );
  const twins = buildRecord(s, [leaf(n("version_major_aa_x", "uint", 0, 2)), leaf(n("version_major_bb_x", "uint", 2, 2))]);
  const twinRow = wrapRows([twins], 4, METRICS, false).lines[0];
  assert.ok(twinRow !== undefined && twinRow.kind === "row");
  // 40 pixels a cell: both cut to the same text, so both are keyed.
  const narrow = rowLabels(twinRow.cells, (c) => (c.x1 - c.x0) * 80, () => true, (p) => ({ full: p.kind === "field" ? p.node.name : "", short: null }), () => ++number, { size: 10, pad: 4, minChars: 3, measure });
  assert.deepEqual(
    twinRow.cells.map((c) => narrow.labels.get(c)?.keyed),
    [true, true],
  );
  assert.equal(narrow.key.length, 2);
});

test("a field the format names as padding is counted as padding and stays a field", () => {
  // A cpio entry's name, the byte after it that brings the data to a
  // multiple of four, and the data.
  const e = n("[0] init", "composite", 0, 8, { child_count: 3 });
  const r = buildRecord(e, [leaf(n("name", "str", 0, 5)), leaf(n("name_padding", "bytes", 5, 1)), leaf(n("data", "bytes", 6, 2))]);
  const pad = r.pieces[1];
  assert.ok(pad !== undefined && pad.kind === "field" && pad.padding);
  assert.deepEqual(summarise(r), { bytes: 8, fields: 3, paddingBytes: 1, unreadBytes: 0, otherBytes: 0, gaps: 0, misaligned: 0 });
  assert.equal(namedPadding(n("pad2", "bytes", 0, 2)), true);
  assert.equal(namedPadding(n("reserved", "bytes", 0, 2)), false);
  assert.equal(namedPadding(n("padding", "uint", 0, 2)), false);
  assert.equal(namedPadding(n("keypad", "bytes", 0, 2)), false);
});

test("a record cut short ends at its last member given, with no gap after it", () => {
  const s = n("s", "composite", 0, 64, { child_count: 16 });
  const r = buildRecord(s, [leaf(n("a", "uint", 0, 4)), leaf(n("b", "uint", 4, 4))], true);
  assert.equal(r.endBit, 64);
  assert.equal(summarise(r).gaps, 0);
  const whole = buildRecord(s, [leaf(n("a", "uint", 0, 4)), leaf(n("b", "uint", 4, 4))]);
  assert.equal(summarise(whole).unreadBytes, 56);
});

test("a gap other fields are read from is called other, with what they belong to", () => {
  // ELF sections placed by offsets, with the program headers between the
  // list's start and its first section, and one byte nothing reads.
  const list = n("sections", "composite", 64, 400, { list: true, type: "offsets → switch", child_count: 2 });
  const r = buildRecord(list, [leaf(n("[1] .init", "bytes", 288, 27)), leaf(n("[2] .text", "bytes", 316, 148))]);
  const covered = coverGaps(r, (start, end) => (start === 64 * 8 && end === 288 * 8 ? [{ startBit: start, endBit: end, owner: "program_headers" }] : []));
  const gaps = covered.pieces.filter((p) => p.kind !== "field");
  assert.deepEqual(
    gaps.map((g) => [g.kind, (g.endBit - g.startBit) / 8, "owners" in g ? g.owners : []]),
    [
      ["other", 224, ["program_headers"]],
      ["unread", 1, []],
    ],
  );
  const s = summarise(covered);
  assert.equal(s.otherBytes, 224);
  assert.equal(s.unreadBytes, 1);
  // Not known yet: the gap stays as it was.
  assert.equal(coverGaps(r, () => null), r);
});

test("a gap read only in part is cut where the reading starts and stops", () => {
  // 40 bytes between two members: bytes 8 to 16 and 20 to 32 are read by
  // fields elsewhere (two of them overlapping, one running past the gap's
  // start), and the rest is read by nothing.
  const s = n("s", "composite", 0, 48, { child_count: 2 });
  const r = buildRecord(s, [leaf(n("a", "bytes", 0, 4)), leaf(n("b", "bytes", 44, 4))]);
  const covered = coverGaps(r, () => [
    { startBit: 0, endBit: 16 * 8, owner: "x" },
    { startBit: 20 * 8, endBit: 28 * 8, owner: "y" },
    { startBit: 24 * 8, endBit: 32 * 8, owner: "z" },
  ]);
  const gaps = covered.pieces.filter((p) => p.kind !== "field");
  assert.deepEqual(
    gaps.map((g) => [g.kind, g.startBit / 8, g.endBit / 8, "owners" in g ? g.owners : []]),
    [
      ["other", 4, 16, ["x"]],
      ["unread", 16, 20, []],
      ["other", 20, 32, ["y", "z"]],
      ["unread", 32, 44, []],
    ],
  );
  const sum = summarise(covered);
  assert.equal(sum.otherBytes, 24);
  assert.equal(sum.unreadBytes, 16);
  assert.equal(sum.bytes, 48);
  // Pieces stay in the order they start.
  const starts = covered.pieces.map((p) => p.startBit);
  assert.deepEqual(starts, [...starts].sort((x, y) => x - y));
});

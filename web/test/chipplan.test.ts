// What the annotation column says for the rows on screen: which fields are
// named on which row, when a run of list elements becomes one chip, where the
// chips of a row a heading has cut go, and what the strip pinned over the top
// of the rows carries.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { Span } from "../src/doc.ts";
import type { ChipMeasure } from "../src/chipfit.ts";
import {
  bucketChips,
  carriedName,
  chipLabel,
  chipText,
  continuedDetail,
  expandRuns,
  foldable,
  listName,
  pinnedNoteKey,
  firstVisibleByte,
  placeChips,
  planRowChips,
  rowNoteKey,
  sameList,
  valsBeforeChips,
  type Chip,
  type ChipBlock,
} from "../src/chipplan.ts";

/** A span with everything a chip reads, and the rest at its resting value. */
function span(o: Partial<Span> & { offset_bits: number; size_bits: number }): Span {
  return {
    path: [0],
    name: "field",
    trail: ["header"],
    type: "u8",
    value: "1",
    kind: "int",
    gap: false,
    count: 0,
    unit: null,
    line: null,
    sample: [],
    parts: [],
    bits: null,
    opens: false,
    ...o,
  };
}

/** An element of a list, at a byte. */
const element = (i: number, byte: number, o: Partial<Span> = {}): Span =>
  span({ name: `[${i}]`, trail: ["cell_pointers"], offset_bits: byte * 8, size_bits: 16, ...o });

/** One pixel a character, so a width in this file is a character count. */
const CHAR: ChipMeasure = { name: (s) => s.length, value: (s) => s.length };

// ----- what a chip stands for -----

test("an element of a list reads as one of many", () => {
  assert.equal(foldable(element(0, 0)), true);
});

test("text, a one-line structure, an already folded run and a gap each stay their own chip", () => {
  assert.equal(foldable(element(0, 0, { kind: "str" })), false);
  assert.equal(foldable(element(0, 0, { line: "push rbp" })), false);
  assert.equal(foldable(element(0, 0, { count: 8 })), false);
  assert.equal(foldable(element(0, 0, { gap: true })), false);
  // A named field is not an element of anything.
  assert.equal(foldable(span({ name: "page_size", offset_bits: 0, size_bits: 16 })), false);
});

test("two elements are of the same list only when they read the same way", () => {
  assert.equal(sameList(element(0, 0), element(1, 2)), true);
  assert.equal(sameList(element(0, 0), element(1, 2, { type: "u16" })), false);
  assert.equal(sameList(element(0, 0), element(1, 2, { trail: ["freeblocks"] })), false);
  assert.equal(sameList(element(0, 0), element(1, 2, { trail: ["a", "b"] })), false);
});

test("a list element is named for its list, a bare field for itself", () => {
  assert.equal(listName(element(3, 6)), "cell_pointers");
  assert.equal(listName(span({ name: "page_size", trail: [], offset_bits: 0, size_bits: 16 })), "page_size");
});

test("a folded run says how many, a gap says it is unmapped, a one-line structure is the line", () => {
  const run = { span: element(0, 0), carried: false, run: [element(0, 0), element(1, 2), element(2, 4)] };
  assert.deepEqual(chipText(run), { name: "cell_pointers", detail: "3 values" });
  assert.deepEqual(chipText({ span: span({ gap: true, offset_bits: 0, size_bits: 32 }), carried: false, run: [] }), {
    name: "unmapped",
    detail: "4 bytes",
  });
  assert.deepEqual(chipText({ span: element(0, 0, { line: "push rbp" }), carried: false, run: [] }), {
    name: "push rbp",
    detail: "",
  });
});

test("a carried chip is measured with the arrow its stylesheet draws", () => {
  const c: Chip = { span: element(0, 0), carried: true, run: [] };
  assert.equal(carriedName("page_size", c), "↑ page_size");
  assert.equal(carriedName("page_size", { ...c, carried: false }), "page_size");
  assert.equal(carriedName("page_size", undefined), "page_size");
});

test("a chip drawn above the bytes it names says the field runs on", () => {
  assert.equal(continuedDetail("4 bytes"), "4 bytes · continued");
  assert.equal(continuedDetail(""), "continued");
});

// ----- a folded run, opened out again -----

test("the elements of a folded run are drawn after the chip that folded them", () => {
  const elements = [element(0, 0), element(1, 2), element(2, 4)];
  const folded: Chip = { span: elements[0] as Span, carried: false, run: elements };
  const out = expandRuns([folded]);
  assert.equal(out.length, 4);
  assert.deepEqual(chipText(out[0] as Chip), { name: "cell_pointers", detail: "3 values" });
  // Each element is its own chip over its own bytes, so each is its own pick.
  assert.deepEqual(
    out.slice(1).map((c) => [c.span.path, c.element === true]),
    [
      [[0], true],
      [[0], true],
      [[0], true],
    ],
  );
});

test("an element chip says what it reads as and leaves the name to the chip in front", () => {
  const el: Chip = { span: element(1, 2, { value: "2049" }), carried: false, run: [], element: true };
  assert.deepEqual(chipText(el), { name: "", detail: "2049" });
  // Where a chip has to be named in words rather than drawn, it is its number.
  assert.equal(chipLabel(el), "[1]");
});

test("a chip that is not an element is untouched by the expansion", () => {
  const one: Chip = { span: span({ name: "page_size", offset_bits: 0, size_bits: 16 }), carried: false, run: [] };
  assert.deepEqual(expandRuns([one]), [one]);
});

test("an element carried in from above is named by the strip, and the rest of its run opens out below", () => {
  // The fold refuses to start on a chip carried in from above, and the top row
  // hands its carried chips to the pinned strip. What is left is the elements
  // that start on the row, folded among themselves and then opened out: the
  // same chips the row holds anywhere else, which is what keeps its height
  // from depending on where it falls.
  const spans = [element(0, 0), element(1, 2), element(2, 4)];
  const chips = placeChips(spans, 1, 16, 16, 2).byRow[0] as Chip[];
  const top = plan(chips, { top: true, rowStart: 1 });
  assert.equal(top.pinned?.entries.length, 1);
  assert.deepEqual((top.blocks[0] as { texts: { name: string; detail: string }[] }).texts, [
    { name: "cell_pointers", detail: "2 values" },
    { name: "", detail: "1" },
    { name: "", detail: "1" },
  ]);
});

// ----- where the spans land -----

test("a run of list elements on one row becomes one chip", () => {
  const spans = [element(0, 0), element(1, 2), element(2, 4)];
  const { byRow } = placeChips(spans, 0, 16, 16, 2);
  assert.equal((byRow[0] as Chip[]).length, 1);
  assert.equal((byRow[0] as Chip[])[0]?.run.length, 3);
});

test("elements of two different lists stay two chips", () => {
  const spans = [element(0, 0), element(1, 2, { trail: ["freeblocks"] })];
  const { byRow } = placeChips(spans, 0, 16, 16, 2);
  assert.equal((byRow[0] as Chip[]).length, 2);
});

test("text elements are worth reading one by one", () => {
  const spans = [element(0, 0, { kind: "str" }), element(1, 2, { kind: "str" })];
  const { byRow } = placeChips(spans, 0, 16, 16, 2);
  assert.equal((byRow[0] as Chip[]).length, 2);
});

test("a field is named on the row it starts on", () => {
  const spans = [span({ name: "later", offset_bits: 20 * 8, size_bits: 8 })];
  const { byRow } = placeChips(spans, 0, 32, 16, 2);
  assert.equal((byRow[0] as Chip[]).length, 0);
  assert.equal((byRow[1] as Chip[])[0]?.span.name, "later");
});

test("a field that started above the view is carried onto the first row, and stays its own chip", () => {
  // Two elements of one list, both starting before the window: neither folds
  // into the other, since the arrow is about where each began.
  const spans = [element(0, -4, { offset_bits: 0, size_bits: 200 * 8 }), element(1, 0, { offset_bits: 8, size_bits: 200 * 8 })];
  const { byRow } = placeChips(spans, 16, 32, 16, 2);
  const chips = byRow[0] as Chip[];
  assert.equal(chips.length, 2);
  assert.equal(chips.every((c) => c.carried), true);
});

test("every byte knows which span covers it, and an uncovered byte says so", () => {
  const spans = [span({ offset_bits: 8, size_bits: 16 })];
  const { byteSpan } = placeChips(spans, 0, 4, 16, 1);
  assert.deepEqual([...byteSpan], [-1, 0, 0, -1]);
});

// ----- a row a heading has cut -----

test("a chip goes with the piece of the row its bytes are in", () => {
  const chips: Chip[] = [
    { span: span({ name: "a", offset_bits: 0, size_bits: 8 }), carried: false, run: [] },
    { span: span({ name: "b", offset_bits: 5 * 8, size_bits: 8 }), carried: false, run: [] },
    { span: span({ name: "c", offset_bits: 9 * 8, size_bits: 8 }), carried: false, run: [] },
  ];
  const buckets = bucketChips(chips, [0, 4, 8], 0);
  assert.deepEqual(
    buckets.map((b) => b.map((c) => c.span.name)),
    [["a"], ["b"], ["c"]],
  );
});

test("a carried chip belongs to the front of the row, wherever its bytes start", () => {
  const chips: Chip[] = [{ span: span({ name: "long", offset_bits: 0, size_bits: 400 }), carried: true, run: [] }];
  const buckets = bucketChips(chips, [0, 4, 8], 16);
  assert.deepEqual(
    buckets.map((b) => b.length),
    [1, 0, 0],
  );
});

// ----- what one row's chips come to -----

const plan = (chips: Chip[], o: Partial<Parameters<typeof planRowChips>[0]> = {}) =>
  planRowChips({
    chips,
    segs: [0],
    rowStart: 0,
    top: false,
    noteWidth: 300,
    maxLines: Infinity,
    measure: CHAR,
    below: false,
    rowHeight: 24,
    chipLine: 22,
    ...o,
  });

test("only the top row carries anything, and what it carries goes to the pinned strip", () => {
  const carried: Chip = { span: span({ name: "payload", offset_bits: 0, size_bits: 800 }), carried: true, run: [] };
  const own: Chip = { span: span({ name: "own", offset_bits: 8 * 8, size_bits: 8 }), carried: false, run: [] };
  const top = plan([carried, own], { top: true });
  assert.equal(top.pinned?.entries.length, 1);
  assert.equal(top.pinned?.entries[0]?.span.name, "payload");
  // The row itself names only what starts on it.
  assert.deepEqual((top.blocks[0] as { entries: Chip[] }).entries.map((c) => c.span.name), ["own"]);

  const other = plan([carried, own]);
  assert.equal(other.pinned, null);
  assert.deepEqual((other.blocks[0] as { entries: Chip[] }).entries.map((c) => c.span.name), ["payload", "own"]);
});

test("carried chips the pinned strip cannot hold are named on the row rather than dropped", () => {
  const carried: Chip[] = Array.from({ length: 6 }, (_, i) => ({
    span: span({ name: `carried_field_${i}`, offset_bits: 0, size_bits: 800 }),
    carried: true,
    run: [],
  }));
  // One line of 40 characters holds one of these, so five are left over.
  const p = plan(carried, { top: true, noteWidth: 40, maxLines: 1 });
  assert.equal(p.pinned?.shown, 1);
  assert.equal(p.pinned?.entries.length, 6);
  assert.deepEqual(
    (p.blocks[0] as { entries: Chip[] }).entries.map((c) => c.span.name),
    ["carried_field_1", "carried_field_2", "carried_field_3", "carried_field_4", "carried_field_5"],
  );
});

test("a row's height never depends on whether it is the top row", () => {
  const carried: Chip = { span: span({ name: "payload", offset_bits: 0, size_bits: 800 }), carried: true, run: [] };
  // Wide enough that nothing is left over to fall back onto the row.
  const top = plan([carried], { top: true, noteWidth: 400 });
  assert.equal(top.extraHeight, 0);
  assert.equal((top.blocks[0] as { entries: Chip[] }).entries.length, 0);
});

test("a carried field whose last byte has scrolled off the top is not pinned", () => {
  // Row 0xb0, cut at byte 8 by the heading of the part that starts there. A
  // field that ended at 0xb1 is in the piece before the cut, so a scroll far
  // enough to take that piece away takes the field with it.
  const carried: Chip = {
    span: span({ name: "keyword", value: "Comment", offset_bits: 0xaa * 8, size_bits: 7 * 8 }),
    carried: true,
    run: [],
  };
  const cut = { top: true, segs: [0, 8], rowStart: 0xb0, headHeights: [0, 20] };
  // Square against the edge: the whole row is there, and byte 0xb0 with it.
  assert.equal(plan([carried], { ...cut, topPx: 0 }).pinned?.entries.length, 1);
  // Part way up: the first piece is still showing, and so is 0xb0.
  assert.equal(plan([carried], { ...cut, topPx: 20 }).pinned?.entries.length, 1);
  // Past the bottom of the first piece: the bytes it held are gone.
  assert.equal(plan([carried], { ...cut, topPx: 24 }).pinned?.entries.length, 0);
  // The same with the chips under the bytes, where the piece is taller than
  // the bytes in it but the bytes go at the same place.
  assert.equal(plan([carried], { ...cut, below: true, topPx: 20 }).pinned?.entries.length, 1);
  assert.equal(plan([carried], { ...cut, below: true, topPx: 24 }).pinned?.entries.length, 0);
});

test("a carried field that reaches past the cut is pinned however far the row has scrolled", () => {
  const carried: Chip = {
    span: span({ name: "data", offset_bits: 0xa0 * 8, size_bits: 0x40 * 8 }),
    carried: true,
    run: [],
  };
  const cut = { top: true, segs: [0, 8], rowStart: 0xb0, headHeights: [0, 20] };
  assert.equal(plan([carried], { ...cut, topPx: 60 }).pinned?.entries.length, 1);
});

test("a row that no heading cuts keeps its carried fields wherever the edge falls", () => {
  const carried: Chip = { span: span({ name: "payload", offset_bits: 0, size_bits: 800 }), carried: true, run: [] };
  assert.equal(plan([carried], { top: true, topPx: 0 }).pinned?.entries.length, 1);
  assert.equal(plan([carried], { top: true, topPx: 20 }).pinned?.entries.length, 1);
});

test("the first byte still on screen is the one the pieces above the edge do not hold", () => {
  // Two pieces of 24, with a 20-tall heading over the second.
  const at = (topPx: number): number => firstVisibleByte(0xb0, [0, 8], [0, 20], [24, 24], 24, topPx);
  assert.equal(at(0), 0xb0);
  assert.equal(at(23), 0xb0);
  assert.equal(at(24), 0xb8);
  // Inside the heading over the second piece, which is still on its way in.
  assert.equal(at(40), 0xb8);
  assert.equal(at(80), 0xb8);
});

test("chips hanging under a piece do not keep its bytes on screen", () => {
  // The bytes are 24 tall and the chips under them another 44, so the piece
  // is 68 in all. The bytes are gone at 24 whatever is still showing below.
  const at = (topPx: number): number => firstVisibleByte(0xb0, [0, 8], [0, 20], [68, 68], 24, topPx);
  assert.equal(at(23), 0xb0);
  assert.equal(at(24), 0xb8);
  assert.equal(at(60), 0xb8);
});

test("beside the bytes the chips share the row's own line; below them every line adds", () => {
  // Three chips of about 30 characters each in a 40-wide column: three lines.
  const chips: Chip[] = Array.from({ length: 3 }, (_, i) => ({
    span: span({ name: `wide_field_name_${i}`, offset_bits: i * 8, size_bits: 8, value: "12345" }),
    carried: false,
    run: [],
  }));
  const side = plan(chips, { noteWidth: 40 });
  const below = plan(chips, { noteWidth: 40, below: true });
  assert.equal(side.extraHeight, 3 * 22 - 24);
  assert.equal(below.extraHeight, 3 * 22);
});

test("a row with no chips adds nothing to its height", () => {
  assert.equal(plan([]).extraHeight, 0);
});

// ----- where the table of a folded run's values goes on the row -----

test("the values of a run that ends part way along a row go before the chips after it", () => {
  // `userblock-512.h5`: a run of 64 four-byte elements ends eight bytes into
  // the row at 0x1b60 and 52 bytes of compressed data start there. The table
  // holding 62 and 63 was drawn under `bytes 52 bytes`, so the column read
  // backwards: a chip over bytes further along the row, above the values of
  // the bytes before it.
  const after = plan([{ span: span({ name: "bytes", offset_bits: 8 * 8, size_bits: 52 * 8 }), carried: false, run: [] }]);
  assert.equal(valsBeforeChips(after.blocks[0], 8 * 8), true);
  // The row the run starts on: its chip names the run, so it comes first and
  // the table hangs under it, which is where it has always gone.
  const naming = plan([{ span: span({ name: "[0]", offset_bits: 0, size_bits: 4 * 8 }), carried: false, run: [] }]);
  assert.equal(valsBeforeChips(naming.blocks[0], 16 * 8), false);
  // A chip on each side of the run keeps the table last: between two chips it
  // would break them onto a line the row's height was not counted for.
  const both = plan([
    { span: span({ name: "count", offset_bits: 0, size_bits: 8 }), carried: false, run: [] },
    { span: span({ name: "bytes", offset_bits: 12 * 8, size_bits: 4 * 8 }), carried: false, run: [] },
  ]);
  assert.equal(valsBeforeChips(both.blocks[0], 12 * 8), false);
});

test("a row with no table, and one with no chips, leave the order alone", () => {
  const chips = plan([{ span: span({ name: "bytes", offset_bits: 8 * 8, size_bits: 8 }), carried: false, run: [] }]);
  assert.equal(valsBeforeChips(chips.blocks[0], null), false);
  assert.equal(valsBeforeChips(plan([]).blocks[0], 8 * 8), false);
  assert.equal(valsBeforeChips(undefined, 8 * 8), false);
});

// ----- the keys that say when the chips have to be written again -----

test("the key changes when a chip's name or value changes, and not otherwise", () => {
  const one = plan([{ span: span({ name: "page_size", offset_bits: 0, size_bits: 16, value: "4096" }), carried: false, run: [] }]);
  const same = plan([{ span: span({ name: "page_size", offset_bits: 0, size_bits: 16, value: "4096" }), carried: false, run: [] }]);
  const other = plan([{ span: span({ name: "page_size", offset_bits: 0, size_bits: 16, value: "8192" }), carried: false, run: [] }]);
  assert.equal(rowNoteKey(one.blocks, false), rowNoteKey(same.blocks, false));
  assert.notEqual(rowNoteKey(one.blocks, false), rowNoteKey(other.blocks, false));
  // The note that the column has stopped listing fields is part of the key.
  assert.notEqual(rowNoteKey(one.blocks, false), rowNoteKey(one.blocks, true));
});

test("two fields that read the same in different structures are not the same key", () => {
  // What a recycled element used to be handed: same name, same value, a
  // different field. The chip kept the tooltip and the press path of the one
  // it drew before.
  const at = (path: number[]): Chip => ({ span: span({ path, name: "length", offset_bits: 0, size_bits: 16, value: "64" }), carried: false, run: [] });
  assert.notEqual(rowNoteKey(plan([at([3, 0])]).blocks, false), rowNoteKey(plan([at([4, 0])]).blocks, false));
  const pinned = (path: number[]): ChipBlock | null =>
    plan([{ span: span({ path, name: "length", offset_bits: 0, size_bits: 800, value: "64" }), carried: true, run: [] }], { top: true }).pinned;
  assert.notEqual(pinnedNoteKey(pinned([3, 0])), pinnedNoteKey(pinned([4, 0])));
});

test("the key names the first field that did not fit, so a changed tooltip is redrawn", () => {
  const chips = (last: string): Chip[] => [
    { span: span({ name: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", offset_bits: 0, size_bits: 8 }), carried: false, run: [] },
    { span: span({ name: last, offset_bits: 8, size_bits: 8 }), carried: false, run: [] },
  ];
  const a = plan(chips("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"), { noteWidth: 40, maxLines: 1 });
  const b = plan(chips("cccccccccccccccccccccccccccccc"), { noteWidth: 40, maxLines: 1 });
  assert.equal(a.blocks[0]?.shown, 1);
  assert.notEqual(rowNoteKey(a.blocks, false), rowNoteKey(b.blocks, false));
});

test("a carried chip and the same chip drawn plainly are not the same key", () => {
  const s = span({ name: "payload", offset_bits: 0, size_bits: 800 });
  const carried = plan([{ span: s, carried: true, run: [] }]);
  const plain = plan([{ span: s, carried: false, run: [] }]);
  assert.notEqual(rowNoteKey(carried.blocks, false), rowNoteKey(plain.blocks, false));
});

test("an empty strip has an empty key", () => {
  assert.equal(pinnedNoteKey(null), "");
  assert.notEqual(pinnedNoteKey({ entries: [], texts: [], shown: 0 }), pinnedNoteKey(null));
});

test("the strip's key follows what it shows", () => {
  const carried = (name: string): Chip => ({ span: span({ name, offset_bits: 0, size_bits: 800 }), carried: true, run: [] });
  const a = plan([carried("payload")], { top: true }).pinned;
  const b = plan([carried("checksum")], { top: true }).pinned;
  assert.notEqual(pinnedNoteKey(a), pinnedNoteKey(b));
  assert.equal(pinnedNoteKey(a), pinnedNoteKey(plan([carried("payload")], { top: true }).pinned));
});

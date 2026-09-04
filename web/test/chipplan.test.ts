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
  chipText,
  continuedDetail,
  foldable,
  listName,
  pinnedNoteKey,
  placeChips,
  planRowChips,
  rowNoteKey,
  sameList,
  type Chip,
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
    line: null,
    sample: [],
    parts: [],
    bits: null,
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

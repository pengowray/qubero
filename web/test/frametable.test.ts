// A pandas frame read as a table. The cells are the core's answer; what is
// checked here is that the plan asks for the window it needs, keeps it, and
// gives the rows back in order. A plan that asked per row would be one call a
// row down a million-row frame.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { CellAt, Doc, FrameCell, TableShape, TemplateNode } from "../src/doc.ts";
import { computedCell, isTable, tablePlan } from "../src/tableplan.ts";
import { whereLines } from "../src/tableaddress.ts";
import { TABLE } from "../src/strings.ts";

function node(o: Partial<TemplateNode> & { name: string }): TemplateNode {
  return {
    path: [1],
    type: "object",
    offset_bits: 0,
    size_bits: 8,
    value: "",
    edit_text: "",
    kind: "composite",
    problems_within: [0, 0],
    child_count: 4,
    composite: true,
    list: false,
    editable: false,
    inline: false,
    value_bytes: 1,
    value_offset_bits: 0,
    read_at: null,
    read_as: null,
    consumed_by: null,
    machinery: null,
    contents: false,
    framed: false,
    space: 0,
    refused: null,
    decoded: false,
    space_root: false,
    joined: false,
    absent: false,
    line: null,
    table: true,
    ...o,
  } as TemplateNode;
}

const FRAME = node({ name: "data" });

/** A cell's address, in the tab's own bytes. */
function bytesAt(offset: number, size: number): CellAt {
  return { kind: "bytes", space: 0, offset_bits: offset, size_bits: size };
}

function shape(rows: number): TableShape {
  return {
    columns: null,
    names: ["index", "id", "score"],
    units: ["int64", "int64", "float64"],
    column_word: null,
    row_word: "row",
    rate: null,
    facts: [],
    cells: { kind: "computed", rows },
  };
}

/** A document whose frame has `rows` rows, counting the windows it is asked
 *  for so the test can see how many calls a walk costs. */
function doc(rows: number, asked: [number, number][]): Doc {
  return {
    tableShape: () => ({ status: "ok", node: shape(rows) }),
    pickleCells: (_path: readonly number[], from: number, to: number) => {
      asked.push([from, to]);
      const out: FrameCell[][] = [];
      for (let i = from; i < to; i += 1) {
        out.push([
          // A counted index, and then two columns in two blocks: the second
          // block is a thousand bits further on, so the row is not a run.
          { text: String(i), kind: "int", at: { kind: "counted" }, masked: false },
          { text: String(i * 2), kind: "int", at: bytesAt(i * 64, 64), masked: false },
          i % 3 === 0
            ? { text: "", kind: "absent", at: bytesAt(1000 + i * 64, 64), masked: false }
            : { text: `${i}.5`, kind: "float", at: bytesAt(1000 + i * 64, 64), masked: false },
        ]);
      }
      return { status: "ok", node: out };
    },
  } as unknown as Doc;
}

test("a frame is a table of its columns, with the index first", () => {
  const asked: [number, number][] = [];
  const plan = tablePlan(doc(4, asked), FRAME);
  assert.notEqual(plan, null);
  if (plan === null) return;
  assert.equal(plan.count, 4);
  assert.equal(plan.rowWord, "row");
  assert.deepEqual(
    plan.columns.map((c) => `${c.name} (${c.unit})`),
    ["index (int64)", "id (int64)", "score (float64)"],
  );
  assert.deepEqual(
    plan.row(1)?.cells.map((c) => c.text),
    ["1", "2", "1.5"],
  );
  // A value the frame has not got is empty, the way a record without one of
  // the table's keys is.
  assert.deepEqual(plan.row(0)?.cells.map((c) => [c.text, c.kind]), [
    ["0", "int"],
    ["0", "int"],
    ["", "absent"],
  ]);
});

test("the rows of one window are read in one call, and forgotten on request", () => {
  const asked: [number, number][] = [];
  const plan = tablePlan(doc(1000, asked), FRAME);
  assert.notEqual(plan, null);
  if (plan === null) return;
  for (let i = 0; i < 8; i += 1) plan.row(i);
  assert.deepEqual(asked, [[0, 256]]);
  // Past the window, one more call and no re-reading of the first.
  plan.row(300);
  plan.row(301);
  assert.deepEqual(asked, [
    [0, 256],
    [256, 512],
  ]);
  plan.forget();
  plan.row(0);
  assert.equal(asked.length, 3);
});

test("a frame's rows are not a run of bytes, so no bit is in one", () => {
  const plan = tablePlan(doc(4, []), FRAME);
  assert.equal(plan?.rowFor(0), null);
  assert.equal(isTable(doc(4, []), FRAME), true);
});

/** The same document with a table that names no columns, which is what a
 *  pickled array whose cells the core reads has: its columns are places along
 *  an axis rather than names anything wrote down. */
function unnamed(rows: number, wide: number): Doc {
  return {
    tableShape: () => ({
      status: "ok",
      node: { ...shape(rows), names: [], units: [], row_word: "row" },
    }),
    pickleCells: (_path: readonly number[], from: number, to: number) => {
      const out: FrameCell[][] = [];
      for (let i = from; i < to; i += 1) {
        out.push(
          Array.from({ length: wide }, (_, c) => ({
            text: String(i * wide + c),
            kind: "int" as const,
            at: bytesAt((i * wide + c) * 32, 32),
            masked: false,
          })),
        );
      }
      return { status: "ok", node: out };
    },
  } as unknown as Doc;
}

test("a computed table that names no columns heads them the way every other table does", () => {
  const plan = tablePlan(unnamed(4, 3), FRAME);
  assert.notEqual(plan, null);
  if (plan === null) return;
  assert.equal(plan.columns.length, 3);
  assert.deepEqual(
    plan.columns.map((c) => c.name),
    ["column 1", "column 2", "column 3"],
  );
  assert.deepEqual(plan.row(1)?.cells.map((c) => c.text), ["3", "4", "5"]);
  // And a table with no rows still has a column rather than none at all.
  const empty = tablePlan(unnamed(0, 3), FRAME);
  assert.equal(empty?.columns.length, 1);
});

test("a frame's cells carry their own addresses, and the row shows the first", () => {
  const plan = tablePlan(doc(4, []), FRAME);
  assert.notEqual(plan, null);
  if (plan === null) return;
  const row = plan.row(1);
  // The index was counted from a start and a step, so it has no bytes and
  // says which of the reasons that is.
  assert.equal(row?.cells[0]?.at, undefined);
  assert.equal(row?.cells[0]?.noBytes, "counted");
  assert.deepEqual(row?.cells[1]?.at, { space: 0, offsetBits: 64, sizeBits: 64 });
  assert.deepEqual(row?.cells[2]?.at, { space: 0, offsetBits: 1064, sizeBits: 64 });
  // The two columns are in two blocks, so the row is not a run of bytes: it
  // shows where its first cell is and says the rest are elsewhere.
  assert.equal(row?.offsetBits, 64);
  assert.equal(row?.space, 0);
  assert.equal(row?.apart, true);
});

test("a row whose cells are next to each other is one run, the way an ordinary row is", () => {
  const plan = tablePlan(unnamed(4, 3), FRAME);
  const row = plan?.row(1);
  // Elements 3, 4 and 5 of one array: three values of four bytes, in order.
  assert.equal(row?.offsetBits, 3 * 32);
  assert.equal(row?.sizeBits, 96);
  assert.equal(row?.apart, false);
});

test("a row with no cell anywhere has no offset to show", () => {
  const said: Doc = {
    tableShape: () => ({ status: "ok", node: { ...shape(2), names: ["dtype", "shape"], units: ["", ""] } }),
    pickleCells: () => ({
      status: "ok",
      node: [
        [
          { text: "float32", kind: "str", at: { kind: "nowhere" } },
          { text: "3 x 4", kind: "str", at: { kind: "nowhere" } },
        ],
      ],
    }),
  } as unknown as Doc;
  const row = tablePlan(said, FRAME)?.row(0);
  assert.equal(row?.offsetBits, 0);
  assert.equal(row?.sizeBits, 0);
  assert.equal(row?.cells[0]?.noBytes, "nowhere");
});

// A masked array's hidden entry is the other nothing: the cell shows no value
// although the bytes are there, so it keeps its address and the hover says
// which nothing it is. A cell with no bytes at all still says that instead.
test("a masked cell keeps its address and says it is masked", () => {
  const hidden = computedCell({ text: "", kind: "absent", at: bytesAt(64, 64), masked: true });
  assert.equal(hidden.masked, true);
  assert.deepEqual(hidden.at, { space: 0, offsetBits: 64, sizeBits: 64 });
  assert.equal(hidden.noBytes, undefined);
  assert.deepEqual(whereLines(hidden), [TABLE.maskedCell, TABLE.cellAt("@0x8", "8 bytes")]);

  const shown = computedCell({ text: "1.5", kind: "float", at: bytesAt(128, 64), masked: false });
  assert.equal(shown.masked, undefined);
  assert.deepEqual(whereLines(shown), [TABLE.cellAt("@0x10", "8 bytes")]);

  // And the reason a cell has no bytes at all is still the only line.
  const counted = computedCell({ text: "0", kind: "int", at: { kind: "counted" }, masked: false });
  assert.deepEqual(whereLines(counted), [TABLE.noBytesWhy("counted")]);
});

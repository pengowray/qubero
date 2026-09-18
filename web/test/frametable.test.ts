// A pandas frame read as a table. The cells are the core's answer; what is
// checked here is that the plan asks for the window it needs, keeps it, and
// gives the rows back in order. A plan that asked per row would be one call a
// row down a million-row frame.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { Doc, FrameCell, TableShape, TemplateNode } from "../src/doc.ts";
import { isTable, tablePlan } from "../src/tableplan.ts";

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
          { text: String(i), kind: "int" },
          { text: String(i * 2), kind: "int" },
          i % 3 === 0 ? { text: "", kind: "absent" } : { text: `${i}.5`, kind: "float" },
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

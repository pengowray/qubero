// The pickled table, built from what the core declared rather than worked out
// here. Getting a column wrong puts one key's values under another key's
// heading, and a row without a key has to leave a gap rather than shift the
// rest of its cells one column left.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { Doc, TableShape, TemplateNode } from "../src/doc.ts";
import { picklePlan } from "../src/picklerecords.ts";

/** A node as the core sends one, with the few fields this file reads set and
 *  the rest at something harmless. */
function node(o: Partial<TemplateNode> & { name: string }): TemplateNode {
  return {
    path: [],
    type: "u8",
    offset_bits: 0,
    size_bits: 8,
    value: "",
    edit_text: "",
    kind: "uint",
    problems_within: [0, 0],
    child_count: 0,
    composite: false,
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
    ...o,
  } as TemplateNode;
}

function shape(names: string[]): TableShape {
  return {
    columns: null,
    names,
    units: [],
    column_word: null,
    row_word: "row",
    rate: null,
    facts: [],
    cells: { kind: "named", row: "dict", cell: "entry", value: "value" },
  };
}

/**
 * A document holding one list of records, laid out the way the familiar form
 * places one: instructions among the values at every level, so the reader has
 * to pick the rows and the cells out by their type.
 *
 * `children` is keyed by the path joined with commas, which is enough for a
 * tree this small.
 */
function doc(names: string[], rows: [string, string][][]): Doc {
  const children = new Map<string, TemplateNode[]>();
  const under: TemplateNode[] = [node({ name: "empty_list", type: "bytes[]" })];
  rows.forEach((row, i) => {
    under.push(
      node({
        name: `[${i}]`,
        type: "dict",
        composite: true,
        child_count: row.length + 1,
        path: [1, i],
        offset_bits: i * 64,
        size_bits: 64,
      }),
    );
    const parts: TemplateNode[] = [node({ name: "empty_dict", type: "bytes[]" })];
    row.forEach(([key, value], j) => {
      parts.push(node({ name: key, type: "entry", composite: true, child_count: 2, path: [1, i, j] }));
      children.set(`1,${i},${j}`, [
        node({ name: "key", type: "utf8[]", value: key, kind: "str" }),
        node({ name: "value", type: "utf8[]", value, kind: "str" }),
      ]);
    });
    children.set(`1,${i}`, parts);
  });
  children.set("1", under);
  return {
    template: "picklefpf",
    tableShape: () => ({ status: "ok", node: shape(names) }),
    templateChildren: (path: readonly number[]) => ({ status: "ok", node: children.get(path.join(",")) ?? [] }),
  } as unknown as Doc;
}

const TABLE = node({ name: "data", type: "list", composite: true, child_count: 3, path: [1], table: true });

test("the columns are the core's, and a row without one leaves a gap", () => {
  const rows: [string, string][][] = [
    [
      ["id", "1"],
      ["name", "a"],
    ],
    [
      ["id", "2"],
      ["extra", "yes"],
    ],
  ];
  const plan = picklePlan(doc(["id", "name", "extra"], rows), TABLE);
  assert.notEqual(plan, null);
  const built = plan?.build();
  assert.notEqual(built, undefined);
  if (built === null || built === undefined) return;
  assert.deepEqual(built.columns, ["id", "name", "extra"]);
  assert.equal(built.rowWord, "row");
  assert.deepEqual(
    built.rows.map((r) => r.cells.map((c) => c.text)),
    [
      ["1", "a", ""],
      ["2", "", "yes"],
    ],
  );
  // The gap is a missing key rather than an unread one, and says so.
  assert.equal(built.rows[0]?.cells[2]?.kind, "absent");
});

test("a node the core did not declare a named-cell table on is not this reader's", () => {
  const plain = { ...doc([], []), tableShape: () => ({ status: "ok", node: { ...shape([]), cells: null } }) } as unknown as Doc;
  assert.equal(picklePlan(plain, TABLE), null);
  assert.equal(picklePlan(doc([], []), node({ name: "data", composite: true, child_count: 3, path: [1] })), null);
});

// Which steps of a path the inspector's trail folds into one crumb. A list and
// the element taken from it are one step whichever of the five kinds of list it
// is, so the fixtures below write their types the way the core does and say
// which are lists separately, as the core does.

import { test } from "node:test";
import assert from "node:assert/strict";

import { trailItems, type TrailNode } from "../src/trail.ts";

/** A composite node. Lists and structures differ only in `list`. */
function composite(name: string, type: string, list: boolean, child_count = 2, size_bits = 64): TrailNode {
  return { name, type, composite: true, list, size_bits, child_count, inline: false };
}

function leaf(name: string, type: string): TrailNode {
  return { name, type, composite: false, list: false, size_bits: 8, child_count: 0, inline: false };
}

/** Answers for the prefixes of one path, from a table keyed by the prefix. */
function lookup(nodes: Record<string, TrailNode>): (path: readonly number[]) => TrailNode | null {
  return (path) => nodes[path.join("/")] ?? null;
}

const labels = (path: number[], nodes: Record<string, TrailNode>): string[] => trailItems(path, lookup(nodes)).map((i) => i.label);

test("lists placed by descriptors fold into their element, as an array does", () => {
  // A Parquet page: row groups and their column chunks are placed by the
  // footer's records, and the pages under a chunk are laid end to end.
  const nodes = {
    "": composite("file", "Parquet", false, 5),
    "4": composite("row_groups", "descriptors → RowGroup", true, 1, 0),
    "4/0": composite("[0]", "RowGroup", false, 1),
    "4/0/0": composite("columns", "descriptors → ColumnChunk", true, 11),
    "4/0/0/3": composite("[3]", "ColumnChunk", false, 1),
    "4/0/0/3/0": composite("pages", "Page[]", true),
    "4/0/0/3/0/1": composite("[1] DATA_PAGE", "Page", false, 3),
    "4/0/0/3/0/1/0": composite("header", "PageHeader", false, 1),
    "4/0/0/3/0/1/0/0": leaf("hdr", "u8"),
  };
  assert.deepEqual(labels([4, 0, 0, 3, 0, 1, 0, 0], nodes), [
    "file (Parquet)",
    "row_groups[0]",
    "columns[3]",
    "pages[1]",
    "header (PageHeader)",
    "hdr",
  ]);
  // Each folded crumb goes to the element, not to the list.
  const items = trailItems([4, 0, 0, 3, 0, 1, 0, 0], lookup(nodes));
  assert.deepEqual(items[1]?.path, [4, 0]);
  assert.deepEqual(items[2]?.path, [4, 0, 0, 3]);
  assert.equal(items.at(-1)?.here, true);
});

test("a chain and a table of offsets fold the same way", () => {
  const nodes = {
    "": composite("file", "Hdf4", false, 4),
    "3": composite("blocks", "chain → Hdf4DescriptorBlock", true, 1, 0),
    "3/0": composite("[0]", "Hdf4DescriptorBlock", false, 3),
    "3/0/2": composite("entries", "offsets → Entry", true, 16, 0),
    "3/0/2/5": composite("[5]", "Entry", false, 2),
    "3/0/2/5/1": leaf("tag", "u16 be"),
  };
  assert.deepEqual(labels([3, 0, 2, 5, 1], nodes), ["file (Hdf4)", "blocks[0]", "entries[5]", "tag"]);
});

test("a structure is not folded, whatever its type is called", () => {
  const nodes = {
    "": composite("file", "Thing", false),
    "1": composite("body", "Body", false),
    "1/0": leaf("x", "u8"),
  };
  assert.deepEqual(labels([1, 0], nodes), ["file (Thing)", "body (Body)", "x"]);
});

test("a value written as several fields is named for the field, not for the structure", () => {
  // The structure is the template's way of writing one value in two fields,
  // and the name it gave it is a name no reader of the format has met.
  const nodes = {
    "": composite("file", "Tiff", false),
    "4": { ...composite("value", "Elsewhere", false), inline: true },
    "4/0": leaf("offset", "u32 be"),
  };
  assert.deepEqual(labels([4, 0], nodes), ["file (Tiff)", "value", "offset"]);
});

test("a list that is where the trail ends is a crumb of its own", () => {
  const nodes = {
    "": composite("file", "Thing", false),
    "0": composite("chunks", "descriptors → Chunk", true),
  };
  const items = trailItems([0], lookup(nodes));
  assert.deepEqual(
    items.map((i) => [i.label, i.here]),
    [
      ["file (Thing)", false],
      ["chunks (descriptors → Chunk)", true],
    ],
  );
});

test("a pointer and what it points at are one step, and a node not read yet is a question mark", () => {
  const nodes = {
    "": composite("file", "Elf", false),
    "2": composite("table", "at → Table", false, 1, 0),
    "2/0": composite("table", "Table", false, 2),
    "2/0/1": leaf("count", "u32 le"),
  };
  assert.deepEqual(labels([2, 0, 1], nodes), ["file (Elf)", "table (at → Table)", "count"]);
  assert.deepEqual(labels([2, 0, 1, 4], nodes), ["file (Elf)", "table (at → Table)", "count", "?"]);
});

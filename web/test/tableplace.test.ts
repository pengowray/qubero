// Which one of several tables a table is: a GUPPI raw file has a table of
// samples for every channel of every block.

import { test } from "node:test";
import assert from "node:assert/strict";

import { tablePlaces } from "../src/tableplace.ts";
import { TABLE } from "../src/strings.ts";
import { exportName } from "../src/tableexport.ts";
import type { TemplateNode } from "../src/doc.ts";

/** A node with only what the places read set. */
const node = (name: string, list = false, unit?: string, child_count = 0): TemplateNode =>
  ({ name, list, unit, child_count }) as TemplateNode;

const tree = (nodes: Record<string, TemplateNode>) => (path: readonly number[]): TemplateNode | null => nodes[path.join("/")] ?? null;

// file / blocks[1] / data / channels[2] / samples
const GUPPI = tree({
  "": node("GUPPI raw"),
  "0": node("blocks", true, "block", 128),
  "0/1": node("[1]"),
  "0/1/2": node("data"),
  "0/1/2/0": node("channels", true, "channel", 64),
  "0/1/2/0/2": node("[2]"),
  "0/1/2/0/2/0": node("samples", true, undefined, 32),
});

test("a table in counted lists names them as the inspector does, outermost first, from 0", () => {
  const places = tablePlaces([0, 1, 2, 0, 2, 0], GUPPI);
  assert.deepEqual(places, [
    { list: "blocks", index: 1, of: 128 },
    { list: "channels", index: 2, of: 64 },
  ]);
  assert.equal(TABLE.title("samples", places), "samples, blocks[1] (of 128), channels[2] (of 64)");
  assert.equal(TABLE.tabTitle("samples", "a.raw", places), "samples as table · blocks[1], channels[2] · a.raw");
  assert.equal(TABLE.tabTooltip("sample", "samples", "a.raw", places), "Table of samples from samples, blocks[1] (of 128), channels[2] (of 64), of a.raw");
  assert.equal(TABLE.exportTable("samples", places), "samples.blocks[1].channels[2]");
  assert.equal(exportName("a.raw", TABLE.exportTable("samples", places), "csv"), "a.raw.samples.blocks[1].channels[2].csv");
});

test("an index is written without digit grouping, and a count with it", () => {
  const places = [{ list: "rows", index: 1024, of: 4096 }];
  assert.equal(TABLE.title("samples", places), "samples, rows[1024] (of 4,096)");
  assert.equal(TABLE.exportTable("samples", places), "samples.rows[1024]");
});

test("a list of plain items is not a place, and a table in none keeps its name", () => {
  const plain = tree({ "": node("file"), "0": node("items", true, undefined, 9), "0/4": node("[4]"), "0/4/0": node("values") });
  assert.deepEqual(tablePlaces([0, 4, 0], plain), []);
  assert.equal(TABLE.title("values", []), "values");
  assert.equal(TABLE.tabTitle("values", "a.bin"), "values as table · a.bin");
});

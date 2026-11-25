// Which one of several tables a table is: a GUPPI raw file has a table of
// samples for every channel of every block.

import { test } from "node:test";
import assert from "node:assert/strict";

import { tablePlaces } from "../src/tableplace.ts";
import { TABLE } from "../src/strings.ts";
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

test("a table in counted lists names them, outermost first, from 1", () => {
  const places = tablePlaces([0, 1, 2, 0, 2, 0], GUPPI);
  assert.deepEqual(places, [
    { word: "block", n: 2, of: 128 },
    { word: "channel", n: 3, of: 64 },
  ]);
  assert.equal(TABLE.title("samples", places), "samples, block 2 of 128, channel 3 of 64");
  assert.equal(TABLE.tabTitle("samples", "a.raw", places), "samples as table · block 2, channel 3 · a.raw");
  assert.equal(TABLE.tabTooltip("sample", "samples", "a.raw", places), "Table of samples from samples, block 2 of 128, channel 3 of 64, of a.raw");
  assert.equal(TABLE.exportTable("samples", places), "samples.block2.channel3");
});

test("a list of plain items is not a place, and a table in none keeps its name", () => {
  const plain = tree({ "": node("file"), "0": node("items", true, undefined, 9), "0/4": node("[4]"), "0/4/0": node("values") });
  assert.deepEqual(tablePlaces([0, 4, 0], plain), []);
  assert.equal(TABLE.title("values", []), "values");
  assert.equal(TABLE.tabTitle("values", "a.bin"), "values as table · a.bin");
});

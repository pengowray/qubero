// Where a field sits inside the structures around it, as the inspector's
// Offset within rows say it.
//
// The tree below is the one a PDB type stream opened as a tab reads as: the
// stream, a field placed by an offset that names the place it is declared, the
// records it names, and a record's fields. Offsets are in bits, as the core
// gives them.

import { test } from "node:test";
import assert from "node:assert/strict";

import { INSIDE_LEVELS, withinGroup } from "../src/within.ts";
import type { PartGroup } from "../src/joinedpart.ts";
import type { TemplateNode } from "../src/doc.ts";

/** A node with only what the rows read set. */
const node = (name: string, offset_bits: number, size_bits: number, space = 0): TemplateNode =>
  ({ name, offset_bits, size_bits, space }) as TemplateNode;

/** The tree, by path. */
const tree = (nodes: Record<string, TemplateNode>) => (path: readonly number[]): TemplateNode | null => nodes[path.join("/")] ?? null;

const shown = (group: PartGroup | null): string[] => (group === null ? [] : [group.head, ...group.lines.map((l) => l.text)]);

const TPI = tree({
  "": node("stream", 0, 343744),
  "15": node("records", 448, 0),
  "15/0": node("records", 448, 343296),
  "15/0/1641": node("[1641]", 343616, 128),
  "15/0/1641/2": node("data", 343648, 96),
});

test("a field placed by an offset is not a structure the fields under it are within", () => {
  // The offset names where the field is declared, so the node of no bits and
  // the records it names start at the same place and have the same name. Only
  // the records hold anything, and only they get a row.
  const path = [15, 0, 1641, 2];
  const group = withinGroup(path, TPI(path)!, TPI);
  assert.deepEqual(shown(group), ["Offset within", "@+0x4 in [1641]", "@+0xa794 in records"]);
  assert.deepEqual(group?.lines.map((l) => l.path), [[15, 0, 1641], [15, 0]]);
});

test("nor is one whose offset names somewhere else", () => {
  const at = tree({
    "": node("header", 0, 64),
    "3": node("table", 32, 0),
    "3/0": node("table", 0x100 * 8, 0x20 * 8),
    "3/0/1": node("entry", 0x108 * 8, 16),
  });
  const path = [3, 0, 1];
  assert.deepEqual(shown(withinGroup(path, at(path)!, at)), ["Offset within", "@+0x8 in table"]);
});

test("each row says what its offset counts from and leads to that structure", () => {
  const path = [15, 0, 1641, 2];
  const group = withinGroup(path, TPI(path)!, TPI);
  assert.deepEqual(group?.lines.map((l) => l.plus), ["Offset within [1641]", "Offset within records"]);
  assert.ok(group?.lines.every((l) => l.place && l.title === null));
});

test("a structure in another space, at the start of the space, or not read gives no row", () => {
  const mixed = tree({
    "": node("file", 0, 8000),
    "1": node("stream", 800, 1600),
    "1/0": node("contents", 0, 4000, 1),
    "1/0/2": node("record", 64, 400, 1),
    "1/0/2/5": node("field", 96, 8, 1),
  });
  const path = [1, 0, 2, 5];
  // The record is within the stream's contents, which start at nought of the
  // space; the stream itself is at an offset of the file.
  assert.deepEqual(shown(withinGroup(path, mixed(path)!, mixed)), ["Offset within", "@+0x4 in record"]);
  const unread = (p: readonly number[]) => (p.length === 3 ? null : mixed(p));
  assert.equal(withinGroup(path, mixed(path)!, unread), null);
});

test("the nearest few structures, and a field of no bits at the very end of one", () => {
  const deep = tree({
    "": node("root", 0, 4096),
    "0": node("a", 8, 4000),
    "0/0": node("b", 16, 3000),
    "0/0/0": node("c", 24, 2000),
    "0/0/0/0": node("d", 32, 1000),
    "0/0/0/0/0": node("end", 1032, 0),
  });
  const path = [0, 0, 0, 0, 0];
  const group = withinGroup(path, deep(path)!, deep);
  assert.equal(group?.lines.length, INSIDE_LEVELS);
  assert.deepEqual(shown(group), ["Offset within", "@+0x7d in d", "@+0x7e in c", "@+0x7f in b"]);
});

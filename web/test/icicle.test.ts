// The position icicle's layout (`src/viz/iciclelayout.ts`) against trees made
// up here: bars sit where their bytes are, narrow neighbours fold, holes are
// gap bars, `contents` children are drawn one level up, other address spaces
// are left out, and big lists are read by window rather than whole.

import { test } from "node:test";
import assert from "node:assert/strict";

import { ALL_MAX, barAt, barFor, fieldLabel, IcicleCache, layoutIcicle, visibleKids, type IcicleField, type IcicleSource } from "../src/viz/iciclelayout.ts";

type Spec = { name: string; at: number; size: number; kids?: Spec[]; contents?: boolean; space?: number; framed?: boolean };

/** A field in bytes, to keep the trees short. */
function field(spec: Spec, path: number[]): IcicleField {
  return {
    path,
    name: spec.name,
    type: spec.kids === undefined ? "u8" : "Struct",
    kind: spec.kids === undefined ? "uint" : "composite",
    value: "",
    offset_bits: spec.at * 8,
    size_bits: spec.size * 8,
    child_count: spec.kids?.length ?? 0,
    composite: spec.kids !== undefined,
    contents: spec.contents === true,
    framed: spec.framed === true,
    list: false,
    space: spec.space ?? 0,
    problems_within: [0, 0],
  };
}

/** A source over a tree of specs, counting the reads it is asked for. */
function source(root: Spec): { src: IcicleSource; reads: { n: number }; root: IcicleField } {
  const reads = { n: 0 };
  const find = (path: readonly number[]): Spec => path.reduce((s, i) => (s.kids as Spec[])[i] as Spec, root);
  const src: IcicleSource = {
    children: (path, from, to) => {
      reads.n += 1;
      const s = find(path);
      return (s.kids ?? []).slice(from, to).map((k, i) => field(k, [...path, from + i]));
    },
  };
  return { src, reads, root: field(root, []) };
}

const names = (row: readonly { field?: IcicleField; kind: string; count?: number }[]): string[] =>
  row.map((b) => (b.kind === "field" ? (b.field as IcicleField).name : b.kind === "fold" ? `fold ${b.count}` : "gap"));

test("bars sit where their bytes are, one row per depth", () => {
  const { src, root } = source({ name: "file", at: 0, size: 100, kids: [{ name: "a", at: 0, size: 50 }, { name: "b", at: 50, size: 50 }] });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 800, width: 1000, rows: 4 });
  assert.deepEqual(names(l.rows[0] ?? []), ["file"]);
  assert.deepEqual(names(l.rows[1] ?? []), ["a", "b"]);
  const b = l.rows[1]?.[1];
  assert.equal(b?.x0, 500);
  assert.equal(b?.x1, 1000);
});

test("a hole between children is a gap bar, and a framed node has none", () => {
  const tree: Spec = { name: "file", at: 0, size: 100, kids: [{ name: "a", at: 0, size: 10 }, { name: "b", at: 60, size: 40 }] };
  const { src, root } = source(tree);
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 800, width: 1000, rows: 4 });
  assert.deepEqual(names(l.rows[1] ?? []), ["a", "gap", "b"]);
  const gap = l.rows[1]?.[1];
  assert.equal(gap?.startBit, 80);
  assert.equal(gap?.endBit, 480);
  const framed = source({ ...tree, framed: true });
  const f = layoutIcicle(framed.root, new IcicleCache(framed.src), { startBit: 0, endBit: 800, width: 1000, rows: 4 });
  assert.deepEqual(names(f.rows[1] ?? []), ["a", "b"]);
});

test("a template that stops short of the file leaves a gap on row 0", () => {
  const { src, root } = source({ name: "file", at: 0, size: 60, kids: [] });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 800, width: 1000, rows: 2, frame: { startBit: 0, endBit: 800 } });
  assert.deepEqual(names(l.rows[0] ?? []), ["file", "gap"]);
});

test("neighbours under two pixels fold into one bar that counts them", () => {
  const kids: Spec[] = [{ name: "head", at: 0, size: 500 }];
  for (let i = 0; i < 500; i++) kids.push({ name: `[${i}]`, at: 500 + i, size: 1 });
  const { src, root } = source({ name: "file", at: 0, size: 1000, kids });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 8000, width: 1000, rows: 3 });
  assert.deepEqual(names(l.rows[1] ?? []), ["head", "fold 500"]);
  // Zoomed in on the end, the same children are wide enough to draw.
  const z = layoutIcicle(root, new IcicleCache(src), { startBit: 990 * 8, endBit: 8000, width: 1000, rows: 3 });
  assert.deepEqual(names(z.rows[1] ?? []), ["[490]", "[491]", "[492]", "[493]", "[494]", "[495]", "[496]", "[497]", "[498]", "[499]"]);
});

test("children are read only once their parent is wide enough to show them", () => {
  const { src, reads, root } = source({ name: "file", at: 0, size: 10000, kids: [{ name: "big", at: 0, size: 9990, kids: [{ name: "x", at: 0, size: 1 }] }, { name: "small", at: 9990, size: 10, kids: [{ name: "y", at: 9990, size: 10 }] }] });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 80000, width: 1000, rows: 4 });
  // The root and `big` were read; `small` is one pixel wide and was not.
  assert.equal(reads.n, 2);
  const small = l.rows[1]?.find((b) => b.field?.name === "small");
  assert.equal(small?.more, true);
});

test("a contents child is drawn as its children, one level up", () => {
  const { src, root } = source({
    name: "file",
    at: 0,
    size: 16,
    kids: [
      { name: "size", at: 0, size: 4 },
      { name: "type", at: 4, size: 4 },
      { name: "body", at: 8, size: 8, contents: true, kids: [{ name: "inner", at: 8, size: 8 }] },
    ],
  });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 128, width: 1000, rows: 4 });
  assert.deepEqual(names(l.rows[1] ?? []), ["size", "type", "inner"]);
  assert.deepEqual(l.rows[1]?.[2]?.field?.path, [2, 0]);
});

test("children in another address space and children of no size are left out", () => {
  const { src, root } = source({ name: "file", at: 0, size: 10, kids: [{ name: "a", at: 0, size: 10 }, { name: "stream", at: 0, size: 20, space: 1 }, { name: "empty", at: 10, size: 0 }] });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 80, width: 1000, rows: 3 });
  assert.deepEqual(names(l.rows[1] ?? []), ["a"]);
});

test("children that overlap in the file take lanes of one row", () => {
  const { src, root } = source({ name: "file", at: 0, size: 10, kids: [{ name: "whole", at: 0, size: 10 }, { name: "part", at: 2, size: 4 }] });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 80, width: 1000, rows: 3 });
  const row = l.rows[1] ?? [];
  assert.deepEqual(row.map((b) => [b.field?.name, b.lane, b.lanes]), [["whole", 0, 2], ["part", 1, 2]]);
  // The lower lane is hit in the lower half of the row.
  assert.equal(barAt(l, 400, 20 + 15, 20)?.field?.name, "part");
  assert.equal(barAt(l, 400, 20 + 2, 20)?.field?.name, "whole");
});

test("a big list is searched by offset and only the window on screen is read", () => {
  const n = ALL_MAX * 10;
  const kids: Spec[] = [];
  for (let i = 0; i < n; i++) kids.push({ name: `[${i}]`, at: i * 4, size: 4 });
  const { src, reads, root } = source({ name: "file", at: 0, size: n * 4, kids });
  const cache = new IcicleCache(src);
  // Zoomed in to 100 children around the middle.
  const mid = (n / 2) * 4 * 8;
  const v = visibleKids(cache, root, mid, mid + 100 * 32);
  assert.ok(v !== null);
  assert.equal(v.fields.length, 100);
  assert.equal(v.fields[0]?.name, `[${n / 2}]`);
  assert.ok(reads.n < 60, `read ${reads.n} times`);
  // Zoomed out, the window holds them all and is one fold.
  const all = visibleKids(cache, root, 0, n * 32);
  assert.equal(all?.fold?.count, n);
});

test("the hit test reaches a one-pixel bar from 12 pixels away, on its own row only", () => {
  const { src, root } = source({ name: "file", at: 0, size: 1000, kids: [{ name: "tiny", at: 500, size: 2 }] });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 8000, width: 1000, rows: 3 });
  assert.equal(barAt(l, 510, 25, 20)?.field?.name, "tiny");
  assert.equal(barAt(l, 510, 5, 20)?.field?.name, "file");
  assert.equal(barAt(l, 400, 25, 20)?.kind, "gap");
});

test("the selected field lights its own bar, or the fold it is inside", () => {
  const kids: Spec[] = [{ name: "head", at: 0, size: 500 }];
  for (let i = 0; i < 500; i++) kids.push({ name: `[${i}]`, at: 500 + i, size: 1 });
  const { src, root } = source({ name: "file", at: 0, size: 1000, kids });
  const l = layoutIcicle(root, new IcicleCache(src), { startBit: 0, endBit: 8000, width: 1000, rows: 3 });
  assert.equal(barFor(l, [0], 0)?.field?.name, "head");
  assert.equal(barFor(l, [100], (500 + 99) * 8)?.kind, "fold");
});

test("a bare index is named with its type", () => {
  assert.equal(fieldLabel({ name: "[3]", type: "PaletteEntry" }), "[3] PaletteEntry");
  assert.equal(fieldLabel({ name: "[3]", type: "Box[]" }), "[3] Box");
  assert.equal(fieldLabel({ name: "moov", type: "Box" }), "moov");
});

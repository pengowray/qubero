// What a B-tree box's width stands for, on the four shapes HDF5 actually
// writes.
//
// The one rule the picture teaches is that a box's width is how many of the
// things the tree indexes are at or below it, and the dashed band under the
// last row counts the same things for the whole tree. So the root's weight and
// the band's count are one number arrived at two ways: down the tree, and out
// of the file. They disagreed for every version 2 tree, by the records the
// internal nodes hold, and nothing caught it because the weighting lived in a
// panel and could only be read off the screen.
//
// The fixtures are the shapes of four files written by h5py: a group of 4,000
// datasets and a 1000x1000 dataset in 50x50 chunks, each written twice, once
// with `libver="earliest"` for the version 1 trees and once with
// `libver="latest"` for the version 2 ones. Every count below was read out of
// those files rather than invented, so a fixture that stops matching HDF5 is a
// fixture someone changed on purpose.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { Tree, TreeNode } from "../src/doc.ts";
import { boxesOf, leafBand, leafCount, place, readoutLines, widthCaption, type Box } from "../src/btreedraw.ts";
import { BTREES } from "../src/strings.ts";

/** One node of a fixture, with its children. Written as a tree because that is
 *  the shape a reader checks against a file; the walk hands the panel the same
 *  nodes breadth first, and `build` does that flattening. */
type Spec = {
  entries: number;
  kind: TreeNode["kind"];
  /** HDF5's own level, counted up from the bottom row. */
  level: number;
  /** Children this node has and the walk did not reach, which makes every
   *  count at or above it a lower bound. */
  truncated?: boolean;
  kids?: Spec[];
};

type Shape = {
  version: number;
  job: Tree["job"];
  /** The version 2 header's `record_count`, which is the whole tree's total and
   *  not a sum of anything the walk did. Zero for a version 1 tree. */
  records_total?: number;
  record_type?: number;
  omitted?: number;
  root: Spec;
};

/** A fixture tree, breadth first with parent indices, the way the walk sends
 *  one. Addresses are made up and spaced apart: nothing under test reads them,
 *  but a strip drawn over nodes all at zero would be a picture of one column. */
function build(shape: Shape): Tree {
  const nodes: TreeNode[] = [];
  let queue: { spec: Spec; parent: number; depth: number }[] = [{ spec: shape.root, parent: -1, depth: 0 }];
  while (queue.length > 0) {
    const next: typeof queue = [];
    for (const { spec, parent, depth } of queue) {
      const here = nodes.length;
      nodes.push({
        path: [],
        parent,
        kind: spec.kind,
        sign: spec.kind === "links" ? "SNOD" : spec.kind === "leaf" ? "BTLF" : "TREE",
        address: 4096 + here * 64,
        size_bits: 512 * 8,
        level: spec.level,
        depth,
        entries: spec.entries,
        first_key: "",
        last_key: "",
        truncated: spec.truncated ?? false,
      });
      for (const kid of spec.kids ?? []) next.push({ spec: kid, parent: here, depth: depth + 1 });
    }
    queue = next;
  }
  return {
    job: shape.job,
    version: shape.version,
    record_type: shape.record_type ?? 0,
    record_type_name: "",
    records: "read",
    records_total: shape.records_total ?? 0,
    nodes,
    omitted: shape.omitted ?? 0,
    coords: 0,
    coords_pad: false,
  };
}

/** `n` copies of one node, for a row of a fixture whose nodes are all alike. */
function many(n: number, spec: Spec): Spec[] {
  return Array.from({ length: n }, () => ({ ...spec }));
}

/**
 * `tree-group.h5`: a group of 4,000 datasets under `libver="earliest"`.
 *
 * Four rows. The root points at two index nodes, which point at 36 between
 * them, which point at 999 symbol table nodes, which hold 4,000 links: 998
 * tables of four and one of eight. The odd row lengths are the file's, not a
 * rounding: HDF5 fills a node and splits it, so the last of each row is short.
 */
function groupTreeV1(): Tree {
  const tables = [...many(998, { entries: 4, kind: "links", level: 0 }), { entries: 8, kind: "links" as const, level: 0 }];
  let taken = 0;
  const take = (n: number): Spec[] => tables.slice(taken, (taken += n));
  // 35 index nodes of 28 tables each and one of 19, which is 999.
  const index = [...many(35, { entries: 28, kind: "index", level: 0 }), { entries: 19, kind: "index" as const, level: 0 }];
  for (const node of index) node.kids = take(node.entries);
  return build({
    version: 1,
    job: "group",
    root: {
      entries: 2,
      kind: "index",
      level: 2,
      kids: [
        { entries: 28, kind: "index", level: 1, kids: index.slice(0, 28) },
        { entries: 8, kind: "index", level: 1, kids: index.slice(28) },
      ],
    },
  });
}

/**
 * `tree-chunks.h5`: a 1000x1000 dataset in 50x50 chunks under
 * `libver="earliest"`, so 400 chunks.
 *
 * Two rows, and the bottom row is where a version 1 chunk tree stops: its
 * index nodes point at the chunks, which are blocks of data elsewhere in the
 * file and are not nodes of the tree.
 */
function chunkTreeV1(): Tree {
  return build({
    version: 1,
    job: "chunk",
    root: {
      entries: 7,
      kind: "index",
      level: 1,
      kids: [...many(6, { entries: 57, kind: "index", level: 0 }), { entries: 58, kind: "index", level: 0 }],
    },
  });
}

/** The leaf record counts of `tree-v2.h5`, in the file's own order: 24 leaves
 *  under the first internal node and 25 under the second. They come to 1,952,
 *  which is what the old weighting made of a tree of 2,000 records. */
const V2_LEAVES_A = [41, 42, 41, 44, 44, 44, 44, 33, 34, 35, 42, 45, 45, 44, 43, 45, 43, 43, 43, 44, 43, 41, 45, 23];
const V2_LEAVES_B = [22, 36, 41, 41, 43, 43, 41, 39, 32, 31, 32, 38, 42, 45, 45, 41, 38, 41, 38, 39, 42, 38, 42, 40, 31];

/**
 * `tree-v2.h5`: the same group of 2,000 datasets under `libver="latest"`, which
 * gets a version 2 B-tree of link name records (type 5).
 *
 * Three rows, and records on all three: the root holds 1, the two internal
 * nodes 23 and 24, the 49 leaves 1,952. A record sits between every two
 * children, so an internal node points at one more child than it holds
 * records, which is why the root's single record buys two children.
 */
function groupTreeV2(): Tree {
  const leaf = (entries: number): Spec => ({ entries, kind: "leaf", level: 0 });
  return build({
    version: 2,
    job: "group",
    record_type: 5,
    records_total: 2000,
    root: {
      entries: 1,
      kind: "index",
      level: 2,
      kids: [
        { entries: 23, kind: "index", level: 1, kids: V2_LEAVES_A.map(leaf) },
        { entries: 24, kind: "index", level: 1, kids: V2_LEAVES_B.map(leaf) },
      ],
    },
  });
}

/**
 * `tree-v2-chunkbt.h5`: the same 400 chunks under `libver="latest"`, with more
 * than one unlimited dimension so that HDF5 writes a version 2 B-tree of chunk
 * records (type 10) instead of a fixed array.
 *
 * Two rows: one internal node holding 4 records and pointing at 5 leaves, which
 * hold 396 between them.
 */
function chunkTreeV2(): Tree {
  const leaf = (entries: number): Spec => ({ entries, kind: "leaf", level: 0 });
  return build({
    version: 2,
    job: "chunk",
    record_type: 10,
    records_total: 400,
    root: { entries: 4, kind: "index", level: 1, kids: [84, 84, 84, 63, 81].map(leaf) },
  });
}

/** The sum of the bottom row's own counts, which is what the weighting used to
 *  make of every tree and is still the right answer for a version 1 one. */
function bottomRow(tree: Tree): number {
  const depth = Math.max(...tree.nodes.map((n) => n.depth));
  return tree.nodes.filter((n) => n.depth === depth).reduce((sum, n) => sum + n.entries, 0);
}

const WIDTH = 256;

test("a version 1 group tree's root weighs every link the band counts", () => {
  const tree = groupTreeV1();
  assert.equal(leafCount(tree), 4000);
  assert.equal(place(tree, WIDTH)[0]?.weight, 4000);
  // A version 1 index node's entries are pointers at nodes drawn on the next
  // row down, so they must add nothing of their own: counted, the root would
  // weigh 4,000 links plus the 1,037 pointers above them.
  assert.equal(bottomRow(tree), 4000);
});

test("a version 1 chunk tree's root weighs every chunk the band counts", () => {
  const tree = chunkTreeV1();
  assert.equal(leafCount(tree), 400);
  assert.equal(place(tree, WIDTH)[0]?.weight, 400);
  // The bottom row is index nodes here rather than link tables, and their
  // entries are chunks, which are not nodes. Same answer, reached the other way.
  assert.equal(bottomRow(tree), 400);
});

test("a version 2 tree's root weighs the records held above the leaves too", () => {
  const tree = groupTreeV2();
  // The header's `record_count`, which the band prints and does not derive.
  assert.equal(leafCount(tree), 2000);
  assert.equal(place(tree, WIDTH)[0]?.weight, 2000);
  // The defect, kept as a number: summing the leaves alone gives 1,952, and a
  // root box that wide sits under a band saying 2,000. The 48 missing records
  // are the ones the root and the two internal nodes hold themselves.
  assert.equal(bottomRow(tree), 1952);
  assert.notEqual(bottomRow(tree), leafCount(tree));
});

test("a version 2 chunk tree's root weighs its internal records too", () => {
  const tree = chunkTreeV2();
  assert.equal(leafCount(tree), 400);
  assert.equal(place(tree, WIDTH)[0]?.weight, 400);
  assert.equal(bottomRow(tree), 396);
});

test("every internal box weighs what is at or below it, not only its leaves", () => {
  const tree = groupTreeV2();
  const placed = place(tree, WIDTH);
  // The two internal nodes hold 23 and 24 records over leaves holding 991 and
  // 961, so they weigh 1,014 and 985, and those come to the root's 2,000 less
  // the one record the root holds itself.
  assert.equal(placed[1]?.weight, 23 + 991);
  assert.equal(placed[2]?.weight, 24 + 961);
  assert.equal((placed[1]?.weight ?? 0) + (placed[2]?.weight ?? 0) + 1, leafCount(tree));
});

test("a bottom-row box weighs its own count in both versions", () => {
  // Nothing is below these, so the weight is the node's own number and the
  // width is the one the number on the box is printed over.
  const v1 = groupTreeV1();
  const placedV1 = place(v1, WIDTH);
  for (const [i, node] of v1.nodes.entries()) {
    if (node.kind !== "links") continue;
    assert.equal(placedV1[i]?.weight, node.entries);
  }
  const v2 = groupTreeV2();
  const placedV2 = place(v2, WIDTH);
  for (const [i, node] of v2.nodes.entries()) {
    if (node.kind !== "leaf") continue;
    assert.equal(placedV2[i]?.weight, node.entries);
  }
});

test("children fill their parent's span exactly, so the picture stays a tree", () => {
  for (const tree of [groupTreeV1(), chunkTreeV1(), groupTreeV2(), chunkTreeV2()]) {
    const placed = place(tree, WIDTH);
    const kids = tree.nodes.map((): number[] => []);
    for (const [i, node] of tree.nodes.entries()) {
      if (node.parent >= 0) kids[node.parent]?.push(i);
    }
    for (const [i, own] of kids.entries()) {
      if (own.length === 0) continue;
      const here = placed[i];
      const first = placed[own[0] ?? -1];
      const last = placed[own[own.length - 1] ?? -1];
      assert.ok(here !== undefined && first !== undefined && last !== undefined);
      assert.equal(first.x, here.x);
      assert.ok(Math.abs(last.x + last.w - (here.x + here.w)) < 1e-9);
    }
  }
});

test("an empty node still gets a sliver, because an empty node is a fact", () => {
  const tree = build({
    version: 1,
    job: "group",
    root: { entries: 2, kind: "index", level: 1, kids: [{ entries: 0, kind: "links", level: 0 }, { entries: 4, kind: "links", level: 0 }] },
  });
  const placed = place(tree, WIDTH);
  assert.equal(placed[1]?.weight, 1);
  assert.ok((placed[1]?.w ?? 0) > 0);
});

test("a walk that stopped above the bottom row prints no band and no count", () => {
  // Index nodes whose children were never reached. What is under them was
  // never counted, so there is no honest number for a band; `omitted` is what
  // says so instead.
  const tree = build({
    version: 1,
    job: "group",
    omitted: 12,
    root: { entries: 2, kind: "index", level: 2, kids: many(2, { entries: 28, kind: "index", level: 1 }) },
  });
  assert.equal(leafCount(tree), null);
  assert.equal(leafBand(tree), null);
});

test("a capped version 2 walk weighs less than the band, and the band still stands", () => {
  // The one place the root's weight and the band's count are meant to differ.
  // The band prints the header's `record_count`, which is true of the whole
  // tree whatever the walk reached; the weight is what the walk actually
  // found. Making them agree here would mean either inventing records the
  // walk never saw or printing a total the file disagrees with, and
  // `omitted` is the line that accounts for the gap.
  const leaf = (entries: number): Spec => ({ entries, kind: "leaf", level: 0 });
  const tree = build({
    version: 2,
    job: "chunk",
    record_type: 10,
    records_total: 400,
    omitted: 3,
    // The core sets this wherever the cap cuts a node's children
    // (`hdf5_tree.rs`, the version 2 walk: `taken < pointers`), so a fixture
    // with `omitted` and no flag is a tree the walk could not have produced.
    root: { entries: 4, kind: "index", level: 1, truncated: true, kids: [84, 84].map(leaf) },
  });
  assert.equal(leafCount(tree), 400);
  assert.equal(place(tree, WIDTH)[0]?.weight, 4 + 168);
  assert.notEqual(leafBand(tree), null);
  // And the box says its count is a lower bound, so the reader is not left to
  // read 172 against a band printing 400 and take one of them for wrong.
  const box = boxesOf(tree, place(tree, WIDTH), WIDTH).find((b) => b.row === 0);
  assert.ok(box !== undefined);
  assert.ok(readoutLines(tree, box).includes(BTREES.widthStandsV2(4 + 168, false, true)));
});

test("the band says links or chunks in words, over the count that is weighed", () => {
  // The band's label and the root's weight are the same number, so a reader
  // comparing the widest box with the line under it is comparing one fact.
  assert.equal(leafBand(groupTreeV1())?.label, BTREES.leafLinks(4000, false));
  assert.equal(leafBand(chunkTreeV1())?.label, BTREES.leafChunks(400, false));
  assert.equal(leafBand(groupTreeV2())?.label, BTREES.leafLinksV2(2000));
  assert.equal(leafBand(chunkTreeV2())?.label, BTREES.leafChunksV2(400));
});

test("a box says what its own width stands for, in the tree's noun", () => {
  // The caption under the picture gives the rule once; this is the number for
  // the one box under the pointer, which is the only way a reader can hold the
  // rule against a box. It is the check that would have caught a root weighing
  // 1,952 under a band saying 2,000.
  const root = (tree: Tree): string[] => {
    const box = boxesOf(tree, place(tree, WIDTH), WIDTH).find((b) => b.row === 0);
    assert.ok(box !== undefined);
    return readoutLines(tree, box);
  };
  const g1 = groupTreeV1();
  assert.ok(root(g1).includes(BTREES.widthStands(4000, "link", false)));
  const c1 = chunkTreeV1();
  assert.ok(root(c1).includes(BTREES.widthStands(400, "chunk", false)));
  // The root's number and the band's are one quantity counted two ways, so a
  // reader comparing the widest box with the line under it compares one fact.
  const g2 = groupTreeV2();
  assert.ok(root(g2).includes(BTREES.widthStandsV2(leafCount(g2) ?? 0, false, false)));
  assert.equal(place(g2, WIDTH)[0]?.count, leafCount(g2));
  assert.equal(place(g1, WIDTH)[0]?.count, leafCount(g1));
});

test("a version 2 leaf's width stands for what is in it, and stops there", () => {
  // "And in the nodes below it" on a leaf sends the reader below it, where the
  // band prints the header's total, a different number.
  const tree = groupTreeV2();
  // Wide enough that a leaf gets a box of its own rather than being pooled
  // with its neighbours, which is what the line is about.
  const wide = 4000;
  const all = boxesOf(tree, place(tree, wide), wide);
  const last = Math.max(...all.map((b) => b.row));
  const box = all.find((b) => b.row === last && b.nodes.length === 1);
  assert.ok(box !== undefined);
  const node = tree.nodes[box.nodes[0] ?? -1];
  assert.equal(node?.kind, "leaf");
  assert.ok(readoutLines(tree, box).includes(BTREES.widthStandsV2(box.count, true, false)));
});

test("a count that is a lower bound says so, all the way up the tree", () => {
  // The flag is carried up here rather than read off the node, because the
  // core carries it up only where it settles a key range, which a version 2
  // group tree never does.
  const tree = build({
    version: 2,
    job: "group",
    records_total: 90,
    root: { entries: 2, kind: "index", level: 1, kids: [
      { entries: 44, kind: "leaf", level: 0 },
      { entries: 44, kind: "leaf", level: 0, truncated: true },
    ] },
  });
  const boxes = boxesOf(tree, place(tree, WIDTH), WIDTH);
  const root = boxes.find((b) => b.row === 0);
  assert.ok(root !== undefined);
  assert.equal(root.floor, true);
  assert.ok(readoutLines(tree, root).includes(BTREES.widthStandsV2(90, false, true)));
});

test("a box with no honest number to print says nothing about its width", () => {
  // An empty node is drawn a sliver wide so it can be pressed, and that 1 is a
  // fact about the picture. Printed against "holds 0 links" on the line above
  // it would be a fact about the file, and false.
  const empty = build({
    version: 1,
    job: "group",
    root: { entries: 2, kind: "index", level: 1, kids: [{ entries: 0, kind: "links", level: 0 }, { entries: 4, kind: "links", level: 0 }] },
  });
  const placed = place(empty, WIDTH);
  assert.equal(placed[1]?.weight, 1);
  assert.equal(placed[1]?.count, 0);
  const box = boxesOf(empty, placed, WIDTH).find((b) => b.nodes.length === 1 && b.nodes[0] === 1);
  assert.ok(box !== undefined);
  assert.ok(!readoutLines(empty, box).some((line) => line.startsWith("width stands for")));

  // And a version 1 node above the bottom row whose children were never
  // reached: its entries are nodes, not links, so there is no number of links
  // to print at all. `truncated` is the line that says what happened.
  const stopped = build({
    version: 1,
    job: "group",
    omitted: 12,
    root: { entries: 2, kind: "index", level: 2, kids: many(2, { entries: 28, kind: "index", level: 1 }) },
  });
  assert.equal(leafCount(stopped), null);
  assert.equal(place(stopped, WIDTH)[0]?.count, 0);
  const top = boxesOf(stopped, place(stopped, WIDTH), WIDTH).find((b) => b.row === 0);
  assert.ok(top !== undefined);
  assert.ok(!readoutLines(stopped, top).some((line) => line.startsWith("width stands for")));
});

test("a version 2 tree gets the caption about records, the others their own", () => {
  assert.equal(widthCaption(groupTreeV2()), BTREES.widthRecords);
  assert.equal(widthCaption(chunkTreeV2()), BTREES.widthRecords);
  assert.equal(widthCaption(groupTreeV1()), BTREES.widthGroup);
  assert.equal(widthCaption(chunkTreeV1()), BTREES.widthChunk);
});

test("a box carries the weight its width stands for, pooled boxes included", () => {
  // The number behind the pixels, so a box can say what its own width means
  // rather than leaving the caption under the picture to say it for all of
  // them at once. The root's is the whole tree's, which is the number that was
  // wrong for every version 2 tree.
  const tree = groupTreeV2();
  const placed = place(tree, WIDTH);
  const boxes = boxesOf(tree, placed, WIDTH);
  const root = boxes.find((b) => b.row === 0);
  assert.equal(root?.weight, placed[0]?.weight);
  assert.equal(root?.weight, leafCount(tree));
  // A pooled box stands for its nodes' weights added up, not for one of them.
  const pooled = boxesOf(groupTreeV1(), place(groupTreeV1(), WIDTH), WIDTH).find((b) => b.nodes.length > 1);
  assert.ok(pooled !== undefined);
  const sum = pooled.nodes.reduce((total, i) => total + (place(groupTreeV1(), WIDTH)[i]?.weight ?? 0), 0);
  assert.equal(pooled.weight, sum);
});

test("a row too wide for its boxes pools neighbouring siblings, and only those", () => {
  // 999 link tables across 256 pixels: most boxes have to stand for several,
  // and a box that stood for two nodes with different parents would sit across
  // a parent boundary and make the picture a lie.
  const tree = groupTreeV1();
  const boxes = boxesOf(tree, place(tree, WIDTH), WIDTH);
  const bottom = boxes.filter((b) => b.row === 3);
  assert.ok(bottom.length < 999);
  for (const box of bottom) {
    const parents = new Set(box.nodes.map((i) => tree.nodes[i]?.parent));
    assert.equal(parents.size, 1);
  }
  // Every node on the row is in exactly one box, so nothing is drawn twice and
  // nothing is dropped.
  assert.equal(bottom.reduce((sum, b) => sum + b.nodes.length, 0), 999);
});

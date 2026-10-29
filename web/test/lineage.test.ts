// The lineage breadcrumb's arithmetic (`viz/lineagemath.ts`): which streams a
// path goes through, what each is called, and which names fit in a tab.

import { test } from "node:test";
import assert from "node:assert/strict";

import { fitCrumbs, spaceSteps, type StepNode } from "../src/viz/lineagemath.ts";

/** A node with only what a step reads. */
function node(name: string, type: string, extra: Partial<StepNode> = {}): StepNode {
  return { name, type, offset_bits: 0, size_bits: 0, space_root: false, decoded: false, list: false, space: 0, ...extra };
}

/** A reading as a map from path to node. */
function reading(nodes: Record<string, StepNode>): (path: readonly number[]) => StepNode | null {
  return (p) => nodes[p.join(".")] ?? null;
}

// The shape a PNG's pixels have in the file's reading: the IDAT runs joined,
// the zlib payload inflated, the scanlines unfiltered.
const png = reading({
  "": node("file", "PNG", { size_bits: 159 * 8 }),
  "2": node("image", "joined → ZlibStream", { offset_bits: 159 * 8 }),
  "2.0": node("image", "ZlibStream", { space_root: true, size_bits: 24 * 8, space: 1 }),
  "2.0.6": node("compressed", "deflate", { decoded: true, offset_bits: 16, size_bits: 18 * 8, space: 1 }),
  "2.0.6.0": node("compressed", "Inflated", { space_root: true, size_bits: 48 * 8, space: 2 }),
  "2.0.6.0.0": node("scanlines", "png scanlines", { decoded: true, size_bits: 48 * 8, space: 2 }),
  "2.0.6.0.0.0": node("scanlines", "Unfiltered", { space_root: true, size_bits: 45 * 8, space: 4 }),
  "2.0.6.0.0.0.0": node("pixels", "raster", { size_bits: 45 * 8, space: 4 }),
});

test("a PNG pixel is inside three streams: joined, inflated, unfiltered", () => {
  const steps = spaceSteps([2, 0, 6, 0, 0, 0, 0], png);
  assert.ok(steps !== null);
  assert.deepEqual(
    steps.map((s) => [s.label, s.joined, s.codec, s.opener.join("."), s.root.join(".")]),
    [
      ["image", true, "joined → ZlibStream", "2", "2.0"],
      ["compressed", false, "deflate", "2.0.6", "2.0.6.0"],
      ["scanlines", false, "png scanlines", "2.0.6.0.0", "2.0.6.0.0.0"],
    ],
  );
  const inflate = steps[1];
  assert.equal(inflate?.inBits, 18 * 8);
  assert.equal(inflate?.outBits, 48 * 8);
  assert.equal(inflate?.atBits, 16);
});

test("only a run of the reading's own bytes is top level", () => {
  const steps = spaceSteps([2, 0, 6, 0, 0, 0], png) ?? [];
  assert.deepEqual(steps.map((s) => s.topLevel), [true, false, false]);
});

test("a field of the file outside every stream is inside none", () => {
  const steps = spaceSteps([], png);
  assert.deepEqual(steps, []);
});

test("the root of a tab over a stream is the tab, not a step", () => {
  // A tab over the inflated stream: its [] is the stream's root.
  const tab = reading({
    "": node("compressed", "Inflated", { space_root: true }),
    "0": node("scanlines", "png scanlines", { decoded: true }),
    "0.0": node("scanlines", "Unfiltered", { space_root: true }),
  });
  assert.deepEqual(spaceSteps([0, 0], tab)?.map((s) => s.label), ["scanlines"]);
});

test("a node still on its way makes the whole answer wait", () => {
  assert.equal(spaceSteps([2, 0, 9], png), null);
});

test("a zip entry's stream is called by the entry's name", () => {
  const zip = reading({
    "": node("file", "TorchZip"),
    "0": node("records", "ZipRecord[]", { list: true }),
    "0.0": node("[0] state-dict-zip/data.pkl", "ZipRecord"),
    "0.0.1": node("body", "LocalFile"),
    "0.0.1.14": node("data", "stored", { decoded: true, offset_bits: 64 * 8, size_bits: 319 * 8 }),
    "0.0.1.14.0": node("data", "DecodedText", { space_root: true, size_bits: 319 * 8, space: 1 }),
  });
  assert.deepEqual(spaceSteps([0, 0, 1, 14, 0], zip)?.map((s) => s.label), ["state-dict-zip/data.pkl"]);
});

test("a stream in an unnamed record is called by the record's place", () => {
  const gz = reading({
    "": node("file", "Gzip"),
    "0": node("members", "GzipMember[]", { list: true }),
    "0.3": node("[3]", "GzipMember"),
    "0.3.10": node("compressed", "deflate", { decoded: true }),
    "0.3.10.0": node("compressed", "Cut", { space_root: true }),
  });
  assert.deepEqual(spaceSteps([0, 3, 10, 0], gz)?.map((s) => s.label), ["members[3].compressed"]);
});

test("an element that is itself the run is called by its place alone", () => {
  const blocks = reading({
    "": node("file", "X"),
    "1": node("blocks", "Block[]", { list: true }),
    "1.4": node("[4]", "zlib", { decoded: true }),
    "1.4.0": node("[4]", "Inflated", { space_root: true }),
  });
  assert.deepEqual(spaceSteps([1, 4, 0], blocks)?.map((s) => s.label), ["blocks[4]"]);
});

test("a record name above an earlier stream does not name a later one", () => {
  // The entry's name belongs to the stream made in the file. The stream
  // inside that one is called by its own opener.
  const nested = reading({
    "": node("file", "Zip"),
    "0": node("records", "ZipRecord[]", { list: true }),
    "0.0": node("[0] pages.gz", "ZipRecord"),
    "0.0.1": node("data", "deflate", { decoded: true }),
    "0.0.1.0": node("data", "Gzip", { space_root: true }),
    "0.0.1.0.5": node("compressed", "deflate", { decoded: true }),
    "0.0.1.0.5.0": node("compressed", "Inflated", { space_root: true }),
  });
  assert.deepEqual(spaceSteps([0, 0, 1, 0, 5, 0], nested)?.map((s) => s.label), ["pages.gz", "compressed"]);
});

// ----- fitting names into a tab -----

const sep = 10;
const more = (): number => 8;

test("crumbs that fit are all drawn", () => {
  assert.deepEqual(fitCrumbs([40, 50, 60], 200, sep, more), [
    { kind: "crumb", index: 0, room: null },
    { kind: "crumb", index: 1, room: null },
    { kind: "crumb", index: 2, room: null },
  ]);
});

test("the middle goes behind one mark first, nearest the end put back first", () => {
  // 50 + 10 + 8 + 10 + 40 + 10 + 60 = 188: the first, a mark for 1 and 2,
  // and the last two. Putting 2 back as well would need 248.
  const fit = fitCrumbs([50, 50, 50, 40, 60], 200, sep, more);
  assert.deepEqual(fit, [
    { kind: "crumb", index: 0, room: null },
    { kind: "more", from: 1, to: 3 },
    { kind: "crumb", index: 3, room: null },
    { kind: "crumb", index: 4, room: null },
  ]);
});

test("when even the two ends do not fit, the last is cut to what is left", () => {
  const fit = fitCrumbs([120, 50, 150], 100, sep, more);
  // 100 - 8 (mark) - 10 (separator) = 82 for the last.
  assert.deepEqual(fit, [
    { kind: "more", from: 0, to: 2 },
    { kind: "crumb", index: 2, room: 82 },
  ]);
});

test("a single crumb too wide is cut to the room", () => {
  assert.deepEqual(fitCrumbs([300], 120, sep, more), [{ kind: "crumb", index: 0, room: 120 }]);
});

test("nothing to fit is nothing drawn", () => {
  assert.deepEqual(fitCrumbs([], 100, sep, more), []);
});

test("the drawn row is never wider than the room", () => {
  for (const room of [30, 60, 90, 120, 150, 200, 260]) {
    const widths = [70, 30, 90, 45, 55];
    const fit = fitCrumbs(widths, room, sep, more);
    let total = sep * (fit.length - 1);
    for (const f of fit) total += f.kind === "more" ? 8 : (f.room ?? (widths[f.index] as number));
    assert.ok(total <= room, `room ${room}: drawn ${total}`);
    assert.equal(fit[fit.length - 1]?.kind, "crumb", "the last crumb is always drawn");
  }
});

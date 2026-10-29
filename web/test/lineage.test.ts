// The lineage breadcrumb's arithmetic (`viz/lineagemath.ts`): which streams a
// path goes through, what each is called, and which names fit in a tab.

import { test } from "node:test";
import assert from "node:assert/strict";

import { cutMiddle, fitCrumbs, NAME_NEIGHBOURS, spaceSteps, type StepNode } from "../src/viz/lineagemath.ts";
import { fieldLineage, noteOpened, tabLineage, tabTitleText, titleCrumbs } from "../src/viz/lineage.ts";
import type { Doc } from "../src/doc.ts";

/** A node with only what a step reads. */
function node(name: string, type: string, extra: Partial<StepNode> = {}): StepNode {
  return { name, type, offset_bits: 0, size_bits: 0, space_root: false, decoded: false, list: false, space: 0, child_count: 0, ...extra };
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

// Four zTXt chunks between other chunks, as pngsuite's ctzn0g04 has them:
// each chunk's label is its type, so the type names none of the four.
function pngText(count: number, extra: Record<string, StepNode> = {}): (path: readonly number[]) => StepNode | null {
  const nodes: Record<string, StepNode> = {
    "": node("file", "PNG"),
    "1": node("chunks", "Chunk[]", { list: true, child_count: count }),
    ...extra,
  };
  for (let k = 0; k < count; k++) {
    const type = k >= 4 && k < 8 ? "zTXt" : k === 0 ? "IHDR" : k === count - 1 ? "IEND" : "tEXt";
    nodes[`1.${k}`] = node(`[${k}] ${type}`, "Chunk");
    nodes[`1.${k}.2`] = node("data", type);
    nodes[`1.${k}.2.2`] = node("text", "ZlibStream");
    nodes[`1.${k}.2.2.6`] = node("compressed", "deflate", { decoded: true });
    nodes[`1.${k}.2.2.6.0`] = node("compressed", "Text", { space_root: true });
  }
  return reading(nodes);
}

test("records that share a name are called by their place", () => {
  const png = pngText(10);
  const labels = [4, 5, 6, 7].map((k) => spaceSteps([1, k, 2, 2, 6, 0], png)?.[0]?.label);
  assert.deepEqual(labels, ["chunks[4].compressed", "chunks[5].compressed", "chunks[6].compressed", "chunks[7].compressed"]);
  assert.equal(new Set(labels).size, 4, "four streams, four names");
});

test("a name repeated by a record with no stream still names the stream", () => {
  // A zip's central directory names each entry again, next to the entries.
  const zip = reading({
    "": node("file", "Zip"),
    "0": node("records", "ZipRecord[]", { list: true, child_count: 2 }),
    "0.0": node("[0] state-dict-zip/data.pkl", "ZipRecord"),
    "0.0.1": node("body", "LocalFile"),
    "0.0.1.14": node("data", "stored", { decoded: true }),
    "0.0.1.14.0": node("data", "DecodedText", { space_root: true }),
    "0.1": node("[1] state-dict-zip/data.pkl", "ZipRecord"),
    "0.1.1": node("body", "CentralDirectoryEntry"),
    "0.1.1.14": node("comment", "bytes[]"),
  });
  assert.equal(spaceSteps([0, 0, 1, 14, 0], zip)?.[0]?.label, "state-dict-zip/data.pkl");
});

test("a record whose name no neighbour shares keeps its name", () => {
  // The IHDR chunk is the only one of its type.
  assert.equal(spaceSteps([1, 0, 2, 2, 6, 0], pngText(10))?.[0]?.label, "IHDR");
});

test("a name shared only further away than the neighbours looked at still counts as a name", () => {
  // The last chunk is IEND. Chunk 1 is called IEND too, further from the
  // last than the neighbours looked at.
  const count = NAME_NEIGHBOURS * 2 + 12;
  const png = pngText(count);
  const withFar = (p: readonly number[]): StepNode | null => (p.join(".") === "1.1" ? node("[1] IEND", "Chunk") : png(p));
  assert.ok(count - 2 > NAME_NEIGHBOURS);
  assert.equal(spaceSteps([1, count - 1, 2, 2, 6, 0], withFar)?.[0]?.label, "IEND");
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

// ----- cutting a name -----

/** One pixel per character. */
const chars = (t: string): number => t.length;

test("a name that fits is not cut", () => {
  assert.equal(cutMiddle("state-dict-zip/data/0", 40, chars, 4), "state-dict-zip/data/0");
});

test("a long name keeps its end, where entries of one zip differ", () => {
  const cut = cutMiddle("state-dict-zip/data/model.layers.17.attention.weight", 24, chars, 4);
  assert.equal(cut, "state-di…ttention.weight");
  assert.ok(cut !== null && cut.length <= 24);
});

test("a name cut to fewer characters than the least is left off", () => {
  assert.equal(cutMiddle("abcdefghij", 3, chars, 4), null);
});

test("a cut name is never wider than the room", () => {
  const name = "state-dict-zip/data/model.layers.17.attention.weight";
  for (let room = 5; room < name.length; room++) {
    const cut = cutMiddle(name, room, chars, 4);
    assert.ok(cut !== null && cut.length <= room, `room ${room}: ${cut}`);
  }
});

// ----- a tab's lineage, over stand-in documents -----

/** A document with only what the lineage asks of one. */
function fakeDoc(opts: {
  name?: string;
  isFile?: boolean;
  recognised?: boolean;
  origin?: number[];
  nodes?: (path: readonly number[]) => StepNode | null;
}): Doc {
  const nodes = opts.nodes ?? (() => null);
  return {
    name: opts.name ?? "file.png",
    isFile: opts.isFile ?? false,
    recognised: opts.recognised ?? false,
    lengthBits: 159 * 8,
    spaceOrigin: () => opts.origin ?? [],
    templateNode: (p: readonly number[]) => {
      const n = nodes(p);
      return n === null ? { status: "pending" } : { status: "ok", node: n };
    },
  } as unknown as Doc;
}

test("a stream opened from a joined stream's tab is walked in the file's reading", () => {
  const file = fakeDoc({ isFile: true, nodes: png });
  // The joined IDAT stream as a tab, and the scanlines opened from that tab.
  // The scanlines' origin in the tab it was opened from is [6, 0, 0]; in the
  // file's reading it is [2, 0, 6, 0, 0], which is what the editor keeps.
  const image = fakeDoc({ origin: [2] });
  const scanlines = fakeDoc({ origin: [2, 0, 6, 0, 0] });
  noteOpened(image, file);
  noteOpened(scanlines, image);
  const crumbs = tabLineage(scanlines);
  assert.deepEqual(crumbs?.map((c) => c.label), ["file.png", "image", "compressed", "scanlines"]);
  assert.ok(crumbs?.slice(1).every((c) => c.from === file), "every stream opens from the file's tab");
  assert.equal(tabTitleText(crumbs ?? []), "file.png › image › compressed › scanlines");
  assert.deepEqual(titleCrumbs(crumbs ?? [], file).map((c) => c.label), ["image", "compressed", "scanlines"]);
});

test("a stream tab nobody said was opened has no lineage", () => {
  assert.equal(tabLineage(fakeDoc({ origin: [2] })), null);
});

test("a field of the file inside streams lists them after the file", () => {
  const file = fakeDoc({ isFile: true, nodes: png });
  assert.deepEqual(fieldLineage(file, [2, 0, 6, 0, 0, 0, 0])?.map((c) => c.label), ["file.png", "image", "compressed", "scanlines"]);
});

test("a recognised stream's own streams are walked in its own reading", () => {
  const file = fakeDoc({
    isFile: true,
    nodes: reading({
      "": node("file", "Gzip"),
      "1": node("decoded", "joined → DecodedText"),
      "1.0": node("decoded", "DecodedText", { space_root: true }),
    }),
  });
  const tar = fakeDoc({
    recognised: true,
    origin: [1],
    nodes: reading({
      "": node("file", "TarArchive"),
      "0": node("entries", "Entry[]", { list: true }),
      "0.2": node("[2] inner.gz", "Entry"),
      "0.2.4": node("data", "deflate", { decoded: true }),
      "0.2.4.0": node("data", "Inflated", { space_root: true }),
    }),
  });
  const inner = fakeDoc({ origin: [0, 2, 4] });
  noteOpened(tar, file);
  noteOpened(inner, tar);
  const crumbs = tabLineage(inner);
  assert.deepEqual(crumbs?.map((c) => c.label), ["file.png", "decoded", "inner.gz"]);
  assert.equal(crumbs?.[2]?.from, tar);
});

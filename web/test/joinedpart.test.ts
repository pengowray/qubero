// What the inspector says about a stream joined from several runs: where a
// byte of it is kept, read in the file and in a tab of the stream's own, and
// where the stream is offered as a tab.
//
// The parts below are the core's answers for the three shapes there are: a PDB
// page stored as it sits, an HDF4 block the same, and a BGZF block that was
// unpacked. The words are asserted character for character, since the file's
// view and the tab's have to say the same thing about the same byte.

import { test } from "node:test";
import assert from "node:assert/strict";

import { JOINED } from "../src/strings.ts";
import { JOINED_WHOLE_CAP_BITS, startsInGroup, streamOffer, tabGroups, type PartGroup, type PartLine } from "../src/joinedpart.ts";
import type { JoinedPart, TemplateNode } from "../src/doc.ts";

/** Byte 0x50 of the stream is 0x50 into its first page, a 4 KiB page stored
 *  at 0x144000 in the file. */
const PAGE: JoinedPart = {
  index: 0,
  parts: 11,
  path: [11, 0, 2, 2, 4, 0, 0, 0],
  label: "pages[0]",
  in_part: 0x50,
  part_len: 4096,
  run_offset_bits: 0x144000 * 8,
  run_space: 0,
  packed: false,
  block_offset: null,
  in_block: null,
};

/** Byte 0x1c of what BGZF block 3 unpacks to, the block starting at byte 0x12
 *  of the file and its deflate run eighteen bytes in. */
const BLOCK: JoinedPart = {
  index: 3,
  parts: 9,
  path: [0, 3, 4],
  label: "blocks[3].compressed",
  in_part: 0x1c,
  part_len: 65280,
  run_offset_bits: (0x12 + 18) * 8,
  run_space: 0,
  packed: true,
  block_offset: 0x12,
  in_block: 0x1c,
};

const STEP = "bits @0xf3b.2 to @0xf3d.5 of range.bam: match, 5 bytes back 5,154";

const texts = (lines: readonly PartLine[]): string[] => lines.map((l) => l.text);
const shown = (groups: readonly PartGroup[]): string[][] => groups.map((g) => [g.head, ...texts(g.lines)]);

test("a field in the file names the run its first byte starts in, and goes there", () => {
  const group = startsInGroup(PAGE);
  assert.deepEqual(shown([group]), [["Starts in", "@+0x50 in pages[0]", "@0x144050 in the file"]]);
  assert.deepEqual(group.lines[0]?.path, PAGE.path);
  assert.equal(group.lines[0]?.plus, "Offset within pages[0]");
});

test("a joined tab says where the byte under the cursor is kept, not that it was unpacked", () => {
  const groups = tabGroups(PAGE, "modules.pdb", true, "bits @0x144000.0 to @0x145000.0 of modules.pdb: stored");
  // No decoder line: a stored run is one step over the whole run, which the
  // rows above have already placed. And the file by name, since the tab is a
  // document too and `the file` could be read as it.
  assert.deepEqual(shown(groups), [["Byte under the cursor is at", "@+0x50 in pages[0]", "@0x144050 in modules.pdb"]]);
});

test("a joined tab pinned to a field the cursor is not in speaks of the field's first byte, as the file does", () => {
  assert.deepEqual(shown(tabGroups(PAGE, "tdata.hdf", false, null)), [["Starts in", "@+0x50 in pages[0]", "@0x144050 in tdata.hdf"]]);
});

test("a joined tab's run is not a place to go in the tab", () => {
  // The run's path is a field of the file. Followed in the tab it would land
  // on some field of the stream instead.
  for (const g of [...tabGroups(PAGE, "modules.pdb", true, null), ...tabGroups(BLOCK, "range.bam", true, STEP)]) {
    for (const line of g.lines) assert.equal(line.path, null);
  }
});

test("a joined tab's unpacked run says so, with its virtual offset, and the step that made the byte under its own heading", () => {
  const groups = tabGroups(BLOCK, "range.bam", true, STEP);
  assert.deepEqual(shown(groups), [
    ["Byte under the cursor is at", "@+0x1c in unpacked blocks[3].compressed", `Virtual offset ${(0x12 * 65536 + 0x1c).toString()} · block_offset 18 · in_block 28`],
    ["Unpacked from", STEP],
  ]);
  assert.equal(groups[0]?.lines[0]?.plus, "Offset within unpacked blocks[3].compressed");
  // The step says what made the byte and names no place.
  assert.equal(groups[1]?.lines[0]?.place, false);
});

test("the open button and its refusals say joined, not unpacked", () => {
  assert.equal(JOINED.open, "Open joined stream");
  assert.equal(JOINED.tooLarge, "Too large to open as a document of its own (over 64 MiB)");
  assert.equal(
    JOINED.failed(JOINED.tabTitle("stream", "range.bam")),
    "Couldn't open stream joined from range.bam: one of its parts couldn't be read or unpacked.",
  );
});

/** A node with only what the offer reads set. */
const node = (over: Partial<TemplateNode>): TemplateNode =>
  ({ composite: false, child_count: 0, decoded: false, refused: null, space_root: false, joined: false, size_bits: 0, ...over }) as TemplateNode;

test("a joined stream is offered on the node that joined it and on the node it holds", () => {
  const root = node({ joined: true, space_root: true, composite: true, child_count: 16, size_bits: 42968 * 8 });
  const stitched = node({ composite: true, child_count: 1 });
  assert.deepEqual(streamOffer([1, 2], stitched, root), { open: [1, 2], joined: true });
  assert.deepEqual(streamOffer([1, 2, 0], root, null), { open: [1, 2], joined: true });
  // A field further in is not where the stream is offered.
  assert.equal(streamOffer([1, 2, 0, 3], node({ joined: true }), null), null);
});

test("a joined stream past the cap is refused before anyone asks", () => {
  const long = node({ joined: true, space_root: true, size_bits: JOINED_WHOLE_CAP_BITS + 8 });
  assert.deepEqual(streamOffer([4, 0], long, null), { refused: "too-large" });
  assert.deepEqual(streamOffer([4], node({ composite: true, child_count: 1 }), long), { refused: "too-large" });
  // Exactly at the cap opens, as the core's test is `len > CAP_BYTES`.
  const full = node({ joined: true, space_root: true, size_bits: JOINED_WHOLE_CAP_BITS });
  assert.deepEqual(streamOffer([4, 0], full, null), { open: [4], joined: true });
});

test("a compressed run keeps its own offer, and a refused one gets none here", () => {
  assert.deepEqual(streamOffer([2], node({ decoded: true }), null), { open: [2], joined: false });
  assert.equal(streamOffer([2], node({ decoded: true, refused: "failed" }), null), null);
  // The root of an unpacked stream is offered in the listing, and in the
  // inspector on the run itself.
  assert.equal(streamOffer([2, 0], node({ space_root: true }), null), null);
});

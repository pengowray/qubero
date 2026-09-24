// The report's reading of the core's data: the ledger's lines, the landmarks
// from the format profile, the window the extent figure draws, which wheel
// events a figure takes, and whether a directory's targets are in order.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { LedgerRow, Profile, ProfileRow } from "../src/report/coredata.ts";
import { sameOrder } from "../src/report/directories.ts";
import { extentWindow } from "../src/report/extentfigure.ts";
import { isGapLine, ledgerLines, fieldRows } from "../src/report/ledger.ts";
import { landmarks, readingWriting, rowLabel } from "../src/report/profiletext.ts";
import { SETTLE_MS, takesWheel } from "../src/report/wheel.ts";

function row(part: number[], partName: string, group: string, role: LedgerRow["role"], bits: number, first: number[], firstOffset: number, extra: Partial<LedgerRow> = {}): LedgerRow {
  return {
    part,
    part_name: partName,
    group,
    group_from: group === "" ? "none" : "key",
    role,
    bits,
    count: 1,
    zero_bits: 0,
    unscanned_bits: 0,
    first_path: first,
    first_offset_bits: firstOffset,
    align: 0,
    ...extra,
  };
}

test("a part's groups are a line each, and their roles are summed under them", () => {
  const lines = ledgerLines([
    row([0], "records", "local file", "content", 800, [0, 0, 1], 32),
    row([0], "records", "local file", "machinery", 80, [0, 0, 0], 0),
    row([0], "records", "central directory file", "content", 400, [0, 11, 1], 1000),
  ]);
  assert.equal(lines.length, 2);
  const local = lines[0];
  assert.equal(local?.group, "local file");
  assert.equal(local?.bits, 880);
  // The line starts where its first bytes are, whichever role they have.
  assert.equal(local?.firstOffsetBits, 0);
  assert.deepEqual(local?.roles.map((r) => r.role), ["content", "machinery"]);
});

test("a structure named by its type inside another group's element is counted in that group's line", () => {
  // A JPEG's dht segment holds its Huffman tables.
  const lines = ledgerLines([
    row([1], "segments", "dht, huffman tables", "machinery", 128, [1, 4, 0], 1416),
    row([1], "segments", "HuffmanTable", "content", 2816, [1, 4, 1, 1, 0, 0], 1448, { group_from: "type" }),
    row([1], "segments", "HuffmanTable", "machinery", 512, [1, 4, 1, 1, 0, 2, 0], 1456, { group_from: "type" }),
    row([1], "segments", "sof0, baseline dct", "content", 40, [1, 3, 1, 1, 0], 1300),
  ]);
  assert.deepEqual(lines.map((l) => l.group), ["sof0, baseline dct", "dht, huffman tables"]);
  assert.equal(lines[1]?.bits, 128 + 2816 + 512);
  assert.equal(lines[1]?.roles.find((r) => r.role === "machinery")?.bits, 640);
});

test("a group named by a value inside another group's element keeps its own line", () => {
  // A MIDI track's meta events are named by their status byte.
  const lines = ledgerLines([
    row([1], "file", "MTrk", "machinery", 192, [1, 0], 112),
    row([1], "file", "meta", "content", 520, [1, 2, 0, 0], 176),
  ]);
  assert.deepEqual(lines.map((l) => l.group), ["MTrk", "meta"]);
});

test("the plain fields of one structure are one line, and one field alone keeps its name", () => {
  const lines = ledgerLines([
    row([7, 0], "type", "", "content", 16, [7, 0], 128),
    row([7, 1], "machine", "", "machinery", 16, [7, 1], 144),
    row([7, 2], "version", "", "content", 32, [7, 2], 160),
    row([0], "magic", "", "content", 32, [0], 0),
  ]);
  assert.equal(lines.length, 2);
  const header = lines.find((l) => l.fields.length === 3);
  assert.deepEqual(header?.parentPath, [7]);
  assert.equal(header?.part, null);
  assert.equal(header?.bits, 64);
  const magic = lines.find((l) => l.fields.length === 1);
  assert.equal(magic?.part, "magic");
});

test("bytes no field describes are one line, with their zero bytes kept", () => {
  const lines = ledgerLines([
    row([7], "header", "", "gap", 800, [7], 400, { zero_bits: 800 }),
    row([3], "tail", "", "gap", 80, [3], 9000, { zero_bits: 0 }),
  ]);
  assert.equal(lines.length, 1);
  const gap = lines[0];
  assert.ok(gap !== undefined && isGapLine(gap));
  assert.equal(gap?.roles[0]?.zeroBits, 800);
  assert.equal(gap?.bits, 880);
});

function profile(rows: Partial<ProfileRow>[], facts: Partial<Profile["facts"]> = {}): Profile {
  return {
    done: true,
    rows: rows.map((r) => ({ category: "number", kind: "unsigned", width: 16, order: "big", detail: "", fields: 1, bits: 16, unpacked_fields: 0, unpacked_bits: 0, ...r })),
    choices: [],
    facts: { from_end: 0, placed: 0, forward: 0, backward: 0, lengths_before: 0, lengths_after: 0, every_field_follows: null, ...facts },
  };
}

test("the landmarks say the byte order and the widths the file uses", () => {
  const p = profile([
    { width: 16, order: "big" },
    { width: 32, order: "big" },
    { width: 8, order: "none" },
    { width: 64, order: "big", fields: 0 },
  ]);
  const said = landmarks(p);
  assert.equal(said[0], "Big-endian.");
  assert.equal(said[1], "Integers are 8, 16, and 32 bits.");
});

test("both byte orders are said with their counts", () => {
  const said = landmarks(profile([{ order: "big", fields: 3 }, { order: "little", fields: 5 }]));
  assert.equal(said[0], "Both byte orders: 5 little-endian numbers and 3 big-endian numbers.");
});

test("a reader who can go straight through is told so, and a writer what to know", () => {
  const said = readingWriting(profile([], { every_field_follows: true, lengths_before: 4 }));
  assert.equal(said[0], "A reader can go from the start of the file to the end without going back.");
  assert.match(said[1] ?? "", /^4 lengths come before what they size/);
});

test("profile rows read as plain words", () => {
  assert.equal(rowLabel({ category: "number", kind: "unsigned", width: 16, order: "little", detail: "", fields: 1, bits: 0, unpacked_fields: 0, unpacked_bits: 0 }), "16-bit unsigned integer, little-endian");
  assert.equal(rowLabel({ category: "text", kind: "terminated", width: 0, order: "", detail: "utf8", fields: 1, bits: 0, unpacked_fields: 0, unpacked_bits: 0 }), "UTF-8, ends at a terminator");
  assert.equal(rowLabel({ category: "sizing", kind: "length-field", width: 0, order: "", detail: "", fields: 1, bits: 0, unpacked_fields: 0, unpacked_bits: 0 }), "a length worked out from other fields");
});

test("ends far apart are drawn from the part's start", () => {
  const w = extentWindow(0, [1000, 800]);
  assert.equal(w.lo, 0);
  assert.equal(w.cut, false);
});

test("ends close together at the end of a long part are drawn near the ends, at one scale", () => {
  // The pipistrelle WAV: 301,020 bytes stated from 12, and a file of 301,024.
  const start = 96;
  const ends = [start + 2408160, 2408192, start + 2408096];
  const w = extentWindow(start, ends);
  assert.equal(w.cut, true);
  assert.ok(w.lo > start);
  assert.ok(w.lo < Math.min(...ends));
  assert.ok(w.hi > Math.max(...ends));
});

test("a figure takes the wheel only once the page is still", () => {
  assert.equal(takesWheel(SETTLE_MS + 1, false, false), true);
  assert.equal(takesWheel(SETTLE_MS - 1, false, false), false);
});

test("a figure zoomed all the way out lets a zoom-out through to the page", () => {
  assert.equal(takesWheel(10_000, true, true), false);
  assert.equal(takesWheel(10_000, false, true), true);
  assert.equal(takesWheel(10_000, true, false), true);
});

test("a part's rows leave out a field as big as the part and count neighbours of one name", () => {
  const kid = (name: string, at: number, size: number) => ({ name, path: [at], offset_bits: at, size_bits: size });
  const runs = fieldRows({ offsetBits: 0, sizeBits: 400 }, [kid("body", 0, 400), kid("[0] dht", 0, 100), kid("[1] dht", 100, 100), kid("[2] dqt", 200, 200)]);
  assert.deepEqual(runs.map((r) => [r.name, r.count, r.startBit, r.endBit]), [["dht", 2, 0, 200], ["dqt", 1, 200, 400]]);
});

test("targets in the order of their entries are in order, and any step back is not", () => {
  assert.equal(sameOrder([0, 10, 10, 40]), true);
  assert.equal(sameOrder([40, 10]), false);
});

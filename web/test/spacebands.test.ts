// The arithmetic behind the file-against-memory picture (`viz/spacebands.ts`):
// the stretches read out of ELF and PE headers, the memory axis with its long
// gaps cut out, the names put on rows, and the bands' hit test. The numbers
// are those of the two samples the piece is checked on,
// elf/busybox-x86_64 and pe/busybox-w32-x86_64.exe.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  accessOf,
  axisAt,
  axisY,
  bandAt,
  bandEdges,
  elfPicture,
  enumWord,
  fileAxis,
  memoryAxis,
  pePicture,
  placeLabels,
  regionAtByte,
  sectionCounts,
  type ElfIn,
  type PeIn,
} from "../src/viz/spacebandslayout.ts";

/** elf/busybox-x86_64, as its headers state it. */
const BUSYBOX_ELF: ElfIn = {
  fileBytes: 1131168,
  headerSize: 64,
  headerEntry: null,
  phoff: 64,
  phnum: 4,
  phentsize: 56,
  phEntry: null,
  shoff: 1130528,
  shnum: 10,
  shentsize: 64,
  shEntry: null,
  sections: [
    { index: 0, name: "", type: 0, flags: "", address: 0, offset: 0, size: 0, entry: null, body: null },
    { index: 1, name: ".init", type: 1, flags: "alloc, execute", address: 4194592, offset: 288, size: 13, entry: null, body: null },
    { index: 2, name: ".text", type: 1, flags: "alloc, execute", address: 4194608, offset: 304, size: 890662, entry: null, body: null },
    { index: 3, name: ".fini", type: 1, flags: "alloc, execute", address: 5085270, offset: 890966, size: 8, entry: null, body: null },
    { index: 4, name: ".rodata", type: 1, flags: "alloc", address: 5085280, offset: 890976, size: 228795, entry: null, body: null },
    { index: 5, name: ".eh_frame", type: 1, flags: "alloc", address: 5314076, offset: 1119772, size: 4, entry: null, body: null },
    { index: 6, name: ".ctors", type: 1, flags: "write, alloc", address: 7413728, offset: 1122272, size: 16, entry: null, body: null },
    { index: 7, name: ".dtors", type: 1, flags: "write, alloc", address: 7413744, offset: 1122288, size: 16, entry: null, body: null },
    { index: 8, name: ".data", type: 1, flags: "write, alloc", address: 7413760, offset: 1122304, size: 8157, entry: null, body: null },
    { index: 9, name: ".shstrtab", type: 3, flags: "", address: 0, offset: 1130461, size: 67, entry: null, body: null },
  ],
  segments: [
    { index: 0, type: "load", flags: "execute, read", offset: 0, vaddr: 4194304, fileSize: 1119776, memSize: 1119776, entry: null },
    { index: 1, type: "load", flags: "write, read", offset: 1122272, vaddr: 7413728, fileSize: 8189, memSize: 8189, entry: null },
    { index: 2, type: "gnu_stack", flags: "write, read", offset: 0, vaddr: 0, fileSize: 0, memSize: 0, entry: null },
    { index: 3, type: "gnu_relro", flags: "read", offset: 1122272, vaddr: 7413728, fileSize: 32, memSize: 32, entry: null },
  ],
};

/** pe/busybox-w32-x86_64.exe, as its section table states it. */
const BUSYBOX_PE: PeIn = {
  fileBytes: 719360,
  imageBase: 5368709120,
  headersSize: 1024,
  headersEntry: null,
  sections: [
    [".text", 534912, 4096, 535040, 1024, "cnt code, mem execute, mem read"],
    [".data", 1104, 540672, 1536, 536064, "cnt initialized data, mem read, mem write"],
    [".rdata", 87440, 544768, 87552, 537600, "cnt initialized data, mem read"],
    [".pdata", 22824, 634880, 23040, 625152, "cnt initialized data, mem read"],
    [".xdata", 24600, 659456, 25088, 648192, "cnt initialized data, mem read"],
    [".bss", 5584, 688128, 0, 0, "cnt uninitialized data, mem read, mem write"],
    [".idata", 11220, 696320, 11264, 673280, "cnt initialized data, mem read"],
    [".tls", 16, 708608, 512, 684544, "cnt initialized data, mem read, mem write"],
    [".rsrc", 32280, 712704, 32768, 685056, "cnt initialized data, mem read"],
    [".reloc", 1280, 745472, 1536, 717824, "cnt initialized data, mem discardable, mem read"],
  ].map(([name, virtualSize, virtualAddress, rawSize, rawOffset, flags], index) => ({
    index,
    name: name as string,
    virtualSize: virtualSize as number,
    virtualAddress: virtualAddress as number,
    rawSize: rawSize as number,
    rawOffset: rawOffset as number,
    flags: flags as string,
    entry: null,
    body: null,
  })),
};

// ----- access and words -----

test("flags words give the access, whole words only", () => {
  assert.equal(accessOf("alloc, execute", true), "exec");
  assert.equal(accessOf("write, alloc", true), "write");
  assert.equal(accessOf("alloc", true), "read");
  assert.equal(accessOf("cnt code, mem execute, mem read", true), "exec");
  assert.equal(accessOf("mem read, mem write, mem execute", true), "write-exec");
  assert.equal(accessOf("executable image", true), "read");
  assert.equal(accessOf("alloc, execute", false), "unloaded");
});

test("an enum value's word is the part before its number", () => {
  assert.equal(enumWord("load (1)"), "load");
  assert.equal(enumWord("gnu_relro (1685382482)"), "gnu_relro");
  assert.equal(enumWord("dynamic"), "dynamic");
});

// ----- ELF -----

test("busybox ELF: sections, headers and segments", () => {
  const pic = elfPicture(BUSYBOX_ELF);
  const names = pic.regions.map((r) => r.name);
  assert.deepEqual(names, ["ELF header", "Program header table", ".init", ".text", ".fini", ".rodata", ".eh_frame", ".ctors", ".dtors", ".data", ".shstrtab", "Section header table"]);
  const by = new Map(pic.regions.map((r) => [r.name, r]));
  // The headers are inside the first LOAD segment, so they are loaded too.
  assert.deepEqual(by.get("ELF header")?.mem, { start: 4194304, end: 4194368 });
  assert.equal(by.get("ELF header")?.access, "exec");
  assert.deepEqual(by.get("Program header table")?.file, { start: 64, end: 288 });
  assert.equal(by.get(".text")?.access, "exec");
  assert.equal(by.get(".rodata")?.access, "read");
  assert.equal(by.get(".data")?.access, "write");
  assert.equal(by.get(".shstrtab")?.mem, null);
  assert.equal(by.get(".shstrtab")?.access, "unloaded");
  assert.equal(by.get("Section header table")?.mem, null);
  // gnu_stack states only flags, so there is nothing of it to draw.
  assert.deepEqual(pic.segments.map((s) => s.name), ["load 1", "load 2", "gnu_relro 4"]);
  assert.equal(pic.segments[0]?.access, "exec");
  assert.equal(pic.segments[1]?.access, "write");
  const counts = sectionCounts(pic.regions);
  assert.equal(counts.total, 9);
  assert.equal(counts.loaded, 8);
  assert.deepEqual(counts.fileOnly.map((r) => r.name), [".shstrtab"]);
});

test("an ELF NOBITS section has no file bytes, a point in the file, and memory", () => {
  const pic = elfPicture({
    ...BUSYBOX_ELF,
    sections: [{ index: 1, name: ".bss", type: 8, flags: "write, alloc", address: 7421920, offset: 1130461, size: 4096, entry: null, body: null }],
  });
  const bss = pic.regions.find((r) => r.name === ".bss");
  assert.equal(bss?.file, null);
  assert.equal(bss?.filePoint, 1130461);
  assert.deepEqual(bss?.mem, { start: 7421920, end: 7426016 });
  assert.deepEqual(sectionCounts(pic.regions).memOnly.map((r) => r.name), [".bss"]);
});

// ----- PE -----

test("busybox PE: headers, sections at image base plus RVA, .bss memory only", () => {
  const pic = pePicture(BUSYBOX_PE);
  const by = new Map(pic.regions.map((r) => [r.name, r]));
  assert.deepEqual(by.get("Headers")?.mem, { start: 5368709120, end: 5368710144 });
  assert.deepEqual(by.get(".text")?.file, { start: 1024, end: 536064 });
  assert.deepEqual(by.get(".text")?.mem, { start: 5368709120 + 4096, end: 5368709120 + 4096 + 534912 });
  assert.equal(by.get(".text")?.access, "exec");
  assert.equal(by.get(".data")?.access, "write");
  assert.equal(by.get(".rdata")?.access, "read");
  assert.equal(by.get(".bss")?.file, null);
  assert.equal(by.get(".bss")?.filePoint, null);
  assert.equal(by.get(".bss")?.access, "write");
  // The last section ends at the end of the file, so there is no overlay.
  assert.equal(by.has("Data after the last section"), false);
  // File order first, then the memory-only .bss.
  assert.equal(pic.regions[pic.regions.length - 1]?.name, ".bss");
});

test("PE bytes past the last section are a stretch in the file only", () => {
  const pic = pePicture({ ...BUSYBOX_PE, fileBytes: 719360 + 4000 });
  const over = pic.regions.find((r) => r.key === "pe-overlay");
  assert.deepEqual(over?.file, { start: 719360, end: 723360 });
  assert.equal(over?.mem, null);
});

test("a PE virtual size of zero means the raw size", () => {
  const pic = pePicture({ ...BUSYBOX_PE, sections: [{ ...(BUSYBOX_PE.sections[0] as PeIn["sections"][number]), virtualSize: 0 }] });
  const text = pic.regions.find((r) => r.name === ".text");
  assert.equal((text?.mem?.end ?? 0) - (text?.mem?.start ?? 0), 535040);
});

// ----- axes -----

test("the ELF memory axis cuts the 2 MiB gap between the two LOAD segments", () => {
  const pic = elfPicture(BUSYBOX_ELF);
  const ranges = pic.regions.flatMap((r) => (r.mem === null ? [] : [r.mem]));
  const axis = memoryAxis(ranges, 600);
  assert.equal(axis.pieces.length, 2);
  assert.equal(axis.cuts.length, 1);
  const cut = axis.cuts[0];
  assert.equal(cut?.start, 5314080);
  assert.equal(cut?.end, 7413728);
  assert.equal((cut?.y1 ?? 0) - (cut?.y0 ?? 0), 18);
  // The data segment is a hundredth of the code, raised to the least height.
  const data = axis.pieces[1];
  assert.equal(Math.round((data?.y1 ?? 0) - (data?.y0 ?? 0)), 28);
  assert.equal(Math.round(axis.pieces[axis.pieces.length - 1]?.y1 ?? 0), 600);
  // Addresses run down the axis in order, and back again.
  let last = -1;
  for (const r of ranges) {
    const y = axisY(axis, r.start);
    assert.ok(y >= last, `${r.start} drawn above the one before`);
    last = y;
  }
  const y = axisY(axis, 4194608);
  assert.ok(Math.abs(axisAt(axis, y).at - 4194608) < 1);
  assert.notEqual(axisAt(axis, ((cut?.y0 ?? 0) + (cut?.y1 ?? 0)) / 2).cut, null);
});

test("the PE memory axis keeps the alignment gaps to scale", () => {
  const pic = pePicture(BUSYBOX_PE);
  const axis = memoryAxis(pic.regions.flatMap((r) => (r.mem === null ? [] : [r.mem])), 500);
  assert.equal(axis.cuts.length, 0);
  assert.equal(axis.pieces.length, 1);
  assert.equal(axis.start, 5368709120);
});

test("the file axis is the whole file to scale", () => {
  const axis = fileAxis(1000, 500);
  assert.equal(axisY(axis, 500), 250);
  assert.equal(axisAt(axis, 125).at, 250);
});

// ----- names -----

test("names never share a row, keep their order, and drop when there is no room", () => {
  // Five names that all want the same height, and room for three rows.
  const wants = [10, 11, 12, 13, 14].map((y, i) => ({ y, priority: i }));
  const rows = placeLabels(wants, 10, 0, 30, 4);
  const kept = rows.filter((r): r is number => r !== null);
  assert.equal(kept.length, 3);
  assert.equal(new Set(kept).size, 3);
  // The three most important survive.
  assert.deepEqual(rows.map((r) => r !== null), [false, false, true, true, true]);
  // In the order of the heights they want, so the leaders do not cross.
  const placed = rows.map((r, i) => ({ r, y: wants[i]?.y ?? 0 })).filter((p) => p.r !== null);
  for (let i = 1; i < placed.length; i++) assert.ok((placed[i]?.r ?? 0) > (placed[i - 1]?.r ?? 0));
});

test("a name off the drawing is dropped, one with room stays level with its section", () => {
  const rows = placeLabels([{ y: -20, priority: 9 }, { y: 55, priority: 1 }], 10, 0, 100);
  assert.equal(rows[0], null);
  assert.equal(rows[1], 55);
});

// ----- bands -----

test("a band is picked inside its edges, and a hairline from near it", () => {
  const wide = { xa: 0, xb: 100, fa: 0, fb: 100, ma: 0, mb: 100 };
  const thin = { xa: 0, xb: 100, fa: 50, fb: 50.5, ma: 200, mb: 200.5 };
  // Halfway across, the thin band's edge is halfway down its slope.
  assert.equal(bandEdges(thin, 50).top, 125);
  assert.equal(bandAt([wide, thin], 50, 125), 1);
  assert.equal(bandAt([wide, thin], 50, 129), 1);
  assert.equal(bandAt([wide, thin], 50, 50), 0);
  assert.equal(bandAt([wide, thin], 150, 50), null);
});

test("the region at a byte is the smallest that holds it", () => {
  const pic = elfPicture(BUSYBOX_ELF);
  assert.equal(regionAtByte(pic.regions, 300)?.name, ".init");
  assert.equal(regionAtByte(pic.regions, 10)?.name, "ELF header");
  assert.equal(regionAtByte(pic.regions, 1121000), null);
});

// The index-lookup piece (`viz/indexlookup.ts`) without a page: the readers
// in `viz/tableentries.ts` run against a stand-in document shaped like the
// three samples the piece was built on (an ELF section header naming itself
// from `.shstrtab`, an indexed PNG pixel and its palette, a TIFF header's
// pointer to its first directory), and the strip arithmetic in
// `viz/lookuplayout.ts` is checked on its own.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { Origin, Relation, TemplateNode, TemplateReply } from "../src/doc.ts";
import {
  byStart,
  entryAt,
  entryLabel,
  hexColour,
  lookupFor,
  mayLookUp,
  PENDING,
  scanPointers,
  windowAround,
  type Lookup,
  type Reads,
  type TableEntry,
} from "../src/viz/tableentries.ts";
import { axisOrigin, centreOn, inView, lookupTicks, openingFromStart, openingView, stepEntry, stripLayout, unitOf, unitX } from "../src/viz/lookuplayout.ts";

// ----- a stand-in document -----

type Spec = Partial<TemplateNode> & { name: string; kids?: Spec[]; origins?: Origin[]; relations?: Relation[] };

/** A tree of nodes by path, with the few calls the readers make. Offsets are
 *  given in bytes in the specs and stored in bits, as the core does. */
class FakeDoc implements Reads {
  private readonly nodes = new Map<string, { node: TemplateNode; kids: string[]; origins: Origin[]; relations: Relation[] }>();
  pendingAt: string | null = null;

  constructor(root: Spec) {
    this.add(root, []);
  }

  private add(spec: Spec, path: number[]): void {
    const kids = spec.kids ?? [];
    const node: TemplateNode = {
      path,
      name: spec.name,
      type: spec.type ?? (kids.length > 0 ? "struct" : "u8"),
      offset_bits: (spec.offset_bits ?? 0) * 8,
      size_bits: (spec.size_bits ?? 0) * 8,
      value: spec.value ?? String(kids.length),
      edit_text: spec.edit_text ?? spec.value ?? "",
      kind: spec.kind ?? (kids.length > 0 ? "composite" : "uint"),
      problems_within: [0, 0] as unknown as TemplateNode["problems_within"],
      child_count: kids.length,
      composite: kids.length > 0 || spec.composite === true,
      list: spec.list ?? false,
      editable: false,
      inline: false,
      value_bytes: 0,
      value_offset_bits: 0,
      read_at: spec.read_at === undefined || spec.read_at === null ? null : spec.read_at * 8,
      read_as: null,
      consumed_by: null,
      machinery: null,
      contents: false,
      framed: false,
      space: spec.space ?? 0,
      refused: null,
      decoded: false,
      space_root: false,
      joined: false,
      lsb_first: false,
      absent: false,
      line: spec.line ?? null,
    };
    const key = path.join("/");
    this.nodes.set(key, { node, kids: kids.map((_, i) => [...path, i].join("/")), origins: spec.origins ?? [], relations: spec.relations ?? [] });
    kids.forEach((k, i) => this.add(k, [...path, i]));
  }

  private reply<T>(key: string, value: T | undefined): TemplateReply<T> {
    if (this.pendingAt === key) return { status: "pending", reachedBytes: 0 };
    return value === undefined ? { status: "error", message: "no such field" } : { status: "ok", node: value };
  }

  templateNode(path: readonly number[]): TemplateReply<TemplateNode> {
    const key = path.join("/");
    return this.reply(key, this.nodes.get(key)?.node);
  }
  templateChildren(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]> {
    const key = path.join("/");
    const n = this.nodes.get(key);
    return this.reply(key, n?.kids.slice(from, to).map((k) => (this.nodes.get(k) as { node: TemplateNode }).node));
  }
  origins(path: readonly number[]): TemplateReply<Origin[]> {
    const key = path.join("/");
    return this.reply(key, this.nodes.get(key)?.origins);
  }
  relations(path: readonly number[]): TemplateReply<Relation[]> {
    const key = path.join("/");
    return this.reply(key, this.nodes.get(key)?.relations);
  }
  /** The deepest field covering a bit, preferring the last child that does,
   *  which is enough for these trees. */
  locate(bit: number): TemplateReply<number[]> {
    let path: number[] = [];
    for (;;) {
      const n = this.nodes.get(path.join("/"));
      if (n === undefined) break;
      const inside = n.kids.map((k) => this.nodes.get(k)?.node).findIndex((c) => c !== undefined && c.size_bits > 0 && c.read_at === null && c.offset_bits <= bit && bit < c.offset_bits + c.size_bits);
      if (inside < 0) break;
      path = [...path, inside];
    }
    return { status: "ok", node: path };
  }
  node(path: readonly number[]): TemplateNode {
    const r = this.templateNode(path);
    assert.equal(r.status, "ok");
    return (r as { node: TemplateNode }).node;
  }
}

const points = (label: string, targetByte: number): Origin => ({ role: "points", label, stored: null, path: [], value: "", target_bits: targetByte * 8 });

/** An ELF in miniature: three section headers whose `name_offset` points
 *  into a string table, which is section 2. The name strings start at 100. */
function elf(): FakeDoc {
  const strings = ["", ".shstrtab", ".text"];
  let at = 100;
  const strs = strings.map((s, i) => {
    const spec: Spec = { name: `[${i}]`, kind: "str", type: "cstr", value: s, offset_bits: at, size_bits: s.length + 1 };
    at += s.length + 1;
    return spec;
  });
  const starts = [100, 101, 111];
  const header = (i: number, nameOffset: number): Spec => ({
    name: `[${i}]`,
    offset_bits: 200 + i * 8,
    size_bits: 8,
    kids: [
      { name: "name_offset", type: "u32 le", value: String(nameOffset), offset_bits: 200 + i * 8, size_bits: 4, origins: [points("name", 100 + nameOffset)] },
      {
        name: "name",
        type: "at → cstr",
        composite: true,
        offset_bits: 204 + i * 8,
        size_bits: 0,
        read_at: 100 + nameOffset,
        value: "1",
        kids: [
          {
            name: "name",
            kind: "str",
            type: "cstr",
            value: strings[starts.indexOf(100 + nameOffset)] ?? "",
            offset_bits: 100 + nameOffset,
            size_bits: 1,
            relations: [{ role: "position", written: "base + name_offset", template: null, substituted: `100 + ${nameOffset}`, result: String(100 + nameOffset) }],
          },
        ],
      },
      { name: "type", type: "u32 le", value: "1", offset_bits: 204 + i * 8, size_bits: 4 },
    ],
  });
  return new FakeDoc({
    name: "file",
    offset_bits: 0,
    size_bits: 300,
    kids: [
      { name: "sections", list: true, offset_bits: 0, size_bits: 200, kids: [{ name: "[0]", size_bits: 0, offset_bits: 0, kind: "bytes", value: "" }, { name: "[1] .text", offset_bits: 0, size_bits: 100, kind: "bytes", value: "" }, { name: "[2] .shstrtab", list: true, type: "cstr[]", offset_bits: 100, size_bits: 17, kids: strs }] },
      { name: "section_headers", list: true, offset_bits: 200, size_bits: 24, kids: [header(0, 0), header(1, 11), header(2, 1)] },
    ],
  });
}

test("an ELF name_offset looks up its string in the string table, not in the list of headers", () => {
  const doc = elf();
  const l = lookupFor(doc, doc.node([1, 1, 0])) as Lookup;
  assert.notEqual(l, null);
  assert.notEqual(l, PENDING);
  assert.equal(l.table.node.name, "[2] .shstrtab");
  assert.equal(l.table.kind, "strings");
  assert.equal(l.table.counts, "bytes");
  assert.equal(l.hit, 2);
  assert.equal(l.target, 111 * 8);
  assert.deepEqual(l.table.entries.map((e) => e.label), ["", ".shstrtab", ".text"]);
  assert.equal(l.indexLabel, "section_headers[1].name_offset");
  assert.equal(l.via?.written, "base + name_offset");
  assert.equal(l.how, "points");
});

test("the field read at the far end finds the same lookup through its sibling", () => {
  const doc = elf();
  const l = lookupFor(doc, doc.node([1, 2, 1])) as Lookup;
  assert.equal(l.hit, 1);
  assert.deepEqual(l.index.path, [1, 2, 0]);
});

test("a plain number with no pointer is not a lookup, and a reply on its way says so", () => {
  const doc = elf();
  assert.equal(lookupFor(doc, doc.node([1, 0, 2])), null);
  const n = doc.node([1, 1, 0]);
  doc.pendingAt = "1/1/0";
  assert.equal(lookupFor(doc, n), PENDING);
});

test("the scan finds which header points at each string", () => {
  const doc = elf();
  const l = lookupFor(doc, doc.node([1, 0, 0])) as Lookup;
  const p = scanPointers(doc, l);
  assert.notEqual(p, PENDING);
  assert.notEqual(p, null);
  const ps = p as Exclude<typeof p, null | typeof PENDING>;
  assert.equal(ps.total, 3);
  assert.equal(ps.scanned, 3);
  assert.deepEqual(ps.byEntry.get(2)?.map((x) => x.label), ["section_headers[1].name_offset"]);
  assert.deepEqual(ps.byEntry.get(1)?.map((x) => x.label), ["section_headers[2].name_offset"]);
  assert.deepEqual(ps.byEntry.get(0)?.map((x) => x.label), ["section_headers[0].name_offset"]);
});

/** A PNG in miniature: IHDR, a PLTE of three colours, a tRNS of two alphas,
 *  and a decoded raster of IndexedPixel elements. */
function png(): FakeDoc {
  const chunk = (name: string, at: number, data: Spec): Spec => ({
    name,
    type: "Chunk",
    offset_bits: at,
    size_bits: 12 + (data.size_bits ?? 0),
    kids: [
      { name: "length", offset_bits: at, size_bits: 4 },
      { name: "type", kind: "str", offset_bits: at + 4, size_bits: 4 },
      { ...data, offset_bits: at + 8 },
      { name: "crc", offset_bits: at + 8 + (data.size_bits ?? 0), size_bits: 4 },
    ],
  });
  const rgb = (i: number, r: number, g: number, b: number): Spec => ({
    name: `[${i}]`,
    type: "PaletteEntry",
    offset_bits: 41 + i * 3,
    size_bits: 3,
    line: `${r} ${g} ${b}`,
    kids: [
      { name: "red", value: String(r), offset_bits: 41 + i * 3, size_bits: 1 },
      { name: "green", value: String(g), offset_bits: 42 + i * 3, size_bits: 1 },
      { name: "blue", value: String(b), offset_bits: 43 + i * 3, size_bits: 1 },
    ],
  });
  const pixel = (i: number, v: number): Spec => ({ name: `[${i}]`, type: "IndexedPixel", space: 3, offset_bits: i, size_bits: 1, kids: [{ name: "index", value: String(v), offset_bits: i, size_bits: 1, space: 3 }] });
  return new FakeDoc({
    name: "file",
    size_bits: 200,
    kids: [
      { name: "signature", kind: "magic", size_bits: 8 },
      {
        name: "chunks",
        list: true,
        offset_bits: 8,
        size_bits: 100,
        kids: [
          chunk("[0] IHDR", 8, { name: "data", type: "IHDR", size_bits: 13, kids: [{ name: "width", size_bits: 4 }] }),
          chunk("[1] PLTE", 33, { name: "data", type: "PLTE", size_bits: 9, kids: [{ name: "entries", list: true, offset_bits: 41, size_bits: 9, kids: [rgb(0, 0, 0, 0), rgb(1, 230, 90, 20), rgb(2, 20, 120, 240)] }] }),
          chunk("[2] tRNS", 54, { name: "data", type: "tRNSPalette", size_bits: 2, kids: [{ name: "alpha", list: true, kids: [{ name: "[0]", value: "0", size_bits: 1 }, { name: "[1]", value: "140", size_bits: 1 }] }] }),
          chunk("[3] IDAT", 68, { name: "data", type: "IDAT", size_bits: 4, kids: [{ name: "image_data", size_bits: 4 }] }),
        ],
      },
      { name: "image", offset_bits: 0, size_bits: 4, kids: [{ name: "pixels", list: true, space: 3, size_bits: 4, kids: [pixel(0, 1), pixel(1, 2), pixel(2, 1), pixel(3, 9)] }] },
    ],
  });
}

test("a pixel of an indexed PNG looks up its colour in the palette, with its tRNS alpha", () => {
  const doc = png();
  const l = lookupFor(doc, doc.node([2, 0, 1, 0])) as Lookup;
  assert.equal(l.how, "palette");
  assert.equal(l.table.kind, "palette");
  assert.equal(l.table.counts, "entries");
  assert.equal(l.table.count, 3);
  assert.equal(l.hit, 2);
  assert.equal(l.colour, "#1478f0");
  assert.deepEqual(l.alphas, [0, 140]);
  assert.equal(l.indexLabel, "pixels[1].index");
  // The cursor on the pixel itself finds the same entry.
  assert.equal((lookupFor(doc, doc.node([2, 0, 1])) as Lookup).hit, 2);
});

test("a pixel past the end of the palette lands on no entry", () => {
  const doc = png();
  const l = lookupFor(doc, doc.node([2, 0, 3, 0])) as Lookup;
  assert.equal(l.hit, -1);
});

test("the scan counts the pixels using each palette entry", () => {
  const doc = png();
  const l = lookupFor(doc, doc.node([2, 0, 0, 0])) as Lookup;
  const ps = scanPointers(doc, l) as Exclude<ReturnType<typeof scanPointers>, null | typeof PENDING>;
  assert.equal(ps.total, 4);
  assert.deepEqual(ps.byEntry.get(1)?.map((x) => x.label), ["pixels[0].index", "pixels[2].index"]);
  assert.equal(ps.byEntry.get(0), undefined);
});

/** A TIFF in miniature: a header whose `ifd_offset` points at a directory
 *  that is read there, and a run of bytes that covers the same place. */
function tiff(): FakeDoc {
  return new FakeDoc({
    name: "file",
    size_bits: 60,
    kids: [
      { name: "ifd_offset", offset_bits: 4, size_bits: 4, value: "8", origins: [points("ifd", 8)] },
      {
        name: "ifd",
        type: "at → Ifd",
        composite: true,
        offset_bits: 8,
        size_bits: 0,
        read_at: 8,
        kids: [
          {
            name: "ifd",
            type: "Ifd",
            offset_bits: 8,
            size_bits: 30,
            line: "entries 2 entries · next_ifd_offset 0",
            kids: [
              { name: "count", offset_bits: 8, size_bits: 2, value: "2" },
              { name: "entries", list: true, offset_bits: 10, size_bits: 24, kids: [{ name: "[0] image width", offset_bits: 10, size_bits: 12, kids: [{ name: "tag", size_bits: 2 }] }, { name: "[1] image height", offset_bits: 22, size_bits: 12, kids: [{ name: "tag", size_bits: 2 }] }] },
              { name: "next_ifd_offset", offset_bits: 34, size_bits: 4, value: "0" },
            ],
          },
        ],
      },
      { name: "body", kind: "bytes", offset_bits: 8, size_bits: 52 },
    ],
  });
}

test("a pointer to one structure draws the structure's fields, its list's elements among them", () => {
  const doc = tiff();
  const l = lookupFor(doc, doc.node([0])) as Lookup;
  assert.equal(l.table.reached, true);
  assert.equal(l.result.name, "ifd");
  assert.deepEqual(l.table.entries.map((e) => e.label), ["count", "image width", "image height", "next_ifd_offset"]);
  assert.equal(l.hit, 0);
  assert.equal(l.indexLabel, "ifd_offset");
  const ps = scanPointers(doc, l) as Exclude<ReturnType<typeof scanPointers>, null | typeof PENDING>;
  assert.equal(ps.total, 1);
});

test("only numbers, pixels and fields read elsewhere are asked", () => {
  const doc = elf();
  assert.equal(mayLookUp(doc.node([1, 1, 0])), true);
  assert.equal(mayLookUp(doc.node([1, 1, 1])), true);
  assert.equal(mayLookUp(doc.node([1, 1])), false);
  assert.equal(mayLookUp(doc.node([0, 2, 1])), false);
});

// ----- small helpers -----

test("an entry's label drops the place its list writes in front", () => {
  assert.equal(entryLabel("[0] image width", 0), "image width");
  assert.equal(entryLabel("[3]", 3), "[3]");
  assert.equal(entryLabel("[3] x", 4), "[3] x");
  assert.equal(hexColour(230, 90, 20), "#e65a14");
});

test("the window of a large table keeps the entry near the middle and inside the table", () => {
  assert.deepEqual(windowAround(5, 100, 10), { first: 0, end: 10 });
  assert.deepEqual(windowAround(50, 100, 10), { first: 45, end: 55 });
  assert.deepEqual(windowAround(98, 100, 10), { first: 90, end: 100 });
  assert.deepEqual(windowAround(3, 8, 10), { first: 0, end: 8 });
});

test("entryAt finds the entry holding a bit, and none in a gap", () => {
  const e = (index: number, start: number, end: number): TableEntry => ({ index, path: [index], name: "", label: "", value: "", startBit: start, endBit: end, colour: null, space: 0 });
  const es = [e(4, 0, 8), e(5, 8, 24), e(6, 32, 40)];
  assert.equal(entryAt(es, 0), 4);
  assert.equal(entryAt(es, 23), 5);
  assert.equal(entryAt(es, 24), -1);
  assert.equal(entryAt(es, 39), 6);
  assert.equal(entryAt(es, 40), -1);
});

test("a list placed by offsets out of file order is searched and drawn by its bytes", () => {
  const e = (index: number, start: number, end: number): TableEntry => ({ index, path: [index], name: "", label: "", value: "", startBit: start, endBit: end, colour: null, space: 0 });
  const es = [e(0, 80, 96), e(1, 0, 16), e(2, 40, 56)];
  assert.equal(entryAt(byStart(es), 44), 2);
  assert.equal(entryAt(byStart(es), 90), 0);
  const node = { offset_bits: 0 } as TemplateNode;
  const layout = stripLayout({ counts: "bytes", node, entries: es }, 120);
  assert.deepEqual(layout.domain, [0, 12]);
  assert.deepEqual(layout.cells.map((c) => [c.x0, c.x1]), [[100, 120], [0, 20], [50, 70]]);
});

// ----- the strip -----

test("a string table's strip is drawn in bytes from the table's start", () => {
  const doc = elf();
  const l = lookupFor(doc, doc.node([1, 1, 0])) as Lookup;
  const layout = stripLayout(l.table, 170);
  assert.deepEqual(layout.domain, [0, 17]);
  assert.deepEqual(layout.cells.map((c) => [c.x0, c.x1]), [[0, 10], [10, 110], [110, 170]]);
  // The number 11 lands at byte 11, the start of `.text`.
  assert.equal(unitOf(l.table, l.target, l.hit), 11);
  assert.equal(unitX(layout, 170, 11), 110);
});

test("the axis counts from where the index's number counts from", () => {
  // An ELF name offset counts from its string table: the axis starts there.
  const doc = elf();
  const l = lookupFor(doc, doc.node([1, 1, 0])) as Lookup;
  assert.equal(axisOrigin(l.table, l.target, 11), l.table.node.offset_bits);
  // A TIFF directory offset of 8 counts from the file's start, so the
  // directory at byte 8 is ticked 0x8, not 0x0.
  const e = (index: number, start: number, end: number): TableEntry => ({ index, path: [index], name: "", label: "", value: "", startBit: start, endBit: end, colour: null, space: 0 });
  const dir = { counts: "bytes" as const, node: { offset_bits: 64 } as TemplateNode, entries: [e(0, 64, 80), e(1, 80, 176)] };
  assert.equal(axisOrigin(dir, 64, 8), 0);
  const layout = stripLayout(dir, 100, 0);
  assert.deepEqual(layout.domain, [8, 22]);
  assert.equal(unitOf(dir, 64, 0, 0), 8);
  // A number that cannot count bytes from at or before the table (a count
  // of words, here) leaves the axis counting from the table's start.
  assert.equal(axisOrigin(dir, 64, 20), 64);
  // An entry axis has no origin to find.
  assert.equal(axisOrigin({ ...dir, counts: "entries" }, 64, 8), 64);
});

test("a palette's strip is drawn in entries, and the arrow lands mid-cell", () => {
  const doc = png();
  const l = lookupFor(doc, doc.node([2, 0, 1, 0])) as Lookup;
  const layout = stripLayout(l.table, 300);
  assert.deepEqual(layout.domain, [0, 3]);
  assert.deepEqual(layout.cells.map((c) => c.x0), [0, 100, 200]);
  assert.equal(unitOf(l.table, l.target, l.hit), 2.5);
});

test("the opening view zooms until the entry is wide enough, centred, inside the strip", () => {
  const v = openingView({ x0: 100, x1: 110, index: 0 }, 300, 50, 64);
  assert.equal(v.k, 5);
  assert.equal(v.x, 150 - 525);
  // Wide enough already: no zoom.
  assert.deepEqual(openingView({ x0: 0, x1: 100, index: 0 }, 300, 50, 64), { k: 1, x: 0, y: 0 });
  // At the left end the view stops at the strip's start.
  assert.equal(openingView({ x0: 0, x1: 2, index: 0 }, 300, 50, 64).x, 0);
  // Never past the deepest zoom.
  assert.equal(openingView({ x0: 0, x1: 0.1, index: 0 }, 300, 50, 64).k, 64);
});

test("a structure opens from its start, zoomed for its middle-sized field", () => {
  const cells = [{ x0: 0, x1: 2, index: 0 }, { x0: 2, x1: 14, index: 1 }, { x0: 14, x1: 26, index: 2 }, { x0: 26, x1: 30, index: 3 }];
  const v = openingFromStart(cells, 300, 60, 64);
  assert.equal(v.k, 5);
  assert.equal(v.x, 0);
});

test("the axis keeps the number's tick over any it collides with", () => {
  // Ten pixels a unit, labels 20 pixels wide: ticks every 2 units would
  // touch, so they thin; the hit at 3 always stays.
  const at = (v: number): number => v * 10;
  const ticks = lookupTicks([0, 16], 8, at, 160, () => 20, 3);
  assert.ok(ticks.some((t) => t.value === 3 && t.hit));
  for (let i = 1; i < ticks.length; i++) assert.ok(at((ticks[i] as { value: number }).value) - at((ticks[i - 1] as { value: number }).value) >= 20);
  // A label by an edge runs inward.
  const plain = lookupTicks([0, 16], 4, at, 160, () => 20, null);
  assert.equal(plain[0]?.value, 0);
  assert.equal(plain[0]?.anchor, "start");
  assert.equal(plain[plain.length - 1]?.anchor, "end");
});

test("a step stays inside the entries read, and an off-screen entry is brought to the middle", () => {
  assert.equal(stepEntry(5, 1, 0, 10), 6);
  assert.equal(stepEntry(9, 1, 0, 10), 9);
  assert.equal(stepEntry(0, -1, 0, 10), 0);
  assert.equal(stepEntry(0, -1, -1, 11), -1);
  const view = { k: 4, x: 0, y: 0 };
  assert.equal(inView(view, 10, 20, 100), true);
  assert.equal(inView(view, 50, 60, 100), false);
  const v = centreOn(view, 50, 60, 100);
  assert.equal(v.k, 4);
  assert.equal(inView(v, 50, 60, 100), true);
  // A cell wider than the strip is brought in from its start, a tenth of the
  // way in, so where the number lands is on screen.
  const w = centreOn(view, 40, 80, 100);
  assert.equal(40 * w.k + w.x, 10);
  assert.equal(inView(w, 40, 80, 100), true);
  assert.equal(inView(view, 40, 80, 100), false);
});

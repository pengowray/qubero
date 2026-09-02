// The flattener on trees shaped like the files it has to survive: a SQLite
// database, which is where the mockup's rules came from, and a GGUF, which is
// where they have to stay cheap.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { TemplateNode, TemplateReply } from "../src/doc.ts";
import { emptyState, flatten, pathKey, refold, sectionBreaks } from "../src/flatten.ts";
import type { Item, ListingState, TreeSource } from "../src/flatten.ts";

type Spec = {
  name: string;
  bytes: number;
  /** Where it starts, when that is not straight after the field before it. */
  at?: number;
  kids?: Spec[];
  /** How many children it claims to have, when the fixture writes fewer. */
  count?: number;
  consumed_by?: number;
  machinery?: boolean;
  /** True when the field is only its parent's contents, as `StructDef::contents`
   *  says of a ZIP entry's `body`. */
  contents?: boolean;
  /** The type name, where the rules turn on it: `computed` for a value the
   *  template works out rather than reads. */
  type?: string;
};

type Fixture = { node: TemplateNode; kids: Fixture[] };

function build(spec: Spec, path: number[], start: number): Fixture {
  const at = spec.at ?? start;
  const kids: Fixture[] = [];
  let cursor = at;
  for (const [i, k] of (spec.kids ?? []).entries()) {
    const built = build(k, [...path, i], cursor);
    cursor = (built.node.offset_bits + built.node.size_bits) / 8;
    kids.push(built);
  }
  const composite = spec.kids !== undefined;
  const node: TemplateNode = {
    path,
    name: spec.name,
    type: spec.type ?? (composite ? "struct" : "u8"),
    offset_bits: at * 8,
    size_bits: spec.bytes * 8,
    value: "",
    edit_text: "",
    kind: composite ? "composite" : "uint",
    ok: true,
    child_count: spec.count ?? kids.length,
    composite,
    editable: false,
    space: 0,
    refused: null,
    value_bytes: spec.bytes,
    value_offset_bits: at * 8,
    read_as: null,
    consumed_by: spec.consumed_by ?? null,
    machinery: spec.machinery ?? null,
    contents: spec.contents ?? false,
  };
  return { node, kids };
}

/** A source over a fixture. `absent` names paths whose children are not read
 *  yet, so the pending path can be exercised without a document. */
function source(root: Fixture, absent: ReadonlySet<string> = new Set()): TreeSource {
  const find = (path: readonly number[]): Fixture | null => {
    let at: Fixture | undefined = root;
    for (const i of path) at = at?.kids[i];
    return at ?? null;
  };
  return {
    node(path): TemplateReply<TemplateNode> {
      const f = find(path);
      return f === null ? { status: "error", message: "no such path" } : { status: "ok", node: f.node };
    },
    children(path, from, to): TemplateReply<TemplateNode[]> {
      if (absent.has(pathKey(path))) return { status: "pending", reachedBytes: 64 };
      const f = find(path);
      if (f === null) return { status: "error", message: "no such path" };
      // A fixture may claim more children than it writes: a list of a quarter
      // of a million is described, not built.
      return { status: "ok", node: f.kids.slice(from, to).map((k) => k.node) };
    },
  };
}

function run(spec: Spec, state: ListingState = emptyState, absent?: ReadonlySet<string>) {
  const root = build(spec, [], 0);
  return flatten(source(root, absent), state);
}

function shape(items: readonly Item[]): string[] {
  return items.map((i) => {
    const name = i.kind === "heading" ? (i.node?.name ?? "(run)") : i.kind === "row" ? i.node.name : "";
    const at = (i.offsetBits / 8).toString(16);
    return `${i.section} ${i.kind}${name === "" ? "" : ` ${name}`} @${at}+${i.sizeBits / 8}`;
  });
}

// A SQLite database as the template declares it: a flat structure whose first
// fields are the header and whose last are the pages. Trimmed to the fields
// the rules turn on; the offsets are the real ones.
const PAGE_HEADER: Spec[] = [
  { name: "page_type", bytes: 1, machinery: true },
  { name: "first_freeblock", bytes: 2, machinery: true },
  { name: "cell_count", bytes: 2, consumed_by: 5 },
  { name: "cell_content_start", bytes: 2, machinery: true },
  { name: "fragmented_free_bytes", bytes: 1, machinery: true },
  { name: "cell_pointers", bytes: 6, consumed_by: 6, kids: [{ name: "[0]", bytes: 2 }, { name: "[1]", bytes: 2 }, { name: "[2]", bytes: 2 }] },
];

const page = (at: number, cellsAt: number): Spec => ({
  name: "TableLeaf",
  bytes: 4096,
  at,
  kids: [
    ...PAGE_HEADER,
    {
      name: "cells",
      bytes: 336,
      at: cellsAt,
      kids: [
        { name: "[0]", bytes: 22 },
        { name: "[1]", bytes: 294 },
        { name: "[2]", bytes: 20 },
      ],
    },
  ],
});

const SQLITE: Spec = {
  name: "file",
  bytes: 12288,
  kids: [
    { name: "magic", bytes: 16 },
    { name: "page_size", bytes: 2, consumed_by: 4 },
    { name: "page_count", bytes: 4 },
    { name: "reserved", bytes: 78 },
    { name: "page1", bytes: 3996, at: 100, kids: [...PAGE_HEADER, { name: "cells", bytes: 200, at: 3896, kids: [{ name: "[0]", bytes: 200 }] }] },
    { name: "pages", bytes: 8192, at: 4096, kids: [page(4096, 7856), page(8192, 12000)] },
  ],
};

test("the file divides into its top-level parts", () => {
  const { items } = run(SQLITE);
  const headings = items.filter((i) => i.kind === "heading" && i.level === 0);
  assert.deepEqual(
    headings.map((h) => (h.kind === "heading" ? (h.node?.name ?? "(run)") : "")),
    ["(run)", "page1", "TableLeaf", "TableLeaf"],
  );
  // The header has no field of its own to name it and still runs 0 to 100.
  const header = headings[0];
  assert.equal(header?.kind === "heading" && header.node, null);
  assert.equal(header?.offsetBits, 0);
  assert.equal(header?.sizeBits, 100 * 8);
});

test("a header field that sizes something elsewhere stays a row", () => {
  const { items } = run(SQLITE);
  const names = items.filter((i) => i.kind === "row").map((i) => (i.kind === "row" ? i.node.name : ""));
  assert.ok(names.includes("page_size"), `page_size folded away: ${names.join(", ")}`);
  assert.ok(names.includes("magic"));
});

test("a page header is rows, each saying what reads it", () => {
  const { items } = run(SQLITE);
  const rows = items.filter((i) => i.kind === "row").map((i) => (i.kind === "row" ? [i.node.name, i.reads?.name ?? null] : []));
  // Every field of the page header, in place and in order, and the two of
  // them that something reads say what: a chain, not one lump.
  assert.deepEqual(rows.slice(4, 10), [
    ["page_type", null],
    ["first_freeblock", null],
    ["cell_count", "cell_pointers"],
    ["cell_content_start", null],
    ["fragmented_free_bytes", null],
    ["cell_pointers", "cells"],
  ]);
  // `page_size` sizes pages that are nowhere near it, and says so from the
  // header: a link works at any distance, where hiding it behind the field it
  // places only worked when the two were side by side.
  assert.deepEqual(rows[1], ["page_size", "page1"]);
});

test("the template's word beats what the shapes say, either way", () => {
  const both: Spec = {
    name: "file",
    bytes: 40,
    kids: [
      { name: "page", bytes: 40, kids: [
        // Reads as machinery and is the point: a field the reader came for.
        { name: "width", bytes: 2, consumed_by: 3, machinery: false },
        // Reads as nothing in particular and is plumbing all the same.
        { name: "reserved", bytes: 2, machinery: true },
        { name: "height", bytes: 2, consumed_by: 3 },
        { name: "rows", bytes: 34, kids: [{ name: "r", bytes: 34 }] },
      ] },
    ],
  };
  const items = run(both, { ...emptyState, open: new Set(["0"]) }).items;
  const rows = items.filter((i) => i.kind === "row").map((i) => (i.kind === "row" ? [i.node.name, i.reads?.name ?? null] : []));
  assert.deepEqual(rows, [
    ["width", "rows"],
    ["reserved", null],
    ["height", "rows"],
  ]);
});

test("a field that is only its parent's contents spends no level on itself", () => {
  // A ZIP entry: a signature that picks the record type, and a `body` holding
  // the record. Reading "body" as a heading of its own says nothing.
  const zip: Spec = {
    name: "file",
    bytes: 40,
    kids: [
      { name: "entry", bytes: 40, kids: [
        { name: "signature", bytes: 4, consumed_by: 1 },
        { name: "body", bytes: 36, contents: true, kids: [
          { name: "name_len", bytes: 2, consumed_by: 1 },
          { name: "name", bytes: 34 },
        ] },
      ] },
    ],
  };
  const items = run(zip).items;
  assert.deepEqual(
    items.map((i) => `${i.kind}:${i.kind === "heading" ? (i.node?.name ?? "(run)") : i.kind === "row" ? i.node.name : ""}`),
    // The signature places the body and the length prefix measures the name,
    // and each is alone where it stands. One field is not worth a fold, so
    // both stay where they are and take the dimming instead. What the test is
    // for is the depth: `body` spends no level on itself, so its length sits
    // beside the signature rather than a step in from it.
    ["heading:entry", "row:signature", "row:name_len", "row:name"],
  );
});

test("two lengths for two fields are two rows", () => {
  // A ZIP entry: one length for the name, another for the extra field. They
  // stay where they are, each next to what it measures.
  const two: Spec = {
    name: "file",
    bytes: 30,
    kids: [
      { name: "entry", bytes: 30, kids: [
        { name: "name_len", bytes: 2, consumed_by: 2 },
        { name: "extra_len", bytes: 2, consumed_by: 3 },
        { name: "name", bytes: 13 },
        { name: "extra", bytes: 13 },
      ] },
    ],
  };
  const rows = run(two).items.filter((i) => i.kind === "row");
  assert.deepEqual(rows.map((i) => (i.kind === "row" ? [i.node.name, i.reads?.name ?? null] : [])), [
    ["name_len", "name"],
    ["extra_len", "extra"],
    ["name", null],
    ["extra", null],
  ]);
});

test("asking for a part's bytes opens on its machinery, not on all of it", () => {
  const page = run(SQLITE).items.find((i) => i.kind === "heading" && i.level === 0 && i.offsetBits === 4096 * 8);
  assert.ok(page?.kind === "heading");
  const strip = run(SQLITE, { ...emptyState, bytes: new Set([page.key]) }).items.find((i) => i.kind === "bytes");
  assert.ok(strip?.kind === "bytes");
  // The strip runs from the page's start to where its cells begin, which is
  // less than the page. This fixture declares the cells at the back of the
  // page, so that is 3,760 bytes of it; the real file declares them right
  // after the header and covering the free space, where the same rule gives
  // the fourteen bytes the mockup opens on.
  assert.equal(strip.offsetBits, page.offsetBits);
  assert.ok(strip.sizeBits < page.sizeBits);
  assert.equal(strip.sizeBits / 8, 7856 - 4096);
  assert.equal(strip.owner, page.key);
});

test("a row's bytes are its own", () => {
  const rows = run(SQLITE).items;
  const row = rows.find((i) => i.kind === "row" && i.node.name === "page_size");
  assert.ok(row?.kind === "row");
  const openRow = run(SQLITE, { ...emptyState, bytes: new Set([row.key]) }).items.find((i) => i.kind === "bytes");
  assert.equal(openRow?.sizeBits, 16);
});

test("a strip sits directly under the item it belongs to", () => {
  const rows = run(SQLITE).items;
  const row = rows.find((i) => i.kind === "row" && i.node.name === "page_size");
  assert.ok(row !== undefined);
  const items = run(SQLITE, { ...emptyState, bytes: new Set([row.key]) }).items;
  const at = items.findIndex((i) => i.key === row.key);
  assert.equal(items[at + 1]?.kind, "bytes");
  // And nothing opens a strip nobody asked for.
  assert.equal(run(SQLITE).items.some((i) => i.kind === "bytes"), false);
});

test("bytes no field covers are an item of their own", () => {
  const { items } = run(SQLITE);
  const gaps = items.filter((i) => i.kind === "gap");
  assert.equal(gaps.length, 3);
  const inPage2 = gaps[1];
  // Page 2 runs 0x1000 to 0x1fff: fourteen bytes of header, then free space
  // until the cells at 0x1eb0.
  assert.equal(inPage2?.offsetBits, (4096 + 14) * 8);
  assert.equal(inPage2?.sizeBits, (7856 - 4110) * 8);
});

test("free space at a structure's own edges is accounted for", () => {
  // What a b-tree page does: pointers at the front, cells at the back, and
  // the free space they grow into between them belongs to neither.
  const spaced: Spec = {
    name: "file",
    bytes: 100,
    kids: [
      { name: "page", bytes: 100, kids: [{ name: "cell", bytes: 10, at: 40 }] },
    ],
  };
  const gaps = run(spaced, { ...emptyState, open: new Set(["0"]) }).items.filter((i) => i.kind === "gap");
  assert.deepEqual(
    gaps.map((g) => [g.offsetBits / 8, g.sizeBits / 8]),
    [
      [0, 40],
      [50, 50],
    ],
  );
});

test("the rows of a page are a part of it, not another indent", () => {
  const { items } = run(SQLITE);
  const subs = items.filter((i) => i.kind === "heading" && i.level === 1);
  assert.equal(subs.length, 3);
  assert.deepEqual(subs.map((s) => s.depth), [1, 1, 1]);
});

test("opening a part lists what is in it", () => {
  const closed = run(SQLITE);
  const open = run(SQLITE, { ...emptyState, open: new Set(["4.6"]) });
  assert.ok(open.items.length > closed.items.length);
  const names = open.items.filter((i) => i.kind === "row").map((i) => (i.kind === "row" ? i.node.name : ""));
  assert.ok(names.includes("[0]"));
});

test("children are listed where their bytes are, not where they are declared", () => {
  const scattered: Spec = {
    name: "file",
    bytes: 300,
    kids: [
      { name: "directory", bytes: 100, at: 200 },
      { name: "body", bytes: 200, at: 0 },
    ],
  };
  assert.deepEqual(shape(run(scattered).items).filter((s) => s.includes("row")), ["0 row body @0+200", "0 row directory @c8+100"]);
});

// GGUF: a handful of header fields and then lists long enough that walking
// them all is the whole problem.
const GGUF: Spec = {
  name: "file",
  bytes: 4_000_000,
  kids: [
    { name: "magic", bytes: 4 },
    { name: "version", bytes: 4 },
    { name: "tensor_count", bytes: 8, consumed_by: 4 },
    { name: "metadata_kv_count", bytes: 8, consumed_by: 5 },
    { name: "tensors", bytes: 40_000, at: 24, count: 389, kids: Array.from({ length: 389 }, (_, i) => ({ name: `t${i}`, bytes: 100, kids: [{ name: "n", bytes: 100 }] })) },
    { name: "metadata", bytes: 60_000, at: 40_024, count: 250_000, kids: Array.from({ length: 400 }, (_, i) => ({ name: `k${i}`, bytes: 150 })) },
  ],
};

test("a long list stops at a page and says how much is left", () => {
  const { items } = run(GGUF);
  const more = items.filter((i) => i.kind === "more");
  assert.deepEqual(
    more.map((m) => (m.kind === "more" ? [m.side, m.from, m.to, m.remaining] : [])),
    [
      ["later", 0, 200, 189],
      ["later", 0, 200, 249_800],
    ],
  );
  // 389 tensors and a quarter of a million metadata entries, and the list
  // stays in the hundreds.
  assert.ok(items.length < 900, `${items.length} items`);
});

test("a window partway into a list draws that stretch and both its ends", () => {
  const state: ListingState = { ...emptyState, shown: new Map([["4", { from: 200, to: 389 }]]) };
  const { items } = run(GGUF, state);
  const rows = items.filter((i) => i.kind === "heading" && i.level === 1);
  assert.equal(rows.length, 189);
  assert.equal(rows[0]?.kind === "heading" ? rows[0].node?.name : "", "t200");
  const more = items.filter((i) => i.kind === "more" && pathKey(i.path) === "4");
  assert.deepEqual(
    more.map((m) => (m.kind === "more" ? [m.side, m.remaining] : [])),
    [["earlier", 200]],
  );
});

test("the elements a window leaves out are not a gap in the file", () => {
  const state: ListingState = { ...emptyState, shown: new Map([["4", { from: 100, to: 200 }]]) };
  const { items } = run(GGUF, state);
  // The two ends stand for 100 tensors before and 189 after, and their bytes
  // are the ones those tensors cover. Nothing in the list is unaccounted for.
  const ends = items.filter((i) => i.kind === "more" && pathKey(i.path) === "4");
  assert.deepEqual(
    ends.map((m) => (m.kind === "more" ? [m.side, m.offsetBits / 8, m.sizeBits / 8] : [])),
    [
      ["earlier", 24, 100 * 100],
      ["later", 24 + 200 * 100, 40_000 - 200 * 100],
    ],
  );
  assert.deepEqual(items.filter((i) => i.kind === "gap" && pathKey(i.path) === "4"), []);
});

test("a list too long to be parts of the file is one part", () => {
  const { items } = run(GGUF);
  const headings = items.filter((i) => i.kind === "heading" && i.level === 0);
  assert.deepEqual(
    headings.map((h) => (h.kind === "heading" ? (h.node?.name ?? "(run)") : "")),
    ["(run)", "tensors", "metadata"],
  );
});

test("a short list of small things is one part, not one part each", () => {
  // A GGUF's metadata: a handful of entries, all structures, and one per cent
  // of the file. Three SQLite pages are two thirds of theirs and do divide it.
  const meta: Spec = {
    name: "file",
    bytes: 100_000,
    kids: [
      { name: "magic", bytes: 4 },
      { name: "metadata", bytes: 1_000, kids: [{ name: "kv0", bytes: 300, kids: [{ name: "k", bytes: 300 }] }, { name: "kv1", bytes: 700, kids: [{ name: "k", bytes: 700 }] }] },
      { name: "weights", bytes: 98_996, kids: [{ name: "w", bytes: 98_996 }] },
    ],
  };
  const headings = run(meta).items.filter((i) => i.kind === "heading" && i.level === 0);
  assert.deepEqual(
    headings.map((h) => (h.kind === "heading" ? (h.node?.name ?? "(run)") : "")),
    ["(run)", "metadata", "weights"],
  );
});

test("bytes that have not been read yet are marked, not guessed", () => {
  const { items, pending, reachedBytes } = run(GGUF, emptyState, new Set(["4"]));
  assert.equal(pending, true);
  assert.equal(reachedBytes, 64);
  assert.equal(items.filter((i) => i.kind === "pending").length, 1);
  // The parts that could be read are still there.
  assert.ok(items.some((i) => i.kind === "heading" && i.node?.name === "metadata"));
});

test("a run of plain fields is one part and each composite its own", () => {
  const leaf = (name: string): TemplateNode => build({ name, bytes: 1 }, [], 0).node;
  const comp = (name: string): TemplateNode => build({ name, bytes: 1, kids: [{ name: "x", bytes: 1 }] }, [], 0).node;
  assert.deepEqual(sectionBreaks([leaf("a"), leaf("b"), comp("c"), leaf("d"), comp("e")]), [0, 2, 3, 4]);
  assert.deepEqual(sectionBreaks([]), []);
  // Computed values weigh nothing: they open no part of their own, and go
  // with the leaf run after them or stay in the part before them.
  const none = (name: string): TemplateNode => build({ name, bytes: 0 }, [], 0).node;
  assert.deepEqual(sectionBreaks([leaf("a"), comp("c"), none("x"), none("y"), leaf("d")]), [0, 1, 2]);
  assert.deepEqual(sectionBreaks([leaf("a"), comp("c"), none("x"), comp("e")]), [0, 1, 3]);
  assert.deepEqual(sectionBreaks([none("x"), none("y")]), [0]);
});

// A ZIP entry's tail, where the template stops reading and starts working
// things out: two computed sizes that answer the ZIP64 question, and the data
// one of them measures.
const ZIP_TAIL: Spec = {
  name: "file",
  bytes: 140,
  kids: [
    { name: "name_length", bytes: 2, consumed_by: 2 },
    { name: "extra_length", bytes: 2, consumed_by: 3 },
    { name: "name", bytes: 10 },
    { name: "extra", bytes: 0 },
    { name: "data_size", bytes: 0, type: "computed", consumed_by: 6 },
    { name: "unpacked_size", bytes: 0, type: "computed" },
    { name: "data", bytes: 126 },
  ],
};

test("a value the template works out is never folded away", () => {
  const { items } = run(ZIP_TAIL);
  const names = items.filter((i) => i.kind === "row").map((i) => (i.kind === "row" ? i.node.name : ""));
  assert.ok(names.includes("data_size"), names.join(", "));
  assert.ok(names.includes("unpacked_size"), names.join(", "));
  // A computed value says what reads it like any other field.
  const links = items.filter((i) => i.kind === "row" && i.reads !== null).map((i) => (i.kind === "row" ? [i.node.name, i.reads?.name] : []));
  assert.deepEqual(links, [
    ["name_length", "name"],
    ["extra_length", "extra"],
    ["data_size", "data"],
  ]);
});

test("a field that places another says what reads it", () => {
  const one: Spec = {
    name: "file",
    bytes: 20,
    kids: [
      { name: "count", bytes: 2, consumed_by: 1 },
      { name: "items", bytes: 18 },
    ],
  };
  const { items } = run(one);
  const count = items.find((i) => i.kind === "row" && i.node.name === "count");
  assert.equal(count?.kind === "row" ? count.reads?.name : null, "items");
  const rest = items.find((i) => i.kind === "row" && i.node.name === "items");
  assert.equal(rest?.kind === "row" ? rest.reads : null, null);
});

test("bytes past the last part of the file are not lost", () => {
  // An HDF5 file: the template describes ninety-six bytes and reaches the rest
  // through addresses. The listing has to say so rather than stopping.
  const h5: Spec = {
    name: "file",
    bytes: 471_863_054,
    kids: [
      { name: "signature", bytes: 8 },
      { name: "superblock", bytes: 88, kids: [{ name: "offset_size", bytes: 1 }] },
    ],
  };
  const { items } = run(h5);
  const gaps = items.filter((i) => i.kind === "gap");
  assert.deepEqual(
    gaps.map((g) => (g.kind === "gap" ? [g.offsetBits / 8, g.sizeBits / 8, g.unmapped] : [])),
    [
      // Free space the template left inside the superblock, and then the rest
      // of the file, which is a different claim and says so.
      [9, 87, false],
      [96, 471_862_958, true],
    ],
  );
});

test("a hole between two parts of the file is a row of its own", () => {
  const split: Spec = {
    name: "file",
    bytes: 300,
    kids: [
      { name: "head", bytes: 100, kids: [{ name: "a", bytes: 100 }] },
      { name: "tail", bytes: 100, at: 200, kids: [{ name: "b", bytes: 100 }] },
    ],
  };
  const gaps = run(split).items.filter((i) => i.kind === "gap");
  assert.deepEqual(
    gaps.map((g) => (g.kind === "gap" ? [g.offsetBits / 8, g.sizeBits / 8] : [])),
    [[100, 100]],
  );
});

// Opening a fold is walking one item again, and has to give the same list as
// walking the whole file would. If these two ever part, the listing shows one
// thing after a click and another after the next scroll.

/** The state key a fold on this item turns on and off, as `listingreport`
 *  works it out. */
function foldKey(item: Item): string | null {
  if (item.kind === "heading") return item.node === null || item.level === 0 ? null : pathKey(item.path);
  if (item.kind === "row") return item.node.composite && item.node.child_count > 0 ? pathKey(item.path) : null;
  return null;
}

function ids(items: readonly Item[]): string[] {
  return items.map((i) => `${i.key} ${i.section}/${i.depth} @${i.offsetBits}+${i.sizeBits}`);
}

/** Every fold in the list, opened one at a time and shut again, both ways
 *  round: the spliced list against the walked one. */
function foldsAgree(spec: Spec, start: ListingState = emptyState): number {
  const root = build(spec, [], 0);
  const src = source(root);
  let tried = 0;
  const base = flatten(src, start).items;
  for (const [at, item] of base.entries()) {
    const key = foldKey(item);
    if (key === null) continue;
    const want = !start.open.has(key);
    const open = new Set(start.open);
    if (want) open.add(key);
    else open.delete(key);
    const next: ListingState = { ...start, open };
    const cut = refold(src, next, {}, base, at);
    assert.ok(cut !== null, `no refold for ${item.key}`);
    const patched = [...base.slice(0, cut.from), ...cut.items, ...base.slice(cut.to)];
    assert.deepEqual(ids(patched), ids(flatten(src, next).items), `${want ? "opening" : "closing"} ${key}`);
    tried += 1;
  }
  return tried;
}

test("opening one fold gives the list a whole walk would", () => {
  assert.ok(foldsAgree(SQLITE) > 0);
  assert.ok(foldsAgree(GGUF) > 0);
  assert.ok(foldsAgree(ZIP_TAIL) >= 0);
});

test("closing one fold gives the list a whole walk would", () => {
  const open = new Set(["4.5", "5.0", "5.0.5", "5.1"]);
  assert.ok(foldsAgree(SQLITE, { ...emptyState, open }) > 0);
});

test("a fold walked again keeps the byte strips under it", () => {
  // The strip is part of what an item opens into, so it goes and comes back
  // with the rest of the run rather than being left standing on its own.
  const open = new Set(["4.5"]);
  const bytes = new Set(["r:4.5", "r:4.2"]);
  assert.ok(foldsAgree(SQLITE, { ...emptyState, open, bytes }) > 0);
});

/** Every item that can show its bytes, opened one at a time and shut again:
 *  the spliced list against the walked one. A strip belongs to the item whose
 *  key it is, so that item is the one walked again. */
function stripsAgree(spec: Spec, start: ListingState = emptyState): number {
  const root = build(spec, [], 0);
  const src = source(root);
  let tried = 0;
  const base = flatten(src, start).items;
  for (const [at, item] of base.entries()) {
    if (item.kind !== "row" && item.kind !== "heading") continue;
    if (item.node === null || item.sizeBits === 0) continue;
    const want = !start.bytes.has(item.key);
    const bytes = new Set(start.bytes);
    if (want) bytes.add(item.key);
    else bytes.delete(item.key);
    const next: ListingState = { ...start, bytes };
    const cut = refold(src, next, {}, base, at);
    if (cut === null) continue; // a run heading has no field of its own to walk
    const patched = [...base.slice(0, cut.from), ...cut.items, ...base.slice(cut.to)];
    assert.deepEqual(ids(patched), ids(flatten(src, next).items), `${want ? "opening" : "closing"} bytes of ${item.key}`);
    tried += 1;
  }
  return tried;
}

test("opening one byte strip gives the list a whole walk would", () => {
  assert.ok(stripsAgree(SQLITE) > 0);
  assert.ok(stripsAgree(GGUF) > 0);
  assert.ok(stripsAgree(ZIP_TAIL) >= 0);
});

test("closing one byte strip gives the list a whole walk would", () => {
  const open = new Set(["4.5", "5.0"]);
  const bytes = new Set(["r:4.5", "r:4.2", "h:5.0"]);
  assert.ok(stripsAgree(SQLITE, { ...emptyState, open, bytes }) > 0);
});

// The content card: what an image file is a picture of, ahead of its parts.
test("an image file opens with its picture, which is not a part of the file", () => {
  const root = build(SQLITE, [], 0);
  const src = source(root);
  const plain = flatten(src, emptyState).items;
  const { items } = flatten(src, emptyState, { card: "image" });
  const first = items[0];
  assert.equal(first?.kind, "card");
  assert.equal(first?.kind === "card" ? first.card : null, "image");
  assert.equal(first?.section, -1);
  assert.equal(first?.sizeBits, 0);
  // Everything else is as it was: the same items, the same section numbers.
  assert.deepEqual(shape(items.slice(1)), shape(plain));
  assert.equal(plain.some((i) => i.kind === "card"), false);
  assert.equal(flatten(src, emptyState, { card: null }).items.some((i) => i.kind === "card"), false);
  // Nothing of its own to walk: the caller walks the whole tree.
  assert.equal(refold(src, emptyState, { card: "image" }, items, 0), null);
});

test("a root smaller than its file still accounts for the whole file", () => {
  // An HDF5 root as the template really reports it: ninety-six bytes, with
  // the rest of the file reached by address. The file is what the listing
  // has to account for, not the root.
  const h5: Spec = {
    name: "file",
    bytes: 96,
    kids: [
      { name: "signature", bytes: 8 },
      { name: "superblock", bytes: 88, kids: [{ name: "offset_size", bytes: 1 }] },
    ],
  };
  const root = build(h5, [], 0);
  const { items } = flatten(source(root), emptyState, { fileBits: 113_705_336 * 8 });
  const tail = items[items.length - 1];
  assert.equal(tail?.kind, "gap");
  assert.equal(tail?.kind === "gap" ? tail.unmapped : null, true);
  assert.equal(tail?.offsetBits, 96 * 8);
  assert.equal(tail?.sizeBits, (113_705_336 - 96) * 8);
  // Without the length, the root is all there is to go on.
  const bare = flatten(source(root), emptyState).items;
  assert.equal(bare.some((i) => i.kind === "gap" && i.unmapped && i.offsetBits === 96 * 8), false);
});

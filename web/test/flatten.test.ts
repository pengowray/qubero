// The flattener on trees shaped like the files it has to survive: a SQLite
// database, which is where the mockup's rules came from, and a GGUF, which is
// where they have to stay cheap.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { TemplateNode, TemplateReply } from "../src/doc.ts";
import { emptyState, flatten, pathKey, sectionBreaks } from "../src/flatten.ts";
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
    type: composite ? "struct" : "u8",
    offset_bits: at * 8,
    size_bits: spec.bytes * 8,
    value: "",
    edit_text: "",
    kind: composite ? "composite" : "uint",
    ok: true,
    child_count: spec.count ?? kids.length,
    composite,
    editable: false,
    value_bytes: spec.bytes,
    value_offset_bits: at * 8,
    read_as: null,
    consumed_by: spec.consumed_by ?? null,
    machinery: spec.machinery ?? null,
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

test("a page header folds behind the cells it places", () => {
  const { items } = run(SQLITE);
  const folds = items.filter((i) => i.kind === "fold");
  assert.equal(folds.length, 3);
  const first = folds[0];
  assert.ok(first?.kind === "fold");
  assert.deepEqual(
    first.nodes.map((n) => n.name),
    ["page_type", "first_freeblock", "cell_count", "cell_content_start", "fragmented_free_bytes", "cell_pointers"],
  );
  // One item covering the whole header, so the payload keeps the reader's eye.
  assert.equal(first.sizeBits, 14 * 8);
  assert.equal(first.owner, 6);
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

test("the rows of a page are a part of it, not another indent", () => {
  const { items } = run(SQLITE);
  const subs = items.filter((i) => i.kind === "heading" && i.level === 1);
  assert.equal(subs.length, 3);
  assert.deepEqual(subs.map((s) => s.depth), [1, 1, 1]);
});

test("opening a part lists what is in it", () => {
  const closed = run(SQLITE);
  const open = run(SQLITE, { open: new Set(["4.6"]), shown: new Map() });
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
    more.map((m) => (m.kind === "more" ? [m.shown, m.remaining] : [])),
    [
      [200, 189],
      [200, 249_800],
    ],
  );
  // 389 tensors and a quarter of a million metadata entries, and the list
  // stays in the hundreds.
  assert.ok(items.length < 900, `${items.length} items`);
});

test("a list too long to be parts of the file is one part", () => {
  const { items } = run(GGUF);
  const headings = items.filter((i) => i.kind === "heading" && i.level === 0);
  assert.deepEqual(
    headings.map((h) => (h.kind === "heading" ? (h.node?.name ?? "(run)") : "")),
    ["(run)", "tensors", "metadata"],
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
});

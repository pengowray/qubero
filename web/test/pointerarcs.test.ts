// The DOM-free halves of the pointer arcs: the arithmetic in
// `src/viz/arcmath.ts` (heights, bundling, hit testing, which arcs touch a
// field, the bytes no part covers) and the walk in `src/viz/pointerwalk.ts`
// that finds the arcs by asking a document about every field, run here
// against a small fake document shaped like a PDF's xref and objects.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  arcHeight,
  arcPoint,
  arcShapes,
  arcTouches,
  arcsTouching,
  isForward,
  nearestArc,
  reach,
  roleCounts,
  sideRoom,
  uncovered,
  type PointerArc,
} from "../src/viz/arcmath.ts";
import { childLabel, fileParts, partName, PointerWalk, type WalkDoc } from "../src/viz/pointerwalk.ts";
import type { Origin, TemplateNode, TemplateReply } from "../src/doc.ts";

// ----- arcs -----

const end = (path: number[], startByte: number, bytes: number, label = path.join(".")) => ({ path, startBit: startByte * 8, endBit: (startByte + bytes) * 8, label });
const arc = (role: string, from: ReturnType<typeof end>, to: ReturnType<typeof end>): PointerArc => ({ role, from, to, value: "" });

test("an arc landing later in the file than it leaves is forward", () => {
  assert.equal(isForward(arc("length", end([0], 0, 4), end([1], 4, 16))), true);
  assert.equal(isForward(arc("position", end([9], 7000, 10), end([2], 15, 48))), false);
});

test("arc height grows with reach, keeps a floor and stops at the room", () => {
  const span = 8000 * 8;
  assert.equal(arcHeight(span, span, 90), 90);
  assert.equal(arcHeight(span * 4, span, 90), 90);
  assert.equal(arcHeight(8, span, 90), 3);
  const quarter = arcHeight(span / 4, span, 90);
  assert.ok(Math.abs(quarter - 45) < 1e-9, "a quarter of the file reaches half the room");
  assert.ok(arcHeight(100 * 8, span, 90) < arcHeight(1000 * 8, span, 90));
});

test("shapes put the left end first and say which side", () => {
  const back = arc("position", end([4], 900, 10), end([2], 100, 20));
  const [s] = arcShapes([back], (b) => b / 8, { left: 0, right: 1000, spanBits: 8000, maxPx: 90 });
  assert.ok(s !== undefined);
  assert.equal(s.x0, 100);
  assert.equal(s.x1, 905);
  assert.equal(s.up, false);
});

test("arcs entirely off screen are left out, and one spanning the view is kept", () => {
  const a = arc("length", end([0], 0, 2), end([1], 2, 2));
  const across = arc("position", end([3], 0, 2), end([4], 990, 2));
  const shapes = arcShapes([a, across], (b) => b / 8, { left: 400, right: 600, spanBits: 8000, maxPx: 90 });
  assert.equal(shapes.length, 1);
  assert.equal(shapes[0]?.arcs[0], across);
});

test("past the limit, arcs with both ends in the same buckets become one bundle", () => {
  const arcs: PointerArc[] = [];
  for (let i = 0; i < 40; i++) arcs.push(arc("position", end([9, i], 1000 + i * 0.01, 1), end([2, i], 10 + i * 0.01, 1)));
  arcs.push(arc("length", end([0], 0, 1), end([1], 1, 1)));
  const shapes = arcShapes(arcs, (b) => b / 8, { left: 0, right: 2000, spanBits: 16000, maxPx: 90 }, 10, 4);
  assert.equal(shapes.length, 2);
  const bundle = shapes.find((s) => s.role === "position");
  assert.equal(bundle?.arcs.length, 40);
  // Under the limit nothing is bundled.
  assert.equal(arcShapes(arcs, (b) => b / 8, { left: 0, right: 2000, spanBits: 16000, maxPx: 90 }).length, 41);
});

test("the top of a forward arc is above the axis and of a backward one below", () => {
  const up = { x0: 0, x1: 100, ry: 30, up: true, arcs: [], role: "length" };
  const down = { ...up, up: false };
  assert.deepEqual(arcPoint(up, 50, 0.5), { x: 50, y: 20 });
  const p = arcPoint(down, 50, 0.5);
  assert.ok(Math.abs(p.x - 50) < 1e-9 && Math.abs(p.y - 80) < 1e-9);
  const start = arcPoint(up, 50, 0);
  assert.ok(Math.abs(start.x) < 1e-9 && Math.abs(start.y - 50) < 1e-9);
});

test("the hit test takes an end before a curve and says which end", () => {
  const back = arc("position", end([4], 100, 1), end([2], 0, 1));
  const [s] = arcShapes([back], (b) => b / 8, { left: 0, right: 200, spanBits: 1600, maxPx: 60 });
  assert.ok(s !== undefined);
  // Near the right end, which is where the backward arc leaves from.
  assert.equal(nearestArc([s], s.x1 - 3, 52, 50)?.at, "from");
  assert.equal(nearestArc([s], s.x0 + 2, 51, 50)?.at, "to");
  // On the curve at its lowest point, far from either end.
  const mid = arcPoint(s, 50, 0.5);
  assert.equal(nearestArc([s], mid.x, mid.y + 2, 50)?.at, "curve");
  // On the other side of the axis, nothing.
  assert.equal(nearestArc([s], mid.x, 50 - 30, 50), null);
});

test("an arc touches a field it ends in, one inside it, and one around it", () => {
  const a = arc("position", end([4, 0, 2, 0], 7058, 11), end([9, 2], 15, 48));
  assert.equal(arcTouches(a, [9, 2]), true);
  assert.equal(arcTouches(a, [9, 2, 0]), true, "a field inside the object lights the pointer to it");
  assert.equal(arcTouches(a, [4, 0]), true, "the whole table lights each of its pointers");
  assert.equal(arcTouches(a, [9, 3]), false);
  assert.equal(arcsTouching([a], [9, 3]).length, 0);
});

test("an arc landing inside a part's bytes touches it, whatever its path", () => {
  // A ZIP central directory entry places a field of its own entry over the
  // bytes of the first record.
  const a = arc("position", end([0, 4, 1, 15], 221, 4), end([0, 4, 1, 19, 0], 0, 46));
  assert.equal(arcTouches(a, [0, 0]), false);
  assert.equal(arcTouches(a, [0, 0], { startBit: 0, endBit: 76 * 8 }), true);
  assert.equal(arcTouches(a, [0, 2], { startBit: 92 * 8, endBit: 163 * 8 }), false);
});

test("role counts follow the given order, then the rest", () => {
  const arcs = [arc("type", end([0], 0, 1), end([1], 1, 1)), arc("length", end([0], 0, 1), end([1], 1, 1)), arc("length", end([2], 2, 1), end([3], 3, 1)), arc("odd", end([2], 2, 1), end([3], 3, 1))];
  assert.deepEqual(roleCounts(arcs, ["length", "position", "type"]), [
    { role: "length", count: 2 },
    { role: "type", count: 1 },
    { role: "odd", count: 1 },
  ]);
});

test("uncovered finds the gaps between parts, before the first and after the last", () => {
  const parts = [
    { startBit: 8, endBit: 16 },
    { startBit: 24, endBit: 40 },
    { startBit: 30, endBit: 48 },
  ];
  assert.deepEqual(uncovered(parts, 64), [
    { startBit: 0, endBit: 8 },
    { startBit: 16, endBit: 24 },
    { startBit: 48, endBit: 64 },
  ]);
  assert.deepEqual(uncovered([{ startBit: 0, endBit: 64 }], 64), []);
});

test("side room is the tallest arc on each side, with a floor", () => {
  const back = arc("position", end([9], 7000, 8), end([2], 0, 8));
  const room = sideRoom([back], 7008 * 8, 96);
  assert.equal(room.up, 16);
  assert.ok(room.down > 80 && room.down <= 96);
});

test("reach is in bytes from the pointer to its target", () => {
  assert.deepEqual(reach(arc("position", end([4], 7058, 11), end([9], 15, 48))), { bytes: 7043, forward: false });
});

// ----- the walk, over a fake document -----

type Fake = { node: TemplateNode; kids: Fake[]; origins: Origin[] };

function field(name: string, startByte: number, bytes: number, opts: Partial<TemplateNode> & { origins?: Origin[] } = {}, kids: Fake[] = []): Fake {
  const node = {
    path: [],
    name,
    type: "",
    offset_bits: startByte * 8,
    size_bits: bytes * 8,
    value: "",
    edit_text: "",
    kind: kids.length > 0 ? "composite" : "uint",
    problems_within: [0, 0],
    child_count: kids.length,
    composite: kids.length > 0,
    list: false,
    editable: false,
    inline: false,
    value_bytes: bytes,
    value_offset_bits: startByte * 8,
    read_at: null,
    read_as: null,
    consumed_by: null,
    machinery: null,
    contents: false,
    framed: false,
    space: 0,
    refused: null,
    decoded: false,
    space_root: false,
    joined: false,
    lsb_first: false,
    ...opts,
  } as unknown as TemplateNode;
  return { node, kids, origins: opts.origins ?? [] };
}

function withPaths(f: Fake, path: number[]): Fake {
  (f.node as { path: readonly number[] }).path = path;
  f.kids.forEach((k, i) => withPaths(k, [...path, i]));
  return f;
}

function fakeDoc(root: Fake, pendingFirst = 0): WalkDoc & { calls: number } {
  const at = (path: readonly number[]): Fake | null => {
    let f: Fake | undefined = root;
    for (const step of path) f = f?.kids[step];
    return f ?? null;
  };
  let pending = pendingFirst;
  const doc = {
    calls: 0,
    templateNode(path: readonly number[]): TemplateReply<TemplateNode> {
      const f = at(path);
      return f === null ? { status: "error", message: "no such field" } : { status: "ok", node: f.node };
    },
    templateChildren(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]> {
      doc.calls++;
      if (pending > 0) {
        pending--;
        return { status: "pending", reachedBytes: 0 };
      }
      const f = at(path);
      return f === null ? { status: "error", message: "no such field" } : { status: "ok", node: f.kids.slice(from, to).map((k) => k.node) };
    },
    origins(path: readonly number[]): TemplateReply<Origin[]> {
      return { status: "ok", node: at(path)?.origins ?? [] };
    },
  };
  return doc;
}

const origin = (role: Origin["role"], path: number[], label: string, value: string, target_bits: number | null = null): Origin => ({ role, label, stored: null, path, value, target_bits });

/** A file shaped like a small PDF: a header, two objects the xref places, a
 *  stream whose inside is another address space, and the xref at the end
 *  with an entry per object. */
function pdfLike(): Fake {
  const entry = (i: number, at: number, target: number) =>
    field(`[${i}]`, at, 20, {}, [field("offset", at, 10, { value: String(target), origins: [origin("points", [], "objects", String(target), target * 8)] }), field("generation", at + 10, 10)]);
  const obj = (i: number, at: number, bytes: number) => field(`[${i}] ${i + 1}`, at, bytes, { origins: [origin("position", [3, 0, i, 0], `entries[${i}].offset`, "")] }, [field("number", at, 2)]);
  const stream = field("data", 100, 20, { decoded: true }, [field("inner", 0, 20, { space: 1, origins: [origin("length", [0], "version", "20")] })]);
  const root = field("file", 0, 400, {}, [
    field("version", 0, 9),
    field("objects", 10, 100, { list: true }, [obj(0, 10, 40), obj(1, 60, 40)]),
    stream,
    field("xref", 300, 40, {}, [field("entries", 300, 40, { list: true }, [entry(0, 300, 10), entry(1, 320, 60)])]),
  ]);
  return withPaths(root, []);
}

test("the walk finds the xref's arc to each object, once each, named from the tree", () => {
  const walk = new PointerWalk(fakeDoc(pdfLike()));
  assert.equal(walk.step(Number.POSITIVE_INFINITY, () => 0), "done");
  const positions = walk.arcs.filter((a) => a.role === "position");
  assert.equal(positions.length, 2, "the points origin and the position origin are one arc");
  const first = positions.find((a) => a.to.startBit === 10 * 8);
  assert.ok(first !== undefined);
  assert.deepEqual(first.to.path, [1, 0], "the field end replaced the bare bit");
  assert.equal(first.to.label, "objects[0]");
  assert.equal(first.from.label, "xref.entries[0].offset");
  assert.equal(first.value, "10", "the offset field's own value");
  assert.equal(isForward(first), false);
});

test("the walk stays out of a decoded stream's own address space", () => {
  const walk = new PointerWalk(fakeDoc(pdfLike()));
  walk.step(Number.POSITIVE_INFINITY, () => 0);
  assert.equal(walk.arcs.some((a) => a.role === "length"), false);
});

test("the walk waits on bytes and carries on where it stopped", () => {
  const doc = fakeDoc(pdfLike(), 2);
  const walk = new PointerWalk(doc);
  assert.equal(walk.step(Number.POSITIVE_INFINITY, () => 0), "pending");
  assert.equal(walk.step(Number.POSITIVE_INFINITY, () => 0), "pending");
  assert.equal(walk.step(Number.POSITIVE_INFINITY, () => 0), "done");
  assert.equal(walk.arcs.filter((a) => a.role === "position").length, 2);
});

test("the walk stops at its deadline and at its field limit", () => {
  let t = 0;
  const walk = new PointerWalk(fakeDoc(pdfLike()));
  assert.equal(
    walk.step(1, () => t++),
    "more",
  );
  assert.equal(walk.done, false);
  const capped = new PointerWalk(fakeDoc(pdfLike()), 4);
  assert.equal(capped.step(Number.POSITIVE_INFINITY, () => 0), "done");
  assert.equal(capped.capped, true);
  assert.ok(capped.visited <= 5);
});

test("child labels index list elements and skip contents and wrappers", () => {
  const list = field("records", 0, 10, { list: true }, [field("[4] notes.txt", 0, 10)]);
  withPaths(list, [0]);
  assert.equal(childLabel(list.node, "records", list.kids[0]!.node), "records[0]");
  const body = field("body", 4, 6, { contents: true });
  assert.equal(childLabel(field("[4]", 0, 10).node, "records[4]", body.node), "records[4]");
  const wrapper = field("local_header", 0, 0, { composite: true, child_count: 1 } as Partial<TemplateNode>);
  assert.equal(childLabel(wrapper.node, "records[4].local_header", field("local_header measurements.csv", 0, 46).node), "records[4].local_header");
});

test("part names drop the index a list element's name starts with", () => {
  assert.equal(partName(field("[2] notes.txt", 0, 1).node), "notes.txt");
  assert.equal(partName(field("[2]", 0, 1).node), "[2]");
  assert.equal(partName(field("trailer", 0, 1).node), "trailer");
});

test("file parts open out a part that takes most of the file", () => {
  const parts = fileParts(fakeDoc(pdfLike()), 400 * 8);
  assert.ok(parts !== null);
  // `objects` is only a quarter of this file, so it stays one part; the
  // records of a ZIP below are the whole file and are opened out.
  assert.deepEqual(
    parts.map((p) => p.name),
    ["version", "objects", "data", "xref"],
  );
  const zip = withPaths(field("file", 0, 300, {}, [field("records", 0, 300, { list: true }, [field("[0] a.txt", 0, 100, {}, [field("sig", 0, 4)]), field("[1] b.txt", 100, 200, {}, [field("sig", 100, 4)])])]), []);
  const zparts = fileParts(fakeDoc(zip), 300 * 8);
  assert.deepEqual(
    zparts?.map((p) => p.name),
    ["[0] a.txt", "[1] b.txt"],
    "the listing's name, index and all, so two parts of one name differ",
  );
  assert.deepEqual(
    zparts?.map((p) => p.label),
    ["records[0]", "records[1]"],
    "named as the arcs name them",
  );
  assert.equal(fileParts(fakeDoc(pdfLike(), 1), 400 * 8), null, "null while the tree is read");
});

test("an arc says what its field holds, from the tree, and a field only measured is no arc", () => {
  // `tail` is placed at `at + sizeof(at)`: `at` holds 30, and is 4 bytes
  // long, which the core answers with as the value of `size of at`.
  const root = withPaths(
    field("file", 0, 40, {}, [
      field("at", 0, 4, { value: "30" }),
      field("list", 4, 20, { list: true }, [field("[0]", 4, 20)]),
      field("tail", 30, 10, { origins: [origin("position", [0], "at", "30"), origin("position", [0], "size of at", "4"), origin("position", [1], "size of list", "20")] }),
    ]),
    [],
  );
  const walk = new PointerWalk(fakeDoc(root));
  walk.step(Number.POSITIVE_INFINITY, () => 0);
  assert.equal(walk.arcs.length, 1, "the two sizeof origins draw nothing");
  assert.equal(walk.arcs[0]?.from.label, "at");
  assert.equal(walk.arcs[0]?.value, "30");
  // A count read from a list says nothing as a value: the listing's value of
  // a structure is how many fields it has.
  const counted = withPaths(field("file", 0, 40, {}, [field("list", 0, 20, { list: true, value: "1" }, [field("[0]", 0, 20)]), field("tail", 30, 10, { origins: [origin("count", [0], "list", "7")] })]), []);
  const w2 = new PointerWalk(fakeDoc(counted));
  w2.step(Number.POSITIVE_INFINITY, () => 0);
  assert.equal(w2.arcs[0]?.value, "");
});

test("file parts open a structure that is most of the file, among small fields", () => {
  // An ELF: the identification bytes, then a header spanning the rest of the
  // file with its tables inside, beside a field of no bytes.
  const elf = withPaths(
    field("file", 0, 100, {}, [field("magic", 0, 4), field("class", 4, 1), field("header", 5, 95, {}, [field("entry", 5, 8), field("sections", 13, 87)]), field("arch", 0, 0)]),
    [],
  );
  assert.deepEqual(
    fileParts(fakeDoc(elf), 100 * 8)?.map((p) => p.label),
    ["magic", "class", "header.entry", "header.sections"],
  );
});

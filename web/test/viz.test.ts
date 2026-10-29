// The DOM-free parts of the shared diagram infrastructure in `src/viz/`: lane
// assignment, label fitting with a fake measure, the level-of-detail table,
// the linked hover and selection bus, the registries, the canvas hit test and
// the zoom arithmetic behind the board.

import { test } from "node:test";
import assert from "node:assert/strict";

import { assignLanes, depthOf } from "../src/viz/lanes.ts";
import { ellipsise, fitLabel, freeRect, monoMeasure, thinLabels, type Measure } from "../src/viz/fit.ts";
import { byteDetail, bytePx, LOD, opensChildren } from "../src/viz/lod.ts";
import { LinkedBus, linkedFor, subscribeWhileConnected, type LinkedChange } from "../src/viz/linked.ts";
import { Registry } from "../src/viz/registry.ts";
import { nearestTarget, nearestOnAxis, pickFromData } from "../src/viz/pickmath.ts";
import { applyX, fitTransform, IDENTITY, invertX, rescaleDomain } from "../src/viz/boardmath.ts";
import { roleColor, ROLES } from "../src/fieldstyle.ts";
import { inspectorSectionsFor, registerInspectorSection } from "../src/viz/inspectorsections.ts";
import { registerRowGlyph, rowGlyphsFor } from "../src/viz/rowglyphs.ts";
import { overviewTabs, registerOverviewTab } from "../src/viz/overviewtabs.ts";
import { headingButtonsFor, registerHeadingButton, type HeadingItem } from "../src/viz/headingbuttons.ts";
import type { Doc, TemplateNode } from "../src/doc.ts";
import type { DrawContext } from "../src/listingdraw.ts";
import { childStripLayout, hexTicks } from "../src/viz/childstrip.ts";

// ----- lanes -----

test("intervals that do not overlap share lane 0", () => {
  const { lane, count } = assignLanes([{ start: 0, end: 4 }, { start: 4, end: 8 }, { start: 10, end: 12 }], (s) => s);
  assert.deepEqual(lane, [0, 0, 0]);
  assert.equal(count, 1);
});

test("overlapping intervals take the first free lane, and the count is the depth", () => {
  const spans = [
    { start: 0, end: 10 },
    { start: 2, end: 5 },
    { start: 3, end: 4 },
    { start: 6, end: 9 },
    { start: 11, end: 12 },
  ];
  const { lane, count } = assignLanes(spans, (s) => s);
  assert.deepEqual(lane, [0, 1, 2, 1, 0]);
  assert.equal(count, 3);
  assert.equal(depthOf(spans), 3);
});

test("an enclosing interval sits nearer the axis than one starting with it", () => {
  // Given inner first: the order of the input must not decide the nesting.
  const { lane } = assignLanes([{ start: 0, end: 2 }, { start: 0, end: 8 }], (s) => s);
  assert.deepEqual(lane, [1, 0]);
});

test("a gap keeps touching intervals apart", () => {
  const spans = [{ start: 0, end: 4 }, { start: 4, end: 8 }];
  assert.equal(assignLanes(spans, (s) => s, 0).count, 1);
  assert.equal(assignLanes(spans, (s) => s, 1).count, 2);
});

test("no intervals, no lanes", () => {
  assert.deepEqual(assignLanes([], (s: { start: number; end: number }) => s), { lane: [], count: 0 });
});

// ----- fit -----

/** Ten pixels a character at size 10, whatever the face: easy to reason about. */
const fake: Measure = (text, size) => text.length * size;

test("a label is drawn at the largest size it fits at", () => {
  const opts = { sizes: [20, 10], minChars: 3, padX: 0, padY: 0, measure: fake };
  // `abc` at 20 needs 60 pixels across and 22 down.
  assert.equal(fitLabel("abc", 60, 22, opts)?.size, 20);
  assert.equal(fitLabel("abc", 59, 22, opts)?.size, 10);
  assert.equal(fitLabel("abc", 29, 22, opts), null);
});

test("a long label fits once its first characters do, and wants its whole width", () => {
  const fit = fitLabel("abcdefghij", 40, 20, { sizes: [10], minChars: 4, padX: 0, padY: 0, measure: fake });
  assert.notEqual(fit, null);
  assert.equal(fit?.wanted, 100);
});

test("a tall narrow box takes the label sideways only when allowed", () => {
  const opts = { sizes: [10], minChars: 3, padX: 0, padY: 0, measure: fake };
  assert.equal(fitLabel("abc", 12, 40, opts), null);
  assert.equal(fitLabel("abc", 12, 40, { ...opts, sideways: true })?.sideways, true);
});

test("the monospace measure is a ratio of the size", () => {
  assert.equal(monoMeasure(0.6)("abcde", 10), 30);
});

test("ellipsise keeps a fitting text whole, cuts a long one, and drops a short one", () => {
  assert.equal(ellipsise("abc", 30, 10, fake, 2), "abc");
  // Room for 5 characters at 10 each: 4 letters and the ellipsis.
  assert.equal(ellipsise("abcdefgh", 50, 10, fake, 3), "abcd…");
  // A short text is never cut: `0x8…` would be a different byte.
  assert.equal(ellipsise("0x8d", 30, 10, fake, 4), null);
  assert.equal(ellipsise("abcdefgh", 20, 10, fake, 3), null);
});

test("a name that lands on one already placed steps down inside its box, or is dropped", () => {
  const placed = [{ x0: 0, y0: 0, x1: 50, y1: 10 }];
  assert.deepEqual(freeRect({ x0: 0, y0: 0, x1: 20, y1: 10 }, placed, 100), { x0: 0, y0: 10, x1: 20, y1: 20 });
  assert.equal(freeRect({ x0: 0, y0: 0, x1: 20, y1: 10 }, placed, 15), null);
});

test("thinLabels keeps the important labels and drops the ones they would cover", () => {
  const keep = thinLabels([
    { start: 0, end: 10, priority: 1 },
    { start: 5, end: 15, priority: 2 },
    { start: 16, end: 20, priority: 0 },
    { start: 14, end: 17, priority: 0 },
  ]);
  assert.deepEqual(keep, [false, true, true, false]);
  assert.deepEqual(thinLabels([{ start: 0, end: 10, priority: 0 }, { start: 11, end: 20, priority: 0 }], 2), [true, false]);
});

// ----- lod -----

test("a byte cell shows colour, then hex, then bits as it widens", () => {
  assert.equal(byteDetail(LOD.hexPx - 1), "colour");
  assert.equal(byteDetail(LOD.hexPx), "hex");
  assert.equal(byteDetail(LOD.bitsPx), "bits");
  assert.equal(opensChildren(23), false);
  assert.equal(opensChildren(24), true);
  assert.equal(bytePx(100, 0), 0);
  assert.equal(bytePx(100, 50), 2);
});

// ----- linked -----

test("the bus tells listeners what changed, once per change", () => {
  const bus = new LinkedBus();
  const heard: LinkedChange[] = [];
  const stop = bus.subscribe((_s, changed) => heard.push(changed));
  bus.publish({ hover: { startBit: 0, endBit: 8 } });
  bus.publish({ hover: { startBit: 0, endBit: 8 } });
  bus.publish({ selection: { path: [1, 2], startBit: 8, endBit: 16 } });
  bus.publish({ selection: { path: [1, 2], startBit: 8, endBit: 16 } });
  bus.publish({ hover: null, selection: null });
  assert.deepEqual(heard, ["hover", "selection", "hover", "selection"]);
  assert.equal(bus.state.hover, null);
  stop();
  bus.publish({ hover: { startBit: 1, endBit: 2 } });
  assert.equal(heard.length, 4);
  assert.equal(bus.listening, 0);
});

test("a field left out of a publish keeps its value", () => {
  const bus = new LinkedBus();
  bus.publish({ selection: { path: [3], startBit: 0, endBit: 8 } });
  bus.publish({ hover: { startBit: 0, endBit: 1 } });
  assert.deepEqual(bus.state.selection?.path, [3]);
});

test("a listener tied to an element lets go once the element has left the page", () => {
  const bus = new LinkedBus();
  const el = { isConnected: false };
  let heard = 0;
  subscribeWhileConnected(bus, el, () => heard++);
  // Not on the page yet: still listening.
  bus.publish({ hover: { startBit: 0, endBit: 1 } });
  el.isConnected = true;
  bus.publish({ hover: { startBit: 0, endBit: 2 } });
  el.isConnected = false;
  bus.publish({ hover: { startBit: 0, endBit: 3 } });
  assert.equal(heard, 2);
  assert.equal(bus.listening, 0);
});

test("each document has its own bus", () => {
  const a = {};
  const b = {};
  assert.equal(linkedFor(a), linkedFor(a));
  assert.notEqual(linkedFor(a), linkedFor(b));
});

// ----- registries -----

test("a registry keeps the order entries were added, and refuses a second id", () => {
  const r = new Registry<{ id: string; n: number }>("test entry");
  r.add({ id: "a", n: 1 });
  r.add({ id: "b", n: 2 });
  assert.deepEqual(r.all().map((e) => e.id), ["a", "b"]);
  assert.equal(r.get("b")?.n, 2);
  assert.throws(() => r.add({ id: "a", n: 3 }), /registered twice/);
});

test("the host registries hand back only the entries whose predicate matches", () => {
  const doc = {} as Doc;
  const leaf = { composite: false } as TemplateNode;
  const struct = { composite: true } as TemplateNode;
  registerInspectorSection({ id: "test-structs", applies: (_d, n) => n.composite, draw: () => null });
  registerRowGlyph({ id: "test-leaves", applies: (_d, n) => !n.composite, draw: () => null });
  registerOverviewTab({ id: "test-tab", label: "Test", title: "Test", offers: () => true, mount: () => ({ el: {} as HTMLElement }) });
  assert.deepEqual(inspectorSectionsFor(doc, struct).map((s) => s.id).filter((id) => id.startsWith("test-")), ["test-structs"]);
  assert.deepEqual(inspectorSectionsFor(doc, leaf).filter((s) => s.id.startsWith("test-")), []);
  assert.deepEqual(rowGlyphsFor(doc, leaf).map((g) => g.id).filter((id) => id.startsWith("test-")), ["test-leaves"]);
  assert.ok(overviewTabs().some((t) => t.id === "test-tab"));
  assert.throws(() => registerOverviewTab({ id: "test-tab", label: "", title: "", offers: () => false, mount: () => ({ el: {} as HTMLElement }) }));
  // A heading with no field of its own is offered nothing.
  registerHeadingButton({ id: "test-button", applies: () => true, draw: () => ({}) as HTMLElement });
  assert.deepEqual(headingButtonsFor({} as DrawContext, { node: null } as HeadingItem), []);
  assert.equal(headingButtonsFor({} as DrawContext, { node: struct } as HeadingItem).length >= 1, true);
});

// ----- pick -----

test("the hit test prefers a target under the pointer, the innermost first", () => {
  const targets = [
    { x0: 0, y0: 0, x1: 100, y1: 20, value: "outer" },
    { x0: 10, y0: 0, x1: 30, y1: 20, value: "inner" },
  ];
  assert.equal(nearestTarget(targets, 20, 10)?.value, "inner");
  assert.equal(nearestTarget(targets, 50, 10)?.value, "outer");
});

test("a thin target is picked within the minimum target size", () => {
  const targets = [{ x0: 50, y0: 0, x1: 52, y1: 20, value: "sliver" }];
  // 24 pixels wide in all: 11 either side of a 2-pixel bar.
  assert.equal(nearestTarget(targets, 62, 10)?.value, "sliver");
  assert.equal(nearestTarget(targets, 64, 10), null);
  // Inside the padding of two, the nearer one wins.
  const two = [...targets, { x0: 60, y0: 0, x1: 61, y1: 20, value: "next" }];
  assert.equal(nearestTarget(two, 58, 10)?.value, "next");
});

test("the axis hit test finds the stretch under x, or the nearest within reach", () => {
  const spans = [
    { start: 0, end: 10, value: "a" },
    { start: 10, end: 11, value: "b" },
    { start: 40, end: 60, value: "c" },
  ];
  assert.equal(nearestOnAxis(spans, 5)?.value, "a");
  assert.equal(nearestOnAxis(spans, 10.5)?.value, "b");
  assert.equal(nearestOnAxis(spans, 20)?.value, "b");
  // `c` is 20 wide, so it reaches only 2 pixels past its edges.
  assert.equal(nearestOnAxis(spans, 39)?.value, "c");
  assert.equal(nearestOnAxis(spans, 37), null);
  assert.equal(nearestOnAxis(spans, 25), null);
  assert.equal(nearestOnAxis([], 0), null);
});

test("the click contract reads a field, a bit range, or nothing", () => {
  assert.deepEqual(pickFromData({ reads: "1.2", at: "16", size: "8" }), { kind: "field", pick: { path: [1, 2], startBit: 16, endBit: 24 } });
  assert.deepEqual(pickFromData({ reads: "", at: "0", size: "8" }), { kind: "field", pick: { path: [], startBit: 0, endBit: 8 } });
  assert.deepEqual(pickFromData({ at: "3", size: "4" }), { kind: "bits", ranges: [{ startBit: 3, endBit: 7 }] });
  assert.equal(pickFromData({ size: "4" }), null);
  assert.equal(pickFromData({ at: "x", size: "4" }), null);
});

// ----- board arithmetic -----

test("a transform maps x both ways and rescales a domain", () => {
  const t = { k: 2, x: -50, y: 0 };
  assert.equal(applyX(t, 100), 150);
  assert.equal(invertX(t, 150), 100);
  // Zoomed in twice and panned 50 pixels: the 500 pixels on screen show 250
  // pixels of the layout, from 25, which is 500 units of the domain from 50.
  assert.deepEqual(rescaleDomain(t, [0, 1000], [0, 500]), [50, 550]);
  assert.deepEqual(rescaleDomain(IDENTITY, [0, 1000], [0, 500]), [0, 1000]);
});

test("fit centres the content and scales it to the view, x only when asked", () => {
  const box = { x0: 0, y0: 0, x1: 200, y1: 100 };
  assert.deepEqual(fitTransform(box, 400, 400, false, 0), { k: 2, x: 0, y: 100 });
  assert.deepEqual(fitTransform(box, 400, 400, true, 0), { k: 2, x: 0, y: 0 });
});

// ----- the example piece -----

test("the example strip puts overlapping children in separate lanes", () => {
  const node = (offset_bits: number, size_bits: number) => ({ offset_bits, size_bits, path: [offset_bits] }) as unknown as TemplateNode;
  const parent = node(0, 64);
  const layout = childStripLayout(parent, [node(0, 32), node(32, 32), node(16, 16), node(80, 0)], 64);
  assert.equal(layout.lanes, 2);
  assert.deepEqual(layout.bars.map((b) => [b.x0, b.x1, b.lane]), [[0, 32, 0], [32, 64, 0], [16, 32, 1]]);
});

test("hex ticks are a power of two apart and their labels stay inside the axis", () => {
  // 0x10 to 0x1d across 280 pixels, as the IHDR strip is drawn.
  const at = (b: number) => ((b - 16) / 13) * 280;
  const ticks = hexTicks([16, 29], 3, at, 280, () => 30);
  assert.deepEqual(ticks.map((t) => t.byte), [16, 24]);
  assert.equal(ticks[0]?.anchor, "start");
  assert.equal(ticks[1]?.anchor, "middle");
  // Labels wider than the gaps between ticks are thinned.
  assert.equal(hexTicks([0, 64], 64, (b) => b * 4, 256, () => 30).length <= 7, true);
});

// ----- role colours -----

test("every role has a colour in both themes, and an unknown role is grey", () => {
  for (const role of ROLES) {
    assert.match(roleColor(role, false), /^#[0-9a-f]{6}$/);
    assert.match(roleColor(role, true), /^#[0-9a-f]{6}$/);
  }
  assert.equal(roleColor("position", false), roleColor("points", false));
  assert.equal(roleColor("no such role", false), "#8a8f98");
});

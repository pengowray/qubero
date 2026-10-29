// The DOM-free half of the value plot (`src/viz/valueplot.ts`): reading a
// run's element type, decoding its bytes, min-max decimation, the grid's
// per-pixel means and its colour scale, and which Steim frame a sample came
// from.

import { test } from "node:test";
import assert from "node:assert/strict";

import { decode, elementType, envelope, extentOf, frameCount, nearestFrame, numType, rangeOf, valueText } from "../src/viz/decimate.ts";
import { colourRange, mix, parseHex, pixelBlock, ramp, rampIndex, sampleGrid } from "../src/viz/gridmath.ts";
import { frameOfSample } from "../src/samplespanel.ts";
import type { SamplesInfo } from "../src/doc.ts";

// ----- types and decoding -----

test("number types are read from the core's spelling, and anything else is refused", () => {
  assert.equal(elementType("i16 le[]"), "i16 le");
  assert.deepEqual(numType("i16 le"), { name: "i16 le", bytes: 2, float: false, signed: true, le: true });
  assert.deepEqual(numType("f32 be"), { name: "f32 be", bytes: 4, float: true, signed: true, le: false });
  assert.deepEqual(numType("u8"), { name: "u8", bytes: 1, float: false, signed: false, le: true });
  assert.equal(numType("bf16 le")?.bytes, 2);
  // No byte order for a wide type is not a type to guess at.
  assert.equal(numType("u32"), null);
  for (const t of ["ascii[]", "Chunk", "q4_0", "u24 le", "f8", "bf32 le", "bytes[]"]) assert.equal(numType(t), null, t);
});

test("bytes decode in the order the type names", () => {
  const le = decode(Uint8Array.from([0x01, 0x00, 0xff, 0xff, 0x00, 0x80]), numType("i16 le")!, 3);
  assert.deepEqual([...le], [1, -1, -32768]);
  const be = decode(Uint8Array.from([0x3f, 0x80, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00]), numType("f32 be")!, 2);
  assert.deepEqual([...be], [1, -2]);
  // Half precision: 1.0, -2.0 and the smallest subnormal.
  const h = decode(Uint8Array.from([0x00, 0x3c, 0x00, 0xc0, 0x01, 0x00]), numType("f16 le")!, 3);
  assert.deepEqual([...h], [1, -2, 2 ** -24]);
  // bfloat16 1.5 is the top half of binary32 1.5 (0x3fc00000).
  assert.deepEqual([...decode(Uint8Array.from([0xc0, 0x3f]), numType("bf16 le")!, 1)], [1.5]);
  // Asked for more than the bytes hold: only what is there.
  assert.equal(decode(Uint8Array.from([1, 2, 3]), numType("u16 le")!, 5).length, 1);
});

// ----- decimation -----

const mono = (values: number[]) => ({ values, stride: 1, channel: 0 });

test("a run no longer than two points a pixel is drawn whole", () => {
  const c = mono([5, 1, 4, 2]);
  const e = envelope(c, 0, 4, 2);
  assert.equal(e.decimated, false);
  assert.deepEqual(e.frames, [0, 1, 2, 3]);
  assert.deepEqual(e.values, [5, 1, 4, 2]);
});

test("decimation keeps each pixel's lowest and highest, in the order they occur", () => {
  // Two pixels of four: the spike at 2 and the dip at 5 survive, each with
  // the other extreme of its pixel.
  const c = mono([0, 1, 9, 2, 3, -7, 1, 0]);
  const e = envelope(c, 0, 8, 2);
  assert.equal(e.decimated, true);
  assert.deepEqual(e.frames, [0, 2, 4, 5]);
  assert.deepEqual(e.values, [0, 9, 3, -7]);
  // Frames are in increasing order, which is what uPlot needs of x.
  for (let i = 1; i < e.frames.length; i++) assert.ok((e.frames[i] as number) > (e.frames[i - 1] as number));
});

test("one frame either side of the window is kept, so the line runs off the edge", () => {
  const c = mono([0, 1, 2, 3, 4, 5]);
  assert.deepEqual(envelope(c, 2, 4, 10).frames, [1, 2, 3, 4]);
});

test("interleaved channels are read by stride, and NaN breaks the line", () => {
  const stereo = [1, -1, 2, -2, NaN, -3];
  const left = { values: stereo, stride: 2, channel: 0 };
  const right = { values: stereo, stride: 2, channel: 1 };
  assert.equal(frameCount(left), 3);
  assert.deepEqual(envelope(left, 0, 3, 10).values, [1, 2, null]);
  assert.deepEqual(envelope(right, 0, 3, 10).values, [-1, -2, -3]);
  assert.deepEqual(rangeOf(left, 0, 3), { min: 1, max: 2, frames: 3 });
  assert.deepEqual(extentOf([left, right]), [-3, 2]);
});

test("a flat run still has some height, and an empty one a range", () => {
  assert.deepEqual(extentOf([mono([4, 4, 4])]), [3.6, 4.4]);
  assert.deepEqual(extentOf([mono([0, 0])]), [-1, 1]);
  assert.deepEqual(extentOf([mono([NaN])]), [-1, 1]);
  assert.equal(rangeOf(mono([NaN, NaN]), 0, 2), null);
});

test("the nearest frame is kept inside the run", () => {
  assert.equal(nearestFrame(-3, 10), 0);
  assert.equal(nearestFrame(4.6, 10), 5);
  assert.equal(nearestFrame(99, 10), 9);
});

test("values are written grouped for integers and to seven digits for floats", () => {
  assert.equal(valueText(-32768, false), (-32768).toLocaleString());
  assert.equal(valueText(0.1 + 0.2, true), "0.3");
  assert.equal(valueText(Math.fround(0.8507393), true), "0.8507393");
  assert.equal(valueText(NaN, true), "NaN");
  assert.equal(valueText(-Infinity, true), "-∞");
});

// ----- the grid -----

test("zoomed out, each pixel is the mean of the block under it", () => {
  // A 4 by 2 grid drawn into 2 by 1 pixels: each pixel is two columns of two rows.
  const values = [1, 2, 3, 4, 5, 6, 7, 8];
  const s = sampleGrid(values, 4, 2, { c0: 0, c1: 4, r0: 0, r1: 2 }, 2, 1);
  assert.equal(s.per, 4);
  assert.deepEqual([...s.means], [(1 + 2 + 5 + 6) / 4, (3 + 4 + 7 + 8) / 4]);
});

test("zoomed in, each pixel is the one value under it, and off the grid is empty", () => {
  const values = [1, 2, 3, 4];
  // Columns 1 to 3 of a 2-column grid across 4 pixels: half a column each,
  // and the last two pixels are past the grid's edge.
  const s = sampleGrid(values, 2, 2, { c0: 1, c1: 3, r0: 0, r1: 1 }, 4, 1);
  assert.equal(s.per, 1);
  assert.deepEqual([...s.means].map((v) => (Number.isNaN(v) ? null : v)), [2, 2, null, null]);
  assert.deepEqual(pixelBlock(0, 4, 2, 1, 4), [2, 4]);
});

test("colours span zero symmetrically for weights of both signs", () => {
  assert.deepEqual(colourRange([-1, 0.5, 2], false), { lo: -2, hi: 2, diverging: true });
  assert.deepEqual(colourRange([1, 2, 3], false), { lo: 1, hi: 3, diverging: false });
  // One outlier among a thousand values does not set the clipped range.
  const many = Array.from({ length: 1000 }, (_, i) => (i % 2 ? 1 : -1) * (i / 1000));
  many.push(500);
  const clipped = colourRange(many, true);
  assert.ok(clipped.hi < 2, `clipped hi ${clipped.hi}`);
  assert.equal(colourRange(many, false).hi, 500);
});

test("the ramp runs from one pole through the midpoint to the other", () => {
  const blue = parseHex("#2a78d6")!;
  const grey = parseHex("#f0efec")!;
  const orange = parseHex("#eb6834")!;
  assert.deepEqual(parseHex("#2A78D6"), [42, 120, 214]);
  assert.equal(parseHex("blue"), null);
  const r = ramp(blue, grey, orange, true, 3);
  assert.deepEqual([...r.subarray(0, 3)], [...blue]);
  assert.deepEqual([...r.subarray(4, 7)], [...grey]);
  assert.deepEqual([...r.subarray(8, 11)], [...orange]);
  assert.deepEqual(mix(blue, orange, 0), blue);
  assert.equal(rampIndex(0, -1, 1, 256), 128);
  assert.equal(rampIndex(-5, -1, 1, 256), 0);
  assert.equal(rampIndex(5, -1, 1, 256), 255);
});

// ----- Steim samples -----

test("a sample's frame is found by adding up what each frame made", () => {
  const info = { steim: true, frames: [{ held: 14, used: 14 }, { held: 15, used: 15 }, { held: 15, used: 3 }] } as unknown as SamplesInfo;
  assert.equal(frameOfSample(info, 0), 0);
  assert.equal(frameOfSample(info, 13), 0);
  assert.equal(frameOfSample(info, 14), 1);
  assert.equal(frameOfSample(info, 31), 2);
  assert.equal(frameOfSample(info, 32), null);
  assert.equal(frameOfSample({ ...info, steim: false } as SamplesInfo, 0), null);
});

import assert from "node:assert/strict";
import { test } from "node:test";

import { pictureText, pixelBytes, pngOutlineOf, type PngChunk, type PngHeader } from "../src/pngoutline.ts";

const header: PngHeader = { width: 64, height: 48, depth: 8, colourType: "rgba", interlaced: false };

let at = 8;
function chunk(index: number, type: string, dataBytes: number, said = "", label = ""): PngChunk {
  const c = { index, type, path: [1, index], offsetBits: at * 8, dataBytes, label, said };
  at += dataBytes + 12;
  return c;
}

test("the picture reads in words a reader uses", () => {
  assert.equal(pictureText(header), "64 × 48 · 8-bit RGBA");
  assert.equal(pictureText({ ...header, colourType: "indexed", depth: 4, interlaced: true }), "64 × 48 · 4-bit palette · interlaced");
  // Rows start on a byte, so a 13-pixel one-bit row is two bytes.
  assert.equal(pixelBytes({ width: 13, height: 7, depth: 1, colourType: "greyscale", interlaced: false }), 14);
  assert.equal(pixelBytes(header), 64 * 48 * 4);
  assert.equal(pixelBytes({ ...header, colourType: "nonsense" }), null);
});

test("chunks are grouped by what they are for, and a run of IDAT is one row", () => {
  at = 8;
  const chunks = [
    chunk(0, "IHDR", 13, "64 × 48 · 8-bit RGBA"),
    chunk(1, "tEXt", 19, "Gradient test", "Title"),
    chunk(2, "gAMA", 4, "0.45455"),
    chunk(3, "pHYs", 9, "300 dpi"),
    chunk(4, "IDAT", 8192),
    chunk(5, "IDAT", 8192),
    chunk(6, "IDAT", 100),
    chunk(7, "prVt", 5),
    chunk(8, "IEND", 0),
  ];
  const o = pngOutlineOf(header, chunks, 0);
  const ids = o.nodes.map((n) => n.id);
  assert.deepEqual(ids, ["/", "/0", "/colour", "/colour/2", "/text", "/text/1", "/data", "/data/4", "/other", "/other/3", "/other/7"]);
  const data = o.nodes.find((n) => n.id === "/data/4");
  assert.equal(data?.value, "3 IDAT chunks");
  assert.equal(data?.logicalBytes, 8192 * 2 + 100);
  // A text chunk is named by its keyword.
  assert.equal(o.nodes.find((n) => n.id === "/text/1")?.label, "Title");
  assert.equal(o.nodes[0]?.logicalBytes, 64 * 48 * 4);
  assert.equal(o.summary, "64 × 48 · 8-bit RGBA · 9 chunks");
});

test("each frame of an animation keeps its own data", () => {
  at = 8;
  const chunks = [
    chunk(0, "IHDR", 13),
    chunk(1, "acTL", 8),
    chunk(2, "fcTL", 26),
    chunk(3, "IDAT", 25),
    chunk(4, "fcTL", 26),
    chunk(5, "fdAT", 29),
    chunk(6, "fdAT", 29),
    chunk(7, "fcTL", 26),
    chunk(8, "fdAT", 29),
  ];
  const o = pngOutlineOf(header, chunks, 0);
  const animation = o.nodes.filter((n) => n.parentId === "/animation");
  assert.deepEqual(
    animation.map((n) => [n.type, n.value]),
    [["acTL", ""], ["fcTL", ""], ["fcTL", ""], ["fdAT", "2 fdAT chunks"], ["fcTL", ""], ["fdAT", "1 fdAT chunk"]],
  );
});

test("chunks left unread are counted in the summary", () => {
  at = 8;
  const o = pngOutlineOf(header, [chunk(0, "IHDR", 13), chunk(1, "IDAT", 10)], 5000);
  assert.match(o.summary, /5,002 chunks/);
});

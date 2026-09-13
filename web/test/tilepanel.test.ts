// The FITS tile panel's wording, from the numbers the core hands over.
//
// Every number comes from the core, so what is left to get wrong here is how
// they are counted: a tile index from 0 beside an ordinal and coordinates from
// 1, which algorithm a tile in a fallback column really went through, and
// whether the row of pixels is all of them.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { TypeInfo } from "../src/doc.ts";
import { ordinal } from "../src/strings.ts";
import { columnLine, pixelsHead, sizeLine, tileLine, tileNote, tileStepText } from "../src/tilepanel.ts";

/** Tile 3 of comp.fits: a row of 440 pixels, Rice coded into 223 bytes. */
function info(over: Partial<TypeInfo> = {}): TypeInfo {
  const base = {
    kind: "tile",
    problem: "",
    tile_index: 3,
    tile_count: 300,
    tile_start: [0, 3],
    tile_shape: [440, 1],
    tile_image_shape: [440, 300],
    tile_algorithm: "RICE_1",
    tile_column: "COMPRESSED_DATA",
    tile_packed: 223,
    tile_decoded: 880,
    tile_steps: [],
    tile_values: Array.from({ length: 32 }, (_, i) => String(i)),
    tile_total: 440,
    tile_pixels: 440,
    tile_element_type: "i16",
  };
  return { ...base, ...over } as unknown as TypeInfo;
}

test("the tile's index counts from 0 and its ordinal and coordinates from 1", () => {
  assert.equal(
    tileLine(info()),
    "Tile [3], the 4th of 300: 440 × 1 pixels, first pixel at (1, 4) in the 440 × 300 image (pixel coordinates: axis 1 first, counting from 1)",
  );
  assert.equal(tileNote(info()), "440 pixels");
  assert.equal(tileNote(info({ tile_pixels: 1, tile_shape: [1, 1] })), "1 pixel");
});

test("an image whose header would not read says only why", () => {
  const bad = info({ tile_count: 0, problem: "Not unpacked: the header has no ZBITPIX keyword, so the pixel type is unknown." });
  assert.equal(tileLine(bad), null);
  assert.equal(sizeLine(bad), null);
  assert.equal(tileNote(bad), "");
});

test("the size line names the algorithm that ran, not the one ZCMPTYPE names", () => {
  assert.equal(sizeLine(info()), "RICE_1, 223 bytes in the file, 880 bytes unpacked");
  const gzipped = info({ tile_column: "GZIP_COMPRESSED_DATA", tile_packed: 38, tile_decoded: 2000 });
  assert.equal(sizeLine(gzipped), "gzip, 38 bytes in the file, 2,000 bytes unpacked");
  assert.match(columnLine(gzipped) ?? "", /^Stored in GZIP_COMPRESSED_DATA, not COMPRESSED_DATA: .* ZCMPTYPE \(RICE_1\) does not apply to this tile\.$/);
  // Stored bytes come out the size they went in, and saying so twice is noise.
  const stored = info({ tile_algorithm: "NOCOMPRESS", tile_packed: 40, tile_decoded: 40 });
  assert.equal(sizeLine(stored), "NOCOMPRESS, 40 bytes in the file");
  assert.equal(columnLine(info()), null);
});

test("a step says what it did to the bytes, then its note", () => {
  const step = (over: object) => ({ what: "x", in_bytes: 0, out_bytes: 0, note: "", skipped: false, ...over });
  assert.equal(tileStepText(step({ in_bytes: 38, out_bytes: 2000 })), "38 → 2,000 bytes");
  assert.equal(tileStepText(step({ in_bytes: 880, note: "440 pixels, as big-endian i16" })), "440 pixels, as big-endian i16");
  // A step that stopped before anything came out says what it was handed.
  assert.equal(tileStepText(step({ in_bytes: 1 })), "1 byte");
  // A step on numbers rather than bytes is its note alone.
  assert.equal(tileStepText(step({ note: "float = integer × 0.5 + 10 (ZSCALE and ZZERO for this tile)" })), "float = integer × 0.5 + 10 (ZSCALE and ZZERO for this tile)");
});

test("the pixels are the first ones only when some are left out", () => {
  assert.equal(pixelsHead(info()), "First pixels, as i16");
  assert.equal(pixelsHead(info({ tile_values: ["7"], tile_total: 1, tile_element_type: "f32" })), "Pixels, as f32");
});

test("an ordinal counts the teens apart", () => {
  assert.deepEqual([1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 111, 1001].map(ordinal), [
    "1st",
    "2nd",
    "3rd",
    "4th",
    "11th",
    "12th",
    "13th",
    "21st",
    "22nd",
    "101st",
    "111th",
    "1,001st",
  ]);
});

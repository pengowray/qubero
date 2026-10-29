// The pixel grid's arithmetic (`src/viz/pixelgridmath.ts`): placing the
// picture, the pixel under a point, the pixels on screen, whether a value fits
// its cell, and a JPEG scan's MCUs and blocks and a PNG's passes, checked
// against small scans built by hand in the shape the core sends.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  blockAt,
  blockBits,
  blockSize,
  codedSize,
  heatClass,
  heatStep,
  hexSample,
  indexScan,
  labelFits,
  mcuAt,
  mcuBits,
  mcusInBits,
  mcuSize,
  pixelAt,
  pixelToScreen,
  placePicture,
  restartInterval,
  scanlineOf,
  scanlinePixels,
  visiblePixels,
  type ScanShape,
} from "../src/viz/pixelgridmath.ts";
import { passOf } from "../src/report/pngpasses.ts";

const NO_ZOOM = { k: 1, x: 0, y: 0 };

// ----- placing the picture -----

test("a small picture is magnified by a whole number and centred", () => {
  const p = placePicture(32, 32, 400, 300);
  assert.equal(p.scale, 9);
  assert.equal(p.x0, (400 - 288) / 2);
  assert.equal(p.y0, (300 - 288) / 2);
});

test("magnifying stops at the cap", () => {
  assert.equal(placePicture(2, 2, 400, 400, 16).scale, 16);
});

test("a large picture is scaled down to fit the box", () => {
  const p = placePicture(1000, 500, 400, 300);
  assert.equal(p.scale, 0.4);
  assert.equal(p.y0, (300 - 200) / 2);
});

test("the pixel under a point, before and after a zoom", () => {
  const p = { scale: 10, x0: 5, y0: 0 };
  assert.deepEqual(pixelAt(p, NO_ZOOM, 5, 0, 8, 8), { x: 0, y: 0 });
  assert.deepEqual(pixelAt(p, NO_ZOOM, 24.9, 19.9, 8, 8), { x: 1, y: 1 });
  assert.equal(pixelAt(p, NO_ZOOM, 4, 0, 8, 8), null);
  assert.equal(pixelAt(p, NO_ZOOM, 85, 0, 8, 8), null);
  const zoomed = { k: 2, x: -10, y: -20 };
  // Pixel (3, 2) starts at layout (35, 20), screen (60, 20).
  assert.deepEqual(pixelToScreen(p, zoomed, 3, 2), { sx: 60, sy: 20 });
  assert.deepEqual(pixelAt(p, zoomed, 61, 21, 8, 8), { x: 3, y: 2 });
});

test("only the pixels on screen are visited", () => {
  const p = { scale: 10, x0: 0, y0: 0 };
  assert.deepEqual(visiblePixels(p, NO_ZOOM, 35, 1000, 100, 100), { x0: 0, y0: 0, x1: 4, y1: 100 });
  // Zoomed four times and panned to pixel 50 across.
  assert.deepEqual(visiblePixels(p, { k: 4, x: -2000, y: 0 }, 400, 400, 100, 100), { x0: 50, y0: 0, x1: 60, y1: 10 });
  // Panned off the picture altogether.
  const off = visiblePixels(p, { k: 1, x: 5000, y: 0 }, 400, 400, 100, 100);
  assert.equal(off.x1 - off.x0, 0);
});

// ----- values in cells -----

test("a value is written only when every channel fits its cell", () => {
  // Two digits a channel at 10px mono is 12px plus padding; four lines of
  // 11px is 44px plus padding.
  assert.equal(labelFits(23, 1, 2), false, "under the 24px floor");
  assert.equal(labelFits(24, 1, 2), true);
  assert.equal(labelFits(40, 4, 2), false, "four channels need 48px");
  assert.equal(labelFits(48, 4, 2), true);
  assert.equal(labelFits(24, 1, 4), false, "four digits need 28px");
  assert.equal(labelFits(28, 1, 4), true);
});

test("samples are written padded to their bit depth", () => {
  assert.equal(hexSample(5, 8), "05");
  assert.equal(hexSample(65535, 16), "ffff");
  assert.equal(hexSample(10, 16), "000a");
  assert.equal(hexSample(1, 1), "1");
});

// ----- a JPEG scan -----

/** Two MCUs across and one down of a 4:2:0 scan: four Y blocks, one Cb and
 *  one Cr to each, with a restart marker every MCU. */
function scan420(): ScanShape {
  const yBits = [10, 11, 12, 13, 20, 21, 22, 23];
  return {
    width: 30,
    height: 14,
    mcus_across: 2,
    mcus_down: 1,
    channels: [
      { name: "Y", id: 1, h: 2, v: 2, blocks_across: 4, blocks_down: 2, quant_id: 0, quant: null },
      { name: "Cb", id: 2, h: 1, v: 1, blocks_across: 2, blocks_down: 1, quant_id: 1, quant: null },
      { name: "Cr", id: 3, h: 1, v: 1, blocks_across: 2, blocks_down: 1, quant_id: 1, quant: null },
    ],
    scan: [0, 1, 2],
    // Each MCU's blocks plus 20 bits of padding and marker.
    mcu_bits: [10 + 11 + 12 + 13 + 5 + 6 + 20, 20 + 21 + 22 + 23 + 7 + 8 + 20],
    block_channel: [0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 1, 2],
    block_x: [0, 1, 0, 1, 0, 0, 2, 3, 2, 3, 1, 1],
    block_y: [0, 0, 1, 1, 0, 0, 0, 0, 1, 1, 0, 0],
    block_bits: [yBits[0], yBits[1], yBits[2], yBits[3], 5, 6, yBits[4], yBits[5], yBits[6], yBits[7], 7, 8] as number[],
    run_offset_bits: 1000,
    restart_interval: 1,
  };
}

test("an MCU of a 4:2:0 scan is 16 pixels square and a chroma block covers all of it", () => {
  const s = scan420();
  assert.deepEqual(mcuSize(s), { mw: 16, mh: 16 });
  assert.deepEqual(blockSize(s, 0), { bw: 8, bh: 8 });
  assert.deepEqual(blockSize(s, 1), { bw: 16, bh: 16 });
  assert.deepEqual(codedSize(s), { w: 32, h: 16 });
});

test("a scan of one channel has one block to an MCU", () => {
  const s = { ...scan420(), scan: [1] };
  assert.deepEqual(mcuSize(s), { mw: 16, mh: 16 });
  const y = { ...scan420(), scan: [0] };
  assert.deepEqual(mcuSize(y), { mw: 8, mh: 8 });
});

test("blocks and MCUs are found by pixel", () => {
  const s = scan420();
  const ix = indexScan(s);
  assert.equal(ix.perMcu, 6);
  assert.equal(mcuAt(s, 0, 0), 0);
  assert.equal(mcuAt(s, 16, 15), 1);
  assert.equal(mcuAt(s, 32, 0), null);
  assert.equal(blockAt(s, ix, 0, 9, 9), 3, "the fourth Y block of the first MCU");
  assert.equal(blockAt(s, ix, 0, 24, 0), 7, "Y block (3, 0) is the second of the second MCU");
  assert.equal(blockAt(s, ix, 1, 20, 3), 10, "the second MCU's Cb block");
  assert.equal(blockAt(s, ix, 2, 31, 15), 11);
  assert.equal(blockAt(s, ix, 0, 32, 0), null);
});

test("a block's bits follow the block before it, from its MCU's start", () => {
  const s = scan420();
  const ix = indexScan(s);
  assert.deepEqual(mcuBits(ix, 0), { startBit: 1000, endBit: 1077 });
  assert.deepEqual(mcuBits(ix, 1), { startBit: 1077, endBit: 1198 });
  assert.deepEqual(blockBits(s, ix, 0), { startBit: 1000, endBit: 1010 });
  assert.deepEqual(blockBits(s, ix, 5), { startBit: 1051, endBit: 1057 });
  // The second MCU starts after the first's padding, not after its last block.
  assert.deepEqual(blockBits(s, ix, 6), { startBit: 1077, endBit: 1097 });
});

test("a run of bits lights the MCUs it meets", () => {
  const ix = indexScan(scan420());
  assert.deepEqual(mcusInBits(ix, 1000, 1001), { from: 0, to: 1 });
  assert.deepEqual(mcusInBits(ix, 1076, 1078), { from: 0, to: 2 });
  assert.deepEqual(mcusInBits(ix, 1100, 1100), { from: 1, to: 2 }, "a cursor meets the MCU it is in");
  assert.deepEqual(mcusInBits(ix, 0, 900), { from: 0, to: 0 });
  assert.deepEqual(mcusInBits(ix, 1198, 1300), { from: 0, to: 0 });
  assert.deepEqual(mcusInBits(ix, 0, 5000), { from: 0, to: 2 });
});

test("restart intervals count MCUs in coding order", () => {
  assert.equal(restartInterval({ restart_interval: 12 }, 11), 0);
  assert.equal(restartInterval({ restart_interval: 12 }, 12), 1);
  assert.equal(restartInterval({ restart_interval: 0 }, 5), null);
});

test("the heat layer's classes are round steps", () => {
  assert.equal(heatStep(853), 200, "853 / 7 is 122, which rounds up to 200");
  assert.equal(heatStep(7), 1);
  assert.equal(heatStep(700), 100);
  assert.equal(heatClass(0, 125), 0);
  assert.equal(heatClass(124, 125), 0);
  assert.equal(heatClass(125, 125), 1);
  assert.equal(heatClass(5000, 125), 6, "the top class takes everything above it");
});

// ----- a PNG -----

test("every pixel of an interlaced picture is in the scanline its pass stores it in", () => {
  // Pass 1 stores (0, 0) and (8, 8), in its rows 0 and 1.
  assert.deepEqual(scanlineOf(true, 0, 0), { pass: 1, row: 0 });
  assert.deepEqual(scanlineOf(true, 8, 8), { pass: 1, row: 1 });
  // Pass 7 stores the odd rows, each a row of its own.
  assert.deepEqual(scanlineOf(true, 5, 3), { pass: 7, row: 1 });
  // Pass 6 stores the odd columns of the even rows.
  assert.deepEqual(scanlineOf(true, 1, 4), { pass: 6, row: 2 });
  assert.deepEqual(scanlineOf(false, 7, 9), { pass: null, row: 9 });
});

test("a scanline's pixels are the ones whose pass and row it is", () => {
  const w = 32;
  const h = 32;
  for (const [pass, row] of [
    [1, 2],
    [4, 3],
    [6, 0],
    [7, 15],
  ] as const) {
    const s = scanlinePixels(w, h, pass, row);
    assert.ok(s !== null);
    for (let i = 0; i < s.count; i++) {
      const x = s.x0 + i * s.dx;
      assert.equal(passOf(x, s.y), pass);
      assert.deepEqual(scanlineOf(true, x, s.y), { pass, row });
    }
  }
  assert.equal(scanlinePixels(w, h, 1, 4), null, "pass 1 has 4 rows in a 32-pixel picture");
  assert.deepEqual(scanlinePixels(10, 4, null, 2), { y: 2, x0: 0, dx: 1, count: 10 });
});

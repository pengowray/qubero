// The GWF vector panel's wording, from the numbers the core hands over.
//
// Every number comes from the core, so what is left to get wrong here is what
// a `compress` number is said to mean, which steps say more than their byte
// counts, and which count the line under the values is out of.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { VectorInfo } from "../src/doc.ts";
import { showingLine } from "../src/steplist.ts";
import { schemeLine, sizeLine, valuesHead, vectorNote, vectorStepText } from "../src/vectorpanel.ts";

/** fastAdc1 in FrameL's test file: 2,000 shorts, zero-suppressed and packed
 *  little-endian. */
function info(over: Partial<VectorInfo> = {}): VectorInfo {
  return {
    kind: "vector",
    compress: 261,
    little_endian: true,
    declared: 2000,
    packed: 1391,
    decoded: 4000,
    steps: [
      { what: "zero suppression", in_bytes: 1391, out_bytes: 4000 },
      { what: "differencing", in_bytes: 4000, out_bytes: 4000 },
    ],
    element_type: "i16",
    values: Array.from({ length: 32 }, (_, i) => String(80 + i)),
    total: 2000,
    problem: "",
    ...over,
  };
}

test("the compress number leads the line that says what it means", () => {
  assert.equal(schemeLine(info()), "compress = 261: zero suppression of 2-byte words, with differencing, packed little-endian");
  assert.equal(schemeLine(info({ compress: 10, little_endian: false })), "compress = 10: zero suppression of 8-byte words, with differencing, packed big-endian");
  assert.equal(schemeLine(info({ compress: 259 })), "compress = 259: gzip, with differencing, packed little-endian");
  assert.equal(schemeLine(info({ compress: 263 })), "compress = 263: scheme 7, packed little-endian");
});

test("a step that leaves the size alone says what it did", () => {
  const [suppress, difference] = info().steps;
  assert.equal(vectorStepText(suppress!), "1,391 → 4,000 bytes");
  assert.equal(vectorStepText(difference!), "4,000 → 4,000 bytes, running sum of the differences");
  assert.match(vectorStepText({ what: "interleave", in_bytes: 48, out_bytes: 48 }), /^48 → 48 bytes, the run of real parts and the run of imaginary parts interleaved/);
});

test("a vector that stopped early counts what came out, and says both in the note", () => {
  assert.equal(vectorNote(info()), "2,000 values");
  assert.equal(sizeLine(info()), "1,391 bytes in the file, 4,000 bytes unpacked");
  assert.equal(showingLine(32, 2000, 2000, "value"), "Showing the first 32 of 2,000 values.");
  const short = info({ total: 1200, decoded: 2400, problem: "Stopped at zero suppression after 1,200 of 2,000 values: the packed bits ran out." });
  assert.equal(vectorNote(short), "1,200 of 2,000 values");
  assert.equal(showingLine(32, 1200, 2000, "value"), "Showing the first 32 of the 1,200 values unpacked.");
  // Nothing came out: the size line says only what is in the file.
  assert.equal(sizeLine(info({ decoded: 0, total: 0 })), "1,391 bytes in the file");
  assert.equal(showingLine(8, 8, 8, "value"), null);
});

test("the values are the first ones only when some are left out", () => {
  assert.equal(valuesHead(info()), "First values, as i16");
  assert.equal(valuesHead(info({ values: ["1.5-2i"], total: 1, element_type: "complex f32" })), "Values, as complex f32");
});

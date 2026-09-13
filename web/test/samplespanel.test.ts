// The miniSEED samples panel's wording, from the numbers the core hands over.
//
// Every number comes from the core, so what is left to get wrong here is
// which of them a line claims to show: whether a last sample is the record's
// last sample or only the last one decoded, and which samples a frame made.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { SamplesInfo } from "../src/doc.ts";
import { checkLine, encodingLine, FRAME_COLUMNS, frameRows, samplesNote, showingLine } from "../src/samplespanel.ts";

/** A Steim2 record of 5,980 samples in 62 frames, of which two are listed. */
function info(over: Partial<SamplesInfo> = {}): SamplesInfo {
  const base = {
    kind: "samples",
    problem: "",
    encoding: "Steim2",
    encoding_number: 11,
    big_endian: true,
    declared: 5980,
    bytes: 4032,
    steim: true,
    x0: 2787,
    xn: 2863,
    first_difference: 0,
    frames: [
      { held: 97, used: 97 },
      { held: 105, used: 52 },
    ],
    frames_walked: 2,
    frames_in_record: 62,
    rule: "",
    values: Array.from({ length: 32 }, (_, i) => String(i)),
    last: "2863",
    total: 5980,
    check: true,
  };
  return { ...base, ...over } as unknown as SamplesInfo;
}

test("a frame's row says which samples it made, counting from sample 0", () => {
  assert.deepEqual(FRAME_COLUMNS, { label: "frame", text: "differences", note: "samples" });
  const rows = frameRows(info());
  assert.deepEqual(rows[0], { label: "0", text: "97", note: "0 to 96" });
  assert.deepEqual(rows[1], { label: "1", text: "52 of 105", note: "97 to 148" });
  // One sample is one sample, and a frame that made none says nothing to the right.
  const odd = frameRows(info({ frames: [{ held: 1, used: 1 }, { held: 4, used: 0 }] }));
  assert.deepEqual(odd[0], { label: "0", text: "1", note: "0" });
  assert.deepEqual(odd[1], { label: "1", text: "0 of 4", note: "" });
});

test("the last sample is only called that when every sample was decoded", () => {
  assert.equal(showingLine(info()), "Showing the first 32 of 5,980 samples. Last sample: 2863");
  const short = info({ total: 1200, last: "2801", check: null, problem: "The frames ran out" });
  assert.equal(showingLine(short), "Showing the first 32 of the 1,200 samples decoded.");
  assert.equal(samplesNote(short), "1,200 of 5,980 samples");
  assert.equal(samplesNote(info()), "5,980 samples");
  // All of them already in the row: nothing to add under it.
  assert.equal(showingLine(info({ total: 20, declared: 20, values: ["1"] })), "Showing the first 1 of 20 samples. Last sample: 2863");
  assert.equal(showingLine(info({ total: 1, declared: 1, values: ["1"] })), null);
});

test("a failed check gives both numbers, and there is no check without a Steim record", () => {
  assert.deepEqual(checkLine(info()), { ok: true, text: "Check passed: last sample 2863 equals the reverse integration constant." });
  assert.deepEqual(checkLine(info({ last: "2860", check: false })), {
    ok: false,
    text: "Check failed: last sample 2860 does not equal the reverse integration constant 2863.",
  });
  assert.equal(checkLine(info({ steim: false, xn: null, check: null })), null);
  assert.equal(encodingLine(info({ big_endian: false })), "Steim2, little-endian, 4,032 bytes of data");
});

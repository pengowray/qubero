// The miniSEED samples panel's wording, from the numbers the core hands over.
//
// Every number comes from the core, so what is left to get wrong here is
// which of them a line claims to show: whether a last sample is the record's
// last sample or only the last one decoded, and which samples a frame made.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { TypeInfo } from "../src/doc.ts";
import { checkLine, encodingLine, frameRows, samplesNote, showingLine } from "../src/samplespanel.ts";

/** A Steim2 record of 5,980 samples in 62 frames, of which two are listed. */
function info(over: Partial<TypeInfo> = {}): TypeInfo {
  const base = {
    kind: "samples",
    problem: "",
    mseed_encoding: "Steim2",
    mseed_encoding_number: 11,
    mseed_big_endian: true,
    mseed_declared: 5980,
    mseed_bytes: 4032,
    mseed_steim: true,
    mseed_x0: 2787,
    mseed_xn: 2863,
    mseed_first_difference: 0,
    mseed_frames: [
      { held: 97, used: 97 },
      { held: 105, used: 52 },
    ],
    mseed_frames_walked: 2,
    mseed_frames_in_record: 62,
    mseed_rule: "",
    mseed_values: Array.from({ length: 32 }, (_, i) => String(i)),
    mseed_last: "2863",
    mseed_total: 5980,
    mseed_check: true,
  };
  return { ...base, ...over } as unknown as TypeInfo;
}

test("a frame's row says which samples it made, counting from sample 0", () => {
  const rows = frameRows(info());
  assert.deepEqual(rows[0], { label: "frame 0", text: "97 differences", note: "samples 0 to 96" });
  assert.deepEqual(rows[1], { label: "frame 1", text: "52 of 105 differences used", note: "samples 97 to 148" });
  // One sample is one sample, and a frame that made none says nothing to the right.
  const odd = frameRows(info({ mseed_frames: [{ held: 1, used: 1 }, { held: 4, used: 0 }] }));
  assert.deepEqual(odd[0], { label: "frame 0", text: "1 difference", note: "sample 0" });
  assert.deepEqual(odd[1], { label: "frame 1", text: "0 of 4 differences used", note: "" });
});

test("the last sample is only called that when every sample was decoded", () => {
  assert.equal(showingLine(info()), "Showing the first 32 of 5,980 samples. Last sample: 2863");
  const short = info({ mseed_total: 1200, mseed_last: "2801", mseed_check: null, problem: "The frames ran out" });
  assert.equal(showingLine(short), "Showing the first 32 of the 1,200 samples decoded.");
  assert.equal(samplesNote(short), "1,200 of 5,980 samples");
  assert.equal(samplesNote(info()), "5,980 samples");
  // All of them already in the row: nothing to add under it.
  assert.equal(showingLine(info({ mseed_total: 20, mseed_declared: 20, mseed_values: ["1"] })), "Showing the first 1 of 20 samples. Last sample: 2863");
  assert.equal(showingLine(info({ mseed_total: 1, mseed_declared: 1, mseed_values: ["1"] })), null);
});

test("a failed check gives both numbers, and there is no check without a Steim record", () => {
  assert.deepEqual(checkLine(info()), { ok: true, text: "Check passed: last sample 2863 equals the reverse integration constant." });
  assert.deepEqual(checkLine(info({ mseed_last: "2860", mseed_check: false })), {
    ok: false,
    text: "Check failed: last sample 2860 does not equal the reverse integration constant 2863.",
  });
  assert.equal(checkLine(info({ mseed_steim: false, mseed_xn: null, mseed_check: null })), null);
  assert.equal(encodingLine(info({ mseed_big_endian: false })), "Steim2, little-endian, 4,032 bytes of data");
});

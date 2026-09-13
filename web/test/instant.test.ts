// The digits of a moment, and the line under one, for the times a CDF counts
// on a clock with leap seconds.

import { test } from "node:test";
import assert from "node:assert/strict";

import { instantDigits } from "../src/instant.ts";
import { timeNoteText } from "../src/strings.ts";
import type { FieldTime } from "../src/doc.ts";

/** 2016-12-31T23:59:59Z, the second before the last leap second. */
const LAST_OF_2016 = 1_483_228_799;

test("a moment inside a leap second is second 60 of the minute before it", () => {
  assert.equal(instantDigits(LAST_OF_2016, 500_000_000, 1, true), "2016-12-31 23:59:60.500000000");
  // The same count without the flag is the second before.
  assert.equal(instantDigits(LAST_OF_2016, 500_000_000, 1), "2016-12-31 23:59:59.500000000");
  assert.equal(instantDigits(LAST_OF_2016 + 1, 0, 1_000_000_000), "2017-01-01 00:00:00");
});

test("a field's precision still decides the places", () => {
  assert.equal(instantDigits(LAST_OF_2016, 123_456_789, 1_000_000, true), "2016-12-31 23:59:60.123");
  assert.equal(instantDigits(-62_135_596_800, 0, 1_000_000_000), "0001-01-01 00:00:00");
});

function time(over: Partial<FieldTime>): FieldTime {
  return {
    state: "at",
    unix_seconds: 0,
    nanos: 0,
    zone: "utc",
    step_nanos: 1,
    note: null,
    leap_table_expires: null,
    ...over,
  };
}

test("the line under a moment says what the core noted", () => {
  assert.equal(timeNoteText(time({})), null);
  assert.equal(timeNoteText(time({ state: "leap" })), "Inside a leap second");
  assert.equal(
    timeNoteText(time({ note: "past_leap_second_table", leap_table_expires: 1_814_140_800 })),
    "Leap seconds after 2027-06-28 not yet known",
  );
  assert.equal(timeNoteText(time({ note: "before_leap_seconds" })), "UTC before 1972 per NASA's CDF library, other tools may differ");
});

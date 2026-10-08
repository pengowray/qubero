// The words of the report's protections section. See
// web/src/report/hardeningtext.ts.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { HardeningRow } from "../src/report/coredata.ts";
import { HV, type Tally } from "../src/report/hardeningtext.ts";

function row(key: string, state: string, more: Partial<HardeningRow> = {}): HardeningRow {
  return { key, state, verdict: "info", count: null, total: null, items: [], more: [], evidence: [], ...more };
}

function tally(good: number, partial: number, bad: number, unknown: number): Tally {
  return { good, partial, bad, unknown, total: good + partial + bad + unknown, off: [], partly: [], unknownKeys: [] };
}

test("the heading counts what is in place, then the rest by verdict", () => {
  assert.equal(HV.heading(tally(11, 0, 0, 0)), "All 11 protections in place");
  assert.equal(HV.heading(tally(9, 0, 2, 0)), "9 of 11 protections in place, 2 off");
  assert.equal(HV.heading(tally(3, 1, 3, 2)), "3 of 9 protections in place, 1 partial, 3 off, 2 unknown");
});

test("the line under the heading names what is off, partial and unknown", () => {
  const t = { ...tally(3, 1, 2, 1), off: ["PIE", "IBT"], partly: ["RELRO"], unknownKeys: ["stack canary"] };
  assert.equal(HV.lede(t, false), "Off: PIE, IBT. Partial: RELRO. Unknown: stack canary.");
  assert.equal(HV.lede(tally(5, 0, 0, 0), false), "");
});

test("partial RELRO says why it matters only for a dynamically linked program", () => {
  assert.match(HV.status(row("relro", "partial"), "dynamic").detail, /GOT entries for library functions stay writable/);
  assert.match(HV.status(row("relro", "partial"), "static").detail, /Statically linked/);
});

test("FORTIFY_SOURCE gives its counts, and says when nothing it calls has a checked version", () => {
  assert.deepEqual(HV.status(row("fortify", "yes", { count: 20, total: 36 }), ""), { word: "Yes", detail: "20 of the 36 functions that have a checked version use it." });
  assert.equal(HV.status(row("fortify", "no", { count: 0, total: 0 }), "").word, "N/A");
  assert.equal(HV.status(row("fortify", "no-symbols"), "").word, "Unknown");
});

test("a relative or empty RPATH entry is called out", () => {
  assert.equal(HV.status(row("rpath", "set", { items: ["/opt/lib", "$ORIGIN/../lib"] }), "").detail, "");
  assert.match(HV.status(row("rpath", "set", { items: ["lib"] }), "").detail, /^Relative: lib is searched from the current directory/);
  assert.match(HV.status(row("runpath", "set", { items: ["/usr/lib", ""] }), "").detail, /^Empty entry/);
});

test("a state the text does not know is still shown", () => {
  assert.equal(HV.status(row("ibt", "on"), "").word, "On");
  assert.equal(HV.status(row("mystery", "half-on"), "").word, "Half-on");
});

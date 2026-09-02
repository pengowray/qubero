// What the toolbar says a file's line endings are.

import { test } from "node:test";
import assert from "node:assert/strict";

import { TEXTVIEW } from "../src/strings.ts";

const say = TEXTVIEW.lineEndings;

test("one kind of ending is named on its own", () => {
  assert.equal(say({ lf: 900, cr: 0, crlf: 0 }), "LF");
  assert.equal(say({ lf: 0, cr: 0, crlf: 12 }), "CRLF");
  assert.equal(say({ lf: 0, cr: 4, crlf: 0 }), "CR");
});

test("a file with no endings at all says nothing", () => {
  assert.equal(say({ lf: 0, cr: 0, crlf: 0 }), "");
});

test("a mix says so, largest share first", () => {
  assert.equal(say({ lf: 2, cr: 0, crlf: 1 }), "Mixed: LF 67%, CRLF 33%");
  assert.equal(say({ lf: 1, cr: 0, crlf: 3 }), "Mixed: CRLF 75%, LF 25%");
});

test("a handful of odd lines in a million is one per cent rather than none", () => {
  // Nought per cent under a heading that says the file is mixed contradicts
  // itself, and the odd line is the whole reason to look.
  assert.equal(say({ lf: 2_109_320, cr: 527, crlf: 2161 }), "Mixed: LF 98%, CRLF 1%, CR 1%");
});

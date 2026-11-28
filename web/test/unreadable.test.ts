// Bytes of a tab that cannot be made, as the views that draw or read bytes
// treat them: marked where bytes are drawn, refused where a value is read.

import { test } from "node:test";
import assert from "node:assert/strict";
import { masked, reaches, stretchAt, unreadableError } from "../src/unreadable.ts";
import { hexMarked } from "../src/hexcell.ts";
import { STRINGSVIEW, TEXTVIEW } from "../src/strings.ts";

const bad = { at: 100, len: 50, why: "blocks[2].compressed unpacks to 49 bytes, not the 50 the file says" };

test("a stretch reaches the bytes it overlaps and no others", () => {
  assert.equal(reaches(bad, 99, 1), false);
  assert.equal(reaches(bad, 99, 2), true);
  assert.equal(reaches(bad, 149, 1), true);
  assert.equal(reaches(bad, 150, 10), false);
  assert.equal(reaches(bad, 120, 0), false);
  assert.equal(reaches(null, 0, 1000), false);
  assert.equal(reaches(undefined, 0, 1000), false);
});

test("the text view finds the stretch under a character, wide ones included", () => {
  const other = { at: 300, len: 10, why: "another part" };
  assert.equal(stretchAt([bad, other], 98, 2), undefined);
  assert.equal(stretchAt([bad, other], 99, 2)?.why, bad.why);
  assert.equal(stretchAt([bad, other], 305, 1)?.why, "another part");
  assert.equal(stretchAt([], 120, 1), undefined);
});

test("the byte dump writes no byte of a stretch as the zero the read gave", () => {
  const bytes = new Uint8Array([0x41, 0x42, 0, 0, 0x43]);
  // Bytes 98 to 102, of which 100 and 101 cannot be made.
  assert.equal(hexMarked(bytes, 98, { at: 100, len: 2 }), "41 42 ?? ?? 43");
  assert.equal(hexMarked(bytes, 98, null), "41 42 00 00 43");
  assert.deepEqual([...masked(bytes, 98, { at: 100, len: 2 }, 0x20)], [0x41, 0x42, 0x20, 0x20, 0x43]);
  assert.equal(masked(bytes, 98, null, 0x20), bytes);
});

test("a read whose bytes cannot be made is opened as an error naming the part", () => {
  assert.equal(unreadableError({ unreadable: null }), null);
  assert.equal(unreadableError({}), null);
  assert.equal(unreadableError({ unreadable: bad })?.message, `Can't read these bytes: ${bad.why}`);
});

test("the strings view and the text view say what they did with the bytes", () => {
  assert.equal(STRINGSVIEW.statusUnreadable(65_281, bad.why), `63.8 KiB not searched: ${bad.why}`);
  assert.equal(TEXTVIEW.lineUnreadable, "unreadable bytes");
  assert.equal(TEXTVIEW.copyUnreadable(bad.why), `Couldn't copy: ${bad.why}.`);
});

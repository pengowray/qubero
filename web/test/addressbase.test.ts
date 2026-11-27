// The setting that writes addresses, and field values that are offsets, in hex
// or in decimal (`format.ts`).

import { test } from "node:test";
import assert from "node:assert/strict";
import { formatAddress, formatOffset, positionValue, setAddressBase } from "../src/format.ts";

test("addresses follow the base, and the sub-byte suffix stays", () => {
  try {
    setAddressBase("hex");
    assert.equal(formatOffset(31 * 8), "@0x1f");
    assert.equal(formatOffset(31 * 8 + 3), "@0x1f+3b");
    assert.equal(formatAddress(28 * 8, 1), "@+0x1c");
    setAddressBase("dec");
    assert.equal(formatOffset(31 * 8), "@31");
    assert.equal(formatOffset(31 * 8 + 3), "@31+3b");
    assert.equal(formatAddress(28 * 8, 1), "@+28");
  } finally {
    setAddressBase("hex");
  }
});

test("an offset value is written in hex only where it locates something and addresses are hex", () => {
  const offset = { locates: true, kind: "uint", edit_text: "64" };
  try {
    setAddressBase("hex");
    assert.equal(positionValue(offset), "0x40");
    // Every digit of a 64-bit offset, which a double would round.
    assert.equal(positionValue({ ...offset, edit_text: "18446744073709551615" }), "0xffffffffffffffff");
    assert.equal(positionValue({ ...offset, locates: false }), null);
    assert.equal(positionValue({ ...offset, kind: "int" }), "0x40");
    assert.equal(positionValue({ ...offset, kind: "int", edit_text: "-1" }), null);
    assert.equal(positionValue({ ...offset, kind: "float", edit_text: "64" }), null);
    assert.equal(positionValue({ ...offset, edit_text: "" }), null);
    setAddressBase("dec");
    assert.equal(positionValue(offset), null);
  } finally {
    setAddressBase("hex");
  }
});

test("an enum-typed offset with no name for its value is written as an address", () => {
  setAddressBase("hex");
  assert.equal(positionValue({ locates: true, kind: "enum", edit_text: "75668", value: "75668 (unknown)" }), "0x12794");
  assert.equal(positionValue({ locates: true, kind: "enum", edit_text: "18446744073709551615", value: "undefined (18446744073709551615)" }), null);
  assert.equal(positionValue({ locates: false, kind: "enum", edit_text: "3", value: "3 (unknown)" }), null);
});

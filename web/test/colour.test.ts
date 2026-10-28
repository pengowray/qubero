import assert from "node:assert/strict";
import { test } from "node:test";

import { cssColour } from "../src/colour.ts";

test("a colour the core worked out reads as itself, alpha and all", () => {
  assert.equal(cssColour("#dccd80"), "#dccd80");
  assert.equal(cssColour("#DCCD8080"), "#dccd8080");
  assert.equal(cssColour("#abcd"), "#abcd");
});

test("text that only looks like a colour is not one", () => {
  assert.equal(cssColour("#dccd8"), null);
  assert.equal(cssColour("#dccd80808"), null);
  assert.equal(cssColour("dccd80"), null);
});

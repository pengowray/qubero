// The GRIB section 7 panel's wording, from the numbers the core hands over.
//
// Every number comes from the core, so what is left to get wrong here is
// which field a line names, and how the packed integer X is said to relate to
// the number that field holds: all of it under simple packing, the offset
// above a group's reference under complex packing, and a difference under
// spatial differencing.

import { test } from "node:test";
import assert from "node:assert/strict";

import type { GribInfo } from "../src/doc.ts";
import { cursorLines, gribNote, packingLine, valuesHead } from "../src/gribpanel.ts";

/** Message 1 of the GFS sample: 65,160 pressures, second-order differencing. */
function info(over: Partial<GribInfo> = {}): GribInfo {
  return {
    kind: "grib",
    template: 3,
    spatial_order: 2,
    reference: "947324.3",
    binary_scale: 1,
    decimal_scale: 1,
    minimum: -3,
    declared: 65160,
    packed: 41337,
    steps: [],
    values: Array.from({ length: 32 }, (_, i) => String(101124 + i)),
    total: 65160,
    at: null,
    problem: "",
    ...over,
  };
}

test("the packing line names the template and the order of differencing", () => {
  assert.equal(packingLine(info()), "Data representation template 5.3: complex packing with second-order spatial differencing");
  assert.equal(packingLine(info({ spatial_order: 1 })), "Data representation template 5.3: complex packing with first-order spatial differencing");
  assert.equal(packingLine(info({ template: 2, spatial_order: 0 })), "Data representation template 5.2: complex packing");
  assert.equal(packingLine(info({ template: 0, spatial_order: 0 })), "Data representation template 5.0: simple packing");
  assert.equal(gribNote(info()), "65,160 values");
  assert.equal(gribNote(info({ total: 1200 })), "1,200 of 65,160 values");
});

test("a value of simple packing is its field, and the field holds X", () => {
  const simple = info({ template: 0, spatial_order: 0, reference: "253.02", binary_scale: -5, decimal_scale: 0, declared: 496, total: 496 });
  const at = { index: 5, place: "values", group: 0, position: 0, written: 0, value: "272.98875", packed: 639 } as const;
  assert.deepEqual(cursorLines({ ...simple, at }), [
    "values[5], the 6th of 496 values, decodes to 272.98875.",
    "The packed integer X is 639, as written in the field.",
    "272.98875 = (253.02 + 639 × 2^-5) / 10^0",
  ]);
});

test("a group's value is named as the listing names it, and its field holds only part of X", () => {
  const complex = info({ template: 2, spatial_order: 0, minimum: null, reference: "253.02", binary_scale: -5, decimal_scale: 0, declared: 496 });
  const at = { index: 90, place: "group", group: 2, position: 0, written: 1050, value: "285.84", packed: 1050 } as const;
  const lines = cursorLines({ ...complex, at });
  assert.equal(lines[0], "groups[2].values[0], the 91st of the message's 496 values, decodes to 285.84.");
  assert.equal(lines[1], "With its group's reference added, the packed integer X is 1050; the field's 1050 is only the offset above that reference.");

  const at3 = { index: 2, place: "group", group: 0, position: 2, written: 17, value: "101123.83", packed: 18862 } as const;
  assert.deepEqual(cursorLines(info({ at: at3 })).slice(0, 3), [
    "groups[0].values[2], the 3rd of the message's 65,160 values, decodes to 101123.83.",
    "The field's 17 plus its group's reference plus the overall minimum -3 is a second-order difference, not a value.",
    "With the spatial differencing undone, the packed integer X is 18862.",
  ]);
});

test("a first value is written whole, and the cursor off a value has no lines", () => {
  const at = { index: 1, place: "first", group: 0, position: 0, written: 0, value: "101123.43", packed: 18856 } as const;
  assert.deepEqual(cursorLines(info({ at })), [
    "first_values[1], the 2nd of the message's 65,160 values, decodes to 101123.43.",
    "The packed integer X is 18856, as written in the field: the first 2 values are written as values, not differences, and have no group reference.",
    "101123.43 = (947324.3 + 18856 × 2^1) / 10^1",
  ]);
  assert.match(cursorLines(info({ spatial_order: 1, at }))[1] ?? "", /: the first value is written as a value, not a difference, and has no group reference\.$/);
  assert.deepEqual(cursorLines(info()), []);
  assert.equal(valuesHead(info()), "First values");
  assert.equal(valuesHead(info({ values: ["1"], total: 1 })), "Values");
});

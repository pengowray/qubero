// A name too long for the toolbar is written shorter, never cut.

import { test } from "node:test";
import assert from "node:assert/strict";

import { shorterForms } from "../src/labelfit.ts";

test("a name is shortened at its middle dot, then its closing remark, then its first comma", () => {
  assert.deepEqual(shorterForms("PSRFITS search mode (filterbank samples) · FITS", "FITS"), [
    "PSRFITS search mode (filterbank samples) · FITS",
    "PSRFITS search mode (filterbank samples)",
    "PSRFITS search mode",
    "FITS",
  ]);
  assert.deepEqual(shorterForms("Raw CD-ROM image, 2352-byte sectors"), ["Raw CD-ROM image, 2352-byte sectors", "Raw CD-ROM image"]);
  assert.deepEqual(shorterForms("Blackfin boot stream (LDR, BF50x/51x/52x/54x/59x/60x/70x)"), [
    "Blackfin boot stream (LDR, BF50x/51x/52x/54x/59x/60x/70x)",
    "Blackfin boot stream",
  ]);
});

test("a remark in the middle stays, and the format's own short name is the last resort", () => {
  assert.deepEqual(shorterForms("Hierarchical Data Format (version 5) data", "HDF5"), ["Hierarchical Data Format (version 5) data", "HDF5"]);
});

test("a short name no shorter than the name is left out", () => {
  assert.deepEqual(shorterForms("HDF5", "HDF5"), ["HDF5"]);
  assert.deepEqual(shorterForms("PNG image", "PNG image"), ["PNG image"]);
  assert.deepEqual(shorterForms(""), [""]);
});

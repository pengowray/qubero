// One formatter for every length of time the app shows.

import { test } from "node:test";
import assert from "node:assert/strict";

import { axisTimes, columnTimeText, durationText, secondsValueText, spacingText, timeAtText, timeColumn } from "../src/timetext.ts";

test("a length of time is in the unit its size calls for", () => {
  assert.equal(durationText(3.41333333333e-7), "341.3 ns");
  assert.equal(durationText(6.4e-5), "64 µs");
  assert.equal(durationText(0.00029257), "292.6 µs");
  assert.equal(durationText(0.1), "100 ms");
  assert.equal(durationText(0.9), "900 ms");
  assert.equal(durationText(1), "1 s");
  assert.equal(durationText(12.345), "12.35 s");
  assert.equal(durationText(0), "0 s");
  assert.equal(durationText(Number.NaN), "");
});

test("from a minute up a duration is minutes and seconds, then hours", () => {
  assert.equal(durationText(59.99), "59.99 s");
  assert.equal(durationText(59.9996), "1 min 0 s");
  assert.equal(durationText(252), "4 min 12 s");
  assert.equal(durationText(3723), "1 h 2 min 3 s");
  assert.equal(durationText(90_061), "25 h 1 min 1 s");
});

test("a whole rate keeps its 1/N form, any other rate is an interval", () => {
  assert.equal(spacingText(8000), "1/8,000 s");
  assert.equal(spacingText(44_100), "1/44,100 s");
  assert.equal(spacingText(1 / 3.41333333333e-7), "341.3 ns");
  assert.equal(spacingText(24_414.0625), "40.96 µs");
  assert.equal(spacingText(1 / 0.899), "899 ms");
  assert.equal(spacingText(1 / 15), "15 s");
  assert.equal(spacingText(0), "");
});

test("a time among neighbours has places enough to tell them apart", () => {
  assert.equal(timeAtText(1234 / 44_100, 1 / 44_100), "27.982 ms");
  assert.equal(timeAtText(1235 / 44_100, 1 / 44_100), "28.005 ms");
  assert.equal(timeAtText(0, 1 / 44_100), "0 µs");
  assert.equal(timeAtText(100, 1 / 44_100), "100.000000 s");
});

test("an axis has one unit for every tick", () => {
  assert.deepEqual(axisTimes([0, 0.25, 0.5], 0.25), ["0 ms", "250 ms", "500 ms"]);
  assert.deepEqual(axisTimes([0, 0.5, 1, 1.5], 0.5), ["0.0 s", "0.5 s", "1.0 s", "1.5 s"]);
  assert.deepEqual(axisTimes([0, 2e-6, 4e-6], 2e-6), ["0 µs", "2 µs", "4 µs"]);
});

test("a time column has one unit and places enough for each row to differ", () => {
  const audio = timeColumn(26_000_000, 44_100);
  assert.equal(audio.unit.symbol, "s");
  assert.equal(columnTimeText(1, audio), "0.000023");
  assert.notEqual(columnTimeText(1, audio), columnTimeText(2, audio));
  const psrfits = timeColumn(4096, 1 / 6.4e-5);
  assert.equal(psrfits.unit.symbol, "ms");
  assert.equal(columnTimeText(1, psrfits), "0.064");
  assert.equal(columnTimeText(4095, psrfits), "262.080");
  // A step that ends is shown to its end: 125 µs is 0.125 ms, not 0.13.
  const parkes = timeColumn(4096, 8000);
  assert.equal(columnTimeText(1, parkes), "0.125");
  const guppi = timeColumn(2, 1 / 3.41333333333e-7);
  assert.equal(guppi.unit.symbol, "ns");
  assert.equal(columnTimeText(1, guppi), "341");
});

test("a fact in seconds is said in the unit that suits it", () => {
  assert.equal(secondsValueText("6.4E-05", "s"), "64 µs");
  assert.equal(secondsValueText("0.9", "s"), "900 ms");
  assert.equal(secondsValueText("44100", "Hz"), null);
  assert.equal(secondsValueText("", "s"), null);
  assert.equal(secondsValueText("abc", "s"), null);
});

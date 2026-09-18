import { test } from "node:test";
import assert from "node:assert/strict";

import { rememberChoice, storedChoice, storedNumber, storedText } from "../src/stored.ts";

/** Put a storage in front of the module under test. The value stands in for
 *  `localStorage`, which the page has and node does not. */
function storage(value: unknown): void {
  Object.defineProperty(globalThis, "localStorage", { value, configurable: true });
}

/** A storage that keeps what it is given, which is the browser working. */
function memory(start: Readonly<Record<string, string>> = {}): Map<string, string> {
  const kept = new Map(Object.entries(start));
  storage({ getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) });
  return kept;
}

const VIEWS = ["hex", "listing", "text"];

test("what was kept comes back, and what was not kept reads as nothing", () => {
  memory({ "qubero.view": "listing" });
  assert.equal(storedText("qubero.view"), "listing");
  assert.equal(storedText("qubero.column.plain"), null);
  assert.equal(storedChoice("qubero.view", VIEWS, "hex"), "listing");
});

test("a choice that is no longer on offer falls back", () => {
  // A build that dropped a view leaves readers holding its name. The chooser
  // must not point at nothing.
  memory({ "qubero.view": "gopher" });
  assert.equal(storedChoice("qubero.view", VIEWS, "hex"), "hex");
});

test("a round trip through a working storage", () => {
  const kept = memory();
  rememberChoice("qubero.view", "text");
  assert.equal(kept.get("qubero.view"), "text");
  assert.equal(storedText("qubero.view"), "text");
  assert.equal(storedChoice("qubero.view", VIEWS, "hex"), "text");
});

test("a number is checked before it is believed", () => {
  memory({ good: "42", words: "abc", zero: "0", negative: "-3", empty: "" });
  assert.equal(storedNumber("good", 4), 42);
  assert.equal(storedNumber("words", 4), 4);
  assert.equal(storedNumber("zero", 4), 4);
  assert.equal(storedNumber("negative", 4), 4);
  assert.equal(storedNumber("empty", 4), 4);
  assert.equal(storedNumber("missing", 4), 4);
});

test("a storage that throws on reading is a browser that kept nothing", () => {
  storage({
    getItem: (): string | null => { throw new Error("site data is blocked"); },
    setItem: (): void => {},
  });
  assert.equal(storedText("qubero.view"), null);
  assert.equal(storedChoice("qubero.view", VIEWS, "hex"), "hex");
  assert.equal(storedNumber("qubero.strings.min", 4), 4);
});

test("a storage that throws on writing forgets the choice and nothing else", () => {
  storage({
    getItem: (): string | null => null,
    setItem: (): void => { throw new Error("the quota is full"); },
  });
  assert.doesNotThrow(() => rememberChoice("qubero.view", "text"));
});

test("a browser that throws at the sight of storage still gets its page", () => {
  // Blocked site data can make the property itself throw, so the reach for it
  // has to be inside the guard as well. This is the one that took the page
  // down before anything was drawn.
  Object.defineProperty(globalThis, "localStorage", {
    get: (): never => { throw new Error("access is denied"); },
    configurable: true,
  });
  assert.equal(storedText("qubero.view"), null);
  assert.equal(storedChoice("qubero.view", VIEWS, "hex"), "hex");
  assert.equal(storedNumber("qubero.strings.min", 4), 4);
  assert.doesNotThrow(() => rememberChoice("qubero.view", "text"));
});

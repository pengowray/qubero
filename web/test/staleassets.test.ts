import { test } from "node:test";
import assert from "node:assert/strict";

import { reloadForStaleAssets, watchForStaleAssets } from "../src/staleassets.ts";

/** Session storage and a page that counts how often it was asked to reload. */
function browser(storage: { get: (k: string) => string | null; set: (k: string, v: string) => void }) {
  let reloads = 0;
  Object.defineProperty(globalThis, "sessionStorage", {
    value: { getItem: storage.get, setItem: storage.set },
    configurable: true,
  });
  Object.defineProperty(globalThis, "location", {
    value: { reload: () => { reloads += 1; } },
    configurable: true,
  });
  return () => reloads;
}

function memory() {
  const kept = new Map<string, string>();
  return { get: (k: string) => kept.get(k) ?? null, set: (k: string, v: string) => void kept.set(k, v) };
}

test("the page is asked for again, once", () => {
  const reloads = browser(memory());
  assert.equal(reloadForStaleAssets(), true);
  assert.equal(reloads(), 1);
  // The second failure is the caller's to report: a page that reloads forever
  // takes the tab away from the reader, message and all.
  assert.equal(reloadForStaleAssets(), false);
  assert.equal(reloads(), 1);
});

test("a browser that will not remember still gets the one reload", () => {
  const throws = {
    get: (): string | null => { throw new Error("site data is blocked"); },
    set: (): void => { throw new Error("site data is blocked"); },
  };
  const reloads = browser(throws);
  assert.equal(reloadForStaleAssets(), true);
  assert.equal(reloads(), 1);
});

test("a page with a file open is not taken away to fix a chunk", () => {
  const listeners = new Map<string, (e: { preventDefault: () => void }) => void>();
  Object.defineProperty(globalThis, "window", {
    value: { addEventListener: (name: string, fn: (e: { preventDefault: () => void }) => void) => void listeners.set(name, fn) },
    configurable: true,
  });
  const reloads = browser(memory());
  let open = true;
  watchForStaleAssets(() => !open);
  const fire = (): boolean => {
    let stopped = false;
    listeners.get("vite:preloadError")?.({ preventDefault: () => { stopped = true; } });
    return stopped;
  };
  // The rule database failing while a file is being read is a sentence about
  // the file, not the file. Reloading would answer it by throwing the file away.
  assert.equal(fire(), false);
  assert.equal(reloads(), 0);
  // Nothing open, so the page is the page's own to spend.
  open = false;
  assert.equal(fire(), true);
  assert.equal(reloads(), 1);
});

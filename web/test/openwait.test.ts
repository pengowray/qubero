// Open unpacked after one press: asking the core again while a stream's bytes
// load and while a large one is unpacked, and giving up when the bytes never
// all arrive.

import { test } from "node:test";
import assert from "node:assert/strict";

import { MAX_ROUNDS, waitToOpen, type OpenStep } from "../src/openwait.ts";

type Step = OpenStep<string>;
const done = (value: string): Step => ({ kind: "done", value });
const working: Step = { kind: "working", unpacking: null };
const pending = (...chunks: number[]): Step => ({ kind: "pending", chunks });

/**
 * Run one wait over `replies`, the core's answers in order after the press's
 * own, which is the first. `load` stands in for `Doc.fileChunks`. Resolves
 * with what the wait did, once it ends or `live` turns it off.
 */
function run(
  replies: readonly Step[],
  opts: { load?: (chunks: readonly number[]) => Promise<void>; live?: () => boolean } = {},
): Promise<{ said: string[]; asks: number; loads: number[][] }> {
  const said: string[] = [];
  const loads: number[][] = [];
  let at = 0;
  const next = (): Step => {
    const step = replies[at++];
    assert.ok(step !== undefined, `asked a ${at}th time, with ${replies.length} replies`);
    return step;
  };
  return new Promise((resolve) => {
    const end = (what: string): void => {
      said.push(what);
      resolve({ said, asks: at, loads });
    };
    const live = opts.live ?? ((): boolean => true);
    // A wait turned off while its chunks load ends with nothing said, so the
    // test ends once the wait has had its chance to say something.
    const quietly = (): void => {
      if (!live()) setTimeout(() => resolve({ said, asks: at, loads }), 0);
    };
    waitToOpen(
      next(),
      next,
      (chunks) => {
        loads.push([...chunks]);
        const loaded = (opts.load ?? (() => Promise.resolve()))(chunks);
        loaded.then(quietly, quietly);
        return loaded;
      },
      {
        live,
        working: () => said.push("working"),
        loading: () => said.push("loading"),
        done: (value) => end(`done ${value}`),
        lost: () => end("lost"),
        failed: (error) => end(`failed ${error instanceof Error ? error.message : String(error)}`),
      },
      (then) => queueMicrotask(then),
    );
  });
}

test("a stream waiting on bytes loads them and asks again, through unpacking, to the tab", async () => {
  const r = await run([pending(0, 1), working, working, pending(5), done("tab")]);
  assert.deepEqual(r.said, ["loading", "working", "working", "loading", "done tab"]);
  assert.deepEqual(r.loads, [[0, 1], [5]]);
  assert.equal(r.asks, 5);
});

test("an answer at once is the tab, with nothing loaded", async () => {
  const r = await run([done("tab")]);
  assert.deepEqual(r.said, ["done tab"]);
  assert.deepEqual(r.loads, []);
});

test("the same chunks asked for twice running end the wait", async () => {
  const r = await run([pending(3, 4), pending(3, 4)]);
  assert.deepEqual(r.said, ["loading", "lost"]);
  assert.deepEqual(r.loads, [[3, 4]]);
});

test("chunks pushed out by their own load end the wait, though the list is not the same", async () => {
  // A run of six chunks with room for four: loading them all pushes out the
  // first two, and loading those would push out the next two, and so on.
  const r = await run([pending(0, 1, 2, 3, 4, 5), pending(0, 1)]);
  assert.deepEqual(r.said, ["loading", "lost"]);
  assert.deepEqual(r.loads, [[0, 1, 2, 3, 4, 5]]);
});

test("chunks asked for again after the core got further are loaded again", async () => {
  const r = await run([pending(7), working, pending(7), done("tab")]);
  assert.deepEqual(r.said, ["loading", "working", "loading", "done tab"]);
  assert.deepEqual(r.loads, [[7], [7]]);
});

test("a reply waiting on no chunks at all ends the wait", async () => {
  const r = await run([pending()]);
  assert.deepEqual(r.said, ["lost"]);
});

test("a chunk that cannot be read ends the wait with the read's own error", async () => {
  const r = await run([pending(2)], { load: () => Promise.reject(new Error("Could not read 64 KiB at offset 0x20000")) });
  assert.deepEqual(r.said, ["loading", "failed Could not read 64 KiB at offset 0x20000"]);
  assert.equal(r.asks, 1);
});

test("a wait no longer wanted asks nothing more once its chunks load", async () => {
  let wanted = true;
  const r = await run([pending(1), done("tab")], {
    live: () => wanted,
    load: () => {
      // Another stream is pressed while these load.
      wanted = false;
      return Promise.resolve();
    },
  });
  assert.deepEqual(r.said, ["loading"]);
  assert.equal(r.asks, 1);
});

test("a wait no longer wanted says nothing of a read that failed", async () => {
  let wanted = true;
  const r = await run([pending(1)], {
    live: () => wanted,
    load: () => {
      wanted = false;
      return Promise.reject(new Error("gone"));
    },
  });
  assert.deepEqual(r.said, ["loading"]);
});

test("a wait that loads new chunks every round still stops after the last round allowed", async () => {
  const replies = Array.from({ length: MAX_ROUNDS + 1 }, (_, i) => pending(i));
  const r = await run(replies);
  assert.equal(r.loads.length, MAX_ROUNDS);
  assert.equal(r.said.at(-1), "lost");
});

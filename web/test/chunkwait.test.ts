// Waiting on chunks of the file: reading a long range in pieces, the rule
// that stops a wait that has stopped moving while letting one that keeps
// moving go on, and grouping missing chunks into one read per stretch.

import { test } from "node:test";
import assert from "node:assert/strict";

import { askUntilLoaded, ChunkWait, MAX_ROUNDS, readInPieces, roundLimit, stretches } from "../src/chunkwait.ts";

/** Byte `i` of the file or the stream, a pattern that does not repeat every
 *  256 bytes, so that a piece joined in the wrong place shows. */
const byteAt = (i: number): number => (i * 7 + (i >> 8)) & 0xff;

/**
 * A chunk store that keeps `keeps` chunks and pushes out the one loaded
 * longest ago, as the file's store does. `load` stands in for
 * `Doc.fileChunks`, and records what each round loaded.
 */
function store(keeps: number): {
  readonly has: (c: number) => boolean;
  readonly load: (chunks: readonly number[]) => Promise<void>;
  readonly clear: () => void;
  readonly loads: number[][];
} {
  let held: number[] = [];
  const loads: number[][] = [];
  return {
    has: (c) => held.includes(c),
    load: (chunks) => {
      loads.push([...chunks]);
      for (const c of chunks) held = [...held.filter((h) => h !== c), c];
      while (held.length > keeps) held.shift();
      return Promise.resolve();
    },
    clear: () => {
      held = [];
    },
    loads,
  };
}

type Store = ReturnType<typeof store>;
type Read = (at: number, out: Uint8Array) => readonly number[];

/** A read of the file, as space 0 reads it: every missing chunk of the
 *  range, and zeros until there are none. */
function fileRead(s: Store, chunkBytes: number, reads: [number, number][] = []): Read {
  return (at, out) => {
    reads.push([at, out.length]);
    out.fill(0);
    const missing: number[] = [];
    for (let c = Math.floor(at / chunkBytes); c <= Math.floor((at + out.length - 1) / chunkBytes); c++) {
      if (!s.has(c)) missing.push(c);
    }
    if (missing.length === 0) out.forEach((_, i) => (out[i] = byteAt(at + i)));
    return missing;
  };
}

/**
 * A read of a stream through an index, as a tab reads it: span after span,
 * each needing `chunksPerSpan` chunks of the file, the spans decoded so far
 * kept in a cache of `cacheSpans`. `names` is which missing chunks a read
 * gives: those of the first span it cannot decode, as the core does today,
 * every one of the range, or the first few of the first span.
 */
function streamRead(
  s: Store,
  opts: { readonly spanBytes: number; readonly chunksPerSpan: number; readonly cacheSpans: number; readonly names: "span" | "all" | number },
): Read {
  let cache: number[] = [];
  const keep = (span: number): void => {
    cache = [...cache.filter((c) => c !== span), span];
    while (cache.length > opts.cacheSpans) cache.shift();
  };
  return (at, out) => {
    out.fill(0);
    const missing: number[] = [];
    for (let span = Math.floor(at / opts.spanBytes); span <= Math.floor((at + out.length - 1) / opts.spanBytes); span++) {
      if (cache.includes(span)) {
        keep(span);
        continue;
      }
      const absent: number[] = [];
      for (let c = span * opts.chunksPerSpan; c < (span + 1) * opts.chunksPerSpan; c++) if (!s.has(c)) absent.push(c);
      if (absent.length === 0) {
        keep(span);
        continue;
      }
      missing.push(...absent);
      if (opts.names !== "all") break;
    }
    if (missing.length > 0) return typeof opts.names === "number" ? missing.slice(0, opts.names) : missing;
    out.forEach((_, i) => (out[i] = byteAt(at + i)));
    return [];
  };
}

function expected(at: number, len: number): Uint8Array {
  return Uint8Array.from({ length: len }, (_, i) => byteAt(at + i));
}

test("a range longer than the store is read in pieces and joined in order", async () => {
  const s = store(30);
  const reads: [number, number][] = [];
  // 50 chunks of 100 bytes, from byte 30, with room for 30 chunks.
  const got = await readInPieces({ at: 30, len: 5_000, read: fileRead(s, 100, reads), load: s.load, wait: new ChunkWait(), pieceBytes: 1_000 });
  assert.equal(got.complete, true);
  assert.deepEqual(got.bytes, expected(30, 5_000));
  // Each piece read once while it waits, and once more when it is all there.
  const asked = [...new Set(reads.map(([at, n]) => `${at}+${n}`))];
  assert.deepEqual(asked, ["30+1000", "1030+1000", "2030+1000", "3030+1000", "4030+1000"]);
});

test("a short last piece is read at its own length", async () => {
  const s = store(30);
  const reads: [number, number][] = [];
  const got = await readInPieces({ at: 0, len: 2_500, read: fileRead(s, 100, reads), load: s.load, wait: new ChunkWait(), pieceBytes: 1_000 });
  assert.equal(got.complete, true);
  assert.deepEqual(got.bytes, expected(0, 2_500));
  assert.deepEqual(reads.at(-1), [2_000, 500]);
});

test("a stream whose input is longer than the store reads in pieces a span at a time", async () => {
  // Ten spans of 400 bytes, each needing 8 chunks, over a store of 12 chunks
  // and a cache of 3 spans: 80 chunks of input in all.
  const s = store(12);
  const read = streamRead(s, { spanBytes: 400, chunksPerSpan: 8, cacheSpans: 3, names: "span" });
  const got = await readInPieces({ at: 0, len: 4_000, read, load: s.load, wait: new ChunkWait(), pieceBytes: 1_000 });
  assert.equal(got.complete, true);
  assert.deepEqual(got.bytes, expected(0, 4_000));
  // Every chunk of the input loaded once: no span was decoded twice.
  assert.equal(s.loads.flat().length, 80);
});

test("the same stream read in one piece gives up, since its first spans are gone by the last", async () => {
  const s = store(12);
  const read = streamRead(s, { spanBytes: 400, chunksPerSpan: 8, cacheSpans: 3, names: "span" });
  const got = await readInPieces({ at: 0, len: 4_000, read, load: s.load, wait: new ChunkWait(), pieceBytes: 4_000 });
  assert.equal(got.complete, false);
});

test("a read that names only a few chunks at a time still completes", async () => {
  const s = store(12);
  const read = streamRead(s, { spanBytes: 400, chunksPerSpan: 8, cacheSpans: 3, names: 2 });
  const got = await readInPieces({ at: 0, len: 4_000, read, load: s.load, wait: new ChunkWait(), pieceBytes: 1_000 });
  assert.equal(got.complete, true);
  assert.deepEqual(got.bytes, expected(0, 4_000));
  assert.ok(s.loads.every((l) => l.length <= 2));
});

test("a read that names every missing chunk of a piece completes when a round loads only some", async () => {
  // A piece over three spans names 24 chunks, twice what the store keeps.
  const s = store(12);
  const read = streamRead(s, { spanBytes: 400, chunksPerSpan: 8, cacheSpans: 3, names: "all" });
  const got = await readInPieces({ at: 0, len: 4_000, read, load: s.load, wait: new ChunkWait({ most: 6 }), pieceBytes: 1_000 });
  assert.equal(got.complete, true);
  assert.deepEqual(got.bytes, expected(0, 4_000));
  assert.ok(s.loads.every((l) => l.length <= 6));
});

test("loading all of a list longer than the store pushes out its own first chunks, and the read gives up", async () => {
  const s = store(12);
  const read = streamRead(s, { spanBytes: 400, chunksPerSpan: 8, cacheSpans: 3, names: "all" });
  const got = await readInPieces({ at: 0, len: 4_000, read, load: s.load, wait: new ChunkWait(), pieceBytes: 1_000 });
  assert.equal(got.complete, false);
  assert.equal(s.loads.length, 1);
});

test("a piece that needs more of the file at once than the store keeps gives up, and no later piece is read", async () => {
  // A piece of 10 chunks over a store of 5.
  const s = store(5);
  const reads: [number, number][] = [];
  const got = await readInPieces({ at: 0, len: 3_000, read: fileRead(s, 100, reads), load: s.load, wait: new ChunkWait(), pieceBytes: 1_000 });
  assert.equal(got.complete, false);
  assert.deepEqual(s.loads, [[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]]);
  assert.ok(reads.every(([at]) => at === 0), JSON.stringify(reads));
});

test("a chunk loaded for one piece and pushed out before the next asks for it is loaded again", async () => {
  // Chunk 1 holds the end of the first piece and the start of the second,
  // and something else fills the store between the two.
  const s = store(4);
  const plain = fileRead(s, 400);
  const read: Read = (at, out) => {
    const missing = plain(at, out);
    if (missing.length === 0) s.clear();
    return missing;
  };
  const got = await readInPieces({ at: 0, len: 1_200, read, load: s.load, wait: new ChunkWait(), pieceBytes: 600 });
  assert.equal(got.complete, true);
  assert.deepEqual(got.bytes, expected(0, 1_200));
  assert.deepEqual(s.loads, [[0, 1], [1, 2]]);
});

test("a chunk that cannot be read fails the read with the load's own error", async () => {
  const s = store(30);
  const failing = (): Promise<void> => Promise.reject(new Error("Could not read 64 KiB at offset 0x20000"));
  await assert.rejects(
    readInPieces({ at: 0, len: 1_000, read: fileRead(s, 100), load: failing, wait: new ChunkWait(), pieceBytes: 500 }),
    /Could not read 64 KiB at offset 0x20000/,
  );
});

test("a read of no bytes is complete without asking the core", async () => {
  const reads: [number, number][] = [];
  const got = await readInPieces({ at: 10, len: 0, read: fileRead(store(1), 100, reads), load: () => Promise.resolve(), wait: new ChunkWait() });
  assert.equal(got.complete, true);
  assert.equal(got.bytes.length, 0);
  assert.deepEqual(reads, []);
});

/** An ask that names chunk `i` on its `i`th round and nothing after `rounds` rounds. */
function newChunkEachRound(rounds: number): () => { value: number; missing: readonly number[] } {
  let i = 0;
  return () => {
    const at = i++;
    return { value: at, missing: at < rounds ? [at] : [] };
  };
}

test("a wait that loads a chunk it has not loaded before every round goes past MAX_ROUNDS when its file has more chunks", async () => {
  // A check over a long run, or the strings scan, with no progress to go by.
  const rounds = MAX_ROUNDS + 100;
  const limit = roundLimit(rounds * 65_536, 65_536);
  const loads: number[][] = [];
  const got = await askUntilLoaded({
    ask: newChunkEachRound(rounds),
    load: (chunks) => (loads.push([...chunks]), Promise.resolve()),
    wait: new ChunkWait({ limit }),
  });
  assert.equal(got.complete, true);
  assert.equal(loads.length, rounds);
});

test("a wait still stops at its limit, as a backstop", async () => {
  const loads: number[][] = [];
  const got = await askUntilLoaded({
    ask: newChunkEachRound(Infinity),
    load: (chunks) => (loads.push([...chunks]), Promise.resolve()),
    wait: new ChunkWait({ limit: 10 }),
  });
  assert.equal(got.complete, false);
  assert.equal(loads.length, 10);
});

test("a round that asks again for a chunk loaded in this wait ends it, with the last answer", async () => {
  const answers = [[1], [2], [3, 1]];
  let i = 0;
  const got = await askUntilLoaded({
    ask: () => ({ value: i, missing: answers[i++] ?? [] }),
    load: () => Promise.resolve(),
    wait: new ChunkWait(),
  });
  assert.deepEqual(got, { value: 2, complete: false });
});

test("the backstop is the file's chunk count, and never less than MAX_ROUNDS", () => {
  assert.equal(roundLimit(1_000, 65_536), MAX_ROUNDS);
  assert.equal(roundLimit(512 * 1024 * 1024, 65_536), 8_192);
  assert.equal(roundLimit(512 * 1024 * 1024 + 1, 65_536), 8_193);
});

test("missing chunks are read as one stretch for each run of neighbours, in order, each once", () => {
  const got = stretches({ chunks: [5, 3, 4, 4, 9, 10, 1], total: 100, skip: () => false, most: 256 });
  assert.deepEqual(got, [
    [1, 2],
    [3, 6],
    [9, 11],
  ]);
});

test("a chunk here already or on its way splits the stretch it is in", () => {
  const got = stretches({ chunks: [3, 4, 5, 6], total: 100, skip: (c) => c === 4, most: 256 });
  assert.deepEqual(got, [
    [3, 4],
    [5, 7],
  ]);
});

test("a chunk past the end of the file is not read", () => {
  const got = stretches({ chunks: [8, 9, 10, 11], total: 10, skip: () => false, most: 256 });
  assert.deepEqual(got, [[8, 10]]);
});

test("a run of neighbours longer than one read takes in is split", () => {
  const got = stretches({ chunks: Array.from({ length: 10 }, (_, i) => i), total: 100, skip: () => false, most: 4 });
  assert.deepEqual(got, [
    [0, 4],
    [4, 8],
    [8, 10],
  ]);
});

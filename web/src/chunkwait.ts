// Waiting on chunks of the file. Every loop in doc.ts that reads, loads the
// chunks the read names and reads again stops by the rule in `ChunkWait`;
// `readInPieces` is that loop over a long range, a few MiB at a time; and
// `stretches` groups missing chunks so that each run of neighbours is one read
// of the file. Kept apart from doc.ts, which cannot be loaded without the
// editor's wasm, so that node tests cover them.

/**
 * How many rounds one wait loads chunks, since the core last said it got
 * further, before it gives up, for a wait that is given no other limit. A
 * backstop only: see `ChunkWait` for the rule that stops a wait that has
 * stopped moving, and `roundLimit` for the limit the waits in doc.ts use.
 */
export const MAX_ROUNDS = 512;

/**
 * The backstop for a wait over a file of `fileBytes`: as many rounds as the
 * file has chunks, and never fewer than `MAX_ROUNDS`. A wait that keeps
 * moving loads in each round at least one chunk it had not loaded since the
 * core last got further, so with no "working" reply in between it cannot go
 * more rounds than the file has chunks. Past that, the core is asking for
 * chunks that are not the file's.
 */
export function roundLimit(fileBytes: number, chunkBytes: number): number {
  return Math.max(MAX_ROUNDS, Math.ceil(fileBytes / chunkBytes));
}

/**
 * What one wait for chunks of the file has loaded, and whether it is still
 * moving.
 *
 * A round is moving when none of the chunks it asks for was loaded in this
 * wait since the core last got further. A round that asks again for a chunk
 * loaded here has stopped moving: the chunk was pushed out of the store
 * before the core could use it, which happens when the core needs more at
 * once than the store keeps, and loading it again would go round for ever.
 * The list is a different one each time, so comparing it with the last
 * round's list would never stop such a wait.
 *
 * The rounds since the core last got further are counted against `limit`
 * as well, as a backstop.
 */
export class ChunkWait {
  private readonly loaded = new Set<number>();
  private rounds = 0;
  private readonly limit: number;
  private readonly most: number;

  /** `limit` is the backstop (`MAX_ROUNDS` unless given). `most` is how
   *  many chunks one round loads at most: the first ones named, with the
   *  rest left for the core to ask for again. */
  constructor(opts: { readonly limit?: number; readonly most?: number } = {}) {
    this.limit = opts.limit ?? MAX_ROUNDS;
    this.most = opts.most ?? Infinity;
  }

  /** The core said it got further: chunks loaded before may be asked for
   *  again, and the rounds count from here. */
  gotFurther(): void {
    this.loaded.clear();
    this.rounds = 0;
  }

  /** The core did a go of work without saying how far it got. Chunks loaded
   *  before may be asked for again, since the work may have pushed them out
   *  itself, but the rounds go on counting. */
  worked(): void {
    this.loaded.clear();
  }

  /**
   * What to load for a round that asks for `chunks`, or null when the wait
   * should stop: on a round that asks for no chunks, or for a chunk loaded
   * since the core last got further, or once the rounds since then pass the
   * limit. At most `most` chunks, the first named. What this returns counts
   * as loaded from here on.
   */
  next(chunks: readonly number[]): readonly number[] | null {
    this.rounds += 1;
    if (chunks.length === 0 || this.rounds > this.limit || chunks.some((c) => this.loaded.has(c))) return null;
    const load = chunks.length > this.most ? chunks.slice(0, this.most) : chunks;
    for (const c of load) this.loaded.add(c);
    return load;
  }
}

/** One ask of the core: its answer, and the chunks of the file it found
 *  missing, none when the answer is complete. */
export type Asked<T> = { readonly value: T; readonly missing: readonly number[] };

/**
 * Ask until the answer is missing no chunks, loading the chunks each answer
 * names before asking again, or until `wait` says the wait has stopped
 * moving. Resolves with the last answer and whether it was complete. Rejects
 * when `load` does.
 */
export async function askUntilLoaded<T>(opts: {
  readonly ask: () => Asked<T>;
  readonly load: (chunks: readonly number[]) => Promise<void>;
  readonly wait: ChunkWait;
}): Promise<{ readonly value: T; readonly complete: boolean }> {
  for (;;) {
    const got = opts.ask();
    if (got.missing.length === 0) return { value: got.value, complete: true };
    const chunks = opts.wait.next(got.missing);
    if (chunks === null) return { value: got.value, complete: false };
    await opts.load(chunks);
  }
}

/**
 * How much of a range `readInPieces` reads at a time: 4 MiB, a quarter of a
 * stream's span cache (16 MiB of output).
 *
 * A read of a stream read through an index decodes every span it touches,
 * one after another. Each round decodes the spans whose chunks are loaded and
 * stops at the first span whose chunks are not, and the spans decoded so far
 * stay in the span cache, so the next round does not need their chunks
 * again. A piece is complete once every span it touches is in the cache or
 * has its chunks loaded.
 *
 * A deflate span holds at most 8 MiB of output. When the spans are at least
 * as long as a piece, a piece touches at most two of them, 16 MiB at most.
 * When they are shorter, the spans it touches add up to less than three
 * pieces, 12 MiB. Either way they fit in the cache together. Only spans of
 * very different lengths, such as a short one between two of 8 MiB, add up
 * to more, and then the chunks of a span pushed out of the cache are most
 * likely still in the store, which keeps 32 MiB. A piece of 16 MiB could
 * touch 32 MiB of spans of any length. A smaller piece does not lower the
 * 16 MiB of two long spans, and means more reads: a 64 MiB picture is 16
 * reads as it is.
 *
 * A zstd or xz span made of one frame or block can hold up to 64 MiB of
 * output, which no cache of 16 MiB keeps, and is decoded from its start for
 * every read that touches it, whatever the piece. Pieces neither help nor
 * hurt there.
 */
export const PIECE_BYTES = 4 * 1024 * 1024;

/**
 * The bytes in `[at, at + len)`, read `pieceBytes` at a time: each piece is
 * read, the chunks it names are loaded, and it is read again until it is
 * complete, by the rule in `ChunkWait`. A piece that completes is the core
 * getting further, so the chunks loaded for it may be asked for again by
 * the next piece. Stops at the first piece that does not complete, with
 * `complete` false. Rejects when `load` does.
 *
 * `read` fills `out` with the bytes from `at` and says which chunks of the
 * file it found missing.
 */
export async function readInPieces(opts: {
  readonly at: number;
  readonly len: number;
  readonly read: (at: number, out: Uint8Array) => readonly number[];
  readonly load: (chunks: readonly number[]) => Promise<void>;
  readonly wait: ChunkWait;
  readonly pieceBytes?: number;
}): Promise<{ readonly bytes: Uint8Array; readonly complete: boolean }> {
  const bytes = new Uint8Array(opts.len);
  const piece = opts.pieceBytes ?? PIECE_BYTES;
  for (let o = 0; o < opts.len; o += piece) {
    const out = bytes.subarray(o, Math.min(o + piece, opts.len));
    const got = await askUntilLoaded({
      ask: () => ({ value: null, missing: opts.read(opts.at + o, out) }),
      load: opts.load,
      wait: opts.wait,
    });
    if (!got.complete) return { bytes, complete: false };
    opts.wait.gotFurther();
  }
  return { bytes, complete: true };
}

/** Chunks `[start, end)` of the file: one read. */
export type Stretch = readonly [start: number, end: number];

/**
 * `chunks` as stretches of neighbouring chunks, in order, so that each
 * stretch is one read of the file. Each chunk is in at most one stretch, and
 * none is in any when `skip` says so (it is here already, or on its way) or
 * when it is not one of the file's `total` chunks. No stretch is longer than
 * `most` chunks; a longer run of neighbours is split.
 */
export function stretches(opts: {
  readonly chunks: Iterable<number>;
  readonly total: number;
  readonly skip: (chunk: number) => boolean;
  readonly most: number;
}): Stretch[] {
  const wanted = [...new Set(opts.chunks)]
    .filter((c) => Number.isInteger(c) && c >= 0 && c < opts.total && !opts.skip(c))
    .sort((a, b) => a - b);
  const out: [number, number][] = [];
  for (const c of wanted) {
    const last = out[out.length - 1];
    if (last !== undefined && last[1] === c && c - last[0] < opts.most) last[1] = c + 1;
    else out.push([c, c + 1]);
  }
  return out;
}

// Where the lines of a file start.
//
// The text view used to know only the lines on screen, so scrolling back over
// text already read asked the file for it again, the line ending it marked as
// the odd one out changed as the screen did, and the scrollbar stood for an
// estimate that drifted and had to be nudged back. All three are the same
// missing thing: nothing remembered where the lines were.
//
// This remembers. The file is scanned for line starts in the background, a few
// megabytes at a time, and what has been scanned is kept. Once the scan has
// passed a byte, the line holding it is known exactly: which line number it is,
// where it starts, how many lines are in front of it. Before that it is an
// estimate from the average line so far, and the estimate is replaced rather
// than corrected as the scan goes by.
//
// The index is a chain of segments rather than one array. A segment is a
// stretch of the file whose line starts are known, and they are never welded
// together, because an edit at a byte only invalidates the segment holding it
// and the ones after it: keeping them apart is what lets a keystroke in a
// hundred-megabyte file keep the ninety-nine megabytes in front of it.
//
// A jump to somewhere the scan has not reached puts a segment down around it,
// detached from the front of the file. Such a segment is provisional: where a
// line is cut for being too long depends on where the line before it started,
// and a scan that began in the middle cannot know that. When the scan from the
// front arrives, it wins, and a detached segment that does not line up with it
// is dropped rather than believed.

/** How many of each line ending a stretch of the file used. */
export type Endings = { readonly lf: number; readonly cr: number; readonly crlf: number };

/** A stretch of the file whose line starts are known. */
export type Segment = {
  /** Where the first known line starts, which is `starts[0]`. */
  readonly from: number;
  /** Where the knowledge stops: the start of the first line not in `starts`. */
  readonly to: number;
  readonly starts: Float64Array;
  readonly endings: Endings;
  /** How many lines are in front of `starts[0]`, when that is known: the
   *  segment is joined to the front of the file by an unbroken chain. Null in
   *  a detached segment, where nothing can say what line number it is at. */
  firstLine: number | null;
};

/** A line's place in the file. `line` is a line number when the index reaches
 *  the front of the file, and null when only the segment is known. */
export type Placed = { readonly at: number; readonly line: number | null; readonly segment: Segment; readonly index: number };

/** How long a line is taken to be before the file has been scanned at all. */
export const GUESS_LINE = 64;

/** How many line starts are kept, which at eight bytes each is 256 MiB. Past
 *  this the index stops growing and the rest of the file stays an estimate,
 *  which is what it was for the whole file before there was an index. */
export const MAX_STARTS = 32_000_000;

export class LineIndex {
  private segs: Segment[] = [];
  private starts = 0;

  private length: number;
  /** Where the text begins, past any byte-order mark. */
  readonly base: number;

  constructor(length: number, base = 0) {
    this.length = length;
    this.base = base;
  }

  /** The segments, front to back. */
  get segments(): readonly Segment[] {
    return this.segs;
  }

  get lengthBytes(): number {
    return this.length;
  }

  setLength(n: number): void {
    this.length = n;
  }

  /** Throw the whole index away, which is what a change of encoding does:
   *  where the lines are is a fact about a reading of the file, not the file. */
  clear(): void {
    this.segs = [];
    this.starts = 0;
  }

  /** How far an unbroken chain of segments from the front of the file reaches.
   *  Every line before this is known by number. */
  get indexedTo(): number {
    let end = this.base;
    for (const s of this.segs) {
      if (s.from !== end) break;
      end = s.to;
    }
    return end;
  }

  /** True when every line in the file is known. */
  get complete(): boolean {
    return this.indexedTo >= this.length;
  }

  /** True when the index has grown as far as it is allowed to. */
  get full(): boolean {
    return this.starts >= MAX_STARTS;
  }

  /** Lines known by number, which is the lines up to `indexedTo`. */
  get knownLines(): number {
    let end = this.base;
    let n = 0;
    for (const s of this.segs) {
      if (s.from !== end) break;
      end = s.to;
      n += s.starts.length;
    }
    return n;
  }

  /** How long a line is in this file, from what has been scanned. */
  get bytesPerLine(): number {
    const known = this.knownLines;
    const span = this.indexedTo - this.base;
    if (known === 0 || span <= 0) return GUESS_LINE;
    return Math.max(1, span / known);
  }

  /** How many lines the file has: exact once the scan has run out of file,
   *  and the lines scanned plus an estimate for the rest until then. */
  get totalLines(): number {
    const known = this.knownLines;
    if (this.complete) return Math.max(known, 1);
    const left = Math.max(0, this.length - this.indexedTo);
    return Math.max(1, known + Math.ceil(left / this.bytesPerLine));
  }

  /** True when `totalLines` is a count rather than an estimate. */
  get exact(): boolean {
    return this.complete;
  }

  /** Every line ending seen so far, over everything indexed. */
  get endings(): Endings {
    let lf = 0;
    let cr = 0;
    let crlf = 0;
    for (const s of this.segs) {
      lf += s.endings.lf;
      cr += s.endings.cr;
      crlf += s.endings.crlf;
    }
    return { lf, cr, crlf };
  }

  /**
   * Take in what a scan found. `starts` must be line starts in order and
   * `next` where the scan stopped, which is itself a line start.
   *
   * A scan overlapping a segment that begins earlier is dropped rather than
   * believed: the earlier one is nearer the front of the file and so knows
   * more about where its lines were cut. A scan overlapping later segments
   * takes them over, which is how the chain from the front replaces the
   * provisional segments a jump left behind.
   */
  add(starts: Float64Array, next: number, endings: Endings): void {
    const from = starts[0];
    if (starts.length === 0 || from === undefined) return;
    const to = Math.max(next, from);
    if (this.full) return;
    for (const s of this.segs) {
      if (s.from <= from && s.to > from) return;
    }
    const kept: Segment[] = [];
    for (const s of this.segs) {
      if (s.from >= from && s.from < to) {
        this.starts -= s.starts.length;
        continue;
      }
      kept.push(s);
    }
    kept.push({ from, to, starts, endings, firstLine: null });
    kept.sort((a, b) => a.from - b.from);
    this.segs = kept;
    this.starts += starts.length;
    this.number();
  }

  /** Give every segment joined to the front of the file its line number. */
  private number(): void {
    let end = this.base;
    let n = 0;
    for (const s of this.segs) {
      if (s.from === end) {
        s.firstLine = n;
        end = s.to;
        n += s.starts.length;
      } else {
        s.firstLine = null;
      }
    }
  }

  /** Where the background scan should carry on from, or null when there is
   *  nothing left to scan. */
  get gap(): number | null {
    if (this.full) return null;
    const end = this.indexedTo;
    return end >= this.length ? null : end;
  }

  /** The segment holding a byte, or null when nothing has scanned it. */
  segmentAt(byte: number): Segment | null {
    for (const s of this.segs) {
      if (byte >= s.from && byte < s.to) return s;
    }
    return null;
  }

  /** Where the line holding a byte starts, and which line it is. Null when
   *  nothing has scanned that far. */
  place(byte: number): Placed | null {
    const seg = this.segmentAt(byte);
    if (seg === null) return null;
    const i = before(seg.starts, byte);
    const at = seg.starts[i];
    if (at === undefined) return null;
    return { at, line: seg.firstLine === null ? null : seg.firstLine + i, segment: seg, index: i };
  }

  /** Which line a byte is on, when the file has been scanned that far. */
  lineAt(byte: number): number | null {
    return this.place(byte)?.line ?? null;
  }

  /** Where a line starts, when the file has been scanned that far. */
  byteOfLine(n: number): number | null {
    if (n < 0) return null;
    for (const s of this.segs) {
      if (s.firstLine === null) continue;
      const i = n - s.firstLine;
      if (i >= 0 && i < s.starts.length) return s.starts[i] ?? null;
    }
    // Past the last line of a fully scanned file is the end of the file, which
    // is where a scrollbar at its floor lands.
    return this.complete && n >= this.knownLines ? this.length : null;
  }

  /** Where a line probably starts, for the part of the file nothing has
   *  scanned. An estimate, and it says so by being this rather than
   *  `byteOfLine`. */
  guessByteOfLine(n: number): number {
    const exact = this.byteOfLine(n);
    if (exact !== null) return exact;
    const known = this.knownLines;
    return Math.min(this.length, this.indexedTo + Math.max(0, n - known) * this.bytesPerLine);
  }

  /** Which line a byte is probably on. */
  guessLineAt(byte: number): number {
    const exact = this.lineAt(byte);
    if (exact !== null) return exact;
    const end = this.indexedTo;
    if (byte < end) return 0;
    return this.knownLines + Math.floor((byte - end) / this.bytesPerLine);
  }

  /** An edit landed at `byte`. Everything from the line holding it onwards is
   *  no longer known and is scanned again. */
  dropFrom(byte: number): void {
    const from = this.place(byte)?.at ?? byte;
    const kept: Segment[] = [];
    for (const s of this.segs) {
      if (s.to <= from) {
        kept.push(s);
        continue;
      }
      this.starts -= s.starts.length;
    }
    this.segs = kept;
    this.number();
  }

  /**
   * An edit at `byte` changed the file's length by `delta` without touching a
   * line ending, so every line after it is the line it was, that much further
   * along. Typing a letter is this; anything that could make or unmake an
   * ending is a `dropFrom` instead.
   */
  shiftFrom(byte: number, delta: number): void {
    this.length = Math.max(0, this.length + delta);
    if (delta === 0) return;
    const here = this.place(byte);
    // The line the edit landed on grew or shrank, so its own start stands and
    // every start after it moves.
    const after = here === null ? byte : here.at + 1;
    const kept: Segment[] = [];
    for (const s of this.segs) {
      if (s.to <= after) {
        kept.push(s);
        continue;
      }
      const starts = s.starts.slice();
      for (let i = 0; i < starts.length; i++) {
        const v = starts[i] ?? 0;
        if (v >= after) starts[i] = v + delta;
      }
      const first = starts[0] ?? s.from;
      kept.push({ from: first, to: s.to + delta, starts, endings: s.endings, firstLine: s.firstLine });
    }
    this.segs = kept;
    this.number();
  }
}

/** The index of the last value at or before `want`, in a sorted array. */
export function before(a: Float64Array, want: number): number {
  let lo = 0;
  let hi = a.length - 1;
  let best = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((a[mid] ?? 0) <= want) {
      best = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return best;
}

// The file format identification patterns listed on Wikidata, and which of
// them a file's bytes match. The patterns come from wikidata/formats.json,
// which tools/wikidata/build.mjs writes; this reads that file and matches.
//
// A match here is weak evidence. Most patterns were imported from TrID and
// PRONOM, and hundreds of formats share a prefix: every format that is XML
// underneath starts `<?xml`, every one that is ZIP starts `PK\3\4`, and 140 of
// them are identified by nothing more than a first byte of `<`. So a match is
// ranked by how many bytes it pinned down, and one that agrees with the file's
// extension goes first.

/** One pattern: canonical PRONOM syntax, its offset, and whether that offset
 *  counts back from the end of the file. */
export type WikiSig = readonly [pattern: string, offset: number, from?: "eof"];

export type WikiFormat = {
  /** The Wikidata item, `Q1047541`. */
  readonly id: string;
  readonly label: string;
  readonly desc?: string;
  /** Extensions from Wikidata, lowercase, no dot. */
  readonly ext?: readonly string[];
  /** Extensions only English Wikipedia's infobox gives. */
  readonly wpExt?: readonly string[];
  readonly mime?: readonly string[];
  /** English Wikipedia article title. */
  readonly wp?: string;
  /** For an item with no article: the format it is a version or part of, which has one. */
  readonly parent?: { readonly id: string; readonly label: string; readonly wp: string };
  readonly sigs: readonly WikiSig[];
};

export type WikiData = { readonly source: string; readonly fetched: string; readonly formats: readonly WikiFormat[] };

export type WikiMatch = {
  readonly format: WikiFormat;
  /** The pattern that matched, as Wikidata's signature syntax writes it. */
  readonly pattern: string;
  readonly offset: number;
  readonly fromEnd: boolean;
  /** Bytes the pattern pins to a value. */
  readonly fixed: number;
  /** The file's own extension is one the format lists. */
  readonly extensionAgrees: boolean;
};

/** A compiled pattern. Gaps have a minimum and maximum length. */
type Token =
  | { readonly k: "lit"; readonly b: Uint8Array }
  | { readonly k: "gap"; readonly min: number; readonly max: number }
  | { readonly k: "alt"; readonly opts: readonly Token[][] }
  | { readonly k: "range"; readonly lo: number; readonly hi: number; readonly neg: boolean };

/** Compile PRONOM signature syntax. Throws on anything else: build.mjs has
 *  already checked every pattern, so a failure here is a bug. */
export function compile(pattern: string): Token[] {
  let i = 0;
  const seq = (inAlt: boolean): Token[] => {
    const out: Token[] = [];
    let lit: number[] = [];
    const flush = (): void => {
      if (lit.length > 0) out.push({ k: "lit", b: Uint8Array.from(lit) });
      lit = [];
    };
    while (i < pattern.length) {
      const c = pattern[i] ?? "";
      if (inAlt && (c === "|" || c === ")")) break;
      if (/[0-9A-F]/.test(c)) {
        lit.push(parseInt(pattern.slice(i, i + 2), 16));
        i += 2;
        continue;
      }
      flush();
      if (c === "?") {
        out.push({ k: "gap", min: 1, max: 1 });
        i += 2;
      } else if (c === "*") {
        out.push({ k: "gap", min: 0, max: Infinity });
        i += 1;
      } else if (c === "{") {
        const m = /^\{(\d+)(?:-(\d+|\*))?\}/.exec(pattern.slice(i));
        if (m === null) throw new Error(`bad gap in ${pattern}`);
        const min = Number(m[1]);
        const max = m[2] === undefined ? min : m[2] === "*" ? Infinity : Number(m[2]);
        out.push({ k: "gap", min, max });
        i += m[0].length;
      } else if (c === "[") {
        const m = /^\[(!?)([0-9A-F]{2})(?::([0-9A-F]{2}))?\]/.exec(pattern.slice(i));
        if (m === null) throw new Error(`bad range in ${pattern}`);
        const lo = parseInt(m[2] ?? "", 16);
        out.push({ k: "range", lo, hi: m[3] === undefined ? lo : parseInt(m[3], 16), neg: m[1] === "!" });
        i += m[0].length;
      } else if (c === "(") {
        i += 1;
        const opts: Token[][] = [];
        for (;;) {
          opts.push(seq(true));
          const d = pattern[i];
          i += 1;
          if (d === ")") break;
          if (d !== "|") throw new Error(`unclosed alternatives in ${pattern}`);
        }
        out.push({ k: "alt", opts });
      } else throw new Error(`unexpected ${c} in ${pattern}`);
    }
    flush();
    return out;
  };
  return seq(false);
}

/** How much backtracking one pattern may do before it counts as no match.
 *  Only `*` and ranged gaps backtrack, and only a few dozen patterns have one. */
const STEP_LIMIT = 200_000;

/**
 * Where the pattern ends when it matches `bytes` starting at `at`, or -1.
 * The first way it matches wins, which for a pattern with gaps is the
 * shortest.
 */
export function matchAt(tokens: readonly Token[], bytes: Uint8Array, at: number): number {
  let steps = 0;
  const run = (ts: readonly Token[], ti: number, pos: number, then: (pos: number) => number): number => {
    if (++steps > STEP_LIMIT) return -1;
    if (ti === ts.length) return then(pos);
    const t = ts[ti];
    if (t === undefined) return -1;
    switch (t.k) {
      case "lit": {
        if (pos + t.b.length > bytes.length) return -1;
        for (let j = 0; j < t.b.length; j++) if (bytes[pos + j] !== t.b[j]) return -1;
        return run(ts, ti + 1, pos + t.b.length, then);
      }
      case "range": {
        const b = bytes[pos];
        if (b === undefined || (b >= t.lo && b <= t.hi) === t.neg) return -1;
        return run(ts, ti + 1, pos + 1, then);
      }
      case "gap": {
        const most = Math.min(t.max, bytes.length - pos);
        for (let g = t.min; g <= most; g++) {
          const end = run(ts, ti + 1, pos + g, then);
          if (end >= 0) return end;
        }
        return -1;
      }
      case "alt": {
        for (const opt of t.opts) {
          const end = run(opt, 0, pos, (p) => run(ts, ti + 1, p, then));
          if (end >= 0) return end;
        }
        return -1;
      }
    }
  };
  return run(tokens, 0, at, (pos) => pos);
}

/** Bytes pinned to a value: literals, and the shortest option of alternatives. */
export function fixedBytes(tokens: readonly Token[]): number {
  let n = 0;
  for (const t of tokens) {
    if (t.k === "lit") n += t.b.length;
    else if (t.k === "alt") n += Math.min(...t.opts.map(fixedBytes));
  }
  return n;
}

/** The shortest run of bytes the pattern can match, to know where to start
 *  looking for one measured from the end. */
function minLength(tokens: readonly Token[]): number {
  let n = 0;
  for (const t of tokens) {
    if (t.k === "lit") n += t.b.length;
    else if (t.k === "range") n += 1;
    else if (t.k === "gap") n += t.min;
    else n += Math.min(...t.opts.map(minLength));
  }
  return n;
}

type Compiled = { readonly format: WikiFormat; readonly sig: WikiSig; readonly tokens: Token[]; readonly fixed: number };

/** Every pattern compiled once, for a set of formats. */
export function compileAll(data: WikiData): Compiled[] {
  const out: Compiled[] = [];
  for (const format of data.formats) {
    for (const sig of format.sigs) {
      const tokens = compile(sig[0]);
      out.push({ format, sig, tokens, fixed: fixedBytes(tokens) });
    }
  }
  return out;
}

/** The part of the file name after its last dot, lowercase, or "". */
export function extensionOf(name: string): string {
  const base = name.slice(name.lastIndexOf("/") + 1);
  const dot = base.lastIndexOf(".");
  return dot <= 0 ? "" : base.slice(dot + 1).toLowerCase();
}

export type FileBytes = {
  /** The start of the file. */
  readonly head: Uint8Array;
  /** The end of the file, which is the same bytes as `head` for a small one. */
  readonly tail: Uint8Array;
  readonly name: string;
};

/**
 * The formats whose patterns the file matches, best first: one match per
 * format, the one pinning the most bytes. A pattern from the start of the file
 * must match at its offset exactly. One from the end matches anywhere in the
 * last `offset` bytes, since that is how PRONOM means it: the PDF `%%EOF` is
 * given at 1024, and is followed by however many line endings the writer liked.
 */
export function matchFormats(compiled: readonly Compiled[], file: FileBytes): WikiMatch[] {
  const ext = extensionOf(file.name);
  const best = new Map<string, WikiMatch>();
  for (const c of compiled) {
    const [pattern, offset, from] = c.sig;
    let ok = false;
    if (from === "eof") {
      const { tail } = file;
      const earliest = Math.max(0, tail.length - offset - minLength(c.tokens) - 64);
      for (let at = tail.length - minLength(c.tokens); at >= earliest && !ok; at--) {
        const end = matchAt(c.tokens, tail, at);
        ok = end >= 0 && tail.length - end <= offset;
      }
    } else {
      ok = offset < file.head.length && matchAt(c.tokens, file.head, offset) >= 0;
    }
    if (!ok) continue;
    const prev = best.get(c.format.id);
    if (prev !== undefined && prev.fixed >= c.fixed) continue;
    const agrees = ext !== "" && ((c.format.ext?.includes(ext) ?? false) || (c.format.wpExt?.includes(ext) ?? false));
    best.set(c.format.id, { format: c.format, pattern, offset, fromEnd: from === "eof", fixed: c.fixed, extensionAgrees: agrees });
  }
  return [...best.values()].sort(
    (a, b) =>
      b.fixed - a.fixed ||
      Number(b.extensionAgrees) - Number(a.extensionAgrees) ||
      a.format.label.localeCompare(b.format.label),
  );
}

/**
 * A match good enough to name a file nothing else could: at least four bytes,
 * the kind of magic number a format picks on purpose, and either the only
 * format at that strength or the only one there whose extension agrees.
 * Null when the best is a tie between formats that cannot be told apart.
 */
export const NAMING_MIN_BYTES = 4;
export function namingMatch(matches: readonly WikiMatch[]): WikiMatch | null {
  const top = matches[0];
  if (top === undefined || top.fixed < NAMING_MIN_BYTES) return null;
  const tied = matches.filter((m) => m.fixed === top.fixed);
  if (tied.length === 1) return top;
  const agreeing = tied.filter((m) => m.extensionAgrees);
  return agreeing.length === 1 ? (agreeing[0] ?? null) : null;
}

export const wikidataUrl = (id: string): string => `https://www.wikidata.org/wiki/${id}`;
export const wikipediaUrl = (title: string): string =>
  `https://en.wikipedia.org/wiki/${encodeURIComponent(title.replace(/ /g, "_")).replace(/%2F/g, "/")}`;

let loaded: Promise<Compiled[] | null> | null = null;

/** The compiled patterns, fetched on first use; null when they cannot be had. */
export function loadWikiFormats(): Promise<Compiled[] | null> {
  loaded ??= fetch("wikidata/formats.json")
    .then((r) => (r.ok ? (r.json() as Promise<WikiData>) : null))
    .then((d) => (d === null ? null : compileAll(d)))
    .catch((e: unknown) => {
      console.error("wikidata formats", e);
      return null;
    });
  return loaded;
}

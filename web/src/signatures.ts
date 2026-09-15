// The file signature database, and which of its formats a file's bytes match.
// The patterns come from signatures.json, which tools/signatures.mjs builds
// from two sources: the identification patterns listed on Wikidata, and the
// top-level rules of the file(1) magic files.
//
// A match here is weak evidence. Most of the Wikidata patterns were imported
// from TrID and PRONOM, and hundreds of formats share a prefix: every format
// that is XML underneath starts `<?xml`, every one that is ZIP starts `PK\3\4`,
// and 140 of them are identified by nothing more than a first byte of `<`. So a
// match is ranked by how many bytes it pinned down, a zero counting for half,
// with the file's extension counting for some more when the format lists it,
// and a match worth less than two bytes is dropped unless the extension agrees.

/** One pattern: canonical PRONOM syntax, its offset, and whether that offset
 *  counts back from the end of the file. */
export type Sig = readonly [pattern: string, offset: number, from?: "eof"];

export type SigFormat = {
  /** Which list this came from. */
  readonly source: "wikidata" | "file";
  /** The Wikidata item (`Q1047541`), or the rule's magic file and line (`archive:2345`). */
  readonly id: string;
  readonly label: string;
  /** Extensions the source lists, lowercase, no dot. */
  readonly ext?: readonly string[];
  /** Extensions only English Wikipedia's infobox gives. */
  readonly wpExt?: readonly string[];
  readonly mime?: readonly string[];
  /** English Wikipedia article title. */
  readonly wp?: string;
  /** For an item with no article: the format it is a version or part of, which has one. */
  readonly parent?: { readonly id: string; readonly label: string; readonly wp: string };
  /** The full rule says more once it has read the file's own values, so the
   *  label is only the start of its sentence. */
  readonly unfinished?: boolean;
  /** file(1)'s own measure of how much the rule is worth. Not used in the
   *  ranking here, which counts bytes from both sources the same way. */
  readonly strength?: number;
  readonly sigs: readonly Sig[];
};

export type SigData = {
  /** One line naming the sources and when each was taken. */
  readonly fetched: string;
  readonly formats: readonly SigFormat[];
};

export type SigMatch = {
  readonly format: SigFormat;
  /** The pattern that matched, as PRONOM's signature syntax writes it. */
  readonly pattern: string;
  readonly offset: number;
  readonly fromEnd: boolean;
  /** Bytes the pattern pins to a value. */
  readonly fixed: number;
  /** Those bytes as the ranking counts them, a zero for `ZERO_WORTH`. */
  readonly worth: number;
  /** The file's own extension is one the format lists. */
  readonly extensionAgrees: boolean;
  /** What the ranking sorts by: the worth, and `EXTENSION_WORTH` more when the extension agrees. */
  readonly score: number;
};

/**
 * What a pinned zero byte counts for, where any other pinned byte counts one.
 * Zero is the commonest byte in a binary file: padding, reserved fields, the
 * high bytes of small numbers. Set by sweeping the sample collection. Counting
 * zeros in full, KDC (`4D4D002A0000000800`) named a Canon DNG, a Nikon NEF and
 * a big-endian TIFF, and Delta RPM (`EDABEEDB0300000000`) named four plain
 * RPMs. A half puts all seven right. Three quarters leaves Delta RPM tied with
 * RPM, so the RPMs go unnamed, and a quarter leaves an ICO's `00000100` too
 * little to name it even with the extension.
 */
export const ZERO_WORTH = 0.5;

/**
 * How many pinned bytes an agreeing extension is worth. Set by sweeping the
 * sample collection. Counting it for nothing, a `.lzh` came out as an Amiga
 * WHDLoad package (five bytes) ahead of LHA (three), and a NetCDF `.nc` as
 * MINC1 (four) ahead of NetCDF (three). Four puts both right without letting
 * an extension outrank a long signature.
 */
export const EXTENSION_WORTH = 4;

/**
 * The least worth that counts as a match without the extension behind it,
 * which a zero byte alone, or a `0001`, falls short of.
 * One byte says nothing about a file: "Vue D'Esprit 4 Atmosphere Preset"
 * is a zero at offset 12, which 132 of the 592 sample files have, HDF5 among
 * them, and a lone `M` or `P` at offset 0 made every big-endian TIFF a DMIS
 * file and every ZIP a PrintFox bitmap. With its extension, a one-byte match
 * stays: a `{` in a .json is at least consistent with GeoJSON.
 */
export const LISTING_BYTES_ALONE = 2;

/** A compiled pattern. Gaps have a minimum and maximum length. */
export type Token =
  | { readonly k: "lit"; readonly b: Uint8Array }
  | { readonly k: "gap"; readonly min: number; readonly max: number }
  | { readonly k: "alt"; readonly opts: readonly Token[][] }
  | { readonly k: "range"; readonly lo: number; readonly hi: number; readonly neg: boolean };

/** Compile PRONOM signature syntax. Throws on anything else: the build has
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

/** The pinned bytes as the ranking counts them: `ZERO_WORTH` for a zero, one
 *  for anything else, and the least of the alternatives. */
export function pinnedWorth(tokens: readonly Token[]): number {
  let n = 0;
  for (const t of tokens) {
    if (t.k === "lit") for (const b of t.b) n += b === 0 ? ZERO_WORTH : 1;
    else if (t.k === "alt") n += Math.min(...t.opts.map(pinnedWorth));
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

export type Compiled = {
  readonly format: SigFormat;
  readonly sig: Sig;
  readonly tokens: Token[];
  readonly fixed: number;
  readonly worth: number;
  /** Where this sits in the order `compileAll` built, which breaks a tie
   *  between two signatures of one format that are worth the same.
   *  Without it the answer would depend on which way the index was walked. */
  readonly seq: number;
};

/** Every pattern compiled once, for a set of formats. */
export function compileAll(data: SigData): Compiled[] {
  const out: Compiled[] = [];
  for (const format of data.formats) {
    for (const sig of format.sigs) {
      const tokens = compile(sig[0]);
      out.push({ format, sig, tokens, fixed: fixedBytes(tokens), worth: pinnedWorth(tokens), seq: out.length });
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
 * The patterns arranged so that matching a file is a handful of map lookups
 * rather than a pass over all ten thousand of them.
 *
 * Nearly every pattern is a run of literal bytes at a fixed offset from the
 * start, and those go in `plain`: offset, then length, then the bytes
 * themselves as a key. The key is a latin-1 string, one character a byte, so
 * that a probe is one `subarray` and one `Map.get` with no hex conversion.
 *
 * What is left over goes in `rest` and is matched one at a time as before:
 * a pattern with a gap, alternatives or a byte range, whose length is not
 * fixed, and every pattern measured from the end of the file, of which there
 * are fourteen.
 */
export type SigIndex = {
  readonly plain: ReadonlyMap<number, ReadonlyMap<number, ReadonlyMap<string, readonly Compiled[]>>>;
  readonly rest: readonly Compiled[];
  /** Every offset in `plain`, and the lengths within it shortest first, so
   *  that `matchFormats` can walk them without re-reading the map keys, and
   *  can grow one key rather than build a new one for each length. */
  readonly offsets: readonly { readonly offset: number; readonly lengths: readonly { readonly length: number; readonly keys: ReadonlyMap<string, readonly Compiled[]> }[] }[];
};

/** Bytes as a string, one character a byte. */
const latin1 = (bytes: Uint8Array, from: number, to: number): string => {
  let s = "";
  for (let i = from; i < to; i++) s += String.fromCharCode(bytes[i] ?? 0);
  return s;
};

/** A pattern that is one run of literal bytes and nothing else, or null. */
const wholeLiteral = (tokens: readonly Token[]): Uint8Array | null => {
  const first = tokens[0];
  return tokens.length === 1 && first !== undefined && first.k === "lit" ? first.b : null;
};

export function buildIndex(compiled: readonly Compiled[]): SigIndex {
  const plain = new Map<number, Map<number, Map<string, Compiled[]>>>();
  const rest: Compiled[] = [];
  for (const c of compiled) {
    const bytes = c.sig[2] === "eof" ? null : wholeLiteral(c.tokens);
    if (bytes === null) {
      rest.push(c);
      continue;
    }
    const offset = c.sig[1];
    let byLength = plain.get(offset);
    if (byLength === undefined) plain.set(offset, (byLength = new Map()));
    let byKey = byLength.get(bytes.length);
    if (byKey === undefined) byLength.set(bytes.length, (byKey = new Map()));
    const key = latin1(bytes, 0, bytes.length);
    const here = byKey.get(key);
    if (here === undefined) byKey.set(key, [c]);
    else here.push(c);
  }
  const offsets = [...plain].map(([offset, byLength]) => ({
    offset,
    lengths: [...byLength].map(([length, keys]) => ({ length, keys })).sort((a, b) => a.length - b.length),
  }));
  return { plain, rest, offsets };
}

type Best = { readonly offer: (c: Compiled) => void; readonly matches: () => SigMatch[] };

/** Collects the best match for each format as candidates come in, and ranks
 *  them at the end. */
function bestOf(ext: string): Best {
  const byFormat = new Map<string, Compiled>();
  return {
    offer(c: Compiled): void {
      const prev = byFormat.get(c.format.id);
      // The most worth wins; the earliest signature breaks a tie, so that the
      // answer does not depend on the order the index happened to be walked.
      if (prev !== undefined && (prev.worth > c.worth || (prev.worth === c.worth && prev.seq <= c.seq))) return;
      byFormat.set(c.format.id, c);
    },
    matches(): SigMatch[] {
      const out: SigMatch[] = [];
      for (const c of byFormat.values()) {
        const f = c.format;
        const agrees = ext !== "" && ((f.ext?.includes(ext) ?? false) || (f.wpExt?.includes(ext) ?? false));
        if (c.worth < LISTING_BYTES_ALONE && !agrees) continue;
        out.push({
          format: f,
          pattern: c.sig[0],
          offset: c.sig[1],
          fromEnd: c.sig[2] === "eof",
          fixed: c.fixed,
          worth: c.worth,
          extensionAgrees: agrees,
          score: c.worth + (agrees ? EXTENSION_WORTH : 0),
        });
      }
      return out.sort((a, b) => b.score - a.score || b.worth - a.worth || a.format.label.localeCompare(b.format.label));
    },
  };
}

/** Whether one signature matches, for the patterns the index cannot hold. */
function hits(c: Compiled, file: FileBytes): boolean {
  const [, offset, from] = c.sig;
  if (from !== "eof") return offset < file.head.length && matchAt(c.tokens, file.head, offset) >= 0;
  // One from the end matches anywhere in the last `offset` bytes, since that
  // is how PRONOM means it: the PDF `%%EOF` is given at 1024, and is followed
  // by however many line endings the writer liked.
  const { tail } = file;
  const least = minLength(c.tokens);
  const earliest = Math.max(0, tail.length - offset - least - 64);
  for (let at = tail.length - least; at >= earliest; at--) {
    const end = matchAt(c.tokens, tail, at);
    if (end >= 0 && tail.length - end <= offset) return true;
  }
  return false;
}

/**
 * The formats whose patterns the file matches, best first: one match per
 * format, the one pinning the most bytes. A pattern from the start of the file
 * must match at its offset exactly.
 *
 * The literal patterns are looked up rather than run: for each offset the
 * index holds, and each pattern length at that offset, the file's bytes there
 * are one map key. The lengths at an offset come shortest first and the key
 * for each is the one before it plus the bytes between, so the file is read
 * once per offset rather than once per length.
 */
export function matchFormats(index: SigIndex, file: FileBytes): SigMatch[] {
  const best = bestOf(extensionOf(file.name));
  const { head } = file;
  for (const { offset, lengths } of index.offsets) {
    if (offset >= head.length) continue;
    let key = "";
    let have = 0;
    for (const { length, keys } of lengths) {
      if (offset + length > head.length) break;
      key += latin1(head, offset + have, offset + length);
      have = length;
      const found = keys.get(key);
      if (found === undefined) continue;
      for (const c of found) best.offer(c);
    }
  }
  for (const c of index.rest) if (hits(c, file)) best.offer(c);
  return best.matches();
}

/** The same answer, worked out by running every pattern. The reference the
 *  index is tested against, and slow enough that only the test uses it. */
export function matchFormatsSlowly(compiled: readonly Compiled[], file: FileBytes): SigMatch[] {
  const best = bestOf(extensionOf(file.name));
  for (const c of compiled) if (hits(c, file)) best.offer(c);
  return best.matches();
}

/**
 * A match good enough to name a file nothing else could, or null. It must be
 * the only format with the best score, and either have the file's extension
 * behind it (and more than one byte: a `{` names nothing, even in a .json) or
 * be worth seven bytes on its own. Four unexplained bytes were not enough in
 * the sample sweep: they called a MATLAB file LiteDB and a PowerShell script
 * an ArtCAM model. Seven would have been too low while a zero counted in full:
 * seven bytes called another MATLAB file AceMoney (`00000001000000`, now worth
 * four). Eight is too high now that a zero counts for half: an EDID dump's
 * `00FFFFFFFFFFFF00` is worth seven. At seven, UnityFS bundles and StuffIt 5
 * archives are named too.
 */
export const NAMING_BYTES_WITH_EXTENSION = 2;
export const NAMING_BYTES_ALONE = 7;
export function namingMatch(matches: readonly SigMatch[]): SigMatch | null {
  const top = matches[0];
  if (top === undefined) return null;
  // A tie between two sources naming the same bytes is not a tie between
  // formats: file(1) and Wikidata both know a PNG. The rule's label wins,
  // since it is the one the file(1) module would have written. A tie within
  // one source is two formats that cannot be told apart, and names nothing.
  const tied = matches.filter((m) => m.score === top.score);
  if (tied.length > 1) {
    const fromRules = tied.filter((m) => m.format.source === "file");
    const fromWikidata = tied.filter((m) => m.format.source === "wikidata");
    if (fromRules.length !== 1 || fromWikidata.length !== tied.length - 1) return null;
    const [rule] = fromRules;
    if (rule === undefined) return null;
    const enough = rule.extensionAgrees ? rule.worth >= NAMING_BYTES_WITH_EXTENSION : rule.worth >= NAMING_BYTES_ALONE;
    return enough ? rule : null;
  }
  const enough = top.extensionAgrees ? top.worth >= NAMING_BYTES_WITH_EXTENSION : top.worth >= NAMING_BYTES_ALONE;
  return enough ? top : null;
}

export const wikidataUrl = (id: string): string => `https://www.wikidata.org/wiki/${id}`;
export const wikipediaUrl = (title: string): string =>
  `https://en.wikipedia.org/wiki/${encodeURIComponent(title.replace(/ /g, "_")).replace(/%2F/g, "/")}`;

// ---------------------------------------------------------------------------
// Reading the file the build wrote. Its `about` field says what each column
// is; this turns the columns back into one object a format.

type Columns = {
  readonly qid: readonly number[];
  readonly ruleId: readonly string[];
  readonly label: readonly string[];
  readonly ext: readonly string[];
  readonly mimeWords: readonly string[];
  readonly mimeAt: readonly number[];
  readonly wpAt: readonly number[];
  readonly wpValues: readonly string[];
  readonly parentAt: readonly number[];
  readonly parentValues: readonly [string, string, string][];
  readonly wpExtAt: readonly number[];
  readonly wpExtValues: readonly string[];
  readonly unfinishedAt: readonly number[];
  readonly strength: readonly number[];
};

export type Stored = {
  readonly wikidata: string;
  readonly magicDb: string;
  readonly fileFrom: number;
  readonly formats: Columns;
  readonly sigPattern: readonly string[];
  readonly sigOffsetAt: readonly number[];
  readonly sigOffsetValues: readonly number[];
  readonly sigEofAt: readonly number[];
  readonly sigFormats: readonly (readonly number[])[];
};

const words = (s: string | undefined): string[] | undefined => (s === undefined || s === "" ? undefined : s.split(" "));

/** The columns as formats, each with the signatures that named its row. */
export function decode(stored: Stored): SigData {
  const c = stored.formats;
  const sigs: Sig[][] = c.label.map(() => []);
  const offsets = new Map(stored.sigOffsetAt.map((at, i) => [at, stored.sigOffsetValues[i] ?? 0]));
  const fromEnd = new Set(stored.sigEofAt);
  for (let s = 0; s < stored.sigPattern.length; s++) {
    const sig: Sig = fromEnd.has(s)
      ? [stored.sigPattern[s] ?? "", offsets.get(s) ?? 0, "eof"]
      : [stored.sigPattern[s] ?? "", offsets.get(s) ?? 0];
    for (const row of stored.sigFormats[s] ?? []) sigs[row]?.push(sig);
  }
  const wp = new Map(c.wpAt.map((at, i) => [at, c.wpValues[i] ?? ""]));
  const parent = new Map(c.parentAt.map((at, i) => [at, c.parentValues[i]]));
  const wpExt = new Map(c.wpExtAt.map((at, i) => [at, c.wpExtValues[i] ?? ""]));
  const unfinished = new Set(c.unfinishedAt);
  const formats: SigFormat[] = [];
  for (let i = 0; i < c.label.length; i++) {
    const fromFile = i >= stored.fileFrom;
    const row = i - stored.fileFrom;
    const f: {
      -readonly [K in keyof SigFormat]: SigFormat[K];
    } = {
      source: fromFile ? "file" : "wikidata",
      id: fromFile ? c.ruleId[row] ?? "" : `Q${c.qid[i] ?? 0}`,
      label: c.label[i] ?? "",
      sigs: sigs[i] ?? [],
    };
    const ext = words(c.ext[i]);
    if (ext !== undefined) f.ext = ext;
    const extra = words(wpExt.get(i));
    if (extra !== undefined) f.wpExt = extra;
    const mime = words(c.mimeWords[c.mimeAt[i] ?? -1]);
    if (mime !== undefined) f.mime = mime;
    const article = wp.get(i);
    if (article !== undefined) f.wp = article;
    const p = parent.get(i);
    if (p !== undefined) f.parent = { id: p[0], label: p[1], wp: p[2] };
    if (unfinished.has(i)) f.unfinished = true;
    if (fromFile) f.strength = c.strength[row] ?? 0;
    formats.push(f);
  }
  return { fetched: `Wikidata ${stored.wikidata}, file rules magic-db ${stored.magicDb}`, formats };
}

export type Signatures = { readonly index: SigIndex; readonly compiled: readonly Compiled[]; readonly fetched: string };

let loaded: Promise<Signatures | null> | null = null;

/** The compiled patterns, fetched on first use; null when they cannot be had. */
export function loadSignatures(): Promise<Signatures | null> {
  loaded ??= fetch("signatures.json")
    .then((r) => (r.ok ? (r.json() as Promise<Stored>) : null))
    .then((s) => {
      if (s === null) return null;
      const data = decode(s);
      const compiled = compileAll(data);
      return { index: buildIndex(compiled), compiled, fetched: data.fetched };
    })
    .catch((e: unknown) => {
      console.error("signatures", e);
      return null;
    });
  return loaded;
}

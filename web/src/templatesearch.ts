// Searching for a template: by name, by extension, and by the bytes a file of
// the format starts with. The signature database answers for formats no
// template reads, and those answers come back marked as signatures only.
//
// Every reading of the query is tried at once. `cafe` is a word and also the
// bytes CA FE, and a Java class file starts CAFEBABE: picking one reading
// would hide the other.

import { compile, type Compiled, type SigFormat, type Token } from "./signatures.ts";
import { parseCString, parseHex, signatureHolds, type Where } from "./bytequery.ts";

/** One template the search can offer. `value` is what picking it applies. */
export type TemplateEntry = {
  readonly value: string;
  readonly label: string;
  readonly kind: "builtin" | "kaitai" | "extra";
  /** Extensions, lowercase, no dot. */
  readonly ext: readonly string[];
  /** Byte patterns the format declares, in PRONOM syntax, from the start. */
  readonly sigs: readonly { readonly pattern: string; readonly offset: number }[];
};

export type ByteQuery = { readonly bytes: Uint8Array; readonly where: Where };

export type Query = {
  /** Lowercase, for matching names; empty when the query is bytes only. */
  readonly word: string;
  /** An extension to look for, lowercase, no dot. */
  readonly ext: string | null;
  readonly bytes: readonly ByteQuery[];
};

/** Why a row is in the results. */
export type Reason =
  | { readonly by: "name"; readonly at: number; readonly length: number }
  | { readonly by: "ext"; readonly ext: string }
  | { readonly by: "bytes"; readonly pattern: string; readonly offset: number; readonly fromEnd: boolean; readonly pinned: number };

export type TemplateHit = { readonly entry: TemplateEntry; readonly reason: Reason; readonly rank: number };
export type SignatureHit = { readonly format: SigFormat; readonly reason: Reason; readonly rank: number };
export type Results = { readonly templates: TemplateHit[]; readonly signatures: SignatureHit[] };

/**
 * Read a query. `"text"` is text and nothing else; `0x` or spaces between
 * hex digits make it hex and nothing else. A leading `*` looks for the bytes
 * at any offset inside a signature, a trailing `$` in one measured from the
 * end of the file. Anything else is tried as a name, an extension, hex and a
 * C string all at once.
 */
export function parseQuery(raw: string): Query {
  let t = raw.trim();
  let where: Where = "start";
  if (t.startsWith("*")) {
    where = "anywhere";
    t = t.slice(1).trim();
  } else if (t.endsWith("$") && !t.endsWith("\\$")) {
    where = "end";
    t = t.slice(0, -1).trim();
  }
  const placed = where !== "start";
  const quoted = /^"(.*)"$/s.exec(t) ?? /^'(.*)'$/s.exec(t);
  if (quoted !== null) {
    const b = parseCString(quoted[1] ?? "");
    return { word: "", ext: null, bytes: b === null ? [] : [{ bytes: b, where }] };
  }
  const hex = parseHex(t);
  if (hex !== null && (/0x/i.test(t) || /[0-9a-f]\s+[0-9a-f]/i.test(t))) return { word: "", ext: null, bytes: [{ bytes: hex, where }] };
  const bytes: ByteQuery[] = [];
  if (hex !== null) bytes.push({ bytes: hex, where });
  const text = parseCString(t);
  if (text !== null && (hex === null || !same(text, hex))) bytes.push({ bytes: text, where });
  // A `*` or `$` is about bytes, so the query is not also a name.
  if (placed) return { word: "", ext: null, bytes };
  const bare = t.replace(/^\./, "").toLowerCase();
  return { word: t.startsWith(".") ? "" : t.toLowerCase(), ext: /^[a-z0-9_+-]+$/.test(bare) ? bare : null, bytes };
}

const same = (a: Uint8Array, b: Uint8Array): boolean => a.length === b.length && a.every((x, i) => x === b[i]);

/** Extensions shared by so many unrelated formats that one in common says
 *  nothing about whether a signature and a template are the same format. */
const LOOSE_EXT: ReadonlySet<string> = new Set(["bin", "dat", "db", "o", "obj", "img", "json", "idx", "hdr", "env", "p", "txt", "xml", "raw", "data", "stream", "res"]);

/** Where a template compiles its signatures. Compiled once per entry. */
const compiled = new WeakMap<TemplateEntry, { tokens: Token[]; pattern: string; offset: number }[]>();
function tokensOf(entry: TemplateEntry): { tokens: Token[]; pattern: string; offset: number }[] {
  let c = compiled.get(entry);
  if (c === undefined) {
    c = [];
    for (const s of entry.sigs) {
      try {
        c.push({ tokens: compile(s.pattern), pattern: s.pattern, offset: s.offset });
      } catch {
        // A pattern that does not compile is not searched; nothing else to do.
      }
    }
    compiled.set(entry, c);
  }
  return c;
}

/** Rank: a name that starts with the query, then an exact extension, then a
 *  name that holds it, then bytes. Lower is better. */
function byName(label: string, extraNames: readonly string[], q: Query, ext: readonly string[]): { reason: Reason; rank: number } | null {
  if (q.word !== "") {
    const l = label.toLowerCase();
    if (l.startsWith(q.word)) return { reason: { by: "name", at: 0, length: q.word.length }, rank: 0 };
  }
  if (q.ext !== null && ext.includes(q.ext)) return { reason: { by: "ext", ext: q.ext }, rank: 1 };
  if (q.word !== "") {
    const at = label.toLowerCase().indexOf(q.word);
    if (at > 0) return { reason: { by: "name", at, length: q.word.length }, rank: 2 };
    if (extraNames.some((n) => n.toLowerCase().includes(q.word))) return { reason: { by: "name", at: -1, length: 0 }, rank: 2 };
  }
  return null;
}

function bestBytes(
  candidates: Iterable<{ tokens: readonly Token[]; pattern: string; offset: number; fromEnd: boolean }>,
  q: Query,
): Reason | null {
  let best: Reason | null = null;
  for (const c of candidates) {
    for (const b of q.bytes) {
      const pinned = signatureHolds(c.tokens, c.offset, c.fromEnd, b.bytes, b.where);
      if (pinned > 0 && (best === null || (best.by === "bytes" && pinned > best.pinned))) {
        best = { by: "bytes", pattern: c.pattern, offset: c.offset, fromEnd: c.fromEnd, pinned };
      }
    }
  }
  return best;
}

const formatExt = (f: SigFormat): string[] => [...(f.ext ?? []), ...(f.wpExt ?? [])];

/** Whether a signature entry is the same format as a template, so listing it
 *  again under signatures would say PNG twice. An extension in common and the
 *  template's name in the entry's label or media type. */
function twin(f: SigFormat, entry: TemplateEntry): boolean {
  const shared = formatExt(f).filter((e) => !LOOSE_EXT.has(e) && entry.ext.includes(e));
  if (shared.length === 0) return false;
  const said = `${f.label} ${(f.mime ?? []).join(" ")}`.toLowerCase();
  const names = [entry.value.replace(/^ksy:/, ""), ...entry.ext, ...entry.label.toLowerCase().split(/[^a-z0-9]+/)].filter((w) => w.length >= 3);
  return names.some((w) => new RegExp(`\\b${w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`).test(said));
}

/**
 * Every template and signature that answers the query, best first. A
 * signature that matches bytes lends its match to the templates that share
 * its extension, which is how `89 50 4E 47` reaches the PNG template, whose
 * own recogniser is code rather than a pattern.
 */
export function searchTemplates(raw: string, entries: readonly TemplateEntry[], signatures: readonly Compiled[]): Results {
  const q = parseQuery(raw);
  const templates = new Map<TemplateEntry, TemplateHit>();
  const offer = (entry: TemplateEntry, reason: Reason, rank: number): void => {
    const had = templates.get(entry);
    if (had === undefined || rank < had.rank || (rank === had.rank && reason.by === "bytes" && had.reason.by === "bytes" && reason.pinned > had.reason.pinned)) {
      templates.set(entry, { entry, reason, rank });
    }
  };
  if (q.word === "" && q.ext === null && q.bytes.length === 0) return { templates: [], signatures: [] };

  for (const entry of entries) {
    const named = byName(entry.label, [entry.value], q, entry.ext);
    if (named !== null) offer(entry, named.reason, named.rank);
    if (q.bytes.length > 0) {
      const r = bestBytes(
        tokensOf(entry).map((c) => ({ ...c, fromEnd: false })),
        q,
      );
      if (r !== null) offer(entry, r, 3);
    }
  }

  // One row per format, the pattern of its that pins the most query bytes.
  const sigHits = new Map<SigFormat, SignatureHit>();
  for (const c of signatures) {
    const f = c.format;
    if (!sigHits.has(f)) {
      const named = byName(f.label, [], q, formatExt(f));
      if (named !== null) sigHits.set(f, { format: f, reason: named.reason, rank: named.rank });
    }
    if (q.bytes.length === 0) continue;
    const had = sigHits.get(f);
    if (had !== undefined && had.reason.by !== "bytes") continue;
    const r = bestBytes([{ tokens: c.tokens, pattern: c.sig[0], offset: c.sig[1], fromEnd: c.sig[2] === "eof" }], q);
    if (r !== null && r.by === "bytes" && (had === undefined || (had.reason.by === "bytes" && r.pinned > had.reason.pinned))) {
      sigHits.set(f, { format: f, reason: r, rank: 3 });
    }
  }

  // Lend each byte match to the built-ins that share the format's extension.
  const builtins = entries.filter((e) => e.kind === "builtin");
  for (const hit of sigHits.values()) {
    if (hit.reason.by !== "bytes") continue;
    const exts = formatExt(hit.format).filter((e) => !LOOSE_EXT.has(e));
    for (const entry of builtins) if (exts.some((e) => entry.ext.includes(e))) offer(entry, hit.reason, 3);
  }

  const shown = [...templates.values()].map((h) => h.entry);
  const signaturesLeft = [...sigHits.values()].filter((h) => !shown.some((e) => twin(h.format, e)));
  const order = <T extends { rank: number; reason: Reason }>(label: (h: T) => string) => (a: T, b: T): number =>
    a.rank - b.rank ||
    (a.reason.by === "bytes" && b.reason.by === "bytes" ? b.reason.pinned - a.reason.pinned : 0) ||
    label(a).localeCompare(label(b));
  return {
    templates: [...templates.values()].sort(order<TemplateHit>((h) => h.entry.label)),
    signatures: signaturesLeft.sort(order<SignatureHit>((h) => h.format.label)),
  };
}

/** A PRONOM pattern as a reader would write the bytes: spaced pairs, `??`
 *  for any byte, and the rest cut short past `most` bytes. */
export function spacedPattern(pattern: string, most = 8): string {
  const out: string[] = [];
  for (let i = 0; i < pattern.length; ) {
    const part = /^([0-9A-F]{2}|\?\?|\*|\{[^}]*\}|\[[^\]]*\]|\([^)]*\))/i.exec(pattern.slice(i));
    if (part === null) break;
    out.push(part[0] === "*" ? "…" : part[0].toUpperCase());
    i += part[0].length;
  }
  return out.length > most ? `${out.slice(0, most).join(" ")} …` : out.join(" ");
}

/** Kaitai's pinned bytes, as offset and hex, as one pattern from the first
 *  of them with the gaps between written in. */
export function magicPattern(magic: readonly (readonly [number, string])[]): { pattern: string; offset: number } | null {
  const sorted = [...magic].sort((a, b) => a[0] - b[0]);
  const first = sorted[0];
  if (first === undefined) return null;
  let pattern = "";
  let at = first[0];
  for (const [offset, hex] of sorted) {
    if (offset < at) return null;
    if (offset > at) pattern += `{${offset - at}}`;
    pattern += hex.toUpperCase();
    at = offset + hex.length / 2;
  }
  return { pattern, offset: first[0] };
}

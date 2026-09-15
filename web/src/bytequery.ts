// The bytes a template search asks about, and whether a signature pattern
// could be the start of a file holding them.
//
// A search is typed by someone who remembers part of a signature: `89 50 4E
// 47`, `\x89PNG`, `PK\x03\x04`. So the question put to each pattern is not
// "does this match" but "could a file matching this pattern hold these bytes
// there", which lets the query be shorter or longer than the pattern.

import type { Token } from "./signatures.ts";

/** Where in a file the searched bytes sit. */
export type Where = "start" | "anywhere" | "end";

/** `89 50 4E 47`, `89504e47` or `0x89 0x50`: whole bytes of hex and nothing
 *  else. Null for anything that is not. */
export function parseHex(text: string): Uint8Array | null {
  const t = text.trim().replace(/0x/gi, "");
  if (!/^[0-9a-f\s,]+$/i.test(t)) return null;
  const digits = t.replace(/[\s,]/g, "");
  if (digits.length === 0 || digits.length % 2 !== 0) return null;
  return Uint8Array.from(digits.match(/../g) ?? [], (x) => parseInt(x, 16));
}

const SIMPLE: Record<string, number> = { n: 10, r: 13, t: 9, "0": 0, a: 7, b: 8, f: 12, v: 11, e: 27, "\\": 92, '"': 34, "'": 39, "?": 63 };

/**
 * Text with C escapes, as bytes: `\x89PNG`, `PK\3\4`, `\r\n`. A character
 * typed as itself is its UTF-8 bytes. Null when an escape is broken, so that
 * `\x` alone is not quietly read as an `x`.
 */
export function parseCString(text: string): Uint8Array | null {
  const out: number[] = [];
  const enc = new TextEncoder();
  for (let i = 0; i < text.length; ) {
    const c = text[i] ?? "";
    if (c !== "\\") {
      const cp = text.codePointAt(i) ?? 0;
      const ch = String.fromCodePoint(cp);
      out.push(...enc.encode(ch));
      i += ch.length;
      continue;
    }
    const d = text[i + 1];
    if (d === undefined) return null;
    if (d === "x" || d === "X") {
      const m = /^[0-9a-f]{1,2}/i.exec(text.slice(i + 2));
      if (m === null) return null;
      out.push(parseInt(m[0], 16));
      i += 2 + m[0].length;
    } else if (/[0-7]/.test(d)) {
      const m = /^[0-7]{1,3}/.exec(text.slice(i + 1)) ?? ["0"];
      const v = parseInt(m[0], 8);
      if (v > 255) return null;
      out.push(v);
      i += 1 + m[0].length;
    } else if (SIMPLE[d] !== undefined) {
      out.push(SIMPLE[d]);
      i += 2;
    } else return null;
  }
  return out.length === 0 ? null : Uint8Array.from(out);
}

/**
 * How many of the query's bytes a pattern pins, when a file matching the
 * pattern could hold the query at `qAt`; 0 when it could not, or when it
 * could only because the pattern pins none of them.
 *
 * The pattern starts at file offset `pAt`. Positions outside the query are
 * not checked, and a pattern that ends, or reaches a gap, before the query
 * does leaves the rest of the query free, unless `inside` asks for every
 * query byte to fall within the pattern.
 */
export function overlap(tokens: readonly Token[], pAt: number, query: Uint8Array, qAt: number, inside: boolean): number {
  const qEnd = qAt + query.length;
  let steps = 0;
  const check = (pos: number, ok: (b: number) => boolean): boolean | null => {
    if (pos < qAt || pos >= qEnd) return null;
    return ok(query[pos - qAt] ?? -1);
  };
  const run = (ts: readonly Token[], ti: number, pos: number, pinned: number, then: (pos: number, pinned: number) => number): number => {
    if (++steps > 20_000) return 0;
    if (pos >= qEnd) return pinned;
    if (ti === ts.length) return then(pos, pinned);
    const t = ts[ti];
    if (t === undefined) return 0;
    switch (t.k) {
      case "lit": {
        let n = pinned;
        for (let j = 0; j < t.b.length; j++) {
          const r = check(pos + j, (b) => b === t.b[j]);
          if (r === false) return 0;
          if (r === true) n++;
        }
        return run(ts, ti + 1, pos + t.b.length, n, then);
      }
      case "range": {
        if (check(pos, (b) => (b >= t.lo && b <= t.hi) !== t.neg) === false) return 0;
        return run(ts, ti + 1, pos + 1, pinned, then);
      }
      case "gap": {
        // A gap that can reach past the query settles it: everything after
        // is free to be whatever the query says.
        if (t.max === Infinity || pos + t.max >= qEnd) {
          if (!inside || pos + t.min >= qEnd) return pinned;
        }
        const most = Math.min(t.max, qEnd - pos);
        for (let g = t.min; g <= most; g++) {
          const n = run(ts, ti + 1, pos + g, pinned, then);
          if (n > 0) return n;
        }
        return 0;
      }
      case "alt": {
        for (const opt of t.opts) {
          const n = run(opt, 0, pos, pinned, (p, k) => run(ts, ti + 1, p, k, then));
          if (n > 0) return n;
        }
        return 0;
      }
    }
  };
  return run(tokens, 0, pAt, 0, (pos, pinned) => (inside && pos < qEnd ? 0 : pinned));
}

/** The longest a pattern can reach, capped: where to stop sliding a query
 *  along it. */
function reach(tokens: readonly Token[]): number {
  let n = 0;
  for (const t of tokens) {
    if (t.k === "lit") n += t.b.length;
    else if (t.k === "range") n += 1;
    else if (t.k === "gap") n += t.max === Infinity ? 64 : Math.min(t.max, 64);
    else n += Math.max(...t.opts.map(reach));
  }
  return Math.min(n, 1024);
}

/**
 * How many query bytes a signature pins, searched for `where`; 0 for no
 * match.
 *
 * - `start`: the query is the first bytes of the file, and the signature sits
 *   at its own offset from the start.
 * - `anywhere`: the query is somewhere inside the signature, wherever that
 *   is in the file.
 * - `end`: the query is inside a signature measured from the end of the file.
 */
export function signatureHolds(tokens: readonly Token[], offset: number, fromEnd: boolean, query: Uint8Array, where: Where): number {
  if (query.length === 0) return 0;
  if (where === "start") return fromEnd ? 0 : overlap(tokens, offset, query, 0, false);
  if (where === "end" && !fromEnd) return 0;
  const most = reach(tokens);
  for (let at = 0; at + query.length <= most; at++) {
    const n = overlap(tokens, 0, query, at, true);
    if (n > 0) return n;
  }
  return 0;
}

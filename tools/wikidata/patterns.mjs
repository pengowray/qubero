// Turns the values of Wikidata's "file format identification pattern" (P4152)
// into one notation Qubero can match against bytes.
//
// Nearly all of the values are plain uppercase hex, imported from PRONOM and
// TrID. The rest were typed in by hand, and are written however the editor
// thought of it at the time: hex with spaces or a 0x in front, lowercase hex,
// text, C string escapes, a GUID, a regular expression, or PRONOM's own
// signature syntax. The "encoding" qualifier (P3294) says which, and is wrong
// often enough that the shape of the value gets a say as well.
//
// The notation everything becomes is PRONOM's internal signature syntax, since
// the hex already is a subset of it and it has names for everything else the
// values need: `??` for any byte, `{4}` and `{0-32}` for runs of them, `*` for
// any number, `(0D0A|0A)` for alternatives and `[30:37]` for a byte range.
//
// Each function returns either `{ pattern, note }`, where the note says what
// had to be done to the value (null when nothing was), or `{ skip }` with the
// reason it could not be used.

/** Wikidata items the encoding qualifier points at. */
export const ENC = {
  hex: "Q82828",
  ascii: "Q8815",
  // "ASCII Corporation": a slip for ASCII when picking from the suggestions.
  asciiCorp: "Q297303",
  utf8: "Q193537",
  pronomSignature: "Q35432091",
  // "PRONOM" itself, used for the same thing.
  pronom: "Q7120402",
  guid: "Q254972",
  pcre2: "Q98056596",
  posixEre: "Q50349151",
};

const HEX_PAIRS = /^(?:[0-9A-F]{2})+$/;

/** `4d 5a`, `0x4D5A`, `0x4D 0x5A`: hex with the decoration taken off, or null. */
function looseHex(value) {
  const bare = value.replace(/0x/gi, "").replace(/[\s,]/g, "").toUpperCase();
  return /^[0-9A-F]+$/.test(bare) ? bare : null;
}

/** True when text tagged as ASCII is plainly hex written out instead. The
 *  only safe tells are pairs separated by spaces, or a run long enough that
 *  no real text would be nothing but hex digits: `8BCB` is text, and so is
 *  `AC1012`. */
function textIsReallyHex(value) {
  if (/^[0-9A-Fa-f]{2}(?: [0-9A-Fa-f]{2})+$/.test(value)) return true;
  return /^(?:[0-9A-Fa-f]{2}){8,}$/.test(value) && /[0-9]/.test(value);
}

/** Bytes to canonical hex. */
const toHex = (bytes) => Array.from(bytes, (b) => b.toString(16).toUpperCase().padStart(2, "0")).join("");

/**
 * Text to bytes, reading C escapes (`\r`, `\x1A`, `\211`) when there are any.
 * A character past U+007F that did not come from an escape is written as
 * UTF-8, since that is what a file holding the text would contain.
 */
function textBytes(value) {
  const out = [];
  let escaped = false;
  for (let i = 0; i < value.length; i++) {
    const c = value[i];
    if (c !== "\\" || i + 1 === value.length) {
      out.push(...new TextEncoder().encode(c));
      continue;
    }
    const n = value[i + 1];
    const simple = { n: 0x0a, r: 0x0d, t: 0x09, "\\": 0x5c, '"': 0x22, "'": 0x27, a: 0x07, b: 0x08, f: 0x0c, v: 0x0b, e: 0x1b };
    const oct = /^[0-7]{1,3}/.exec(value.slice(i + 1));
    const hex = /^x([0-9A-Fa-f]{2})/.exec(value.slice(i + 1));
    if (hex) {
      out.push(parseInt(hex[1], 16));
      i += 3;
    } else if (oct) {
      out.push(parseInt(oct[0], 8) & 0xff);
      i += oct[0].length;
    } else if (n in simple) {
      out.push(simple[n]);
      i += 1;
    } else {
      out.push(0x5c);
      continue;
    }
    escaped = true;
  }
  return { bytes: out, escaped };
}

/**
 * A GUID as the 16 bytes a file stores: the first three groups little-endian,
 * the last two as written. That is how Windows writes a CLSID, which is what
 * the one GUID-encoded value (a shell link's) is.
 */
function guidBytes(value) {
  const m = /^\{?([0-9a-f]{8})-([0-9a-f]{4})-([0-9a-f]{4})-([0-9a-f]{4})-([0-9a-f]{12})\}?$/i.exec(value.trim());
  if (!m) return null;
  const rev = (h) => h.match(/../g).reverse().join("");
  return (rev(m[1]) + rev(m[2]) + rev(m[3]) + m[4] + m[5]).toUpperCase();
}

/**
 * Check a PRONOM signature and write it in the canonical form: uppercase,
 * no spaces. Returns null when it uses something the matcher does not read.
 * The grammar here is the part of DROID's that the 52 PRONOM-encoded values
 * on Wikidata use, plus `[!xx]`, which is too small to leave out.
 */
export function canonicalPronom(value) {
  const s = value.replace(/\s+/g, "").toUpperCase();
  let i = 0;
  const seq = (inAlt) => {
    while (i < s.length) {
      const c = s[i];
      if (/[0-9A-F]/.test(c)) {
        if (!/[0-9A-F]/.test(s[i + 1] ?? "")) return false;
        i += 2;
      } else if (c === "?") {
        if (s[i + 1] !== "?") return false;
        i += 2;
      } else if (c === "*") {
        if (inAlt) return false;
        i += 1;
      } else if (c === "{") {
        const m = /^\{(\d+)(?:-(\d+|\*))?\}/.exec(s.slice(i));
        if (!m || inAlt) return false;
        if (m[2] !== undefined && m[2] !== "*" && Number(m[2]) < Number(m[1])) return false;
        i += m[0].length;
      } else if (c === "[") {
        const m = /^\[(?:!?[0-9A-F]{2}|[0-9A-F]{2}:[0-9A-F]{2})\]/.exec(s.slice(i));
        if (!m) return false;
        i += m[0].length;
      } else if (c === "(") {
        if (inAlt) return false;
        i += 1;
        for (;;) {
          const start = i;
          if (!seq(true) || i === start) return false;
          if (s[i] === "|") i += 1;
          else if (s[i] === ")") {
            i += 1;
            break;
          } else return false;
        }
      } else if (inAlt && (c === "|" || c === ")")) {
        return true;
      } else return false;
    }
    return !inAlt;
  };
  return seq(false) && s.length > 0 ? s : null;
}

/**
 * The few regular expressions are all of one shape: text, `.` for any byte,
 * `.{4}` for several, and `\x20` for a byte by number. That much becomes a
 * signature; anything with anchors, classes, repeats or groups is skipped.
 */
function pcreToPronom(value) {
  let out = "";
  for (let i = 0; i < value.length; i++) {
    const c = value[i];
    if (c === "\\") {
      const hex = /^x([0-9A-Fa-f]{2})/.exec(value.slice(i + 1));
      if (hex) {
        out += hex[1].toUpperCase();
        i += 3;
        continue;
      }
      const lit = value[i + 1];
      if (lit !== undefined && /[\\.^$|?*+()[\]{}\/-]/.test(lit)) {
        out += toHex(new TextEncoder().encode(lit));
        i += 1;
        continue;
      }
      return null;
    }
    if (c === ".") {
      const rep = /^\{(\d+)\}/.exec(value.slice(i + 1));
      if (rep) {
        out += `{${rep[1]}}`;
        i += rep[0].length;
      } else out += "??";
      continue;
    }
    if (/[\^$|?*+()[\]{}]/.test(c)) return null;
    out += toHex(new TextEncoder().encode(c));
  }
  return out === "" ? null : out;
}

/** The bytes a text value stands for, or a skip. Shared by every text encoding. */
function fromText(value, what) {
  if (textIsReallyHex(value)) {
    const hex = looseHex(value);
    if (hex !== null && hex.length % 2 === 0) return { pattern: hex, note: `tagged as ${what} but written as hex` };
  }
  // `L.......` is a shell link's first byte followed by seven the editor did
  // not want to spell out. Dots can also be text (`AC1.2`), so only a run of
  // them is taken to be this, and it is skipped rather than guessed at.
  if (/\.{3,}/.test(value)) return { skip: "text with a run of dots, which may mean any byte" };
  const { bytes, escaped } = textBytes(value);
  if (bytes.length === 0) return { skip: "empty" };
  return { pattern: toHex(bytes), note: escaped ? "text with C escapes" : null };
}

/**
 * One statement's value, with the encodings its qualifiers name, as a
 * pattern or a skip. `encodings` is a list because a few statements carry two.
 */
export function cleanPattern(value, encodings) {
  if (value.includes("/.well-known/genid/")) return { skip: "unknown value" };
  const enc = new Set(encodings);
  if (enc.has(ENC.posixEre)) return { skip: "a file name pattern, not content" };
  if (enc.has(ENC.pcre2)) {
    const p = pcreToPronom(value);
    return p === null ? { skip: "regular expression beyond literal text and any-byte runs" } : { pattern: p, note: "regular expression" };
  }
  if (enc.has(ENC.guid)) {
    const hex = guidBytes(value);
    if (hex !== null) return { pattern: hex, note: "GUID, stored little-endian" };
  }
  if (enc.has(ENC.pronomSignature) || enc.has(ENC.pronom)) {
    const p = canonicalPronom(value);
    if (p === null) return { skip: "PRONOM signature syntax the matcher does not read" };
    return { pattern: p, note: HEX_PAIRS.test(p) ? null : "PRONOM signature" };
  }
  if (enc.has(ENC.hex)) {
    const hex = looseHex(value);
    if (hex === null) return { skip: "tagged as hex but not hex" };
    if (hex.length % 2 !== 0) return { skip: "odd number of hex digits" };
    return { pattern: hex, note: hex === value ? null : "hex with spaces, 0x or lowercase" };
  }
  if (enc.has(ENC.ascii) || enc.has(ENC.asciiCorp)) return fromText(value, "ASCII");
  if (enc.has(ENC.utf8)) return fromText(value, "UTF-8");
  if (enc.size > 0) return { skip: `unknown encoding ${[...enc].join(", ")}` };
  // No encoding given: hex if it can only be hex, text if it has C escapes,
  // otherwise too ambiguous to use.
  const hex = looseHex(value);
  if (hex !== null && hex.length % 2 === 0 && hex.length >= 4) return { pattern: hex, note: "no encoding given, read as hex" };
  if (/\\(?:[0-7]{1,3}|x[0-9A-Fa-f]{2}|[nrt])/.test(value)) {
    const r = fromText(value, "text");
    return "skip" in r ? r : { pattern: r.pattern, note: "no encoding given, read as text with C escapes" };
  }
  return { skip: "no encoding given and not clearly hex" };
}

/**
 * How many bytes of a pattern are pinned to a value: the measure of how much
 * a match says. Alternatives count their shortest option, and ranges and
 * wildcards count nothing.
 */
export function fixedBytes(pattern) {
  let n = 0;
  let depth = 0;
  let alt = [];
  for (let i = 0; i < pattern.length; ) {
    const c = pattern[i];
    if (c === "(") {
      depth = 1;
      alt = [0];
      i += 1;
    } else if (c === "|") {
      alt.push(0);
      i += 1;
    } else if (c === ")") {
      n += Math.min(...alt);
      depth = 0;
      i += 1;
    } else if (c === "{") i = pattern.indexOf("}", i) + 1;
    else if (c === "[") i = pattern.indexOf("]", i) + 1;
    else if (c === "*") i += 1;
    else if (c === "?") i += 2;
    else {
      if (depth) alt[alt.length - 1] += 1;
      else n += 1;
      i += 2;
    }
  }
  return n;
}

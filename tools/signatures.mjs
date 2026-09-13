// Builds web/public/signatures.json, the one database of file signatures the
// page matches an unknown file against.
//
//   node tools/signatures.mjs
//
// Two sources go in:
//
//   "wikidata"  tools/wikidata/formats.json, which build.mjs writes from the
//               "file format identification pattern" property (P4152). Around
//               9,400 patterns on as many items, mostly imported from TrID and
//               PRONOM, each with the item's label, extensions and media types.
//   "file"      The top-level rules of the file(1) magic files, from the
//               magic-db crate. The compiled database in the wasm module reads
//               these same rules, but only answers with the one strongest
//               match; here every rule that matches is a row, so a file the
//               strongest rule called a ZIP still shows the rules under it.
//
// Only the rules that pin bytes at a fixed place are taken from file(1): the
// ones the matcher in web/src/signatures.ts can run without a magic engine.
// Everything skipped is counted by reason and printed at the end.
//
// The two sources overlap heavily, and by design: two independent lists saying
// the same four bytes mean PNG is worth more than one saying it twice. What is
// shared is the signature itself, stored once per (pattern, offset, fromEnd)
// with the formats that claim it.

import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { magicDb } from "./magicdb.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const WIKIDATA = join(ROOT, "tools", "wikidata", "formats.json");
const OUT = join(ROOT, "web", "public", "signatures.json");

// ---------------------------------------------------------------------------
// file(1) rules

/** The types whose value is a run of bytes at the rule's offset, and how wide
 *  each numeric one is. Plain short/long/quad are the machine's own byte order,
 *  which for everything the page runs on is little-endian. */
const NUMERIC = {
  byte: [1, "le"], short: [2, "le"], long: [4, "le"], quad: [8, "le"],
  beshort: [2, "be"], belong: [4, "be"], bequad: [8, "be"],
  leshort: [2, "le"], lelong: [4, "le"], lequad: [8, "le"],
};
// `ubyte` and the rest differ from the signed form only in how the rule
// compares, and an equality test compares the same bytes either way.
for (const [name, spec] of Object.entries({ ...NUMERIC })) NUMERIC[`u${name}`] = spec;

const ESCAPES = { n: 0x0a, r: 0x0d, t: 0x09, a: 0x07, b: 0x08, f: 0x0c, v: 0x0b, e: 0x1b, "\\": 0x5c, " ": 0x20 };

/**
 * The bytes a file(1) string value stands for, reading its escapes: octal
 * `\0` to `\377`, `\xHH`, the single-letter ones, and `\ ` for a space.
 *
 * A trailing `\0` is one of the bytes. Dropping it once made every Parquet
 * file a "PARity archive", because the PAR rule's value is `PAR\0` and a
 * Parquet file starts `PAR1`.
 */
function stringBytes(value) {
  const out = [];
  for (let i = 0; i < value.length; i++) {
    const c = value[i];
    if (c !== "\\") {
      const code = c.codePointAt(0);
      if (code > 0xff) return null; // Not a byte the rule could have meant.
      out.push(code);
      continue;
    }
    const rest = value.slice(i + 1);
    const hex = /^x([0-9A-Fa-f]{1,2})/.exec(rest);
    const oct = /^[0-7]{1,3}/.exec(rest);
    if (hex) {
      out.push(parseInt(hex[1], 16));
      i += hex[0].length;
    } else if (oct) {
      out.push(parseInt(oct[0], 8) & 0xff);
      i += oct[0].length;
    } else if (rest[0] !== undefined && rest[0] in ESCAPES) {
      out.push(ESCAPES[rest[0]]);
      i += 1;
    } else if (rest[0] !== undefined) {
      // An escape file(1) does not define stands for the character itself.
      const code = rest.codePointAt(0);
      if (code > 0xff) return null;
      out.push(code);
      i += 1;
    } else return null; // A lone backslash at the end.
  }
  return out.length === 0 ? null : out;
}

/** A file(1) numeric value as the bytes of `width` in `order`, or null. C
 *  number syntax: a leading 0 is octal, 0x is hex, and a trailing L or U is
 *  the width suffix the rule writer added and the parser stops at. */
function numericBytes(value, width, order) {
  const m = /^(-?)(0[xX][0-9a-fA-F]+|0[0-7]*|[1-9][0-9]*)[LlUu]*$/.exec(value);
  if (m === null) return null;
  let n = BigInt(/^0[0-7]+$/.test(m[2]) ? `0o${m[2].slice(1)}` : m[2]);
  if (m[1] === "-") n = -n;
  const bits = BigInt(width) * 8n;
  n &= (1n << bits) - 1n; // Two's complement, so a negative is its bytes.
  const out = [];
  for (let i = 0; i < width; i++) out.push(Number((n >> BigInt(i * 8)) & 0xffn));
  return order === "be" ? out.reverse() : out;
}

/** file(1)'s own measure of how much a rule is worth, from apprentice.c:
 *  two bytes' worth to start with, and ten a byte for what it pins down. */
const MULT = 10;
const baseStrength = (bytes) => 2 * MULT + bytes * MULT;

/** Apply a `!:strength` line: `+N`, `-N`, `*N`, `/N`. */
function applyStrength(base, spec) {
  const m = /^([-+*/])\s*(\d+)$/.exec(spec.trim());
  if (m === null) return base;
  const n = Number(m[2]);
  const v = m[1] === "+" ? base + n : m[1] === "-" ? base - n : m[1] === "*" ? base * n : n === 0 ? base : Math.floor(base / n);
  return Math.max(0, v);
}

const toHex = (bytes) => bytes.map((b) => b.toString(16).toUpperCase().padStart(2, "0")).join("");

/**
 * The message a rule prints, as much of it as is true of every file the rule
 * matches. A `%` starts a placeholder the file's own values fill in, so the
 * text is cut there; what is left is trimmed of a trailing comma, which is how
 * a rule that leads into its children ends. `unfinished` says the full rule
 * had more to say: a cut placeholder, a trailing comma, or child lines.
 */
function ruleMessage(raw, hasChildren) {
  const cut = raw.indexOf("%");
  let text = (cut < 0 ? raw : raw.slice(0, cut)).trim();
  const comma = text.endsWith(",");
  if (comma) text = text.slice(0, -1).trim();
  return { message: text, unfinished: cut >= 0 || comma || hasChildren };
}

/** Every usable top-level rule in one magic file, and why the rest were left out. */
function readMagicFile(name, text, count) {
  const lines = text.split(/\r?\n/);
  const rules = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!/^[0-9]/.test(line)) continue;
    count.read += 1;
    const head = /^(\S+)[ \t]+(\S+)[ \t]*(.*)$/.exec(line);
    if (head === null) {
      count.skip("the line is an offset and nothing else", `${name}:${i + 1}`);
      continue;
    }
    const [, offsetText, type, rest] = head;
    if (!/^(?:0[xX][0-9a-fA-F]+|[0-9]+)$/.test(offsetText)) {
      count.skip("offset is worked out from the file rather than fixed", `${name}:${i + 1}`);
      continue;
    }
    // C number syntax here too, so a leading 0 is octal: `0774` is byte 508.
    const offset = Number(/^0[0-7]+$/.test(offsetText) ? `0o${offsetText.slice(1)}` : offsetText);
    const spec = NUMERIC[type];
    const isString = type === "string" || type === "string/b";
    if (!isString && spec === undefined) {
      count.skip(`type the matcher cannot run: ${type.replace(/&.*/, "&…")}`, `${name}:${i + 1}`);
      continue;
    }
    // The value runs to the first unescaped space; `\ ` and `\040` are spaces
    // inside it. What follows is the message.
    const split = /^((?:\\.|\S)*)[ \t]*(.*)$/.exec(rest);
    const value = split[1];
    if (value === "") {
      count.skip("no value to match against", `${name}:${i + 1}`);
      continue;
    }
    // `x` is file(1)'s match-anything test only when it stands alone, so a
    // string value of `xar!` is four literal bytes.
    if (value === "x" || /^[<>=!&^~]/.test(value)) {
      count.skip(`value is a comparison, not a constant: ${value === "x" ? "x" : value[0]}`, `${name}:${i + 1}`);
      continue;
    }
    const raw = split[2];
    if (raw.startsWith("\\b")) {
      count.skip("the message continues the line before it", `${name}:${i + 1}`);
      continue;
    }
    const bytes = isString ? stringBytes(value) : numericBytes(value, spec[0], spec[1]);
    if (bytes === null) {
      count.skip(isString ? "value has an escape that is not a byte" : "value is not a number", `${name}:${i + 1}`);
      continue;
    }
    // The `!:` lines belong to the magic line just above them, so stop at the
    // first child: its mime is not this rule's.
    let mime = "";
    let ext = [];
    let strength = null;
    let hasChildren = false;
    for (let j = i + 1; j < lines.length; j++) {
      const next = lines[j];
      if (next.startsWith("#") || next.trim() === "") continue;
      if (next.startsWith("!:")) {
        const m = /^!:(\w+)[ \t]+(.*)$/.exec(next);
        if (m === null) continue;
        if (m[1] === "mime") mime = m[2].trim();
        else if (m[1] === "ext") ext = m[2].trim().split("/").filter((e) => e !== "");
        else if (m[1] === "strength") strength = m[2];
        continue;
      }
      hasChildren = next.startsWith(">");
      break;
    }
    const { message, unfinished } = ruleMessage(raw, hasChildren);
    if (message === "") {
      count.skip("the rule prints nothing of its own", `${name}:${i + 1}`);
      continue;
    }
    rules.push({
      id: `${name}:${i + 1}`,
      label: message,
      pattern: toHex(bytes),
      offset,
      ext,
      mime,
      unfinished,
      strength: applyStrength(baseStrength(bytes.length), strength ?? ""),
    });
  }
  return rules;
}

// ---------------------------------------------------------------------------
// Reading both sources

const count = {
  read: 0,
  skipped: new Map(),
  skip(reason, where) {
    let list = this.skipped.get(reason);
    if (list === undefined) this.skipped.set(reason, (list = []));
    list.push(where);
  },
};

const { dir: magdir, version: magicVersion } = magicDb();
const fileRules = [];
for (const name of readdirSync(magdir).sort()) {
  const path = join(magdir, name);
  if (!statSync(path).isFile()) continue;
  fileRules.push(...readMagicFile(name, readFileSync(path, "utf8"), count));
}

const wikidata = JSON.parse(readFileSync(WIKIDATA, "utf8"));

/** One row of the formats table, and the signatures it claims. */
const formats = [];
const sigs = new Map();
const addSig = (pattern, offset, fromEnd, index) => {
  const key = `${pattern} ${offset} ${fromEnd ? "e" : ""}`;
  let entry = sigs.get(key);
  if (entry === undefined) sigs.set(key, (entry = { pattern, offset, fromEnd, formats: [] }));
  entry.formats.push(index);
};

// Wikidata first, then file(1), so that the columns only one source fills are
// one block of rows rather than a value a row.
for (const f of wikidata.formats) {
  const index = formats.length;
  formats.push({ id: f.id, label: f.label, ext: f.ext ?? [], wpExt: f.wpExt ?? [], mime: f.mime ?? [], wp: f.wp ?? "", parent: f.parent ?? null });
  for (const [pattern, offset, from] of f.sigs) addSig(pattern, offset, from === "eof", index);
}
const fileFrom = formats.length;
for (const r of fileRules) {
  const index = formats.length;
  formats.push({ id: r.id, label: r.label, ext: r.ext, wpExt: [], mime: r.mime === "" ? [] : [r.mime], wp: "", parent: null, unfinished: r.unfinished, strength: r.strength });
  addSig(r.pattern, r.offset, false, index);
}

// ---------------------------------------------------------------------------
// Writing it out
//
// Columnar, because a row of its own per format spends more on repeated keys
// than on the data: `"label":` nine thousand times is 75 KiB of nothing. Each
// column is one array as long as the formats table, or, where nearly every
// entry would be empty, a pair of arrays holding only the rows that have a
// value. The formats table is the Wikidata rows and then the file(1) rows, so
// a column only one source fills is only as long as that block. The `about`
// field says which is which, so the file can be read without this script.

/** A column only a few rows fill: the row numbers, then their values. */
const sparse = (rows, pick) => {
  const at = [];
  const values = [];
  for (let i = 0; i < rows.length; i++) {
    const v = pick(rows[i]);
    if (v !== null && v !== "" && !(Array.isArray(v) && v.length === 0)) {
      at.push(i);
      values.push(v);
    }
  }
  return [at, values];
};

/** A column whose values repeat: the distinct ones, then one index a row. */
const dictionary = (rows, pick) => {
  const seen = new Map();
  const words = [];
  const at = [];
  for (const row of rows) {
    const v = pick(row);
    if (v === "") {
      at.push(-1);
      continue;
    }
    let k = seen.get(v);
    if (k === undefined) {
      seen.set(v, (k = words.length));
      words.push(v);
    }
    at.push(k);
  }
  return [words, at];
};

const entries = [...sigs.values()];
const fileRows = formats.slice(fileFrom);
const [wpAt, wpValues] = sparse(formats, (f) => f.wp);
const [parentAt, parentValues] = sparse(formats, (f) => f.parent);
const [wpExtAt, wpExtValues] = sparse(formats, (f) => f.wpExt.join(" "));
const [mimeWords, mimeAt] = dictionary(formats, (f) => f.mime.join(" "));
const [sigOffsetAt, sigOffsetValues] = sparse(entries, (e) => e.offset || "");
const [sigEofAt] = sparse(entries, (e) => (e.fromEnd ? "e" : ""));

const data = {
  about:
    "File signatures from two sources, merged by tools/signatures.mjs, stored as columns rather than rows. " +
    "The formats table is the Wikidata rows first and then the file(1) rows, and `fileFrom` is the row the second block starts at. " +
    "Arrays under `formats` run one entry a format unless said otherwise; the `sig*` arrays run one entry a signature. " +
    "A pair named `xAt`/`xValues` is a column only some rows fill: the row numbers, then their values in the same order. " +
    "formats.qid: the Wikidata Q number without the Q, one a Wikidata row. " +
    "formats.ruleId: the magic file and line the rule is on, one a file(1) row. " +
    "formats.label: the item's name, or the sentence the rule prints. " +
    "formats.ext: extensions, space separated, empty for none. " +
    "formats.mimeAt indexes formats.mimeWords, whose entries are media types, space separated; -1 for none. " +
    "formats.wpAt/wpValues: the rows with an English Wikipedia article, and its title. " +
    "formats.parentAt/parentValues: the rows with no article of their own, and the [id, label, title] of the format they are a version or part of. " +
    "formats.wpExtAt/wpExtValues: the rows with extensions only the Wikipedia infobox gave. " +
    "formats.unfinishedAt: the rows whose rule has more to say once it has read the file's own values. " +
    "formats.strength: file(1)'s own measure of how much a rule is worth, one a file(1) row. " +
    "sigPattern: the pattern in PRONOM signature syntax, uppercase hex with ?? { } ( | ) and [ : ] for the rest. " +
    "sigOffsetAt/sigOffsetValues: the signatures not measured from byte 0, and where they are measured from. " +
    "sigEofAt: the signatures measured back from the end of the file rather than forward from the start. " +
    "sigFormats: for each signature, the rows of the formats table that claim it.",
  wikidata: wikidata.fetched,
  magicDb: magicVersion,
  sources: [wikidata.source, `file(1) magic rules, from the magic-db crate ${magicVersion}, BSD 2-clause`],
  fileFrom,
  formats: {
    qid: formats.slice(0, fileFrom).map((f) => Number(f.id.slice(1))),
    ruleId: fileRows.map((f) => f.id),
    label: formats.map((f) => f.label),
    ext: formats.map((f) => f.ext.join(" ")),
    mimeWords,
    mimeAt,
    wpAt,
    wpValues,
    parentAt,
    parentValues: parentValues.map((p) => [p.id, p.label, p.wp]),
    wpExtAt,
    wpExtValues,
    unfinishedAt: sparse(formats, (f) => (f.unfinished ? "u" : ""))[0],
    strength: fileRows.map((f) => f.strength),
  },
  sigPattern: entries.map((e) => e.pattern),
  sigOffsetAt,
  sigOffsetValues,
  sigEofAt,
  sigFormats: entries.map((e) => e.formats),
};

// One array to a line, so a refresh reads as a diff of the columns that moved.
const json = JSON.stringify(data).replace(/,"(\w+)":\[/g, ',\n"$1":[');
writeFileSync(OUT, json);

// ---------------------------------------------------------------------------
// The summary.

const skipped = [...count.skipped].sort((a, b) => b[1].length - a[1].length);
const totalSkipped = skipped.reduce((n, [, list]) => n + list.length, 0);
console.log(`file(1): ${count.read} top-level rules read, ${fileRules.length} kept, ${totalSkipped} left out`);
for (const [reason, list] of skipped) {
  console.log(`  ${String(list.length).padStart(4)}  ${reason}  (${list.slice(0, 2).join(", ")})`);
}
const gz = gzipSync(Buffer.from(json), { level: 9 }).length;
console.log(`wikidata: ${wikidata.formats.length} formats, fetched ${wikidata.fetched}`);
console.log(
  `signatures.json: ${formats.length} formats, ${entries.length} signatures, ` +
    `${(json.length / 1024).toFixed(0)} KiB raw, ${(gz / 1024).toFixed(0)} KiB gzipped`,
);

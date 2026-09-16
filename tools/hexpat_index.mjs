// Rewrite `web/public/hexpat-index.json` from a checkout of ImHex-Patterns.
//
//   node tools/hexpat_index.mjs ~/github/ImHex-Patterns
//
// or set IMHEX_PATTERNS. The result is metadata and nothing else: for each
// pattern its path, what `#pragma description` and `#pragma author` say, what
// `#pragma magic` pins, which files it needs alongside it, and whether the
// converter gets through it without leaving anything behind. No pattern text
// is copied, because the repository is GPL-2.0 and the library browser fetches
// the one a reader picks at the moment they pick it.
//
// The clean/gaps column comes from `cargo run --example hexpat_gaps` over the
// same checkout, so the list says what this build of the converter actually
// does rather than what it did when someone last looked.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "web", "public", "hexpat-index.json");
const CHECKOUT = process.argv[2] ?? process.env.IMHEX_PATTERNS;
if (!CHECKOUT || !existsSync(join(CHECKOUT, "patterns"))) {
  console.error("usage: node tools/hexpat_index.mjs <ImHex-Patterns checkout>   (or set IMHEX_PATTERNS)");
  process.exit(2);
}

/** The include paths the converter answers for out of its own table, read from
 *  the core so the two cannot drift. A pattern asking only for these needs
 *  nothing fetched alongside it. */
const builtinPaths = (() => {
  const rs = readFileSync(join(ROOT, "crates", "core", "src", "hexpat", "includes.rs"), "utf8");
  const block = /pub const BUILTIN_PATHS: &\[&str\] = &\[([\s\S]*?)\];/.exec(rs);
  if (block === null) throw new Error("includes.rs has lost BUILTIN_PATHS");
  return new Set([...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]));
})();

function walk(dir, ext, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, ext, out);
    else if (name.endsWith(ext)) out.push(path);
  }
  return out;
}

const rel = (path) => relative(CHECKOUT, path).split("\\").join("/");

/** `#pragma <key> <the rest of the line>`, the way the lexer reads one. */
function pragma(text, key) {
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*#pragma\s+(\S+)\s*(.*)$/.exec(line);
    if (m !== null && m[1] === key) return m[2].trim();
  }
  return "";
}

/** `#pragma magic [ 4D 5A ] @ 0x00` as `{ at, bytes }`, by the converter's own
 *  rule: bytes up to the first wildcard, and nothing for a magic measured back
 *  from the end of the file. */
function magic(text) {
  const raw = pragma(text, "magic");
  if (raw === "") return null;
  const value = (raw.split("//")[0] ?? "").trim().replace(/;$/, "").trim();
  const at = value.lastIndexOf("@");
  if (at < 0) return null;
  const list = value.slice(0, at).trim();
  const address = value.slice(at + 1).trim();
  if (!list.startsWith("[") || !list.endsWith("]")) return null;
  const inner = list.slice(1, -1).trim();
  const bytes = [];
  if (inner.startsWith('"') && inner.endsWith('"')) {
    for (const ch of inner.slice(1, -1)) bytes.push(ch.charCodeAt(0).toString(16).padStart(2, "0"));
  } else {
    for (const word of inner.split(/\s+/)) {
      if (word.includes("?")) break;
      if (!/^[0-9A-Fa-f]{2}$/.test(word)) return null;
      bytes.push(word.toLowerCase());
    }
  }
  if (bytes.length === 0) return null;
  if (address.startsWith("-")) return null;
  const offset = /^0[xX]/.test(address) ? Number.parseInt(address.slice(2), 16) : Number.parseInt(address, 10);
  if (!Number.isFinite(offset) || offset < 0) return null;
  return { at: offset, bytes: bytes.join("") };
}

/** Every path an `#include` or an `import` names, as the resolver sees it. */
function wants(text) {
  const out = [];
  for (const line of text.split(/\r?\n/)) {
    const t = line.trim();
    let path = null;
    if (t.startsWith("#include")) {
      const rest = t.slice("#include".length).trim();
      const angle = /^<([^>]+)>/.exec(rest);
      const quoted = /^"([^"]+)"/.exec(rest);
      path = angle?.[1] ?? quoted?.[1] ?? null;
    } else if (t.startsWith("import ")) {
      let rest = t.slice("import ".length).trim().replace(/;$/, "");
      rest = (rest.split(" as ")[0] ?? rest).trim();
      const from = rest.split(" from ");
      rest = (from[from.length - 1] ?? rest).trim();
      if (rest !== "" && !rest.includes(" ")) path = rest.split(".").join("/");
    }
    if (path !== null && !out.includes(path)) out.push(path);
  }
  return out;
}

const stem = (p) => p.replace(/\.(hexpat|pat)$/, "");

/** Where a wanted path lives in the checkout, in the order the reference's own
 *  resolver looks. Null for a path only the built-in table answers for, and for
 *  one nothing in the checkout has. */
function resolve(path) {
  const key = stem(path);
  if (builtinPaths.has(key)) return null;
  for (const root of ["includes", "patterns", "."]) {
    for (const ext of [".pat", ".hexpat", ""]) {
      const full = join(CHECKOUT, root, `${key}${ext}`);
      if (existsSync(full) && statSync(full).isFile()) return rel(full);
    }
  }
  return null;
}

/** Everything a pattern needs fetched alongside it, includes of includes and
 *  all, as paths relative to the repository root. */
function closure(path, text, seen = new Set()) {
  const out = [];
  for (const want of wants(text)) {
    const found = resolve(want);
    if (found === null || seen.has(found)) continue;
    seen.add(found);
    out.push(found);
    const next = join(CHECKOUT, found);
    if (existsSync(next)) out.push(...closure(found, readFileSync(next, "utf8"), seen));
  }
  return [...new Set(out)];
}

// ---- what the converter makes of each one ----

const gaps = execFileSync("cargo", ["run", "-q", "-p", "qubero-core", "--example", "hexpat_gaps", "--", CHECKOUT], {
  cwd: ROOT,
  encoding: "utf8",
  maxBuffer: 256 << 20,
});
/** path -> "clean" | number of gaps | -1 for a pattern the converter refuses. */
const verdict = new Map();
for (const line of gaps.split(/\r?\n/)) {
  const m = /^(patterns\/[^:]+\.hexpat): (clean|refused:|(\d+) gaps)/.exec(line);
  if (m === null) continue;
  verdict.set(m[1], m[2] === "clean" ? 0 : m[2] === "refused:" ? -1 : Number(m[3]));
}

const entries = walk(join(CHECKOUT, "patterns"), ".hexpat").map((full) => {
  const path = rel(full);
  const text = readFileSync(full, "utf8");
  const said = verdict.get(path);
  return {
    // Relative to `patterns/`, which is what the raw URL is built from.
    path: path.replace(/^patterns\//, ""),
    name: (path.split("/").pop() ?? "").replace(/\.hexpat$/, ""),
    description: pragma(text, "description"),
    author: pragma(text, "author"),
    magic: magic(text),
    needs: closure(path, text),
    // How much of the pattern the converter can express: 0 for all of it, a
    // count for the parts it cannot, and null for a pattern it refuses or that
    // the run said nothing about.
    gaps: said === undefined || said === -1 ? null : said,
  };
});

writeFileSync(
  OUT,
  `${JSON.stringify(
    {
      // What this is and where it came from, so the file explains itself to
      // anyone who opens it without the script beside them.
      source: "https://github.com/WerWolv/ImHex-Patterns",
      licence: "GPL-2.0",
      generated: "tools/hexpat_index.mjs",
      note: "Metadata only. No pattern text is copied; the panel fetches a pattern when a reader asks for it.",
      patterns: entries,
    },
    null,
    1,
  )}\n`,
  "utf8",
);

const clean = entries.filter((e) => e.gaps === 0).length;
console.log(`${entries.length} patterns, ${clean} convert whole, ${entries.filter((e) => e.gaps === null).length} refused`);

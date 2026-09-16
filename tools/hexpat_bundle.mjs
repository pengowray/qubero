// Rewrite the generated half of `crates/core/src/hexpat/bundled.rs` and the
// table in `crates/core/formats-hexpat/README.md` from the `.hexpat` files in
// that directory. Running this after copying a file in is the whole procedure.
//
//   node tools/hexpat_bundle.mjs
//
// Two passes, like tools/ksy_bundle.mjs. The table of `include_str!`s is
// written from the directory listing alone, so a file that was deleted stops
// being referenced before anything compiles; then
// `cargo run --example hexpat_bundled` converts the collection and says how
// much of each pattern the IR could express, which fills in the README.
//
// Nothing here decides what to bundle. The whole ImHex-Patterns repository is
// GPL-2.0, so the only files that may be copied in are the ones carrying a
// licence of their own in the header. That decision is recorded in the README
// and in docs/HANDOVER-imhex.md.

import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DIR = join(ROOT, "crates", "core", "formats-hexpat");
const BUNDLED_RS = join(ROOT, "crates", "core", "src", "hexpat", "bundled.rs");
const README = join(DIR, "README.md");

/** The only licences a file here may carry. Anything else is refused outright:
 *  the repository's own GPL-2.0 is the default for a file that says nothing. */
const ALLOWED = ["MIT", "MPL-2.0"];

/** Files that are here only because another one imports them. */
const IMPORT_ONLY = new Map([]);

/** Why a bundled pattern ships although the conversion left parts of it
 *  behind. Every pattern here has gaps; the panel lists each one beside the
 *  line it came from, so what matters is that the gaps are off the path
 *  through the file. */
const WHY_WITH_GAPS = {
  bink_container: "six bit fields cross a byte boundary packed from the low bit up; every frame offset is placed",
  gltf: "the JSON chunk is decoded by an ImHex plugin rather than by the pattern, so its bytes are left unread; the header and the chunk table are placed",
  mbr: "one `break` inside the partition loop; all four partition entries are placed",
  vhd: "two assignments and a `break` in the disc geometry; the footer and the dynamic-disc header are placed",
};

function files(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) files(path, out);
    else if (name.endsWith(".hexpat")) out.push(path);
  }
  return out;
}

/** `#pragma <key> <the rest of the line>`, the way the lexer reads one. */
function pragma(text, key) {
  for (const line of text.split(/\r?\n/)) {
    const m = /^\s*#pragma\s+(\S+)\s*(.*)$/.exec(line);
    if (m !== null && m[1] === key) return m[2].trim();
  }
  return "";
}

/** The licence in the file's own header, or null for a file that carries
 *  none, which means the repository's GPL-2.0 and may not be bundled. */
function licence(text) {
  const head = text.slice(0, 4000);
  if (/Mozilla Public License/.test(head)) return "MPL-2.0";
  if (/Permission is hereby granted, free of charge/.test(head)) return "MIT";
  return null;
}

/** `#pragma magic [ 4D 5A ] @ 0x00` as offset and bytes, by the same rule the
 *  converter uses: bytes up to the first wildcard, and nothing at all for a
 *  magic measured back from the end of the file. */
function magic(text) {
  const raw = pragma(text, "magic");
  if (raw === "") return [];
  const value = (raw.split("//")[0] ?? "").trim().replace(/;$/, "").trim();
  const at = value.lastIndexOf("@");
  if (at < 0) return [];
  const list = value.slice(0, at).trim();
  const address = value.slice(at + 1).trim();
  if (!list.startsWith("[") || !list.endsWith("]")) return [];
  const bytes = [];
  for (const word of list.slice(1, -1).trim().split(/\s+/)) {
    if (word.includes("?")) break;
    if (!/^[0-9A-Fa-f]{2}$/.test(word)) return [];
    bytes.push(word.toLowerCase());
  }
  if (bytes.length === 0) return [];
  // A negative address counts back from the end of the file, which leading
  // bytes cannot be matched against.
  if (address.startsWith("-")) return [];
  const offset = /^0[xX]/.test(address) ? Number.parseInt(address.slice(2), 16) : Number.parseInt(address, 10);
  if (!Number.isFinite(offset) || offset < 0) return [];
  return [[offset, bytes]];
}

const rust = (s) => JSON.stringify(s);

const entries = files(DIR).map((path) => {
  const text = readFileSync(path, "utf8");
  const source = relative(DIR, path).split("\\").join("/");
  const id = (source.split("/").pop() ?? "").replace(/\.hexpat$/, "");
  const lic = licence(text);
  if (lic === null || !ALLOWED.includes(lic)) {
    throw new Error(`${source} carries no licence of its own (${lic ?? "none found"}); only ${ALLOWED.join(", ")} may be bundled`);
  }
  return {
    id,
    source,
    licence: lic,
    title: pragma(text, "description"),
    author: pragma(text, "author"),
    magics: magic(text),
    offered: !IMPORT_ONLY.has(id),
  };
});

function writeTable(rows) {
  const body = rows
    .map(
      (r) =>
        `\tBundled {\n` +
        `\t\tid: ${rust(r.id)},\n` +
        `\t\tname: ${rust(`hexpat:${r.id}`)},\n` +
        `\t\ttitle: ${rust(r.title)},\n` +
        `\t\tauthor: ${rust(r.author)},\n` +
        `\t\tlicence: ${rust(r.licence)},\n` +
        `\t\tsource: ${rust(`patterns/${r.source}`)},\n` +
        `\t\ttext: include_str!("../../formats-hexpat/${r.source}"),\n` +
        `\t\toffered: ${r.offered},\n` +
        `\t\tsignature: ${
          r.magics.length === 0
            ? "&[]"
            : `&[${r.magics.map(([at, bytes]) => `(${at}, &[${bytes.map((b) => `0x${b}`).join(", ")}])`).join(", ")}]`
        },\n` +
        `\t},\n`,
    )
    .join("");
  const text = readFileSync(BUNDLED_RS, "utf8");
  const begin = "// BEGIN GENERATED by tools/hexpat_bundle.mjs -- do not edit by hand\n";
  const end = "// END GENERATED";
  const at = text.indexOf(begin);
  const to = text.indexOf(end, at);
  if (at < 0 || to < 0) throw new Error("bundled.rs has lost its generated markers");
  writeFileSync(BUNDLED_RS, text.slice(0, at + begin.length) + `pub const BUNDLED: &[Bundled] = &[\n${body}];\n` + text.slice(to), "utf8");
}

writeTable(entries);

// ---------- pass two: what each one converts to ----------

const json = execFileSync("cargo", ["run", "-q", "-p", "qubero-core", "--example", "hexpat_bundled"], {
  cwd: ROOT,
  encoding: "utf8",
  maxBuffer: 64 << 20,
});
const report = new Map(JSON.parse(json).map((r) => [r.id, r]));

const decided = entries.map((e) => {
  const r = report.get(e.id);
  if (!r) throw new Error(`hexpat_bundled said nothing about ${e.id}`);
  if (r.error) throw new Error(`${e.id} does not convert: ${r.error}`);
  return { ...e, gaps: r.gaps, notes: r.notes, fields: r.fields };
});

const why = new Map(Object.entries(WHY_WITH_GAPS));
const reason = (d) => {
  if (!d.offered) return IMPORT_ONLY.get(d.id) ?? "here to be imported";
  if (d.gaps === 0) return "";
  return why.get(d.id) ?? "GAPS WITH NO REASON WRITTEN -- add one to WHY_WITH_GAPS in tools/hexpat_bundle.mjs";
};

const rows = decided
  .slice()
  .sort((a, b) => a.id.localeCompare(b.id))
  .map(
    (d) =>
      `| \`${d.id}\` | ${d.licence} | ${d.offered ? "yes" : "import"} | ${d.gaps} | ${
        d.magics.length === 0 ? "no magic" : `${d.magics[0][1].length} bytes at 0x${d.magics[0][0].toString(16)}`
      } | ${reason(d)} |`,
  );

const readme = `# Bundled ImHex patterns

Verbatim copies of \`.hexpat\` files from
[ImHex-Patterns](https://github.com/WerWolv/ImHex-Patterns), compiled into
\`qubero-core\` and converted on demand. \`crates/core/src/hexpat/bundled.rs\` is
the table; this file is the record of which patterns are here and why.

The repository is GPL-2.0 as a whole, so almost none of it can be copied in.
These ${decided.length} files each carry a licence of their own in the header, and those are
the only licences allowed here: ${ALLOWED.map((l) => `\`${l}\``).join(", ")}. Nothing under the repository
licence is bundled, and neither are the include files under \`includes/\`, which
are GPL-2.0 too. A bundled pattern that imports one is given the
declaration-only table in \`crates/core/src/hexpat/includes.rs\` instead.

Every other pattern in the library is reachable from the converter panel, which
fetches the one a reader picks and keeps it for that session only.

Both this file and the table are generated. \`node tools/hexpat_bundle.mjs\`
rewrites them from the directory. Nothing here is edited by hand except the
reasons, which live in the script.

## What the columns say

* **Bundled** is \`yes\` for a pattern a reader can open a file as, and
  \`import\` for a file that is here because another one imports it.
* **Gaps** is how many things in the pattern the IR cannot say. The conversion
  report names every one with the line and column it came from, and the panel
  shows them. A test asserts this number, so the table cannot drift.
* **Magic** is what \`#pragma magic\` pins, which is the one thing that may claim
  a dropped file. A magic measured back from the end of the file is not one
  leading bytes can be matched against, so it is not counted.
* **Reason** says why a pattern with gaps ships anyway. Every one of these has
  gaps: the ImHex pattern language has an imperative half that no template can
  hold, and the test is whether what was left behind is off the path through
  the file.

| Pattern | Licence | Bundled | Gaps | Magic | Reason |
| --- | --- | --- | --- | --- | --- |
${rows.join("\n")}

## Licences

Each file's licence is in its own header and again in \`THIRD-PARTY-NOTICES.md\`
beside the licence text and the URL it came from.
`;

writeFileSync(README, readme, "utf8");

console.log(`${decided.length} patterns, ${decided.filter((d) => d.offered).length} offered, ${decided.reduce((n, d) => n + d.gaps, 0)} gaps`);

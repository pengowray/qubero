// Runs the file signature database over the sample collection and prints, for
// each file, how many formats matched, which one would name the file, and the
// top three with the bytes each pinned down. This is how EXTENSION_WORTH and
// the naming thresholds in src/signatures.ts were set, and how to check them
// again after `npm run wikidata` or `npm run signatures` rebuilds the database.
//
//   node tools/wikisweep.mjs [folder] [--source wikidata|file]
//
// `--source` keeps only one of the two lists, which is how a change to one of
// them is compared against the run before it.
//
// The folder defaults to QUBERO_SAMPLES, or the qubero-samples checkout beside
// the repository, which from a git worktree under .claude/worktrees is not
// where it looks, so set the variable there.

import { readFileSync, readdirSync, statSync, openSync, readSync, closeSync } from "node:fs";
import { join, relative, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { buildIndex, compileAll, decode, matchFormats, namingMatch } from "../src/signatures.ts";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const sourceAt = args.indexOf("--source");
const only = sourceAt < 0 ? null : args[sourceAt + 1];
if (sourceAt >= 0) args.splice(sourceAt, 2);
if (only !== null && only !== "wikidata" && only !== "file") throw new Error("--source takes wikidata or file");
const root = args[0] ?? process.env.QUBERO_SAMPLES ?? join(WEB, "..", "..", "qubero-samples");
const WINDOW = 64 * 1024;
const TAIL = 4096;

const stored = JSON.parse(readFileSync(join(WEB, "public", "signatures.json"), "utf8"));
const data = decode(stored);
const formats = only === null ? data.formats : data.formats.filter((f) => f.source === only);
const t0 = performance.now();
const compiled = compileAll({ ...data, formats });
const index = buildIndex(compiled);
console.log(`${data.fetched}${only === null ? "" : `, ${only} only`}`);
console.log(`compiled ${compiled.length} patterns for ${formats.length} formats in ${(performance.now() - t0).toFixed(0)} ms`);

const files = [];
const walk = (d) => {
  for (const n of readdirSync(d)) {
    if (n.startsWith(".")) continue;
    const p = join(d, n);
    if (statSync(p).isDirectory()) walk(p);
    else if (!/\.(md|tsv|py)$/.test(n)) files.push(p);
  }
};
walk(root);

let named = 0;
let time = 0;
for (const f of files) {
  let size, fd;
  try {
    size = statSync(f).size;
    fd = openSync(f, "r");
  } catch {
    continue; // A temporary file a writer in the samples folder has already renamed.
  }
  const head = Buffer.alloc(Math.min(WINDOW, size));
  readSync(fd, head, 0, head.length, 0);
  const tailLen = Math.min(TAIL, size);
  const tail = Buffer.alloc(tailLen);
  readSync(fd, tail, 0, tailLen, size - tailLen);
  closeSync(fd);
  const t = performance.now();
  const m = matchFormats(index, { head, tail, name: f });
  time += performance.now() - t;
  const n = namingMatch(m);
  if (n !== null) named++;
  const top = m.slice(0, 3).map((x) => `${x.format.label}[${x.fixed}${x.extensionAgrees ? ",ext" : ""}]`).join("; ");
  console.log(`${relative(root, f).padEnd(50).slice(0, 50)} ${String(m.length).padStart(4)} ${n ? "NAME " + n.format.label : "-"} | ${top}`);
}
console.log({ files: files.length, named, msPerFile: (time / files.length).toFixed(1) });

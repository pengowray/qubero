// Runs the Wikidata format patterns over the sample collection and prints,
// for each file, how many formats matched, which one would name the file, and
// the top three with the bytes each pinned down. This is how EXTENSION_WORTH
// and the naming thresholds in src/wikiformats.ts were set, and how to check
// them again after `npm run wikidata` refreshes the patterns.
//
//   node tools/wikisweep.mjs [folder]
//
// The folder defaults to QUBERO_SAMPLES, or the qubero-samples checkout beside
// the repository, which from a git worktree under .claude/worktrees is not
// where it looks, so set the variable there.

import { readFileSync, readdirSync, statSync, openSync, readSync, closeSync } from "node:fs";
import { join, relative, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { compileAll, matchFormats, namingMatch } from "../src/wikiformats.ts";

const WEB = join(dirname(fileURLToPath(import.meta.url)), "..");
const root = process.argv[2] ?? process.env.QUBERO_SAMPLES ?? join(WEB, "..", "..", "qubero-samples");
const WINDOW = 64 * 1024;
const TAIL = 4096;

const data = JSON.parse(readFileSync(join(WEB, "public", "wikidata", "formats.json"), "utf8"));
const t0 = performance.now();
const compiled = compileAll(data);
console.log(`compiled ${compiled.length} patterns in ${(performance.now() - t0).toFixed(0)} ms`);

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
  const size = statSync(f).size;
  const fd = openSync(f, "r");
  const head = Buffer.alloc(Math.min(WINDOW, size));
  readSync(fd, head, 0, head.length, 0);
  const tailLen = Math.min(TAIL, size);
  const tail = Buffer.alloc(tailLen);
  readSync(fd, tail, 0, tailLen, size - tailLen);
  closeSync(fd);
  const t = performance.now();
  const m = matchFormats(compiled, { head, tail, name: f });
  time += performance.now() - t;
  const n = namingMatch(m);
  if (n !== null) named++;
  const top = m.slice(0, 3).map((x) => `${x.format.label}[${x.fixed}${x.extensionAgrees ? ",ext" : ""}]`).join("; ");
  console.log(`${relative(root, f).padEnd(50).slice(0, 50)} ${String(m.length).padStart(4)} ${n ? "NAME " + n.format.label : "-"} | ${top}`);
}
console.log({ files: files.length, named, msPerFile: (time / files.length).toFixed(1) });

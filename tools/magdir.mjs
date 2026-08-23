// Copies the magic rule files into web/public/magdir, where the page can fetch
// one when it needs to know what a format's first bytes mean.
//
//   node tools/magdir.mjs
//
// They come from the `magic-db` crate rather than from the `file` project,
// because that is the copy the compiled database in the wasm module was built
// from. Taking the text from anywhere else would let the name a file is given
// and the fields shown under it come from two different sets of rules, which
// disagree in small ways.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "web", "public", "magdir");

const meta = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--filter-platform", "wasm32-unknown-unknown"], {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  }),
);

const db = meta.packages.find((p) => p.name === "magic-db");
if (!db) throw new Error("magic-db is not in the dependency tree");
const src = join(dirname(db.manifest_path), "src", "magdir");

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

let files = 0;
let bytes = 0;
for (const name of readdirSync(src)) {
  const from = join(src, name);
  if (!statSync(from).isFile()) continue;
  copyFileSync(from, join(OUT, name));
  files += 1;
  bytes += statSync(from).size;
}

console.log(`magdir: ${files} files, ${(bytes / 1024).toFixed(0)} KiB, from magic-db ${db.version}`);

// Copies the magic rule files into web/public/magdir, where the page can fetch
// one when it needs to know what a format's first bytes mean.
//
//   node tools/magdir.mjs
//
// Where they come from, and why from there, is in tools/magicdb.mjs.

import { copyFileSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { magicDb } from "./magicdb.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "web", "public", "magdir");

const { dir: src, version } = magicDb();

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

console.log(`magdir: ${files} files, ${(bytes / 1024).toFixed(0)} KiB, from magic-db ${version}`);

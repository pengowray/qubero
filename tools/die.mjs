// Bundles the Detect It Easy signature rules that Qubero can read, into
// web/public/diesig, where the page fetches them when it opens an executable.
//
//   node tools/die.mjs
//
// Where file(1) answers what format a file is, these answer what tool produced
// it: which packer, which compiler, which protector.
//
// The rules are MIT and stay byte for byte as their authors wrote them,
// author credits included. This only concatenates them, putting a marker line
// before each so the parser can tell them apart and name the one that answered.
// Nothing is converted, so a newer database drops straight in: change COMMIT
// below and run this again. That is the whole update process.

import { execFileSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The Detect It Easy commit these rules come from. Bump to update. */
const COMMIT = "7f3119cb840e59451b876935d50f72bf982cdc02";
const REPO = "https://github.com/horsicq/Detect-It-Easy.git";

// Which directories to take, and what to call the bundle each becomes. `db`
// is the source database; `dbs_min` is a generated copy of it and is not used.
const BUNDLES = [
  // A .COM is loaded flat, so its rules test from the first byte.
  { dir: "COM", out: "com.sig" },
  // An MZ executable's rules test from the instruction the loader jumps to.
  { dir: "MSDOS", out: "msdos.sig" },
];

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "web", "public", "diesig");
const WORK = join(ROOT, "target", "die-checkout");

// Fetch just this one commit, into target/ where build output lives.
if (!existsSync(join(WORK, ".git"))) {
  rmSync(WORK, { recursive: true, force: true });
  mkdirSync(WORK, { recursive: true });
  execFileSync("git", ["init", "--quiet"], { cwd: WORK, stdio: "inherit" });
  execFileSync("git", ["remote", "add", "origin", REPO], { cwd: WORK, stdio: "inherit" });
}
execFileSync("git", ["fetch", "--quiet", "--depth", "1", "origin", COMMIT], { cwd: WORK, stdio: "inherit" });
execFileSync("git", ["checkout", "--quiet", "--force", "FETCH_HEAD"], { cwd: WORK, stdio: "inherit" });
const head = execFileSync("git", ["rev-parse", "HEAD"], { cwd: WORK, encoding: "utf8" }).trim();

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

for (const { dir, out } of BUNDLES) {
  const from = join(WORK, "db", dir);
  const names = readdirSync(from).filter((n) => n.endsWith(".sg")).sort();
  const parts = [`// Detect It Easy signature rules, from ${REPO} at ${head}`, `// Directory: db/${dir}. Files below are unmodified.`];
  for (const name of names) {
    parts.push(`// >>> file: ${name.replace(/\.\d+\.sg$/, "").replace(/\.sg$/, "")}`);
    parts.push(readFileSync(join(from, name), "utf8").replace(/\r\n/g, "\n").replace(/\n*$/, "\n"));
  }
  const text = parts.join("\n");
  writeFileSync(join(OUT, out), text, "utf8");
  console.log(`${out}: ${names.length} rules, ${(text.length / 1024).toFixed(0)} KiB`);
}

console.log(`Detect It Easy at ${head}`);

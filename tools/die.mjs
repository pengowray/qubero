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
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The Detect It Easy commit these rules come from. Bump to update. */
const COMMIT = "7f3119cb840e59451b876935d50f72bf982cdc02";
const REPO = "https://github.com/horsicq/Detect-It-Easy";

// Which directories to take, and what to call the bundle each becomes. `db`
// is the source database; `dbs_min` is a generated copy of it and is not used.
const BUNDLES = [
  // A .COM is loaded flat, so its rules test from the first byte.
  { dir: "COM", out: "com.sig" },
  // An MZ executable's rules test from the instruction the loader jumps to.
  { dir: "MSDOS", out: "msdos.sig" },
  // A Windows executable's too, though there the header gives that as an
  // address in memory and the section table turns it back into an offset.
  { dir: "PE", out: "pe.sig" },
];

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "web", "public", "diesig");
const WORK = join(ROOT, "target", "die-checkout");
const TARBALL_NAME = `die-${COMMIT}.tar.gz`;
const TARBALL = join(ROOT, "target", TARBALL_NAME);

// Take the commit as a tarball rather than a git fetch. Asking a remote to
// hand over a bare commit id is answered only if it agrees the commit is one
// of its own, and GitHub kept saying it was not, to the build runners though
// never here. An archive download is a plain GET, and it always answers.
const url = `${REPO}/archive/${COMMIT}.tar.gz`;
const res = await fetch(url);
if (!res.ok) throw new Error(`${res.status} ${res.statusText} for ${url} - is commit ${COMMIT} still in the repository?`);
mkdirSync(dirname(TARBALL), { recursive: true });
writeFileSync(TARBALL, Buffer.from(await res.arrayBuffer()));

// Only the rule directories come out, under their own names. A bumped commit
// gets a clean directory so no file the new one dropped can survive in it.
rmSync(WORK, { recursive: true, force: true });
mkdirSync(WORK, { recursive: true });
const members = BUNDLES.map(({ dir }) => `Detect-It-Easy-${COMMIT}/db/${dir}`);
// Run in the directory and name the tarball relative to it: a Windows path
// starts `C:`, and GNU tar reads that as a machine to log in to.
execFileSync("tar", ["-xzf", join("..", TARBALL_NAME), "--strip-components", "1", ...members], { cwd: WORK, stdio: "inherit" });

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

for (const { dir, out } of BUNDLES) {
  const from = join(WORK, "db", dir);
  const names = readdirSync(from).filter((n) => n.endsWith(".sg")).sort();
  const parts = [`// Detect It Easy signature rules, from ${REPO} at ${COMMIT}`, `// Directory: db/${dir}. Files below are unmodified.`];
  for (const name of names) {
    parts.push(`// >>> file: ${name.replace(/\.\d+\.sg$/, "").replace(/\.sg$/, "")}`);
    parts.push(readFileSync(join(from, name), "utf8").replace(/\r\n/g, "\n").replace(/\n*$/, "\n"));
  }
  const text = parts.join("\n");
  writeFileSync(join(OUT, out), text, "utf8");
  console.log(`${out}: ${names.length} rules, ${(text.length / 1024).toFixed(0)} KiB`);
}

console.log(`Detect It Easy at ${COMMIT}`);

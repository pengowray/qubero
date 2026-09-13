// Where the file(1) rule text lives, for the tools that read it.
//
// It comes from the `magic-db` crate rather than from the `file` project,
// because that is the copy the compiled database in the wasm module was built
// from. Taking the text from anywhere else would let the name a file is given
// and the fields shown under it come from two different sets of rules, which
// disagree in small ways.

import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** The magdir directory and the crate version it came from. */
export function magicDb() {
  const meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--filter-platform", "wasm32-unknown-unknown"], {
      cwd: ROOT,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  const db = meta.packages.find((p) => p.name === "magic-db");
  if (!db) throw new Error("magic-db is not in the dependency tree");
  return { dir: join(dirname(db.manifest_path), "src", "magdir"), version: db.version };
}

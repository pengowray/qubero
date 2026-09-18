// Where the sample collection is, for the browser tests, answered the way
// `crates/samples` answers it for the Rust ones: what `QUBERO_SAMPLES` names
// (or `SAMPLES`, which these tests used first), else the first
// `qubero-samples` directory beside this checkout or beside an ancestor of it.
//
// Each of these tests used to carry an absolute path of its own as the
// fallback, and two of them still named a Windows drive after the checkout
// moved to Linux: a path written down once is wrong on the next machine, and a
// test that cannot find the collection says "skipped" rather than failing, so
// nothing announced it.
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The collection, or null where there is none. */
export function samplesDir() {
  const set = process.env.QUBERO_SAMPLES || process.env.SAMPLES || "";
  for (const named of set.split(";")) {
    if (named && existsSync(named)) {
      return resolve(named);
    }
  }
  let at = dirname(fileURLToPath(import.meta.url));
  for (;;) {
    const beside = join(at, "qubero-samples");
    if (existsSync(beside)) {
      return beside;
    }
    const up = dirname(at);
    if (up === at) {
      return null;
    }
    at = up;
  }
}

/** One file of the collection, or null where it is not there. */
export function sampleFile(...parts) {
  const root = samplesDir();
  if (root === null) {
    return null;
  }
  const path = join(root, ...parts);
  return existsSync(path) ? path : null;
}

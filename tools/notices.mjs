// Regenerates THIRD-PARTY-NOTICES.md from the crates actually linked into the
// two wasm modules. Run it after changing dependencies:
//
//   node tools/notices.mjs
//
// The MIT, BSD and Apache licences all require the copyright notice to travel
// with the binary, and a wasm module in a web page is a binary distribution, so
// the file has to be shipped and has to be right. Hand-maintaining 70 entries
// would not stay right, hence this.
//
// Licence texts come from the crate sources in ~/.cargo/registry. A crate that
// ships none is listed with its SPDX expression alone and needs checking by
// hand; the summary at the top of the generated file counts those.

import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = "wasm32-unknown-unknown";

// Which crates ship in the browser. A proc-macro crate and its dependencies run
// on the build machine and never reach the module, so they are followed for
// completeness but marked, not bundled.
const ROOTS = ["qubero-wasm", "qubero-magic"];

const meta = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--filter-platform", TARGET], {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  }),
);

const byId = new Map(meta.packages.map((p) => [p.id, p]));
const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));

/** Every non-dev dependency reachable from `id`, with whether it is build-only. */
const reached = new Map(); // id -> { buildOnly: boolean }
function walk(id, buildOnly) {
  const seen = reached.get(id);
  if (seen && (seen.buildOnly === false || buildOnly === true)) return;
  reached.set(id, { buildOnly: seen ? seen.buildOnly && buildOnly : buildOnly });
  for (const d of nodes.get(id)?.deps ?? []) {
    if (d.dep_kinds.length > 0 && d.dep_kinds.every((k) => k.kind === "dev")) continue;
    const p = byId.get(d.pkg);
    if (!p) continue;
    const isBuildTool = p.targets.some((t) => t.kind.includes("proc-macro")) || d.dep_kinds.some((k) => k.kind === "build");
    walk(d.pkg, buildOnly || isBuildTool);
  }
}
for (const name of ROOTS) {
  const p = meta.packages.find((q) => q.name === name);
  if (!p) throw new Error(`no such package: ${name}`);
  walk(p.id, false);
}

const LICENCE_FILE = /^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)([-_.].*)?$/i;

// Where a crate offers a choice, this is the order we take it in, and the arm
// we take is the only one whose text belongs in the notices. Listing the GNU
// GPL under a crate we took under BSD terms would say we accepted the GPL.
const PREFERENCE = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "0BSD", "Zlib", "MIT-0", "Unlicense", "CC0-1.0"];

/** Which licence file is which, by the names crates actually use. */
const FILE_LICENCE = [
  [/MIT/i, "MIT"],
  [/APACHE/i, "Apache-2.0"],
  [/BSD/i, "BSD-2-Clause"],
  [/GPL/i, "GPL-3.0"],
  [/ZLIB/i, "Zlib"],
  [/UNLICEN[CS]E/i, "Unlicense"],
  [/CC0/i, "CC0-1.0"],
  [/UNICODE/i, "Unicode-3.0"],
];

/**
 * The licences we are taking a crate under: every `AND` term, and for each
 * `OR` the first arm in PREFERENCE. An expression we cannot read comes back
 * empty, which lands the crate in the check-by-hand list.
 */
function chosen(expr) {
  if (!expr) return [];
  const out = [];
  for (const term of expr.split(/\s+AND\s+/i)) {
    const arms = term
      .replace(/[()]/g, "")
      .split(/\s+OR\s+|\//)
      .map((a) => a.trim().replace(/\s+WITH\s+.*$/i, ""))
      .filter(Boolean);
    const pick = PREFERENCE.find((l) => arms.includes(l)) ?? (arms.length === 1 ? arms[0] : null);
    if (pick === null) return [];
    out.push(pick);
  }
  return out;
}

/** The texts for the licences we took, longest first so the real one leads. */
function texts(pkg) {
  const dir = dirname(pkg.manifest_path);
  if (!existsSync(dir)) return [];
  const want = chosen(pkg.license);
  const out = [];
  for (const name of readdirSync(dir)) {
    if (!LICENCE_FILE.test(name)) continue;
    const isFor = FILE_LICENCE.find(([re]) => re.test(name))?.[1];
    // A file naming a licence we did not take is skipped. One that names none
    // (a bare LICENSE or COPYING) is the crate's only text, so it is kept.
    if (isFor !== undefined && want.length > 0 && !want.includes(isFor)) continue;
    const body = readFileSync(join(dir, name), "utf8").trim();
    if (body.length > 0) out.push({ name, body });
  }
  return out.sort((a, b) => b.body.length - a.body.length);
}

const crates = [...reached.entries()]
  .map(([id, { buildOnly }]) => ({ pkg: byId.get(id), buildOnly }))
  .filter(({ pkg }) => pkg && !pkg.name.startsWith("qubero"))
  .sort((a, b) => a.pkg.name.localeCompare(b.pkg.name));

const shipped = crates.filter((c) => !c.buildOnly);
const buildOnly = crates.filter((c) => c.buildOnly);

// Notices that cannot come from a crate's own source: the crates that upload
// no licence file, and the rule database, whose upstream is a project rather
// than a crate. Written by hand in tools/notices-extra.md.
const extraPath = join(ROOT, "tools", "notices-extra.md");
const extra = existsSync(extraPath) ? readFileSync(extraPath, "utf8").trim() : "";

// A crate with no licence text of its own, and no mention in the hand-written
// part either, is a gap somebody has to close.
const missing = shipped.filter((c) => texts(c.pkg).length === 0 && !extra.includes(c.pkg.name));

let md = `# Third-party notices

Generated by \`node tools/notices.mjs\`. Do not edit by hand.

Qubero ships two WebAssembly modules built from Rust. ${shipped.length} crates are compiled
into them and their licences require this notice to travel with the code.
${buildOnly.length} further crates run only on the build machine (procedural macros and
their dependencies); they are listed separately and are not distributed.
`;

if (missing.length > 0) {
  md += `\n${missing.length} shipped crate(s) carry an SPDX expression but no licence file in\ntheir source: ${missing.map((c) => c.pkg.name).join(", ")}. Check these by hand.\n`;
}

if (extra !== "") md += `\n${extra}\n`;

md += `\n## Shipped in the browser\n\n| Crate | Version | Offered under | Taken under |\n| --- | --- | --- | --- |\n`;
for (const { pkg } of shipped) {
  const took = chosen(pkg.license);
  md += `| ${pkg.name} | ${pkg.version} | ${pkg.license ?? pkg.license_file ?? "see below"} | ${took.join(" and ") || "check by hand"} |\n`;
}

md += `\n## Build machine only, not distributed\n\n| Crate | Version | Licence |\n| --- | --- | --- |\n`;
for (const { pkg } of buildOnly) {
  md += `| ${pkg.name} | ${pkg.version} | ${pkg.license ?? pkg.license_file ?? "see below"} |\n`;
}

md += `\n## Licence texts\n\nOne section per shipped crate, in the crate's own words.\n`;
for (const { pkg } of shipped) {
  const took = chosen(pkg.license).join(" and ") || "**a licence expression this script could not read: check by hand**";
  md += `\n### ${pkg.name} ${pkg.version}\n\nOffered under \`${pkg.license ?? pkg.license_file ?? "unstated"}\`, taken under ${took}.  \n`;
  if (pkg.repository) md += `Source: ${pkg.repository}\n`;
  for (const t of texts(pkg)) {
    md += `\n<details><summary>${t.name}</summary>\n\n\`\`\`\n${t.body}\n\`\`\`\n\n</details>\n`;
  }
}

writeFileSync(join(ROOT, "THIRD-PARTY-NOTICES.md"), md, "utf8");
console.log(`${shipped.length} shipped, ${buildOnly.length} build-only, ${missing.length} without a licence file`);

// Regenerates THIRD-PARTY-NOTICES.md from what actually ships in the browser:
// the crates linked into the two wasm modules, and the npm packages the page
// bundles. Run it after changing dependencies:
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
//
// It reads two trees and needs both. `node_modules` is not tracked, so a git
// worktree usually has none; this looks in the main checkout before giving up,
// and refuses to write at all rather than emit a file with one half missing.
// A notices file that quietly loses its npm half still looks like a notices
// file, which is why that is an error and not a warning.

import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync, existsSync, statSync } from "node:fs";
import { join, dirname, relative } from "node:path";
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

/**
 * The npm packages that end up in the bundle.
 *
 * `dependencies` in web/package.json and everything they in turn depend on.
 * `devDependencies` are the build (vite, typescript, the type packages) and
 * reach no browser. Read off the installed tree rather than off a lockfile,
 * because what is installed is what vite bundles, and because that is where
 * the licence texts are.
 */
const WEB = join(ROOT, "web");

/**
 * Where the installed npm tree is.
 *
 * Beside `web/package.json`, normally. In a git worktree it is usually not:
 * `node_modules` is not tracked, so a fresh worktree has none, and this used
 * to quietly find nothing and write a file with every npm notice missing. So
 * the main checkout is asked as well, which is where an agent working in a
 * worktree will have one.
 */
function nodeModules() {
  const here = join(WEB, "node_modules");
  if (existsSync(here)) return here;
  try {
    const common = execFileSync("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], {
      cwd: ROOT,
      encoding: "utf8",
    }).trim();
    const main = join(dirname(common), "web", "node_modules");
    if (main !== here && existsSync(main)) {
      console.error(`notices: no web/node_modules here, reading the main checkout's at ${main}`);
      return main;
    }
  } catch {
    // Not a git checkout, or no git. Nothing to fall back to.
  }
  return null;
}
const NODE_MODULES = nodeModules();

function npmManifest(name) {
  if (NODE_MODULES === null) return null;
  const file = join(NODE_MODULES, name, "package.json");
  return existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : null;
}

const npmSeen = new Map();
function walkNpm(name) {
  if (npmSeen.has(name)) return;
  const pkg = npmManifest(name);
  if (pkg === null) return;
  npmSeen.set(name, pkg);
  for (const dep of Object.keys(pkg.dependencies ?? {})) walkNpm(dep);
}
const webPkg = existsSync(join(WEB, "package.json")) ? JSON.parse(readFileSync(join(WEB, "package.json"), "utf8")) : {};
for (const name of Object.keys(webPkg.dependencies ?? {})) {
  // A `@types/*` package is declarations only: nothing of it survives `tsc`,
  // so nothing of it is distributed.
  if (!name.startsWith("@types/")) walkNpm(name);
}
const npmPkgs = [...npmSeen.values()].sort((a, b) => a.name.localeCompare(b.name));

// Stop rather than write a file that is missing what it exists to carry. A
// licence notice that silently loses two hundred lines is worse than no run at
// all: the file still looks like a notices file, and nothing says the npm half
// is gone. This is the check that was not here, and it cost two agents an
// afternoon between them.
const declared = Object.keys(webPkg.dependencies ?? {}).filter((n) => !n.startsWith("@types/"));
if (declared.length > 0 && npmPkgs.length === 0) {
  console.error(
    `notices: web/package.json declares ${declared.length} dependencies and none of them resolved.\n` +
      (NODE_MODULES === null
        ? "  No node_modules was found here or in the main checkout.\n"
        : `  Looked in ${NODE_MODULES}.\n`) +
      "  Run `npm install` in web/, or run this from a checkout that has one.\n" +
      "  THIRD-PARTY-NOTICES.md has NOT been written.",
  );
  process.exit(1);
}

// The same rule for the other half. `cargo metadata` failing to reach the wasm
// crates would leave a file with the npm notices and no crate ones.
if (shipped.length === 0) {
  console.error("notices: no crates resolved for the wasm target. THIRD-PARTY-NOTICES.md has NOT been written.");
  process.exit(1);
}

/** A package's own licence text, by the file names npm packages use. */
function npmTexts(pkg) {
  const dir = join(NODE_MODULES, pkg.name);
  if (!existsSync(dir)) return [];
  const out = [];
  for (const name of readdirSync(dir)) {
    if (!LICENCE_FILE.test(name)) continue;
    const body = readFileSync(join(dir, name), "utf8").trim();
    if (body.length > 0) out.push({ name, body });
  }
  return out.sort((a, b) => b.body.length - a.body.length);
}

const npmMissing = npmPkgs.filter((p) => npmTexts(p).length === 0);

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

Qubero ships two WebAssembly modules built from Rust and a page built from
TypeScript. ${shipped.length} crates are compiled into the modules and ${npmPkgs.length} npm packages are
bundled into the page; their licences require this notice to travel with the
code. ${buildOnly.length} further crates run only on the build machine (procedural macros and
their dependencies); they are listed separately and are not distributed, and
neither is anything in \`devDependencies\`.
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

md += `\n## Bundled into the page\n\n| Package | Version | Licence |\n| --- | --- | --- |\n`;
for (const pkg of npmPkgs) {
  md += `| ${pkg.name} | ${pkg.version} | ${pkg.license ?? "see below"} |\n`;
}
if (npmMissing.length > 0) {
  md += `\n${npmMissing.length} bundled package(s) ship no licence file: ${npmMissing.map((p) => p.name).join(", ")}. Check these by hand.\n`;
}

// ---- Kaitai Struct format descriptions ----
//
// Not a crate and not an npm package: `.ksy` files copied verbatim into
// `crates/core/formats-ksy` and compiled into the core. Each is licensed on
// its own, so each is listed on its own, and the licence texts follow once per
// licence rather than once per file.
const KSY_DIR = join(ROOT, "crates", "core", "formats-ksy");
const KSY_UPSTREAM = "https://github.com/kaitai-io/kaitai_struct_formats/blob/master";

function ksyFiles(dir, out = []) {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) ksyFiles(path, out);
    else if (name.endsWith(".ksy")) out.push(path);
  }
  return out;
}

const ksy = ksyFiles(KSY_DIR).map((path) => {
  const source = relative(KSY_DIR, path).replaceAll("\\", "/");
  const text = readFileSync(path, "utf8");
  const read = (key) => new RegExp(`^ {2}${key}: *(.*?) *$`, "m").exec(text)?.[1]?.replace(/^["'](.*)["']$/, "$1") ?? "";
  return { source, id: read("id"), licence: read("license") };
});

const ksyLicences = [...new Set(ksy.map((f) => f.licence))].sort();

if (ksy.length > 0) {
  md += `\n## Kaitai Struct format descriptions\n\n${ksy.length} \`.ksy\` files from the [Kaitai Struct format\nlibrary](https://github.com/kaitai-io/kaitai_struct_formats) are copied\nverbatim into \`crates/core/formats-ksy\` and compiled into the core, where they\nare converted into templates. Each file carries its own licence; nothing from\nthe Kaitai Struct compiler, which is GPL-3.0, is used here. Which files these\nare and why is \`crates/core/formats-ksy/README.md\`.\n\n| Format | Licence | Source |\n| --- | --- | --- |\n`;
  for (const f of ksy) md += `| ${f.id} | ${f.licence} | [${f.source}](${KSY_UPSTREAM}/${f.source}) |\n`;
}

// ---- ImHex patterns ----
//
// The ImHex-Patterns repository is GPL-2.0 as a whole, so the only files copied
// into `crates/core/formats-hexpat` are the handful carrying a licence of their
// own in the header. The licence is read from the header, which is the only
// place it is stated: there is no `meta/license` key in this language.
const HEXPAT_DIR = join(ROOT, "crates", "core", "formats-hexpat");
const HEXPAT_UPSTREAM = "https://github.com/WerWolv/ImHex-Patterns/blob/master/patterns";

function hexpatFiles(dir, out = []) {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) hexpatFiles(path, out);
    else if (name.endsWith(".hexpat")) out.push(path);
  }
  return out;
}

const hexpat = hexpatFiles(HEXPAT_DIR).map((path) => {
  const source = relative(HEXPAT_DIR, path).replaceAll("\\", "/");
  const head = readFileSync(path, "utf8").slice(0, 4000);
  const licence = /Mozilla Public License/.test(head)
    ? "MPL-2.0"
    : /Permission is hereby granted, free of charge/.test(head)
      ? "MIT"
      : "**no licence in the header: this file may not be bundled**";
  const author = /^\s*#pragma\s+author\s+(.*)$/m.exec(head)?.[1]?.trim() ?? "";
  return { source, id: (source.split("/").pop() ?? "").replace(/\.hexpat$/, ""), licence, author };
});

const hexpatLicences = [...new Set(hexpat.map((f) => f.licence))].sort();

if (hexpat.length > 0) {
  md +=
    `\n## ImHex patterns\n\n${hexpat.length} \`.hexpat\` files from\n[ImHex-Patterns](https://github.com/WerWolv/ImHex-Patterns) are copied verbatim\ninto \`crates/core/formats-hexpat\` and compiled into the core, where they are\nconverted into templates. The repository is GPL-2.0 as a whole and none of it\nis bundled under that licence: these files each carry a licence of their own in\nthe header, and those headers travel with them. Nothing from the ImHex pattern\nlanguage runtime, which is LGPL-2.1, is used here. Which files these are and\nwhy is \`crates/core/formats-hexpat/README.md\`.\n\n` +
    `Every other pattern in that library stays where it is. The converter panel\nfetches the one a reader asks for, uses it for that session and does not store\nit.\n\n| Pattern | Licence | Author | Source |\n| --- | --- | --- | --- |\n`;
  for (const f of hexpat) md += `| ${f.id} | ${f.licence} | ${f.author} | [${f.source}](${HEXPAT_UPSTREAM}/${f.source}) |\n`;
}

md += `\n## Licence texts\n\nOne section per shipped crate and bundled package, in its own words.\n`;

// The `.ksy` and `.hexpat` files share a handful of licences between them, so
// the texts are written once each here rather than once per file.
const descriptions = [...ksy, ...hexpat];
const descLicences = [...new Set([...ksyLicences, ...hexpatLicences])].sort();
if (descriptions.length > 0) {
  md += `\nThe licences the format descriptions are under come first, once each, since\n${descriptions.length} files share ${descLicences.length} of them. The texts are SPDX's, with SPDX's own\nmatching markup taken out.\n`;
  for (const spdx of descLicences) {
    const path = join(ROOT, "tools", "licences", `${spdx}.txt`);
    if (!existsSync(path)) {
      md += `\n### ${spdx}\n\n**No text for this licence in \`tools/licences\`: add \`${spdx}.txt\` from https://spdx.org/licenses/${spdx}.txt.**\n`;
      continue;
    }
    const used = descriptions.filter((f) => f.licence === spdx);
    md += `\n### ${spdx}\n\nThe licence of ${used.length} of the format descriptions above.  \nSource: https://spdx.org/licenses/${spdx}.html\n`;
    md += `\n<details><summary>${spdx}</summary>\n\n\`\`\`\n${readFileSync(path, "utf8").trim()}\n\`\`\`\n\n</details>\n`;
  }
}
for (const { pkg } of shipped) {
  const took = chosen(pkg.license).join(" and ") || "**a licence expression this script could not read: check by hand**";
  md += `\n### ${pkg.name} ${pkg.version}\n\nOffered under \`${pkg.license ?? pkg.license_file ?? "unstated"}\`, taken under ${took}.  \n`;
  if (pkg.repository) md += `Source: ${pkg.repository}\n`;
  for (const t of texts(pkg)) {
    md += `\n<details><summary>${t.name}</summary>\n\n\`\`\`\n${t.body}\n\`\`\`\n\n</details>\n`;
  }
}

for (const pkg of npmPkgs) {
  md += `\n### ${pkg.name} ${pkg.version}\n\nOffered under \`${pkg.license ?? "unstated"}\`.  \n`;
  const repo = typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url;
  if (repo) md += `Source: ${repo.replace(/^git\+/, "").replace(/\.git$/, "")}\n`;
  for (const t of npmTexts(pkg)) {
    md += `\n<details><summary>${t.name}</summary>\n\n\`\`\`\n${t.body}\n\`\`\`\n\n</details>\n`;
  }
}

writeFileSync(join(ROOT, "THIRD-PARTY-NOTICES.md"), md, "utf8");
console.log(
  `${shipped.length} crates shipped, ${npmPkgs.length} npm bundled, ${buildOnly.length} build-only, ` +
    `${missing.length + npmMissing.length} without a licence file`,
);

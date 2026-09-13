// Generates the BUFR tables under crates/core/src/formats/bufr_tables/ from
// the WMO's own copy of them and from ecCodes.
//
//   node tools/bufrtables.mjs
//
// A BUFR message does not say how wide any of its values are. It names each
// one by a descriptor, and Table B is what says that descriptor 0-12-101 is an
// air temperature in kelvin, scaled by 100, sixteen bits wide. Table D says what
// a sequence descriptor stands for. Without both, section 4 is bits with no
// boundaries in them, so the tables are not a nicety here: they are the file
// format's other half.
//
// Two sources, for two different questions:
//
// - The current version, in full, from the WMO's BUFR4 repository (MIT). That
//   is where the tables are maintained, and its names are written in sentence
//   case, which is how everything else in this program names things.
// - What older versions said differently, from ecCodes (Apache-2.0), which
//   keeps a copy of every version the WMO has published. The tables only grow:
//   a newer version adds descriptors and almost never changes one, so an older
//   version is the current one with a few dozen entries put back the way they
//   were. Only those entries are kept.
//
// The deltas are worked out inside ecCodes, its old version against its own
// copy of the current one, so that a difference between two copies of the same
// table is never mistaken for a change between versions. The WMO's current
// table is checked against ecCodes' copy of it, and a disagreement stops the
// run.
//
// Which older versions are bundled is OLDER, below: the ones the sample
// collection's files are written against. A message against any other version
// is read with the nearest newer one, which is what ecCodes does for the
// versions it only has a link for.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The WMO release the current tables come from. Bump to update. */
const WMO_TAG = "v46";
const LATEST = 46;
/** The ecCodes commit the older versions come from. */
const ECCODES_COMMIT = "9ad744c51deb09c831ca7750919093ad19fae5ad";
/** The older versions to bundle the differences of. */
const OLDER = [13, 15];

const RAW = "https://raw.githubusercontent.com";
const WMO = `${RAW}/wmo-im/BUFR4/${WMO_TAG}/txt`;
const ECCODES = `${RAW}/ecmwf/eccodes/${ECCODES_COMMIT}/definitions/bufr/tables/0/wmo`;

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "crates", "core", "src", "formats", "bufr_tables");

async function text(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: ${r.status}`);
  return (await r.text()).replace(/^﻿/, "");
}

/** RFC 4180 rows: quoted fields may hold commas, quotes and newlines. */
function csv(src) {
  const rows = [];
  let row = [];
  let field = "";
  let quoted = false;
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (quoted) {
      if (c === '"' && src[i + 1] === '"') {
        field += '"';
        i++;
      } else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"') quoted = true;
    else if (c === ",") {
      row.push(field);
      field = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && src[i + 1] === "\n") i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
    } else field += c;
  }
  if (field !== "" || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  const [head, ...body] = rows.filter((r) => r.length > 1);
  return body.map((r) => Object.fromEntries(head.map((h, i) => [h, (r[i] ?? "").trim()])));
}

/** One line of TSV: no tabs or line breaks inside a field. */
const cell = (s) => String(s).replace(/[\t\r\n]+/g, " ").replace(/ {2,}/g, " ").trim();

/**
 * What decides how a value is read: its kind, scale, reference and width.
 * The unit's spelling is not part of it: the two copies of one table write
 * `degree true` and `deg`, `umol kg-1` and `umol/kg`. What the unit does
 * decide is whether the value is scaled at all, since `Code table`, `Flag
 * table` and `CCITT IA5` are not. Both copies write words around the first
 * two, `Common Code table C-1`, and ecCodes writes them in capitals.
 */
function kind(unit) {
  const u = unit.toLowerCase();
  if (u.includes("code table")) return "Code table";
  if (u.includes("flag table")) return "Flag table";
  if (u === "ccitt ia5") return "CCITT IA5";
  return "numeric";
}

async function eccodesVersion(v) {
  let dir = String(v);
  let elements;
  try {
    elements = await text(`${ECCODES}/${dir}/element.table`);
  } catch {
    // A version ecCodes only links to another: the link's contents are the
    // name of the directory it points at.
    dir = (await text(`${ECCODES}/${dir}`)).trim();
    elements = await text(`${ECCODES}/${dir}/element.table`);
  }
  const b = new Map();
  for (const line of elements.split("\n").slice(1)) {
    const p = line.split("|");
    if (p.length < 8) continue;
    b.set(p[0], { name: p[3], unit: p[4], scale: Number(p[5]), reference: Number(p[6]), width: Number(p[7]) });
  }
  const d = new Map();
  const seq = await text(`${ECCODES}/${dir}/sequence.def`);
  for (const m of seq.matchAll(/"(\d{6})"\s*=\s*\[([^\]]*)\]/g)) {
    d.set(m[1], m[2].split(",").map((s) => s.trim()).filter((s) => s !== ""));
  }
  return { b, d, dir };
}

const same = (x, y) =>
  kind(x.unit) === kind(y.unit) && x.scale === y.scale && x.reference === y.reference && x.width === y.width;

/** An ecCodes name, which is in capitals, as a sentence. */
const sentence = (s) => (s.length === 0 ? s : s[0] + s.slice(1).toLowerCase());

// The current version, from the WMO.
const tableB = csv(await text(`${WMO}/BUFRCREX_TableB_en.txt`));
const tableC = csv(await text(`${WMO}/BUFR_TableC_en.txt`));
const tableD = csv(await text(`${WMO}/BUFR_TableD_en.txt`));

const elements = new Map();
for (const r of tableB) {
  elements.set(r.FXY, {
    name: r.ElementName_en,
    unit: r.BUFR_Unit,
    scale: Number(r.BUFR_Scale),
    reference: Number(r.BUFR_ReferenceValue),
    width: Number(r.BUFR_DataWidth_Bits),
  });
}
const sequences = new Map();
for (const r of tableD) {
  if (!sequences.has(r.FXY1)) {
    // Titles are written in brackets, and some sequences have none.
    sequences.set(r.FXY1, { title: r.Title_en.replace(/^\((.*)\)$/, "$1"), members: [] });
  }
  sequences.get(r.FXY1).members.push(r.FXY2);
}

// The WMO's current table against ecCodes' copy of it.
const current = await eccodesVersion(LATEST);
const disagree = [];
for (const [code, e] of elements) {
  const other = current.b.get(code);
  if (!other || !same(e, other)) disagree.push(`B ${code}`);
}
for (const code of current.b.keys()) if (!elements.has(code)) disagree.push(`B ${code} only in ecCodes`);
for (const [code, s] of sequences) {
  const other = current.d.get(code);
  if (!other || other.join(" ") !== s.members.join(" ")) disagree.push(`D ${code}`);
}
for (const code of current.d.keys()) if (!sequences.has(code)) disagree.push(`D ${code} only in ecCodes`);
if (disagree.length > 0) throw new Error(`WMO ${WMO_TAG} and ecCodes ${LATEST} disagree: ${disagree.join(", ")}`);

mkdirSync(OUT, { recursive: true });

let b = "code\tscale\treference\twidth\tunit\tname\n";
for (const [code, e] of elements) b += [code, e.scale, e.reference, e.width, cell(e.unit), cell(e.name)].join("\t") + "\n";
writeFileSync(join(OUT, "element.tsv"), b, "utf8");

let d = "code\ttitle\tmembers\n";
for (const [code, s] of sequences) d += [code, cell(s.title), s.members.join(" ")].join("\t") + "\n";
writeFileSync(join(OUT, "sequence.tsv"), d, "utf8");

let c = "code\tname\n";
for (const r of tableC) c += [r.FXY, cell(r.OperatorName_en)].join("\t") + "\n";
writeFileSync(join(OUT, "operator.tsv"), c, "utf8");

// The older versions, as what each says differently.
let older = "version\ttable\tcode\tscale\treference\twidth\tunit\tname\tmembers\n";
const counts = [];
for (const v of OLDER) {
  const old = await eccodesVersion(v);
  let nb = 0;
  let nd = 0;
  for (const [code, e] of old.b) {
    const now = current.b.get(code);
    if (now && same(e, now)) continue;
    // A name only where the current table has none to lend, and the current
    // spelling of the unit wherever the kind of value is the same.
    const wmo = elements.get(code);
    const name = wmo ? "" : sentence(e.name);
    const unit = wmo && kind(wmo.unit) === kind(e.unit) ? wmo.unit : kind(e.unit) === "numeric" ? e.unit : kind(e.unit);
    older += [v, "B", code, e.scale, e.reference, e.width, cell(unit), cell(name), ""].join("\t") + "\n";
    nb++;
  }
  for (const [code, members] of old.d) {
    const now = current.d.get(code);
    if (now && now.join(" ") === members.join(" ")) continue;
    older += [v, "D", code, "", "", "", "", "", members.join(" ")].join("\t") + "\n";
    nd++;
  }
  counts.push(`${v}: ${nb} elements and ${nd} sequences differ (ecCodes directory ${old.dir})`);
}
writeFileSync(join(OUT, "older.tsv"), older, "utf8");

const rs = `//! Which tables the TSV files beside this hold, generated by
//! \`node tools/bufrtables.mjs\`. Do not edit.
//!
//! The current version is the WMO's BUFR4 release ${WMO_TAG}, under its MIT
//! licence. The older versions' differences are from ecCodes at commit
//! \`${ECCODES_COMMIT}\`, under the Apache License 2.0.

/// The version \`element.tsv\`, \`sequence.tsv\` and \`operator.tsv\` are.
pub const LATEST: u32 = ${LATEST};

/// The versions whose differences from it \`older.tsv\` holds.
pub const OLDER: &[u32] = &[${OLDER.join(", ")}];

pub const ELEMENTS: &str = include_str!("element.tsv");
pub const SEQUENCES: &str = include_str!("sequence.tsv");
pub const OPERATORS: &str = include_str!("operator.tsv");
pub const OLDER_TSV: &str = include_str!("older.tsv");
`;
writeFileSync(join(OUT, "generated.rs"), rs, "utf8");

console.log(`Table B: ${elements.size} elements, ${b.length} bytes`);
console.log(`Table D: ${sequences.size} sequences, ${d.length} bytes`);
console.log(`Table C: ${tableC.length} operators, ${c.length} bytes`);
console.log(`Older versions, ${older.length} bytes:\n  ${counts.join("\n  ")}`);

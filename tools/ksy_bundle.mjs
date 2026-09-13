// Rewrite the generated half of `crates/core/src/ksy/bundled.rs` and the table
// in `crates/core/formats-ksy/README.md` from the `.ksy` files in that
// directory. Running this after copying a file in is the whole procedure.
//
// Two passes, because the second one needs a build that includes the first.
// The table of `include_str!`s is written from the directory listing alone, so
// a `.ksy` that was deleted stops being referenced before anything compiles.
// Then `cargo run --example ksy_gaps` converts the collection and says what
// each format became, and that fills in the signatures, the gap counts and the
// README.
//
//   node tools/ksy_bundle.mjs
//
// Nothing here decides what to bundle. That decision is made by reading the
// conversion report, and it is recorded in DROPPED below and in the README.

import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const KSY_DIR = join(ROOT, "crates", "core", "formats-ksy");
const BUNDLED_RS = join(ROOT, "crates", "core", "src", "ksy", "bundled.rs");
const README = join(KSY_DIR, "README.md");

// Candidates from HANDOVER-kaitai-gaplist.md that are NOT here, and why. The
// reason is the conversion report's, in one line; the gap list's own licence
// exclusions are separate, below.
const DROPPED = [
  ["android_sparse", "CC0-1.0", "the size of a chunk's header reads `_root.header`, which a field of the same name hides, so no chunk body is placed"],
  ["asn1_der", "CC0-1.0", "a sequence's body is another `asn1_der`, and the converter does not register a root type under its own name"],
  ["bson", "CC0-1.0", "an element's value is another `bson` document, and the converter does not register a root type under its own name"],
  ["dbf", "CC0-1.0", "a record's field widths come from `.length` over the header's field list, which the IR cannot take"],
  ["gettext_mo", "BSD-2-Clause", "the whole file's endianness is chosen from its first word, and the IR has no form for that"],
  ["msgpack", "CC0-1.0", "the root holds another `msgpack`, and the converter does not register a root type under its own name"],
  ["nitf", "MIT", "every sub-header's count is text read as a number, so twenty-one fields on the path through the file are left as bytes"],
  ["packet_ppi", "CC0-1.0", "the packet body switches to `packet_ppi` for PPI inside PPI, and the converter does not register a root type under its own name"],
  ["pcap", "CC0-1.0", "the whole file's endianness is chosen from its magic, and the IR has no form for that"],
  ["windows_resource_file", "CC0-1.0", "a resource's name is a `repeat-until` over `_`, which the IR cannot say, so everything after the name misplaces"],
];

// From the gap list. These never reach this repository.
const BY_LICENCE = [
  ["vdi", "GPL-3.0"],
  ["nt_mdt", "GPL-3.0"],
  ["broadcom_trx", "GPL-2.0"],
  ["lvm2", "GFDL-1.3"],
  ["pif", "LGPL-2.1"],
  ["renderware_binary_stream", "no licence tag"],
];

// Files copied in only because something else imports them. A root type that
// takes parameters is worked out rather than listed: nothing can open a file
// as one, so it is never offered whether or not anything imports it.
const IMPORT_ONLY = new Map([
  ["bytes_with_io", "imported by six formats that need a sub-stream; on its own it says the file is bytes, which is what no template says already"],
  ["pcx","imported by pcx_dcx; Qubero reads a PCX with its own template"],
  ["php_serialized_value", "imported by phar_without_stub; on its own, a mapping entry is another php_serialized_value, which the converter cannot say"],
  ["rtp_packet", "imported by rtpdump; a packet on the wire, with no file form of its own"],
  ["protocol_body", "imported by ipv4_packet and ipv6_packet; a dispatch table, not a file"],
]);

// A bundled format with gaps ships with the gaps showing, and this is the line
// that says why they are tolerable: the gaps have to be in a rare instance or
// a leaf value, never on the path through the file.
const WHY_WITH_GAPS = {
  android_bootldr_asus: "one instance naming a file inside the image is text, and the IR's expressions are integers",
  android_super: "three bit fields, all of them reserved padding, are read most-significant-bit first instead of least",
  compressed_resource: "one instance reads inside another field's stream; the header and the compressed run are placed",
  creative_voice_file: "three sample-rate instances are floating point; every block is placed",
  dcmp_0: "two instances of the decompressor's own bookkeeping; the compressed run is placed",
  dcmp_1: "two instances of the decompressor's own bookkeeping; the compressed run is placed",
  dcmp_variable_length_integer: "the one instance that reassembles the integer needs a bitwise or",
  dicom: "two tag numbers and one transfer-syntax test are instances; every data element is placed",
  dos_datetime: "the packed date and time read most-significant-bit first instead of least, and six instances only zero-pad the parts for display",
  ds_store: "three instances of the B-tree's bookkeeping; every record is placed",
  dune_2_pak: "one instance measures the last entry against the length of the file, which the IR has no expression for",
  edid: "every gap is an instance: nine are floating-point colour coordinates, eight reassemble a ten-bit number, one spells the manufacturer",
  ext2: "three instances reaching into the first block group; the superblock and the group descriptors are placed",
  hccap: "one instance re-reads the EAPOL buffer as its own stream",
  heroes_of_might_and_magic_agg: "one instance measures a field the IR does not let it measure; the entry table is placed",
  icc_4: "the 8-bit and 16-bit LUT tag types need an exclusive or for the size of their table, which costs those two tags' last two fields; the header and the tag table are placed",
  mac_os_resource_snd: "two sample-rate instances are floating point",
  mcap: "two instances, one measuring the file and one reading a value worked out later; every record is placed",
  mifare_classic: "seven instances: access-condition arithmetic and the value-block checks, all of which need bitwise operators",
  pcf_font: "three instances reading the string table as its own stream; every table is placed",
  phar_without_stub: "four instances read a decimal count out of text; the manifest and every entry are placed",
  resource_fork: "three instances reading the name table and the data blocks as their own streams; the map and the type list are placed",
  riff: "six instances re-reading a chunk's data as its own stream, and one spelling the chunk id; every chunk is placed",
  ruby_marshal: "the one instance that unpacks a small integer needs a bitwise complement",
  saints_row_2_vpp_pc: "two instances reading the name tables as their own streams; the entry table is placed",
  specpr: "two floating-point instances",
  ttf: "three instances reading a sub-table through its parent's stream; every table is placed",
  uefi_te: "one instance measures a field the IR does not let it measure",
  utf8_string: "the one instance that assembles a code point needs a bitwise or",
  vfat: "the packed date and time read most-significant-bit first instead of least, and six instances only zero-pad the parts for display",
  zisofs: "one instance with no `pos`, which has no place in the file the IR can name",
};

const ALLOWED = ["CC0-1.0", "MIT", "Unlicense", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause"];

// ---------- the directory ----------

function walk(dir, out = []) {
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (name.endsWith(".ksy")) out.push(path);
  }
  return out;
}

/** `meta/id`, `meta/title` and `meta/license`, off the top of the file. */
function meta(text) {
  const out = {};
  let inMeta = false;
  for (const line of text.split(/\r?\n/)) {
    if (line && !line.startsWith(" ") && !line.startsWith("#")) {
      inMeta = line.startsWith("meta:");
      continue;
    }
    if (!inMeta) continue;
    const m = /^ {2}(id|title|license): *(.*?) *$/.exec(line);
    if (m) out[m[1]] = m[2].replace(/^["'](.*)["']$/, "$1");
  }
  return out;
}

const files = walk(KSY_DIR).map((path) => {
  const source = relative(KSY_DIR, path).replaceAll("\\", "/");
  const text = readFileSync(path, "utf8");
  return { source, category: source.split("/")[0], ...meta(text) };
});

for (const f of files) {
  if (!f.id) throw new Error(`${f.source} has no meta/id`);
  if (!ALLOWED.includes(f.license)) throw new Error(`${f.source} is ${f.license}, which is not bundleable`);
}

// ---------- pass one: the include_str! table ----------

const rust = (s) => JSON.stringify(s);

function writeTable(rows) {
  const body = rows
    .map(
      (r) =>
        `\tBundled {\n` +
        `\t\tid: ${rust(r.id)},\n` +
        `\t\tname: ${rust("ksy:" + r.id)},\n` +
        `\t\tcategory: ${rust(r.category)},\n` +
        `\t\ttitle: ${rust(r.title ?? "")},\n` +
        `\t\tlicence: ${rust(r.license)},\n` +
        `\t\tsource: ${rust(r.source)},\n` +
        `\t\ttext: include_str!("../../formats-ksy/${r.source}"),\n` +
        `\t\toffered: ${r.offered ?? false},\n` +
        `\t\tsignature: ${r.signature ?? "&[]"},\n` +
        `\t\tsniffs: ${r.sniffs ?? false},\n` +
        `\t},\n`,
    )
    .join("");
  const text = readFileSync(BUNDLED_RS, "utf8");
  const begin = "// BEGIN GENERATED by tools/ksy_bundle.mjs -- do not edit by hand\n";
  const end = "// END GENERATED";
  const at = text.indexOf(begin);
  const to = text.indexOf(end, at);
  if (at < 0 || to < 0) throw new Error("bundled.rs has lost its generated markers");
  const table = `pub const BUNDLED: &[Bundled] = &[\n${body}];\n`;
  writeFileSync(BUNDLED_RS, text.slice(0, at + begin.length) + table + text.slice(to), "utf8");
}

writeTable(files);

// ---------- pass two: what each one converts to ----------

const json = execFileSync(
  "cargo",
  ["run", "-q", "-p", "qubero-core", "--example", "ksy_gaps", "--", "crates/core/formats-ksy"],
  { cwd: ROOT, encoding: "utf8", maxBuffer: 64 << 20 },
);
const report = new Map(JSON.parse(json).map((r) => [r.id, r]));

// A pattern claims a file no other bundled pattern could also claim. Two
// formats whose leading magic is the same are both dropped: picking either
// would be a guess, and a guess is what sniffing exists not to do.
function conflicts(a, b) {
  const fits = (x, y) => y.every(([at, hex]) => {
    const known = x.find(([xat, xhex]) => xat <= at && xat + xhex.length / 2 >= at + hex.length / 2);
    if (!known) return false;
    const from = (at - known[0]) * 2;
    return known[1].slice(from, from + hex.length) === hex;
  });
  return fits(a, b) || fits(b, a);
}

const decided = files.map((f) => {
  const r = report.get(f.id);
  if (!r) throw new Error(`ksy_gaps said nothing about ${f.id}`);
  const gaps = r.gaps ?? [];
  const offered = !r.error && !DROPPED.some(([id]) => id === f.id) && r.params === 0 && !IMPORT_ONLY.has(f.id);
  return { ...f, gaps: gaps.length, magics: r.magics ?? [], params: r.params, offered, error: r.error };
});

for (const d of decided) {
  const rivals = decided.filter((o) => o !== d && o.magics.length && conflicts(d.magics, o.magics));
  d.rivals = rivals.map((o) => o.id);
  d.sniffs = d.offered && d.magics.length > 0 && rivals.length === 0 && d.magics.reduce((n, m) => n + m[1].length / 2, 0) >= 2;
  d.signature = d.magics.length
    ? `&[${d.magics.map(([at, hex]) => `(${at}, &[${(hex.match(/../g) ?? []).map((b) => `0x${b}`).join(", ")}])`).join(", ")}]`
    : "&[]";
}

writeTable(decided);

// ---------- the README ----------

const why = new Map(Object.entries(WHY_WITH_GAPS));
function reason(d) {
  if (!d.offered) {
    if (d.params > 0) return `a root type with ${d.params} parameter${d.params === 1 ? "" : "s"}: nothing can open a file as it, and it is here to be imported`;
    return IMPORT_ONLY.get(d.id) ?? "here to be imported";
  }
  if (d.gaps === 0) return "converts whole";
  return why.get(d.id) ?? "GAPS WITH NO REASON WRITTEN -- add one to WHY_WITH_GAPS in tools/ksy_bundle.mjs";
}

const rows = decided
  .slice()
  .sort((a, b) => a.id.localeCompare(b.id))
  .map((d) => {
    const bundled = d.offered ? "yes" : "import";
    const sniff = d.sniffs ? `${d.magics.reduce((n, m) => n + m[1].length / 2, 0)} bytes` : d.magics.length ? "no, " + (d.rivals.length ? `same magic as ${d.rivals.join(", ")}` : "magic under 2 bytes") : "no magic";
    return `| \`${d.id}\` | ${d.category} | ${d.license} | ${bundled} | ${d.gaps} | ${sniff} | ${reason(d)} |`;
  });

const dropped = DROPPED.map(([id, lic, r]) => `| \`${id}\` | | ${lic} | no | | | ${r} |`);
const licence = BY_LICENCE.map(([id, lic]) => `| \`${id}\` | ${lic} |`);

const readme = `# Bundled Kaitai Struct formats

Verbatim copies of \`.ksy\` files from the [Kaitai Struct format
library](https://github.com/kaitai-io/kaitai_struct_formats), compiled into
\`qubero-core\` and converted on demand. \`crates/core/src/ksy/bundled.rs\` is the
table; this file is the record of which formats are here and why.

Both are generated. \`node tools/ksy_bundle.mjs\` rewrites them from the
directory, so adding a format is: copy the \`.ksy\` in, run the script, run the
tests. Nothing here is edited by hand except the reasons, which live in the
script.

Each file keeps its own licence, which is in its \`meta/license\` and again in
\`THIRD-PARTY-NOTICES.md\` beside the licence text and the URL it came from. The
only licences that may appear are ${ALLOWED.map((l) => `\`${l}\``).join(", ")}.

## What the columns say

* **Bundled** is \`yes\` for a format a reader can open a file as, and \`import\`
  for a file that is here because another one imports it, or whose root type
  takes parameters nobody can supply.
* **Gaps** is how many things in the \`.ksy\` the IR cannot say. The conversion
  report names every one of them, and the panel shows them. A test asserts this
  number, so the table cannot drift from the code.
* **Sniffs** is how much evidence a dropped file has to match for this format to
  be offered: the first \`seq\` field's \`contents\`, plus any further \`contents\`
  the walk reaches through fields of a fixed width. Built-in formats are asked
  first, always, so this only ever answers for a file no builtin claims.

## Bundled

| Format | Category | Licence | Bundled | Gaps | Sniffs | Why |
| --- | --- | --- | --- | --- | --- | --- |
${rows.join("\n")}

## Not bundled: the conversion does not reach the file

Candidates from \`HANDOVER-kaitai-gaplist.md\` whose report has gaps on the path
through the file: the root's own \`seq\`, or the type most of the file is made
of. A format that reads the front of a file and then loses the rest is worse
than no format at all, so these wait for the converter to grow.

| Format | Category | Licence | Bundled | Gaps | Sniffs | Why |
| --- | --- | --- | --- | --- | --- | --- |
${dropped.join("\n")}

## Not bundled: the licence

From the gap list. These are not copied into this repository at all.

| Format | Licence |
| --- | --- |
${licence.join("\n")}

## Notes

* ${decided.filter((d) => !d.title).length} of the ${decided.length} files carry no \`meta/title\`. Nothing invents one: the
  chooser shows those by their id.
* No bundled format's magic is a single byte, so none was dropped from sniffing
  for claiming too much on one byte. Two were dropped for sharing a magic with
  each other, and one more for reasons the table gives.
* \`ruby_marshal\` (\`04 08\`) and \`psx_tim\` (\`10 00 00 00\`) pass the two-byte rule
  with weak evidence: both are version numbers rather than a name. They sniff
  only for a file no builtin claims, which is the whole reason that is
  tolerable.
`;

writeFileSync(README, readme, "utf8");

console.log(`${decided.length} files, ${decided.filter((d) => d.offered).length} offered, ${decided.filter((d) => d.sniffs).length} sniffable, ${decided.reduce((n, d) => n + d.gaps, 0)} gaps`);

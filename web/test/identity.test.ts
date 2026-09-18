// Which answer names the file, when several sources have one. Each case is a
// file the editor has met: the numbers and sentences are the ones its sources
// gave.

import { test } from "node:test";
import assert from "node:assert/strict";

import { agrees, decide } from "../src/identity.ts";
import type { Identification, ToolMatch } from "../src/doc.ts";
import type { SigMatch, SigFormat } from "../src/signatures.ts";

const rule = (message: string, ext: string[] = [], mime = "", strength = 70): Identification => ({ message, mime, ext, strength, source: "x" });
const template = (name: string, label: string) => ({ name, label, sentence: null });
const sig = (label: string, fixed: number, extensionAgrees: boolean, source: SigFormat["source"] = "wikidata"): SigMatch => ({
  format: { id: "Q1", source, label, sigs: [] } as unknown as SigFormat,
  pattern: "",
  offset: 0,
  fromEnd: false,
  fixed,
  worth: fixed,
  extensionAgrees,
  score: fixed + (extensionAgrees ? 4 : 0),
});

test("a template that read the file outranks a rule that disagrees", () => {
  const id = decide({ template: template("parquet", "Parquet file"), file: rule("PARity archive data - file number 5376", [], "application/octet-stream", 80) });
  assert.equal(id.name, "Parquet file");
  assert.equal(id.source, "template");
  assert.equal(id.candidates[1]?.source, "file");
  assert.equal(id.candidates[1]?.disagrees, true);
});

test("a rule that agrees with the template names the file, since it says more", () => {
  const id = decide({ template: template("pe", "Windows executable"), file: rule("PE32+ executable for MS Windows 6.00 (console), x86-64, 5 sections", ["exe", "dll"], "application/vnd.microsoft.portable-executable") });
  assert.equal(id.name, "PE32+ executable for MS Windows 6.00 (console), x86-64, 5 sections");
  assert.equal(id.source, "file");
  assert.equal(id.candidates[0]?.source, "file");
  assert.equal(id.candidates.some((c) => c.disagrees), false);
});

test("agreement comes from extensions, media type, or a naming word", () => {
  assert.ok(agrees(rule("Zip archive data", ["zip"]), "zip", "ZIP archive"));
  assert.ok(agrees(rule("EPUB document", ["epub"], "application/epub+zip"), "zip", "ZIP archive"));
  assert.ok(agrees(rule("Hierarchical Data Format (version 5) data", ["h5", "hdf5"], "application/x-hdf"), "hdf5", "HDF5"));
  assert.ok(agrees(rule("NetCDF Data Format data", [], ""), "netcdf", "NetCDF classic"));
  assert.ok(!agrees(rule("PARity archive data", [], "application/octet-stream"), "parquet", "Parquet file"));
  // The rules call an OMF module with a five-byte first record a XENIX object.
  assert.ok(agrees(rule("XENIX 8086 relocatable or i286 small model", [], "application/octet-stream", 50), "omf", "OMF object"));
  // "data" and "archive" are in every other sentence and name nothing.
  assert.ok(!agrees(rule("Par archive data", [], ""), "tar", "tar archive"));
});

test("a weak template yields to the rules", () => {
  const id = decide({ template: template("zlib", "zlib stream"), file: rule("Zstandard compressed data (v0.8+), Dictionary ID: None", ["zst"]) });
  assert.equal(id.source, "file");
});

test("a weak template does not yield to a rule that compared two bytes", () => {
  // `80 05` opens every protocol 5 pickle and is the whole of the XENIX rule.
  const xenix: Identification = { message: "XENIX 8086 relocatable or i286 small model", mime: "application/octet-stream", ext: [], strength: 50, source: "xenix" };
  const id = decide({ template: template("pickle", "Python pickle file"), file: xenix });
  assert.equal(id.name, "Python pickle file");
  assert.equal(id.source, "template");
  const listed = id.candidates[1];
  assert.equal(listed?.source, "file");
  assert.equal(listed?.evidence, "file(1) rule in xenix, strength 50");
  assert.equal(listed?.disagrees, true);
  // Four bytes is a magic number, and the template yields as it did.
  assert.equal(decide({ template: template("pickle", "Python pickle file"), file: { ...xenix, strength: 70 } }).source, "file");
});

test("a rule that only half matched loses to the template, and the comma goes", () => {
  const id = decide({ template: template("godot", "Godot binary resource/scene"), file: rule("National Instruments,", []) });
  assert.equal(id.name, "Godot binary resource/scene");
  assert.equal(id.candidates[1]?.name, "National Instruments");
});

test("with no template the rules name the file, then the tool, then a signature", () => {
  assert.equal(decide({ template: null, file: rule("GIF image data, version 89a, 8 x 8", ["gif"]) }).source, "file");
  const upx: ToolMatch = { category: "packer", name: "UPX", version: "3.96", options: null, source: "com.sig" };
  const byTool = decide({ template: null, file: null, tools: [upx] });
  assert.equal(byTool.name, "UPX v3.96 (packer)");
  assert.equal(byTool.source, "tools");
  const bySig = decide({ template: null, file: null, tools: [], signatures: [sig("Fuchsia archive format", 8, true)] });
  assert.equal(bySig.name, "Fuchsia archive format");
  assert.equal(bySig.source, "signature");
  assert.equal(decide({ template: null, file: null, tools: [], signatures: [sig("Any XML", 1, true)] }).name, null);
});

test("a template's own sentence beats everything", () => {
  const id = decide({ template: { name: "claudetheme", label: "Claude Code theme", sentence: 'Claude Code theme "Ember", based on dark' }, file: rule("JSON text data", ["json"], "application/json") });
  assert.equal(id.name, 'Claude Code theme "Ember", based on dark');
});

test("a BGZF file is named by what it holds, and file(1)'s container sentence still agrees", () => {
  const bam = { name: "bgzf", label: "BGZF gzip blocks", sentence: "BAM alignments \u00b7 compressed with BGZF" };
  const id = decide({ template: bam, file: rule("Blocked GNU Zip Format (BGZF; gzip compatible), block length 502", [], "application/x-gzip") });
  assert.equal(id.name, "BAM alignments \u00b7 compressed with BGZF");
  assert.equal(id.source, "template");
  assert.equal(id.candidates.some((c) => c.disagrees), false);
  // A rule that calls it plain gzip agrees too, by the extension.
  const vcf = { ...bam, sentence: "VCF variant calls \u00b7 compressed with BGZF" };
  const gz = decide({ template: vcf, file: rule("gzip compressed data, extra field", ["gz", "tgz"], "application/gzip") });
  assert.equal(gz.name, vcf.sentence);
  assert.equal(gz.candidates.some((c) => c.disagrees), false);
  // And with nothing to say about the contents, the container is the name.
  const none = decide({ template: { ...bam, sentence: null }, file: rule("Blocked GNU Zip Format (BGZF; gzip compatible), block length 28", [], "application/x-gzip") });
  assert.equal(none.source, "file");
});

test("nothing answered yet is no name and no candidates", () => {
  const id = decide({ template: null });
  assert.equal(id.name, null);
  assert.deepEqual(id.candidates, []);
});

test("a template applied over a signature that does not match says so, and disagrees", () => {
  // Picking PNG for a ZIP: the fields were read, the file is not a PNG, and
  // the answer has to say which of the two happened.
  const png = { name: "png", label: "PNG image", sentence: null, signatureMismatch: true };
  const id = decide({ template: png, file: rule("Zip archive data, at least v2.0 to extract", ["zip"], "application/zip") });
  // The rules name the file, since the template did not recognise it; the
  // template's answer is listed under that with what is wrong.
  assert.equal(id.source, "file");
  const listed = id.candidates.find((c) => c.source === "template");
  assert.equal(listed?.evidence, "Template PNG image was applied, but the signature does not match");
  assert.equal(listed?.disagrees, true);
  // With nothing else having answered the template is all there is, and it
  // still disagrees: what it disagrees with is the file's own first bytes.
  const alone = decide({ template: png });
  assert.equal(alone.source, "template");
  assert.equal(alone.candidates[0]?.disagrees, true);
  // A template whose signature matched reads as it always did.
  const ok = decide({ template: { ...png, signatureMismatch: false } });
  assert.equal(ok.candidates[0]?.evidence, "Qubero read the file's structure");
  assert.equal(ok.candidates[0]?.disagrees, false);
});

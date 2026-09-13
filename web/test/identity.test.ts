// Which answer names the file, when several sources have one. Each case is a
// file the editor has met: the numbers and sentences are the ones its sources
// gave.

import { test } from "node:test";
import assert from "node:assert/strict";

import { agrees, decide } from "../src/identity.ts";
import { templateTypeName } from "../src/filetype.ts";
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
  // "data" and "archive" are in every other sentence and name nothing.
  assert.ok(!agrees(rule("Par archive data", [], ""), "tar", "tar archive"));
});

test("a weak template yields to the rules", () => {
  const id = decide({ template: template("zlib", "zlib stream"), file: rule("Zstandard compressed data (v0.8+), Dictionary ID: None", ["zst"]) });
  assert.equal(id.source, "file");
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

test("nothing answered yet is no name and no candidates", () => {
  const id = decide({ template: null });
  assert.equal(id.name, null);
  assert.deepEqual(id.candidates, []);
});

test("a template's type name gets 'file' only when its label does not already say what it is", () => {
  assert.equal(templateTypeName("parquet"), "Parquet file");
  assert.equal(templateTypeName("zip"), "ZIP archive");
  assert.equal(templateTypeName("png"), "PNG image");
  assert.equal(templateTypeName("sqlite"), "SQLite database");
  assert.equal(templateTypeName("utmp"), "Login records");
  assert.equal(templateTypeName("pe"), "Windows PE executable");
});

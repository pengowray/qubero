// The file signature database: reading the Wikidata values as editors wrote
// them, and matching the patterns against bytes. The values in the cleaning
// tests are real ones from Wikidata, each the first of its kind.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { cleanPattern, canonicalPronom, ENC } from "../../tools/wikidata/patterns.mjs";
import { infoboxExtensions } from "../../tools/wikidata/infobox.mjs";
import {
  buildIndex,
  compile,
  compileAll,
  decode,
  extensionOf,
  fixedBytes,
  matchAt,
  matchFormats,
  matchFormatsSlowly,
  namingMatch,
  type SigData,
  type SigFormat,
} from "../src/signatures.ts";

const bytes = (s: string): Uint8Array => new TextEncoder().encode(s);
const hex = (h: string): Uint8Array => Uint8Array.from(h.match(/../g) ?? [], (x) => parseInt(x, 16));

test("plain hex is used as written", () => {
  assert.deepEqual(cleanPattern("1F8B08", [ENC.hex]), { pattern: "1F8B08", note: null });
});

test("hex with spaces, 0x or lowercase is tidied", () => {
  assert.equal((cleanPattern("00 61 73 6d", [ENC.hex]) as { pattern: string }).pattern, "0061736D");
  assert.equal((cleanPattern("0x0507", [ENC.hex]) as { pattern: string }).pattern, "0507");
  assert.equal((cleanPattern("4455434b", [ENC.hex]) as { pattern: string }).pattern, "4455434B");
});

test("an odd number of hex digits is left out rather than guessed at", () => {
  assert.ok("skip" in cleanPattern("415649204", [ENC.hex]));
});

test("text is its bytes, unless it is hex tagged as text", () => {
  assert.equal((cleanPattern("8BCB", [ENC.ascii]) as { pattern: string }).pattern, "38424342");
  assert.equal((cleanPattern("AC1012", [ENC.ascii]) as { pattern: string }).pattern, "414331303132");
  assert.equal((cleanPattern("41 54 43 4F 2D 43 49 46", [ENC.ascii]) as { pattern: string }).pattern, "4154434F2D434946");
  assert.ok("skip" in cleanPattern("L.......", [ENC.ascii]));
});

test("C escapes are read, with no encoding given", () => {
  assert.equal((cleanPattern("\\211HDF\\r\\n\\032\\n", []) as { pattern: string }).pattern, "894844460D0A1A0A");
});

test("a GUID is written the way Windows stores one", () => {
  const r = cleanPattern("00021401-0000-0000-c000-000000000046", [ENC.hex, ENC.guid]) as { pattern: string };
  assert.equal(r.pattern, "0114020000000000C000000000000046");
});

test("a simple regular expression becomes a signature, and anything more is left out", () => {
  assert.equal((cleanPattern("RIFF.{4}WEBPVP8\\x20", [ENC.pcre2]) as { pattern: string }).pattern, "52494646{4}5745425056503820");
  assert.ok("skip" in cleanPattern("\\.ofip$", [ENC.posixEre]));
  assert.ok("skip" in cleanPattern("^RIFF[0-9]+", [ENC.pcre2]));
});

test("PRONOM syntax is checked", () => {
  assert.equal(canonicalPronom("1A45DFA3{0-32}4282847765626D4287"), "1A45DFA3{0-32}4282847765626D4287");
  assert.equal(canonicalPronom("2525454F(46|460A|460D0A)"), "2525454F(46|460A|460D0A)");
  assert.equal(canonicalPronom("255044462D312E[30:37]"), "255044462D312E[30:37]");
  assert.equal(canonicalPronom("4D5A*50450000"), "4D5A*50450000");
  assert.equal(canonicalPronom("4D5"), null);
  assert.equal(canonicalPronom("(4D|"), null);
});

test("infobox extensions come from any shape of wikitext", () => {
  const gif = "{{Infobox file format\n| icon =\n| extension = {{code|.gif}}\n<!-- or: | extensions = -->\n| mime = {{code|image/gif}}\n}}";
  assert.deepEqual(infoboxExtensions(gif), ["gif"]);
  const eps = "{{Infobox file format\n| name = EPS\n| extension = {{Plain list|\n* .eps\n* .epsf\n}}\n| mime = x\n}}";
  assert.deepEqual(infoboxExtensions(eps), ["eps", "epsf"]);
  const ref = "{{Infobox file format | extension = <code>.mp3</code><ref>See [[x|.nope]]</ref>, .bit }}";
  assert.deepEqual(infoboxExtensions(ref), ["bit", "mp3"]);
  assert.equal(infoboxExtensions("No infobox here."), null);
});

test("gaps, alternatives and ranges match", () => {
  const t = compile("52494646{4}57454250");
  assert.ok(matchAt(t, bytes("RIFF\x10\x20\x30\x40WEBPVP8 "), 0) >= 0);
  assert.equal(matchAt(t, bytes("RIFF\x10\x20\x30WEBP"), 0), -1);
  assert.ok(matchAt(compile("4D5A*50450000"), hex("4D5A0000FFFF50450000"), 0) >= 0);
  assert.equal(matchAt(compile("255044462D312E[30:37]"), bytes("%PDF-1.8"), 0), -1);
  assert.ok(matchAt(compile("255044462D312E[30:37]"), bytes("%PDF-1.7"), 0) >= 0);
  // The first option fits but leaves no line ending for the rest, so the second is tried.
  assert.equal(matchAt(compile("2525454F(460A|46)0A"), bytes("%%EOF\n"), 0), 6);
});

test("a match counts only the bytes it pins down", () => {
  assert.equal(fixedBytes(compile("52494646{4}57454250")), 8);
  assert.equal(fixedBytes(compile("2525454F(46|460A|460D0A)")), 5);
  assert.equal(fixedBytes(compile("0000{6}01(01|04|08)")), 4);
});

const fmt = (id: string, label: string, sigs: SigFormat["sigs"], ext?: string[]): SigFormat =>
  ({ source: "wikidata", id, label, sigs, ...(ext ? { ext } : {}) });
const indexOf = (data: SigData): ReturnType<typeof buildIndex> => buildIndex(compileAll(data));

test("the longest match comes first, with an agreeing extension counting for more", () => {
  const data: SigData = {
    fetched: "",
    formats: [
      fmt("Q1", "Any XML", [["3C", 0]]),
      fmt("Q2", "XML with a prolog", [["3C3F786D6C", 0]], ["xml"]),
      fmt("Q3", "Some other XML", [["3C3F786D6C", 0]], ["foo"]),
      fmt("Q4", "PDF", [["2525454F46", 1024, "eof"]], ["pdf"]),
    ],
  };
  const index = indexOf(data);
  const xml = bytes('<?xml version="1.0"?><a/>');
  const found = matchFormats(index, { head: xml, tail: xml, name: "thing.foo" });
  assert.deepEqual(found.map((m) => m.format.id), ["Q3", "Q2", "Q1"]);
  assert.equal(namingMatch(found)?.format.id, "Q3");
  // Nothing breaks the tie without an extension, so nothing names the file.
  assert.equal(namingMatch(matchFormats(index, { head: xml, tail: xml, name: "thing" })), null);

  const pdf = bytes("%PDF-1.4\n...\n%%EOF\r\n");
  assert.deepEqual(matchFormats(index, { head: pdf, tail: pdf, name: "a.pdf" }).map((m) => m.format.id), ["Q4"]);
});

test("a few bytes name a file only with its extension behind them", () => {
  const index = indexOf({
    fetched: "",
    formats: [fmt("Q1", "Any XML", [["3C", 0]], ["xml"]), fmt("Q2", "Compress", [["1F9D", 0]], ["z"]), fmt("Q3", "LiteDB", [["00000000", 0]])],
  });
  const one = (head: Uint8Array, name: string): string | undefined => namingMatch(matchFormats(index, { head, tail: head, name }))?.format.id;
  assert.equal(one(bytes("<a/>"), "a.xml"), undefined);
  assert.equal(one(hex("1F9D90"), "words.Z"), "Q2");
  assert.equal(one(hex("1F9D90"), "words"), undefined);
  assert.equal(one(hex("0000000001"), "test.mat"), undefined);
});

test("the extension is what follows the last dot", () => {
  assert.equal(extensionOf("archive.tar.GZ"), "gz");
  assert.equal(extensionOf(".bashrc"), "");
  assert.equal(extensionOf("README"), "");
});

test("the index gives the same answer as running every pattern", () => {
  // Two formats sharing a pattern, one format with two patterns that pin the
  // same number of bytes, patterns at an offset, the shapes the index cannot
  // hold, and one measured from the end.
  const data: SigData = {
    fetched: "",
    formats: [
      fmt("Q1", "Any XML", [["3C", 0]]),
      fmt("Q2", "XML with a prolog", [["3C3F786D6C", 0]], ["xml"]),
      fmt("Q3", "Also that prolog", [["3C3F786D6C", 0]]),
      fmt("Q4", "Two ways, same length", [["3C3F786D6C", 0], ["3C3F786D60", 0]]),
      fmt("Q5", "RIFF with a gap", [["52494646{4}57454250", 0]]),
      fmt("Q6", "A version range", [["255044462D312E[30:37]", 0]]),
      fmt("Q7", "Deep in the file", [["57415645", 8]]),
      fmt("Q8", "Alternatives", [["2525454F(46|460A)", 0]]),
      fmt("Q9", "From the end", [["2525454F46", 1024, "eof"]], ["pdf"]),
      fmt("Q10", "Never matches", [["FFFFFFFFFF", 0]]),
    ],
  };
  const compiled = compileAll(data);
  const index = buildIndex(compiled);
  const files: readonly { head: Uint8Array; name: string }[] = [
    { head: bytes('<?xml version="1.0"?>'), name: "a.xml" },
    { head: bytes('<?xml version="1.0"?>'), name: "a" },
    { head: bytes("<html>"), name: "page.html" },
    { head: bytes("RIFF\x10\x20\x30\x40WEBPVP8 "), name: "a.webp" },
    { head: bytes("RIFF\x10\x20\x30\x40WAVEfmt "), name: "a.wav" },
    { head: bytes("%PDF-1.4\nbody\n%%EOF\r\n"), name: "a.pdf" },
    { head: bytes("%%EOF\n"), name: "odd" },
    { head: bytes(""), name: "empty" },
    { head: hex("3C3F786D60"), name: "near.xml" },
  ];
  for (const { head, name } of files) {
    const file = { head, tail: head, name };
    assert.deepEqual(matchFormats(index, file), matchFormatsSlowly(compiled, file), name);
  }
});

test("every pattern in the shipped database compiles", () => {
  const path = new URL("../public/signatures.json", import.meta.url);
  const data = decode(JSON.parse(readFileSync(path, "utf8")));
  const compiled = compileAll(data);
  assert.ok(compiled.length > 9000);
  assert.ok(data.formats.some((f) => f.source === "file"));
  assert.ok(data.formats.some((f) => f.source === "wikidata"));
});

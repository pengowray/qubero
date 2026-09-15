// Searching for a template by name, extension and signature bytes.

import { test } from "node:test";
import assert from "node:assert/strict";

import { compile, compileAll, type SigData } from "../src/signatures.ts";
import { parseCString, parseHex, signatureHolds } from "../src/bytequery.ts";
import { magicPattern, parseQuery, searchTemplates, spacedPattern, type TemplateEntry } from "../src/templatesearch.ts";

const hex = (h: string): number[] => h.match(/../g)?.map((x) => parseInt(x, 16)) ?? [];

const data: SigData = {
  fetched: "",
  formats: [
    { source: "wikidata", id: "Q178051", label: "Portable Network Graphics", ext: ["png"], mime: ["image/png"], sigs: [["89504E470D0A1A0A", 0], ["49454E44AE426082", 0, "eof"]] },
    { source: "wikidata", id: "Q1", label: "Waveform Audio File Format", ext: ["wav"], sigs: [["52494646{4}57415645", 0]] },
    { source: "file", id: "archive:1", label: "Zip archive data", ext: ["zip"], sigs: [["504B0304", 0]] },
    { source: "wikidata", id: "Q2", label: "Inno Setup installer", ext: ["exe"], sigs: [["4D5A*496E6E6F", 0]] },
    { source: "wikidata", id: "Q3", label: "Java class file", ext: ["class"], sigs: [["CAFEBABE", 0]] },
    { source: "wikidata", id: "Q4", label: "PDF", ext: ["pdf"], sigs: [["2525454F46", 1024, "eof"]] },
    { source: "wikidata", id: "Q5", label: "ISO base media file", ext: ["mp4"], sigs: [["66747970", 4]] },
  ],
};
const sigs = compileAll(data);

const entries: TemplateEntry[] = [
  { value: "png", label: "PNG image", kind: "builtin", ext: ["png"], sigs: [] },
  { value: "zip", label: "ZIP archive", kind: "builtin", ext: ["zip", "jar"], sigs: [] },
  { value: "pe", label: "Windows executable", kind: "builtin", ext: ["exe", "dll"], sigs: [] },
  { value: "ksy:gif", label: "GIF (Graphics Interchange Format) image", kind: "kaitai", ext: ["gif"], sigs: [{ pattern: "474946", offset: 0 }] },
];

const names = (q: string): string[] => searchTemplates(q, entries, sigs).templates.map((h) => h.entry.value);
const sigNames = (q: string): string[] => searchTemplates(q, entries, sigs).signatures.map((h) => h.format.label);

test("hex is read with or without spaces and 0x", () => {
  assert.deepEqual([...(parseHex("89 50 4E 47") ?? [])], hex("89504e47"));
  assert.deepEqual([...(parseHex("89504e47") ?? [])], hex("89504e47"));
  assert.deepEqual([...(parseHex("0x89 0x50") ?? [])], hex("8950"));
  assert.equal(parseHex("895"), null);
  assert.equal(parseHex("png"), null);
});

test("C strings take escapes, and a broken escape is not read as a letter", () => {
  assert.deepEqual([...(parseCString("\\x89PNG") ?? [])], hex("89504e47"));
  assert.deepEqual([...(parseCString("PK\\3\\4") ?? [])], hex("504b0304"));
  assert.deepEqual([...(parseCString("\\r\\n\\0") ?? [])], [13, 10, 0]);
  assert.equal(parseCString("\\x"), null);
  assert.equal(parseCString("\\q"), null);
});

test("quotes force text, spaces force hex, and * and $ place the bytes", () => {
  assert.deepEqual(parseQuery('"cafe"').bytes.map((b) => [...b.bytes]), [[0x63, 0x61, 0x66, 0x65]]);
  assert.equal(parseQuery('"cafe"').word, "");
  assert.equal(parseQuery("89 50").word, "");
  assert.equal(parseQuery("89 50").bytes.length, 1);
  const cafe = parseQuery("cafe");
  assert.equal(cafe.word, "cafe");
  assert.equal(cafe.bytes.length, 2);
  assert.equal(parseQuery("*57 41 56 45").bytes[0]?.where, "anywhere");
  assert.equal(parseQuery("49 45 4E 44$").bytes[0]?.where, "end");
  assert.equal(parseQuery(".exe").ext, "exe");
  assert.equal(parseQuery(".exe").word, "");
});

test("an extension finds templates with or without the dot", () => {
  assert.deepEqual(names(".exe"), ["pe"]);
  assert.deepEqual(names("exe"), ["pe"]);
  assert.deepEqual(names("jar"), ["zip"]);
});

test("hex at the start reaches a built-in through a signature sharing its extension", () => {
  const r = searchTemplates("89 50 4E 47", entries, sigs);
  assert.deepEqual(r.templates.map((h) => h.entry.value), ["png"]);
  const reason = r.templates[0]?.reason;
  assert.equal(reason?.by, "bytes");
  // The signature entry for PNG is the template's twin, so it is not listed again.
  assert.deepEqual(r.signatures, []);
});

test("a C string finds the same bytes hex does", () => {
  assert.deepEqual(names("\\x89PNG"), ["png"]);
  assert.deepEqual(names("PK\\x03\\x04"), ["zip"]);
});

test("bytes match from the start by default, not in the middle or at the end", () => {
  assert.deepEqual(names("50 4E 47"), []);
  assert.deepEqual(names("49 45 4E 44"), []);
  assert.deepEqual(names("49 45 4E 44$"), ["png"]);
  assert.deepEqual(sigNames("57 41 56 45"), []);
  assert.deepEqual(sigNames("*57 41 56 45"), ["Waveform Audio File Format"]);
});

test("a plain word is bytes only when a signature holds all of it", () => {
  assert.deepEqual(sigNames("PKzz"), []);
  assert.deepEqual(names("PK"), ["zip"]);
  assert.deepEqual(names("PK\\x03\\x04zz"), ["zip"]);
});

test("a Kaitai format's own magic is searched", () => {
  assert.deepEqual(names("GIF"), ["ksy:gif"]);
  assert.deepEqual(names("47 49"), ["ksy:gif"]);
});

test("a format with no template comes back as a signature only", () => {
  const r = searchTemplates("cafe", entries, sigs);
  assert.deepEqual(r.templates, []);
  assert.deepEqual(r.signatures.map((h) => [h.format.label, h.format.source]), [["Java class file", "wikidata"]]);
  assert.equal(r.signatures[0]?.reason.by, "bytes");
});

test("a signature that shares an extension but is another format stays listed", () => {
  const r = searchTemplates("exe", entries, sigs);
  assert.deepEqual(r.templates.map((h) => h.entry.value), ["pe"]);
  assert.deepEqual(r.signatures.map((h) => h.format.label), ["Inno Setup installer"]);
});

test("a name that starts with the query ranks above one that only holds it", () => {
  const r = searchTemplates("image", entries, sigs);
  assert.deepEqual(r.templates.map((h) => h.entry.value), ["ksy:gif", "png"]);
  assert.deepEqual(names("png"), ["png"]);
  assert.equal(searchTemplates("png", entries, sigs).templates[0]?.reason.by, "name");
});

test("a pattern at offset 4 pins the query bytes that reach it", () => {
  const tokens = compile("66747970");
  assert.equal(signatureHolds(tokens, 4, false, Uint8Array.from(hex("000000186674")), "start"), 2);
  assert.equal(signatureHolds(tokens, 4, false, Uint8Array.from(hex("6674")), "start"), 0);
  assert.equal(signatureHolds(tokens, 4, false, Uint8Array.from(hex("6674")), "anywhere"), 2);
});

test("a gap lets the query run on past the pattern's known bytes", () => {
  const tokens = compile("4D5A*496E6E6F");
  assert.equal(signatureHolds(tokens, 0, false, Uint8Array.from(hex("4d5a9000")), "start"), 2);
  assert.equal(signatureHolds(tokens, 0, false, Uint8Array.from(hex("496e6e6f")), "anywhere"), 4);
  assert.equal(signatureHolds(compile("2525454F46"), 1024, true, Uint8Array.from(hex("2525454f46")), "start"), 0);
  assert.equal(signatureHolds(compile("2525454F46"), 1024, true, Uint8Array.from(hex("454f46")), "end"), 3);
});

test("patterns are shown as spaced bytes, cut short", () => {
  assert.equal(spacedPattern("89504E470D0A1A0A"), "89 50 4E 47 0D 0A 1A 0A");
  assert.equal(spacedPattern("52494646{4}57415645"), "52 49 46 46 {4} 57 41 56 …");
  assert.equal(spacedPattern("4D5A*49"), "4D 5A … 49");
});

test("Kaitai's scattered magic becomes one pattern with gaps", () => {
  assert.deepEqual(magicPattern([[0, "52494646"], [8, "57415645"]]), { pattern: "52494646{4}57415645", offset: 0 });
  assert.deepEqual(magicPattern([]), null);
});

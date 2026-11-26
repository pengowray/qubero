// The report's reading of the core's data: the ledger's lines, the landmarks
// from the format profile, the window the extent figure draws, which wheel
// events a figure takes, and whether a directory's targets are in order.

import { test } from "node:test";
import assert from "node:assert/strict";

import { factRows } from "../src/report/bodies.ts";
import type { LedgerRow, Profile, ProfileRow } from "../src/report/coredata.ts";
import { sameOrder } from "../src/report/directories.ts";
import { extentWindow } from "../src/report/extentfigure.ts";
import { isGapLine, ledgerLines, fieldRows } from "../src/report/ledger.ts";
import { runPosition } from "../src/report/partrules.ts";
import { landmarks, readingWriting, rowLabel } from "../src/report/profiletext.ts";
import { RV } from "../src/report/text.ts";
import { codecWords, DECODED_REFUSED, DECODED_REFUSED_TITLE, REPORT } from "../src/strings.ts";
import { formatLimit } from "../src/format.ts";
import { SETTLE_MS, takesWheel } from "../src/report/wheel.ts";

test("text inside a list of numbers is counted with its nouns, one and many", () => {
  assert.equal(RV.findingTextNum(1), "1 list of numbers that contains text");
  assert.equal(RV.findingTextNum(3), "3 lists of numbers that contain text");
  assert.equal(RV.textNumReason(12, 338, "i16 le"), "12 runs of text, 338 bytes in total, in a list the template reads as i16 le numbers");
  assert.equal(RV.textNumReason(1, 29, "f32 be"), "1 run of text, 29 bytes, in a list the template reads as f32 be numbers");
  assert.equal(RV.textNumFirst(5, 12), "First 5 of 12 runs of text");
  assert.equal(RV.textNumFirst(3, 3), "3 runs of text");
});

function row(part: number[], partName: string, group: string, role: LedgerRow["role"], bits: number, first: number[], firstOffset: number, extra: Partial<LedgerRow> = {}): LedgerRow {
  return {
    part,
    part_name: partName,
    group,
    group_from: group === "" ? "none" : "key",
    role,
    bits,
    count: 1,
    zero_bits: 0,
    unscanned_bits: 0,
    first_path: first,
    first_offset_bits: firstOffset,
    align: 0,
    ...extra,
  };
}

test("a part's groups are a line each, and their roles are summed under them", () => {
  const lines = ledgerLines([
    row([0], "records", "local file", "content", 800, [0, 0, 1], 32),
    row([0], "records", "local file", "machinery", 80, [0, 0, 0], 0),
    row([0], "records", "central directory file", "content", 400, [0, 11, 1], 1000),
  ]);
  assert.equal(lines.length, 2);
  const local = lines[0];
  assert.equal(local?.group, "local file");
  assert.equal(local?.bits, 880);
  // The line starts where its first bytes are, whichever role they have.
  assert.equal(local?.firstOffsetBits, 0);
  assert.deepEqual(local?.roles.map((r) => r.role), ["content", "machinery"]);
});

test("a structure named by its type inside another group's element is counted in that group's line", () => {
  // A JPEG's dht segment holds its Huffman tables.
  const lines = ledgerLines([
    row([1], "segments", "dht, huffman tables", "machinery", 128, [1, 4, 0], 1416),
    row([1], "segments", "HuffmanTable", "content", 2816, [1, 4, 1, 1, 0, 0], 1448, { group_from: "type" }),
    row([1], "segments", "HuffmanTable", "machinery", 512, [1, 4, 1, 1, 0, 2, 0], 1456, { group_from: "type" }),
    row([1], "segments", "sof0, baseline dct", "content", 40, [1, 3, 1, 1, 0], 1300),
  ]);
  assert.deepEqual(lines.map((l) => l.group), ["sof0, baseline dct", "dht, huffman tables"]);
  assert.equal(lines[1]?.bits, 128 + 2816 + 512);
  assert.equal(lines[1]?.roles.find((r) => r.role === "machinery")?.bits, 640);
});

test("a group named by a value inside another group's element keeps its own line", () => {
  // A MIDI track's meta events are named by their status byte.
  const lines = ledgerLines([
    row([1], "file", "MTrk", "machinery", 192, [1, 0], 112),
    row([1], "file", "meta", "content", 520, [1, 2, 0, 0], 176),
  ]);
  assert.deepEqual(lines.map((l) => l.group), ["MTrk", "meta"]);
});

test("a group says which part it is in only where groups are in more than one part", () => {
  // A ZIP file's records are all in one list, and "in records" says nothing.
  const zip = ledgerLines([
    row([0], "records", "local file", "content", 800, [0, 0, 1], 32),
    row([0], "records", "central directory file", "content", 400, [0, 11, 1], 1000),
  ]);
  assert.deepEqual(zip.map((l) => l.partShown), [false, false]);
  // A MIDI file is one list, the root, and "in file" would read as "in the file".
  const midi = ledgerLines([row([], "file", "MThd", "machinery", 64, [0, 0], 0), row([], "file", "meta", "content", 520, [1, 2, 0, 0], 176)]);
  assert.deepEqual(midi.map((l) => l.partShown), [false, false]);
  // A SQLite file has groups in page1 and in pages.
  const sqlite = ledgerLines([
    row([24], "page1", "SchemaRecord", "content", 776, [24, 6, 0, 2], 3256, { group_from: "case" }),
    row([27], "pages", "Overflow", "content", 800, [27, 1, 0], 8192, { group_from: "case" }),
    row([0], "magic", "", "content", 128, [0], 0),
  ]);
  assert.deepEqual(sqlite.map((l) => [l.group, l.partShown, l.byType]), [
    ["", false, false],
    ["SchemaRecord", true, true],
    ["Overflow", true, true],
  ]);
});

test("a count and its verb agree", () => {
  assert.equal(RV.directorySummary(1, 1, 1), ": 1 entry points to 1 place in the file");
  assert.equal(RV.directorySummary(4, 4, 4), ": 4 entries point to 4 places in the file");
  assert.equal(RV.directorySummary(1, 7, 1), ": 1 of its 7 entries points to 1 place in the file");
  assert.equal(RV.ledgerUnlisted(1), "1 more part after these was not listed.");
  assert.equal(landmarks(profile([], { lengths_before: 1 })).at(-1), "1 field gives the length or count of a later field.");
});

test("a table's facts go by their own field's name, and numbers get separators and units", () => {
  const rows = factRows([
    { label: "body.sample_rate", path: [1, 1, 3], value: "500000", unit: "Hz" },
    { label: "body.format", path: [1, 1, 0], value: "pcm", unit: "" },
    // Two facts that would share a name keep the structures they came through.
    { label: "fmt.size", path: [1, 0, 1], value: "16", unit: "" },
    { label: "data.size", path: [1, 1, 1], value: "300000", unit: "" },
    // A FITS card is named by its keyword, and its unit is from its comment.
    { label: "TBIN", path: [0, 2, 0, 41], value: "6.4E-05", unit: "s" },
  ]);
  assert.deepEqual(rows.map((r) => r.name), ["sample_rate", "format", "fmt.size", "data.size", "TBIN"]);
  assert.equal(rows[0]?.value, `${(500000).toLocaleString()} Hz`);
  assert.equal(rows[1]?.value, "pcm");
  // In seconds, it is said in the unit that suits it, as any length of time is.
  assert.equal(rows[4]?.value, "64 µs");
});

test("a table's heading in the report says how long its rows last", () => {
  assert.equal(RV.tableHeading(12_000, "sample", 8000), "12,000 samples at 8,000 a second, 1.5 s");
  assert.equal(RV.tableHeading(26_460_000, "sample", 44_100), "26,460,000 samples at 44,100 a second, 10 min");
});

test("the kinds a file does not use are counted by the table's two halves", () => {
  assert.equal(RV.profileUnused(23, 0), "The template also declares 23 kinds of value that this file does not use.");
  assert.equal(RV.profileUnused(21, 2), "The template also declares 21 kinds of value and 2 ways to find a field or set its length that this file does not use.");
  assert.equal(RV.profileUnused(0, 1), "The template also declares 1 way to find a field or set its length that this file does not use.");
});

test("the plain fields of one structure are one line, and one field alone keeps its name", () => {
  const lines = ledgerLines([
    row([7, 0], "type", "", "content", 16, [7, 0], 128),
    row([7, 1], "machine", "", "machinery", 16, [7, 1], 144),
    row([7, 2], "version", "", "content", 32, [7, 2], 160),
    row([0], "magic", "", "content", 32, [0], 0),
  ]);
  assert.equal(lines.length, 2);
  const header = lines.find((l) => l.fields.length === 3);
  assert.deepEqual(header?.parentPath, [7]);
  assert.equal(header?.part, null);
  assert.equal(header?.bits, 64);
  const magic = lines.find((l) => l.fields.length === 1);
  assert.equal(magic?.part, "magic");
});

test("a stream at the root is a part of its own, not one of the root's plain fields", () => {
  // A Tascam firmware update: a header, then the scrambled image and the
  // scrambled checksum copy, each one leaf of the file.
  const lines = ledgerLines([
    row([1], "image", "", "content", 203_948_000, [1], 256, { stream: true }),
    row([0], "header", "", "content", 256, [0, 0], 0),
    row([2], "checksum_copy", "", "content", 32, [2], 203_948_256, { stream: true }),
  ]);
  assert.deepEqual(lines.map((l) => l.part).sort(), ["checksum_copy", "header", "image"]);
  assert.ok(lines.every((l) => l.parentPath === null));
});

/** What the report calls each line, the way `lineLabel` in bytes.ts does for
 *  a line of plain fields at the top of the file. */
function lineNames(rows: LedgerRow[], fileBits: number): string[] {
  return ledgerLines(rows).map((l) => l.part ?? REPORT.unnamedPart(runPosition({ offsetBits: l.firstOffsetBits, sizeBits: l.bits }, fileBits)));
}

test("plain fields on either side of a stream are separate lines, each named by where it is", () => {
  // compressed/hello.zz: the zlib header's bit fields, the deflate stream,
  // then the Adler-32 at the end of the file.
  const names = lineNames(
    [
      row([6], "compressed", "", "content", 544, [6], 16, { stream: true }),
      row([7], "adler32", "", "content", 32, [7], 560),
      row([4], "fcheck", "", "content", 5, [4], 11),
      row([0], "cinfo", "", "content", 4, [0], 0),
      row([1], "cm", "", "content", 4, [1], 4),
      row([2], "flevel", "", "content", 2, [2], 8),
      row([3], "fdict", "", "machinery", 1, [3], 10),
    ],
    592,
  );
  assert.deepEqual(names, ["Header", "compressed", "adler32"]);
});

test("plain fields after the lists of a file are a trailer, not part of the header", () => {
  // git/pack-v2-thirteen-objects.idx: magic and version, four lists, then
  // two SHA-1 checksums at the end of the file.
  const names = lineNames(
    [
      row([2], "fanout", "", "machinery", 8192, [2, 0], 64),
      row([3], "names", "", "content", 2080, [3, 0], 0x408 * 8),
      row([4], "crcs", "", "content", 416, [4, 0], 0x50c * 8),
      row([5], "offsets", "", "content", 416, [5, 0], 0x540 * 8),
      row([7], "pack_checksum", "", "content", 160, [7], 0x574 * 8),
      row([8], "checksum", "", "content", 160, [8], 0x588 * 8),
      row([0], "magic", "", "content", 32, [0], 0),
      row([1], "version", "", "content", 32, [1], 32),
    ],
    0x59c * 8,
  );
  assert.deepEqual(names, ["Header", "fanout", "names", "crcs", "offsets", "Trailer"]);
});

test("plain fields with a gap between them are separate lines", () => {
  const lines = ledgerLines([
    row([0], "magic", "", "content", 32, [0], 0),
    row([1], "version", "", "content", 32, [1], 32),
    row([], "file", "", "gap", 64, [], 64),
    row([2], "count", "", "content", 32, [2], 128),
    row([3], "flags", "", "content", 32, [3], 160),
  ]);
  assert.deepEqual(
    lines.filter((l) => !isGapLine(l)).map((l) => l.fields),
    [
      ["magic", "version"],
      ["count", "flags"],
    ],
  );
});

test("a JSON object's braces are on the object's own line", () => {
  // json/jsontestsuite-unicode-escapes.json, whose root is the one part.
  const lines = ledgerLines([
    row([], "file", "", "content", 856, [0], 8),
    row([], "file", "", "framing", 16, [], 0x6c * 8),
    row([], "file", "", "gap", 8, [], 0),
  ]);
  const file = lines.filter((l) => !isGapLine(l));
  assert.equal(file.length, 1);
  assert.equal(file[0]?.part, "file");
  assert.deepEqual(file[0]?.roles.map((r) => r.role), ["content", "framing"]);
});

test("bytes no field describes are one line, with their zero bytes kept", () => {
  const lines = ledgerLines([
    row([7], "header", "", "gap", 800, [7], 400, { zero_bits: 800 }),
    row([3], "tail", "", "gap", 80, [3], 9000, { zero_bits: 0 }),
  ]);
  assert.equal(lines.length, 1);
  const gap = lines[0];
  assert.ok(gap !== undefined && isGapLine(gap));
  assert.equal(gap?.roles[0]?.zeroBits, 800);
  assert.equal(gap?.bits, 880);
});

function profile(rows: Partial<ProfileRow>[], facts: Partial<Profile["facts"]> = {}): Profile {
  return {
    done: true,
    rows: rows.map((r) => ({ category: "number", kind: "unsigned", width: 16, order: "big", detail: "", fields: 1, bits: 16, unpacked_fields: 0, unpacked_bits: 0, ...r })),
    choices: [],
    facts: { from_end: 0, placed: 0, forward: 0, backward: 0, lengths_before: 0, lengths_after: 0, every_field_follows: null, ...facts },
  };
}

test("the landmarks say the byte order and the widths the file uses", () => {
  const p = profile([
    { width: 16, order: "big" },
    { width: 32, order: "big" },
    { width: 8, order: "none" },
    { width: 64, order: "big", fields: 0 },
  ]);
  const said = landmarks(p);
  assert.equal(said[0], "Big-endian.");
  assert.equal(said[1], "Integers are 8, 16, and 32 bits.");
});

test("the landmarks say what each kind of codec did, and leave out the ones that change nothing", () => {
  const codec = (kind: string, detail: string): Partial<ProfileRow> => ({ category: "codec", kind, detail, width: 0, order: "" });
  const said = landmarks(
    profile([
      codec("tascam byte table", "scrambling"),
      codec("iar lz77", "compression"),
      codec("png scanlines", "filter"),
      codec("stored", "stored"),
      codec("base64", "encoding"),
      codec("low bits argb", "steganography"),
    ]),
  );
  assert.deepEqual(said, ["Compressed with iar lz77.", "Encoded with base64.", "Scrambled with tascam byte table.", "Data hidden with low bits argb."]);
  // Rows from a core that sent no kind keep the old sentence.
  assert.deepEqual(landmarks(profile([codec("deflate", ""), codec("png scanlines", "")])), ["Compressed with deflate."]);
});

test("the streams section names its streams by what their codec does", () => {
  const scrambled = codecWords("scrambling");
  const compressed = codecWords("compression");
  assert.equal(RV.streamsHeading(1, 1, 4, compressed), "1 compressed stream: 1 byte unpacks to 4 bytes");
  assert.equal(RV.streamHeading("data", 100, "deflate", 380, compressed), "data: 100 bytes of deflate unpack to 380 bytes");
  // Streams of more than one kind.
  assert.equal(RV.streamsHeading(3, 10, 40, null), "3 streams: 10 bytes unpack to 40 bytes");
  assert.equal(RV.streamUnpacked(null), "Unpacked");
  assert.equal(RV.streamUnpacked(scrambled), "Unscrambled");
  assert.equal(RV.stagesCaption(100, 380, compressed), "Each bar is drawn to scale. The unpacked data is 3.8 times the size of the compressed data.");
  assert.equal(RV.stagesCaption(61, 47, codecWords("encoding")), "Each bar is drawn to scale. The decoded data is 77% of the size of the encoded data.");
  assert.equal(RV.unpacksTo("tascam byte table", "4 bytes", scrambled), "tascam byte table, unscrambles to 4 bytes");
});

test("a scrambled stream that comes out the same size has its size said once, with its codec", () => {
  const scrambled = codecWords("scrambling");
  // One size over more than one stream is their total.
  assert.equal(RV.streamsHeading(2, 25_493_504, 25_493_504, scrambled), "2 scrambled streams: 25,493,504 bytes in all");
  assert.equal(RV.streamsHeading(2, 25_493_504, null, scrambled), "2 scrambled streams: 25,493,504 bytes in all");
  assert.equal(RV.streamsHeading(1, 4, 4, scrambled), "1 scrambled stream: 4 bytes");
  assert.equal(RV.streamsHeading(2, 9_000, null, codecWords("compression")), "2 compressed streams: 9,000 bytes in all");
  assert.equal(RV.streamHeading("checksum_copy", 4, "tascam byte table", 4, scrambled), "checksum_copy: 4 bytes, scrambled with tascam byte table");
  assert.equal(RV.streamHeading("image", 4, "tascam byte table", null, scrambled), "image: 4 bytes, scrambled with tascam byte table");
});

test("a stream of another kind that happens to come out the same size says both sizes", () => {
  const compressed = codecWords("compression");
  assert.equal(RV.streamsHeading(1, 4_096, 4_096, compressed), "1 compressed stream: 4,096 bytes unpack to 4,096 bytes");
  assert.equal(RV.streamHeading("body", 12, "deflate", 12, compressed), "body: 12 bytes of deflate unpack to 12 bytes");
  assert.equal(RV.streamHeading("body", 12, "base64", 12, codecWords("encoding")), "body: 12 bytes of base64 decode to 12 bytes");
  // Streams of more than one kind.
  assert.equal(RV.streamsHeading(2, 10, 10, null), "2 streams: 10 bytes unpack to 10 bytes");
});

test("the format profile says how many of a codec's streams it measured", () => {
  assert.equal(RV.unpackedTo(2, 2, "25,493,504 bytes", "scrambling"), "2 streams, unscrambled to 25,493,504 bytes in all");
  assert.equal(RV.unpackedTo(1, 1, "9 bytes", null), "1 stream, unpacked to 9 bytes");
  assert.equal(RV.unpackedTo(2, 3, "1,024 bytes", "compression"), "3 streams, 2 of them unpacked to 1,024 bytes in all");
  assert.equal(RV.unpackedTo(1, 3, "1,024 bytes", "compression"), "3 streams, 1 of them unpacked to 1,024 bytes");
});

test("a stored stream is extracted, and says both sizes where they differ", () => {
  const stored = codecWords("stored");
  assert.equal(RV.streamsHeading(1, 29, 29, stored), "1 stored stream: 29 bytes");
  assert.equal(RV.streamsHeading(3, 87, 87, stored), "3 stored streams: 87 bytes in all");
  assert.equal(RV.streamHeading("data", 29, "stored", 29, stored), "data: 29 bytes, stored");
  assert.equal(RV.streamHeading("data", 29, "stored", null, stored), "data: 29 bytes, stored");
  // An OLE file's Workbook stream is stored, and smaller than the whole
  // compound file it is gathered from.
  assert.equal(RV.streamsHeading(1, 45_056, 12_288, stored), "1 stored stream: 45,056 bytes extract to 12,288 bytes");
  assert.equal(RV.streamHeading("workbook", 45_056, "cfb-workbook", 12_288, stored), "workbook: 45,056 bytes of cfb-workbook extract to 12,288 bytes");
  assert.equal(RV.unpacksTo("cfb-workbook", "12,288 bytes", stored), "cfb-workbook, extracts to 12,288 bytes");
  assert.equal(RV.unpackedTo(1, 1, "12,288 bytes", "stored"), "1 stream, extracted to 12,288 bytes");
  assert.equal(RV.streamUnpacked(stored), "Extracted");
  assert.equal(RV.stageUnpacked(stored), "Extracted");
  assert.equal(RV.stagesCaption(45_056, 12_288, stored), "Each bar is drawn to scale. The extracted data is 27% of the size of the stored data.");
});

test("a stream the report did not draw says which limit it passed, on which size", () => {
  const limits = { size: "4.00 MiB", count: 32, total: "16.0 MiB" };
  assert.equal(RV.streamNotDrawnNote("size", limits), "This stream takes up more than 4.00 MiB of the file, so this stream is not drawn. Open this stream in a tab to read it.");
  assert.equal(RV.streamNotDrawnNote("count", limits), "The report draws at most 32 streams, so this stream is not drawn. Open this stream in a tab to read it.");
  assert.equal(
    RV.streamNotDrawnNote("total", limits),
    "The report draws at most 16.0 MiB of streams in all, counting the space each stream takes up in the file, so this stream is not drawn. Open this stream in a tab to read it.",
  );
});

test("a stream that would not decode says why, with its codec's verb", () => {
  assert.equal(
    RV.streamFailedNote(codecWords("scrambling"), "tascam byte table", "failed"),
    "The report could not unscramble this stream: the tascam byte table decoder returned an error.",
  );
  assert.equal(RV.streamFailedNote(codecWords("compression"), "zstd", "too-large"), "The report could not unpack this stream: too large to unpack (over 64 MiB).");
  assert.equal(
    RV.streamFailedNote(codecWords("compression"), "zstd", "too-large-zstd-frame"),
    "The report could not unpack this stream: a zstd frame is too large to unpack (over 64 MiB).",
  );
  assert.equal(RV.streamFailedNote(codecWords("stored"), "cfb-workbook", "failed"), "The report could not extract this stream: the cfb-workbook decoder returned an error.");
  // No reason from the core, or one this build does not know.
  assert.equal(RV.streamFailedNote(codecWords("compression"), "deflate", null), "The report could not unpack this stream.");
  assert.equal(RV.streamFailedNote(codecWords("compression"), "deflate", "new-reason"), "The report could not unpack this stream.");
  assert.equal(RV.streamNotMeasured, "not measured");
});

test("a finding for a stream left as bytes says it is too large past either limit", () => {
  assert.equal(RV.refused("too-large", "32 MiB"), "Too large to unpack here");
  assert.equal(RV.refused("too-large-lazy", "32 MiB"), "Too large to unpack here");
  assert.equal(RV.refused("too-large-zstd-frame", "32 MiB"), "Too large to unpack here");
  assert.equal(RV.refused("too-large-xz-block", "32 MiB"), "Too large to unpack here");
  assert.equal(
    RV.refused("too-much-at-once", "32 MiB"),
    "Takes up over 32 MiB of the file, more than the editor keeps in memory at once, so it was not unpacked",
  );
  assert.equal(RV.refused("unaligned", "32 MiB"), "Does not start on a byte boundary, so it was not unpacked");
  assert.equal(RV.refused("failed", "32 MiB"), "Did not unpack");
});

test("a limit is written as a whole number of its unit, and a refusal that leaves it out says it on hover", () => {
  assert.equal(formatLimit(32 * 2 ** 20), "32 MiB");
  assert.equal(formatLimit(192 * 2 ** 10), "192 KiB");
  assert.equal(formatLimit(2 ** 30), "1 GiB");
  assert.equal(formatLimit(1536 * 2 ** 10), "1.50 MiB");
  assert.equal(DECODED_REFUSED["too-much-at-once"], "too large for the editor to keep in memory at once");
  assert.equal(DECODED_REFUSED_TITLE["too-much-at-once"]?.("32 MiB"), "The editor keeps at most 32 MiB of the file in memory.");
  assert.equal(DECODED_REFUSED_TITLE["too-large"], undefined);
});

test("the ribbon's sentences count no codes where there is not one, and say what writes no bytes", () => {
  assert.equal(RV.ribbonKinds.match, "A copy repeats earlier bytes, so its band widens toward the output bytes.");
  assert.equal(RV.ribbonKinds.stored, "A stored run is copied to the output as it is.");
  assert.equal(RV.ribbonKinds.ignored, "Whitespace produces no bytes, so its band narrows to a point.");
  assert.match(RV.ribbonIgnoredQp, /^A soft line break \(an = sign at the end of a line\)/);
});

test("both byte orders are said with their counts", () => {
  const said = landmarks(profile([{ order: "big", fields: 3 }, { order: "little", fields: 5 }]));
  assert.equal(said[0], "Both byte orders: 5 little-endian numbers and 3 big-endian numbers.");
});

test("a reader who can go straight through is told so, and a writer what to know", () => {
  const said = readingWriting(profile([], { every_field_follows: true, lengths_before: 4 }));
  assert.deepEqual(said.map((s) => s.summary), ["Readable in a single pass", "4 fields whose length or count is stored before them"]);
  assert.equal(said[0]?.detail, "A program that reads this format can read this file from start to end without seeking back.");
  assert.match(said[1]?.detail ?? "", /^A program that writes this format from start to end has to know the length or count of each of those fields before it writes the field/);
});

test("the lines about offsets and lengths agree with a count of one", () => {
  const said = readingWriting(profile([], { from_end: 1, backward: 1, forward: 1, lengths_before: 1, lengths_after: 1 }));
  assert.deepEqual(said.map((s) => s.summary), [
    "1 field whose position is worked out from the end of the file",
    "1 field that a backward offset points to",
    "1 field that a forward offset points to",
    "1 field whose length or count is stored before it",
    "1 field whose length or count is stored after it",
  ]);
  assert.match(said[0]?.detail ?? "", /before it can find that field\.$/);
  assert.match(said[1]?.detail ?? "", /seek back to read that field\. A program that writes this format can write the field first/);
  assert.match(said[2]?.detail ?? "", /has to know the position of that field before it writes the offset/);
  assert.match(said[3]?.detail ?? "", /has to know the field's length or count before it writes the field/);
  assert.match(said[4]?.detail ?? "", /can write the field first and then its length or count\.$/);
});

test("the lines about offsets and lengths agree with a count above one", () => {
  const said = readingWriting(profile([], { from_end: 2, backward: 3, forward: 4, lengths_after: 5 }));
  assert.deepEqual(said.map((s) => s.summary), [
    "2 fields whose position is worked out from the end of the file",
    "3 fields that backward offsets point to",
    "4 fields that forward offsets point to",
    "5 fields whose length or count is stored after them",
  ]);
  assert.match(said[0]?.detail ?? "", /before it can find those fields\.$/);
  assert.match(said[1]?.detail ?? "", /seek back to read those fields\./);
});

test("profile rows read as plain words", () => {
  assert.equal(rowLabel({ category: "number", kind: "unsigned", width: 16, order: "little", detail: "", fields: 1, bits: 0, unpacked_fields: 0, unpacked_bits: 0 }), "16-bit unsigned integer, little-endian");
  assert.equal(rowLabel({ category: "text", kind: "terminated", width: 0, order: "", detail: "utf8", fields: 1, bits: 0, unpacked_fields: 0, unpacked_bits: 0 }), "UTF-8, ends at a terminator");
  assert.equal(rowLabel({ category: "sizing", kind: "length-field", width: 0, order: "", detail: "", fields: 1, bits: 0, unpacked_fields: 0, unpacked_bits: 0 }), "a length worked out from other fields");
});

test("ends far apart are drawn from the part's start", () => {
  const w = extentWindow(0, [1000, 800]);
  assert.equal(w.lo, 0);
  assert.equal(w.cut, false);
});

test("ends close together at the end of a long part are drawn near the ends, at one scale", () => {
  // The pipistrelle WAV: 301,020 bytes stated from 12, and a file of 301,024.
  const start = 96;
  const ends = [start + 2408160, 2408192, start + 2408096];
  const w = extentWindow(start, ends);
  assert.equal(w.cut, true);
  assert.ok(w.lo > start);
  assert.ok(w.lo < Math.min(...ends));
  assert.ok(w.hi > Math.max(...ends));
});

test("a figure takes the wheel only once the page is still", () => {
  assert.equal(takesWheel(SETTLE_MS + 1, false, false), true);
  assert.equal(takesWheel(SETTLE_MS - 1, false, false), false);
});

test("a figure zoomed all the way out lets a zoom-out through to the page", () => {
  assert.equal(takesWheel(10_000, true, true), false);
  assert.equal(takesWheel(10_000, false, true), true);
  assert.equal(takesWheel(10_000, true, false), true);
});

test("a part's rows leave out a field as big as the part and count neighbours of one name", () => {
  const kid = (name: string, at: number, size: number) => ({ name, path: [at], offset_bits: at, size_bits: size });
  const runs = fieldRows({ offsetBits: 0, sizeBits: 400 }, [kid("body", 0, 400), kid("[0] dht", 0, 100), kid("[1] dht", 100, 100), kid("[2] dqt", 200, 200)]);
  assert.deepEqual(runs.map((r) => [r.name, r.count, r.startBit, r.endBit]), [["dht", 2, 0, 200], ["dqt", 1, 200, 400]]);
});

test("targets in the order of their entries are in order, and any step back is not", () => {
  assert.equal(sameOrder([0, 10, 10, 40]), true);
  assert.equal(sameOrder([40, 10]), false);
});

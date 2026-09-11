// Document facade: owns the wasm Editor and streams chunks in from a File/Blob.
// Nothing here ever reads the whole file; only the chunks the view asks for.

import init, { Editor, dump_scan, dump_bytes, glyph_column, text_encode } from "./pkg/qubero_wasm.js";
import { ADDRESS_MARK, formatBytes, formatOffset, offsetDigits } from "./format.ts";
import type { GlyphSet } from "./hexcell.ts";
export { ADDRESS_MARK, byteText, formatBytes, formatOffset, offsetDigits, percentText } from "./format.ts";
import { UNPACKED } from "./strings.ts";

const CHUNK_SIZE = 64 * 1024;
/** How many rounds of fetch-and-ask-again a read of text is given. Each round
 *  is one window of file, so this is a batch of lines spanning a few tens of
 *  megabytes: further than any screenful reaches, and short of forever. */
const TEXT_ROUNDS = 512;
/** Chunks to fetch past one the template asked for and did not have. Placing
 * fields runs forward through a file, so the chunk after the missing one is
 * nearly always the next one wanted. They are read in one go: asking the file
 * for 64 KiB at a time costs a round trip each time, and reading a file's
 * worth of metadata that way is hundreds of them. */
const READ_AHEAD = 48;
const CHUNK_CAPACITY = 512; // 32 MiB resident at most

// What the file(1) rules get to look at. Their offsets count from the start of
// the file, so this is always the head and never a window from elsewhere. Rules
// that search rather than test a fixed offset stop where it stops: the widest
// in the database reaches 16 KiB, and a handful measure from the end of the
// file, which nothing here can answer.
const IDENTIFY_WINDOW = 64 * 1024;

/** The OME-NGFF keys that distinguish an OME-Zarr metadata JSON document from
 * an ordinary Zarr store record. */
function isOmeZarrMetadata(name: string, bytes: Uint8Array): boolean {
  const leaf = name.replace(/\\/g, "/").split("/").pop()?.toLowerCase();
  if (leaf !== ".zattrs" && leaf !== "zarr.json") return false;
  const text = new TextDecoder().decode(bytes);
  return /"(?:multiscales|ome)"\s*:/.test(text);
}

/** Formats with nothing in the bytes to recognise them by, where the file name
 * is the only evidence there is. A `.c16` capture is raw samples from the first
 * byte: no header, no magic number, nothing but the extension to go on. A
 * `.ppma` does open with `P3` and is usually recognised by that; the mapping is
 * here for one written without the newline the check expects. A `.mat` from
 * before MATLAB 5 has no header either, and while the sniffer does recognise
 * one by the five integers it opens with, a file those integers do not fit
 * still opens as what it is called. */
const BY_EXTENSION: Record<string, string> = { c16: "c16", ppma: "pnm", mat: "mat" };

function templateByExtension(name: string): string | null {
  const ext = name.toLowerCase().split(".").pop() ?? "";
  return BY_EXTENSION[ext] ?? null;
}

type MagicModule = typeof import("./pkg-magic/qubero_magic.js");
let magic: Promise<MagicModule> | null = null;

/** The rule database, fetched the first time a file needs it and kept after. */
function loadMagic(): Promise<MagicModule> {
  magic ??= import("./pkg-magic/qubero_magic.js")
    .then(async (m) => {
      await m.default();
      return m;
    })
    .catch((e: unknown) => {
      // A failed fetch is forgotten rather than remembered, so opening the next
      // file tries again instead of inheriting one bad moment offline.
      magic = null;
      throw e;
    });
  return magic;
}

/**
 * Fetch the rule database before a file asks for it, so the wait lands on an
 * idle browser rather than on someone watching. Called once a file is open and
 * the page has settled.
 *
 * It costs a megabyte to someone who only ever opens formats the editor has a
 * template for, which is why it waits for idle and stops on a connection that
 * would rather not: Save Data on, or anything the browser rates below 4G.
 * Those cases still identify on demand, and say they are doing it.
 */
export function prefetchMagic(): void {
  if (magic !== null) return;
  const link = (navigator as Navigator & { connection?: NetworkInformation }).connection;
  if (link?.saveData === true) return;
  if (link?.effectiveType !== undefined && link.effectiveType !== "4g") return;
  const start = (): void => {
    // Nothing waits on this. A failure clears itself in `loadMagic`, so the
    // first file that needs the rules asks for them again.
    void loadMagic().catch(() => {});
  };
  // Safari only got requestIdleCallback in 16.4, so this checks rather than
  // assumes. The timeout stops a busy page putting it off forever.
  const idle: unknown = window.requestIdleCallback;
  if (typeof idle === "function") window.requestIdleCallback(start, { timeout: 5000 });
  else window.setTimeout(start, 2000);
}

/** Rule bundles already fetched, so a second file of the same kind is free. */
const ruleCache = new Map<string, Promise<string | null>>();

/** One bundle of signature rules, or null when it cannot be had. */
function fetchRules(name: string): Promise<string | null> {
  let p = ruleCache.get(name);
  if (p === undefined) {
    p = fetch(`diesig/${name}`)
      .then((r) => (r.ok ? r.text() : null))
      .catch(() => null);
    ruleCache.set(name, p);
  }
  return p;
}

/** The parts of the Network Information API we use; Safari has none of it. */
type NetworkInformation = { saveData?: boolean; effectiveType?: string };

/** The subset of Blob we need; lets tests and dev tooling supply synthetic files. */
export type ByteSource = {
  readonly size: number;
  readonly name: string;
  slice(start: number, end: number): { arrayBuffer(): Promise<ArrayBuffer> } | Blob;
};

/** A field picked in one of the structure views: which node, and the bits
 *  it covers. Every view that lists fields hands one out and the rest of
 *  the app follows it, so it belongs beside the tree the views read rather
 *  than in whichever of them happens to be showing. */
export type FieldPick = { readonly path: readonly number[]; readonly startBit: number; readonly endBit: number };

export type TemplateNode = {
  readonly path: readonly number[];
  readonly name: string;
  readonly type: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly value: string;
  /** What the in-place editor starts with; differs from `value` for enums. */
  readonly edit_text: string;
  /** `unset` is a number holding the value its format writes for a slot
   *  nobody filled in; `edit_text` is still the number underneath it. */
  readonly kind: "uint" | "int" | "float" | "bytes" | "unread" | "str" | "insn" | "magic" | "enum" | "flags" | "unset" | "composite";
  readonly ok: boolean;
  readonly child_count: number;
  /** What one child is called, for counting them. Absent when they are items. */
  readonly unit?: string;
  readonly composite: boolean;
  /** True when `writeNode` accepts typed text for this field. */
  readonly editable: boolean;
  /** True when the template says this structure is one row rather than one row
   * per field: a value with parts, not a part of the file. */
  readonly inline: boolean;
  /** Bytes the value occupies: short of the field's size when text is padded
   * or terminated, since neither belongs to the value. */
  readonly value_bytes: number;
  /** Where the value starts, past a byte-order mark if the field has one. */
  readonly value_offset_bits: number;
  /** How the encoding was settled, or that the bytes do not fit it. */
  readonly read_as: string | null;
  /** Which sibling this field settles the length, count, type or position of,
   * as an index among the parent's children. Null for a field no sibling
   * reads, which is most of them. This is the fact the listing folds a
   * structure's machinery on; whether to fold is the view's decision, since
   * a field and the field it places can end up in different sections. */
  readonly consumed_by: number | null;
  /** What the template says over the top of that: true for machinery, false
   * for payload, null when it has no opinion. */
  readonly machinery: boolean | null;
  /** True when this field is only its parent's contents. A ZIP entry is a
   * signature and a `body`; giving `body` a heading of its own spends a level
   * of structure on the word "body". */
  readonly contents: boolean;
  /** True when the node's own bytes include punctuation its children do not
   *  account for: the braces of a JSON object, the brackets of an array. Its
   *  children tile what is between them, and what is left over is the node's
   *  own syntax rather than bytes nothing describes. */
  readonly framed: boolean;
  /** Which address space `offset_bits` counts in. 0 is the file. Anything
   *  else is the bytes a compressed stream came to, and the offset is counted
   *  from the front of those rather than from the front of the file. */
  readonly space: number;
  /** For a compressed run that would not open, why not: `too-large`,
   *  `failed` or `unaligned`. Null for every other field. */
  readonly refused: string | null;
  /** True for a compressed run. One that opened can be opened as a document of
   *  its own; one that did not says why in `refused` instead. */
  readonly decoded: boolean;
  /** True for the one node a stream holds, whose parent is the stream. There is
   *  one per space and it is always drawn, which the stream itself need not be,
   *  so this is where the listing offers Open unpacked. */
  readonly space_root: boolean;
};

/** The bit range a successful `writeNode` replaced. */
/** One entry of the annotation column: a field, a run of them, or a stretch
 *  the template does not describe. */
export type Span = {
  readonly path: number[];
  readonly name: string;
  /** What it sits inside, outermost first. */
  readonly trail: string[];
  readonly type: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly value: string;
  readonly kind: string;
  readonly gap: boolean;
  /** Fields this entry stands for, when a large run is shown as one. */
  readonly count: number;
  /** What one of those is called, singular, when the format has a word for it:
   *  a deflate block holds symbols. Null when they read as values. */
  readonly unit: string | null;
  /** A structure that reads on one row, already joined: an instruction rather
   *  than its opcode and its immediate. Null for a field that reads as its own
   *  value. */
  readonly line: string | null;
  /** The first few values of a run shown as one entry. */
  readonly sample: string[];
  /** First element extents of a collapsed run, then its remaining extent. */
  readonly parts: readonly { readonly size_bits: number; readonly label: string; readonly rest: boolean }[];
  /** How a variable-length number's bits divide into framing and value. Null
   *  for a field that reads as whole bytes, which is most of them. Which bits
   *  are which is decode knowledge and comes from the core; only the words
   *  beside the split are the view's. */
  readonly bits: BitRoles | null;
  /** These bytes are a document of their own: a compressed run that unpacked,
   *  or the contents of one. What the annotation column marks so a reader
   *  running down the bytes can see there is a file in front of them. */
  readonly opens: boolean;
};

/** One element of a folded run, for the value table beside the bytes. A span
 *  with a count stands for a whole run; these are the elements of it that a
 *  stretch of rows actually covers. */
export type Cell = {
  /** Where the element sits in its run: the last step of its path. */
  readonly index: number;
  readonly offset_bits: number;
  readonly size_bits: number;
  /** What the listing would say about it on a shared row, or the symbol's name
   *  for a block of a decoder's trace. Never reformatted here. */
  readonly text: string;
  /** What the cell itself shows, where that is shorter than what the tooltip
   *  says: a deflate literal is the byte it decodes to, so a row of them reads
   *  as the text coming out of the block, and a match is its two numbers. The
   *  same as `text` for everything else. */
  readonly label: string;
  /** `"uint" | "int" | "float" | "bytes" | "str" | "enum" | "flags" |
   *  "composite" | "symbol" | "scale"` */
  readonly kind: string;
  /** False when the element's bits are not one run, which is what sends the
   *  table to its uniform layout: a `q5_0` weight is four bits of `qs` and a
   *  fifth bytes away in `qh`. */
  readonly contiguous: boolean;
  /** True when this record reads exactly as the record before it in the run,
   *  so the cell is drawn without its text: a capture that is nine tenths idle
   *  packets is nine tenths one sentence written again, and the records that
   *  say something else are what the reader is scrolling for. `text` is still
   *  what it says, for the tooltip and for the width the table is laid out
   *  to. */
  readonly repeat: boolean;
};

/** What a decoder does with one run of bits: `more` and `stop` are the
 *  continuation bit either way round, `width` is EBML spending leading zeros
 *  on how wide the number is, and `payload` is the number itself. */
export type BitRole = "more" | "stop" | "width" | "payload";

/** One variable-length number's bits, in the order they are stored. `rule` is
 *  the key the view has wording for; the core carries no wording. */
export type BitRoles = {
  readonly rule: string;
  readonly groups: readonly { readonly bits: string; readonly role: BitRole }[];
};

/** What the byte-class scan behind the overview has found so far. */
export type OverviewState = {
  /** True once every bucket is classified; until then ask again. */
  readonly done: boolean;
  /** Bytes one bucket stands for. A power of two, and 1 for a small file. */
  readonly bucket_bytes: number;
  readonly total_buckets: number;
  /** One digit per finished bucket, in file order: 0 zeros, 1 one repeated
   *  byte, 2 text, 3 data, 4 high entropy. */
  readonly classes: string;
  readonly zero_bytes: number;
  readonly text_bytes: number;
  /** How far the scan has read, in bytes. */
  readonly read_bytes: number;
  /** How many of each byte value have been read, value 0 first, 256 entries.
   *  The whole spread rather than the commonest few: what a reader wants from
   *  it is the shape, and the shape is in the tail. */
  readonly histogram: readonly number[];
};

/** The same scan over one block, with what the block's bytes turned out to be
 *  as a whole rather than what each of its buckets did. */
export type FocusState = OverviewState & {
  /** The block, in bytes. */
  readonly start: number;
  readonly end: number;
  /** Entropy over the block, and the most a block this long could reach. */
  readonly entropy: number;
  readonly entropy_max: number;
  readonly distinct: number;
  /** The values that appear most, commonest first. `histogram` sorted and cut
   *  short, for a view that wants only the peaks. */
  readonly common: readonly { readonly value: number; readonly count: number }[];
};

/** One kind of field, of one type, and what the file spends on it. */
export type KindTotal = {
  /** The same word `TemplateNode.kind` carries, worked out from the type
   *  rather than from a value: `unread` and `unset` are facts about bytes
   *  rather than about types and never appear here. */
  readonly kind: string;
  /** The type as the type column writes it: `u16 le`, `cstr`, `ZipRecord`. */
  readonly type: string;
  readonly bits: number;
  readonly count: number;
};

/** Where the file's bits have gone, as far as the walk has got. */
export type KindTotals = {
  /** True once the whole file is accounted for; until then ask again. */
  readonly done: boolean;
  /** How far into the file the walk has reached. What is past this is in none
   *  of the numbers below, so bits still to do is the file's length less
   *  this. */
  readonly reached_bits: number;
  /** Bits some field covers, which is the sum of `totals`. */
  readonly covered_bits: number;
  /** Bits inside the reached region that no field covers: the slack at the end
   *  of a structure, padding between records, the tail of a file whose
   *  template describes only its header. */
  readonly unmapped_bits: number;
  /** Biggest first. A composite is here only for the bits its children leave
   *  over that its own syntax accounts for, so a structure's bytes are never
   *  counted twice. */
  readonly totals: readonly KindTotal[];
};

/** How the text in the search bar is read. */
/** How the file reads as text: what encoding, whether that was a guess, and
 *  how many bytes of byte-order mark sit in front of the first line. */
export type TextReading = {
  readonly encoding: string;
  readonly mark: number;
  readonly guessed: boolean;
  /** Bytes one character takes at least, which is what an offset in the text
   *  is a multiple of. */
  readonly unit: number;
};

/** One line of the file. `escapes` is flat pairs of character index and
 *  length: the escape sequences are left in the text rather than removed, so
 *  the view decides whether to show them, dim them or act on them. */
export type TextLine = {
  readonly at: number;
  readonly len: number;
  readonly ending: string;
  readonly text: string;
  readonly escapes: readonly number[];
  readonly lossy: boolean;
};

export type TextWindow = {
  readonly lines: readonly TextLine[];
  readonly missing: readonly number[];
  readonly next: number;
};

export type TextBack = {
  readonly start: number;
  readonly back: number;
  readonly missing: readonly number[];
};

/** Which readings the string scan looks for. The names cross the boundary, so
 *  they are what the core answers to. */
export type StringEncoding = "ascii" | "utf16le" | "utf16be";

/** What the scan is asked to look for. */
export type StringScanOpts = {
  readonly minChars: number;
  readonly encodings: readonly StringEncoding[];
};

/** A number in front of a string that comes to its length. `bytes` is what it
 *  was read from, so the reading can be checked against the file rather than
 *  taken on trust. */
export type StringPrefix = {
  readonly kind: string;
  readonly at: number;
  readonly bytes: readonly number[];
  readonly value: number;
  readonly counts: string;
  readonly with_terminator: boolean;
  /** True when the number is no wider than one character of the string and
   *  nothing else vouches for it, which makes it the run's own boundary read
   *  a second time rather than a fact about the file. */
  readonly weak: boolean;
};

/** One string found in a file that is not a text file. */
export type StringHit = {
  readonly at: number;
  readonly len: number;
  readonly enc: string;
  readonly chars: number;
  readonly units: number;
  readonly text: string;
  /** True when a UTF-16 surrogate went unpartnered, which is WTF-16. Such a
   *  character reads as U+FFFD in `text`. */
  readonly lone_surrogates: boolean;
  /** Bytes of zero after the text: none, one, or two after a wide string. */
  readonly terminator: number;
  /** True when the run was cut at the limit and carries on in the next hit. */
  readonly cut: boolean;
  /** Every reading of the bytes in front that comes to this string's length.
   *  More than one is usually the same number written at several widths. */
  readonly prefix: readonly StringPrefix[];
};

export type StringScan = {
  readonly hits: readonly StringHit[];
  readonly missing: readonly number[];
  readonly next: number;
};

/** Where the lines in a stretch of the file start, and how they ended. The
 *  starts arrive as a typed array because there can be a great many of them. */
export type TextIndex = {
  readonly starts: Float64Array;
  /** Where the line after the last one starts: where the next call carries on
   *  from. The scan stops at a line start, never inside a line. */
  readonly next: number;
  readonly lf: number;
  readonly cr: number;
  readonly crlf: number;
};

/** The most text this will read as a dump in one go. A dump is four times the
 *  size of what it describes, so this is a file of a few megabytes. */
export const DUMP_LIMIT = 64 * 1024 * 1024;

/** What a text file turned out to be a dump of. */
export type DumpScan = {
  readonly tool: string;
  readonly tier: string;
  readonly from: number;
  readonly to: number;
  readonly covered: number;
  readonly address_base: string;
  readonly address_digits: number;
  readonly bytes_per_line: number;
  readonly group: number;
  readonly upper: boolean;
  readonly reversed_groups: boolean;
  readonly characters: string;
  readonly assumed: readonly string[];
  readonly extents: readonly number[];
  readonly holes: readonly number[];
  readonly names: readonly string[];
  readonly stated_length: number;
  readonly commands: readonly string[];
  readonly skipped_lines: number;
  readonly conflicts: readonly { at: number; wrote: string; digits: number }[];
};

/** One reading of a run of bytes, and the encodings that agree on it. */
export type Reading = { readonly encodings: readonly string[]; readonly text: string };

/** Every way a selection reads as text. `refused` names the encodings the
 *  bytes do not fit, which is worth saying and not worth showing. */
export type SelectionText = {
  readonly readings: readonly Reading[];
  readonly refused: readonly string[];
  readonly read: number;
  readonly all: boolean;
};

/** Typed text turned back into bytes, or the character that stopped it. */
export type TextEncoded = { readonly bytes: readonly number[]; readonly refused: string };

export type NeedleKind = "hex" | "text" | "regex";

/** Everything a search needs to know, which is everything the bar holds. */
export type Query = {
  readonly kind: NeedleKind;
  readonly text: string;
  /** Text only: match letters in either case. */
  readonly fold: boolean;
  readonly backward: boolean;
};

/** What one window of a search found. Offsets are bytes. */
export type SearchStep =
  | { readonly step: "found"; readonly at: number; readonly len: number }
  | { readonly step: "more"; readonly resume: number }
  | { readonly step: "end" };

/** What one other field decided about this one. `points` is the other way
 *  round: this field holds an offset, and that is where it points. */
export type Origin = {
  readonly role: "length" | "count" | "type" | "position" | "value" | "name" | "width" | "points";
  /** The field as the reader would name it: `len`, or `tensors[3].offset`. */
  readonly label: string;
  /** Where it is, so the reader can go there. Empty for a `points` entry. */
  readonly path: number[];
  /** What it says, in brief. Empty when it could not be read. */
  readonly value: string;
  /** For `points`: the bit this field's value points at. */
  readonly target_bits: number | null;
};

/**
 * What a field checks, when it is a checksum rather than a number the format
 * uses for something.
 *
 * The core answers this from the template, so a view never has to recognise a
 * format to know that a field is a sum or what it covers. Nothing is read to
 * answer it: `over` and `unpacked_from` are where the bytes are, not the bytes,
 * and taking the sum is `runCheck`.
 *
 * Exactly one of `over` and `unpacked_from` is set. `over` is a run of the file
 * and a reader can be sent to it. `unpacked_from` is the compressed run whose
 * *contents* are summed, for a sum over bytes that are nowhere in the file: a
 * ZIP entry's CRC-32 is of the file, not of the deflate stream, and the stream
 * is the nearest thing there is to point at.
 */
export type FieldCheck = {
  /** What to call it. `crc16` covers two different sixteen-bit sums; which
   *  polynomial a format chose is not something an interface can act on. */
  readonly algorithm: "crc32" | "crc16" | "sum8" | "sum" | "sha1" | "adler32";
  /** `[offset, length]` in bytes, when the summed bytes are in the file. */
  readonly over: readonly [number, number] | null;
  /** `[offset, length]` in bytes of the compressed run, when they are not. */
  readonly unpacked_from: readonly [number, number] | null;
  /** How many bytes the sum is over, so a view can decide whether to run the
   *  check unasked. Real when `covered_exact` is true; otherwise a stand-in,
   *  which a view must not print as the covered count. See `covered_exact`. */
  readonly covered_bytes: number;
  /** Whether `covered_bytes` is really how many bytes the sum covers. False
   *  only where nothing but a decoder knows the true count and none has run
   *  yet: an unpacked run with no declared length, such as a 7z stream whose
   *  own header carries none, or one member of a run (`unpacked_member`)
   *  before its stream has been opened for some other reason. Once it has,
   *  asking again finds the true count waiting and this turns true. */
  readonly covered_exact: boolean;
  /** Whether the sum is over one member's share of an unpacked run — an xz
   *  block's own check — rather than the whole of what the run unpacks to.
   *  False for every other check, including a whole ZIP entry's CRC-32. */
  readonly unpacked_member: boolean;
  /** `[offset, length, byte]`: the check field's own bytes, and what they are
   *  read as while the sum runs, for a format that seals a record the checksum
   *  sits inside. A tar header is summed with its checksum read as spaces, so
   *  a reader adding up those 512 bytes as they stand would get another
   *  number; the panel has to say so or the arithmetic looks wrong. Null for
   *  every check that sums the bytes as they are. */
  readonly blanked: readonly [number, number, number] | null;
};

/**
 * The moment a field means, once the epoch the template declares has been
 * applied to the number in it.
 *
 * The core answers this, so a view never has to recognise a format to know that
 * four bytes are a date or which epoch they count from. It used to be about
 * eight cases written out in the inspector, keyed on the template's name and
 * the field's; every other timestamp in every other format showed as an
 * integer.
 *
 * The stored number is not in here. It stays on the value row above, which is
 * the point: this is what the number means, not a replacement for it.
 */
export type FieldTime = {
  /** `at` is a moment. `unset` is the value this format writes when it has no
   *  time to record, which gzip's `mtime` of 0 is: not the first second of
   *  1970. `impossible` is a number that names no moment at all, either a year
   *  outside 1 to 9999 or a packed date that is not a date. */
  readonly state: "at" | "unset" | "impossible";
  /** Seconds from 1970-01-01T00:00:00Z, negative before it. Only for `at`. */
  readonly unix_seconds: number | null;
  /** The sub-second part in nanoseconds, 0 to 999,999,999 and never negative,
   *  so the pair reads as one number whichever side of 1970 it falls. Only for
   *  `at`. */
  readonly nanos: number | null;
  /** For `local` and `unknown` the seconds above are the digits the file wrote
   *  laid on the UTC line. Print them as they are and say which this was: a
   *  ZIP's MS-DOS time records no zone, and shifting it into the reader's own
   *  would be inventing one. */
  readonly zone: "utc" | "local" | "unknown";
  /** The smallest step the field can express, in nanoseconds, which is how many
   *  decimal places of a second are worth printing: 1e9 for a field counting
   *  seconds, 1e3 for a journal's microseconds, 100 for a FILETIME. Showing
   *  `.000000` on a field that counts seconds invents precision. */
  readonly step_nanos: number;
};

/**
 * What came of taking a checksum. Both forms are printed to the algorithm's own
 * width, so they compare as strings and line up on screen: `0x2cab616f` for a
 * CRC-32, `0x1f` for a sum-8, forty characters for a SHA-1.
 *
 * There is no third state here. A check that cannot be made never comes back as
 * a mismatch: bytes still arriving are `pending`, and a run too large or a
 * stream that will not unpack is an error carrying the reason.
 */
export type Verdict = {
  readonly computed: string;
  readonly stored: string;
  readonly ok: boolean;
};

/**
 * One relationship behind a field's shape, written out. `origins` says which
 * fields decided a length; this says what was done with them, so a reader can
 * check the number instead of taking it.
 *
 * The core writes both forms. Nothing here parses or rearranges them: the UI
 * never infers a relationship of its own.
 */
export type Relation = {
  readonly role: "length" | "count" | "type" | "value" | "name" | "width" | "position";
  /** The expression as the template writes it: `header_size - sizeof(header_size)`. */
  readonly written: string;
  /** The same with every field's value in its place: `4 - 1`. */
  readonly substituted: string;
  /** What it comes to. */
  readonly result: string;
};

/**
 * How a field came to be where it is, and how long it turned out to be: one
 * word for each question, answered for every field.
 *
 * `origins` answers with fields, and only for the fields another field decided
 * something about, which is a small minority. This answers for all of them, so
 * a panel can say why a plain `u32` is where it is instead of showing an empty
 * list. Which *field* settled it, where one did, is still `origins`.
 *
 * `unknown` means the core does not recognise the shape. It is a real answer
 * and the view says nothing at all for it: a reason invented to fill the line
 * would read exactly like a reason the file gave.
 */
export type Shape = {
  /** `root` the whole file; `first` the first field of what holds it; `follows`
   *  after the field before it; `element` one element of a run; `pointer` a
   *  table of offsets placed it; `chain` the element before it named it;
   *  `address` an address the file gave; `trace` where a decoder had got to;
   *  `stream` the front of what a compressed run unpacked to. */
  readonly placed: "root" | "first" | "follows" | "element" | "pointer" | "chain" | "address" | "trace" | "stream" | "unknown";
  /** `type` the type's own width, which the type's name already carries and
   *  which the panel therefore says nothing about; `fixed` a length the format
   *  fixes that the type does not carry, such as 116 bytes of `ascii[]`;
   *  `expression` worked out from the file;
   *  `terminated` ends at a terminator; `remaining` fills what is left of its
   *  container; `children` as long as the fields inside it; `scattered` a list
   *  whose elements are wherever its offsets point; `count` as many elements
   *  as a count says; `encoded` its own bytes say where it ends; `trace` as
   *  much as the decoder read; `table` a code-length table in this file gave
   *  the width; `nothing` no bytes of its own. */
  readonly sized:
    | "type"
    | "fixed"
    | "expression"
    | "terminated"
    | "remaining"
    | "children"
    | "scattered"
    | "count"
    | "encoded"
    | "trace"
    | "table"
    | "nothing"
    | "unknown";
};

/** One field of a subtree, as much of it as an arrow needs. No value: what a
 *  field says is the expensive half of reading it, and nothing a graph draws
 *  shows it. */
export type GraphNode = {
  readonly path: number[];
  readonly name: string;
  /** The resolved type said coarsely, to group or colour by: `u8`, `u16`,
   *  `i32`, `uint`, `varint`, `f16`, `bf16`, `f32`, `f64`, `f80`, `f8`,
   *  `fixed`, `digits`, `magic`, `bytes`, `str`, `enum`, `flags`, `computed`,
   *  `insn`, `json`, `struct`, `array`, `repeat`, `pointers`, `chain`, `at`,
   *  `stream`, `trace`, `switch`, `named`. The core decides these; nothing
   *  here should read a type name and work out its own. */
  readonly kind: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  /** Index into the node list, or -1 for the node the graph was asked for. */
  readonly parent: number;
  readonly child_count: number;
  /** True when this node has children the walk stopped short of. */
  readonly truncated: boolean;
};

/** One field deciding something about another. The arrow runs the way a reader
 *  follows it: from the field that decided to the field it decided about. */
export type GraphEdge = {
  /** Index into the node list: the field that decided. */
  readonly from: number;
  /** Index into the node list: the field it decided about. */
  readonly to: number;
  readonly role: "length" | "count" | "type" | "position" | "value" | "name" | "width" | "points";
};

/**
 * A subtree of fields and what they decide about each other. `origins` answers
 * for one field at a time, which is what a panel showing one row needs; this
 * answers for all of them at once, which is what a picture of the format needs.
 *
 * Every edge joins two nodes of this list. One whose far end fell outside the
 * subtree or past the cap is dropped rather than left hanging.
 */
export type FieldGraph = {
  readonly nodes: GraphNode[];
  readonly edges: GraphEdge[];
  /** How many nodes under the one asked about were left out, as far as is
   *  known. A run whose count would take a walk of the whole file to settle
   *  contributes what the walk reached, so this is a floor rather than a
   *  total. */
  readonly omitted: number;
};

/** One node of an HDF5 B-tree, of either version. */
export type TreeNode = {
  /** Where the node is in the template. Empty for a version 2 node below the
   *  root, which the template does not place: such a box goes to its bytes and
   *  is not opened in the Listing, because there is no field there to open. */
  readonly path: readonly number[];
  /** Index into the node list, or -1 for the root. Every node but the root
   *  comes after its parent in the list. */
  readonly parent: number;
  /** `index` for a `TREE` node at any level or a version 2 `BTIN`, `links` for
   *  the symbol table node a version 1 group tree hangs below its bottom row,
   *  `leaf` for a version 2 `BTLF`. Only a version 1 group tree has `links`
   *  nodes: every other silhouette here stops at a row that points at things
   *  which are not nodes. */
  readonly kind: "index" | "links" | "leaf";
  /** The four bytes written at `address`: `TREE`, `SNOD`, `BTIN` or `BTLF`. */
  readonly sign: string;
  /** Where the node starts in the file, in bytes. */
  readonly address: number;
  readonly size_bits: number;
  /** What the file wrote as this node's level, for a version 1 index node.
   *  Zero for a link table, which sits below the levels rather than on one. A
   *  version 2 node writes no level, so this is the header's depth less the
   *  rows walked to reach it: level 0 is the bottom row in both versions. */
  readonly level: number;
  /** Rows below the root of this tree, counted by the walk. */
  readonly depth: number;
  /** The file's own `entries_used`, `symbol_count`, or version 2 record count,
   *  with no denominator: the bound HDF5 puts on these is not settled here, and
   *  a fraction whose bottom half was guessed is worse than a count. A version
   *  2 internal node points at one more child than it holds records, because a
   *  record sits between every two children. */
  readonly entries: number;
  /** The ends of the node's key range, or empty where the walk could not
   *  settle both, and empty throughout a version 2 group tree, whose records
   *  hold the hash of a name rather than the name. Link names for a version 1
   *  group tree; for a chunk tree of either version, the comma-separated
   *  numbers of a chunk's offset inside the dataset. */
  readonly first_key: string;
  readonly last_key: string;
  /** True when children of this node were not reached, so its count stands and
   *  its range does not. */
  readonly truncated: boolean;
};

/**
 * One HDF5 B-tree, walked into the shape it has in the file.
 *
 * Which tree is decided by the core from the path handed in: the tree the
 * cursor is inside, else the tree the object header it is inside names, else
 * the root group's, else the first tree under the root group.
 */
export type Tree = {
  /** `group` for a tree indexing a group's links, `chunk` for one indexing a
   *  dataset's chunks, `other` for a version 2 tree indexing neither. */
  readonly job: "group" | "chunk" | "other";
  /** 1 or 2: which of the two structures this is. The silhouettes differ, so
   *  the captions do. */
  readonly version: number;
  /** The record type byte a version 2 header writes, and the name the HDF5
   *  specification gives it. Zero and empty for a version 1 tree. */
  readonly record_type: number;
  readonly record_type_name: string;
  /** How far the walk got with the records: `read` where what a record holds is
   *  known, `unread` where the specification names the type and the core does
   *  not read it, `unknown` where no version of the specification names it. The
   *  shape is drawn for all three, because it depends only on the record size;
   *  a shape drawn over records nobody read has to say so. */
  readonly records: "read" | "unread" | "unknown";
  /** How many records the tree holds in all, from the version 2 header's own
   *  field. Zero for a version 1 tree. Not the sum of the nodes' counts: a
   *  version 2 internal node holds records the leaves under it do not repeat,
   *  so summing the bottom row would print a total the file disagrees with, and
   *  this stands when the walk was capped. */
  readonly records_total: number;
  readonly nodes: readonly TreeNode[];
  /** Children that exist and were not walked, as far as the walk knows. */
  readonly omitted: number;
  /** How many numbers one chunk key holds. Zero for a group tree and for a tree
   *  whose records were not read. */
  readonly coords: number;
  /** True where the last of those numbers is the always-zero offset within an
   *  element, which a version 1 chunk key ends with and a version 2 record does
   *  not. Without it a reader counting the numbers in `950, 950, 0` gets a rank
   *  one too high. */
  readonly coords_pad: boolean;
};

/** What a type permits, beyond what this file's bytes happen to say. */
/** One row of a cross-reference stream, already decoded. `offset` is a real
 *  place in the file for an in-use row and -1 for every other kind. */
/** One object inside an object stream. Its offset is inside the decompressed
 *  bytes, so there is nowhere in the file to go to. */
export type ObjStmObject = {
  readonly number: number;
  /** How long the object is in the decompressed bytes. */
  readonly len: number;
  /** The object as written, cut at the limit the core keeps. */
  readonly text: string;
  readonly cut: boolean;
};

/** One column of a row that was put back together from the pages it spilled
 *  onto. `at` counts in the joined row, not in the file: a column can cross a
 *  page break, so there is no single file offset for it. */
export type SqliteColumn = {
  /** What SQLite calls the type: `i32`, `text, 8189 bytes`, `null`. */
  readonly type: string;
  readonly value: string;
  readonly value_kind: string;
  readonly at: number;
  readonly len: number;
};

/** One filter undone on the way back to a chunk's elements. */
export type ChunkStep = {
  readonly filter: string;
  readonly in_bytes: number;
  readonly out_bytes: number;
  /** True when this chunk's own mask said the filter was not applied to it. */
  readonly skipped: boolean;
};

/** One object of an HDF5 file, as the contents list reads it. */
export type ContentObject = {
  readonly path: readonly number[];
  /** The path it goes by inside the file: `/obs/n_genes`. */
  readonly name: string;
  readonly group: boolean;
  /** What the file calls it, where it says: `dataframe`, `csr_matrix`. */
  readonly encoding: string;
  readonly shape: readonly number[];
  /** What one element is. */
  readonly element: string;
  /** Which of the three ways its bytes are kept: "contiguous", "compact",
   *  "chunked", or nothing at all for a group. */
  readonly storage: string;
  /** How many bytes, where they are in one run. */
  readonly bytes: number;
  /** The chunk it is kept in, where it is kept in chunks. */
  readonly chunk_dims: readonly number[];
  /** The filters its chunks were written through, in that order. */
  readonly filters: readonly string[];
  /** Where its object header is. */
  readonly address: number;
};

/** What an HDF5 file holds, and what kind of file it is. */
export type Contents = {
  readonly objects: readonly ContentObject[];
  readonly total: number;
  /** Whether it is an AnnData object, and what the root group calls itself. */
  readonly anndata: boolean;
  readonly encoding: string;
  readonly rows: number;
  readonly columns: number;
};

export type ElfContents = {
  readonly sections: readonly {
    readonly path: readonly number[];
    readonly name: string;
    readonly kind: number;
    readonly address: number;
    readonly offset: number;
    readonly size: number;
  }[];
  readonly symbols: readonly {
    readonly path: readonly number[];
    readonly source_bits: number;
    readonly name: string;
    readonly kind: number;
    readonly section: number;
    readonly value: number;
    readonly size: number;
  }[];
  readonly symbol_total: number;
};

export type IsoVolume = {
  readonly descriptor_path: readonly number[];
  readonly volume: string;
  readonly joliet: boolean;
  readonly block_size: number;
  readonly blocks: number;
  readonly root_extent: number;
  readonly root_size: number;
  readonly root_source_bits: number;
};

export type IsoDirectory = {
  readonly entries: readonly {
    readonly name: string;
    readonly directory: boolean;
    readonly extent: number;
    readonly size: number;
    readonly source_bits: number;
    readonly extents: number;
    readonly multi_extent: boolean;
  }[];
  readonly total: number;
};

export type XrefRow = {
  readonly object: number;
  readonly kind: string;
  /** The type number the row held: 0, 1, 2, or whatever an undefined type
   *  wrote. `kind` is the same word for every undefined one. */
  readonly type_raw: number;
  readonly offset: number;
  readonly second: number;
  readonly third: number;
};

export type TypeInfo = {
  readonly kind: "magic" | "enum" | "flags" | "float" | "quant" | "xref" | "objstm" | "sqliterow" | "chunk" | "plain";
  /** The type's own name, for an enum or a flags field. */
  readonly name: string;
  /** Magic: what the format requires, and what is there. */
  readonly expected: number[];
  readonly actual: number[];
  /** Enum: every value it names, and the one in the file. */
  readonly cases: readonly { readonly value: number; readonly name: string }[];
  readonly current: number;
  /** Enum: what the value in the file is called, where the name comes from a
   *  counted run of values rather than from `cases`. Empty when it has none. */
  readonly named: string;
  readonly hex: boolean;
  /** Flags: one entry per bit of the field, from bit 0 up. */
  readonly bits: readonly { readonly bit: number; readonly name: string | null; readonly set: boolean }[];
  /** Float: which layout it is, how wide, and its bits in value order in hex. */
  readonly format: string;
  /** Float: how many bits wide. Quant: how many bits one weight is worth. */
  readonly width: number;
  readonly pattern: string;
  /** Quant: the block's shared scale, and what it pairs with it, named as the
   *  file names it. `second_name` is empty where the layout has no second. */
  readonly scale: number;
  readonly second_name: string;
  readonly second: number;
  /** Quant: whether that second number is taken away rather than added, and
   *  whether it is multiplied by the group's own minimum first. */
  readonly second_subtract: boolean;
  readonly second_per_group: boolean;
  /** Quant: where the block starts, so a weight's bits can be found from the
   *  offset it carries. */
  readonly block_bits: number;
  /** Quant: the scale the block keeps for each run of weights, where it keeps
   *  them, and how many weights one run covers. Empty for a block with one
   *  scale for all of them. */
  readonly groups: readonly QuantGroup[];
  readonly group_weights: number;
  /** Quant: taken off the packed value to get the stored one, and whether that
   *  value is read signed instead of biased. */
  readonly bias: number;
  readonly signed: boolean;
  /** Quant: every weight the block stands for, in the order the tensor reads
   *  them, and which one the cursor is inside (-1 for none). */
  readonly weights: readonly QuantWeight[];
  readonly at: number;
  /** Xref: the widths from `/W`, and the PNG predictor where there was one
   *  (-1 where there was not). */
  readonly xref_widths: readonly number[];
  readonly xref_predictor: number;
  /** Xref: how many bytes the rows are in the file, and how many once
   *  decompressed. */
  readonly xref_packed: number;
  readonly xref_decoded: number;
  /** Xref: how many rows of each kind, over the whole table rather than over
   *  the ones listed. */
  readonly xref_free: number;
  readonly xref_in_file: number;
  readonly xref_in_stream: number;
  readonly xref_unknown: number;
  /** Xref: the rows, and how many there are altogether. */
  readonly xref_rows: readonly XrefRow[];
  readonly xref_total: number;
  /** Xref: why there are no rows, where there are none. An object stream that
   *  would not open says why here too. Empty otherwise. */
  readonly problem: string;
  /** ObjStm: how many bytes the objects are in the file, and how many once
   *  decompressed. */
  readonly objstm_packed: number;
  readonly objstm_decoded: number;
  /** ObjStm: the object stream this one continues, or -1 where it continues
   *  none. */
  readonly objstm_extends: number;
  /** ObjStm: the objects, and how many there are altogether. */
  readonly objstm_objects: readonly ObjStmObject[];
  readonly objstm_total: number;
  /** Row: how many bytes the row claims, how many were found, and how many of
   *  them stayed on the row's own page. A whole row has the first two equal. */
  readonly row_declared: number;
  readonly row_found: number;
  readonly row_on_page: number;
  /** Row: the pages the rest of it is on, in chain order, and how many there
   *  are when that is more than the few listed. */
  readonly row_pages: readonly number[];
  readonly row_chain: number;
  /** Row: the columns, and how many there are altogether. */
  readonly row_columns: readonly SqliteColumn[];
  readonly row_total_columns: number;
  /** Chunk: how many bytes it is in the file, and how many its elements came
   *  to once the filters were undone. */
  readonly chunk_packed: number;
  readonly chunk_decoded: number;
  /** Chunk: each filter, in the order it was undone. */
  readonly chunk_steps: readonly ChunkStep[];
  /** Chunk: what one element is called, the first few of them, and how many
   *  there are altogether. */
  readonly chunk_element_type: string;
  readonly chunk_values: readonly string[];
  readonly chunk_total: number;
};

/** One run of weights inside a block that share a scale of their own. */
export type QuantGroup = {
  /** The scale as stored, after whatever bias the type takes off it. */
  readonly scale: number;
  /** The minimum taken off every weight in the run, or null where the type has
   *  none. */
  readonly min: number | null;
};

/** One weight of a block of packed weights. */
export type QuantWeight = {
  /** The stored integer, after whatever bias the layout takes off it. */
  readonly q: number;
  /** That integer through the block's scale: the number the model reads. */
  readonly value: number;
  /** The run holding its low bits, and the rest of the packed value where the
   *  layout keeps that somewhere else in the block. */
  readonly bits: QuantPart;
  readonly high: QuantPart | null;
};

/** One run of bits that makes up part of a packed weight. */
export type QuantPart = {
  /** The block field these bits are in, as the file names it: `qs`, `qh`. */
  readonly field: string;
  /** Where they are, counted in bits from the start of the block. */
  readonly bit: number;
  readonly width: number;
  /** Where they sit in the packed value: 0 for the low part. */
  readonly shift: number;
};

/** What one rule, or the editor itself, concluded about what produced a file. */
export type ToolMatch = {
  /** The database's own word: `packer`, `compiler`, `protector`. */
  readonly category: string;
  readonly name: string;
  readonly version: string | null;
  /** Free text written by the rule's author, shown as written. */
  readonly options: string | null;
  /** The signature file that answered, or `OWN_SOURCE` for the editor's own. */
  readonly source: string;
};

/**
 * What `source` says for an answer the editor worked out itself rather than
 * took from the signature database. Kept in step with `dosbasic::SOURCE` in
 * the core, so the dialog credits each answer to whatever actually found it.
 */
export const OWN_SOURCE = "qubero";

/**
 * The largest a .COM can be. It is loaded whole into one 64 KiB segment below
 * the stack, so anything bigger is not one, whatever its name says.
 */
const COM_LIMIT = 65280;

/**
 * How much of a DOS executable the signature rules get to look at.
 *
 * More than the 64 KiB an unknown format is identified from, because what says
 * which BASIC runtime one of these needs is at the end of the program rather
 * than the start, and so is the entry point of anything but a small one. A DOS
 * program cannot be much larger than this and still be one, and 1 MiB is 16
 * chunks against a cache of 512.
 */
const DOS_WINDOW = 1024 * 1024;

export type WrittenRange = { readonly offset_bits: number; readonly size_bits: number };

/** What the file(1) rules made of a file the editor has no template for. */
export type Identification = {
  /** The rule's own sentence, values and all: `PNG image data, 1280 x 720`. */
  readonly message: string;
  /** Media type, or "" where the rule carries none. */
  readonly mime: string;
  /** Extensions the rule lists, alphabetical. */
  readonly ext: readonly string[];
  /** The matching rule's strength; higher beat it to the answer. */
  readonly strength: number;
  /** The rule file it came from. */
  readonly source: string;
};

/**
 * What to call a template built from a rule.
 *
 * One extension is the best name there is: `gif` says more than the `images`
 * rule file it came from. Several are no use, because the rules hand them over
 * as an unordered set, so "the first" is whichever way they fell: a Windows
 * executable would be called `com` as readily as `exe`. In that case the rule
 * file's own name is at least stable and is what the dialog says beside it.
 */
function signatureName(id: Identification): string {
  if (id.ext.length === 1) return id.ext[0] ?? id.source;
  if (id.source !== "") return id.source;
  return (id.mime.split("/").pop() ?? "").replace(/^x-/, "");
}

export type TemplateReply<T> =
  | { readonly status: "ok"; readonly node: T }
  | { readonly status: "pending"; readonly reachedBytes: number }
  /** Still being worked out. `reachedBytes` is how far into the file the
   * reading has got. Asking again carries on from there. */
  | { readonly status: "working"; readonly reachedBytes: number }
  | { readonly status: "error"; readonly message: string };

/**
 * One thing a decoder did: which bits of the compressed run it read, which
 * bytes of the unpacked stream that came to, and what it was doing.
 *
 * The same shape answers both directions. `mapOut` asks what made a byte of the
 * output; `mapIn` asks what a bit of the input was read as. Either range may be
 * empty: a header field produces no output, and a short match reads no input of
 * its own past the code that named it.
 */
export type MapStep = {
  readonly in_start: number;
  readonly in_end: number;
  readonly out_start: number;
  readonly out_end: number;
  readonly kind:
    | "literal"
    | "match"
    | "stored"
    | "pixel"
    | "block"
    | "header"
    | "table"
    | "end-of-block"
    | "opaque";
  /** Which named field, for a header or a table step. */
  readonly field?: string;
  /** What that field said, or the byte a literal is. */
  readonly value?: number;
  /** A match's length; a table repeat's count; a code length. */
  readonly len?: number;
  /** A match's distance. */
  readonly dist?: number;
};

/**
 * One Huffman-coded number of a deflate step: which symbol the code stood for,
 * how wide the code was, the bits that followed it outright, and what the two
 * came to.
 *
 * None of this is in the trace, which keeps a decoded length and distance and
 * throws the rest away. It is worked out again from the block's tables, one
 * step at a time and only when asked. See `Editor.decode_step`.
 */
export type DecodedCode = {
  /** Which symbol of the alphabet the code decoded to: 77 for a literal `M`,
   *  256 for the end of a block, 257 and up for a length. */
  readonly symbol: number;
  readonly code_bits: number;
  /** How many bits followed the code outright, and what they said. Both zero
   *  for a literal, for the end mark, and for the many lengths and distances
   *  a symbol names on its own. */
  readonly extra_bits: number;
  readonly extra: number;
  /** What the symbol and its extra bits came to: the byte, the length, or the
   *  distance. Zero for the end mark, which stands for no number. */
  readonly value: number;
  /** Which step of the block's head set this code's width, as an index into
   *  the trace. Absent for a fixed-Huffman block, whose two tables are in RFC
   *  1951 rather than in the file, so there is nowhere honest to point. */
  readonly entry?: number;
  /** The same step counted from the front of its block, which is where the row
   *  sits among the block's children and so what a path needs. */
  readonly entry_child?: number;
};

/** One step of a deflate stream taken apart: the one or two codes that carried
 *  it, and every bit they cost. */
export type DecodedStep = {
  readonly kind: "literal" | "end-of-block" | "match";
  /** Which block of the stream, as an index among the stream's blocks. */
  readonly block: number;
  readonly block_kind: "fixed" | "dynamic";
  /** The literal/length code, which every step begins with. */
  readonly symbol: DecodedCode;
  /** The distance code that followed the length. Absent for a literal and for
   *  the end mark, which read no second code. */
  readonly distance?: DecodedCode;
  /** The four widths added up, which is the step's own width. Given rather
   *  than left to be summed, so a panel can show the arithmetic and its answer
   *  without doing the arithmetic itself. */
  readonly bits: number;
};

export type ExtentEstimate = {
  readonly path: readonly number[];
  readonly measured_items: number;
  readonly total_items: number;
  readonly measured_bits: number;
  readonly estimated_bits: number;
};

type RawReply<T> =
  | { status: "ok"; node: T; wanted?: number[] }
  | { status: "pending"; chunks: number[]; reached_bytes: number }
  | { status: "working"; reached_bytes: number }
  | { status: "error"; message: string };

export class ReadFailure extends Error {
  constructor(
    readonly offset: number,
    readonly length: number,
    cause: unknown,
  ) {
    super(describeReadFailure(offset, length, cause), { cause });
    this.name = "ReadFailure";
  }
}

function describeReadFailure(offset: number, length: number, cause: unknown): string {
  const where = `${formatBytes(length)} at offset ${formatOffset(offset * 8)}`;
  const reason =
    cause instanceof DOMException && cause.name === "NotReadableError"
      ? "The file has changed or moved since it was opened."
      : cause instanceof Error
        ? cause.message
        : "The file may have changed or moved since it was opened.";
  return `Could not read ${where} from the original file. ${reason}`;
}

/**
 * An address, in whatever space it belongs to. A field of the file gets the
 * plain address; one inside a decoded stream gets a `+` as well, because
 * `0x1c` of a stream and `0x1c` of the file are different bytes and a reader
 * comparing the listing against the hex view has to be able to tell.
 *
 * The mark stays in front of the `+`: `@+0x1c`, not `+@0x1c`. Every address
 * on screen begins with the same character, and what the address is counted
 * from is the next question, not the first one.
 */
export function formatAddress(bits: number, space: number): string {
  return space === 0 ? formatOffset(bits) : `${ADDRESS_MARK}+${offsetDigits(bits)}`;
}

/**
 * The shift-and-mask that reads `width` bits starting at `bitOffset` and leaves
 * them right-aligned in a plain number.
 *
 * Bits are counted the way the rest of the editor counts them: bit 0 of a byte
 * is its top bit, so a field three bits into a byte starts under the mask
 * `0x1f`. The first term is masked because the bits above it belong to whatever
 * came before; the last is shifted down by whatever the field leaves behind in
 * its final byte. `b[n]` is the byte at address `n`.
 */
export function bitFormula(bitOffset: number, width: number): string {
  const first = Math.floor(bitOffset / 8);
  const skip = bitOffset % 8;
  const bytes = Math.ceil((skip + width) / 8);
  const trailing = bytes * 8 - (skip + width);
  const terms: string[] = [];
  for (let i = 0; i < bytes; i++) {
    const shift = 8 * (bytes - 1 - i) - trailing;
    let term = `b[0x${(first + i).toString(16)}]`;
    if (i === 0 && skip !== 0) {
      const mask = ` & 0x${(0xff >> skip).toString(16).padStart(2, "0")}`;
      // Brackets only where something else would bind tighter than the mask.
      term = bytes === 1 && shift === 0 ? term + mask : `(${term}${mask})`;
    }
    if (shift > 0) term = `${term} << ${shift}`;
    else if (shift < 0) term = `${term} >> ${-shift}`;
    terms.push(term);
  }
  return terms.join(" | ");
}

/**
 * How much of the file something covers, written the way an address is: whole
 * bytes, then whatever bits are left over. `12`, or `0+4b` for a nibble.
 */
export function formatLength(bits: number): string {
  const bytes = Math.floor(bits / 8);
  const rem = bits % 8;
  return rem === 0 ? String(bytes) : `${bytes}+${rem}b`;
}

/** In-memory bytes as a `ByteSource`, for documents that come out of another
 *  one rather than off a disk: a decompressed zip entry, a run of bytes opened
 *  on its own. */
export function bytesSource(bytes: Uint8Array, name: string): ByteSource {
  return {
    size: bytes.length,
    name,
    slice(start, end) {
      return { arrayBuffer: () => Promise.resolve(bytes.slice(start, end).buffer as ArrayBuffer) };
    },
  };
}

export type ReadResult = {
  readonly bytes: Uint8Array;
  /** True when every byte came from loaded data. False means a reload will follow. */
  readonly complete: boolean;
};

/**
 * The editor's own code could not be fetched.
 *
 * Told apart from every other reason a file will not open because there is
 * something to be done about this one: the page may have outlived the files it
 * names, and asking for it again would find the new ones. See `staleassets`.
 */
export class EditorMissing extends Error {
  constructor(cause: unknown) {
    super("the editor could not be loaded", { cause });
    this.name = "EditorMissing";
  }
}

let wasmReady: Promise<unknown> | null = null;
function ensureWasm(): Promise<unknown> {
  // A failure is forgotten rather than remembered, the same way `loadMagic`
  // forgets one: a rejected promise kept here would be handed to every file
  // opened afterwards, so one bad moment would last as long as the tab and
  // trying again could never work. The same reasoning, and the same two lines.
  wasmReady ??= init().catch((e: unknown) => {
    wasmReady = null;
    throw new EditorMissing(e);
  });
  return wasmReady;
}

/** Sets already fetched, by the name they were asked for. @see glyphColumn */
const glyphSets = new Map<string, GlyphSet>();

/**
 * The characters the hex view's text column writes under one named set, or
 * null for a name the core does not know.
 *
 * The whole table of 256 comes over at once and is kept: the column is
 * redrawn a few hundred cells at a time, and a call per cell would charge
 * every frame for a choice the reader made once. Only callable once a file is
 * open, which is also the only time there is a column to draw.
 */
export function glyphColumn(name: string): GlyphSet | null {
  const had = glyphSets.get(name);
  if (had !== undefined) return had;
  // Split by code point rather than by UTF-16 unit: every page the app offers
  // stays inside the BMP today, and a set that did not would otherwise arrive
  // as halves of characters.
  const chars = [...glyph_column(name)];
  if (chars.length !== 256) return null;
  // The core writes U+FFFD for a byte the set has no character for. Here that
  // becomes an empty entry, so a cell asks "is there a character" rather than
  // comparing against one, and so no set can ever draw a replacement
  // character as if it meant something.
  const table = chars.map((c) => (c === "\ufffd" ? "" : c));
  glyphSets.set(name, table);
  return table;
}

export class Doc {
  private readonly inflight = new Set<number>();
  private readonly listeners = new Set<() => void>();
  /** A go at unfinished work is already queued. */
  private workScheduled = false;

  private constructor(
    private readonly editor: Editor,
    private readonly blob: ByteSource,
    readonly name: string,
    /** Which address space this document is. 0 is the file; anything else is
     *  a compressed run that was unpacked and opened in its own right. */
    readonly space = 0,
    /** For a space, the `Decoded` node of the file it was unpacked from. */
    readonly origin: readonly number[] = [],
  ) {}

  static async open(file: ByteSource): Promise<Doc> {
    await ensureWasm();
    const editor = new Editor(file.size, CHUNK_SIZE, CHUNK_CAPACITY);
    return new Doc(editor, file, file.name);
  }

  /** True for the file, false for an unpacked stream. */
  get isFile(): boolean {
    return this.space === 0;
  }

  /** Said when a change was asked for and refused. */
  onRefuseEdit: (why: string) => void = () => {};

  /**
   * Whether this document refuses to be changed, saying so once if it does.
   *
   * Every way of changing bytes goes through here rather than through a guard
   * in each view, because there are a dozen of those and one of them being
   * missed would write the change into the file the stream came out of: a
   * space's offsets are its own, and byte 4 of an unpacked stream is not byte 4
   * of anything else.
   */
  private refusesEdit(): boolean {
    if (this.space === 0) return false;
    this.onRefuseEdit(UNPACKED.readOnly);
    return true;
  }

  /**
   * Unpack the compressed stream at `path` and open what comes out as a
   * document of its own, over the same editor. A stream already open comes
   * back as the document it already is rather than being unpacked again.
   *
   * Null when the stream would not open. Which of the three ways it would not
   * is already on the node, so the caller does not have to ask twice.
   */
  openSpace(path: readonly number[]): Doc | null {
    const r = this.handleReply<{ space: number; template: string; refused?: string }>(
      this.editor.open_space(Uint32Array.from(path)),
    );
    if (r.status !== "ok" || r.node.space === 0) return null;
    const opened = new Doc(this.editor, this.blob, this.name, r.node.space, [...path]);
    // The template came with the space rather than being chosen for it, so it
    // is set here and never through `setTemplate`.
    opened.template = r.node.template === "" ? null : r.node.template;
    return opened;
  }

  /**
   * One step of a compressed run taken apart, for the code at `bit` of the run
   * the stream at `path` was unpacked from.
   *
   * Three questions of the editor, because they are three halves of one
   * answer and no caller wants to ask them separately. The stream is opened as
   * a space, which is what carries the trace and the number to ask by and
   * which is idempotent for one already open; the step is taken apart; and the
   * same bit is asked which bytes of the unpacked stream the step wrote, since
   * the taking-apart says what the codes were and not where their output
   * landed.
   *
   * Null for every step that is not a deflate symbol, and null while the
   * compressed bytes are still on their way: the reply says so, the chunks are
   * asked for, and the panel is drawn again when they land. Never called on a
   * scroll or a listing draw; rebuilding the block's tables is worth doing for
   * the one step under the cursor and not for the millions behind it.
   */
  decodedCode(path: readonly number[], bit: number): { readonly step: DecodedStep; readonly out: MapStep | null } | null {
    const opened = this.handleReply<{ space: number; template: string; refused?: string }>(
      this.editor.open_space(Uint32Array.from(path)),
    );
    if (opened.status !== "ok" || opened.node.space === 0) return null;
    const space = opened.node.space;
    const r = this.handleReply<DecodedStep | null>(this.editor.decode_step(space, bit));
    if (r.status !== "ok" || r.node === null) return null;
    const out = this.handleReply<MapStep | null>(this.editor.map_in(space, bit));
    return { step: r.node, out: out.status === "ok" ? out.node : null };
  }

  /** Where the byte at `byte` of this space came from, or null. */
  mapOut(byte: number): MapStep | null {
    const r = this.handleReply<MapStep | null>(this.editor.map_out(this.space, byte));
    return r.status === "ok" ? r.node : null;
  }

  /** Which step read the bit at `bit` of the compressed run this space was
   *  unpacked from, and so which of its bytes that bit produced. */
  mapIn(bit: number): MapStep | null {
    const r = this.handleReply<MapStep | null>(this.editor.map_in(this.space, bit));
    return r.status === "ok" ? r.node : null;
  }

  /** True when the template reading this space came from looking at the
   *  unpacked bytes rather than from what the stream declared. */
  get recognised(): boolean {
    return this.space !== 0 && this.editor.space_recognised(this.space);
  }

  /** Called whenever the document content changes or a pending chunk arrives. */
  onChange(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private notify(): void {
    for (const fn of this.listeners) fn();
  }

  get lengthBytes(): number {
    return this.editor.len_bytes(this.space);
  }
  get lengthBits(): number {
    return this.editor.len_bits(this.space);
  }
  get modified(): boolean {
    return this.editor.is_modified(this.space);
  }
  get canUndo(): boolean {
    return this.editor.can_undo(this.space);
  }
  get canRedo(): boolean {
    return this.editor.can_redo(this.space);
  }
  get pieceCount(): number {
    return this.editor.piece_count(this.space);
  }

  // ----- templates -----

  template: string | null = null;

  /** Whether the file is a ZIP archive, under whichever name it was opened:
   * a Zarr store in a ZIP is read by a template of its own, and its records
   * are an archive's records to the byte. Anything that works on entries has
   * to ask this rather than the template's name. */
  get isZip(): boolean {
    return this.template === "zip" || this.template === "zarrzip";
  }

  /** Whether the file is a PNG, whatever else it also is. A PICO-8 cartridge
   *  and a Picotron one are both PNGs with a payload hidden in the low bits of
   *  the pixels, and they read under their own templates, so anything asking
   *  "is this a PNG" by name alone said no and skipped the chunk checksums a
   *  PNG carries either way. */
  get isPng(): boolean {
    return this.template === "png" || this.template === "p8png" || this.template === "p64png";
  }

  /** Best current projection for a variable-size array still being walked. */
  extentEstimate(): ExtentEstimate | null {
    const raw = this.editor.extent_estimate(this.space);
    if (raw === "") return null;
    try {
      return JSON.parse(raw) as ExtentEstimate;
    } catch {
      return null;
    }
  }

  get templateNames(): string[] {
    return this.editor.template_names();
  }

  /**
   * Install a template built from the rule that identified this file, for a
   * format no built-in covers. It describes the format's signature and nothing
   * else, so most of the file stays unannotated; that is the whole of what the
   * rule proves.
   *
   * Returns the name to show for it, or null when the rule pins no fixed bytes
   * to a fixed place, which is the case for a format found by searching.
   */
  async signatureTemplate(id: Identification): Promise<string | null> {
    if (id.source === "") return null;
    // The rule files are static assets, one per format family, a few KiB each.
    // Only the one the identification named is fetched.
    let rules: string;
    try {
      const res = await fetch(`magdir/${encodeURIComponent(id.source)}`);
      if (!res.ok) return null;
      rules = await res.text();
    } catch {
      return null;
    }
    const n = Math.min(IDENTIFY_WINDOW, this.lengthBytes);
    await this.ensureRange(0, n);
    const { bytes, complete } = this.read(0, n);
    if (!complete) return null;
    const name = signatureName(id);
    if (!this.editor.set_magic_template(name, rules, bytes)) return null;
    this.template = name;
    this.notify();
    return name;
  }

  /** Built-in template name matching the file's first bytes, or null. */
  async sniffTemplate(): Promise<string | null> {
    // A space came with its template: it is what the stream's `Decoded` node
    // says is inside it. Sniffing would be asking the unpacked bytes to name a
    // format that has already been named.
    if (this.space !== 0) return null;
    // The sniffer says how much it wants: enough for a magic number, for the
    // format tag inside a WAVE's first chunk (the only thing that tells a W4V
    // from a WAV), for the PE header a Windows executable puts at an offset of
    // its own choosing, for an Anvil region's two 8 KiB tables, and for the
    // volume descriptor an ISO 9660 image writes 32 KiB in. Read less and the
    // deep formats come back unrecognised rather than wrong.
    const n = Math.min(this.editor.sniff_window(), this.lengthBytes);
    if (n === 0) return null;
    await this.ensureRange(0, n);
    const head = this.read(0, n).bytes;
    // An OME-Zarr store is a directory, so its identifying record is JSON
    // metadata rather than a byte signature.
    if (isOmeZarrMetadata(this.name, head)) return "omezarr";
    const name = this.editor.sniff_template(head, this.lengthBytes);
    // Only once the bytes have had their say: a file that announces what it is
    // is that, whatever it happens to be called.
    return name === "" ? templateByExtension(this.name) : name;
  }

  /**
   * Ask the file(1) rule database what this file is, for the files no template
   * covers. The rules and the engine that runs them outweigh the rest of the
   * editor, so they live in their own wasm module that is fetched on the first
   * call and never at all for a file `sniffTemplate` already answered.
   *
   * The answer is a label, not a layout: it names the format without
   * describing a single field.
   */
  async identify(): Promise<Identification | null> {
    // The unpacked bytes were named by the stream that holds them.
    if (this.space !== 0) return null;
    const n = Math.min(IDENTIFY_WINDOW, this.lengthBytes);
    if (n === 0) return null;
    await this.ensureRange(0, n);
    const { bytes, complete } = this.read(0, n);
    // Rules read what they are given, so a short window would answer for a
    // different file. Nothing is better than a wrong name.
    if (!complete) throw new Error("identify: the head of the file did not arrive");
    const json = (await loadMagic()).identify(bytes);
    return json === "" ? null : (JSON.parse(json) as Identification);
  }

  setTemplate(name: string | null): boolean {
    // A space's template is the one its stream was declared with. There is no
    // menu for it, and `set_template` is the file's alone.
    if (this.space !== 0) return false;
    const ok = this.editor.set_template(name ?? "");
    this.template = ok ? name : null;
    this.notify();
    return ok;
  }

  private handleReply<T>(json: string): TemplateReply<T> {
    const r: RawReply<T> = JSON.parse(json);
    if (r.status === "pending") {
      for (const c of r.chunks) this.fetchChunk(c);
      // A contiguous run of missing chunks means something is being read
      // through from front to back, so the chunks after it are what comes
      // next: worth reading in one go. Scattered ones mean fields across the
      // file wanting a byte each, and reading around those would evict what
      // they asked for.
      const first = r.chunks[0];
      if (first !== undefined && r.chunks.every((c, i) => c === first + i)) {
        this.fetchRun(first + r.chunks.length, READ_AHEAD);
      }
      return { status: "pending", reachedBytes: r.reached_bytes };
    }
    if (r.status === "ok" && r.wanted !== undefined && r.wanted.length > 0) {
      // Answered without some previews, so the rows are here and their first
      // bytes are on their way. Asking again once they land fills them in.
      for (const c of r.wanted) this.fetchChunk(c);
      return { status: "ok", node: r.node };
    }
    if (r.status === "working") {
      // Nothing is going to arrive to wake this up, the way a chunk does, so
      // it has to ask itself again. Yielding first lets the page draw what it
      // has and stay usable while the rest is worked out.
      this.scheduleMoreWork();
      return { status: "working", reachedBytes: r.reached_bytes };
    }
    return r;
  }

  /** Carry on with unfinished work after the page has had a chance to draw. */
  private scheduleMoreWork(): void {
    if (this.workScheduled) return;
    this.workScheduled = true;
    setTimeout(() => {
      this.workScheduled = false;
      this.notify();
    }, 0);
  }

  /**
   * What the file holds, in its own terms rather than the template's: the
   * objects of an HDF5 file, each with the path it goes by, what the file
   * calls it, its shape and where its bytes are. Empty for every other format.
   */
  contents(): TemplateReply<Contents> {
    return this.handleReply(this.editor.contents(this.space));
  }

  /** Named ELF sections and at most `symbolLimit` symbols. */
  elfContents(symbolLimit: number): TemplateReply<ElfContents> {
    return this.handleReply(this.editor.elf_contents(this.space, symbolLimit));
  }

  isoVolume(): TemplateReply<IsoVolume> {
    return this.handleReply(this.editor.iso_volume(this.space));
  }

  isoDirectory(extent: number, size: number, blockSize: number, limit: number, joliet: boolean): TemplateReply<IsoDirectory> {
    return this.handleReply(this.editor.iso_directory(this.space, extent, size, blockSize, limit, joliet));
  }

  templateNode(path: readonly number[]): TemplateReply<TemplateNode> {
    return this.handleReply(this.editor.template_node(this.space, Uint32Array.from(path)));
  }

  templateChildren(path: readonly number[], from: number, to: number): TemplateReply<TemplateNode[]> {
    return this.handleReply(this.editor.template_children(this.space, Uint32Array.from(path), from, to));
  }

  /**
   * The elements of the folded run at `path` whose bits fall between two
   * offsets, at most `max` of them. One call covers a screenful, the way
   * `spans` does: the run itself is one span saying how many elements it has,
   * and this is what those elements read as over the rows on screen.
   *
   * `path` is a span's own `path` when its `count` is above zero, or a block
   * of a decoded stream's trace.
   */
  runCells(path: readonly number[], fromBit: number, toBit: number, max: number): TemplateReply<Cell[]> {
    return this.handleReply<Cell[]>(this.editor.run_cells(this.space, Uint32Array.from(path), fromBit, toBit, max));
  }

  /** The whole text of a text field, decoded in the field's own encoding. */
  fieldText(path: readonly number[]): TemplateReply<{ text: string; truncated: boolean }> {
    return this.handleReply(this.editor.field_text(this.space, Uint32Array.from(path)));
  }

  /**
   * A field's own bytes, up to `limit`, read where the field actually is.
   *
   * Not `readBits` at the node's offset: a field inside a decoded stream is at
   * an offset of that stream, and the file at the same offset is other bytes.
   */
  fieldBytes(path: readonly number[], limit: number): TemplateReply<{ bytes: number[]; truncated: boolean }> {
    return this.handleReply(this.editor.field_bytes(this.space, Uint32Array.from(path), limit));
  }

  /**
   * Every field between two bit offsets, in order. One call covers what is on
   * screen, so the annotation column costs one round trip per view rather than
   * one per field.
   */
  spans(fromBit: number, toBit: number, max: number): TemplateReply<Span[]> {
    return this.handleReply<Span[]>(this.editor.spans(this.space, fromBit, toBit, max));
  }

  /**
   * One step of the byte-class scan behind the overview. The node carries
   * everything found so far, so a partial map can be drawn while the rest is
   * read; `done` says when to stop asking. An edit throws the scan away, and
   * the next step starts it over.
   */
  overviewStep(buckets: number): TemplateReply<OverviewState> {
    return this.handleReply<OverviewState>(this.editor.overview_step(this.space, buckets));
  }

  /**
   * One step of the scan over a single block of the file, divided into up to
   * `buckets` of its own. Asking about a different block starts a new scan.
   */
  overviewFocusStep(from: number, to: number, buckets: number): TemplateReply<FocusState> {
    return this.handleReply<FocusState>(this.editor.overview_focus_step(this.space, from, to, buckets));
  }

  /**
   * One step of the walk that totals the file's bits by field kind and type.
   * The node carries the totals so far, so a partial answer can be drawn while
   * the rest is worked out; `done` says when to stop asking. An edit throws the
   * walk away, and the next step starts it over.
   */
  kindTotalsStep(): TemplateReply<KindTotals> {
    return this.handleReply<KindTotals>(this.editor.kind_totals_step(this.space));
  }

  /** What is wrong with what the search bar holds, or "" when nothing is.
   *  `typing` suppresses the one complaint that is not a mistake yet: half a
   *  hex byte, which every valid needle passes through on the way in. */
  checkNeedle(kind: NeedleKind, text: string, typing: boolean): string {
    return this.editor.check_needle(kind, text, typing);
  }

  /**
   * One window of a search. The reply is the usual tri-state: a step, or
   * pending while the bytes it needs are fetched. The caller loops.
   */
  searchStep(needle: Query, from: number): TemplateReply<SearchStep> {
    return this.handleReply<SearchStep>(
      this.editor.search_step(this.space, needle.kind, needle.text, needle.fold, needle.backward, from),
    );
  }

  /** Put bytes where a match was found. */
  replaceAt(at: number, len: number, bytes: Uint8Array): void {
    if (this.refusesEdit()) return;
    this.editor.replace_at(at, len, bytes);
    this.notify();
  }

  /** Fold the edits that follow into one undo step. */
  beginBatch(): void {
    this.editor.begin_batch();
  }

  endBatch(): void {
    this.editor.end_batch();
  }

  /**
   * Which fields settled the shape of the one at `path`, and where this one
   * points if it holds an offset. Usually empty: most fields are placed and
   * sized by the template outright.
   */
  origins(path: readonly number[]): TemplateReply<Origin[]> {
    return this.handleReply<Origin[]>(this.editor.origins(this.space, Uint32Array.from(path)));
  }

  /**
   * The same question asked of every field under `path` at once: the fields of
   * the subtree and the connections between them, ready to be laid out.
   *
   * `limit` caps the fields. The walk is breadth-first, so what a cap keeps is
   * the top of the format rather than one deep spine of it, and the answer says
   * how many fields it left out.
   */
  graph(path: readonly number[], limit: number): TemplateReply<FieldGraph> {
    return this.handleReply<FieldGraph>(this.editor.graph(this.space, Uint32Array.from(path), limit));
  }

  /**
   * The HDF5 version 1 B-tree the field at `path` belongs to, walked.
   *
   * `path` is where the cursor is, which is usually not a node of a tree; the
   * core works out which tree that means. Null for a file with no such tree,
   * and for every format that is not HDF5.
   *
   * `limit` caps the nodes walked. A group holding a million links has a
   * quarter of a million link tables, and the reply says how many children it
   * left out rather than growing without end.
   */
  btree(path: readonly number[], limit: number): TemplateReply<Tree | null> {
    return this.handleReply<Tree | null>(this.editor.btree(this.space, Uint32Array.from(path), limit));
  }

  /**
   * The relationships behind the shape of the field at `path`, written out
   * with the numbers in them. Empty for a field the template placed and sized
   * outright, and for one whose expression the core has no notation for.
   */
  relations(path: readonly number[]): TemplateReply<Relation[]> {
    return this.handleReply<Relation[]>(this.editor.relations(this.space, Uint32Array.from(path)));
  }

  /**
   * How the field at `path` was placed, and how its length was settled.
   *
   * Answered for every field, which is what tells it from `origins`: a `u32` in
   * a header has no origins and is still where it is for a reason, and the
   * reason is that the field in front of it ended there.
   */
  shape(path: readonly number[]): TemplateReply<Shape> {
    return this.handleReply<Shape>(this.editor.shape(this.space, Uint32Array.from(path)));
  }

  /**
   * What the field at `path` checks, or null when it checks nothing.
   *
   * Cheap enough to ask of every field the cursor lands on: no byte of the run
   * it covers is read. `covered_bytes` is what to decide on before calling
   * `runCheck`, which does the reading.
   */
  checkOf(path: readonly number[]): TemplateReply<FieldCheck | null> {
    return this.handleReply<FieldCheck | null>(this.editor.check_of(this.space, Uint32Array.from(path)));
  }

  /**
   * The moment the field at `path` means, or null when it means none.
   *
   * Cheap enough to ask of every field the cursor lands on: the field's own
   * bytes are read, which the value row has already paid for, and nothing else
   * is. The stored number is not in the answer, because it is on the value row
   * already and this is an addition to it.
   */
  timeOf(path: readonly number[]): TemplateReply<FieldTime | null> {
    return this.handleReply<FieldTime | null>(this.editor.time_of(this.space, Uint32Array.from(path)));
  }

  /**
   * Take the checksum at `path` and compare it with what the file wrote. Null
   * when the field checks nothing.
   *
   * This reads the covered bytes, and unpacks the covered stream where the sum
   * is over what a run comes to rather than over the run, so it is asked for
   * rather than done on the way past. A run whose bytes have not arrived is
   * `pending` like any other read; a check that cannot be made at all is an
   * error with the reason in it.
   */
  runCheck(path: readonly number[]): TemplateReply<Verdict | null> {
    return this.handleReply<Verdict | null>(this.editor.run_check(this.space, Uint32Array.from(path)));
  }

  /** What the type at `path` permits: enum values, magic bytes, flag bits. */
  /** `atBits` is where the cursor is; only a block of packed weights uses it,
   *  to say which weight the reader is standing on. */
  typeInfo(path: readonly number[], atBits = -1): TemplateReply<TypeInfo> {
    return this.handleReply<TypeInfo>(this.editor.type_info(this.space, Uint32Array.from(path), atBits));
  }

  /**
   * What tool produced this file, from the Detect It Easy signature rules.
   *
   * Which rules are worth fetching depends on what the file is, and both
   * bundles are only worth fetching for the files they describe. A DOS
   * executable is asked about at its entry point; a .COM has no header to say
   * it is one, so the format's own limit stands in: it is loaded whole into a
   * single 64 KiB segment, and anything larger is not one.
   */
  async detectTools(identified: boolean): Promise<ToolMatch[]> {
    const n = Math.min(IDENTIFY_WINDOW, this.lengthBytes);
    if (n === 0) return [];
    await this.ensureRange(0, n);
    const first = this.read(0, n);
    if (!first.complete) return [];
    let bytes = first.bytes;
    const bundles: string[] = [];
    const mz = bytes[0] === 0x4d && bytes[1] === 0x5a;
    if (mz) {
      // Both start MZ. Which rules apply is decided by whether there is a PE
      // header where the DOS header points, the same question the built-in
      // sniffer asks.
      const byteAt = (i: number): number => bytes[i] ?? 0;
      const at = byteAt(0x3c) + (byteAt(0x3d) << 8) + (byteAt(0x3e) << 16) + byteAt(0x3f) * 0x1000000;
      const pe = at + 4 <= bytes.length && byteAt(at) === 0x50 && byteAt(at + 1) === 0x45 && byteAt(at + 2) === 0 && byteAt(at + 3) === 0;
      bundles.push(pe ? "pe.sig" : "msdos.sig");
      // A DOS program is asked about further in than the 64 KiB an unknown
      // format is identified from. A Windows one is not: its rules read the
      // section table, which is in the header either way.
      const want = Math.min(DOS_WINDOW, this.lengthBytes);
      if (!pe && want > n) {
        await this.ensureRange(0, want);
        const wider = this.read(0, want);
        if (wider.complete) bytes = wider.bytes;
      }
    }
    // A .COM is bytes with no header at all, so nothing but its size and its
    // name suggest one. Asking for every unknown small file would be worse.
    if (!mz && this.lengthBytes <= COM_LIMIT && (!identified || this.name.toLowerCase().endsWith(".com"))) {
      bundles.push("com.sig");
    }
    const out: ToolMatch[] = [];
    for (const bundle of bundles) {
      const rules = await fetchRules(bundle);
      if (rules === null) continue;
      out.push(...(JSON.parse(this.editor.detect_tools(rules, bytes)) as ToolMatch[]));
    }
    return out;
  }

  /** Path of the deepest template field covering `bitOffset`. */
  locate(bitOffset: number): TemplateReply<number[]> {
    return this.handleReply<number[]>(this.editor.locate(this.space, bitOffset));
  }

  /**
   * Write `text` into the field at `path`, encoded as that field's type. A
   * fixed-width field is written over in place, so nothing after it shifts;
   * a JSON value is as long as the text it holds, so a longer or shorter one
   * moves everything after it along, as one undo step.
   * A "pending" reply means the field's position could not be worked out yet;
   * the chunks are on their way and the caller should ask again.
   */
  writeNode(path: readonly number[], text: string): TemplateReply<WrittenRange> {
    if (this.refusesEdit()) return { status: "error", message: UNPACKED.readOnly };
    const r = this.handleReply<WrittenRange>(this.editor.write_node(this.space, Uint32Array.from(path), text));
    if (r.status === "ok") this.notify();
    return r;
  }

  /** Synchronous read. Missing chunks are zero and fetched in the background. */
  read(at: number, len: number): ReadResult {
    const bytes = new Uint8Array(len);
    const missing = this.editor.read_bytes(this.space, at, bytes);
    for (const chunk of missing) this.fetchChunk(chunk);
    return { bytes, complete: missing.length === 0 };
  }

  /** Synchronous read of `nBits` starting at any bit, packed MSB first. */
  readBits(atBit: number, nBits: number): ReadResult {
    const bytes = new Uint8Array(Math.ceil(nBits / 8));
    const missing = this.editor.read_bits(this.space, atBit, nBits, bytes);
    for (const chunk of missing) this.fetchChunk(chunk);
    return { bytes, complete: missing.length === 0 };
  }

  /**
   * Read `count` chunks from `from` onwards in a single go, skipping any that
   * are already here or already on their way. One read of three megabytes
   * costs about what one read of sixty-four kilobytes costs, and the file is
   * being walked forwards, so this is most of what makes a large file open in
   * seconds rather than minutes.
   */
  private fetchRun(from: number, count: number): void {
    if (this.space !== 0) return;
    const total = Math.ceil(this.blob.size / CHUNK_SIZE);
    let start = from;
    while (start < from + count && start < total && (this.inflight.has(start) || this.editor.has_chunk(this.space, start))) {
      start += 1;
    }
    let end = start;
    while (end < from + count && end < total && !this.inflight.has(end) && !this.editor.has_chunk(this.space, end)) {
      end += 1;
    }
    if (end <= start) return;
    for (let c = start; c < end; c++) this.inflight.add(c);
    const at = start * CHUNK_SIZE;
    void this.blob
      .slice(at, Math.min(end * CHUNK_SIZE, this.blob.size))
      .arrayBuffer()
      .then((buf) => {
        const bytes = new Uint8Array(buf);
        for (let c = start; c < end; c++) {
          const off = (c - start) * CHUNK_SIZE;
          if (off >= bytes.length) break;
          this.editor.feed_chunk(c, bytes.subarray(off, Math.min(off + CHUNK_SIZE, bytes.length)));
        }
      })
      .finally(() => {
        for (let c = start; c < end; c++) this.inflight.delete(c);
        this.notify();
      });
  }

  private fetchChunk(chunk: number): void {
    // An unpacked stream has no file behind it: every byte it has was decoded
    // in one go and none of it can arrive later.
    if (this.space !== 0) return;
    // Reading ahead can run off the end, and a chunk past the end is not a
    // chunk: feeding an empty one would look like bytes that are all zero.
    if (chunk * CHUNK_SIZE >= this.blob.size) return;
    if (this.inflight.has(chunk)) return;
    this.inflight.add(chunk);
    const start = chunk * CHUNK_SIZE;
    void this.blob
      .slice(start, Math.min(start + CHUNK_SIZE, this.blob.size))
      .arrayBuffer()
      .then((buf) => {
        this.editor.feed_chunk(chunk, new Uint8Array(buf));
      })
      .finally(() => {
        this.inflight.delete(chunk);
        this.notify();
      });
  }

  /**
   * What this file turned out to be a dump of, or null. Only asked of a file
   * small enough to read in one go: a dump is text, and text that is a dump of
   * anything worth opening is a few times the size of what it describes.
   */
  async dumpScan(): Promise<DumpScan | null> {
    if (this.lengthBytes === 0 || this.lengthBytes > DUMP_LIMIT) return null;
    await this.ensureRange(0, this.lengthBytes);
    const got = this.read(0, this.lengthBytes);
    if (!got.complete) return null;
    const text = dump_scan(got.bytes);
    return text === "" ? null : (JSON.parse(text) as DumpScan);
  }

  /** The bytes a dump describes, ready to open as a document of their own. */
  async dumpBytes(): Promise<Uint8Array> {
    await this.ensureRange(0, this.lengthBytes);
    const got = this.read(0, this.lengthBytes);
    return got.complete ? dump_bytes(got.bytes) : new Uint8Array();
  }

  /**
   * Typed text as the bytes it is in the file's encoding, or the character the
   * encoding has no room for. `chosen` is what the reader picked, `settled` is
   * what the file was read as when they picked nothing.
   */
  encodeText(chosen: string, settled: string, text: string): TextEncoded {
    return JSON.parse(text_encode(chosen, settled, text)) as TextEncoded;
  }

  /**
   * What a run of bytes says, read every way text can be read. Empty while the
   * bytes are still being fetched, in which case ask again once they are.
   */
  selectionText(atByte: number, len: number, first: string, pageA: string, pageB: string): SelectionText | null {
    const got = this.editor.selection_text(this.space, atByte, len, first, pageA, pageB);
    return got === "" ? null : (JSON.parse(got) as SelectionText);
  }

  /**
   * The same run of bytes written as a string literal in `lang`, or null while
   * the bytes are still being fetched. Cut to the same length the readings
   * are, so both are saying the same run.
   */
  selectionLiteral(atByte: number, len: number, lang: string): string | null {
    const got = this.editor.selection_literal(this.space, atByte, len, lang);
    return got === "" ? null : got;
  }

  /** How the file reads as text. Pass "" to let the file decide. */
  async textReading(encoding: string): Promise<TextReading> {
    await this.ensureRange(0, Math.min(64, this.lengthBytes));
    return JSON.parse(this.editor.text_reading(encoding)) as TextReading;
  }

  /**
   * Lines starting at `from`, which must be where a line starts. A window
   * needing chunks that are not here yet asks for them and tries again.
   *
   * The core stops at the first chunk it has not got, so a batch of lines
   * spanning a lot of file takes as many rounds as it takes: a screenful of
   * four-kilobyte lines is a megabyte, which is a round for every window of
   * it. What is not allowed is going round without getting anywhere, so a
   * round that asks for the same chunks as the one before it is where this
   * gives up rather than spins.
   */
  async textWindow(encoding: string, from: number, want: number): Promise<TextWindow> {
    let asked = "";
    for (let go = 0; go < TEXT_ROUNDS; go++) {
      const w = JSON.parse(this.editor.text_window(this.space, encoding, from, want)) as TextWindow;
      if (w.missing.length === 0) return w;
      const now = w.missing.join(",");
      if (now === asked) return { lines: [], missing: w.missing, next: from };
      asked = now;
      await Promise.all(w.missing.map((c) => this.ensureRange(c * CHUNK_SIZE, CHUNK_SIZE)));
    }
    return { lines: [], missing: [], next: from };
  }

  /**
   * Where every line in `[from, to)` starts, `from` included, which must be
   * where a line starts. The core caps how far one call scans, so `next` is
   * what the caller carries on from rather than `to`.
   */
  async textIndex(encoding: string, from: number, to: number): Promise<TextIndex> {
    let asked = "";
    for (let go = 0; go < TEXT_ROUNDS; go++) {
      const packed = this.editor.text_index(this.space, encoding, from, to);
      const missing = packed[4] ?? 0;
      if (missing === 0) {
        return {
          next: packed[0] ?? from,
          lf: packed[1] ?? 0,
          cr: packed[2] ?? 0,
          crlf: packed[3] ?? 0,
          starts: packed.subarray(5),
        };
      }
      const chunks = Array.from(packed.subarray(5, 5 + missing));
      const now = chunks.join(",");
      if (now === asked) return { starts: new Float64Array(), next: from, lf: 0, cr: 0, crlf: 0 };
      asked = now;
      await Promise.all(chunks.map((c) => this.ensureRange(c * CHUNK_SIZE, CHUNK_SIZE)));
    }
    return { starts: new Float64Array(), next: from, lf: 0, cr: 0, crlf: 0 };
  }

  /** Where the line holding `at` starts, and `lines` line starts back from it. */
  async textBack(encoding: string, at: number, lines: number): Promise<TextBack> {
    for (let go = 0; go < 3; go++) {
      const b = JSON.parse(this.editor.text_back(this.space, encoding, at, lines)) as TextBack;
      if (b.missing.length === 0) return b;
      await Promise.all(b.missing.map((c) => this.ensureRange(c * CHUNK_SIZE, CHUNK_SIZE)));
    }
    return { start: at, back: at, missing: [] };
  }

  /**
   * Strings found from `from` onwards, for a file with no template and no
   * encoding of its own. `from` is zero or whatever the last scan said to
   * carry on from, never an offset picked out of the air.
   *
   * The same retry loop as `textWindow`: the core stops at the first chunk it
   * has not got, and a round that asks for the same chunks as the one before
   * it gives up rather than spins.
   */
  async stringsScan(from: number, want: number, opts: StringScanOpts): Promise<StringScan> {
    const encodings = opts.encodings.join(",");
    if (encodings === "") return { hits: [], missing: [], next: this.lengthBytes };
    let asked = "";
    for (let go = 0; go < TEXT_ROUNDS; go++) {
      const s = JSON.parse(
        this.editor.strings_scan(this.space, from, want, opts.minChars, encodings),
      ) as StringScan;
      if (s.missing.length === 0) return s;
      const now = s.missing.join(",");
      if (now === asked) return { hits: [], missing: s.missing, next: from };
      asked = now;
      await Promise.all(s.missing.map((c) => this.ensureRange(c * CHUNK_SIZE, CHUNK_SIZE)));
    }
    return { hits: [], missing: [], next: from };
  }

  /** Resolve once every chunk covering [at, at+len) is loaded. */
  async ensureRange(at: number, len: number): Promise<void> {
    const first = Math.floor(at / CHUNK_SIZE);
    const last = Math.floor((at + len - 1) / CHUNK_SIZE);
    const waits: Promise<void>[] = [];
    for (let c = first; c <= last; c++) {
      if (!this.editor.has_chunk(this.space, c)) {
        waits.push(this.loadChunk(c).catch((e: unknown) => {
          throw new ReadFailure(c * CHUNK_SIZE, Math.min(CHUNK_SIZE, this.blob.size - c * CHUNK_SIZE), e);
        }));
      }
    }
    await Promise.all(waits);
  }

  private loadChunk(chunk: number): Promise<void> {
    const start = chunk * CHUNK_SIZE;
    return Promise.resolve(this.blob.slice(start, Math.min(start + CHUNK_SIZE, this.blob.size)).arrayBuffer()).then(
      (buf) => {
        this.editor.feed_chunk(chunk, new Uint8Array(buf));
      },
    );
  }

  /**
   * Build the saved file as a Blob of lazy parts. Unchanged stretches of the
   * original are referenced, not copied, so this works for any file size.
   */
  async buildOutput(): Promise<Blob> {
    const plan = this.editor.save_plan();
    const add = this.editor.add_bytes();
    const parts: BlobPart[] = [];
    for (let i = 0; i < plan.length; i += 4) {
      const kind = plan[i] ?? 0;
      const docOff = plan[i + 1] ?? 0;
      const srcOff = plan[i + 2] ?? 0;
      const len = plan[i + 3] ?? 0;
      if (kind === 0) {
        const part = this.blob.slice(srcOff, srcOff + len);
        if (part instanceof Blob) {
          parts.push(part); // lazy reference, nothing copied
        } else {
          // Non-Blob sources (dev synthetic files) must be read; keep pieces bounded.
          const STEP = 16 * 1024 * 1024;
          for (let o = 0; o < len; o += STEP) {
            parts.push(await this.blob.slice(srcOff + o, srcOff + Math.min(len, o + STEP)).arrayBuffer());
          }
        }
      } else if (kind === 1) {
        parts.push(add.slice(srcOff, srcOff + len));
      } else {
        // Bit-unaligned stretch: read it through the piece table in chunks.
        const STEP = 4 * 1024 * 1024;
        for (let o = 0; o < len; o += STEP) {
          const n = Math.min(STEP, len - o);
          await this.ensureRange(docOff + o, n);
          let { bytes, complete } = this.read(docOff + o, n);
          if (!complete) {
            // Chunks were evicted between load and read; one retry covers it.
            await this.ensureRange(docOff + o, n);
            ({ bytes, complete } = this.read(docOff + o, n));
          }
          if (!complete) throw new ReadFailure(docOff + o, n, "Chunks were evicted before they could be used.");
          parts.push(new Uint8Array(bytes) as Uint8Array<ArrayBuffer>);
        }
      }
    }
    return new Blob(parts, { type: "application/octet-stream" });
  }

  overwrite(at: number, data: Uint8Array): void {
    if (this.refusesEdit()) return;
    this.editor.overwrite_bytes(at, data);
    this.notify();
  }
  /** Overwrite that joins the previous edit's undo step. */
  amendOverwrite(at: number, data: Uint8Array): void {
    if (this.refusesEdit()) return;
    this.editor.amend_overwrite_bytes(at, data);
    this.notify();
  }
  insert(at: number, data: Uint8Array): void {
    if (this.refusesEdit()) return;
    this.editor.insert_bytes(at, data);
    this.notify();
  }
  delete(at: number, n: number): void {
    if (this.refusesEdit()) return;
    this.editor.delete_bytes(at, n);
    this.notify();
  }
  overwriteBits(atBit: number, data: Uint8Array, n: number): void {
    if (this.refusesEdit()) return;
    this.editor.overwrite_bits(atBit, data, n);
    this.notify();
  }
  insertBits(atBit: number, data: Uint8Array, n: number): void {
    if (this.refusesEdit()) return;
    this.editor.insert_bits(atBit, data, n);
    this.notify();
  }
  deleteBits(atBit: number, n: number): void {
    if (this.refusesEdit()) return;
    this.editor.delete_bits(atBit, n);
    this.notify();
  }
  undo(): void {
    if (this.refusesEdit()) return;
    if (this.editor.undo()) this.notify();
  }
  redo(): void {
    if (this.refusesEdit()) return;
    if (this.editor.redo()) this.notify();
  }
}

// The words for the core's format profile: what each row of it is called, and
// the sentences the report writes from its facts. The core hands over keys
// (`number`, `length-field`, `at-end`) and this file is the only place they
// become English. See `eval/profile.rs` for every key.

import type { Profile, ProfileRow } from "./coredata.ts";
import { counted, listText } from "./text.ts";

/** What each category of the profile is, as a group heading. */
export const CATEGORY: Readonly<Record<string, string>> = {
  number: "Numbers",
  text: "Text",
  varint: "Variable-length numbers",
  "bit-field": "Bit fields",
  enum: "Numbers with named values",
  flags: "Flags",
  magic: "Magic numbers",
  computed: "Values worked out from other fields",
  opaque: "Data the template does not divide further",
  padding: "Padding",
  checksum: "Checksums",
  codec: "Compressed streams",
  placement: "How each field is found",
  sizing: "How each field's length is set",
};

/** The order the categories are shown in: values first, then how the parts
 *  are found and sized. */
export const CATEGORY_ORDER = ["number", "text", "varint", "bit-field", "enum", "flags", "magic", "computed", "opaque", "padding", "checksum", "codec", "placement", "sizing"];

/** The categories that say what a value is, as opposed to where it is. */
export const VALUE_CATEGORIES = new Set(CATEGORY_ORDER.slice(0, 12));

const ENCODING: Readonly<Record<string, string>> = {
  utf8: "UTF-8",
  ascii: "ASCII",
  latin1: "Latin-1",
  cp437: "code page 437",
  utf16le: "UTF-16LE",
  utf16be: "UTF-16BE",
  bom: "Unicode with a byte-order mark",
  unknown: "unknown encoding",
  p8scii: "P8SCII",
  ebcdic: "EBCDIC",
};

const TEXT_RULE: Readonly<Record<string, string>> = {
  fixed: "fixed length",
  padded: "padded to a fixed length",
  terminated: "ends at a terminator",
  token: "ends at a delimiter",
  "length-field": "length given by another field",
  "to-end": "runs to the end of its part",
};

const VARINT: Readonly<Record<string, string>> = {
  leb128: "LEB128",
  sleb128: "signed LEB128",
  zigzag: "zigzag-encoded varint",
  vlq: "VLQ",
  "ebml-id": "EBML element ID",
  "ebml-size": "EBML size",
  sqlite: "SQLite varint",
  "7z": "7z number",
};

const OPAQUE: Readonly<Record<string, string>> = {
  bytes: "bytes",
  json: "JSON text",
  pickle: "Python pickle",
  "entropy-coded": "entropy-coded bits",
};

const CHECKSUM: Readonly<Record<string, string>> = {
  crc32: "CRC-32",
  crc16: "CRC-16",
  sum8: "8-bit sum",
  sum: "sum of the bytes",
  sha1: "SHA-1",
  adler32: "Adler-32",
};

const PLACEMENT: Readonly<Record<string, string>> = {
  follows: "right after the field before it",
  element: "one after another in a list",
  overlap: "over the same bytes as another field",
  "pointer-list": "by a list of offsets",
  chain: "by a link in the element before it",
  gather: "by a descriptor elsewhere in the file",
  "at-start": "at an offset from the start of the file",
  "at-parent": "at an offset from the start of its parent",
  "at-end": "at an offset counted from the end of the file",
  "at-other": "at an offset from somewhere else",
  stream: "at the start of an unpacked stream",
  trace: "where a decoder read it",
};

const SIZING: Readonly<Record<string, string>> = {
  fixed: "a length the format fixes",
  "length-field": "a length worked out from other fields",
  count: "a count of elements",
  terminator: "ends at a terminator",
  "to-end": "fills the rest of its part",
  fields: "as long as the fields inside it",
  offsets: "a list of offsets",
  "self-delimiting": "its own bytes say where it ends",
  decoder: "as much as a decoder read",
  alignment: "padding up to an alignment",
  other: "some other way",
};

function endian(order: string): string {
  return order === "little" ? ", little-endian" : order === "big" ? ", big-endian" : "";
}

/** What one row of the profile is, in words. */
export function rowLabel(r: ProfileRow): string {
  switch (r.category) {
    case "number": {
      const bits = r.width > 0 ? `${r.width}-bit ` : "";
      const set = r.width > 0 ? "" : ", width set by the file";
      switch (r.kind) {
        case "unsigned":
          return `${bits}unsigned integer${endian(r.order)}${set}`;
        case "signed":
          return `${bits}signed integer${endian(r.order)}${set}`;
        case "sign-magnitude":
          return `${bits}sign-magnitude integer${endian(r.order)}${set}`;
        case "float":
          if (r.detail === "bfloat16") return `bfloat16${endian(r.order)}`;
          if (r.detail === "x87") return `80-bit x87 extended float${endian(r.order)}`;
          if (r.detail === "e4m3" || r.detail === "e5m2") return `8-bit float (${r.detail.toUpperCase()})`;
          if (r.detail === "ibm") return `${bits}IBM float${endian(r.order)}`;
          return `${bits}float${endian(r.order)}`;
        case "fixed-point":
          return `${bits}fixed point (${r.detail})${endian(r.order)}`;
        case "digits":
          return `number written in digits, base ${r.detail}`;
        default:
          return `${bits}${r.kind}${endian(r.order)}`;
      }
    }
    case "text":
      return `${ENCODING[r.detail] ?? r.detail}, ${TEXT_RULE[r.kind] ?? r.kind}`;
    case "varint":
      return VARINT[r.kind] ?? r.kind;
    case "bit-field":
      return "fields narrower than a byte or off a byte boundary";
    case "enum":
      return "numbers the template names";
    case "flags":
      return "numbers whose bits the template names";
    case "magic":
      return "fixed bytes the format requires";
    case "computed":
      return r.kind === "text" ? "text worked out from other fields" : "number worked out from other fields";
    case "opaque":
      return r.kind === "code" ? `machine code (${r.detail})` : (OPAQUE[r.kind] ?? r.kind);
    case "padding":
      return r.width > 0 ? `padding to a multiple of ${counted(r.width, "byte")}` : "padding";
    case "checksum":
      return CHECKSUM[r.kind] ?? r.kind;
    case "codec":
      return r.kind;
    case "placement":
      return PLACEMENT[r.kind] ?? r.kind;
    case "sizing":
      return SIZING[r.kind] ?? r.kind;
    default:
      return r.kind === "" ? r.category : `${r.category}: ${r.kind}`;
  }
}

/** The key a row is matched on between the template's profile and the
 *  file's. */
export function rowKey(r: ProfileRow): string {
  return `${r.category}\u0000${r.kind}\u0000${r.width}\u0000${r.order}\u0000${r.detail}`;
}

/** Which byte order the file's numbers are in, as a short sentence, or null
 *  where no number is wider than a byte. */
function byteOrder(rows: readonly ProfileRow[]): string | null {
  let little = 0;
  let big = 0;
  for (const r of rows) {
    if (r.category !== "number" || r.fields === 0) continue;
    if (r.order === "little") little += r.fields;
    if (r.order === "big") big += r.fields;
  }
  if (little === 0 && big === 0) return null;
  if (big === 0) return "Little-endian.";
  if (little === 0) return "Big-endian.";
  return `Both byte orders: ${counted(little, "little-endian number")} and ${counted(big, "big-endian number")}.`;
}

/** The widths of one kind of number the file holds, smallest first. */
function widths(rows: readonly ProfileRow[], kinds: readonly string[]): number[] {
  const set = new Set<number>();
  for (const r of rows) if (r.category === "number" && r.fields > 0 && kinds.includes(r.kind) && r.width >= 8) set.add(r.width);
  return [...set].sort((a, b) => a - b);
}

function kindsOf(rows: readonly ProfileRow[], category: string, names: Readonly<Record<string, string>>): string[] {
  return [...new Set(rows.filter((r) => r.category === category && r.fields > 0).map((r) => names[r.kind] ?? r.kind))];
}

/**
 * The line of landmarks under the facts table: only what helps a reader find
 * their way around the bytes. Byte order, the widths of the numbers, the
 * variable-length numbers, how parts are found, checksums and compression,
 * each a short sentence, each only when the file has any.
 */
export function landmarks(p: Profile): string[] {
  const out: string[] = [];
  const order = byteOrder(p.rows);
  if (order !== null) out.push(order);
  const ints = widths(p.rows, ["unsigned", "signed", "sign-magnitude"]);
  if (ints.length > 0) out.push(`Integers are ${listText(ints.map(String))} bits.`);
  const floats = widths(p.rows, ["float"]);
  if (floats.length > 0) out.push(`Floats are ${listText(floats.map(String))} bits.`);
  const varints = kindsOf(p.rows, "varint", VARINT);
  if (varints.length > 0) out.push(`Variable-length numbers: ${listText(varints)}.`);
  const f = p.facts;
  if (f.lengths_before > 0) out.push(`${sentence(counted(f.lengths_before, "field"))} give the length or count of a later field.`);
  if (f.placed > 0) out.push(`${sentence(counted(f.placed, "field"))} ${f.placed === 1 ? "is" : "are"} found by an offset.`);
  if (f.from_end > 0) out.push(`${sentence(counted(f.from_end, "field"))} ${f.from_end === 1 ? "is" : "are"} found from the end of the file.`);
  const sums = kindsOf(p.rows, "checksum", CHECKSUM);
  if (sums.length > 0) out.push(`Checked with ${listText(sums)}.`);
  const codecs = kindsOf(p.rows, "codec", {});
  if (codecs.length > 0) out.push(`Compressed with ${listText(codecs)}.`);
  return out;
}

/**
 * What a reader and a writer of the format have to know, one fact to a
 * sentence, from the counts the core took over this file.
 */
export function readingWriting(p: Profile): string[] {
  const f = p.facts;
  const out: string[] = [];
  if (f.every_field_follows === true) out.push("A reader can go from the start of the file to the end without going back.");
  if (f.from_end > 0) out.push(`A reader has to start at the end: ${counted(f.from_end, "field")} ${f.from_end === 1 ? "is" : "are"} found from the end of the file.`);
  if (f.backward > 0) {
    out.push(`A reader has to go back: ${counted(f.backward, "offset")} ${f.backward === 1 ? "points" : "point"} to earlier bytes.`);
    out.push(f.backward === 1 ? "A writer can write the part it points to first, then the offset." : "A writer can write the parts they point to first, then the offsets.");
  }
  if (f.forward > 0) out.push(`${sentence(counted(f.forward, "offset"))} ${f.forward === 1 ? "points" : "point"} ahead, so a writer has to know where those parts go before writing them, or come back to fill the offsets in.`);
  if (f.lengths_before > 0) out.push(`${sentence(counted(f.lengths_before, "length"))} ${f.lengths_before === 1 ? "comes" : "come"} before what ${f.lengths_before === 1 ? "it sizes" : "they size"}, so a writer has to know each size before writing, or come back to fill it in.`);
  if (f.lengths_after > 0) out.push(`${sentence(counted(f.lengths_after, "length"))} ${f.lengths_after === 1 ? "comes" : "come"} after what ${f.lengths_after === 1 ? "it sizes" : "they size"}.`);
  return out;
}

function sentence(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

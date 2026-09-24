// Every string the report view shows, in one place.
//
// Headings are generated and each states a fact with its number, never a fixed
// name for the section: "Where the 5,770 bytes go", not "Overview". The core
// hands over facts and this file writes the words, as `time.rs` and
// `problem.rs` have it for the rest of the app.

import { formatBytes, percentText } from "../format.ts";
import type { ExtentCheck } from "./coredata.ts";

/** `1 byte`, `5,770 bytes`. */
export function bytesText(n: number): string {
  return n === 1 ? "1 byte" : `${n.toLocaleString()} bytes`;
}

/** A size in bits, in bytes where it is whole bytes. */
export function bitsText(bits: number): string {
  if (bits % 8 === 0) return bytesText(bits / 8);
  return bits === 1 ? "1 bit" : `${bits.toLocaleString()} bits`;
}

/** A file's size as a reader says it: the exact count for a small file, the
 *  rounded size first for a large one, with the count after it. */
export function fileSizeText(n: number): string {
  return n < 10 * 1024 ? bytesText(n) : `${formatBytes(n)} (${bytesText(n)})`;
}

/** `48% of the file`. */
export function shareOfFile(part: number, whole: number): string {
  return `${percentText(part, whole)} of the file`;
}

/** A count and its noun, with the noun's plural. */
export function counted(n: number, noun: string): string {
  return `${n.toLocaleString()} ${n === 1 ? noun : pluralOf(noun)}`;
}

export function pluralOf(noun: string): string {
  if (/[^aeiou]y$/.test(noun)) return `${noun.slice(0, -1)}ies`;
  if (/(s|x|z|ch|sh)$/.test(noun)) return `${noun}es`;
  return `${noun}s`;
}

/** Items joined with commas and a final "and", serial comma included. */
export function listText(items: readonly string[]): string {
  if (items.length <= 1) return items[0] ?? "";
  if (items.length === 2) return `${items[0]} and ${items[1]}`;
  return `${items.slice(0, -1).join(", ")}, and ${items[items.length - 1]}`;
}

/** What the upright lines on the extent figure mark. */
function extentMarks(parent: boolean, file: boolean): string {
  if (parent && file) return "The lines mark the end of its parent and the end of the file.";
  if (parent) return "The line marks the end of its parent.";
  if (file) return "The line marks the end of the file.";
  return "";
}

export const RV = {
  /** The main view's button, beside Hex, Listing, Text, Strings, and Diagram. */
  button: "Report",
  buttonTitle: "A report on this file: what it holds, where its bytes go, and what is wrong with it",
  regionLabel: "Report on this file",

  // ----- 1. the title line -----
  /** What identified the file, after its size and format on the title line. */
  identifiedByTemplate: (label: string): string => `Identified by the ${label} template`,
  identifiedByKaitai: (label: string): string => `Read with the Kaitai Struct description ${label}`,
  identifiedByImhex: (label: string): string => `Read with the ImHex pattern ${label}`,
  identifiedByRule: "Identified by a file(1) rule",
  identifiedBySignature: "Identified by its signature bytes",
  notIdentified: "Format not identified",
  /** The link to the format's article, for a reader who wants more than the
   *  sentences under the title. */
  wikipediaTitle: (article: string): string => `Wikipedia: ${article}`,

  // ----- 2. what the format is -----
  /** Shown when the core has no sentences for the format but the file(1)
   *  rules described the file. */
  fileRuleSays: "The file(1) rules describe it as:",

  // ----- 3. the facts table -----
  factSize: "Size",
  factParts: "Parts",
  factPicture: "Picture",
  factFindings: "Findings",
  factDescribed: "Described by the template",
  pictureSize: (w: number, h: number): string => `${w.toLocaleString()} × ${h.toLocaleString()} pixels`,
  /** How many parts, and of how many kinds where that is fewer: the ledger
   *  below names them. */
  partsCount: (parts: number, kinds: number): string =>
    kinds < parts ? `${counted(parts, "part")} of ${counted(kinds, "kind")}` : counted(parts, "part"),
  noFindings: "None found",
  described: (covered: number, total: number, done: boolean): string =>
    `${bytesText(covered)} of ${bytesText(total)} (${percentText(covered, total)})${done ? "" : " so far"}`,

  // ----- 4. findings -----
  findingsHeading: (parts: readonly string[]): string => sentenceCase(listText(parts)),
  findingInvalid: (n: number): string => counted(n, "invalid value"),
  findingUndefined: (n: number): string => counted(n, "undefined value"),
  findingRefused: (n: number): string => counted(n, "stream that did not unpack"),
  findingGap: (n: number): string => counted(n, "unmapped range"),
  findingExtent: (n: number): string => counted(n, "length that does not match its part"),
  /** The quiet line the values without a name open from. */
  undefinedLine: (n: number): string => (n === 1 ? "1 value the template has no name for" : `${n.toLocaleString()} values the template has no name for`),
  rootFailed: (why: string): string => `The file would not read: ${why}`,
  /** What is wrong with a length, after the part's name and address. */
  extentVerdict: (c: ExtentCheck): string => {
    const len = c.role === "length";
    const stated = c.stated ?? 0;
    const read = c.read ?? 0;
    switch (c.verdict) {
      case "past-file":
        return len
          ? `Its length field says it ends ${bitsText(Math.max(0, c.offset_bits + stated - c.space_bits))} past the end of the file`
          : `Its count asks for ${counted(stated, "element")}, and ${read.toLocaleString()} fit before the end of the file`;
      case "past-parent":
        return len
          ? `Its length field says it ends ${bitsText(Math.max(0, stated - c.room_bits))} past the end of its parent`
          : `Its count asks for ${counted(stated, "element")}, and ${read.toLocaleString()} fit in its parent`;
      case "stretched":
        return len
          ? `It runs ${bitsText(Math.max(0, read - stated))} past the length its length field gives`
          : `It holds ${counted(read, "element")}, more than the ${stated.toLocaleString()} its count gives`;
      case "short":
        if (!len) return `Its count says ${counted(stated, "element")}, and ${read.toLocaleString()} were read`;
        return c.content_bits === null
          ? "Its fields stop before the length its length field gives"
          : `Its fields stop ${bitsText(Math.max(0, stated - c.content_bits))} before the length its length field gives`;
      case "unreadable":
        return c.why === "" ? "It would not read" : `It would not read: ${c.why}`;
      default:
        return c.verdict;
    }
  },
  extentLengthField: "Length field",
  extentStated: "Stated length",
  extentRead: "Actual length",
  extentContent: "Its fields reach",
  extentRoom: "Room in its parent",
  extentStatedCount: "Stated count",
  extentReadCount: "Elements read",
  extentUnknown: "not worked out",
  elements: (n: number): string => counted(n, "element"),
  at: "at",
  extentAdjusted: "The template adds bytes the length field leaves out, so the field's own value is not the part's length.",
  extentParentEnds: "End of its parent",
  extentFileEnds: "End of the file",
  extentFigureLabel: "Where the length field says the part ends, and where it does end",
  extentCaption: (start: string, parent: boolean, file: boolean): string => `The part starts at ${start}. ${extentMarks(parent, file)}`.trim(),
  extentCaptionCut: (start: string, parent: boolean, file: boolean): string =>
    `Only the ends are drawn, at one scale. The part starts further left, at ${start}. ${extentMarks(parent, file)}`.trim(),
  gapNonzero: (bytes: number): string => `${bytesText(bytes)} that no field describes`,
  gapZeros: (bytes: number): string => `${bytesText(bytes)} of zeros that no field describes`,
  gapUnchecked: (bytes: number): string => `${bytesText(bytes)} that no field describes, not checked for zeros`,
  refused: (why: string): string =>
    why === "too-large" ? "Too large to unpack here" : why === "unaligned" ? "Does not start on a byte boundary, so it was not unpacked" : "Did not unpack",
  stored: "Stored",
  /** One finding in several places: `in 22 places: @0x6, @0x399, …`. */
  inPlaces: (n: number): string => `in ${n.toLocaleString()} places`,
  andMore: (n: number): string => `and ${n.toLocaleString()} more`,
  moreFindings: (n: number): string => `${counted(n, "more finding")} not listed`,

  // ----- 5. the content -----
  pictureHeading: (w: number, h: number): string => `The picture: ${w.toLocaleString()} × ${h.toLocaleString()} pixels`,
  pictureHeadingWait: "The picture, still decoding",
  pictureFailedHeading: "The picture",
  pictureFailed: "Your browser could not decode this picture.",
  pictureTooLarge: (size: string): string => `Not decoded here: the file is ${size}, more than the report decodes on its own.`,
  tableHeading: (count: number, rowWord: string, rate: number | null): string => {
    const base = counted(count, rowWord);
    if (rate === null || rate <= 0) return sentenceCase(base);
    const seconds = count / rate;
    return sentenceCase(`${base} at ${rate.toLocaleString()} a second, ${secondsText(seconds)}`);
  },
  firstRows: (shown: number, total: number, word: string): string =>
    `The first ${shown.toLocaleString()} of ${counted(total, word)}.`,
  openTable: "Open as a table",
  textHeading: (name: string, bytes: number): string => `The text of ${name}: ${bytesText(bytes)}`,
  textCut: (shown: number, total: number): string => `The first ${shown.toLocaleString()} of ${total.toLocaleString()} characters.`,
  plainTextHeading: (bytes: number): string => `The file as text: ${bytesText(bytes)}`,
  /** A container's entries, from its outline's own title and count. */
  outlineHeading: (title: string, summary: string): string => `${title}: ${summary}`,
  zipHeading: (n: number, unpacked: number, packed: number): string =>
    `${sentenceCase(counted(n, "file"))} in the archive: ${bytesText(unpacked)}, stored in ${bytesText(packed)}`,
  zipHeadingFirst: (n: number): string => `The first ${counted(n, "file")} in the archive`,
  zipMore: "The archive holds more files than these. The rail's Logical tab lists all of them.",
  moreEntries: (n: number): string => `${counted(n, "more entry")} not listed. The rail's Logical tab lists all of them.`,
  entryName: "Name",
  entrySize: "Size",
  entryPacked: "In the archive",
  entryMethod: "Method",
  entryRatio: "Ratio",
  entryWhat: "What it is",
  plainTextCut: (shown: number): string => `The first ${shown.toLocaleString()} characters, read as UTF-8. The Text view shows all of it.`,

  // ----- 6. where the bytes go -----
  bytesHeading: (n: number): string => `Where the ${n.toLocaleString()} bytes go`,
  mapCaptionLead: "The whole file in file order.",
  mapCaption:
    "The top row shows the parts, the middle row the fields, and the strip marks each byte as zero, text, or other. Past about 14 pixels a byte, the strip shows each byte in hex.",
  mapHint: "Scroll over the map to zoom, drag to pan, and double-click to reset. Hover for the part, the field, and the byte.",
  mapRange: (from: string, to: string): string => `Showing ${from} to ${to}`,
  /** In the fields row, when the window holds more fields than it can draw. */
  mapZoomForFields: "Too many fields to draw at this size. Zoom in to see them.",
  /** A stretch no field covers, as the listing names it. */
  gapName: "unmapped",
  /** A stretch holding more parts than the report lists, which it did not
   *  look into. */
  unexaminedName: "not examined",
  unexaminedBody: "The report stopped looking for parts here. The Listing view shows the fields in these bytes.",
  /** An element named like an element of another list, with its list. */
  inList: (name: string, list: string): string => `${name} (${list})`,
  mapByte: (at: string, hex: string, cls: string): string => `Byte at ${at}: ${hex}, ${cls}`,
  classZero: "zero",
  classText: "text",
  classOther: "other",
  classRepeat: "one repeated byte",
  classDense: "high entropy",
  classUnread: "not read yet",
  byteStripLabel: "Byte strip:",
  ledgerPart: "Part",
  ledgerStart: "Starts at",
  ledgerBytes: "Bytes",
  ledgerShare: "Share of file",
  ledgerWhat: "What it is",
  /** Brackets round the share, since a part's name can have a comma of its
   *  own: `sos, start of scan`. */
  ledgerCaptionLead: (largest: string, share: string): string => `Largest part: ${largest} (${share} of the file).`,
  ledgerCaption: "Each row is a part of the file, in file order. Hover a row to light its bytes in the map, and click it to zoom the map to them.",
  ledgerRowTitle: "Click to zoom the map to this part",
  ledgerUnlisted: (n: number): string => `${counted(n, "more part")} after these were not listed.`,
  ledgerGapLabel: "Bytes no field describes",
  ledgerPaddingLabel: "Padding",
  /** Before a structure's name, for its plain fields taken together. */
  fieldsOf: "Fields of ",
  /** Between a group and the part it is in: `local file in records`. */
  inPart: "in",
  largestLead: "Largest: ",
  ofTheFile: "of the file",
  coreLedgerCaption:
    "Each row is a part of the file, or one kind of element in a list, in the order its first bytes come. Hover a row to mark its first bytes on the map, and click it to zoom the map there.",
  ledgerSoFar: (at: string): string => `Counted so far, up to ${at}. The rest is still being read.`,
  /** Under the byte counts of a file no template reads. */
  classLedgerCaption: (readTo: string, done: boolean): string =>
    done ? "Every byte of the file, by what kind of byte it is." : `Counted so far, up to ${readTo}. The rest is still being read.`,

  // ----- 8. the parts -----
  /** Between the two addresses of a part's range, both of them links. */
  rangeTo: " to ",
  colIndex: "#",
  colName: "Name",
  colAt: "At",
  colSize: "Size",
  colReads: "Reads as",
  colBytes: "First bytes",
  colField: "Field",
  colValue: "Value",
  moreRows: (n: number, word: string): string => `${counted(n, word)} more`,
  /** Neighbouring fields of one name, as one row. */
  runOf: (name: string, n: number): string => `${name} × ${n}`,
  showInListing: "Show in the listing",
  allZero: (n: number): string => `All ${bytesText(n)} are zero.`,
  classesOf: (text: number, zero: number, other: number, total: number): string =>
    `${percentText(text, total)} text, ${percentText(zero, total)} zero, and ${percentText(other, total)} other bytes, over the first ${bytesText(total)}.`,
  gapBody: "No field of the template describes these bytes.",

  // ----- 9. decoded streams -----
  streamsHeading: (n: number, packed: number, unpacked: number | null): string =>
    unpacked === null
      ? `${sentenceCase(counted(n, "compressed stream"))}: ${bytesText(packed)}`
      : `${sentenceCase(counted(n, "compressed stream"))}: ${bytesText(packed)} unpack to ${bytesText(unpacked)}`,
  streamHeading: (name: string, packed: number, codec: string, unpacked: number | null): string =>
    unpacked === null ? `${name}: ${bytesText(packed)} of ${codec}` : `${name}: ${bytesText(packed)} of ${codec} unpack to ${bytesText(unpacked)}`,
  streamName: "Stream",
  streamPacked: "In the file",
  streamUnpacked: "Unpacked",
  /** Over the ratio column, whose cells say `3.8×`: unpacked over packed. */
  streamRatio: "Ratio",
  streamRowTitle: "Click to draw this stream below",
  streamNotUnpacked: "not unpacked here",
  ratio: (r: number): string => `${r.toFixed(r < 10 ? 1 : 0)}×`,
  stageCompressed: "In the file",
  stageUnpacked: "Unpacked",
  stagesCaption: (ratio: string): string => `Each bar is drawn to scale. The unpacked bytes are ${ratio} the size of the compressed ones.`,
  ribbonTop: (bits: number): string => `Codes in the stream, ${counted(bits, "bit")}`,
  ribbonBottom: (bytes: number): string => `Bytes they produce, ${bytesText(bytes)}`,
  /** Codes `from` (counted from 0) up to `to` of a stream. */
  ribbonCaptionLead: (from: number, to: number): string =>
    from === 0
      ? `The first ${counted(to, "code")} of the stream, joined to the bytes each one writes.`
      : `Codes ${(from + 1).toLocaleString()} to ${to.toLocaleString()} of the stream, joined to the bytes each one writes.`,
  ribbonCaption:
    "A literal is one code for one byte. A copy is one code that repeats earlier bytes, so its band widens toward the bytes. Hover a band for its bits and bytes.",
  streamNotOpened: (limit: string): string => `Not unpacked here: the report unpacks streams up to ${limit} on its own. Open it as a tab to read it.`,
  openUnpacked: "Open unpacked",
  moreStreams: (n: number): string => `${counted(n, "more stream")} not shown.`,
  /** For a file whose only streams are joined ones. */
  joinsHeading: (n: number): string => sentenceCase(counted(n, "joined stream")),
  /** After a joined stream's name. */
  joinLine: (size: string): string => `: ${size}, joined into one stream from pieces in several places in the file.`,
  moreJoins: (n: number): string => `${counted(n, "more joined stream")} not shown.`,
  stepLiteral: (byte: string): string => `Literal ${byte}`,
  stepMatch: (len: number, dist: number): string => `Copy ${bytesText(len)} from ${dist.toLocaleString()} back`,
  stepOther: (kind: string): string => kind,
  stepBits: (from: number, to: number): string => `Bits ${from.toLocaleString()} to ${(to - 1).toLocaleString()} of the stream`,
  stepBytes: (from: number, to: number): string =>
    to - from === 1 ? `Byte ${from.toLocaleString()} of the output` : `Bytes ${from.toLocaleString()} to ${(to - 1).toLocaleString()} of the output`,

  // ----- 7. what each directory points to -----
  directoriesHeading: (n: number): string =>
    n === 1 ? "1 list points to other parts of the file" : `${n.toLocaleString()} lists point to other parts of the file`,
  /** After the list's name. */
  directorySummary: (placing: number, elements: number, targets: number): string =>
    placing < elements
      ? `: ${placing.toLocaleString()} of its ${counted(elements, "entry")} point to ${counted(targets, "place")} in the file`
      : `: ${counted(elements, "entry")} point to ${counted(targets, "place")} in the file`,
  dirTop: (name: string): string => `${name}, in stored order`,
  dirBottom: (size: string): string => `The file, ${size}, in file order`,
  dirInOrder: "The places come in the same order as the entries.",
  dirOutOfOrder: "The places do not come in the order of the entries.",
  dirCaption: "Each entry along the top is joined to the place it points to along the bottom. Hover a band for its numbers, and click one to put the cursor on the place.",
  dirFirstEntries: (shown: number, total: number): string => `The first ${shown.toLocaleString()} of ${total.toLocaleString()} entries are drawn.`,
  dirEntryAt: (at: string, size: string): string => `Entry at ${at}, ${size}`,
  dirPointsTo: (name: string, at: string, size: string): string => `Points to ${name} at ${at}, ${size}`,
  dirVia: (via: string): string =>
    via === "address" ? "By an offset in the entry" : via === "offsets" ? "By a list of offsets beside the list" : via === "descriptors" ? "By a descriptor" : via,
  moreDirectories: (n: number): string => `${counted(n, "more list")} not shown.`,
  dirUnexamined: (n: number): string => `${counted(n, "more place")} past the limit ${n === 1 ? "was" : "were"} not looked at.`,

  // ----- 10. the format profile -----
  profileHeading: (used: number, declared: number): string =>
    `This file uses ${used.toLocaleString()} of the ${counted(declared, "kind")} of value its template declares`,
  profileHeadingFile: (used: number): string => `This file holds ${counted(used, "kind")} of value`,
  profileKind: "Kind",
  profileFields: "Fields in this file",
  profileBytes: "Bytes",
  profileDeclared: "Declared in the template",
  notInFile: "none",
  unpackedTo: (streams: number, size: string): string => `${counted(streams, "stream")} opened, unpacked to ${size}`,
  profileCaption:
    "A field counts once in each group it belongs to: a named value is also the number under it. The last column counts the template's declarations, which is what the format allows; a row with none in this file is a kind the file does not use.",
  profileCaptionFile: "A field counts once in each group it belongs to: a named value is also the number under it.",
  choicesHeading: (n: number): string => `${sentenceCase(counted(n, "field"))} that could take more forms than this file uses`,
  choiceField: "Field",
  choiceAllows: "Forms the template allows",
  choiceUses: "Forms this file uses",
  choiceFields: "Times in this file",

  // ----- 11. terms -----
  termsHeading: (n: number): string => `What the template says about ${counted(n, "field")}`,

  // ----- 12. every byte -----
  everyHeading: (n: number): string => `All ${n.toLocaleString()} bytes, part by part`,
  everyCaption: "Every part and every field one level inside it, in file order, with its share of the file.",
  everyCut: (n: number): string => `${counted(n, "more row")} not shown. The listing has every field.`,
  coreEveryCaption: "Every byte of the file, part by part in the order its first bytes come, and under each part what its bytes are where that is more than content.",
  /** What a share of a part's bytes is, by the role the core gives it. */
  role: (role: string, aligns: readonly number[]): string => {
    switch (role) {
      case "content":
        return "Content";
      case "machinery":
        return "Lengths, counts, IDs, and offsets";
      case "padding":
        return aligns.length === 0 ? "Padding" : `Padding to a multiple of ${listText(aligns.map(String))} bytes`;
      case "framing":
        return "Brackets and separators";
      case "gap":
        return "Bytes no field describes";
      default:
        return role;
    }
  },
  /** How much of a gap or of padding is zero: `12 bytes zero, 3 other`. */
  zeroSplit: (zeroBits: number, otherBits: number, unscannedBits: number): string => {
    const parts: string[] = [];
    if (zeroBits > 0) parts.push(`${bitsText(zeroBits)} zero`);
    if (otherBits > 0) parts.push(`${bitsText(otherBits)} not zero`);
    if (unscannedBits > 0) parts.push(`${bitsText(unscannedBits)} not read`);
    return listText(parts);
  },

  // ----- byte references and the map's hover -----
  tipRange: (from: string, to: string, size: string): string => `${from} to ${to}, ${size}`,
  tipAt: (at: string, size: string): string => `${at}, ${size}`,
  tipLoading: "The bytes are still being read.",
  tipClick: "Click to put the cursor here. Double-click to go to the hex view.",

};

/** A long reading cut to `max` characters, at a space where there is one. */
export function clip(text: string, max: number): string {
  if (text.length <= max) return text;
  const cut = text.slice(0, max);
  const space = cut.lastIndexOf(" ");
  return `${space > max * 0.6 ? cut.slice(0, space) : cut}…`;
}

/** A string with its first letter capitalised. */
export function sentenceCase(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** Seconds as a reader says them: `0.30 seconds`, `4 minutes 12 seconds`. */
export function secondsText(s: number): string {
  if (s < 1) return `${s.toFixed(s < 0.1 ? 3 : 2)} seconds`;
  if (s < 60) return `${s.toFixed(1)} seconds`;
  const m = Math.floor(s / 60);
  const rest = Math.round(s - m * 60);
  return `${m.toLocaleString()} ${m === 1 ? "minute" : "minutes"} ${rest} ${rest === 1 ? "second" : "seconds"}`;
}

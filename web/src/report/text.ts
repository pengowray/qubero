// Every string the report view shows, in one place.
//
// Headings are generated and each states a fact with its number, never a fixed
// name for the section: "Where the 5,770 bytes go", not "Overview". The core
// hands over facts and this file writes the words, as `time.rs` and
// `problem.rs` have it for the rest of the app.

import { formatBytes, percentText } from "../format.ts";

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
  identifiedByRule: (ruleFile: string): string => `Identified by a file(1) rule in ${ruleFile}`,
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
  factRecords: "Records",
  factPicture: "Picture",
  factFindings: "Findings",
  factDescribed: "Described by the template",
  pictureSize: (w: number, h: number): string => `${w.toLocaleString()} × ${h.toLocaleString()} pixels`,
  /** The parts, named. A dot between them rather than a comma, since the
   *  names the core gives a variant have commas of their own: `app0, jfif`. */
  partsSummary: (total: number, shown: readonly string[], more: number): string =>
    `${total.toLocaleString()}: ${shown.join(" · ")}${more > 0 ? ` · and ${more.toLocaleString()} more` : ""}`,
  groupCount: (n: number, label: string): string => (n === 1 ? label : `${n.toLocaleString()} × ${label}`),
  noFindings: "None found",
  described: (covered: number, total: number, done: boolean): string =>
    `${bytesText(covered)} of ${bytesText(total)} (${percentText(covered, total)})${done ? "" : " so far"}`,

  // ----- 4. findings -----
  findingsHeading: (parts: readonly string[]): string => sentenceCase(listText(parts)),
  findingInvalid: (n: number): string => counted(n, "invalid value"),
  findingUndefined: (n: number): string => counted(n, "undefined value"),
  findingRefused: (n: number): string => counted(n, "stream that did not unpack"),
  findingGap: (n: number): string => counted(n, "unmapped range"),
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
  findingsSoFar: "Counted over the fields read so far.",

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
  ledgerCaptionLead: (largest: string, share: string): string => `${sentenceCase(largest)} takes ${share} of the file.`,
  ledgerCaption: "Each row is a part of the file, in file order. Hover a row to light its bytes in the map.",
  ledgerUnlisted: (n: number): string => `${counted(n, "more part")} after these were not listed.`,
  /** Under the byte counts of a file no template reads. */
  classLedgerCaption: (readTo: string, done: boolean): string =>
    done ? "Every byte of the file, by what kind of byte it is." : `Counted so far, up to ${readTo}. The rest is still being read.`,

  // ----- 8. the parts -----
  partHeading: (label: string, bytes: string, share: string): string => `${label}: ${bytes}, ${share}`,
  groupHeading: (label: string, count: string, bytes: string, share: string): string => `${label}: ${count}, ${bytes}, ${share}`,
  /** Between the two addresses of a part's range, both of them links. */
  rangeTo: " to ",
  colIndex: "#",
  colAt: "At",
  colSize: "Size",
  colReads: "Reads as",
  colBytes: "First bytes",
  colField: "Field",
  colValue: "Value",
  colType: "Type",
  moreRows: (n: number, word: string): string => `${counted(n, word)} more`,
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
  stepLiteral: (byte: string): string => `Literal ${byte}`,
  stepMatch: (len: number, dist: number): string => `Copy ${bytesText(len)} from ${dist.toLocaleString()} back`,
  stepOther: (kind: string): string => kind,
  stepBits: (from: number, to: number): string => `Bits ${from.toLocaleString()} to ${(to - 1).toLocaleString()} of the stream`,
  stepBytes: (from: number, to: number): string =>
    to - from === 1 ? `Byte ${from.toLocaleString()} of the output` : `Bytes ${from.toLocaleString()} to ${(to - 1).toLocaleString()} of the output`,

  // ----- 11. terms -----
  termsHeading: (n: number): string => `${sentenceCase(counted(n, "field"))} described by the template`,

  // ----- 12. every byte -----
  everyHeading: (n: number): string => `All ${n.toLocaleString()} bytes, part by part`,
  everyCaption: "Every part and every field one level inside it, in file order, with its share of the file.",
  everyCut: (n: number): string => `${counted(n, "more row")} not shown. The listing has every field.`,

  // ----- byte references and the map's hover -----
  tipRange: (from: string, to: string, size: string): string => `${from} to ${to}, ${size}`,
  tipAt: (at: string, size: string): string => `${at}, ${size}`,
  tipLoading: "The bytes are still being read.",
  tipClick: "Click to put the cursor here. Double-click to go to the hex view.",

  // ----- the whole view -----
  noTemplateLead: "No template reads this file, so the report shows its bytes only.",
  reading: "Reading the file…",
};

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

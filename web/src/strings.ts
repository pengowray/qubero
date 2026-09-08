// Text that more than one view shows. Two views naming the same thing two ways
// is the reader's problem, not a detail of whichever file happens to draw it.

import { formatBytes, formatOffset } from "./format.js";
// Type only, and erased: `doc.ts` imports this file at run time, and the
// clause tables below are keyed by the words the core sends in `Shape`.
import type { Shape } from "./doc.js";

/** What a stretch of bytes no field covers is called. `Unmapped` makes it
 * clear that the bytes still exist; only the selected template has no
 * definition for them. */
export const GAP_LABEL = "unmapped";

/** Exact on-disk extent of a field. Counts stay in bytes wherever possible:
 * a reader comparing adjacent rows usually wants the stored length, not a
 * rounded human-size approximation. */
export function bitSizeText(bits: number): string {
  if (bits % 8 === 0) {
    const bytes = bits / 8;
    return bytes === 1 ? "1 byte" : `${bytes.toLocaleString()} bytes`;
  }
  return bits === 1 ? "1 bit" : `${bits.toLocaleString()} bits`;
}

/** A compressed run that was opened, and the fields read inside it.
 *
 *  `DECODED_INSIDE` follows an address counted from the front of the unpacked
 *  stream rather than from the front of the file, and the refusals are the
 *  reasons a run was left as the bytes it is, keyed by what the core
 *  reports. */
export const DECODED_INSIDE = "in the unpacked stream";

/** Why a compressed run was left as bytes. Keyed by `TemplateNode.refused`. */
export const DECODED_REFUSED: Readonly<Record<string, string>> = {
  "too-large": "too large to unpack (over 64 MiB)",
  failed: "unpacking failed",
  unaligned: "not on a byte boundary",
};

/** The same, for a run whose reason is one this build does not know. */
export const DECODED_REFUSED_OTHER = "not unpacked";

/** Why a decoded field's address has no byte strip and no place on the file
 *  map. Shown on the address itself, where a reader wonders what `+0x1c`
 *  means. */
export const DECODED_NO_HEX = "Offset in the unpacked stream, not a file address";

/**
 * A compressed stream opened as a document of its own: the tab it becomes, the
 * button that opens it, and the line saying where a byte of it came from.
 *
 * `origin` is the same sentence in the status bar and in the inspector, because
 * it is the same fact: this byte was produced by that step, reading those bits
 * of the file. The step names read as the format's own words, not the
 * decoder's: `match, 5 bytes back 12` says a run of five bytes copied from
 * twelve back, and a reader who has never written an inflater can still tell
 * that nothing new was stored there.
 */
export const UNPACKED = {
  /** Heading over the one row saying which decoder step produced this field's
   *  bytes. The groups above it say which fields decided the field's shape;
   *  this says the bytes are there at all. */
  originHead: "Unpacked from",
  /** Names the tab: what was unpacked, and what out of. */
  tabTitle: (field: string, file: string): string => `${field} unpacked from ${file}`,
  /** On the listing heading and in the inspector, for a stream that opened. */
  open: "Open unpacked",
  openTitle: (field: string): string => `Open ${field} as a document of its own`,
  /** Typing, pasting or any other edit inside an unpacked stream. A byte of it
   *  is worked out from every compressed byte before it, so there is nowhere to
   *  put a change to one. */
  readOnly: "Unpacked data cannot be edited yet",
  /** Where the byte under the cursor came from. `bits` is one range of the
   *  compressed run, `step` says what the decoder did there. */
  origin: (bits: string, file: string, step: string): string => `from ${UNPACKED.originRow(bits, file, step)}`,
  /**
   * The same fact without the leading `from`, for the panel, where the heading
   * over the row already supplies it.
   *
   * The file name stays: several unpacked streams can be open at once, and the
   * panel is read without a glance at the tab strip, so a bit address with no
   * file in front of it names nothing. The step stays because it is the half
   * that says whether anything new was stored there.
   */
  originRow: (bits: string, file: string, step: string): string => `bits ${bits} of ${file}: ${step}`,
  /** One end of that range: `0x1a3.5` is bit 5 of byte 0x1a3. */
  bit: (bit: number): string => `0x${Math.floor(bit / 8).toString(16)}.${bit % 8}`,
  /** Both ends together. */
  bits: (from: number, to: number): string => `${UNPACKED.bit(from)} to ${UNPACKED.bit(to)}`,
  /**
   * What the decoder did, in the format's own words rather than the decoder's.
   *
   * `match, 5 bytes back 12` says a run of five bytes copied from twelve back;
   * a reader who has never written an inflater can still see that nothing new
   * was stored there. A header or table step names its field, because "header"
   * alone does not say which of a dozen fields the cursor is standing on.
   */
  step: (kind: string, len?: number, dist?: number, field?: string): string => {
    if (kind === "match" && len !== undefined && dist !== undefined) {
      return `match, ${len === 1 ? "1 byte" : `${len.toLocaleString()} bytes`} back ${dist.toLocaleString()}`;
    }
    if ((kind === "header" || kind === "table") && field !== undefined) {
      const named = UNPACKED.fieldName[field];
      if (named !== undefined) return named;
    }
    return UNPACKED.stepName[kind] ?? kind;
  },
  /** The steps that are only themselves, with nothing to measure. */
  stepName: {
    literal: "literal",
    stored: "stored",
    pixel: "pixel",
    block: "block header",
    header: "block header",
    table: "Huffman table",
    "end-of-block": "end of block",
    opaque: "unpacked",
  } as Readonly<Record<string, string>>,
  /**
   * The named fields a deflate header and its code tables are made of, keyed by
   * what the core calls each one. The format's own names are kept, because a
   * reader with RFC 1951 open should be able to find the same word there;
   * what is added is the few words saying what it decides.
   */
  fieldName: {
    bfinal: "last-block flag",
    btype: "block type",
    hlit: "literal code count",
    hdist: "distance code count",
    hclen: "code-length count",
    len: "stored length",
    nlen: "stored length check",
    padding: "padding to the next byte",
    wrapper: "wrapper bytes",
    token: "sequence token",
    length_extra: "extra length bits",
    offset: "match offset",
    frame_header: "frame header",
    block_header: "block header",
    filter: "row filter",
    footer: "footer",
    pxu_flags: "element type and compression",
    pxu_width: "elements per row",
    pxu_height: "rows",
    pxu_bits: "index bits per token",
    code_len: "code-length code",
    lit_len: "literal code length",
    dist_len: "distance code length",
    repeat: "repeated code lengths",
  } as Readonly<Record<string, string>>,
  /** The marks the two tabs keep on each other, named where a reader hovers. */
  linkedIn: "The bits this byte was unpacked from",
  linkedOut: "The bytes these bits unpacked to",
  /** Clicking one of those marks. */
  followIn: (file: string): string => `Show these bits in ${file}`,
  followOut: (tab: string): string => `Show these bytes in ${tab}`,
};

/**
 * One line saying where a byte of an unpacked stream came from, as the status
 * bar and the inspector both say it:
 * `from bits 0x1a3.5 to 0x1a4.2 of hello.txt.zst: match, 5 bytes back 12`.
 *
 * Takes the step's parts rather than the step, so that the wording lives here
 * and nothing here has to know the shape the core sends it in.
 */
export function unpackedOrigin(
  file: string,
  inStart: number,
  inEnd: number,
  kind: string,
  len?: number,
  dist?: number,
  field?: string,
): string {
  return UNPACKED.origin(UNPACKED.bits(inStart, inEnd), file, UNPACKED.step(kind, len, dist, field));
}

/** The same for the panel, where the heading over the row says `from`. */
export function unpackedOriginRow(
  file: string,
  inStart: number,
  inEnd: number,
  kind: string,
  len?: number,
  dist?: number,
  field?: string,
): string {
  return UNPACKED.originRow(UNPACKED.bits(inStart, inEnd), file, UNPACKED.step(kind, len, dist, field));
}

/** Shown where fields would be when nothing has said what the file's are. */
export const NO_TEMPLATE = "No template selected";

/** The same, with the way out, where there is room for a second sentence. */
export const NO_TEMPLATE_HINT = `${NO_TEMPLATE}. Pick one from the Template menu to see the file's fields.`;

/** For a file whose first bytes matched no built-in template. Saying "none
 *  selected" there would suggest an answer exists and the user missed it. */
export const NO_TEMPLATE_MATCH = "No template matched this file. Pick one from the Template menu if you know the format.";

/**
 * The treemap: what a box stands for, and what the five ways of dividing the
 * file are called.
 *
 * The dropdown reads as one question with five answers, which is why the
 * options are singular keys rather than plurals: "group by structure", "group
 * by byte value". The label is the question and the option finishes it.
 */
export const TREEMAP = {
  /** The rail's heading over the treemap. The word anybody who has used
   *  WizTree or SpaceSniffer already has, and the partner of "Minimap" over
   *  the byte-class map above it: two words a reader can look up, whose known
   *  meanings are the two questions the rail's two pictures answer, where a
   *  thing is and how much of the file it is. Neither heading has room for a
   *  subtitle saying so; this one shares its row with the picker and the
   *  corner button. */
  title: "Treemap",
  /** The heading's tooltip, mirroring the minimap's. A picture whose partner
   *  says what it is for and which says nothing itself leaves the reader to
   *  work out the difference. */
  what: "How much of the file each part is. Every box's area is its share, and boxes inside a box are the parts of it. Click a box to go to its bytes. Double-click to open it.",
  /** The picker's label, on its tooltip and for a screen reader; what shows
   *  in the picker is the mode itself. The spreadsheet and file-manager phrase
   *  for choosing what one thing is divided by. */
  groupBy: "Group by",
  /** The five ways of dividing the file. "Byte class" is the five classes the
   *  minimap's legend names, at the minimap's own cell size; "Byte value" is
   *  the 256 values. A category and a number, and the value map's four groups
   *  (00, 01-1F, 20-7F, 80-FF) are not called classes anywhere, so the two
   *  words do not collide. */
  modes: { structure: "Structure", classes: "Byte class", kinds: "Field type", bytes: "Byte value", bits: "Bit value" },
  /** Rectangles too small to point at, drawn as one. The leading ellipsis is
   *  the mark this app uses for content cut short rather than absent. */
  pooled: (n: number): string => `… ${n.toLocaleString()} more`,
  /** The second line of the pooled box's tooltip. The first line already
   *  carries its size and share like any other box's, so those are written
   *  here only when a caller has something to put in them; both callers pass
   *  nothing, and a bare " · ()" is what they used to get. */
  pooledTitle: (n: number, noun: string, size: string, share: string): string =>
    `${countText(n, noun)}, too small to draw${size === "" ? "" : ` · ${size} (${share})`}`,
  /** Bytes that are not claimed by anything *yet*. Drawn as an empty box, so
   *  the map's total area stays the whole file: leave them out and every
   *  share on a half-walked map is quietly inflated. "Read" is the word of
   *  the progress line over this map ("Reading the fields…"); the byte-class
   *  map's own pending box says "not scanned yet" to match its line, and
   *  "unmapped" beside either is the final answer, not a pending one. */
  unwalked: "not read yet",
  /** The walk behind the field-type map, in the same shape as the byte scan's
   *  line, which is the line directly above it. */
  reading: (percent: number): string => `Reading the fields… ${percent}%`,
  failed: (message: string): string => `Couldn't read the fields: ${message}`,
  /** The root of the trail, where the root is not a template node with a name
   *  of its own. */
  root: "File",
  /** Both mouse verbs and the way back, in the order a reader meets them. The
   *  same two verbs as the minimap above, and "box" because that is what a
   *  reader calls the shape. The way back is held until there is somewhere to
   *  go back to: in a 256px rail a third sentence costs a third of the map,
   *  and Backspace means nothing before a reader has opened anything. "One
   *  level" is what it does: one crumb off the trail, not the whole trail. */
  hint: "Click a box to go to its bytes. Double-click to open it.",
  hintBack: "Backspace goes back one level.",
  /** On the tooltip of a box with more inside it than the picture is showing,
   *  since nothing about a drawn box says whether opening it would add
   *  anything. */
  openHint: "Double-click to open",
  /** Out of the box the reader opened, one level. A button of its own before
   *  the trail: the trail says where they are, and a row of names does not
   *  look like a way out until someone has worked out that it is one. */
  back: "Back to the box outside this one",
  backIcon: "\u2191",
  /** The map over the whole window and back in the rail. A glance and a read
   *  are different jobs and the same map does both, so the control that
   *  switches between them is a corner button rather than a mode. Escape
   *  also puts it back, and this tooltip is the one place that says so. */
  grow: "Fill the window",
  shrink: "Back to the sidebar (Esc)",
  growIcon: "\u2922",
  shrinkIcon: "\u2921",
  /** What a zoomed map is a picture of. A treemap fills its box whatever it
   *  is a picture of, so without this line nothing says whether the boxes on
   *  screen are the file or half a per cent of it. */
  zoomedTo: (name: string, size: string, share: string | null, at: string | null): string => {
    const parts = [size];
    if (share !== null) parts.push(`${share} of file`);
    if (at !== null) parts.push(at);
    return `${name} \u00b7 ${parts.join(" \u00b7 ")}`;
  },
  /** Byte values, in four contiguous groups. Zero is on its own because it is
   *  the one value every reader wants isolated, which the plain 00-7F split
   *  would have buried in among the text. */
  byteGroups: [
    { from: 0x00, to: 0x00, label: "00", title: "0x00 · zeros" },
    { from: 0x01, to: 0x1f, label: "01-1F", title: "0x01 to 0x1F · control characters" },
    { from: 0x20, to: 0x7f, label: "20-7F", title: "0x20 to 0x7F · printable ASCII" },
    { from: 0x80, to: 0xff, label: "80-FF", title: "0x80 to 0xFF · high bit set" },
  ],
  /** A run of an array too long to draw one box per element. Indexes rather
   *  than offsets: the box beside it is the next run of the same array, and
   *  what tells them apart is which elements they hold. The detail is the
   *  count alone: a run is openable, so the tooltip's last line already says
   *  "Double-click to open", and this line was saying it a second time. */
  runName: (from: number, to: number): string => `[${from.toLocaleString()}\u2013${to.toLocaleString()}]`,
  runDetail: (n: number, noun: string): string => countText(n, noun),
  /** The five things a stretch of bytes can be when nothing describes it. The
   *  same words the minimap's legend uses, because they are the same five
   *  colours and the same scan. */
  classLabel: ["Zeros", "One repeated byte", "Text", "Data", "High entropy"] as readonly string[],
  /** Under a class box: how many separate places in the file the class sits
   *  in. "Runs" because the pooled box beside it counts runs too, and
   *  "separate" so that "12 runs" is not read as twelve passes of the scan. */
  classDetail: (runs: number): string => (runs === 1 ? "in one run" : `in ${runs.toLocaleString()} separate runs`),
  /** The tail of a file the scan has not reached. Drawn empty, so the map's
   *  area stays the whole file while it fills in. "Scanned" to match the
   *  "Scanning the file…" line it sits under, and to be a different word
   *  from the field walk's "not read yet". */
  unscanned: "not scanned yet",
  byteNoun: "byte value",
  /** One byte value's tooltip: which of the four groups it is in, since a
   *  zoomed map may not be showing the group. The count is not repeated here
   *  because the tooltip's first line already says "1,234 bytes". */
  byteDetail: (_count: number, group: string): string => group,
  /** Two rectangles are one number, so the number is written out under them.
   *  What it means is in the title, since a bare 41% says nothing on its own. */
  bits: { set: "1", clear: "0" },
  bitsLine: (setShare: string, clearShare: string): string => `1 bits ${setShare} · 0 bits ${clearShare}`,
  bitsTitle: (which: "1" | "0", count: string, share: string): string =>
    `${which} bits · ${count} · ${share} (random or compressed data is near 50%; zeros pull it down)`,
  /** How much of the picture a box is, and what the picture is of. Every mode
   *  draws the whole file, with what it has not read yet as a box of its own,
   *  so an unopened share is a share of the file however far the reading has
   *  got. Once a reader opens a box, that box is the picture, and the share
   *  has to say so: "12% of file" from inside a part is a wrong number in
   *  confident words. */
  ofFile: (share: string): string => `${share} of file`,
  ofPart: (share: string, part: string): string => `${share} of ${part}`,
} as const;

/**
 * What the treemap calls one field kind. `TemplateNode.kind` is the
 * evaluator's vocabulary and half of it is not self-explaining: `unread`,
 * `unset` and `magic` say nothing to someone reading a picture.
 *
 * `bytes` and `unread` share one entry because `fieldClass` already treats
 * them as one colour, and "unread bytes" beside the treemap's own "not read
 * yet" box would be two different meanings of the same word touching.
 */
export const KIND_LABEL: Readonly<Record<string, string>> = {
  uint: "unsigned ints",
  int: "signed ints",
  float: "floats",
  str: "strings",
  // Its own row rather than one more kind of text. A disassembled line reads
  // like a string and is not one, and a picture that told a reader six hundred
  // kilobytes of a program were strings was wrong about the file.
  insn: "machine code",
  bytes: "raw bytes",
  unread: "raw bytes",
  magic: "magic numbers",
  enum: "enums",
  flags: "flags",
  unset: "unset values",
  scale: "scale factors",
};

/** How many children a row stands for, named by what they are: `97,280 blocks`
 *  for a run of quantised weights, `2,560 values` for a run of numbers, and
 *  `items` for a list whose format has no word of its own for them. */
export function countText(n: number, noun: string): string {
  return `${n.toLocaleString()} ${n === 1 ? noun : plural(noun)}`;
}

/**
 * What one child of a row stands for. The format names them when it has a word
 * for them: blocks, tensors, entries. Otherwise a list holds items and a
 * structure has fields, which is what the type name says: a list reads as
 * `X[]`, or as `offsets → X` when its children sit where an earlier array
 * of offsets says.
 */
export function childWord(n: { readonly unit?: string; readonly type: string }): string {
  return n.unit ?? (n.type.endsWith("[]") || n.type.startsWith("offsets ") ? "item" : "field");
}

/** The same word as a heading over the children themselves: `Fields` over a
 *  structure, `Tensors` over a list the format names. One fixed word would be
 *  a third name for what the count beside it already calls tensors. */
export function childrenHead(n: { readonly unit?: string; readonly type: string }): string {
  const word = plural(childWord(n));
  return word.charAt(0).toUpperCase() + word.slice(1);
}

/** The panel at the cursor, where a structure now shows what is in it rather
 *  than only how much. */
export const INSIDE = {
  /** The rest of a structure too long to list at once. The ellipsis is the
   *  mark this app already uses for content elided rather than absent, on the
   *  chips and on the trail's `… 4 internal levels`. Not a twisty: a twisty
   *  promises the click folds something shut again, and this one only reads
   *  more rows. The count stays, since three rows left and three thousand are
   *  not the same button. */
  more: (rest: number): string => `… ${rest.toLocaleString()} more`,
  /** What the glyph does not say, for a screen reader and for hovering. */
  moreTitle: (page: number, rest: number, noun: string): string => `Show ${page} more ${plural(noun)} (${rest.toLocaleString()} not shown)`,
  /** The last page, where the click finishes the list rather than paging it. */
  moreRest: (rest: number, noun: string): string => `Show the remaining ${countText(rest, noun)}`,
} as const;

/** The row width that is one record of what is on screen, which reads a
 *  stream of records as the table it is: the same field of every record in the
 *  same column. `every` is false where the records are only mostly that
 *  length, which the reader wants to know before trusting the columns. */
export function strideOption(bytes: number, every: boolean, unit: string | null): string {
  const word = unit ?? "record";
  return `${bytes} per row (${every ? `one ${word}` : `most ${plural(word)}`})`;
}

/** More than one of them. Nouns here are the words formats use for what they
 *  hold, so this covers the endings those run to and no more. */
function plural(noun: string): string {
  if (/[^aeiou]y$/.test(noun)) return `${noun.slice(0, -1)}ies`;
  if (/(s|x|z|ch|sh)$/.test(noun)) return `${noun}es`;
  return `${noun}s`;
}

/**
 * The report listing's own words. The mockup that settled them,
 * `c2-listing.html`, lives outside this repository in
 * `../qubero2-extras/mockups/`; it has the reviewed wording for the SQLite
 * case, and these are the general forms of the same rows.
 */
export const REPORT = {
  /** A part of the file made of a run of a structure's plain fields, which
   *  has no field of its own to name it. Where it sits is the whole of what
   *  can be said about it: a run that opens the file is a header, and the
   *  same words at the end of one would be a lie. A git pack ends with its
   *  checksums at the root, which is exactly that case. In the middle,
   *  nothing more specific than the fields themselves can be claimed, and the
   *  rows one line below already carry their names. */
  unnamedPart: (where: "start" | "end" | "middle"): string =>
    where === "start" ? "Header" : where === "end" ? "Trailer" : "Fields",
  /** Bytes inside a part that none of its fields covers: the empty middle of
   *  a b-tree page. The same word as `GAP_LABEL` above, because it is the
   *  same claim, that the template describes no field here, and that is all
   *  either row knows. "Unused" would be a claim about the format, and a
   *  template that misses a field it should have read would then be telling
   *  the reader those bytes are spare. Where the row sits says the rest: a
   *  gap inside a structure is one line of that structure's own listing. */
  gap: "unmapped",
  /** The verdict beside a gap row: every byte of it was read and each one is
   *  zero. "verified" is doing the work: it says the bytes were looked at,
   *  where a bare "zeros" could pass as a guess about padding. Only a whole
   *  read earns it. A gap past the check's size cap never gets this string,
   *  however many of its bytes were zero so far. */
  gapZeros: "verified zeros",
  /** The same check found at least one byte that is not zero. Nothing is
   *  said: bytes not being all zero is the ordinary case, and a row saying
   *  so on every gap is one more thing between the reader and the rows that
   *  do say something. The zeros verdict and the two no-verdict rows are the
   *  ones worth a word. */
  gapNonzero: "",
  /** No verdict, and none coming: the gap is past the size cap on the check.
   *  Opens the same way as `gapUnread` so the two no-verdict rows read as one
   *  pattern at a scan, with the tail carrying the difference: this one does
   *  not resolve. */
  gapTooLarge: "not checked, too large",
  /** No verdict yet: the gap's bytes have not been read from the file. Not
   *  the situation `reading` below describes; nothing is being fetched. This
   *  is the verdict column saying the read has not happened, and "yet" says
   *  it can. */
  gapUnread: "not checked yet",
  /** Where a part is too small a slice of the file for a percentage to say
   *  anything: under one per cent, "0%" would read as absent. The number the
   *  reader wants at that size is the byte count beside it. */
  tinyShare: "<1%",
  /** What reads this field, said on the field's own row: `\u2192 cells` after
   *  a cell count. A field exists because something uses it, and that is the
   *  answer to what a length prefix is doing in the middle of a header.
   *
   *  The arrow is already this listing's mark for the relationship: a list
   *  placed by an earlier array of offsets has the type `offsets \u2192 X`.
   *  Same relationship, same glyph, on purpose. */
  reads: (name: string): string => `\u2192 ${name}`,
  /** The same link, spelled out for a tooltip and for a reader who cannot see
   *  the arrow. The inspector's panel says DEPENDS ON, so this says depends
   *  rather than coining a second word for one relationship. */
  readsLabel: (name: string): string => `${name} depends on this field`,
  /** The length of a value the template works out rather than reads. Not
   *  blank: with the address blank too, two empty cells read as something
   *  broken rather than as a value that is nowhere in the file. */
  notStored: "not stored",
  /** The two ends of a list that is only partly drawn. `rest` is already
   *  counted and named: "249,800 items". A click draws the next page, not all
   *  of it, so both labels promise "more" and state the remainder instead of
   *  promising the whole of that end.
   *
   *  Mirrored tails rather than one row's "left", which means "remaining after
   *  this point" and is wrong above the window. The rows are often both on
   *  screen, one at each end of two hundred drawn ones, and the pair has to
   *  read as two edges of one window rather than two unrelated controls. */
  more: (rest: string, side: "earlier" | "later"): string => `Show more · ${rest} ${side === "earlier" ? "above" : "below"}`,
  /** A stretch whose bytes have not arrived yet. Same situation as the older
   *  listing's, and the same words. */
  reading: "Loading bytes needed to map these fields…",
  /** The control on a heading or a row that shows the bytes behind it, and
   *  the one that puts them away again. Both are the mockup's own. */
  showBytes: "bytes",
  hideBytes: "hide bytes ✕",
  /** The tail of a field the strip did not draw, at the end of the bytes it
   *  did: a strip is for seeing where fields are rather than for reading a
   *  kilobyte of one. The count is what is missing, not the field's size,
   *  which the row above and the chip below both already give.
   *
   *  It replaced a label. The cut mark used to go under the column in place
   *  of the field's name, so a run of free space and a forty-byte SQL string
   *  were both called "unused…"; the name stays put now and the tail carries
   *  the cut. No ellipsis with it: the plus already says the run goes on.
   *  "bytes" stays, or a bare "+3,958" after a row of hex pairs reads as one
   *  more value. */
  bytesCut: (rest: number): string => `+${bitSizeText(rest * 8)}`,
  /** The control that opens a long list in a pane of its own, on the list’s
   *  heading and on both ends of a drawn window. Not a bare "Show all": next
   *  to "Show more · 249,800 values below" that reads as drawing a quarter of
   *  a million rows where they stand, which either stops the click or makes
   *  the pane a surprise. Verb first and capitalized, like the row it sits
   *  beside. */
  paneOpen: "Show all in pane",
  /** What the pane calls the list it is showing: its name, and how big it is,
   *  which is the thing the reader opened it to face. The count drops its noun
   *  when the list’s own name already is that noun, since "tensors · 100,000
   *  tensors" says it twice. */
  paneTitle: (name: string, count: number, unit: string): string =>
    `${name} · ${plural(unit) === name ? count.toLocaleString() : countText(count, unit)}`,
  /** Closing the pane. The byte strip spells itself out as "hide bytes ✕"
   *  because an inline strip has no frame to say what the glyph would close;
   *  a pane has a title bar saying what it is, so the glyph is enough and the
   *  word goes to the label. */
  paneClose: "✕",
  paneCloseLabel: "Close",
  /** One row of the pane whose bytes are still being fetched. `gapUnread`
   *  above is the other state, where nothing is coming; this is `reading` cut
   *  down to one cell of one row. Unquoted, where a real string value in the
   *  list would be quoted, so a token list that happens to hold the text
   *  "loading…" is still telling the truth. */
  paneWaiting: "loading…",
  /** The same count, spoken, now that it is a control. The total rather than
   *  the remainder: the visible `+3,812 bytes` already gives what is missing,
   *  and what the click delivers is the field. "as a hex dump" is doing work:
   *  a bare "Show all" beside a byte count reads as a promise to draw 3,824
   *  hex pairs into a column a dozen wide. */
  bytesCutOpen: (size: number): string => `Show all ${bitSizeText(size)} as a hex dump`,
  /** The same control with the dump open. It names the thing the click takes
   *  away, and cannot be read as `hideBytes`, which closes the whole strip.
   *  The visible text does not change either way: "3,812 bytes are not drawn
   *  in this column" stays true while the dump is open, and a minus form
   *  would claim the bytes had gone somewhere. */
  bytesCutClose: "Hide the hex dump",
  /** What the dump calls the field it is showing. The same shape as the list
   *  pane's title, since it is the same question: which one, and how far does
   *  it go. No address, because the first line of the dump begins with one. */
  dumpHead: (name: string, size: number): string => `${name} \u00b7 ${bitSizeText(size)}`,
  /** The tail of a bits chip: what the field is, how long it turned out to be,
   *  and the rule that decided where it stopped. A varint's bytes do not read
   *  as bytes, and the chip has just drawn the split; this says what the split
   *  is. The type name and the size come from the field, so only the rule is
   *  copy, and only the rules the mockup settled have any.
   *
   *  A rule with no wording yet drops the colon and says the first two things,
   *  which are both true and neither invented. `bitsRule` below is the list of
   *  what is still to draft. */
  bitsNote: (type: string, sizeBits: number, rule: string): string => {
    const rest = REPORT.bitsRule(rule);
    const head = `${type}, ${bitSizeText(sizeBits)}`;
    return rest === "" ? head : `${head}: ${rest}`;
  },
  /** How a variable-length number says where it ends, keyed by the rule the
   *  core named. `high_bit` is the mockup's own wording, reviewed; the other
   *  three follow its register. An unknown rule says nothing rather than
   *  something invented. */
  bitsRule: (rule: string): string =>
    rule === "high_bit"     ? "high bit 0 ends it" :
    rule === "sqlite_ninth" ? "no high bit on the 9th byte; all 8 bits are value" :
    rule === "ebml_size"    ? "leading zeros count the bytes; the marker bit is not part of the value" :
    rule === "ebml_id"      ? "leading zeros count the bytes; the marker bit is part of the value" :
    "",
  /** The last column of a table of records: where in the file the row it just
   *  showed is actually written. The mockup's own heading, and the one thing
   *  in the table that is about the file rather than about the data. */
  storedAt: "stored at",
  /** A cell of a record table whose value is a page number pointing at another
   *  page of the same file: a SQLite schema row's `rootpage`, and a b-tree
   *  interior page's `left_child_page`. Clicking it goes there.
   *
   *  The number comes first because it is the stored value and the column is
   *  a numeric one; the trailing arrow is the same "goes to" mark the mockup
   *  writes cross-references with, and `reads` above uses for the other
   *  direction. Lowercase, because it is a value and not a heading. */
  pageLink: (n: number): string => `page ${n} →`,
  /** The same link spelled out, for a tooltip and for a reader who cannot see
   *  the arrow. Verb first, like `readsLabel`. */
  pageLinkLabel: (n: number): string => `go to page ${n}`,
  /** The last row of a b-tree interior page. Its child pointer is stored in
   *  the page header rather than in a cell, and it has no key because it has
   *  no upper bound: every key above the last one in the table is on it.
   *
   *  Says what the row covers rather than that something is absent, so it
   *  cannot be taken for an unread or missing value; those have styling and
   *  words of their own. A table b-tree keys on the rowid and says so; an
   *  index b-tree's keys are the indexed columns, which have no one word
   *  between them. */
  rightMostRowids: "all higher rowids",
  rightMostKeys: "all higher keys",
  /** The section an image file opens with: the picture, before the parts of
   *  the file that encode it. Named for what it is, the way "Header" names
   *  the run of fields below it. */
  imageCard: "Image",
  /** The picture's size in pixels, beside it, where a part of the file would
   *  say its size in bytes. A multiplication sign, since "1920 x 1200" reads
   *  as a letter x. */
  imagePixels: (width: number, height: number): string => `${width.toLocaleString()} × ${height.toLocaleString()} pixels`,
  /** The card while the browser is still decoding the picture, and the card
   *  while the bytes it needs have not all arrived. Both are waits; the
   *  second says what it is waiting for, since a streamed file can take a
   *  while to land. */
  imageDecoding: "Decoding the image…",
  imageLoading: "Loading the image's bytes…",
  /** The browser could not turn the bytes into a picture. The file may be
   *  truncated, damaged, or in a variant the browser does not handle; the
   *  card cannot tell which, so it says only what happened. */
  imageFailed: "Couldn't decode this image",
  /** A file past the size the card is willing to hand the browser whole.
   *  The size is the file's, and the limit is named so the line reads as a
   *  rule rather than a fault. */
  imageTooLarge: (size: string, limit: string): string => `Not decoded: this file is ${size}, and the picture is only shown for files up to ${limit}.`,
  /** The picture is scaled to fit the column, or magnified when it is a few
   *  pixels across; a click shows it pixel for pixel, and another puts it
   *  back. "actual" covers both directions where "full" would promise the
   *  magnified one gets bigger. Tooltips on the picture itself. */
  imageShowFull: "Show at actual size",
  imageShowFit: "Scale to fit",
} as const;

// ---- the JPEG cards ----

/** What a JPEG's tables read as once they are laid out the way they are used
 *  rather than the way they are stored. Every one of these sits on a card in
 *  the listing; the field names underneath are the format's own and are not
 *  repeated here. */
export const JPEG = {
  /** A quantisation table's own line: which of the four it is, and whether
   *  its numbers are one byte or two. Both are read straight off the table's
   *  own fields, and both matter to a reader comparing two of them. */
  quantTable: (id: string, precision: string): string => `Quantisation table ${id}, ${precision}`,
  /** The sentence under the grid. The numbers mean nothing without it: a
   *  reader who does not know where the low frequencies are cannot tell a
   *  gentle table from a brutal one, and the whole point of drawing the
   *  square is that the pattern is visible. */
  quantNote: "Each coefficient is divided by its number here, so bigger numbers throw more away. The average brightness is the top left cell and the finest detail is the bottom right.",
  /** The range in the grid, said in words beside it, since a tint alone
   *  cannot be read off as a number. */
  quantRange: (low: string, high: string): string => `${low} to ${high}`,
  /** A Huffman table's own line. A JPEG has up to four of each kind and the
   *  class is what tells two tables with the same id apart, so both are on
   *  the line and the class comes first. */
  huffmanTable: (kind: string, id: string): string => `Huffman table, ${kind} ${id}`,
  /** What the bar row is: sixteen counts, one per code length. The ends are
   *  named rather than drawn as an axis, since sixteen bars need no scale. */
  huffmanCounts: "Codes by length, 1 to 16 bits",
  /** One bar, as a tooltip: how many codes are that long. */
  huffmanBar: (count: string, bits: number): string => `${count} of ${bits === 1 ? "1 bit" : `${bits} bits`}`,
  /** The total, which the file never writes down: it is the sixteen counts
   *  added up, and it is also how many symbols follow them. */
  huffmanTotal: (symbols: string): string => `${symbols} in all`,
  /** The way into the codes themselves, which are not stored anywhere and
   *  are worked out from the counts. Closed to begin with: the shape of the
   *  table is the answer to most questions, and two hundred rows of bits is
   *  the answer to one. */
  huffmanShow: "Show the codes",
  huffmanHide: "Hide the codes",
  /** Column headings for those codes. */
  huffmanCodeColumn: "code",
  huffmanBitsColumn: "bits",
  huffmanSymbolColumn: "symbol",
  /** The counts add up to a different number than there are symbols, so the
   *  codes cannot be rebuilt. Says what is wrong rather than showing a table
   *  that would be wrong. */
  huffmanMismatch: "The counts and the symbols disagree, so the codes cannot be worked out.",
  /** The picture the frame header describes, as one line: how big, how many
   *  bits a sample, and what the sampling factors add up to. */
  frameSize: (width: string, height: string): string => `${width} × ${height} pixels`,
  framePrecision: (bits: string): string => `${bits}-bit samples`,
  /** The chroma subsampling, named the way it is spoken. Only where the
   *  notation applies; a frame it does not fit says nothing and leaves the
   *  per-channel factors in the table to speak for themselves. */
  frameSubsampling: (ratio: string): string => `${ratio} subsampling`,
  /** A frame with one channel, which has no colour to subsample. */
  frameGreyscale: "greyscale",
  /** Column headings for the channels of a frame. "sampling" carries its own
   *  h × v because the two numbers are not interchangeable. */
  frameColumns: ["channel", "sampling h × v", "quantisation table"] as const,
  /** The scan's summary line, which is all of it that shows until the reader
   *  asks for more. The entropy-coded bits are almost the whole file and are
   *  not decoded, so the honest facts are how many channels are in the scan
   *  and how much of the file the bits take. */
  scanComponents: (channels: string): string => `${channels} in this scan`,
  scanEntropy: (size: string): string => `${size} of entropy-coded data, not decoded`,
  /** Only when there are any. A file with no restart markers says nothing
   *  rather than "0 restart markers", which reads as a fault. */
  scanRestarts: (count: string): string => `${count} in the data`,
  /** The count is not in yet because the bytes are not. */
  scanRestartsUnread: "restart markers not counted yet",
  scanShow: "Show the scan header",
  scanHide: "Hide the scan header",
  /** Which coefficients this scan carries. A baseline file writes 0 to 63
   *  once; a progressive one writes a different band in every scan, which is
   *  the only way to tell its scans apart. */
  scanBand: (from: string, to: string): string => `coefficients ${from} to ${to}`,
  /** Column headings for the channels of a scan: which of the frame's
   *  channels this is, and the two tables it is decoded with. */
  scanColumns: ["channel", "DC table", "AC table"] as const,
  /** What a card counts, for the lines that count them. Nouns rather than
   *  whole sentences, since `countText` puts the number in front and makes
   *  the plural. */
  codeNoun: "code",
  channelNoun: "channel",
  restartNoun: "restart marker",
  /** A card whose fields have not been read from the file yet. */
  waiting: "Loading the table's bytes…",
} as const;

/** The table of values beside a folded run in the hex view. A cell says its
 *  value and nothing else; everything about which element it is lives in the
 *  tooltip, since a table of a hundred cells has no room to repeat itself. */
export const VALUES = {
  cell: (run: string, index: number, type: string, text: string): string => `${run}[${index}] · ${type} · ${text}`,
  /** A step of a traced block: named rather than numbered, and worth its
   *  width in bits, which is the whole point of a coded symbol. */
  symbol: (index: number, text: string, bits: number): string => `symbol ${index} · ${text} · ${bits} bits`,
  /** A value the row above began: the same tint over the bits it ends in,
   *  with its text where it started. */
  continued: (run: string, index: number): string => `${run}[${index}] · continued from the row above`,
  continuedLabel: "continued from the row above",
  /** The other half of a value the row edge cuts: this is the earlier piece,
   *  and the value is on the row below because that is where more of it is. */
  continues: (run: string, index: number): string => `${run}[${index}] · continues on the row below`,
  continuesLabel: "continues on the row below",
  rest: (n: number): string => `+${n}`,
  restTip: (n: number, unit: string | null): string => `${countText(n, unit ?? "value")} more on this row`,
  /** A record that reads exactly as the one above it. The ditto is the table
   *  convention for it, and one muted glyph leaves the records that do say
   *  something else as the only text on the screen. An empty cell was not
   *  free to take: the table already draws one for a piece of a value whose
   *  text is on another row. */
  ditto: "\u2033",
  dittoLabel: (unit: string | null): string => `same as the ${unit ?? "value"} above`,
} as const;

/** What `b[n]` means in a shift-and-mask expression. Worth saying, because the
 *  same panel writes `0x131+4b` for an address four bits into a byte, and one
 *  `b` there is bits and the other is bytes. */
export const BYTE_NOTE = "b[n] is the byte at address n";

// ---- searching ----

export const SEARCH_LABELS = {
  find: "Find",
  replace: "Replace with",
  kind: "Search type",
  kinds: { text: "Text", hex: "Hex", regex: "Regex" },
  fold: "Ignore case",
  /** Folding is ASCII only, and a checkbox that quietly does nothing to an
   *  umlaut has to say so somewhere. */
  foldNote: "A to Z only",
  next: "Next",
  previous: "Previous",
  /** "all" carries the thing to know before clicking on a huge file: unlike
   *  Next, this reads all of it. */
  count: "Count all",
  stop: "Stop",
  replaceOne: "Replace",
  replaceAll: "Replace all",
  close: "Close",
  /** The replace row is folded away, because the bar is opened to find. */
  showReplace: "Show replace",
  hideReplace: "Hide replace",
} as const;

/** What the find and replace boxes show when empty. Two stacked boxes with no
 *  visible labels are told apart by these, and for hex they also teach the
 *  format before the first mistake. */
export const SEARCH_PLACEHOLDER = {
  text: { find: "Find", replace: "Replace with" },
  regex: { find: "Find", replace: "Replace with" },
  hex: { find: "89 50 4e 47", replace: "00 ff" },
} as const;

/** The plain text view: the file read as the text it is. Everything here is a
 *  mark in the margin rather than a sentence, because the text is what the
 *  reader came for and the marks are the exceptions. */
export const TEXTVIEW = {
  /** The main view's button, beside Hex and Listing. */
  viewButton: "Text",
  regionLabel: "File as text",
  /** A line too long to draw across the screen; the rest of it is there, the
   *  view is not drawing it. */
  lineClipped: "…",
  /** The core stopped the line at its own limit: what follows is the same line
   *  carrying on. */
  lineCut: "line continues",
  /** Bytes on this line do not fit the encoding it is being read in. Named
   *  rather than "not this encoding", so the mark says what it means without
   *  the reader looking up at the chooser. Takes the encoding the file was
   *  settled as, never the chooser's own label. */
  lineLossy: (encoding: string): string => `not ${encoding}`,
  /** Beside the encoding, when nothing in the file said which it was. */
  guessed: "guessed",
  /** Typing a character the encoding has no room for. The encoding is named
   *  because it may have been a guess, and this is where a wrong guess is
   *  found out, so the way to act on it is in the sentence. "to type it" keeps
   *  that conditional: a file that really is ASCII and a mistyped key need no
   *  encoding changed.
   *
   *  The character is quoted, since a refused dash or middle dot reads as
   *  stray punctuation bare. One that has no glyph to show is written as its
   *  code point instead: empty quotes say nothing, and a combining mark would
   *  attach itself to the quote. Every encoding offered here holds ASCII, so
   *  the quote character can never be the refused one. */
  refused: (char: string, encoding: string): string => {
    const shown = /[\p{C}\p{Z}\p{M}]/u.test(char)
      ? `U+${(char.codePointAt(0) ?? 0).toString(16).padStart(4, "0").toUpperCase()}`
      : `"${char}"`;
    return `${shown} isn't in ${encoding}. Pick another encoding to type it.`;
  },
  /** The encoding chooser's first entry. "Auto-detect" rather than "from the
   *  file", which would claim the file said, and only a byte-order mark does. */
  encodingAuto: "Auto-detect",
  encodingLabel: "Encoding",
  /** Groups in the encoding chooser. Which family a code page belongs to is
   *  what a reader already knows about a file, so it is what the list is cut
   *  by. ASCII sits under Unicode because UTF-8 is a superset of it. */
  encodingUnicode: "Unicode",
  encodingWindows: "Windows and ISO",
  encodingDos: "DOS",
  /** The clipboard would not take it. */
  copyFailed: "Couldn't copy to the clipboard.",
  /** What the file was read as, beside the chooser. */
  readAs: (encoding: string, guessed: boolean): string => (guessed ? `${encoding}, guessed` : encoding),
  /** Which line ending the file uses, beside the encoding. Counted over the
   *  whole of the file indexed so far rather than over the screen, so it does
   *  not change as the reader scrolls.
   *
   *  One kind is named on its own, because that is the whole answer. A file
   *  with more than one is the interesting case and says so, largest share
   *  first: a stray carriage return in a file of line feeds is what somebody
   *  opening a file in a hex editor is looking for. The shares are whole
   *  percentages and none of them is nought: a file said to be mixed and then
   *  shown as one ending at a hundred per cent contradicts itself, so a kind
   *  that is in the file at all is at least one per cent of it, and the
   *  largest gives up whatever that took. */
  lineEndings: (counts: { lf: number; cr: number; crlf: number }): string => {
    const kinds = [
      ["LF", counts.lf],
      ["CRLF", counts.crlf],
      ["CR", counts.cr],
    ] as const;
    const seen = kinds.filter(([, n]) => n > 0).sort((a, b) => b[1] - a[1]);
    const total = seen.reduce((n, [, k]) => n + k, 0);
    if (seen.length === 0 || total === 0) return "";
    const one = seen[0];
    if (seen.length === 1 || one === undefined) return one?.[0] ?? "";
    const rest = seen.slice(1).map(([name, n]) => [name, Math.max(1, Math.round((n / total) * 100))] as const);
    const top = Math.max(1, 100 - rest.reduce((n, [, share]) => n + share, 0));
    const parts = [`${one[0]} ${top}%`, ...rest.map(([name, share]) => `${name} ${share}%`)];
    return `Mixed: ${parts.join(", ")}`;
  },
} as const;

/** The offer to open the file a hex dump describes. */
export const DUMP = {
  heading: "This file is a hex dump",
  /** What was found, in one line. */
  summary: (tool: string, bytes: number): string =>
    `${tool === "" ? "A" : tool} dump of ${bytes.toLocaleString()} ${bytes === 1 ? "byte" : "bytes"}`,
  open: "Open those bytes",
  /** What to call the opened bytes when the dump did not name the file it
   *  dumped. Two tabs called the same thing is worse than a made-up name. */
  fallbackName: (file: string): string => {
    const cut = file.replace(/\.(txt|log|prn|asc|out|dump)$/i, "");
    return cut === file ? `${file} (bytes)` : cut;
  },
  /** Where the dump starts, when it is not the front of a file. */
  startsAt: (at: number): string => `from 0x${at.toString(16)}`,
  /** The tab's tooltip: where these bytes came from. */
  origin: (file: string, tool: string): string => `Decoded from the ${tool === "" ? "hex" : tool} dump in ${file}`,
  /** Stretches the dump did not describe. They read as zeros; where that
   *  belongs is the mark's own tooltip, not the row. */
  holes: (n: number): string => `${n.toLocaleString()} ${n === 1 ? "gap" : "gaps"} in the dump`,
  holesTitle: "Not in the dump; reads as zeros",
  /** Bytes the two spellings disagree about. The columns are named, since a
   *  bare "hex and text disagree" would read as this app's own two views. */
  conflicts: (n: number): string =>
    `hex and text columns disagree on ${n.toLocaleString()} ${n === 1 ? "byte" : "bytes"}`,
} as const;

/** The page is an older build than the site now serves, and is about to reload
 *  itself once. Names the page as the thing out of date: the file and the
 *  reader did nothing wrong, and the browser's cache is nobody's business
 *  here. */
export const PAGE_OUT_OF_DATE = "This page is out of date. Reloading\u2026";

/** The one reload is spent and loading still failed, so a stale page is ruled
 *  out. Says only what is known and the two things that can help; no support
 *  address or status page is named because neither exists. Ctrl and not Cmd
 *  for the same reason every other shortcut here says Ctrl: the buttons do. */
export const EDITOR_WONT_LOAD =
  "Couldn't load the editor, even after reloading. Try a hard refresh (Ctrl+Shift+R). If that doesn't help, this browser may not support WebAssembly.";

export const NO_MATCH = "No match.";
export const WRAPPED_ON = "Wrapped to the start of the file.";
export const WRAPPED_BACK = "Wrapped to the end of the file.";
export const COUNTING = (n: number): string => `${n.toLocaleString()} matches so far…`;
export const COUNT_STOPPED = (n: number): string => `Stopped at ${n.toLocaleString()} matches.`;
export const COUNTED = (n: number): string => (n === 1 ? "1 match." : `${n.toLocaleString()} matches.`);
export const REPLACED = (n: number): string => (n === 1 ? "Replaced 1 match." : `Replaced ${n.toLocaleString()} matches.`);
export const BAD_REPLACEMENT = "Replacement is hex too: pairs of digits, like 00 ff";

/**
 * What each group of rows decided, where several roles sit under one property.
 * One noun each: the property above them has already supplied the subject and
 * the verb, so a heading repeated down one narrow panel says the one word that
 * is not already on screen.
 *
 * `Bit width` is the exception. Beside `Length` a bare `Width` reads as a
 * synonym of it, and the unit is the entire difference between the two
 * questions: a run of grid values is as long as the count says, and each value
 * in it is as wide as the packing said.
 */
export const ROLE_GROUP: Readonly<Record<string, string>> = {
  position: "Position",
  length: "Length",
  width: "Bit width",
  count: "Count",
  type: "Type",
  value: "Value",
  name: "Name",
};

/** The same words on an arrow over the hex grid. The core writes the role as
 *  one lower-case word; the arrow says what the panel beside it says. */
export function roleLabel(role: string): string {
  return ROLE_GROUP[role] ?? role;
}

/**
 * What one clause is allowed to name. Every field is optional because the
 * panel supplies what it can read: a run whose parent will not resolve has no
 * name to put in the sentence, and the clause has to hold without it.
 */
export type HowContext = {
  /** The one field the answer names, as the reader would name it (`len`,
   *  `tensors[3].offset`), when there is exactly one. */
  readonly field?: string;
  /** How many fields the expression reads, when it is not exactly one. */
  readonly fields?: number;
  /** What the field sits in, by name. For a stream field, the compressed run. */
  readonly parent?: string;
  /** Its index in that parent, zero-based, as the core labels `tensors[3]`. */
  readonly index?: number;
  /** What the parent calls its children (`childWord`): `field`, `tensor`, `item`. */
  readonly child?: string;
};

/** The clause every row that reads the file shares. One field is named; more
 *  than one are counted, because two dotted paths do not fit the line and the
 *  expansion lists them anyway; none at all is still an expression, and the
 *  expansion shows it. */
function fromFields(c: HowContext): string {
  if (c.field !== undefined) return `from ${c.field}`;
  if (c.fields !== undefined && c.fields > 1) return `from ${c.fields.toLocaleString()} fields`;
  return "from an expression";
}

/**
 * The properties list: for the field at the cursor, where it is, how long it
 * is, and how each of those was settled.
 *
 * Every clause here is the second, muted line under a fact the panel has
 * already printed: `Position 0x40`, and under it `after machine`. So a clause
 * never repeats the number and never says "this field"; the row supplies
 * both. What it adds is the one thing the number cannot say, which is whether
 * the file decided it or the template did, and through which field.
 *
 * The two tables are keyed by the words the core sends in `Shape` and typed
 * against them, so a case the core grows is a compile error here rather than
 * a silent blank in the panel. `unknown` is an empty string on purpose: the
 * core says it when it does not know, and a clause invented to fill the line
 * would print in the same place and the same voice as the ones it does know.
 * The panel prints nothing for it, and draws no triangle.
 *
 * `from {field}` is the one pattern shared across the rows. A reader learns
 * it once on Position and reads it at a glance on Length, Count, Type and
 * Name; five verbs would be five things to learn.
 */
export const PROPERTIES = {
  /**
   * The section heading. What every inspector calls the list of a selected
   * thing's attributes, from DevTools to Blender, so it costs nothing to
   * learn. "Layout" was the runner-up and lost on Type, Name and Read by,
   * which are not layout. "Depends on", which this replaces, named the edges;
   * these rows are about the field.
   */
  title: "Properties",

  /**
   * The label column. One noun each, and the same nouns the arrows over the
   * hex grid use (`ROLE_GROUP`), so a row and an arrow about the same thing
   * say the same word.
   *
   * Two differ. `Formula`: the editor above is already headed `Value`, so the
   * row for a value worked out from other fields cannot be, and what its fact
   * column holds is the expression, so the label says that. `Read by` rather
   * than `Used by`: the other field's expression reads this one's value, and
   * "read" says exactly that without suggesting the field is consumed. Not
   * `Referenced by`: too long for the column, and a pointer references a
   * field too, which is the `Points to` row.
   */
  row: {
    position: "Position",
    length: "Length",
    type: "Type",
    count: "Count",
    formula: "Formula",
    name: "Name",
    pointsTo: "Points to",
    readBy: "Read by",
  },

  /**
   * How the field came to be where it is, keyed by `Shape.placed`.
   *
   * `after machine` says only that: the field starts where the one before it
   * ended. How long that one was is that field's own Length row, and nothing
   * here claims to have read it. `index 3 in tensors` rather than "element 3":
   * the breadcrumb two inches up calls the same element `tensors[3]`, and
   * "index" is the word programmers read zero-based. `linked from` covers both
   * ends of a chain: for the head, the named field is a pointer in a header
   * and not a previous element, so nothing here may say "the element before
   * it". With no field to name, the chain clause says what the field is, an
   * element of a linked list, which is true of the head and of every link
   * after it; "linked from another field" named nothing and read as a clue.
   * `where offsets[3] points` is the plain reading of a pointer table entry,
   * and the reciprocal of the `Points to` row on that entry. The stream case
   * keeps "unpacked", the word every other string uses for it. The element
   * fallback without an index cannot fire, since the panel always passes one;
   * it says "its parent" like the other fallbacks and not "a run", which is
   * this app's word for a repeat and not the reader's.
   */
  placed: {
    root: (): string => "the whole file",
    first: (c: HowContext): string => `first field of ${c.parent ?? "its parent"}`,
    follows: (c: HowContext): string => (c.field === undefined ? "after the previous field" : `after ${c.field}`),
    element: (c: HowContext): string =>
      c.index === undefined ? `an element of ${c.parent ?? "its parent"}` : `index ${c.index.toLocaleString()} in ${c.parent ?? "its parent"}`,
    pointer: (c: HowContext): string => (c.field === undefined ? "where an offset table points" : `where ${c.field} points`),
    chain: (c: HowContext): string => (c.field === undefined ? "an element of a linked list" : `linked from ${c.field}`),
    address: fromFields,
    trace: (): string => "where the decoder read it",
    stream: (c: HowContext): string => (c.parent === undefined ? "start of the unpacked stream" : `start of unpacked ${c.parent}`),
    unknown: (): string => "",
  } satisfies Record<Shape["placed"], (c: HowContext) => string>,

  /**
   * How long it turned out to be, keyed by `Shape.sized`.
   *
   * Each clause says what settled the length, plainly, and none is a hint
   * for the reader to work out. Two used to be: `fixed by the type` named
   * the wrong thing once the core told `type` from `fixed`, and `ends after
   * its last field` described a struct ending, which every struct does, and
   * left it to the reader to infer that the fields inside had set the length.
   *
   * `fixed by the format` says whose decision it was, which is the question
   * the row answers and the one the number cannot settle: a FITS card is 80
   * bytes because the format says so, and `Card` above it does not say 80.
   * "Fixed size" describes the field and skips the question; "always this
   * length" cannot say whether it means this file or every file. "Format"
   * rather than "template": the format is what fixed the 80 and the template
   * only writes it down, and "format" is a word the reader had before they
   * opened this app.
   * `total length of its fields`, not "sum of its fields", which reads as
   * adding the values up. `element count from e_phnum`, not "counted by
   * e_phnum": a field is not an actor, and "element count" says what was
   * taken from it. `no bytes of its own` sits under a fact of `0 bytes`, so
   * the zero reads as a kind of field and not as a measurement that failed;
   * "not stored in the file" would be false for a pointer holder, whose
   * bytes are all at the target.
   */
  sized: {
    /** Nothing. The length row already reads `8 bytes` and the type row above
     *  it already reads `u64 le`, so a clause saying the second explains the
     *  first is a line the reader has to read to find out it says nothing. */
    type: (): string => "",
    fixed: (): string => "fixed by the format",
    expression: fromFields,
    terminated: (): string => "ends at a terminator",
    remaining: (c: HowContext): string => `rest of ${c.parent ?? "its parent"}`,
    children: (c: HowContext): string => `total length of its ${plural(c.child ?? "field")}`,
    /** A list of places rather than a stretch of bytes: the elements are
     *  wherever the offsets said, and the number is measured to the end of the
     *  structure the list was declared in, because that is as far as they can
     *  be. The elements come first because the line wraps at this width, and
     *  "to the end of header" on a line of its own is the misreading this
     *  case exists to prevent: a reader going into the header for a block
     *  that is not there. */
    scattered: (c: HowContext): string => `elements where its offsets point; measured to the end of ${c.parent ?? "its parent"}`,
    /** Named where one field holds the count; counted where an expression
     *  reads more than one, since the expansion lists them. With neither the
     *  count is a literal or a bare expression, and the core does not say
     *  which, so the clause can only say the length follows from the count. */
    count: (c: HowContext): string => {
      if (c.field !== undefined) return `element count from ${c.field}`;
      if (c.fields !== undefined && c.fields > 1) return `element count from ${c.fields.toLocaleString()} fields`;
      return "from its element count";
    },
    /** The varint, the instruction and the JSON scalar alike: each was
     *  measured by decoding it, and none holds a length. Not "self-delimiting",
     *  which is the varint's word and not the instruction's, and not "its own
     *  bytes mark the end", which made the bytes the actor and read as a
     *  riddle. */
    encoded: (): string => "decoded from its own bytes",
    /** "Measured" rather than "set": the decoder found out how many bits it
     *  took, it did not choose. Nothing with "read" in it, because the row two
     *  lines down is labelled `Read by`. */
    trace: (): string => "measured by the decoder",
    nothing: (): string => "no bytes of its own",
    unknown: (): string => "",
  } satisfies Record<Shape["sized"], (c: HowContext) => string>,

  /** A switch picked the type. The value is the case that matched, and it is
   *  the half the reader checks: `from chunk_type` alone would send them to
   *  the field to find out which case this was. Without a value to show, which
   *  is what a table read out of a decoder's block has, the field alone. */
  typeFrom: (field: string, value: string): string => (value === "" ? `from ${field}` : `from ${field} = ${value}`),

  /** Over the list of structures round the field, each with the field's
   *  offset inside it: `section_headers[3]  +0x14`, nearest first. "Offset"
   *  because that is what `+0x14` is; "within" because the rows are the
   *  things it is within. */
  within: "Offset within",
  withinAt: (name: string, at: string): string => `${at} in ${name}`,

  /**
   * The control under the rows that opens the same rows for each structure
   * the field sits inside. Counts structures, not levels: "3 more levels up"
   * is a direction and says nothing about what it opens, while an enclosing
   * structure is a thing the reader can name and click. `n` is how many of
   * them have anything to say; a struct the template placed and sized
   * outright has no rows and is not counted.
   */
  enclosing: (n: number): string => countText(n, "enclosing structure"),

  /**
   * The Read by row: fields elsewhere whose length, count, position, type,
   * bit width or name reads this field's value.
   *
   * Three answers that must not look alike: none found, not searched, and
   * found in part. `none` is a finding, and the one a reader about to edit a
   * length field most wants; hiding the row would make it look like `not
   * searched`. The partial case is the file too big to walk and only the
   * enclosing structure walked: a bare count there is a count of some of the
   * answer, so the clause says where the search stopped. `searched only in
   * header`, not "only header searched": read cold, "only header" parses as
   * a noun first, and the verb-first order is the one the reader would say.
   * The limit is printed with the size that broke it, because "too many"
   * with no number is a verdict and not a fact, and it is called the search
   * limit because a bare "limit" leaves the reader asking a limit on what.
   */
  readBy: {
    found: (n: number): string => (n === 0 ? "none" : countText(n, "field")),
    partial: (parent: string): string => `searched only in ${parent}`,
    notSearched: "not searched",
    tooMany: (parent: string, n: number, limit: number): string =>
      `${parent} has ${n.toLocaleString()} fields, search limit ${limit.toLocaleString()}`,
    /** One row of the expansion, before the field's name, which is a link:
     *  `length of` data. A phrase rather than a column of role words, because
     *  under this heading a bare `Length` reads as this field's length, and
     *  it is the other field's. */
    what: (role: string): string => `${roleLabel(role).toLowerCase()} of`,
  },
} as const;

/** The arrows over the hex grid, and the graph view. Both show what the
 *  properties list shows, so the words for it are here rather than in either
 *  view, and neither can drift from the other. */
export const LINKS = {
  button: "Dependencies",
  title: "Draw arrows from the fields the field at the cursor depends on, and outline the structure it is in",
  /**
   * A dependency the overlay could not draw because the field it leaves from
   * is not on screen.
   *
   * The field is named rather than counted: an arrow that is missing because
   * its far end is a thousand rows away looks exactly like no dependency at
   * all, and the name is what lets the reader go and find it. Past three the
   * line would not fit, so the rest are counted.
   */
  offScreen: (fields: readonly { name: string; at: string; above: boolean }[]): string => {
    const first = fields[0];
    if (first === undefined) return "";
    if (fields.length === 1) {
      return `Depends on ${first.name} at ${first.at}, off screen ${first.above ? "above" : "below"}.`;
    }
    const named = fields.slice(0, 3).map((f) => `${f.name} at ${f.at}`);
    const rest = fields.length - named.length;
    const list = rest > 0 ? `${named.join(", ")}, and ${rest.toLocaleString()} more` : named.join(", ");
    return `Depends on ${fields.length.toLocaleString()} fields off screen: ${list}.`;
  },
};

/** The graph view. */
export const GRAPH = {
  button: "Graph",
  /** Kept on screen rather than shown once. A note about a slow layout that
   *  has already gone by the time the layout is slow is no warning at all. */
  experimental: "Experimental. Laying out a large file may be slow, or may not finish.",
  /**
   * More fields under the cursor than the view will lay out.
   *
   * The verdict first, because it is what a skim has to catch, then the
   * numbers as the evidence for it, then the one thing to do and where to do
   * it. Contents is named because it is the panel that is on screen in every
   * view and reaches inside a part; "put the cursor somewhere" tells a reader
   * looking at a graph nothing about how.
   *
   * Never "the first N": the walk is breadth-first, so the fields shown are
   * the top of the structure and not a prefix of the file.
   *
   * `root` is null when the graph covers the whole file.
   */
  omitted: (shown: number, total: number, root: string | null): string => {
    const where = root === null ? "this file" : root;
    return (
      `Too many fields in ${where} to draw: ${shown.toLocaleString()} of ${total.toLocaleString()} shown. ` +
      `Click a smaller part under Contents to graph just that part.`
    );
  },
  /** Over the sliders. Each label finishes the sentence this starts: pull
   *  together the fields that are of the same type, and so on. */
  forcesHeading: "Pull together",
  force: {
    depends: "Dependencies",
    kind: "Same type",
    near: "Near in the file",
    sibling: "Same parent",
  },
  /**
   * A structure's plain fields, counted instead of drawn.
   *
   * A field that no arrow touches is the same shape as every other field that
   * no arrow touches, so a dozen of them drawn round their parent is a
   * starburst that says one thing twelve times. Counted, the picture is the
   * connections, with the rest noted beside them.
   */
  folded: (n: number): string => countText(n, "plain field"),
  /** The boundary drawn round the fields of one type. The count is the fact
   *  the eye cannot get from the shape once a group holds more than a handful,
   *  and it is what lets two groups be compared at a glance. */
  hull: (kind: string, n: number): string => `${kind} \u00b7 ${countText(n, "field")}`,
};

/**
 * The strings view: what a file that is not a text file has to say in words.
 *
 * The one fact on a row is the text. Everything else says where it is and what
 * is unusual about it, so a page of ordinary strings reads as a column of text
 * with a quiet margin, and the rows worth stopping at are the ones with
 * something in that margin.
 */
export const STRINGSVIEW = {
  /** The main view's button, beside Hex, Listing and Text. */
  viewButton: "Strings",
  regionLabel: "Strings found in the file",

  // ---- the controls ----

  minimumLabel: "Minimum length",
  /** After the number. A hex editor's reader reads a bare 4 as bytes. */
  minimumUnit: "characters",
  minimumTitle:
    "Strings shorter than this are left out. Counted in characters, not bytes: 4 characters of UTF-16 is 8 bytes.",
  lookForLabel: "Look for",
  lookForGroup: "Encodings to look for",
  /** The toggles. ASCII names UTF-8 too, or a reader takes UTF-8 for something
   *  that is not scanned. */
  encodingToggle: {
    ascii: "ASCII and UTF-8",
    utf16le: "UTF-16 LE",
    utf16be: "UTF-16 BE",
  } as Readonly<Record<string, string>>,
  encodingToggleTitle: {
    ascii:
      "One pass finds both: UTF-8 is ASCII with wider characters allowed. A string holding one is tagged UTF-8; one holding none is tagged ASCII. Like the Unix strings command, it finds any run of printable ASCII at least the minimum length, noise included.",
    utf16le:
      "UTF-16, little-endian, searched for at every byte offset. A match is found only when the bytes around it mark it as a string: a null terminator after it, or a length prefix of more than 2 bytes in front. Not found: unterminated text with no length prefix, and text that is only CJK or kana, since those code units are also pairs of ASCII letters. Read those in the Text view with UTF-16 LE chosen.",
    utf16be:
      "UTF-16, big-endian, searched for at every byte offset. A match is found only when the bytes around it mark it as a string: a null terminator after it, or a length prefix of more than 2 bytes in front. Not found: unterminated text with no length prefix, and text that is only CJK or kana, since those code units are also pairs of ASCII letters. Read those in the Text view with UTF-16 BE chosen.",
  } as Readonly<Record<string, string>>,
  /** "Filter" alone, beside the app's Find, would be taken for it. */
  filterPlaceholder: "Filter strings",
  filterLabel: "Filter strings by text",
  filterTitle:
    "Show only the strings whose text contains this. Case is ignored. Filters what has been found so far; the scan is not affected.",

  // ---- one row ----

  offsetTitle: "First byte of the text. A length prefix, where there is one, sits just before it.",
  /** What the encoding column says on hover. The names themselves are the
   *  core's, and are the ones the text view's chooser and the panel use. */
  encodingTitle: {
    ASCII: "Printable ASCII, one byte per character",
    "UTF-8": "UTF-8, with at least one character beyond ASCII",
    "UTF-16 LE": "UTF-16, little-endian: two bytes per code unit",
    "UTF-16 BE": "UTF-16, big-endian: two bytes per code unit",
  } as Readonly<Record<string, string>>,
  /** The zero that ends a string, drawn as the control picture for it rather
   *  than spelled out. The common case has to stay quiet, and what a reader
   *  wants to notice is a row in a column of these that has none. */
  terminatorMark: "␀",
  terminatorTitle: (bytes: number): string =>
    bytes === 1
      ? "Null-terminated: a zero byte follows the text"
      : "Null-terminated: two zero bytes follow the text, one UTF-16 code unit",

  /** The number in front of the string, as the row says it.
   *
   *  A reading the file does not vouch for says "possible" rather than
   *  carrying a mark of its own. The hedge is in the leading words because
   *  that is where the eye lands and because a note too long for the row is
   *  cut from its end, which is where a mark would have been. */
  prefix: (
    kind: string,
    bytes: readonly number[],
    value: number,
    counts: string,
    withTerminator: boolean,
    weak: boolean,
  ): { readonly before: string; readonly hex: string; readonly after: string } => {
    const term = withTerminator ? ", terminator included" : "";
    const what = weak ? "possible length prefix" : "length prefix";
    return {
      before: `${what} ${kind}`,
      // A leading space and no "=", so that dropping the hex on a narrow pane
      // leaves "length prefix u64 LE = 20 bytes", which still reads.
      hex: " " + bytes.map((b) => b.toString(16).padStart(2, "0")).join(" "),
      after: ` = ${countText(value, unitWord(counts))}${term}`,
    };
  },
  /** Why a reading is only possible. The number is no wider than one character
   *  of the string, and nothing else in the file counts a string that way, so
   *  the match may be the run's own edge read a second time. */
  prefixWeakTitle: (wide: boolean): string =>
    wide
      ? "Probably a coincidence. The code unit before a run of text is never a printable character (it would be part of the run), and neither is a short length stored in one code unit."
      : "Probably a coincidence. The byte before a run of text is never a printable character (it would be part of the run), and neither is a short length stored in one byte, so the two match by chance about once in 161 runs.",
  /** The readings that come to the same number, which is nearly always the
   *  same number written at narrower widths. Named rather than counted: which
   *  widths agreed is the fact, and "3 readings" is not. */
  prefixAlso: (kinds: readonly string[]): string => ` (also ${kinds.join(", ")})`,
  /** The same, spelled out, one reading a line, with the arithmetic the row
   *  has no room for. */
  prefixTitle: (readings: readonly PrefixReading[], lenBytes: number, wide: boolean): string => {
    const lines = readings.map((r) => {
      const hex = r.bytes.map((b) => b.toString(16).padStart(2, "0")).join(" ");
      const check =
        r.counts !== "bytes"
          ? ` (${countText(lenBytes, "byte")} of text)`
          : r.with_terminator
            ? ` (${lenBytes} of text + ${r.value - lenBytes} terminator)`
            : "";
      return `${r.kind} at ${formatOffset(r.at * 8)}: ${hex} = ${countText(r.value, unitWord(r.counts))}${check}`;
    });
    // Straight after the readings: a reader hovering a "possible" note is
    // asking why, and that is the answer.
    if (readings[0]?.weak === true) {
      lines.push(STRINGSVIEW.prefixWeakTitle(wide));
    }
    if (readings.length > 1) {
      lines.push("Several readings come to the same number. Nothing in the bytes says which width was meant.");
    }
    lines.push("Click to put the cursor on the number.");
    return lines.join("\n");
  },

  /** A UTF-16 surrogate with no partner. The real term on the row; the name
   *  for the encoding that allows it goes in the tooltip, where there is room
   *  to say what it is. */
  loneSurrogate: "lone surrogate",
  loneSurrogateTitle:
    "A UTF-16 surrogate with no partner, shown as �. Not valid UTF-16, but valid WTF-16, which Windows filenames and V8 heap dumps can hold.",

  /** A run cut at the length limit, and the piece that carries it on. Both
   *  ends are named, because a row that starts in the middle of a word with
   *  nothing to say why is the one thing worse than a cut. */
  continuesAt: (offset: number): string => `continues at ${formatOffset(offset * 8)}`,
  continuesAtTitle: (offset: number, limit: number): string =>
    `Cut at ${countText(limit, "byte")}. The rest of the text is the string at ${formatOffset(offset * 8)}.`,
  continuedFrom: (offset: number): string => `continued from ${formatOffset(offset * 8)}`,
  continuedFromTitle: (offset: number, limit: number): string =>
    `The rest of the string at ${formatOffset(offset * 8)}, which was cut at ${countText(limit, "byte")}.`,

  /** The length, far right, so a prefix reading has something on the row to be
   *  checked against. */
  length: (bytes: number): string => bitSizeText(bytes * 8),
  /** "of text" says the prefix and the terminator are not counted in it. */
  lengthTitle: (bytes: number, units: number, chars: number): string => {
    const parts = [`${countText(bytes, "byte")} of text`];
    if (units !== bytes) parts.push(countText(units, "code unit"));
    if (chars !== units) parts.push(countText(chars, "character"));
    return parts.join(" · ");
  },

  // ---- the status line ----

  /** Two facts joined by a middle dot, count then progress, in that order
   *  every time: a reader watching the list grow reads the same two places. */
  statusFound: (n: number): string => (n === 0 ? "No strings yet" : countText(n, "string")),
  statusFiltered: (shown: number, n: number): string =>
    `${shown.toLocaleString()} of ${countText(n, "string")} match`,
  statusScanning: (scanned: number, total: number): string =>
    `first ${formatBytes(scanned)} of ${formatBytes(total)} scanned…`,
  statusWhole: "whole file scanned",
  /** Scanned to the end and found nothing. Names the minimum, because that is
   *  the control to reach for and it is on screen. */
  statusNone: (min: number): string => `No strings of ${min} or more characters · whole file scanned`,
  /** The list is as long as this view holds, so the rest of the file was not
   *  scanned. Said outright, with the two things that would let it be. */
  statusCapped: (scanned: number, total: number): string =>
    `stopped at ${formatBytes(scanned)} of ${formatBytes(total)}, the most this view holds. To scan further, raise the minimum length or turn off an encoding.`,
  statusNoEncodings: "All three encodings are off. Turn one on above to scan for strings.",
  scanProgressLabel: (scanned: number, total: number): string =>
    `Scanned ${formatBytes(scanned)} of ${formatBytes(total)}`,
};

/** One reading of the bytes in front of a string, as the view hands it over. */
export type PrefixReading = {
  readonly kind: string;
  readonly at: number;
  readonly bytes: readonly number[];
  readonly value: number;
  readonly counts: string;
  readonly with_terminator: boolean;
  readonly weak: boolean;
};

/** The singular of what a length prefix counted, for `countText`. The core
 *  answers in the plural because that is how a count reads; one of them needs
 *  the other form. */
function unitWord(counts: string): string {
  return counts === "code units" ? "code unit" : counts === "characters" ? "character" : "byte";
}

// Text that more than one view shows. Two views naming the same thing two ways
// is the reader's problem, not a detail of whichever file happens to draw it.

import { ADDRESS_MARK, formatBytes, formatOffset } from "./format.ts";
// Type only, and erased: `doc.ts` imports this file at run time, and the
// clause tables below are keyed by the words the core sends in `Shape`.
import type { FieldTime, Shape } from "./doc.ts";

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
  /**
   * The file did not say how the run was packed, so nothing tried to unpack
   * it. Kept apart from `failed`, which says a decoder read these bytes and
   * would not have them: a reader told unpacking failed goes looking for
   * damage, and there is none to find here. What a 7z coder that packed
   * nothing, or one whose properties this cannot read, comes to.
   */
  settings: "the file doesn't say how this was packed",
};

/** The same, for a run whose reason is one this build does not know. */
export const DECODED_REFUSED_OTHER = "not unpacked";

/**
 * What the `+` in front of a decoded field's address is counted from, carried
 * on the mark itself.
 *
 * The `+` is written in front of two different kinds of address here: one
 * counted from the front of an unpacked stream, and one counted from the front
 * of an enclosing structure (`PROPERTIES.withinPlusTitle`). Both reach the
 * inspector, and written down they are the same glyph. What they have in
 * common is that a number is being added to something, so the mark says what,
 * and the two are the same sentence: `Offset within X`.
 *
 * It said `, not a file address` as well, and no longer does. Naming the thing
 * the offset is counted from is the whole job, and a reader told the offset is
 * within the unpacked stream has already been told it is not within the file.
 * A clause ruling out the answer the first half has just replaced is a second
 * sentence to read to learn nothing.
 */
export const DECODED_PLUS_TITLE = "Offset within the unpacked stream";

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
/**
 * The Integrity section: what a checksum is of, and where those bytes are.
 *
 * The section used to say only whether a check passed. Over an archive entry
 * that is three different claims, since the sum could be over the stored
 * bytes, over the file they unpack to, or over the header in front of them,
 * and a reader asking whether their file is intact was answered about one of
 * them without being told which.
 */
export const CHECKED = {
  /**
   * The row naming the sum and what it is taken over. A noun, like `Position`
   * and `Length` further down: the row is a fact about the field, and the
   * field is the checksum. `Checks`, which this replaces, made the field the
   * one doing the checking, and on a line of its own it was a verb with no
   * subject. Git calls its trailing SHA-1 a checksum too, so the word holds
   * there.
   */
  sumLabel: "Checksum",
  /**
   * That row's fact: `ZIP CRC-32 of the unpacked file · 252 B`. `of` between
   * the sum and its subject rather than a dot, so the row says what the sum
   * is over instead of leaving the reader to guess the relation between two
   * phrases. The dot is kept for the size, as on the address row, so the two
   * rows' sizes sit in the same place and can be compared at a glance, which
   * for a compressed entry is the whole point: 252 B was summed, 178 B is
   * what is in the file.
   *
   * `size` is null when the core has no true count to give without decoding a
   * stream it must not decode just to answer this: an xz block whose stream
   * has not been opened yet, or a 7z stream whose own header carries no
   * length. What it has instead is the packed length, which is a different
   * run of bytes wearing the same units, and printing it here would be
   * printing that run's size under a claim about this one. Once the check
   * itself opens the stream the true count comes free, and the row is
   * updated rather than left wrong in the meantime.
   */
  of: (label: string, what: string, size: string | null): string =>
    size === null ? `${label} of ${what}` : `${label} of ${what} · ${size}`,
  /**
   * Label on the address row when the summed bytes are bytes of the file.
   * The checksum is its subject, as it is for `Points to` and `Read by`, and
   * it is true before the check has run: a sum too big to take unasked
   * covers its bytes whether or not the button has been pressed, which is
   * where `Bytes checked` fell down. `Over` was a bare preposition on a line
   * of its own.
   */
  overLabel: "Covers",
  /**
   * Label on the address row when the sum is over something unpacked, so the
   * bytes at that address are not what was summed but what the summed bytes
   * came out of. Names those bytes for what they are, so the row is a true
   * fact on its own and a reader who reads only it cannot take the sum to be
   * over them; the row above says what it is over. `Unpacked from` lost: the
   * Properties list uses those words over a row about the field at the
   * cursor, and here they would have been about the file the field checks.
   */
  fromLabel: "Compressed bytes",
  range: (from: string, to: string, size: string): string => `${from} to ${to} · ${size}`,
  /**
   * Label on the row for the check field's own bytes, for a sum that runs over
   * a record the field sits inside. `Its` is the checksum, the subject of
   * `Covers` above it, so the column reads as one sentence: the checksum
   * covers this run, and its own bytes inside it are summed as spaces. That
   * relation is the one thing the address cannot say, and it is the whole
   * reason the bytes are read as something else: a writer cannot know the
   * number until it has added up the field that is going to hold it.
   *
   * `This field` lost: the panel is already about this field, so the label
   * said nothing the reader did not have. `Except` lost on arithmetic:
   * leaving eight bytes out and counting them as spaces give different
   * totals, and a reader who took the word at face value would get the wrong
   * one.
   */
  ownLabel: "Its own bytes",
  /**
   * That row's fact: `@0x94 to @0x9b · 8 B, summed as spaces (0x20)`. The
   * address first, in the same form as the `Covers` row, so the two sit in one
   * column and a reader can see that this run lies inside that one without
   * being told. The clause after the comma is the deviation, and the row only
   * exists when there is one.
   *
   * `summed as`, not `read as`: `Read by` is a label a few rows up in
   * Properties, in another sense of the word, and `summed` ties this row to
   * the Checksum row it belongs to. "Rather than as written" is left to `as`:
   * summing bytes as something is not summing them as themselves, and the
   * sentence that said so was longer than the row. A comma rather than a dot
   * before the clause, as in `calculated X, stored Y`: the dots separate
   * facts, and this is a clause about the fact in front of it.
   */
  own: (from: string, to: string, size: string, byte: number): string =>
    `${CHECKED.range(from, to, size)}, summed as ${CHECKED.summedAs(byte)}`,
  /**
   * The byte those bytes are read as, named. A space printed as hex with its
   * character after it shows a reader nothing they can see, so the two bytes
   * any format uses get a word: `spaces (0x20)`, `zeros (0x00)`. Plural, since
   * it is eight bytes and not one. The word first and the number after it: the
   * word is what a reader takes in, the number is what they add. Any other
   * byte prints as its hex, which no format writes today.
   */
  summedAs: (byte: number): string =>
    byte === 0x20 ? "spaces (0x20)" : byte === 0x00 ? "zeros (0x00)" : `0x${byte.toString(16).padStart(2, "0")}`,
  /** What is being summed, in words, after `of`. This and the four below name
   *  the thing; the exact bytes are the address row's to say. Here, PNG: the
   *  chunk's type and data, and not the length in front of them. */
  chunk: "this chunk's type and data",
  /** gzip's header CRC-16 and LHA's header checksum. `this` rather than
   *  `the`, since an LHA archive has a header per entry. */
  header: "this header",
  /** A stored archive entry: its data as it sits in the archive. `entry`
   *  rather than `the file`, which in a hex editor means the document being
   *  edited. Both users of this string, ZIP and LHA, are archives of entries. */
  file: "this entry's stored data",
  /** A deflated entry or a gzip stream: the sum is over what unpacks, and
   *  those bytes are nowhere in the file. `the file these bytes unpack to`
   *  lost: `these bytes` pointed at a row the reader had not reached. */
  unpacked: "the unpacked file",
  /** An xz block's own check, over what that one block unpacks to rather
   *  than the whole stream around it. `unpacked` above would say the wrong
   *  thing here: a stream holding several blocks unpacks to all of them
   *  together, and this sum is over one block's share of that, not the
   *  whole. `this block's`, not `the block's`: the check field the row
   *  belongs to sits inside exactly one, so there is no other block to
   *  confuse it with. */
  unpackedMember: "this block's unpacked bytes",
  /** Git index and pack index: the trailing SHA-1 over the whole file before
   *  it, in the words git's own format doc uses. */
  upTo: "everything before this checksum",
  /** A check that did not happen, which is neither a pass nor a failure and
   *  must not read as one: the slot it lands in is the slot that otherwise
   *  says Valid or Mismatch. The state first, then what went wrong. */
  notChecked: (why: string): string => `Not checked · ${why}`,
  missingBytes: "some bytes could not be loaded",
  unknownFailure: "the check could not be run",
  /** The button for a sum too big to take without being asked. The size it
   *  would read is on the row above it. */
  run: (label: string): string => `Check the ${label}`,
} as const;

/**
 * The Date & time row: what a timestamp field's number comes to, printed under
 * the number itself, which stays where it is.
 *
 * The row used to be eight cases written out in the panel, one per format, and
 * the words were whichever each case reached for: `(UTC)`, `(QuickTime epoch,
 * UTC)`, `(MS-DOS local time)`. The core now answers one question for any field
 * a template declares as a time, and the answer is one of three things: an
 * instant with a zone, the value this format writes when it has no time to
 * record, or a number that is not a date. The digits are built in code, to the
 * precision the field has; everything round them is here.
 */
export const TIME = {
  /**
   * The instant: `2023-08-05 11:22:47 UTC`. The zone follows the digits with a
   * space and nothing else, as `date -u` and every log line print one. The
   * parentheses it used to sit in made the zone an aside, and it is the
   * opposite of one: the same digits name a different moment in every zone the
   * reader might be in, and which is meant is the one thing the digits cannot
   * say. Each of the three phrases below is read in the same slot after the
   * same shape of digits, so a reader who has seen `UTC` there once reads the
   * other two as the same kind of fact.
   */
  at: (digits: string, zone: FieldTime["zone"]): string => `${digits} ${TIME.zone[zone]}`,
  zone: {
    /**
     * The format counts from a fixed instant, so the digits name one moment
     * everywhere: gzip, tar, PE, MP4 and a FILETIME. Bare, as every tool prints
     * it. The old QuickTime row said `(QuickTime epoch, UTC)`, and the epoch is
     * gone on purpose: once the count is turned into a date, where the count
     * started is how the number was read rather than what it says, and the type
     * row is where that belongs.
     */
    utc: "UTC",
    /**
     * Wall-clock digits from the machine that wrote the file, in whatever zone
     * that machine was in, which the file does not record: a ZIP's or a
     * cabinet's MS-DOS time, and the dates a classic Mac wrote. The digits are
     * shown as written and are never shifted, so what follows them has to say
     * two things without running long: whose clock, and that the zone is not
     * there to be had.
     *
     * `local time` is the term the ZIP specification and zipinfo use, and the
     * one a reader would look up. On its own it can also be read as the
     * reader's local time, converted for them, which is the one reading this
     * row must never allow. `zone not recorded` closes it: a zone the file does
     * not hold cannot have been converted from. It is also the fact a reader
     * who wants the real instant needs, since the zone is theirs to supply.
     *
     * `MS-DOS local time`, which this replaces, named one format's encoding on
     * a row that now covers two, and said nothing about the zone at all.
     */
    local: "local time, zone not recorded",
    /**
     * The format does not say whether the number is UTC or the writer's clock,
     * so this cannot either. Both possibilities named, and the reason in the
     * words `DECODED_REFUSED` uses for a file that does not say how a run was
     * packed. Not `zone unknown`, which reads as this app failing to work it
     * out; and not `zone not recorded`, which is the phrase above and would
     * make this case look like that one, where the reader at least knows a wall
     * clock was read. No built-in template gives this answer yet.
     */
    unknown: "UTC or local, the format doesn't say",
  } satisfies Record<FieldTime["zone"], string>,
  /**
   * The value stored is the one this format writes when it has no time to
   * record: gzip's `mtime` of 0, which means the compressor had none to put
   * there and not the first second of 1970. Only a format that declares its
   * none-value gets this row, since the same 0 in a tar is a real time, so the
   * row can say it outright. A fact about what the writer put in the file,
   * under a heading that already names what was not recorded, so it does not
   * repeat its subject.
   *
   * `Not specified (stored as 0)`, which this replaces, repeated the number
   * from the value row directly above it, which no clause in this panel does,
   * and left "specified" open by whom.
   */
  unset: "Not recorded",
  /**
   * The number does not name a moment. Two ways to get here, one line for both:
   * the instant falls outside years 1 to 9999, which is what a garbage 64-bit
   * tick count in a corrupt file comes to, or a packed MS-DOS field is not a
   * date at all: month 0, a thirteenth month, the thirtieth of February. Said
   * as a fact about the value and not as a check that failed. `Invalid Unix
   * timestamp` and `Invalid MS-DOS date/time`, which this replaces, led with
   * the encoding's name, read as the app refusing to decode one, and were two
   * strings for one fact. No format name, since the heading already says the
   * field is a date and the type row says how it is stored. "Valid" is left out
   * for the same reason: it is a validator's word, and the reader's takeaway
   * should be that these bytes do not hold a date, not that something rejected
   * them. The raw number stays in the value row above.
   */
  impossible: "Not a date",
} as const;

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
  /** Where the byte under the cursor came from. `bits` is the stretch of the
   *  compressed run the decoder read, worded by `bits` below, and `step` says
   *  what the decoder did there. */
  origin: (bits: string, file: string, step: string): string => `from ${UNPACKED.originRow(bits, file, step)}`,
  /**
   * The same fact without the leading `from`, for the panel, where the heading
   * over the row already supplies it.
   *
   * The file name stays: several unpacked streams can be open at once, and the
   * panel is read without a glance at the tab strip, so a bit address with no
   * file in front of it names nothing. The step stays because it is the half
   * that says whether anything new was stored there.
   *
   * The noun comes in with `bits`, since it is not always "bits": a step that
   * read nothing is worded differently from one that read a range, and this
   * sentence should not have to know which it was handed.
   */
  originRow: (bits: string, file: string, step: string): string => `${bits} of ${file}: ${step}`,
  /** One end of that range: `@0x1a3.5` is bit 5 of byte 0x1a3. Marked like
   *  every other address, since it is one, and it is read beside byte counts
   *  and code numbers that are not. */
  bit: (bit: number): string => `${ADDRESS_MARK}0x${Math.floor(bit / 8).toString(16)}.${bit % 8}`,
  /**
   * The stretch of the compressed run a step read, noun and all: `bits @0x1a3.5
   * to @0x1a4.2` is half-open, from the first bit up to but not including the
   * second.
   *
   * A step that read nothing is `no bits at @0x6.0`. Its two ends are the same
   * bit, and `bits @0x6.0 to @0x6.0` would put the step on a bit it never
   * touched. This is the usual case for LZMA, not a corner of it: the range
   * coder pulls a byte only when its arithmetic runs short, so most symbols
   * pull none and one later symbol pulls the byte that paid for several. Two
   * other steps are empty for other reasons: LZMA properties a container
   * supplied instead of the run holding them (lzip fixes them by convention,
   * 7z writes them in the archive header), and a deflate match that read no
   * extra bits past the code that named it.
   *
   * The position is kept because it is real and it is what the reader asked.
   * The trace tiles the run, so an empty step's one bit is where the decoder
   * stood: the bit the next read would have started at. It is also the only
   * thing there is to look by, since a mark of no width cannot be drawn in the
   * other tab.
   *
   * `===` rather than `<=`: the trace asserts that steps tile, so an end before
   * its start is a bug, and a range that reads backwards shows it where "no
   * bits" would hide it.
   */
  bits: (from: number, to: number): string =>
    from === to ? `no bits at ${UNPACKED.bit(from)}` : `bits ${UNPACKED.bit(from)} to ${UNPACKED.bit(to)}`,
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
    filter_def: "filter definition",
    footer: "footer",
    pxu_flags: "element type and compression",
    pxu_width: "elements per row",
    pxu_height: "rows",
    pxu_bits: "index bits per token",
    lzma_props: "LZMA properties (lc, lp, pb)",
    range_init: "range decoder init",
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
 * `from bits 0x1a3.5 to 0x1a4.2 of hello.txt.zst: match, 5 bytes back 12`,
 * or, for a step that read nothing, `from no bits at 0x6.0 of hello.txt.lz:
 * literal`.
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

/**
 * The Decoded section: what one code of a compressed block stands for.
 *
 * A code and the thing it encodes are two levels, and the panel used to show
 * them as one. The bits at the cursor are a Huffman prefix code, nine of them;
 * `literal 'M'` is what this block's table says those nine bits mean. Showing
 * the second as fields of the first put two inferences in the column a reader
 * checks the file against, with `0 bytes` beside each, and neither is written
 * in the file at all.
 *
 * So the box above holds the bits and this section holds the reading, under a
 * heading that says which it is. Nothing here is stored anywhere: it is worked
 * out again from the block's tables on the click that lands the cursor on the
 * code, which is why the rows say what they say rather than being one number
 * apiece. The widths are the point of the section. A reader told `8 + 0 + 5 +
 * 0` who can see the Length row say 13 bits has checked the arithmetic; one
 * told `match 3 back 4` has taken it on trust.
 */
export const DECODED = {
  /** The heading. The reading of the bits, in the word this app already uses
   *  for what a decoder made of a compressed run: a stream opens as `Decoded`,
   *  and a byte of it is `unpacked from` a step. */
  title: "Decoded",
  /**
   * The rows' labels.
   *
   * One rule holds the whole section together: the value beside a label is
   * exactly the thing the label names and nothing else. `Symbol` gets `77`.
   * `Length code` gets `symbol 259`. Every further fact is its own row or its
   * own muted line underneath. What this replaced glued four of them together
   * with an equals sign, so a row labelled `Symbol` read `77 = literal 'M'
   * (0x4d)`, three of whose four parts are not a symbol.
   *
   * All three kinds read down in the same order, so a reader who has learned
   * one can glance at the others: first what the code did, under a label
   * naming its kind; then `Symbol`; then, last, where the bytes went. Before
   * this a literal led with `Symbol` and a match led with `Copies`.
   *
   * `Length code` and `Distance code` are RFC 1951's own words. `Literal
   * byte` and `End of block` carry the kind that used to sit inside the
   * value, which is label work.
   */
  literalLabel: "Literal byte",
  endLabel: "End of block",
  symbolLabel: "Symbol",
  lengthLabel: "Length code",
  distanceLabel: "Distance code",
  copiesLabel: "Copies",
  /**
   * Where the step's output went, in the unpacked stream's own addresses.
   *
   * `Written at`, not `Unpacks to`, which this replaces. The section's rule is
   * that the value is exactly the thing the label names, and `Unpacks to`
   * names a value: read cold, `Unpacks to @+0x0` says either "this code unpacks
   * to what is at @+0x0", which is wrong and which the `Literal byte` row two
   * up has already answered, or "its output was written at +0x0", which is
   * right. `Written at` can only be read the second way.
   *
   * It also pairs with `endWrites` on the end-of-block row: one says a step
   * wrote nothing, the other says where a step's bytes went, on the same verb.
   * The end mark has no row here at all, since `@+0x1c` and a count of no bytes
   * would put it somewhere it never was.
   */
  writtenLabel: "Written at",
  /**
   * The byte a literal code stood for, hex first and the character after it.
   *
   * Hex first so the number sits in the same place whether or not there is a
   * glyph to follow it: `0x0a` alone, `0x4d 'M'`. A reader checking these bits
   * against the hex view wants the number and a reader reading the stream
   * wants the character, and neither should have to hunt for their half in a
   * column two hundred pixels wide.
   *
   * For a literal the symbol number and the byte are the same number in two
   * bases, since symbols 0 to 255 of the literal/length alphabet are the bytes
   * themselves. That is why they are two rows and not one phrase: `77 =
   * literal 'M' (0x4d)` stated one fact twice and dressed it as an equation.
   */
  literalByte: (byte: number): string => {
    const hex = `0x${byte.toString(16).padStart(2, "0")}`;
    return byte >= 0x20 && byte <= 0x7e ? `${hex} '${String.fromCharCode(byte)}'` : hex;
  },
  /** What the end mark produced, said outright so the missing `Unpacks to`
   *  row reads as an answer rather than as a gap. The house says `no bits`
   *  where a step read none; this is the other side of it. */
  endWrites: "writes no bytes",
  /** Which symbol of the alphabet a code stood for: the bare number, under a
   *  label that already says Symbol. */
  symbolNumber: (symbol: number): string => symbol.toLocaleString(),
  /** The same, on a match's two rows, where the label names the code rather
   *  than the symbol and so the word has to be carried. What the symbol came
   *  to is not repeated here: the Copies row above already says `5 bytes back
   *  1`, and saying each half again under its own code is what made these rows
   *  read as `X = Y`. */
  codeSymbol: (symbol: number): string => `symbol ${symbol.toLocaleString()}`,
  /** What the match copies, in the words the status bar already uses for the
   *  same step: `match, 5 bytes back 12` there, and here the same phrase under
   *  a label that supplies the verb. */
  copies: (len: number, dist: number): string =>
    `${len === 1 ? "1 byte" : `${len.toLocaleString()} bytes`} back ${dist.toLocaleString()}`,
  /**
   * A distance of one, which is not really a copy: it is deflate's way of
   * writing a run of one byte, the byte before repeated. Said outright under
   * the Copies row, because `34 bytes back 1` reads as a mistake to anybody
   * who has not met the idiom, and it is the format working exactly as
   * intended.
   */
  repeated: (len: number): string => `one byte, repeated ${len.toLocaleString()} times`,
  /**
   * The same for a copy that reads bytes it is in the middle of writing: the
   * distance is shorter than the length, so the last part of the copy is bytes
   * this step itself produced. Legal, common, and the other thing that reads
   * as a bug when a reader checks the arithmetic and finds the source run
   * shorter than the length.
   */
  overlap: (dist: number): string => `overlapping copy: the last ${dist.toLocaleString()} bytes, repeated`,
  /**
   * How wide one code was, under the row it belongs to, and what followed it.
   *
   * The four widths across a match have to add up to the Length row two
   * sections down, so each says its own two and the reader can add them. `no
   * extra bits` is written out rather than left off: an absent clause would
   * read as a row that had nothing more to say, and what a reader is checking
   * is that nothing else was read there.
   */
  codeBits: (codeBits: number, extraBits: number): string => {
    const code = `${codeBits}-bit code`;
    return extraBits === 0 ? `${code}, no extra bits` : `${code}, then ${countText(extraBits, "extra bit")}`;
  },
  /**
   * Where a fixed block's code width came from, under the width line that
   * states it.
   *
   * In a dynamic block that width line is a button to the row of the block's
   * own code-length table that set it, so a reader who doubts the number can
   * go and look at the bits that chose it. A fixed block has no such row: RFC
   * 1951 chose the widths and nothing in the file could have chosen otherwise,
   * so the same words are a dead end with nowhere to click. This is what the
   * reader would have found there, said outright instead. It is drawn in
   * italic and takes no hover, so that it reads as a fact rather than as a
   * link that has stopped working.
   *
   * The range and not the section alone, because the range is the half a
   * reader can check: told `symbols 256 to 279`, they can see their symbol 256
   * inside it and that seven bits follows. `fixed by RFC 1951` on its own
   * would be one more number to take on trust, which is the thing this whole
   * section exists not to ask for.
   */
  rfcFixed: (range: string): string => `fixed by RFC 1951 for ${range}`,
  /**
   * Which run of the fixed code a symbol falls in. Straight out of RFC 1951
   * section 3.2.6: literal/length symbols 0 to 143 are eight bits, 144 to 255
   * are nine, 256 to 279 are seven and 280 to 287 are eight, and every
   * distance symbol is five.
   *
   * The distance alphabet is named in the phrase rather than left to the row
   * it sits under. Its symbols run 0 to 31 and the literal/length alphabet's
   * first run is 0 to 143, so a bare `symbols 0 to 31` under a Distance code
   * row could be read as a range of the alphabet the row above it is about.
   * `distance symbol` is the panel's existing phrase for the same thing, as in
   * `code length for distance symbol 3`.
   */
  fixedRange: (symbol: number, distance: boolean): string => {
    if (distance) return "distance symbols 0 to 31";
    if (symbol <= 143) return "symbols 0 to 143";
    if (symbol <= 255) return "symbols 144 to 255";
    if (symbol <= 279) return "symbols 256 to 279";
    return "symbols 280 to 287";
  },
  /**
   * What the symbol and its extra bits came to, on its own line under the
   * width, and only where there were extra bits to add.
   *
   * Two sums meet on these rows and used to share a line: `7-bit code, then 1
   * extra bit: 11 + 1` put bit widths and a decoded length either side of one
   * colon, and read cold the `11 + 1` looks like more bit counts. Split, each
   * line is checkable against exactly one row: the width against Length two
   * sections down, this against Copies at the top. The leading word says
   * which sum it is, and the `=` stays, because unlike the row it came from
   * this really is an equation.
   */
  lengthSum: (base: number, extra: number, total: number): string =>
    `length ${base.toLocaleString()} + ${extra.toLocaleString()} = ${total.toLocaleString()}`,
  distanceSum: (base: number, extra: number, total: number): string =>
    `distance ${base.toLocaleString()} + ${extra.toLocaleString()} = ${total.toLocaleString()}`,
  /**
   * Where the step's output landed, in the unpacked stream: one address for a
   * single byte, and the first and last for a run. The leading `+` on both
   * says they count from the front of the stream rather than of the file, and
   * carries what it counts from on the mark itself, which `dom.ts`'s `address`
   * puts there.
   *
   * No size on the end, which is where this parts company with the Integrity
   * section's `Covers` row. A size there is worth comparing, since the rows it
   * sits among can disagree with it. Here it cannot: the count is the range by
   * construction, the `Copies` row above has already said it outright, and the
   * width lines have already added up to it. On a literal it is worse than
   * redundant, since it is `1 byte` on every literal there has ever been.
   */
  wrote: (from: string, to: string | null): string => (to === null ? from : `${from} to ${to}`),
  /**
   * The line under the box of noughts and ones, naming the bytes those bits
   * were packed into, in the order the decoder read them.
   *
   * What this replaced was an assertion and nothing else: `Bits in reading
   * order, low bit of each byte first` told the reader there were byte
   * boundaries in the box and drew none of them, so checking it meant working
   * the boundaries out by hand from an offset three lines up. The fill in the
   * box now draws them and this line names them. That is the same fact twice
   * on purpose. The fill is a colour, and a colour cannot be read out, cannot
   * be searched for, and is the half of the pair a reader who cannot tell the
   * two tints apart does not get; the addresses are the half they can take to
   * the hex view.
   *
   * `then` between the clauses rather than a comma or an `and`, because the
   * order is the whole of what the line is for: it is the order the binary
   * column disagrees with. A code that fits inside one byte has one clause and
   * no `then`, and reads `9 bits from 0x69`.
   */
  bitsFrom: (parts: readonly { readonly bits: number; readonly at: string }[]): string =>
    parts.map((p) => `${countText(p.bits, "bit")} from ${p.at}`).join(", then "),
  /**
   * The convention itself, on the caption's own line, for the reader who wants
   * to know why these bits are the other way round from the Binary view.
   *
   * Two sentences where there were three. The one dropped said that the bits
   * could be compared against the tables in RFC 1951, which the Decoded
   * section's own sublines now say for each width and say more usefully, by
   * naming the table row or the RFC range the width came from.
   */
  bitsFromTitle:
    "DEFLATE reads each byte low bit first, so this shows the code exactly as RFC 1951 writes it. The Binary view reads every byte high bit first, which is why the same bits look reversed there.",
} as const;

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
 * The B-trees tab: one HDF5 version 1 B-tree drawn as bands, and the same
 * nodes again by file address.
 *
 * Two traps run through every string here. A number with no stated origin:
 * every count is either a field the Listing shows by that name (`node_level`,
 * `entries_used`, `symbol_count`) or a sum the row names, and there is
 * deliberately no "% full", because HDF5's capacity per node has not been
 * checked against the specification and a guessed denominator would be a
 * number from nowhere. And a label whose value is not the thing it names: an
 * index node's width is not its entry count, `0, 0, 0` is not a chunk index,
 * and "level 2" is not two below the root. Each of those has a string below
 * that says the other reading out loud.
 *
 * Two of these were paragraphs of caption until the picture was changed to
 * carry them. The number on a box is explained by the band the picture now
 * draws below the last row of boxes, because every box's number counts what is
 * in the band directly beneath it; and the band is also what says where a
 * chunk tree stops, which used to be the sentence "The chunks themselves are
 * not drawn". A caption that a reader has to hold in mind while looking at a
 * picture is a fault in the picture.
 *
 * HDF5's own names for the two node kinds are `TREE` and `SNOD` ("symbol
 * table node"). They are called index node and link table here: what a TREE
 * holds is pointers into the index, and what an SNOD holds is the group's
 * links, so the names say what is inside. The signature is carried along in
 * brackets on the readout, because it is the first four bytes a reader sees
 * after pressing the box and it is the term the specification is written in.
 */
export const BTREES = {
  /** The rail tab beside Contents and Logical. Standard, searchable
   *  vocabulary, and the tab is shown only for HDF5, so it cannot be taken
   *  for any other tree in the file. */
  tab: "B-trees",
  /** The tab's tooltip, in the minimap's order: the question the picture
   *  answers, then what one mark is, then the verbs. Two pictures share the
   *  panel and nothing about a band of boxes says which question it answers,
   *  so both are named. "The number of ... below it" is the phrase the width
   *  captions use too, so the words here are the words there.
   *
   *  A comma and not "and" between the two things said about a box: "each box
   *  under its parent and its width" reads for one beat as a box under its
   *  width, because "and" offers "its width" to the preposition before "in
   *  proportion to" takes it back. The comma makes them two parallel
   *  fragments, which is what they are. The width rule also has to stay inside
   *  the "Top:" part, because every mark on the bottom picture is one pixel
   *  wide and a loose sentence about box width would be false of it. */
  what: "The shape of one B-tree in this HDF5 file: how far it branches, how deep it goes, and where its nodes sit in the file. Top: the tree, root at the top, each box under its parent, its width proportional to the number of links or chunks below it. Bottom: the same nodes placed by file address. Click a box to go to its bytes. Double-click to open it in the Listing.",
  /** Under the heading, which is the owning object's path. The two jobs a
   *  version 1 tree does are two different pictures (a group tree has a row a
   *  chunk tree does not), so the job is stated rather than left to be read
   *  off the shape. "B-tree" is repeated because the heading is a path and
   *  the tab is a plural. */
  jobGroup: "B-tree of this group's links",
  jobChunk: "B-tree of this dataset's chunks",
  /** Line 1 of the heading when the owning object is past the Contents list's
   *  cap. Not "unnamed object": an HDF5 object can genuinely have no name, and
   *  this one has one that was not fetched. The rail already says "{n} more not
   *  listed" for the cap, so this says the same thing in the same words, and
   *  keeps the tree's address so the heading still says which tree is up. */
  unnamed: (treeAt: string): string => `Object not listed under Logical · tree at ${treeAt}`,
  /** What a box's width is, under the picture. A box shows two marks, its
   *  width and the number printed on it, and they are two different facts: the
   *  number is the node's own entry count and the width is the total at the
   *  bottom of its subtree. Left unsaid, a reader takes the wide box with "3"
   *  on it for a mistake.
   *
   *  A sentence, not "Width: ...". A width is not a count, and a colon puts a
   *  count where the reader was promised a width. And "below it" rather than
   *  the old "reached through it", which was a riddle: what made the spatial
   *  word wrong was that nothing used to be drawn below a link table, whose
   *  links are inside it. The picture now draws that band (`leafLinks`), so
   *  every box in the picture, bottom row included, has the things its width
   *  counts drawn directly below it. */
  widthGroup: "Box width is proportional to the number of links below it.",
  widthChunk: "Box width is proportional to the number of chunks below it.",
  /** The key to the number printed on a box, shown beside a drawn box with `N`
   *  in it. The sentence that used to say this ("Number on a box: its own
   *  entry count") was a fact the reader had to hold in mind while looking at
   *  the picture; the drawn box says where the number appears, so the words
   *  only have to say what it counts. "That node" rather than "this node": the
   *  drawn box is a key, and "this" invites reading it as a node in the tree. */
  entriesChip: "N",
  entriesKey: "entries in that node",
  /** The band under the last row of boxes: the links or chunks the tree
   *  indexes. They are not nodes, so they are drawn as one dashed, undivided
   *  band rather than as boxes, and the label is a count with its unit.
   *
   *  Drawn at all because the picture has to say where each kind of tree ends.
   *  A group tree's bottom row of link tables and a chunk tree's bottom row of
   *  index nodes look alike, and the only thing that told a reader the chunk
   *  tree had no further row was a caption saying "the chunks themselves are
   *  not drawn". It also makes the number printed on a box readable without
   *  being told: every box's number is how many things are in the band
   *  directly below it, which at the root is two or three boxes to count.
   *
   *  "Or more" when the walk hit its node cap, because the count is then a
   *  floor. The warning line under the summary counts nodes, and nothing in it
   *  says the link or chunk total is short as well. It goes after the noun
   *  rather than in front of the number, so that the tooltip's own verb can
   *  never run into it: "point at at least 400 chunks" is a stutter, and
   *  "point at 400 chunks or more" is not. */
  leafLinks: (n: number, capped: boolean): string => `${countText(n, "link")}${capped ? " or more" : ""}`,
  leafChunks: (n: number, capped: boolean): string => `${countText(n, "chunk")}${capped ? " or more" : ""}`,
  /** The band's tooltip, and the one string that stops the band being read two
   *  ways. The band sits in the same place under both kinds of tree and means
   *  something different under each: a group's links are inside the link
   *  tables in the row above, and a dataset's chunks are somewhere else in the
   *  file. So each tooltip carries the readout's own verb, `holds` or `points
   *  at`, and then the location word in plain English, because a reader who
   *  reads those two verbs as synonyms still has to get the fact.
   *
   *  The second sentence denies both places a reader would go looking: the
   *  boxes above and the address picture below, named in `stripCaption`'s own
   *  words. "These" only while the count is a total: with `or more` on it the
   *  number no longer names a set anyone can point at. */
  leafLinksTitle: (nodes: number, links: number, capped: boolean): string =>
    `${rowAbove(nodes, "link table", capped)} ${nodes === 1 ? "holds" : "hold"} ${capped ? "" : "these "}${BTREES.leafLinks(links, capped)} inside ${nodes === 1 ? "it" : "them"}. Links are not nodes of the tree, so they are not drawn as boxes and are not among the nodes placed by file address below.`,
  leafChunksTitle: (nodes: number, chunks: number, capped: boolean): string =>
    `${rowAbove(nodes, "index node", capped)} ${nodes === 1 ? "points" : "point"} at ${capped ? "" : "these "}${BTREES.leafChunks(chunks, capped)}, which are blocks of data elsewhere in the file. Chunks are not nodes of the tree, so they are not drawn as boxes and are not among the nodes placed by file address below.`,
  /** One row of the summary for a row of index nodes. The level is written with
   *  the field's own name, `node_level`, because that is where the number came
   *  from and what the Listing calls it at those bytes; a bare "level 2" is a
   *  number with no origin and two directions. The direction is stated once, in
   *  the title, rather than on every row. */
  rowIndex: (n: number, level: number): string => `${countText(n, "index node")} · node_level ${level.toLocaleString()}`,
  /** On the summary rows. HDF5 counts levels up from the leaves, which is the
   *  opposite of how the picture is stacked, so a reader who assumed "level 2
   *  is two rows down" would be wrong about every row but one. */
  levelTitle: "node_level as the file writes it: 0 on the bottom row of index nodes, highest at the root",
  /** The summary row for the link tables, and for a chunk tree's bottom row of
   *  index nodes. Two counts on one line need the relation between them said,
   *  or "36 link tables · 1,204 links" can be read as 1,204 each; the verb says
   *  it. "Pointing at" on the chunk row is the same verb as the readout's and
   *  as the chunk band's tooltip, so the row a reader is looking at and the
   *  band beneath it are described the same way. This is the line in words for
   *  the band the picture draws, which is redundancy on purpose: it is the one
   *  place the counts can be read without hovering anything. */
  rowLinks: (nodes: number, links: number): string =>
    `${countText(nodes, "link table")} holding ${countText(links, "link")}`,
  rowChunks: (nodes: number, chunks: number): string =>
    `${countText(nodes, "index node")} pointing at ${countText(chunks, "chunk")}`,
  /** Siblings too narrow to press, drawn as one box. Says what happened to
   *  them in `TREEMAP.pooledTitle`'s words. Not `… N more`: the treemap's
   *  ellipsis marks content cut short beside content that was drawn, and here
   *  nothing in the pool is drawn on its own. `noun` is singular; `countText`
   *  makes the plural. */
  pooled: (n: number, noun: string): string => `${countText(n, noun)}, too narrow to draw apart`,
  /** The two node kinds, by what they hold, and the signature each one carries
   *  in the file. See the block comment for why these and not HDF5's own
   *  names, and why the signature is here anyway. */
  kindIndex: "index node",
  kindLinks: "link table",
  signIndex: "TREE",
  signLinks: "SNOD",
  /** First line of a node's readout and tooltip, and the whole tooltip of a
   *  mark on the address strip. The signature is what is written at that
   *  address, so the line a reader takes to the hex view names both what we
   *  call it and what they will find there. */
  selectedAt: (kind: string, sign: string, address: string): string => `${kind} (${sign}) at ${address}`,
  /** The node's own count, `entries_used` or `symbol_count`, with the verb that
   *  fits the kind: an index node points at things and a link table holds
   *  names. One string with a swappable noun read "36 link tables" on a box
   *  that holds no link table; the verb is what makes the count the count of
   *  the right thing. `noun` is singular and is what the next row down holds:
   *  index nodes, link tables, chunks, or links. */
  pointsAt: (n: number, noun: string): string => `points at ${countText(n, noun)}`,
  holds: (n: number, noun: string): string => `holds ${countText(n, noun)}`,
  /** A group-tree node's key range, as the two link names at its ends. Two
   *  labelled facts rather than "X to Y": a link name can contain a space or
   *  the word "to", and the labels survive that. One link is one fact. */
  selectedRange: (first: string, last: string): string =>
    first === last ? `link ${first}` : `first link ${first} · last link ${last}`,
  /** A chunk-tree node's key range. The keys are offsets into the dataset
   *  counted in elements, so "element offset" is on the visible line: a reader
   *  who saw `0, 0, 0` with no label would take it for a chunk index and the
   *  range for three chunks. */
  selectedChunkRange: (first: string, last: string): string =>
    `first chunk at element offset ${first} · last at ${last}`,
  /** The line under a chunk range. HDF5 writes one number per dimension and
   *  then one more, an offset within an element, that is always 0; so a reader
   *  counting the numbers in `950, 950, 0` gets a rank one too high and a
   *  dimension that does not exist. The rank is stated as a number so nobody
   *  counts, and the 0 is named for what it is. `coords` is how many numbers
   *  one key holds; the caller leaves the line out when it is 0. */
  chunkRangeNote: (coords: number): string =>
    `${(coords - 1).toLocaleString()} dimensions. The last number of each offset is always 0: HDF5 writes it as the offset within an element.`,
  /** Over the address strip. Says which axis this is, since the picture above
   *  it is in tree order and the two look alike, and that the rows are the
   *  same rows.
   *
   *  "In the same rows" rather than "and rows": the tree picture ends in a
   *  band of links or chunks, which are not nodes and have no address of their
   *  own here, so it has one band more than the strip has rows. A reader
   *  counting four bands above and three below needs the sentence to say that
   *  the rows being matched are the rows of nodes. */
  stripCaption: "The same nodes, in the same rows, placed by file address",
  /** Under the strip. The strip is zoomed to the tree's own span, not the
   *  file, and a reader who assumed the file would misjudge every distance on
   *  it; the warning sits beside the two numbers that are the span. */
  stripSpan: (from: string, to: string): string => `${from} to ${to} · this tree's span, not the whole file`,
  /** Nodes whose addresses land on one pixel column of the strip, drawn as one
   *  mark. Same shape as `pooled`; the strip merges by address and the tree by
   *  width, so the reason is worded for the strip. */
  stripPooled: (n: number): string => `${countText(n, "node")}, too close together to draw apart`,
  /** Under the summary rows when the walk hit its cap. "At least": the count is
   *  the nodes the walk saw and did not take, not their descendants, which it
   *  never saw. The rail's "{n} more not listed", with the floor said out
   *  loud. */
  omitted: (n: number): string => `At least ${n.toLocaleString()} more nodes not drawn`,
  /** The empty state. One string, because the caller cannot tell why the walk
   *  found nothing: the core reads version 1 trees only, parses a version 2
   *  tree no further than its root, and a file can genuinely have no version 1
   *  tree. So it says only what is known, which is where it looked and that it
   *  found nothing there. Not "this file has no B-trees": a file written with
   *  version 2 trees has plenty, and this sentence claims nothing about the
   *  rest of the file.
   *
   *  It used to go on to name the three ordinary reasons a file has no version
   *  1 tree. That was a paragraph about what the tab does not do, in front of
   *  a reader who wanted a tree and did not get one, and none of it said which
   *  reason applied to the file in front of them. */
  none: "No version 1 B-tree found for the object at the cursor, or for the root group.",
  /** The mouse verbs. The first sentence is the treemap's and means the same.
   *  The second is not: the treemap's "open it" zooms into the box, and here a
   *  double-click puts the node in the Listing, so the destination is named
   *  the way the minimap names its Block section. */
  hint: "Click a box to go to its bytes. Double-click to open it in the Listing.",
  /** On a node whose children the walk did not all reach, in its tooltip and
   *  readout. Both consequences on one line because they always arrive
   *  together: a node with an unreached child keeps no range (see
   *  `hdf5_tree.rs`, `ranges`), so a separate "range not known" line would say
   *  the same thing twice. `noun` is `link` or `chunk` by the tree's job. */
  truncated: (noun: string): string =>
    `Not all of this node's children were read; first and last ${noun} not shown.`,
  /** The rail's two shapes for pending and failed, naming what is being read.
   *  `TREEMAP.failed` says "the fields", which this walk does not read. */
  reading: "Reading the B-tree…",
  failed: (message: string): string => `Couldn't read the B-tree: ${message}`,
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
/**
 * The row of boxes directly above the band of links or chunks, named by what
 * is on it, for `BTREES.leafLinksTitle` and `BTREES.leafChunksTitle`.
 *
 * No numeral when there is one box: "The 1 link table in the row above" is a
 * numeral a reader stops on, and the single box is right there to be counted.
 * "Drawn" only when the walk was capped, which is the case where the row on
 * screen is short of the row in the file and the count would otherwise be read
 * as the file's.
 */
function rowAbove(nodes: number, noun: string, capped: boolean): string {
  const what = nodes === 1 ? noun : countText(nodes, noun);
  return `The ${what}${capped ? " drawn" : ""} in the row above`;
}

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
  return heading(plural(childWord(n)));
}

/**
 * The word over the column of chips beside the hex grid, which is the same
 * question the sidebar's `childrenHead` answers: what is in this list.
 *
 * `Fields` was written into the view as a literal. Inside a deflate stream the
 * column holds Huffman codes, and a heading that calls them fields is the same
 * two-levels-in-one mistake the code panel exists to undo: a code is not a
 * field of the file's structure, it is a symbol of a block's alphabet.
 *
 * One word only where the column has one thing in it. A screen holding a run
 * of codes heads `Codes`; a screen holding a header, a chunk and a run of
 * codes has no one word for its contents, and `Fields` is the honest general
 * one to fall back to. An entry the format has no word for is a field, the
 * same fallback the sidebar takes.
 *
 * Entries covering bytes nothing describes are passed over rather than counted
 * as fields. An unmapped stretch is not a field either, and letting one break
 * the agreement would flip the heading back and forth as the reader scrolled
 * past a gap in the middle of a run.
 */
export function chipsHead(entries: Iterable<{ readonly gap: boolean; readonly unit: string | null }>): string {
  let word: string | null = null;
  for (const entry of entries) {
    if (entry.gap) continue;
    const unit = entry.unit ?? "field";
    if (word === null) word = unit;
    else if (word !== unit) return "Fields";
  }
  return word === null ? "Fields" : heading(plural(word));
}

/** A noun as a heading over the things it counts. */
function heading(word: string): string {
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
  /**
   * A code of a traced block: where it sits in the run, what it decodes to,
   * and how wide it is, which is the whole point of an entropy-coded symbol.
   *
   * The same `run[index]` form as `cell` above, so the number cannot be read
   * as anything but a position. `symbol 0 · literal 'M' · 9 bits`, which this
   * replaces, said "symbol 0" about a thing that is alphabet symbol 77: the
   * index in the run and the symbol the code stands for are two different
   * numbers, and the word for the second was being spent on the first.
   *
   * The run is named from what it holds rather than from the entry the column
   * folded. Beside the bytes a whole block is one entry, and the cells under
   * it are the block's codes, which sit in its `codes` run: `dynamic block,
   * last[46]` would name a different node altogether, since index 46 of the
   * block is a row of its code table.
   */
  code: (unit: string, index: number, text: string, bits: number): string =>
    `${plural(unit)}[${index}] · ${text} · ${bits === 1 ? "1 bit" : `${bits.toLocaleString()} bits`}`,
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
 *  same panel writes `@0x131+4b` for an address four bits into a byte, and one
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
  /** Where the dump starts, when it is not the front of a file. The address
   *  is marked because it is read in a line of prose, beside a byte count
   *  that is not an address. */
  startsAt: (at: number): string => `from ${formatOffset(at * 8)}`,
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
    /** `first field of header`, and `first code of dynamic block` for the
     *  codes of a compressed block, where the reader's word for one of them is
     *  not "field" and the thing they are first of is the block rather than
     *  the run that holds them. The child word comes from what the parent
     *  calls its children, as it does on the Length row's `total length of its
     *  fields`, so one vocabulary answers both. */
    first: (c: HowContext): string => `first ${c.child ?? "field"} of ${c.parent ?? "its parent"}`,
    follows: (c: HowContext): string => (c.field === undefined ? "after the previous field" : `after ${c.field}`),
    element: (c: HowContext): string =>
      c.index === undefined ? `an element of ${c.parent ?? "its parent"}` : `index ${c.index.toLocaleString()} in ${c.parent ?? "its parent"}`,
    pointer: (c: HowContext): string => (c.field === undefined ? "where an offset table points" : `where ${c.field} points`),
    chain: (c: HowContext): string => (c.field === undefined ? "an element of a linked list" : `linked from ${c.field}`),
    address: fromFields,
    /**
     * A step of a decoder's trace, which is now the fallback rather than the
     * answer: a block's own header field, a row of its code tables, and the
     * run of codes itself. The steps of a trace tile, so each of these
     * genuinely begins where the step before it ended, and that is worth
     * saying.
     *
     * `where the decoder read it`, which this replaces, answered by appealing
     * to the decoder rather than to the file. It was true and it was the end
     * of the reader's line of enquiry: there is nothing to go and look at in
     * an answer whose subject is a program. What the codes themselves say now
     * is `after literal 'Z'`, naming the step in front, and this says the same
     * thing where there is no name to give.
     */
    trace: (): string => "after the previous step",
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
    /** A width the format chose and nothing in the file could have chosen
     *  otherwise. Not a code of a deflate fixed-Huffman block, which is that
     *  answer too and has a better one: `DECODED.rfcFixed` names the run of
     *  RFC 1951's own table the symbol falls in, which a reader can check.
     *  This stands for everything else, where the format has one width and
     *  there is no range to give. */
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
    /**
     * A code-length table written in this file settled the width: a literal is
     * nine bits because this block's own table gave symbol 77 a nine-bit code,
     * and the same byte in the next block is very likely a different width.
     *
     * The opposite answer to `fixed` above, and the reason the two are told
     * apart: both come out as a literal number of bits, and "fixed by the
     * format" over a dynamic block's code would be false about the one thing
     * the reader is looking at. The row it names is a row of the block's own
     * table, with the name the listing gives it, so the clause leads to the
     * bits that decided the width. Without a name to give, the table is still
     * the answer and the block still holds it.
     */
    table: (c: HowContext): string => (c.field === undefined ? "from this block's code lengths" : `from ${c.field}`),
    nothing: (): string => "no bytes of its own",
    unknown: (): string => "",
  } satisfies Record<Shape["sized"], (c: HowContext) => string>,

  /**
   * The Length clause for a match, which is the one place `sized.encoded` had
   * to be overruled rather than reworded.
   *
   * The core reports `Encoded` for a match, and correctly: how far it runs is
   * settled by decoding it, exactly as it is for a varint. But `decoded from
   * its own bytes` is written for a varint, and a match has no bytes of its
   * own: it is four runs of bits with no boundary between them, spread across
   * whichever bytes they fall in. Rewording that clause to cover both would
   * have made it vaguer for the varints, which are nearly all of its uses, so
   * this is a case of its own and the table above keeps its one entry per
   * word the core sends.
   *
   * What it says is what the number is a total of, in the words of the two
   * rows above it: `11-bit code, no extra bits` and `1-bit code, no extra
   * bits` are the parts, and this row is their sum. `each code says how many
   * extra bits follow it`, which this replaces, described the mechanism and
   * left the reader to work out that the mechanism was the answer to "why
   * 12". "Total length" rather than "total", for the reason `total length of
   * its fields` gives: a total of two codes reads as adding their values, and
   * the `width` clause two rows up already does arithmetic on values. Not
   * "width", which `Bit width` has taken for the packing width of a run's
   * values. "Its two codes" rather than "each code": it counts the rows to
   * add without renaming them, and pins both, so "total length" cannot be
   * read as the total of the length code alone.
   */
  sizedMatch: (): string => "total length of its two codes and their extra bits",

  /** A switch picked the type. The value is the case that matched, and it is
   *  the half the reader checks: `from chunk_type` alone would send them to
   *  the field to find out which case this was. Without a value to show, which
   *  is what a table read out of a decoder's block has, the field alone. */
  typeFrom: (field: string, value: string): string => (value === "" ? `from ${field}` : `from ${field} = ${value}`),

  /** Over the list of structures round the field, each with the field's
   *  offset inside it: `section_headers[3]  @+0x14`, nearest first. "Offset"
   *  because that is what `@+0x14` is; "within" because the rows are the
   *  things it is within. */
  within: "Offset within",

  /** The same words on the `+` of one of those offsets, naming the structure
   *  it counts from. The name is already three characters to its right, so
   *  this is the least necessary of the two `+` hovers; it is here because a
   *  mark that explains itself in one place and not in the other is a mark the
   *  reader cannot trust. See `DECODED_PLUS_TITLE`. */
  withinPlusTitle: (name: string): string => `Offset within ${name}`,
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
   * One answer, and only where it is worth a row. `none` is shown only where
   * it is a finding: on a field the template calls machinery, where a length
   * that settles no length is worth knowing about. Everywhere else a row
   * saying nothing reads this field is left off, because nearly every field
   * is read by nothing and a row that always says the same thing is a row
   * nobody reads.
   *
   * Three strings stood here and are gone. `not searched` said only that the
   * panel had declined to answer, on field after field. `searched only in
   * {parent}` and `{parent} has 12,000 fields, search limit 400` explained a
   * half-finished search, which is the thing not worth doing: the partial
   * walk costs what the whole one costs, and what it finds cannot be told
   * from the whole answer once it is a list of names on screen. So the search
   * is over the file or it does not run, and a search that does not run
   * leaves no row.
   */
  readBy: {
    found: (n: number): string => (n === 0 ? "none" : countText(n, "field")),
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

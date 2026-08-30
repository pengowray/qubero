// Text that more than one view shows. Two views naming the same thing two ways
// is the reader's problem, not a detail of whichever file happens to draw it.

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

/** Shown where fields would be when nothing has said what the file's are. */
export const NO_TEMPLATE = "No template selected";

/** The same, with the way out, where there is room for a second sentence. */
export const NO_TEMPLATE_HINT = `${NO_TEMPLATE}. Pick one from the Template menu to see the file's fields.`;

/** For a file whose first bytes matched no built-in template. Saying "none
 *  selected" there would suggest an answer exists and the user missed it. */
export const NO_TEMPLATE_MATCH = "No template matched this file. Pick one from the Template menu if you know the format.";

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
  /** Bytes inside a part that none of its fields covers, which the format has
   *  left free: the empty middle of a b-tree page. `GAP_LABEL` above says
   *  "unmapped" for the opposite claim, that the template says nothing about
   *  the bytes at all; both rows can appear in one listing, so neither word
   *  may suggest the other. The mockup's "unused page space" is this row with
   *  the part's own name in it, which the general form cannot know. */
  gap: "unused space",
  /** The verdict beside a gap row: every byte of it was read and each one is
   *  zero. "verified" is doing the work: it says the bytes were looked at,
   *  where a bare "zeros" could pass as a guess about padding. Only a whole
   *  read earns it. A gap past the check's size cap never gets this string,
   *  however many of its bytes were zero so far. */
  gapZeros: "verified zeros",
  /** The same check found at least one byte that is not zero. The exact
   *  negation of `gapZeros` and nothing more: the classifier that sorts runs
   *  into text or high entropy does not run on gaps, so no stronger word is
   *  honest here. The row cannot stay silent instead. With the other verdicts
   *  speaking, a blank cell would mean "checked, nonzero" only to a reader
   *  who has already learned the scheme. */
  gapNonzero: "not all zeros",
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
  /** The last column of a table of records: where in the file the row it just
   *  showed is actually written. The mockup's own heading, and the one thing
   *  in the table that is about the file rather than about the data. */
  storedAt: "stored at",
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
  /** Bytes on this line do not fit the encoding it is being read in. */
  lineLossy: "not this encoding",
  /** Beside the encoding, when nothing in the file said which it was. */
  guessed: "guessed",
  /** The encoding chooser's first entry: let the file decide. */
  encodingAuto: "From the file",
} as const;

/** The offer to open the file a hex dump describes. */
export const DUMP = {
  heading: "This file is a hex dump",
  /** What was found, in one line. */
  summary: (tool: string, bytes: number): string =>
    `${tool === "" ? "A dump" : tool} of ${bytes.toLocaleString()} ${bytes === 1 ? "byte" : "bytes"}`,
  open: "Open those bytes",
  /** Stretches the dump did not describe, which read as zeros. */
  holes: (n: number): string => `${n.toLocaleString()} ${n === 1 ? "stretch" : "stretches"} not in the dump`,
  /** Bytes whose two columns disagree. */
  conflicts: (n: number): string => `${n.toLocaleString()} ${n === 1 ? "byte" : "bytes"} where the columns disagree`,
} as const;

export const NO_MATCH = "No match.";
export const WRAPPED_ON = "Wrapped to the start of the file.";
export const WRAPPED_BACK = "Wrapped to the end of the file.";
export const COUNTING = (n: number): string => `${n.toLocaleString()} matches so far…`;
export const COUNT_STOPPED = (n: number): string => `Stopped at ${n.toLocaleString()} matches.`;
export const COUNTED = (n: number): string => (n === 1 ? "1 match." : `${n.toLocaleString()} matches.`);
export const REPLACED = (n: number): string => (n === 1 ? "Replaced 1 match." : `Replaced ${n.toLocaleString()} matches.`);
export const BAD_REPLACEMENT = "Replacement is hex too: pairs of digits, like 00 ff";

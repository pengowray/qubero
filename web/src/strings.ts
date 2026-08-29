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
  /** The fields that exist to manage another field, folded behind it: a
   *  length prefix, a count, an array of offsets, a run of type codes.
   *  "Bookkeeping" is the standing word for all four jobs; a verb like
   *  "place" fits the offsets but not the types, and reads as a coinage.
   *  `owner` is the field they manage, when it is a sibling; without one the
   *  count stands alone rather than gesturing at an unnamed "what follows". */
  fold: (count: number, owner: string | null): string => {
    const fields = count === 1 ? "bookkeeping field" : "bookkeeping fields";
    return owner === null ? `${count} ${fields}` : `${count} ${fields} for ${owner}`;
  },
  /** The tail of a list longer than what has been drawn so far. `rest` is
   *  already counted and named: "249,800 items". A click draws the next page,
   *  not all of it, so the label promises "more" and states the remainder
   *  instead of promising the whole tail. */
  more: (rest: string): string => `Show more · ${rest} left`,
  /** A stretch whose bytes have not arrived yet. Same situation as the older
   *  listing's, and the same words. */
  reading: "Loading bytes needed to map these fields…",
  /** The control on a heading or a row that shows the bytes behind it, and
   *  the one that puts them away again. Both are the mockup's own. */
  showBytes: "bytes",
  hideBytes: "hide bytes ✕",
  /** A run of bytes shown short, because a strip is for seeing where fields
   *  are rather than for reading every byte of one. The mockup's word. */
  moreBytes: "unused…",
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

export const NO_MATCH = "No match.";
export const WRAPPED_ON = "Wrapped to the start of the file.";
export const WRAPPED_BACK = "Wrapped to the end of the file.";
export const COUNTING = (n: number): string => `${n.toLocaleString()} matches so far…`;
export const COUNT_STOPPED = (n: number): string => `Stopped at ${n.toLocaleString()} matches.`;
export const COUNTED = (n: number): string => (n === 1 ? "1 match." : `${n.toLocaleString()} matches.`);
export const REPLACED = (n: number): string => (n === 1 ? "Replaced 1 match." : `Replaced ${n.toLocaleString()} matches.`);
export const BAD_REPLACEMENT = "Replacement is hex too: pairs of digits, like 00 ff";

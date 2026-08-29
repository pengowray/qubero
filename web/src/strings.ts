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
 * The report listing's own words.
 *
 * TODO(copy): every string in here is a placeholder written to get the view
 * on screen and has not been through review. The mockup
 * (`mockups/c2-listing.html`) has the reviewed wording for the SQLite case;
 * these are the general forms of the same rows and are what needs drafting.
 */
export const REPORT = {
  /** A part of the file made of a run of a structure's plain fields, which
   *  has no field of its own to name it. The mockup calls SQLite's "Header". */
  unnamedPart: "Header",
  /** Bytes inside a part that none of its fields covers. The mockup's SQLite
   *  case reads "unused page space"; `GAP_LABEL` above says "unmapped", which
   *  means something else: that the template describes nothing here. These
   *  bytes are inside something the template does describe and are free. */
  gap: "free space",
  /** Where a part is too small a slice of the file for a percentage to say
   *  anything. */
  tinyShare: "<1%",
  /** The fields that place another field, folded behind it. `owner` is what
   *  they place, when it is one of their siblings. The mockup's SQLite case
   *  reads "page header + 3 cell pointers". */
  fold: (count: number, owner: string | null): string =>
    owner === null ? `${count} fields that place what follows` : `${count} fields that place ${owner}`,
  /** The tail of a list longer than what has been drawn so far. `rest` is
   *  already counted and named: "249,800 items". */
  more: (rest: string): string => `Show ${rest}`,
  /** A stretch whose bytes have not arrived yet. Same situation as the older
   *  listing's, and the same words. */
  reading: "Loading bytes needed to map these fields…",
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

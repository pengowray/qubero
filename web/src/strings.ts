// Text that more than one view shows. Two views naming the same thing two ways
// is the reader's problem, not a detail of whichever file happens to draw it.

/** What a stretch of bytes no field covers is called. */
export const GAP_LABEL = "no field";

/** Shown where fields would be when nothing has said what the file's are. */
export const NO_TEMPLATE = "No template selected";

/** The same, with the way out, where there is room for a second sentence. */
export const NO_TEMPLATE_HINT = `${NO_TEMPLATE}. Pick one from the Template menu to see the file's fields.`;

/** For a file whose first bytes matched no built-in template. Saying "none
 *  selected" there would suggest an answer exists and the user missed it. */
export const NO_TEMPLATE_MATCH = "No template matched this file. Pick one from the Template menu if you know the format.";

// ---- searching ----
// Draft wording; run through ui-text before it ships.

export const SEARCH_LABELS = {
  find: "Find",
  replace: "Replace with",
  kind: "How to read what you typed",
  kinds: { text: "Text", hex: "Hex", regex: "Pattern" },
  fold: "Ignore case",
  next: "Next",
  previous: "Previous",
  count: "Count",
  replaceOne: "Replace",
  replaceAll: "Replace all",
  close: "Close",
  badReplacement: "Replacement needs pairs of hex digits.",
} as const;

export const NO_MATCH = "No match.";
export const WRAPPED_ON = "Wrapped to the start of the file.";
export const WRAPPED_BACK = "Wrapped to the end of the file.";
export const COUNTING = (n: number): string => `${n.toLocaleString()} so far, still counting.`;
export const COUNT_STOPPED = (n: number): string => `Stopped after ${n.toLocaleString()}.`;
export const COUNTED = (n: number): string => (n === 1 ? "1 match." : `${n.toLocaleString()} matches.`);
export const REPLACED = (n: number): string => (n === 1 ? "Replaced 1." : `Replaced ${n.toLocaleString()}.`);

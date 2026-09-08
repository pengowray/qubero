/**
 * A small, semantic colour vocabulary shared by every structural view.
 *
 * Colours describe what bytes mean, never which arbitrary sibling they happen
 * to be. Cursor, selection and active-field state use shape and neutral/accent
 * treatments of their own, so those states remain distinguishable when they
 * overlap.
 */
export function fieldClass(kind: string): string {
  switch (kind) {
    case "uint":
    case "int":
    case "float":
    // A slot nobody filled in is still the number field it always was, and
    // colouring it as binary would hide which run of a header the reader is
    // looking at.
    case "unset":
      return "field-number";
    case "str":
    // Machine code is not text, but a disassembled line reads like one and
    // sits among the strings in every view that shows values. Its own kind for
    // counting the file up, the text colour for looking at it.
    case "insn":
      return "field-text";
    case "magic":
    // The scale a packed block keeps for the weights after it. It is a number,
    // but what a reader wants from it in a table of thirty-two nibbles is
    // where one block stops and the next starts, which is what the marker
    // colour says everywhere else.
    case "scale":
      return "field-marker";
    case "enum":
      return "field-category";
    case "composite":
      return "field-structure";
    // Not a value at all: a record table saying what a row stands for where
    // the format wrote nothing, as a b-tree's last branch has no upper key.
    case "note":
      return "field-note";
    case "bytes":
    case "unread":
    default:
      return "field-binary";
  }
}

/**
 * One colour per top-level part of the file, for the swatch on its heading and
 * the lit stretch of the file map beside it.
 *
 * A different question from `fieldClass`, and so a different palette. That one
 * says what bytes mean and is the same everywhere; this one says only "this
 * part and not that one", so its colours carry no meaning beyond telling the
 * parts apart, and are never used for anything else.
 */
const SECTION_HUES = ["#62c48b", "#e0b04c", "#6cb2ff", "#b48ce0", "#d98a9e", "#7fd4c8"];

/** The colour of bytes no part of the file claims: a grey, so it is never
 *  mistaken for one of the section hues. */
export const UNMAPPED_COLOR = "#8a8f98";

export function sectionColor(section: number): string {
  return SECTION_HUES[((section % SECTION_HUES.length) + SECTION_HUES.length) % SECTION_HUES.length] ?? SECTION_HUES[0] ?? "#888";
}

/**
 * The hues cycled through the fields of one open byte strip.
 *
 * A third question again, and the narrowest: inside one strip, which bytes
 * belong to which field. The colours mean nothing outside that strip and are
 * not the section colours, which say which part of the file something is in.
 * Rule 5 of the mockups: a field's hue appears in exactly three places, its
 * bytes, its bracket and its chip, and nowhere else.
 */
const FIELD_HUES = ["#5b8dd6", "#62c48b", "#c9a45c", "#b48ce0", "#d98a9e"];

export function fieldHue(index: number): string {
  return FIELD_HUES[((index % FIELD_HUES.length) + FIELD_HUES.length) % FIELD_HUES.length] ?? FIELD_HUES[0] ?? "#888";
}

/**
 * What a stretch of bytes is like, without a template: zeros, one repeated
 * byte, text, structured data, or bytes using the whole range about evenly.
 *
 * A fourth palette and the widest question of the four. It is the only one
 * that answers for a file nothing describes, which is why the rail's map is
 * painted in it, and it is here rather than in that panel so the treemap of
 * the same classes is the same five colours: two pictures of one scan that
 * disagreed about which colour "high entropy" is would be two scans as far as
 * a reader is concerned.
 */
const CLASS_LIGHT = ["#e9ebee", "#b9bec7", "#4c9a63", "#6b8fd8", "#d08a2e"];
const CLASS_DARK = ["#23252b", "#4a4f58", "#4f9e63", "#6f93e8", "#cf9440"];

/** The five in the theme on screen. Read afresh each time: the theme follows
 *  the system, and a palette read once is a palette that goes wrong at dusk. */
export function byteClassColors(): readonly string[] {
  return matchMedia("(prefers-color-scheme: dark)").matches ? CLASS_DARK : CLASS_LIGHT;
}

export function byteClassColor(cls: number): string {
  const all = byteClassColors();
  return all[cls] ?? all[3] ?? "#888";
}

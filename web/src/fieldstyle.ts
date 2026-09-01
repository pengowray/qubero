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
      return "field-number";
    case "str":
      return "field-text";
    case "magic":
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

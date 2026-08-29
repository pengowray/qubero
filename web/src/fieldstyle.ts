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

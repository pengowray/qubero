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

// Small pictures a listing row draws beside its value: a strip of a sub-byte
// field's bits, the bytes of a varint, a mark on a time field.
//
// `listingdraw.ts` draws one row per field and asks this registry for each
// one, appending what comes back to the row's value cell. A glyph is word
// sized (the `inline` size in docs/DESIGN-diagrams.md): no text of its own
// beyond a hover card, no wider than a short value, and no taller than the
// row's text, because rows are laid out at a fixed height and a glyph that
// made one taller would push every row under it.
//
// `applies` is asked for every row on every paint, which during a scroll is
// every frame, so it looks at the node and nothing else. `draw` is asked only
// for rows where it applies, and may read bytes; it answers null while they
// are on the way, and the row is painted again when they land.

import type { Doc, TemplateNode } from "../doc.ts";
import { Registry } from "./registry.ts";

export type RowGlyph = {
  readonly id: string;
  readonly applies: (doc: Doc, node: TemplateNode) => boolean;
  readonly draw: (doc: Doc, node: TemplateNode) => HTMLElement | null;
};

const GLYPHS = new Registry<RowGlyph>("row glyph");

export function registerRowGlyph(glyph: RowGlyph): void {
  GLYPHS.add(glyph);
}

export function rowGlyphsFor(doc: Doc, node: TemplateNode): RowGlyph[] {
  return GLYPHS.all().filter((g) => g.applies(doc, node));
}

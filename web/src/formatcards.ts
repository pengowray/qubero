// Which structures a format draws as a card of their own, and the drawing.
//
// A card is for a structure the format keeps in a shape that is not the shape
// it means: a JPEG quantisation table written along the diagonals, a PNG
// palette that is a list of triples and means a grid of colours. Which those
// are is not anything declarative about their fields, so it is a registry of
// readers by template rather than a flag in the IR, the way `records.ts` is.
//
// The listing asks `formatCardKind` of each heading it opens and draws what
// `drawFormatCard` gives back; the report draws the same card at the top of
// the part it stands for (`report/cards.ts`). Neither knows one format from
// another.

import type { Doc, TemplateNode } from "./doc.ts";
import type { Item } from "./flatten.ts";
import { drawJpegCard, jpegCardKind } from "./jpegcards.ts";
import type { DrawContext } from "./listingdraw.ts";
import { drawPngCard, pngCardKind } from "./pngcards.ts";

type CardReader = {
  /** Which card this node is drawn as, or null for a node drawn as its
   *  fields. Opaque outside the reader. */
  readonly kind: (doc: Doc, node: TemplateNode) => string | null;
  readonly draw: (c: DrawContext, item: Extract<Item, { kind: "formatcard" }>) => HTMLElement;
};

/** The formats with cards, by template. */
const READERS: ReadonlyMap<string, CardReader> = new Map([
  ["jpeg", { kind: jpegCardKind, draw: drawJpegCard }],
  ["png", { kind: pngCardKind, draw: drawPngCard }],
]);

/** Which card a node is drawn as, or null for every node drawn as its fields,
 *  which is nearly all of them. */
export function formatCardKind(doc: Doc, node: TemplateNode): string | null {
  return READERS.get(doc.template ?? "")?.kind(doc, node) ?? null;
}

/** The card for an item the listing was told is one. */
export function drawFormatCard(c: DrawContext, item: Extract<Item, { kind: "formatcard" }>): HTMLElement {
  const reader = READERS.get(c.doc.template ?? "");
  return reader === undefined ? document.createElement("div") : reader.draw(c, item);
}

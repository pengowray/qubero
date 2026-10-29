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
//
// A second list holds cards by what a node is rather than by which template
// read it: a struct layout figure can be drawn for any composite, in any
// format. Those wait to be asked for, since a card offered for every
// structure in every file would replace every listing with cards. A heading
// button (`viz/headingbuttons.ts`) asks by toggling `askedCardKey` in the
// listing's card set, and the listing walks the file again with that card in
// place of the fields. Registered from `viz/register.ts`, like every piece.

import type { Doc, TemplateNode } from "./doc.ts";
import type { Item } from "./flatten.ts";
import { drawJpegCard, jpegCardKind } from "./jpegcards.ts";
import type { DrawContext } from "./listingdraw.ts";
import { drawPngCard, pngCardKind } from "./pngcards.ts";
import { Registry } from "./viz/registry.ts";

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

/** A card offered by what a node is, for any template. */
export type KindCard = {
  readonly id: string;
  /** Whether the card can be drawn for this node. Asked for every heading the
   *  listing opens, so it looks at the node and nothing else. */
  readonly offers: (doc: Doc, node: TemplateNode) => boolean;
  /** Whether the card stands in for the fields without being asked. Leave it
   *  off unless the card is plainly better than the fields for every node it
   *  offers itself for. */
  readonly unasked?: boolean;
  readonly draw: (c: DrawContext, item: Extract<Item, { kind: "formatcard" }>) => HTMLElement;
};

const KIND_CARDS = new Registry<KindCard>("card by node kind");
/** What a kind card's answer to `formatCardKind` starts with, so it can never
 *  be mistaken for a template reader's own. */
const KIND_PREFIX = "kind:";

export function registerKindCard(card: KindCard): void {
  KIND_CARDS.add(card);
}

/** The key in the listing's card set that asks for one kind card on one
 *  node. Toggling it there (`DrawContext.toggleCard`) shows the card in place
 *  of the node's fields, and toggling it again puts the fields back. */
export function askedCardKey(id: string, path: readonly number[]): string {
  return `ask:${id}:${path.join(".")}`;
}

/** Which card a node is drawn as, or null for every node drawn as its fields,
 *  which is nearly all of them. The format's own cards come first; then any
 *  kind card that offers itself and was asked for in `asked`, the listing's
 *  card set. */
export function formatCardKind(doc: Doc, node: TemplateNode, asked: ReadonlySet<string> = new Set()): string | null {
  const own = READERS.get(doc.template ?? "")?.kind(doc, node) ?? null;
  if (own !== null) return own;
  for (const card of KIND_CARDS.all()) {
    if (!card.offers(doc, node)) continue;
    if (card.unasked === true || asked.has(askedCardKey(card.id, node.path))) return `${KIND_PREFIX}${card.id}`;
  }
  return null;
}

/** The card for an item the listing was told is one. */
export function drawFormatCard(c: DrawContext, item: Extract<Item, { kind: "formatcard" }>): HTMLElement {
  if (item.card.startsWith(KIND_PREFIX)) {
    const card = KIND_CARDS.get(item.card.slice(KIND_PREFIX.length));
    return card === undefined ? document.createElement("div") : card.draw(c, item);
  }
  const reader = READERS.get(c.doc.template ?? "");
  return reader === undefined ? document.createElement("div") : reader.draw(c, item);
}

// Figures drawn inside a listing heading, under its title line, beside the
// heading's fields rather than in place of them.
//
// A heading button (`headingbuttons.ts`) that wants a figure in place of the
// fields asks for a kind card (`formatcards.ts`). Some figures are about the
// part as a whole and belong above its fields: the tiles of a list's records
// say what the rows below them will be, and hiding the rows to show them would
// take away the thing the tiles lead to. Those register here.
//
// What is open is kept in the listing's card set (`DrawContext.cards`, toggled
// by `toggleCard`), under a key from `figureKey`, since the heading is built
// again from nothing on every paint. The listing draws a heading with a figure
// open at its measured height instead of its fixed one (`figureOpenOn`), and
// lays out again when one opens or closes.
//
// A figure is drawn `width` pixels wide and never measures its host. It is
// rebuilt with the heading, so a figure that pans and zooms keeps its own
// element between paints (see `recordtiles.ts`) rather than starting over.

import type { TemplateNode } from "../doc.ts";
import type { DrawContext } from "../listingdraw.ts";
import type { HeadingItem } from "./headingbuttons.ts";
import { Registry } from "./registry.ts";

export type HeadingFigure = {
  readonly id: string;
  readonly draw: (c: DrawContext, node: TemplateNode, item: HeadingItem, width: number) => HTMLElement | null;
};

const FIGURES = new Registry<HeadingFigure>("heading figure");

const PREFIX = "fig:";

export function registerHeadingFigure(figure: HeadingFigure): void {
  FIGURES.add(figure);
}

/** The key in the listing's card set that opens figure `id` under the
 *  heading at `path`. */
export function figureKey(id: string, path: readonly number[]): string {
  return `${PREFIX}${id}:${path.join(".")}`;
}

export function isFigureKey(key: string): boolean {
  return key.startsWith(PREFIX);
}

/** The heading path a figure key opens a figure under. */
export function figurePath(key: string): string | null {
  if (!isFigureKey(key)) return null;
  const rest = key.slice(PREFIX.length);
  const colon = rest.indexOf(":");
  return colon < 0 ? null : rest.slice(colon + 1);
}

/** Whether any figure is open under the heading at `path`. */
export function figureOpenOn(cards: ReadonlySet<string>, path: readonly number[]): boolean {
  const want = path.join(".");
  for (const key of cards) if (figurePath(key) === want) return true;
  return false;
}

/** The figures open under one heading, drawn. */
export function headingFiguresFor(c: DrawContext, item: HeadingItem, width: number): HTMLElement[] {
  const node = item.node;
  if (node === null) return [];
  const out: HTMLElement[] = [];
  for (const f of FIGURES.all()) {
    if (!c.cards.has(figureKey(f.id, item.path))) continue;
    const drawn = f.draw(c, node, item, width);
    if (drawn !== null) out.push(drawn);
  }
  return out;
}

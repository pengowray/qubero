// Buttons a listing heading offers beside the ones it always had (show bytes,
// open the list in a pane, view as a table).
//
// `listingdraw.ts` draws each heading row and asks this registry what else to
// put on it. A piece registers a button that opens its figure, and the
// listing does not need to know the piece exists. The heading is rebuilt on
// every paint, so anything the button opens has to be remembered outside the
// element: a figure drawn as a card in place of the heading's fields keeps
// its open state in the listing's card set (`DrawContext.cards`, toggled by
// `toggleCard` with a key from `askedCardKey` in `formatcards.ts`), and one
// shown somewhere else keeps it wherever that is.
//
// The draw function makes the whole button, listener included, and stops the
// click from reaching the listing, which would otherwise read it as a press
// on the heading. `headingButton` below does both.

import type { TemplateNode } from "../doc.ts";
import type { Item } from "../flatten.ts";
import type { DrawContext } from "../listingdraw.ts";
import { Registry } from "./registry.ts";

export type HeadingItem = Extract<Item, { kind: "heading" }>;

export type HeadingButton = {
  readonly id: string;
  /** Asked for every heading on every paint: look at the node only. */
  readonly applies: (c: DrawContext, node: TemplateNode, item: HeadingItem) => boolean;
  readonly draw: (c: DrawContext, node: TemplateNode, item: HeadingItem) => HTMLElement;
};

const BUTTONS = new Registry<HeadingButton>("heading button");

export function registerHeadingButton(button: HeadingButton): void {
  BUTTONS.add(button);
}

/** The buttons for one heading. A heading with no field of its own (a run of
 *  plain fields the listing grouped) gets none: there is nothing to draw a
 *  figure of. */
export function headingButtonsFor(c: DrawContext, item: HeadingItem): HTMLElement[] {
  const node = item.node;
  if (node === null) return [];
  return BUTTONS.all()
    .filter((b) => b.applies(c, node, item))
    .map((b) => b.draw(c, node, item));
}

/**
 * A heading button in the listing's own style, the one the bytes and table
 * buttons wear, that runs `press` and nothing else. `on` marks it pressed, for
 * a button that opens and closes something. Like the listing's own Show
 * bytes button, it is hidden until the pointer is on the heading row (or the
 * row has the keyboard), and stays shown while pressed: `.rp-bytes` in
 * style.css. Hover the row before looking for it in a browser test.
 */
export function headingButton(label: string, title: string, on: boolean, press: () => void): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = `rp-bytes${on ? " is-on" : ""}`;
  b.textContent = label;
  b.title = title;
  b.setAttribute("aria-pressed", String(on));
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    press();
  });
  return b;
}

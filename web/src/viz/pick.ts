// Click to go to the bytes, the same way from every picture.
//
// Nothing a piece draws is only a picture: a bar, an arc, a band or a cell is
// a route to the field it stands for. A click selects that field (or that run
// of bits) and every view follows, exactly as clicking a row of the listing
// does. Two halves:
//
// - Marking. A DOM or SVG element that stands for a field carries
//   `data-reads` (its path, steps joined with dots), `data-at` and
//   `data-size` (its first bit and its size in bits), which is what
//   `cardparts.ts` `fieldButton` has always written and the listing has
//   always followed. `markPick` writes them; `followPicks` turns a click on
//   any marked element inside a piece into a move. An element with `data-at`
//   and `data-size` and no `data-reads` stands for the bits alone.
// - Hit testing, for canvas and for anything too thin to point at:
//   `nearestTarget` and `nearestOnAxis` (in `pickmath.ts`) find what the
//   pointer is on, counting every target as at least 24 pixels across.
//
// Either way the move is `goToField` or `goToBits`, which hand it to the
// document's own tab in `main.ts` (`setPickRoute`). That tab does what a pick
// in the rail does: moves the cursor, marks the bits, points the inspector at
// the field, brings the listing to it, and keeps the place being left so Back
// returns there. A piece never touches the hex view itself.

import type { Doc, FieldPick } from "../doc.ts";
import type { BitRange } from "./linked.ts";
import { pickFromData, type Picked } from "./pickmath.ts";

export { nearestOnAxis, nearestTarget, pickFromData, type AxisTarget, type HitTarget, type Picked } from "./pickmath.ts";

/** What a document's tab does with a pick. */
export type PickRoute = {
  readonly field: (pick: FieldPick) => void;
  readonly bits: (ranges: readonly BitRange[]) => void;
};

const routes = new WeakMap<object, PickRoute>();

/** Say where picks on this document go. `main.ts` calls it once per document
 *  tab, and calls what comes back when the tab closes. */
export function setPickRoute(doc: Doc, route: PickRoute): () => void {
  routes.set(doc, route);
  return () => {
    if (routes.get(doc) === route) routes.delete(doc);
  };
}

/** Select a field, from any piece. */
export function goToField(doc: Doc, pick: FieldPick): void {
  routes.get(doc)?.field(pick);
}

/** Select one or more runs of bits that are not a field of their own: a cell
 *  of a raster, a stretch of a map, the bits of a split value. The cursor goes
 *  to the first. */
export function goToBits(doc: Doc, ranges: readonly BitRange[]): void {
  if (ranges.length === 0) return;
  routes.get(doc)?.bits(ranges);
}

export function goTo(doc: Doc, picked: Picked): void {
  if (picked.kind === "field") goToField(doc, picked.pick);
  else goToBits(doc, picked.ranges);
}

/** Mark an element as standing for a field (a pick with a path) or for bits
 *  alone (a range), for `followPicks` and the listing to follow. */
export function markPick(el: Element, what: FieldPick | BitRange): void {
  if ("path" in what) el.setAttribute("data-reads", what.path.join("."));
  el.setAttribute("data-at", String(what.startBit));
  el.setAttribute("data-size", String(what.endBit - what.startBit));
}

/** What the marked element at or around `target` stands for, or null where
 *  the event landed on nothing marked. */
export function pickOf(target: EventTarget | null): Picked | null {
  if (!(target instanceof Element)) return null;
  const el = target.closest("[data-at]");
  if (el === null) return null;
  return pickFromData({
    reads: el.getAttribute("data-reads") ?? undefined,
    at: el.getAttribute("data-at") ?? undefined,
    size: el.getAttribute("data-size") ?? undefined,
  });
}

/** Go to the bytes of whatever marked element a click inside `el` lands on.
 *  Returns the way to stop. */
export function followPicks(el: Element, doc: Doc): () => void {
  const click = (e: Event): void => {
    const picked = pickOf(e.target);
    if (picked === null) return;
    e.stopPropagation();
    goTo(doc, picked);
  };
  el.addEventListener("click", click);
  return () => el.removeEventListener("click", click);
}

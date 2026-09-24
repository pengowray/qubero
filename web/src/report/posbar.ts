// The position bar: a thin bar the width of the file with the stretches of one
// part marked, so a reader arriving from a link sees where in the file they are
// without scrolling back to the map.

import type { Extent } from "./partrules.ts";

/** Parts marked on one bar at most. A group of thousands is drawn as its
 *  first ones; the bar is a place, not a census. */
const MARKS = 400;

export function positionBar(fileBits: number, extents: readonly Extent[], color: string): HTMLElement {
  const bar = document.createElement("div");
  bar.className = "rv-posbar";
  bar.setAttribute("aria-hidden", "true");
  if (fileBits <= 0) return bar;
  for (const e of extents.slice(0, MARKS)) {
    const m = document.createElement("span");
    m.className = "rv-posbar-mark";
    m.style.left = `${(e.offsetBits / fileBits) * 100}%`;
    m.style.width = `max(2px, ${(e.sizeBits / fileBits) * 100}%)`;
    m.style.background = color;
    bar.append(m);
  }
  return bar;
}

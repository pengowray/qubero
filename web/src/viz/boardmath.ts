// The arithmetic half of `board.ts`: what a zoom transform does to a point,
// to a scale's domain, and the transform that fits a box into a view. The
// same three numbers d3-zoom keeps (`k`, `x`, `y`: screen = layout * k + x),
// written out so that a piece and the tests can use them without loading
// d3-zoom, which `board.ts` imports only when a board is first made.

/** Zoom `k` about the origin, then move by (`x`, `y`) screen pixels. */
export type View = { readonly k: number; readonly x: number; readonly y: number };

export const IDENTITY: View = { k: 1, x: 0, y: 0 };

/** Where a layout x lands on screen. */
export function applyX(t: View, x: number): number {
  return x * t.k + t.x;
}

export function applyY(t: View, y: number): number {
  return y * t.k + t.y;
}

/** Which layout x is under a screen x. */
export function invertX(t: View, x: number): number {
  return (x - t.x) / t.k;
}

export function invertY(t: View, y: number): number {
  return (y - t.y) / t.k;
}

/**
 * The stretch of a domain on screen, for a piece that draws over an axis.
 * `domain` is what `range` (layout pixels) shows at no zoom; the answer is
 * what the same range of screen pixels shows under `t`. A piece that keeps a
 * d3 scale passes its domain and range and builds the zoomed scale from the
 * answer, which is d3-zoom's `rescaleX` without needing the scale here.
 */
export function rescaleDomain(t: View, domain: readonly [number, number], range: readonly [number, number]): [number, number] {
  const [d0, d1] = domain;
  const [r0, r1] = range;
  const per = r1 === r0 ? 0 : (d1 - d0) / (r1 - r0);
  const at = (screen: number): number => d0 + (invertX(t, screen) - r0) * per;
  return [at(r0), at(r1)];
}

/** A box in layout pixels. */
export type Box = { readonly x0: number; readonly y0: number; readonly x1: number; readonly y1: number };

/**
 * The transform that shows all of `box`, as large as it fits, centred, in a
 * view `width` by `height` with `pad` pixels kept clear at each edge. With
 * `xOnly` the box is fitted across and nothing moves up or down, which is
 * what a picture over an address axis needs: its height is its own, and only
 * file order is being zoomed.
 */
export function fitTransform(box: Box, width: number, height: number, xOnly: boolean, pad = 8): View {
  const bw = Math.max(1e-9, box.x1 - box.x0);
  const bh = Math.max(1e-9, box.y1 - box.y0);
  const aw = Math.max(1, width - 2 * pad);
  const ah = Math.max(1, height - 2 * pad);
  const k = xOnly ? aw / bw : Math.min(aw / bw, ah / bh);
  const x = (width - bw * k) / 2 - box.x0 * k;
  const y = xOnly ? 0 : (height - bh * k) / 2 - box.y0 * k;
  return { k, x, y };
}

/**
 * Where one edge of the picture may be on screen along one axis, so that the
 * picture is never pulled out of the view. `start` is the screen position of
 * the picture's first edge (`content0 * k + x`), `size` its length on screen
 * (`(content1 - content0) * k`), `view` the view's length, and `slack` how
 * many screen pixels of empty view may show past either edge.
 *
 * A picture longer than the view may be pulled until its edge is `slack`
 * pixels inside the view, and no further. A picture shorter than the view
 * stays where it is drawn at no zoom (its first edge at 0) and may be moved
 * anywhere between there and its far edge `slack` pixels from the far side,
 * so a short row of tiles at the left of a wide figure is not jumped to the
 * middle, and no drag can leave the view empty.
 */
export function holdEdge(start: number, size: number, view: number, slack: number): number {
  const lo = Math.min(0, view - slack - size);
  const hi = Math.max(slack, view - slack - size);
  return Math.max(lo, Math.min(hi, start));
}

/**
 * `t` moved so that `content` stays in a view `width` by `height`, with
 * `share` of the view the most that may show empty past the picture's edges
 * at any zoom. Measured in screen pixels, so it holds at every zoom: a slack
 * counted in layout pixels grows with the zoom until a view zoomed in far
 * enough fits inside the slack and shows nothing at all.
 */
export function holdInView(t: View, content: Box, width: number, height: number, share: number, xOnly: boolean): View {
  const x0 = holdEdge(content.x0 * t.k + t.x, (content.x1 - content.x0) * t.k, width, width * share);
  const x = x0 - content.x0 * t.k;
  if (xOnly) return { k: t.k, x, y: 0 };
  const y0 = holdEdge(content.y0 * t.k + t.y, (content.y1 - content.y0) * t.k, height, height * share);
  return { k: t.k, x, y: y0 - content.y0 * t.k };
}

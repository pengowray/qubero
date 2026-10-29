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

// The arithmetic half of `pick.ts`: which drawn thing a pointer is on, and
// what a marked element says it stands for. Kept apart from the half that
// moves the editor so the tests can run it without a page.
//
// Every drawn target counts as at least `LOD.targetPx` (24) pixels across, so
// a two-pixel segment of a map is still something a reader can point at, and
// a one-pixel arc is picked by its end. A pointer inside a target always
// picks it; a pointer in the padding around several picks the nearest, and
// among targets the pointer is inside, the smallest, which is the innermost
// when they nest.

import type { FieldPick } from "../doc.ts";
import type { BitRange } from "./linked.ts";
import { LOD } from "./lod.ts";

/** A drawn rectangle and what it stands for, in the coordinates the pointer
 *  is given in. A point is a rectangle with no size. */
export type HitTarget<T> = { readonly x0: number; readonly y0: number; readonly x1: number; readonly y1: number; readonly value: T };

/** How far outside its edges a target of this size still takes the pointer. */
function reach(size: number, min: number): number {
  return Math.max(0, (min - size) / 2);
}

/**
 * The target a pointer at (`x`, `y`) is on, or null. A linear scan, so for a
 * piece with tens of thousands of targets on one axis use `nearestOnAxis`,
 * which halves its way there.
 */
export function nearestTarget<T>(targets: readonly HitTarget<T>[], x: number, y: number, min: number = LOD.targetPx): HitTarget<T> | null {
  let best: HitTarget<T> | null = null;
  let bestDist = Number.POSITIVE_INFINITY;
  let bestArea = Number.POSITIVE_INFINITY;
  for (const t of targets) {
    const w = t.x1 - t.x0;
    const h = t.y1 - t.y0;
    const dx = Math.max(t.x0 - x, 0, x - t.x1);
    const dy = Math.max(t.y0 - y, 0, y - t.y1);
    if (dx > reach(w, min) || dy > reach(h, min)) continue;
    const dist = Math.hypot(dx, dy);
    const area = w * h;
    if (dist < bestDist || (dist === bestDist && area < bestArea)) {
      best = t;
      bestDist = dist;
      bestArea = area;
    }
  }
  return best;
}

/** A stretch of one axis and what it stands for, end exclusive. */
export type AxisTarget<T> = { readonly start: number; readonly end: number; readonly value: T };

/**
 * The stretch under `x` on an axis, or the nearest one whose padding reaches
 * it. `spans` are sorted by start and do not overlap, which is what a strip
 * of fields or of parts is; the search halves to `x` and then looks only as
 * far either side as a target can reach.
 */
export function nearestOnAxis<T>(spans: readonly AxisTarget<T>[], x: number, min: number = LOD.targetPx): AxisTarget<T> | null {
  if (spans.length === 0) return null;
  let lo = 0;
  let hi = spans.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((spans[mid] as AxisTarget<T>).start <= x) lo = mid + 1;
    else hi = mid;
  }
  // `lo - 1` is the last stretch starting at or before x.
  const under = spans[lo - 1];
  if (under !== undefined && x < under.end) return under;
  const far = min / 2;
  let best: AxisTarget<T> | null = null;
  let bestDist = Number.POSITIVE_INFINITY;
  const consider = (s: AxisTarget<T>): void => {
    const d = x < s.start ? s.start - x : x - s.end;
    if (d <= reach(s.end - s.start, min) && d < bestDist) {
      best = s;
      bestDist = d;
    }
  };
  for (let i = lo - 1; i >= 0; i--) {
    const s = spans[i] as AxisTarget<T>;
    if (x - s.end > far) break;
    consider(s);
  }
  for (let i = lo; i < spans.length; i++) {
    const s = spans[i] as AxisTarget<T>;
    if (s.start - x > far) break;
    consider(s);
  }
  return best;
}

/** What a marked element stands for: a field of the template, or bits with
 *  no field of their own (a cell of a raster, a stretch of a map). */
export type Picked = { readonly kind: "field"; readonly pick: FieldPick } | { readonly kind: "bits"; readonly ranges: readonly BitRange[] };

/**
 * Read the click contract off an element's data attributes, as `pick.ts`
 * writes them and `cardparts.ts` `fieldButton` always has: `data-reads` is
 * the field's path with its steps joined by dots (empty for the root), and
 * `data-at` and `data-size` are its first bit and its size in bits. Without
 * `data-reads` the element stands for the bits alone.
 */
export function pickFromData(data: { readonly reads?: string | undefined; readonly at?: string | undefined; readonly size?: string | undefined }): Picked | null {
  if (data.at === undefined || data.size === undefined) return null;
  const at = Number(data.at);
  const size = Number(data.size);
  if (!Number.isFinite(at) || !Number.isFinite(size) || data.at === "" || data.size === "") return null;
  if (data.reads === undefined) return { kind: "bits", ranges: [{ startBit: at, endBit: at + size }] };
  const path = data.reads === "" ? [] : data.reads.split(".").map(Number);
  if (path.some((step) => !Number.isInteger(step))) return null;
  return { kind: "field", pick: { path, startBit: at, endBit: at + size } };
}

// The arithmetic of the file map (`filemap.ts`), with no page behind it: which
// stretch of the file each cell of a strip stands for, how wide each cell is
// drawn, which cell and which part a pointer is on, and how much of a cell a
// lit range covers. Also the place of a crumb's field in its parent, for the
// bars under the inspector's trail.
//
// The parts a strip is drawn from are whatever its caller has: the listing's
// top-level headings, the Contents list, a report's groups. They are not
// always in file order and do not always tile the file. A PDF's `objects` is
// one list placed by offsets, so its extent runs from byte 9 to the end and
// the cross-reference table and trailer sit inside it; the listing hands its
// headings over in reading order, not file order. Drawn one after another as
// they come, those parts put the trailer before the objects that surround it
// and the strip stops being a map. So the parts are cut into cells first:
// the file axis divided at every part's edges, each piece drawn as the
// smallest part that covers it, which is the innermost when parts nest. A
// part that something else sits inside becomes two or more cells, the strip
// runs in file order left to right, and every byte is drawn once.

import { nearestOnAxis, type AxisTarget } from "./pickmath.ts";
import { LOD } from "./lod.ts";

/** A stretch of the file in bits. */
export type Stretch = { readonly offsetBits: number; readonly sizeBits: number };

/**
 * One cell of a strip: a stretch of the axis, end exclusive, and the index of
 * the part it is drawn as, or -1 for bytes no part covers. A cell folded from
 * several (`foldCells`) lists every part in it under `owners`, in file order.
 */
export type MapCell = {
  readonly start: number;
  readonly end: number;
  readonly owner: number;
  readonly owners: readonly number[];
};

/** How wide a strip is drawn when its caller does not say, and the least a
 *  cell may shrink to. A hundred bytes of header in a twelve-kilobyte database
 *  is under a pixel of ninety-six honestly drawn, and a part of the file that
 *  cannot be seen cannot be lit. */
export const MAP_WIDTH_PX = 96;
export const MIN_SEGMENT_PX = 2;
export const GAP_PX = 1;
/** The least of a cell a lit range may occupy. A twenty-byte cell inside a
 *  four-kilobyte page is a thousandth of it; without a floor the strip would
 *  say the range is nowhere. */
export const MIN_LIT_FRACTION = 0.06;

/**
 * The parts cut into cells in file order: see the file comment. Parts of no
 * size are left out, since they have nothing to draw. Bytes between parts
 * that no part covers are cells of their own, owned by none, so the strip
 * still has the file's shape; `from` and `to` widen the axis past the parts,
 * which is how a strip of the whole file shows bytes after the last part.
 */
export function mapCells(parts: readonly Stretch[], from?: number, to?: number): MapCell[] {
  const sized: number[] = [];
  for (let i = 0; i < parts.length; i++) if ((parts[i] as Stretch).sizeBits > 0) sized.push(i);
  if (sized.length === 0) return from !== undefined && to !== undefined && to > from ? [{ start: from, end: to, owner: -1, owners: [] }] : [];
  const startOf = (i: number): number => (parts[i] as Stretch).offsetBits;
  const endOf = (i: number): number => startOf(i) + (parts[i] as Stretch).sizeBits;

  // The usual case: parts already in order, end to end or with gaps between.
  let ordered = true;
  for (let k = 1; k < sized.length; k++) {
    if (startOf(sized[k] as number) < endOf(sized[k - 1] as number)) {
      ordered = false;
      break;
    }
  }
  const raw: MapCell[] = [];
  const own = (start: number, end: number, owner: number): void => {
    if (end <= start) return;
    const last = raw[raw.length - 1];
    if (last !== undefined && last.owner === owner && last.end === start) raw[raw.length - 1] = { start: last.start, end, owner, owners: last.owners };
    else raw.push({ start, end, owner, owners: owner < 0 ? [] : [owner] });
  };
  if (ordered) {
    let at = from !== undefined ? Math.min(from, startOf(sized[0] as number)) : startOf(sized[0] as number);
    for (const i of sized) {
      own(at, startOf(i), -1);
      own(startOf(i), endOf(i), i);
      at = endOf(i);
    }
    if (to !== undefined) own(at, to, -1);
    return raw;
  }

  // Parts that overlap: divide at every edge and give each piece to the
  // smallest part covering it. A sweep over the edges keeps the parts open at
  // each piece, so a file of a few thousand parts is not a square of them.
  const edges = new Set<number>();
  for (const i of sized) {
    edges.add(startOf(i));
    edges.add(endOf(i));
  }
  if (from !== undefined) edges.add(from);
  if (to !== undefined) edges.add(to);
  const cuts = [...edges].sort((a, b) => a - b);
  const byStart = [...sized].sort((a, b) => startOf(a) - startOf(b) || a - b);
  const open: number[] = [];
  let next = 0;
  for (let c = 0; c + 1 < cuts.length; c++) {
    const a = cuts[c] as number;
    const b = cuts[c + 1] as number;
    while (next < byStart.length && startOf(byStart[next] as number) <= a) open.push(byStart[next++] as number);
    for (let k = open.length - 1; k >= 0; k--) if (endOf(open[k] as number) <= a) open.splice(k, 1);
    let best = -1;
    for (const i of open) {
      if (best < 0) best = i;
      else {
        const s = (parts[i] as Stretch).sizeBits;
        const t = (parts[best] as Stretch).sizeBits;
        if (s < t || (s === t && i < best)) best = i;
      }
    }
    own(a, b, best);
  }
  return raw;
}

/** How many cells fit across `width` at the least width each, with the gaps
 *  between them. */
export function cellsThatFit(width: number): number {
  return Math.max(1, Math.floor((width + GAP_PX) / (MIN_SEGMENT_PX + GAP_PX)));
}

/**
 * Neighbouring cells merged until there are at most `max`. A folded cell is
 * drawn in the colour of the part with the most bits in it and lists every
 * part in it, so the pointer can still name each one (`partAt`). A strip of
 * fifty thousand tar entries is then a strip of thirty-two cells, each a run
 * of entries, rather than fifty thousand slivers squeezed to a third of a
 * pixel each.
 */
export function foldCells(cells: readonly MapCell[], max: number): MapCell[] {
  if (cells.length <= max) return [...cells];
  const per = Math.ceil(cells.length / Math.max(1, max));
  const out: MapCell[] = [];
  for (let i = 0; i < cells.length; i += per) {
    const group = cells.slice(i, i + per);
    const first = group[0] as MapCell;
    const last = group[group.length - 1] as MapCell;
    const bits = new Map<number, number>();
    const owners: number[] = [];
    for (const c of group) {
      for (const o of c.owners) if (!owners.includes(o)) owners.push(o);
      if (c.owner >= 0) bits.set(c.owner, (bits.get(c.owner) ?? 0) + (c.end - c.start));
    }
    let owner = -1;
    let most = -1;
    for (const [o, n] of bits) {
      if (n > most) {
        owner = o;
        most = n;
      }
    }
    out.push({ start: first.start, end: last.end, owner, owners });
  }
  return out;
}

/**
 * Widths in pixels for each stretch, proportional to its bits except that
 * none falls below `MIN_SEGMENT_PX`, with `GAP_PX` between neighbours.
 *
 * Raising the small ones has to come from somewhere, so the rest are scaled
 * down to pay for it. A stretch already above the floor can be pushed below
 * it that way, so the floor is applied again until it holds everywhere or
 * there is nothing left to take.
 */
export function segmentWidths(segments: readonly Stretch[], width = MAP_WIDTH_PX): number[] {
  const n = segments.length;
  if (n === 0) return [];
  const room = Math.max(0, width - GAP_PX * (n - 1));
  const floor = Math.min(MIN_SEGMENT_PX, room / n);
  const total = segments.reduce((sum, s) => sum + Math.max(0, s.sizeBits), 0);
  if (total <= 0) return segments.map(() => room / n);
  const out = segments.map((s) => (Math.max(0, s.sizeBits) / total) * room);
  for (let pass = 0; pass < n; pass++) {
    const short = out.map((w, i) => (w < floor ? i : -1)).filter((i) => i >= 0);
    if (short.length === 0) break;
    const owed = short.reduce((sum, i) => sum + (floor - (out[i] ?? 0)), 0);
    const spare = out.reduce((sum, w) => sum + Math.max(0, w - floor), 0);
    if (spare <= 0) return out.map(() => room / n);
    const take = Math.min(1, owed / spare);
    for (let i = 0; i < n; i++) {
      const w = out[i] ?? 0;
      out[i] = w < floor ? floor : w - (w - floor) * take;
    }
  }
  return out;
}

/** The cells as stretches, for `segmentWidths`. */
export function cellStretches(cells: readonly MapCell[]): Stretch[] {
  return cells.map((c) => ({ offsetBits: c.start, sizeBits: c.end - c.start }));
}

/** Where each cell is drawn across the strip, in pixels from its left edge,
 *  as hit targets carrying the cell's index. */
export function cellSpans(widths: readonly number[]): AxisTarget<number>[] {
  const out: AxisTarget<number>[] = [];
  let x = 0;
  widths.forEach((w, i) => {
    out.push({ start: x, end: x + w, value: i });
    x += w + GAP_PX;
  });
  return out;
}

/** The cell a pointer `x` pixels from the strip's left edge is on, by the hit
 *  test every piece uses: a two-pixel cell is 24 pixels to the pointer. */
export function cellAt(spans: readonly AxisTarget<number>[], x: number, min: number = LOD.targetPx): number | null {
  return nearestOnAxis(spans, x, min)?.value ?? null;
}

/** The bit under `x` inside a cell drawn across `span`, counted across the
 *  cell's own stretch and kept inside it. */
export function bitInCell(cell: MapCell, span: { readonly start: number; readonly end: number }, x: number): number {
  const w = span.end - span.start;
  const f = w > 0 ? Math.min(1, Math.max(0, (x - span.start) / w)) : 0;
  return Math.min(cell.end - 1, Math.max(cell.start, Math.floor(cell.start + f * (cell.end - cell.start))));
}

/**
 * The part a bit of a cell belongs to: the cell's own for a cell of one part,
 * and for a folded cell the smallest of its parts that covers the bit, or the
 * nearest one when none does. -1 for bytes no part covers.
 */
export function partAt(parts: readonly Stretch[], cell: MapCell, bit: number): number {
  if (cell.owners.length <= 1) return cell.owner;
  let best = -1;
  let bestSize = Number.POSITIVE_INFINITY;
  let near = -1;
  let nearDist = Number.POSITIVE_INFINITY;
  for (const o of cell.owners) {
    const p = parts[o] as Stretch;
    const end = p.offsetBits + p.sizeBits;
    if (bit >= p.offsetBits && bit < end) {
      if (p.sizeBits < bestSize) {
        best = o;
        bestSize = p.sizeBits;
      }
    } else {
      const d = bit < p.offsetBits ? p.offsetBits - bit : bit - end + 1;
      if (d < nearDist) {
        near = o;
        nearDist = d;
      }
    }
  }
  return best >= 0 ? best : near;
}

/**
 * How much of `cell` the ranges cover, as pairs of fractions of its width in
 * order, with ranges that touch or overlap merged. A range too thin to see is
 * widened about its middle to `MIN_LIT_FRACTION` of the cell and kept inside
 * it, so a twenty-byte field still shows where it is in a page of four
 * kilobytes. Empty when none of them reaches the cell.
 */
export function litSpans(cell: { readonly start: number; readonly end: number }, ranges: readonly Stretch[], minFraction = MIN_LIT_FRACTION): [number, number][] {
  const size = cell.end - cell.start;
  if (size <= 0) return [];
  const spans: [number, number][] = [];
  for (const r of ranges) {
    const to = r.offsetBits + Math.max(0, r.sizeBits);
    // A range of no size still marks where it is: a pointer with nothing of
    // its own, a cursor between two bytes.
    const hits = r.sizeBits > 0 ? to > cell.start && r.offsetBits < cell.end : r.offsetBits >= cell.start && r.offsetBits < cell.end;
    if (!hits) continue;
    let a = (Math.max(r.offsetBits, cell.start) - cell.start) / size;
    let b = (Math.min(to, cell.end) - cell.start) / size;
    if (b - a < minFraction) {
      const mid = (a + b) / 2;
      a = Math.max(0, Math.min(1 - minFraction, mid - minFraction / 2));
      b = a + minFraction;
    }
    spans.push([a, b]);
  }
  spans.sort((x, y) => x[0] - y[0]);
  const merged: [number, number][] = [];
  for (const s of spans) {
    const last = merged[merged.length - 1];
    if (last !== undefined && s[0] <= last[1]) last[1] = Math.max(last[1], s[1]);
    else merged.push([s[0], s[1]]);
  }
  return merged;
}

/**
 * Where a field sits in the field it is part of, as the fractions of the
 * parent's width it starts and ends at, for the small bar under each crumb of
 * the inspector's trail. A field too small to see is widened to `minFraction`
 * about its middle, the way a lit range is on the map. Null when the parent
 * has no size, or the child is counted in another address space (the bytes
 * a stream unpacks to are not anywhere inside the stream's packed bytes), or
 * when the two do not overlap.
 */
export function placeIn(child: Stretch & { readonly space?: number }, parent: Stretch & { readonly space?: number }, minFraction: number): readonly [number, number] | null {
  if (parent.sizeBits <= 0) return null;
  if ((child.space ?? 0) !== (parent.space ?? 0)) return null;
  const spans = litSpans({ start: parent.offsetBits, end: parent.offsetBits + parent.sizeBits }, [child], minFraction);
  if (spans.length === 0) return null;
  const first = spans[0] as [number, number];
  return [first[0], first[1]];
}

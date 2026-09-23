// The rules that divide a file into the parts the report talks about, and put
// them in the order it reads them in. No DOM and no document here, so each
// rule can be run under `node --test`; `model.ts` feeds them from the file.

/** A stretch of the file, in bits. */
export type Extent = { readonly offsetBits: number; readonly sizeBits: number };

/** What a list element is called in the listing, without its place: `[3]
 *  dqt, quantisation tables` is `dqt, quantisation tables`. Empty when the
 *  element has no name but its place. */
export function stripIndex(name: string): string {
  return name.replace(/^\[\d+\]\s*/, "");
}

/** The value a switch was decided by, as `origins` gives it, without the
 *  number in brackets an enum adds: `local file (0x4034b50)` is `local file`. */
export function variantText(value: string): string {
  return value.replace(/\s*\((?:0x[0-9a-f]+|\d+)\)$/i, "").trim();
}

/**
 * Children in file order, split into runs of plain fields and the rest. A
 * run of three header fields is one part, "Header", the way the listing makes
 * it one heading; a structure or a list is a part of its own.
 */
export function runsOf<T>(kids: readonly T[], plain: (k: T) => boolean): (readonly T[])[] {
  const out: T[][] = [];
  let run: T[] | null = null;
  for (const k of kids) {
    if (plain(k)) {
      if (run === null) {
        run = [];
        out.push(run);
      }
      run.push(k);
    } else {
      run = null;
      out.push([k]);
    }
  }
  return out;
}

/** Where an unnamed run of fields sits, which is all there is to name it by.
 *  The same rule the listing and the rail use. */
export function runPosition(e: Extent, fileBits: number): "start" | "end" | "middle" {
  if (e.offsetBits === 0) return "start";
  return fileBits > 0 && e.offsetBits + e.sizeBits >= fileBits ? "end" : "middle";
}

/** The stretches between `from` and `to` that no extent covers, in order. The
 *  extents may overlap and need not be sorted. */
export function gapsOf(extents: readonly Extent[], from: number, to: number): Extent[] {
  const sorted = [...extents].filter((e) => e.sizeBits > 0).sort((a, b) => a.offsetBits - b.offsetBits);
  const out: Extent[] = [];
  let at = from;
  for (const e of sorted) {
    if (e.offsetBits > at) out.push({ offsetBits: at, sizeBits: Math.min(e.offsetBits, to) - at });
    at = Math.max(at, e.offsetBits + e.sizeBits);
    if (at >= to) break;
  }
  if (at < to) out.push({ offsetBits: at, sizeBits: to - at });
  return out.filter((g) => g.sizeBits > 0);
}

/**
 * The order the report reads its parts in: a part that another one places or
 * sizes comes before the one that places it, and parts with nothing between
 * them go largest first. `places[i]` lists the parts part `i` places.
 *
 * A header that sizes the chunks after it therefore comes after them, which is
 * the order every hand-written report chose: the content, then the structure
 * that produces it, then the header. A loop, which a template can make, is
 * broken at its largest part rather than stopping the order.
 */
export function readingOrder(sizes: readonly number[], places: readonly (readonly number[])[]): number[] {
  const n = sizes.length;
  const waiting = sizes.map((_, i) => new Set((places[i] ?? []).filter((j) => j !== i && j >= 0 && j < n)));
  const done = new Set<number>();
  const out: number[] = [];
  const bySize = (a: number, b: number): number => (sizes[b] ?? 0) - (sizes[a] ?? 0) || a - b;
  while (out.length < n) {
    const ready: number[] = [];
    for (let i = 0; i < n; i++) {
      if (done.has(i)) continue;
      const w = waiting[i];
      if (w === undefined || [...w].every((j) => done.has(j))) ready.push(i);
    }
    const pool = ready.length > 0 ? ready : [...Array(n).keys()].filter((i) => !done.has(i));
    pool.sort(bySize);
    const next = pool[0];
    if (next === undefined) break;
    done.add(next);
    out.push(next);
  }
  return out;
}

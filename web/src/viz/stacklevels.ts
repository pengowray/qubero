// Which levels the zoom stack (`zoomstack.ts`) draws for a field, and where
// each level's lens starts in the level above. DOM-free, for the tests.
//
// The stack is one strip per step of the path from the file down to the
// field: each strip is a structure's children across its own bytes, with the
// child the path goes through lit, and under it that child drawn the full
// width with its own children. Lens lines join the lit child to the strip
// below, so the reader sees each level as a close-up of a piece of the one
// above. Not every step of a path is worth a strip:
// - a structure with one child is a wrapper (the ZIP file and its `records`
//   list, a field that points at what it reads), and its strip would be one
//   bar the full width; the strip below says everything it would;
// - a compressed run's children are in two address spaces at once (the bits
//   of its blocks in the file, and the stream it unpacks to), so a strip of
//   them would draw offsets of two spaces on one axis. The path goes on in
//   the stream, and the stream's strip hangs from the compressed run's bar
//   in the strip above, marked as unpacked.
// A deep path keeps its first strip, the whole file, and its last few, since
// those are the two ends a reader asks about.

/** What the plan reads of a node. */
export type StackNode = {
  readonly path: readonly number[];
  readonly name: string;
  readonly offset_bits: number;
  readonly size_bits: number;
  readonly space: number;
  readonly composite: boolean;
  readonly child_count: number;
};

/** One strip of the stack: the structure drawn, the child of it on the path
 *  (null for the field itself, drawn with its own children), and, for a
 *  strip in a different address space from the one above, the compressed run
 *  it was unpacked from. `source` is where the lens starts in the strip
 *  above: a stretch in that strip's own offsets. Null for the first strip. */
export type StackLevel<N extends StackNode = StackNode> = {
  readonly parent: N;
  readonly lit: N | null;
  readonly unpackedFrom: N | null;
  readonly source: { readonly offsetBits: number; readonly sizeBits: number } | null;
};

/** Most strips drawn. The first is kept and the rest are the deepest. */
export const MAX_LEVELS = 6;

/**
 * The strips for the path whose nodes, from the root down to the field, are
 * `chain`. Returns them top to bottom and how many steps were left out of
 * the middle of a deep path.
 */
export function stackPlan<N extends StackNode>(chain: readonly N[], max = MAX_LEVELS): { levels: StackLevel<N>[]; skipped: number } {
  type Pick = { parent: N; lit: N | null };
  const picked: Pick[] = [];
  for (let k = 0; k < chain.length; k++) {
    const p = chain[k] as N;
    const c = chain[k + 1] ?? null;
    if (c === null) {
      // The field itself, when it has fields of its own to show.
      if (p.composite && p.child_count > 1) picked.push({ parent: p, lit: null });
      continue;
    }
    if (c.space !== p.space) continue;
    // A stream's first node is kept however few children it has: it is the
    // strip of the unpacked bytes, the one place the field's offset means
    // anything.
    const before = chain[k - 1];
    const streamStart = before !== undefined && before.space !== p.space;
    if (p.child_count <= 1 && !streamStart) continue;
    picked.push({ parent: p, lit: c });
  }
  let kept = picked;
  let skipped = 0;
  if (picked.length > max) {
    kept = [picked[0] as Pick, ...picked.slice(picked.length - (max - 1))];
    skipped = picked.length - max;
  }
  const levels: StackLevel<N>[] = [];
  kept.forEach((lv, j) => {
    const above = kept[j - 1];
    if (above === undefined) {
      levels.push({ parent: lv.parent, lit: lv.lit, unpackedFrom: null, source: null });
      return;
    }
    const space = above.parent.space;
    // The lens starts at the deepest node on the way down that is still in
    // the space of the strip above: the structure itself when the two strips
    // share a space, the compressed run when this one is its stream.
    const at = chain.indexOf(lv.parent);
    let from: N = lv.parent;
    for (let k = at; k >= 0; k--) {
      const n = chain[k] as N;
      if (n.space === space) {
        from = n;
        break;
      }
    }
    const unpacked = lv.parent.space !== space;
    levels.push({
      parent: lv.parent,
      lit: lv.lit,
      unpackedFrom: unpacked ? from : null,
      source: { offsetBits: from.offset_bits, sizeBits: from.size_bits },
    });
  });
  return { levels, skipped };
}

/** The stretch a strip's axis covers: its structure's bytes, widened to take
 *  in any child placed outside them, so a field reached by an offset is still
 *  drawn where it is. In the strip's own space only. */
export function levelDomain(parent: StackNode, kids: readonly StackNode[]): readonly [number, number] {
  let a = parent.offset_bits;
  let b = parent.offset_bits + parent.size_bits;
  for (const k of kids) {
    if (k.size_bits <= 0 || k.space !== parent.space) continue;
    a = Math.min(a, k.offset_bits);
    b = Math.max(b, k.offset_bits + k.size_bits);
  }
  return [a, Math.max(b, a + 1)];
}

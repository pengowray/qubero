// SHIM. The elements of a folded run, built out of `templateChildren` until
// the core answers `doc.runCells(path, fromBit, toBit, max)` in the same shape
// (see HANDOVER-values.md, work package A). When that lands, the one call in
// `hexview.ts` moves over to it and this file goes.
//
// What it covers: a run of fixed stride, which is what an array of numbers is
// — WAV samples, a table of offsets, a tensor of weights. The index of the
// first element on screen is arithmetic, and `templateChildren` reads the rest
// in one go.
//
// What it does not: variable-length elements (MIDI events, varints), traced
// blocks of coded symbols, and anything whose bits are not one contiguous run.
// Those need the core to walk from the last element it knows, which is what
// `run_cells` is for; here they simply have no table.
import type { Doc, Span } from "./doc.js";
import type { Cell } from "./valuetable.js";

/** The stride of a run whose elements are all one size, or 0 for one whose
 *  elements are not. `parts` holds the extent of the first elements and then
 *  the rest, so a fixed-stride run says the same size twice over. */
export function runStride(span: Span): number {
  if (span.count <= 0 || span.size_bits <= 0) return 0;
  const first = span.parts[0];
  const stride = first !== undefined && !first.rest && first.size_bits > 0 ? first.size_bits : span.size_bits / span.count;
  if (!Number.isFinite(stride) || stride <= 0) return 0;
  // Fixed stride only: a run whose size is not its count times its stride has
  // elements of more than one length, and the arithmetic below would put every
  // value under the wrong bytes.
  return Math.round(span.size_bits / stride) === span.count ? stride : 0;
}

/**
 * The run's elements whose bits overlap `fromBit..toBit`, at most `max`.
 *
 * Null rather than an empty list when the answer is not ready or the run is
 * not one this shim can read: the caller keeps the last table on screen for
 * the first, and draws no table at all for the second.
 */
export function runCellsShim(doc: Doc, span: Span, fromBit: number, toBit: number, max: number): Cell[] | null {
  const stride = runStride(span);
  if (stride === 0) return null;
  const from = Math.max(0, Math.floor((fromBit - span.offset_bits) / stride));
  const to = Math.min(span.count, Math.ceil((toBit - span.offset_bits) / stride));
  if (to <= from) return [];
  const r = doc.templateChildren(span.path, from, Math.min(to, from + max));
  if (r.status !== "ok") return null;
  return r.node.map((n, i) => ({
    index: from + i,
    offset_bits: n.offset_bits,
    size_bits: n.size_bits,
    // A record with no reading of its own is named instead, as the listing
    // names it.
    text: n.value === "" ? n.name : n.value,
    kind: n.kind,
    contiguous: true,
  }));
}

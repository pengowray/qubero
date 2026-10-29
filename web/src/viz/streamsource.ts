// Where in the file a stream's bytes come from, for the pictures that draw a
// field inside a stream next to the file: the zoom stack's dashed lens
// (`zoomstack.ts`) and the file map's second ring (`filemap.ts`).
//
// A field inside a stream is at an offset of the bytes the stream gives,
// which is nowhere in the file. To draw it beside the file a picture needs
// the stretch of the file those bytes were read from, and the template says
// that in one of two ways:
// - a compressed run is a field with bytes of its own (a ZIP entry's
//   `compressed`, a PNG zlib stream's deflate data), and the stream it
//   unpacks to hangs under it. The run's own bytes are the answer.
// - a stream joined from several runs (a PNG's IDAT chunks, a PDB's pages)
//   hangs from a field of no size that points at it, placed after the last
//   byte of what holds it. Its offset says nothing about where the bytes
//   are; drawn from there, the lens would claim the image came from the end
//   of the file. The runs are found with `Doc.partOf`, which names the run a
//   field of a joined stream starts in: the run of the stream's first field
//   and the run of its last, and the stretch from one to the other.

import type { Doc, TemplateNode } from "../doc.ts";
import type { Stretch } from "./mapcells.ts";

/**
 * The stretch of `from`'s address space that the stream starting at
 * `stream` was read from. `from` is the node the stream hangs from, the last
 * one on the way down still in that space. Null when the template does not
 * say, or not yet.
 */
export function packedStretch(doc: Doc, from: TemplateNode, stream: TemplateNode): Stretch | null {
  if (from.size_bits > 0) return { offsetBits: from.offset_bits, sizeBits: from.size_bits };
  if (!stream.joined) return null;
  const first = runOf(doc, stream.path, from.space);
  if (first === null) return null;
  let last = first;
  if (stream.child_count > 0) {
    const tail = doc.templateChildren(stream.path, stream.child_count - 1, stream.child_count);
    const end = tail.status === "ok" ? tail.node[0] : undefined;
    if (end !== undefined) last = runOf(doc, end.path, from.space) ?? first;
  }
  const a = Math.min(first.offset_bits, last.offset_bits);
  const b = Math.max(first.offset_bits + first.size_bits, last.offset_bits + last.size_bits);
  return { offsetBits: a, sizeBits: b - a };
}

/** The run the field at `path` of a joined stream starts in, as a node of
 *  `space`. */
function runOf(doc: Doc, path: readonly number[], space: number): TemplateNode | null {
  const part = doc.partOf(path);
  if (part === null || part.run_space !== space) return null;
  const run = doc.templateNode(part.path);
  return run.status === "ok" && run.node.space === space ? run.node : null;
}

// Where a field sits inside the structures around it, as the inspector says it:
// `@+0x14 in section_headers[3]`, nearest first. Kept apart from `inspector.ts`
// so that the rule can be read and tested without a document.

import { ADDRESS_MARK, offsetDigits } from "./format.ts";
import { PROPERTIES } from "./strings.ts";
import type { TemplateNode } from "./doc.ts";
import type { PartGroup, PartLine } from "./joinedpart.ts";

/** How many of the structures a field sits inside are worth an offset each. */
export const INSIDE_LEVELS = 3;

/**
 * The field's offset inside each of the nearest few structures above it.
 *
 * An absolute address answers where it is in the file, which is not the
 * question a reader of a record has: a local file header's `crc32` is at `+0xe`
 * of that header wherever in the zip the header landed, and that is the number
 * the specification prints. The nearest few are enough; a member eight levels
 * down a JSON tree has eight of these and only the first are about anything the
 * reader can hold in their head.
 *
 * Only a structure the field starts inside is one it is within. A field placed
 * by an offset is a node of no bits where it is declared, with what the offset
 * names as its one child, so the two are ancestors of everything under it. The
 * child holds the field; the node of no bits holds nothing. Counted, it gave a
 * second row saying the same thing whenever the offset named the place the
 * field was declared, which a PDB type stream's records do, and a row counting
 * from somewhere the field is not whenever it named anywhere else.
 *
 * `nodeAt` reads a field of the same document, or answers null for one that
 * does not read. Null comes back when there is no row to give.
 */
export function withinGroup(path: readonly number[], n: TemplateNode, nodeAt: (path: readonly number[]) => TemplateNode | null): PartGroup | null {
  const lines: PartLine[] = [];
  for (let i = path.length - 1; i >= 1 && lines.length < INSIDE_LEVELS; i--) {
    const at = path.slice(0, i);
    const a = nodeAt(at);
    // A stretch counted in another address space is not a distance from here:
    // the field is at an offset of the unpacked bytes and its stream is at an
    // offset of the file.
    if (a === null || a.space !== n.space) continue;
    // A structure that starts where addresses count from gives the address
    // over again with a plus in front of it.
    if (a.offset_bits === 0) continue;
    const delta = n.offset_bits - a.offset_bits;
    if (!startsInside(delta, n.size_bits, a.size_bits)) continue;
    lines.push({
      text: PROPERTIES.withinAt(a.name, `${ADDRESS_MARK}+${offsetDigits(delta)}`),
      plus: PROPERTIES.withinPlusTitle(a.name),
      title: null,
      path: at,
      place: true,
      stored: null,
    });
  }
  return lines.length === 0 ? null : { head: PROPERTIES.within, lines };
}

/** Whether a field `delta` bits into a structure of `outer` bits starts inside
 *  it. A field of no bits may sit at the very end of one. */
function startsInside(delta: number, size: number, outer: number): boolean {
  if (delta < 0) return false;
  return size === 0 ? delta <= outer : delta < outer;
}

// A chunk of an HDF5 dataset, unpacked. A chunked dataset writes each chunk
// through a list of filters, so what the hex view shows is the last filter's
// output and the numbers are nowhere in the file. This is the walk back: each
// filter, what went in, what came out, and the elements at the end of it.
// Nothing here can be clicked through to, because none of these bytes are in
// the file.

import type { TypeInfo } from "./doc.ts";
import { countText } from "./strings.ts";
import { bytesChange, firstValues, line, problemLine, stepList } from "./steplist.ts";

/** How many elements came out, for the note beside the heading. */
export function chunkNote(info: TypeInfo): string {
  if (info.chunk_total === 0) return "";
  return countText(info.chunk_total, "element");
}

/**
 * The whole panel: the sizes, the filters in the order they were undone, and
 * the first values.
 *
 * A chunk that would not unpack still gets a panel: the filters that were
 * undone before the one that stopped it are how a reader tells an unusual file
 * from a gap in this program.
 */
export function chunkBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();

  const packed = `${info.chunk_packed.toLocaleString()} bytes in the file`;
  const unpacked = info.chunk_decoded > 0 ? `, ${info.chunk_decoded.toLocaleString()} bytes unpacked` : "";
  frag.append(line("insp-qcount", packed + unpacked));

  const rows = info.chunk_steps.map((s) => ({
    label: s.filter,
    text: s.skipped ? "not applied to this chunk" : bytesChange(s.in_bytes, s.out_bytes),
  }));
  frag.append(stepList("Filters, in the order they were undone", rows));

  if (info.problem !== "") {
    frag.append(problemLine(info.problem));
    return frag;
  }

  frag.append(firstValues(`First elements, as ${info.chunk_element_type}`, info.chunk_values, info.chunk_total, "elements"));
  return frag;
}

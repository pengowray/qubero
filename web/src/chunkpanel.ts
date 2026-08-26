// A chunk of an HDF5 dataset, unpacked. A chunked dataset writes each chunk
// through a list of filters, so what the hex view shows is the last filter's
// output and the numbers are nowhere in the file. This is the walk back: each
// filter, what went in, what came out, and the elements at the end of it.
// Nothing here can be clicked through to, because none of these bytes are in
// the file.

import type { ChunkStep, TypeInfo } from "./doc.js";
import { countText } from "./strings.js";

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** How many elements came out, for the note beside the heading. */
export function chunkNote(info: TypeInfo): string {
  if (info.chunk_total === 0) return "";
  return countText(info.chunk_total, "element");
}

/** One filter: what it was, and what it did to the size. */
function stepLine(step: ChunkStep): HTMLElement {
  const line = document.createElement("div");
  line.className = "insp-orow";
  line.append(span("insp-orow-object", step.filter));
  const change = step.skipped
    ? "not applied to this chunk"
    : `${step.in_bytes.toLocaleString()} → ${step.out_bytes.toLocaleString()} bytes`;
  line.append(span("insp-orow-text", change));
  return line;
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

  const sizes = document.createElement("div");
  sizes.className = "insp-qcount";
  const packed = `${info.chunk_packed.toLocaleString()} bytes in the file`;
  const unpacked = info.chunk_decoded > 0 ? `, ${info.chunk_decoded.toLocaleString()} bytes unpacked` : "";
  sizes.textContent = packed + unpacked;
  frag.append(sizes);

  if (info.chunk_steps.length > 0) {
    frag.append(span("insp-qsubhead", "Filters, in the order they were undone"));
    const list = document.createElement("div");
    list.className = "insp-orows";
    for (const s of info.chunk_steps) list.append(stepLine(s));
    frag.append(list);
  }

  if (info.problem !== "") {
    const p = document.createElement("div");
    p.className = "insp-xproblem";
    p.textContent = info.problem;
    frag.append(p);
    return frag;
  }

  if (info.chunk_values.length > 0) {
    frag.append(span("insp-qsubhead", `First elements, as ${info.chunk_element_type}`));
    const values = document.createElement("div");
    values.className = "insp-orow";
    values.append(span("insp-orow-text", info.chunk_values.join("  ")));
    frag.append(values);
    if (info.chunk_values.length < info.chunk_total) {
      frag.append(
        span(
          "insp-qcount",
          `Showing the first ${info.chunk_values.length.toLocaleString()} of ${info.chunk_total.toLocaleString()} elements.`,
        ),
      );
    }
  }
  return frag;
}

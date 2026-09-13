// A page of a Parquet column chunk, read the whole way. What the hex view shows
// under a page is packed bytes, and the values are nowhere in the file as they
// stand, so this is the reading laid out: the codec, each list of levels, the
// encoding, and the values at the end of it. The order of the steps is real
// information: a DATA_PAGE_V2 keeps its levels in front of the packed part and
// reads them before the codec, and a v1 page reads them after. Nothing here
// can be clicked through to, because none of these values are in the file.

import type { PageStep, PageInfo } from "./doc.ts";
import { countText } from "./strings.ts";
import { bytesChange, firstValues, line, problemLine, stepList } from "./steplist.ts";

/** How many values came out, for the note beside the heading. */
export function pageNote(info: PageInfo): string {
  if (info.total === 0) return "";
  return countText(info.total, "value");
}

/**
 * What one step did. A step that produced bytes says how many went in and how
 * many came out; one that produced values says how many in its note.
 * BYTE_STREAM_SPLIT does both, putting bytes back together and then reading
 * them, so both are shown.
 */
function stepText(step: PageStep): string {
  if (step.skipped) return "not applied to this page (is_compressed = false)";
  const parts: string[] = [];
  if (step.out_bytes > 0) parts.push(bytesChange(step.in_bytes, step.out_bytes));
  if (step.note !== "") parts.push(step.note);
  return parts.length > 0 ? parts.join(", ") : `${step.in_bytes.toLocaleString()} bytes`;
}

/**
 * The whole panel: the sizes, the steps in the order they were done, and the
 * first values.
 *
 * A page that would not read still gets a panel: the steps done before the
 * one that stopped it are how a reader tells an unusual file from a gap in
 * this program.
 */
export function pageBody(info: PageInfo): DocumentFragment {
  const frag = document.createDocumentFragment();

  const packed = `${info.packed.toLocaleString()} bytes in the file`;
  const unpacked =
    info.decoded > 0 && info.decoded !== info.packed
      ? `, ${info.decoded.toLocaleString()} bytes unpacked`
      : "";
  frag.append(line("insp-qcount", packed + unpacked));

  const rows = info.steps.map((s) => ({ label: s.what, text: stepText(s) }));
  frag.append(stepList("Steps, in the order they were done", rows));

  if (info.problem !== "") {
    frag.append(problemLine(info.problem));
    return frag;
  }

  // Indices into the dictionary page are not the column's values, and a
  // heading saying "as dictionary index" would read as a type.
  const indices = info.element_type === "dictionary index";
  const head = indices ? "First dictionary indices" : `First values, as ${info.element_type}`;
  frag.append(firstValues(head, info.values, info.total, indices ? "indices" : "values"));
  return frag;
}

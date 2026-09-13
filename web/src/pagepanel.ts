// A page of a Parquet column chunk, read the whole way. What the hex view shows
// under a page is packed bytes, and the values are nowhere in the file as they
// stand, so this is the reading laid out: the codec, each list of levels, the
// encoding, and the values at the end of it. The order of the steps is real
// information: a DATA_PAGE_V2 keeps its levels in front of the packed part and
// reads them before the codec, and a v1 page reads them after. Nothing here
// can be clicked through to, because none of these values are in the file.

import type { PageStep, TypeInfo } from "./doc.ts";
import { countText } from "./strings.ts";

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** How many values came out, for the note beside the heading. */
export function pageNote(info: TypeInfo): string {
  if (info.page_total === 0) return "";
  return countText(info.page_total, "value");
}

/**
 * One step: what it was, and what it did. A step that produced bytes says how
 * many went in and how many came out; one that produced values says how many
 * in its note. BYTE_STREAM_SPLIT does both, putting bytes back together and
 * then reading them, so both are shown.
 */
function stepLine(step: PageStep): HTMLElement {
  const line = document.createElement("div");
  line.className = "insp-orow";
  line.append(span("insp-orow-object", step.what));
  let text: string;
  if (step.skipped) {
    text = "not applied to this page (is_compressed = false)";
  } else {
    const parts: string[] = [];
    if (step.out_bytes > 0) parts.push(`${step.in_bytes.toLocaleString()} → ${step.out_bytes.toLocaleString()} bytes`);
    if (step.note !== "") parts.push(step.note);
    text = parts.length > 0 ? parts.join(", ") : `${step.in_bytes.toLocaleString()} bytes`;
  }
  line.append(span("insp-orow-text", text));
  return line;
}

/**
 * The whole panel: the sizes, the steps in the order they were done, and the
 * first values.
 *
 * A page that would not read still gets a panel: the steps done before the
 * one that stopped it are how a reader tells an unusual file from a gap in
 * this program.
 */
export function pageBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();

  const sizes = document.createElement("div");
  sizes.className = "insp-qcount";
  const packed = `${info.page_packed.toLocaleString()} bytes in the file`;
  const unpacked =
    info.page_decoded > 0 && info.page_decoded !== info.page_packed
      ? `, ${info.page_decoded.toLocaleString()} bytes unpacked`
      : "";
  sizes.textContent = packed + unpacked;
  frag.append(sizes);

  if (info.page_steps.length > 0) {
    frag.append(span("insp-qsubhead", "Steps, in the order they were done"));
    const list = document.createElement("div");
    list.className = "insp-orows";
    for (const s of info.page_steps) list.append(stepLine(s));
    frag.append(list);
  }

  if (info.problem !== "") {
    const p = document.createElement("div");
    p.className = "insp-xproblem";
    p.textContent = info.problem;
    frag.append(p);
    return frag;
  }

  if (info.page_values.length > 0) {
    // Indices into the dictionary page are not the column's values, and a
    // heading saying "as dictionary index" would read as a type.
    const indices = info.page_element_type === "dictionary index";
    const head = indices ? "First dictionary indices" : `First values, as ${info.page_element_type}`;
    frag.append(span("insp-qsubhead", head));
    const values = document.createElement("div");
    values.className = "insp-orow";
    values.append(span("insp-orow-text", info.page_values.join("  ")));
    frag.append(values);
    if (info.page_values.length < info.page_total) {
      const noun = indices ? "indices" : "values";
      frag.append(
        span(
          "insp-qcount",
          `Showing the first ${info.page_values.length.toLocaleString()} of ${info.page_total.toLocaleString()} ${noun}.`,
        ),
      );
    }
  }
  return frag;
}

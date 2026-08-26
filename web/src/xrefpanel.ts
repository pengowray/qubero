// A PDF cross-reference stream, taken apart. The table that says where every
// object in the file is written is, since PDF 1.5, allowed to be compressed
// inside an object, so the hex view can only show the compressed bytes. This is
// where the rows are. An offset here is a real place in the file and clicking
// one goes there, which is the nearest thing to the object list a template
// cannot build from bytes that are not in the file.

import type { GoTo } from "./quantpanel.js";
import type { TypeInfo, XrefRow } from "./doc.js";
import { countText } from "./strings.js";

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** How the table was packed, for the note beside the heading. */
export function xrefNote(info: TypeInfo): string {
  if (info.xref_widths.length !== 3) return "";
  return `${info.xref_widths.join("-")} bytes per row`;
}

/** How many rows there are, and how they split between the three kinds. Kinds
 *  with none are left out: a table of nothing but in-file rows should say so
 *  in four words, not list two zeroes. */
function tally(info: TypeInfo): string {
  const parts: string[] = [];
  if (info.xref_in_file > 0) parts.push(`${info.xref_in_file.toLocaleString()} in the file`);
  if (info.xref_in_stream > 0) parts.push(`${info.xref_in_stream.toLocaleString()} in object streams`);
  if (info.xref_free > 0) parts.push(`${info.xref_free.toLocaleString()} free`);
  const rows = countText(info.xref_total, "row");
  return parts.length === 0 ? rows : `${rows}: ${parts.join(", ")}`;
}

/** What one row says about where its object is. */
function where(r: XrefRow): string {
  if (r.kind === "in file") return r.offset.toLocaleString();
  if (r.kind === "in an object stream") return `In object ${r.second.toLocaleString()}, item ${r.third}`;
  if (r.kind === "free") return "Not in the file";
  return `Row type ${r.second}`;
}

/** The generation, where it is not the zero almost every row carries. A file
 *  that has replaced an object writes a number here, and so does the head of
 *  the free list, so it is worth showing exactly when it is unusual. */
function generation(r: XrefRow): string {
  if (r.kind === "in an object stream" || r.third === 0) return "";
  return `generation ${r.third.toLocaleString()}`;
}

/** One row, with the offset clickable where the row names a place in the file. */
function rowLine(r: XrefRow, goTo: GoTo): HTMLElement {
  const line = document.createElement("div");
  line.className = "insp-xrow";
  line.append(span("insp-xrow-object", r.object.toLocaleString()));
  if (r.offset >= 0) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "insp-xrow-where is-link";
    b.textContent = where(r);
    b.title = "Go to this object";
    // Nothing is marked: how long the object is is not known from the row,
    // and the structure panel says what is there once the cursor arrives.
    b.addEventListener("click", () => goTo(r.offset * 8, []));
    line.append(b);
  } else {
    line.append(span("insp-xrow-where", where(r)));
  }
  line.append(span("insp-xrow-note", generation(r)));
  return line;
}

/**
 * The whole panel: what the stream said, then the rows it held.
 *
 * A stream that would not open still gets a panel. What the dictionary asked
 * for and the reason it could not be done are together how a reader tells an
 * odd file from a gap in this program.
 */
export function xrefBody(info: TypeInfo, goTo: GoTo): DocumentFragment {
  const frag = document.createDocumentFragment();

  const sizes = document.createElement("div");
  sizes.className = "insp-qcount";
  const packed = `${info.xref_packed.toLocaleString()} bytes compressed`;
  const unpacked = info.xref_decoded > 0 ? `, ${info.xref_decoded.toLocaleString()} unpacked` : "";
  const pred = info.xref_predictor >= 0 ? `, PNG predictor ${info.xref_predictor}` : "";
  sizes.textContent = packed + unpacked + pred;
  frag.append(sizes);

  if (info.problem !== "") {
    const p = document.createElement("div");
    p.className = "insp-xproblem";
    p.textContent = `No rows: ${info.problem}.`;
    frag.append(p);
    return frag;
  }

  frag.append(span("insp-qsubhead", tally(info)));

  const head = document.createElement("div");
  head.className = "insp-xrow is-head";
  head.append(span("insp-xrow-object", "Object"), span("insp-xrow-where", "Where it is"), span("insp-xrow-note", ""));
  frag.append(head);

  const list = document.createElement("div");
  list.className = "insp-xrows";
  for (const r of info.xref_rows) list.append(rowLine(r, goTo));
  frag.append(list);

  if (info.xref_rows.length < info.xref_total) {
    frag.append(
      span(
        "insp-qcount",
        `Showing the first ${info.xref_rows.length.toLocaleString()} of ${info.xref_total.toLocaleString()}.`,
      ),
    );
  }
  return frag;
}

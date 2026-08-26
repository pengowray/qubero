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

/** How wide a row is, for the note beside the heading. The plain fact first,
 *  since that is what a reader counting bytes in the hex view wants, and the
 *  dictionary key after it, since that is what they would search for. */
export function xrefNote(info: TypeInfo): string {
  if (info.xref_widths.length !== 3) return "";
  const total = info.xref_widths.reduce((a, b) => a + b, 0);
  return `${total}-byte rows (/W [${info.xref_widths.join(" ")}])`;
}

/** How many rows there are, and how they split between the three kinds. Kinds
 *  with none are left out: a table of nothing but plain offsets should say so
 *  in four words, not list two zeroes.
 *
 *  Not "in the file" for the first of them. An object inside an object stream
 *  is in the file too; what tells the three apart is that one names an offset,
 *  one names another object, and one names nowhere. */
function tally(info: TypeInfo): string {
  const parts: string[] = [];
  const n = (x: number) => x.toLocaleString();
  if (info.xref_in_file > 0) parts.push(`${n(info.xref_in_file)} at offsets`);
  if (info.xref_in_stream > 0) {
    const one = info.xref_in_stream === 1;
    parts.push(`${n(info.xref_in_stream)} in ${one ? "an object stream" : "object streams"}`);
  }
  if (info.xref_free > 0) parts.push(`${n(info.xref_free)} free`);
  const rows = countText(info.xref_total, "row");
  return parts.length === 0 ? rows : `${rows}: ${parts.join(", ")}`;
}

/** What one row says about where its object is. `Free` rather than a
 *  description of it, so that the cell and the tally above use the one word
 *  for the one thing. */
function where(r: XrefRow): string {
  if (r.kind === "in file") return r.offset.toLocaleString();
  if (r.kind === "in an object stream") return `In object stream ${r.second.toLocaleString()}, index ${r.third}`;
  if (r.kind === "free") return "Free";
  return `Unknown type ${r.second}`;
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
  const unpacked = info.xref_decoded > 0 ? `, ${info.xref_decoded.toLocaleString()} decompressed` : "";
  const pred = info.xref_predictor >= 0 ? `, PNG predictor ${info.xref_predictor}` : "";
  sizes.textContent = packed + unpacked + pred;
  frag.append(sizes);

  // Not "no rows": for all but one of these the rows are there and were not
  // decompressed, and saying the table is empty would be saying something
  // about the file that is not true.
  if (info.problem !== "") {
    const p = document.createElement("div");
    p.className = "insp-xproblem";
    p.textContent = info.problem;
    frag.append(p);
    return frag;
  }

  frag.append(span("insp-qsubhead", tally(info)));

  const head = document.createElement("div");
  head.className = "insp-xrow is-head";
  head.append(span("insp-xrow-object", "Object"), span("insp-xrow-where", "Location"), span("insp-xrow-note", ""));
  frag.append(head);

  const list = document.createElement("div");
  list.className = "insp-xrows";
  for (const r of info.xref_rows) list.append(rowLine(r, goTo));
  frag.append(list);

  if (info.xref_rows.length < info.xref_total) {
    frag.append(
      span(
        "insp-qcount",
        `Showing the first ${info.xref_rows.length.toLocaleString()} of ${info.xref_total.toLocaleString()} rows.`,
      ),
    );
  }
  return frag;
}

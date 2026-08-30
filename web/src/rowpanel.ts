// A SQLite row that was too big for its page, put back together and read.
//
// SQLite keeps what fits where the row is and puts the rest on a chain of
// pages elsewhere in the file. The hex view can only show the part that
// stayed, so this is where the row's columns are. A column may begin on one
// page and end on another, so there is nowhere single in the file to go to.

import type { SqliteColumn, TypeInfo } from "./doc.js";
import { countText } from "./strings.js";

/** How many pages of numbers are worth printing instead of counting. */
const PAGES_LISTED = 12;

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** How many columns the row has, for the note beside the heading. */
export function rowNote(info: TypeInfo): string {
  return countText(info.row_total_columns, "column");
}

/** One column: which one it is, what SQLite calls it, and its value. */
function columnLine(column: SqliteColumn, index: number): HTMLElement {
  const line = document.createElement("div");
  line.className = "insp-crow";
  line.append(span("insp-crow-index", String(index)));
  line.append(span("insp-crow-type", column.type));
  line.append(span("insp-crow-value", column.value));
  return line;
}

/** The page numbers, listed while there are few enough to be worth reading. */
function pagesLine(info: TypeInfo): HTMLElement | null {
  if (info.row_chain === 0 || info.row_chain > PAGES_LISTED) return null;
  const numbers = info.row_pages.map((p) => p.toLocaleString()).join(", ");
  return span("insp-qcount", `Continues on ${countText(info.row_chain, "page")}: ${numbers}.`);
}

/**
 * The whole panel: how big the row is and where it went, then its columns.
 *
 * A row whose chain broke still gets a panel. What the cell claimed and how far
 * the walk got are together how a reader tells a damaged file from a gap in
 * this program, which is the same reason the other unpacked views keep theirs.
 */
export function rowBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();

  const sizes = document.createElement("div");
  sizes.className = "insp-qcount";
  const elsewhere = info.row_found - info.row_on_page;
  sizes.textContent =
    `${countText(info.row_declared, "byte")}: ` +
    `${info.row_on_page.toLocaleString()} on this page, ` +
    `${elsewhere.toLocaleString()} on ${countText(info.row_chain, "page")} elsewhere.`;
  frag.append(sizes);

  const pages = pagesLine(info);
  if (pages) frag.append(pages);

  if (info.problem !== "") {
    const p = document.createElement("div");
    p.className = "insp-xproblem";
    p.textContent = info.problem;
    frag.append(p);
  }

  if (info.row_columns.length === 0) return frag;

  // Not the uppercase subhead style: that is for two or three words, and a
  // sentence set in capitals is read a letter at a time.
  frag.append(span("insp-qcount", "These columns have no single file offset: one can cross a page break."));

  const head = document.createElement("div");
  head.className = "insp-crow is-head";
  head.append(span("insp-crow-index", "#"), span("insp-crow-type", "Type"), span("insp-crow-value", "Value"));
  frag.append(head);

  const list = document.createElement("div");
  list.className = "insp-orows";
  info.row_columns.forEach((column, i) => list.append(columnLine(column, i)));
  frag.append(list);

  if (info.row_columns.length < info.row_total_columns) {
    frag.append(
      span(
        "insp-qcount",
        `Showing the first ${info.row_columns.length.toLocaleString()} of ${info.row_total_columns.toLocaleString()} columns.`,
      ),
    );
  }
  return frag;
}

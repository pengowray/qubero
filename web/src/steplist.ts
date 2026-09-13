// The parts the unpacking panels share: a list of the steps something was
// taken back through, a line saying why it stopped, and a row of the first
// values that came out. The HDF5 chunk, Parquet page, FITS tile, GWF vector
// and GRIB section 7 panels are each the same three things around lines of
// their own, and were two copies of them before the tile panel made a third.

import { countText } from "./strings.ts";

export function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

export function line(cls: string, text: string): HTMLElement {
  const e = document.createElement("div");
  e.className = cls;
  e.textContent = text;
  return e;
}

/** One step as a panel lists it: its label, and what it did. */
export type StepRow = { readonly label: string; readonly text: string };

/** What a step that produced bytes did to the size: `223 → 880 bytes`. */
export function bytesChange(inBytes: number, outBytes: number): string {
  return `${inBytes.toLocaleString()} → ${outBytes.toLocaleString()} bytes`;
}

/**
 * The steps under a subhead, in a list that scrolls on its own when there are
 * many. `stacked` puts each label on a line of its own above what the step
 * did, for a few steps whose labels run past the narrow column beside the
 * text and whose notes are sentences: side by side in a sidebar, the two
 * columns leave the note a word a line.
 */
export function stepList(subhead: string, rows: readonly StepRow[], stacked = false): DocumentFragment {
  const frag = document.createDocumentFragment();
  if (rows.length === 0) return frag;
  frag.append(span("insp-qsubhead", subhead));
  const list = document.createElement("div");
  list.className = stacked ? "insp-orows is-stacked" : "insp-orows";
  for (const row of rows) {
    const e = document.createElement("div");
    e.className = "insp-orow";
    e.append(span("insp-orow-object", row.label), span("insp-orow-text", row.text));
    list.append(e);
  }
  frag.append(list);
  return frag;
}

/** Why the walk stopped, where it did. Nothing when it did not. */
export function problemLine(problem: string): DocumentFragment {
  const frag = document.createDocumentFragment();
  if (problem !== "") frag.append(line("insp-xproblem", problem));
  return frag;
}

/**
 * The first values under a subhead, and a line saying how many of how many
 * when that is not all of them. `noun` is plural: `elements`, `pixels`.
 */
export function firstValues(subhead: string, values: readonly string[], total: number, noun: string): DocumentFragment {
  const frag = valuesRow(subhead, values);
  if (values.length > 0 && values.length < total) {
    frag.append(span("insp-qcount", `Showing the first ${values.length.toLocaleString()} of ${total.toLocaleString()} ${noun}.`));
  }
  return frag;
}

/** The values under a subhead and nothing under them, for a panel whose line
 *  beneath has more to say than how many of how many. */
export function valuesRow(subhead: string, values: readonly string[]): DocumentFragment {
  const frag = document.createDocumentFragment();
  if (values.length === 0) return frag;
  frag.append(span("insp-qsubhead", subhead));
  // Across the whole row: without `is-values` the run of values sat in the
  // label column, a value or two a line.
  const row = document.createElement("div");
  row.className = "insp-orow is-values";
  row.append(span("insp-orow-text", values.join("  ")));
  frag.append(row);
  return frag;
}

/**
 * How many of the values a row shows, where it does not show all that came
 * out. `declared` is how many the file says there are, which is more than came
 * out when unpacking stopped early, and then the line counts what came out.
 * `noun` is singular. Null when the row shows every value that came out.
 */
export function showingLine(shown: number, total: number, declared: number, noun: string): string | null {
  if (shown >= total) return null;
  const first = `Showing the first ${shown.toLocaleString()} of`;
  if (total < declared) return `${first} the ${countText(total, noun)} unpacked.`;
  return `${first} ${countText(Math.max(total, declared), noun)}.`;
}

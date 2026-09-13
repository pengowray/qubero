// A BUFR message's section 4, read through the WMO tables. The hex view shows
// bits with no boundaries in them: where one value ends and the next begins is
// decided by the descriptors, the replication counts in the data and the
// operators in force, so none of it can be seen by looking. This is the walk
// laid out, cursor first: the value the cursor is on and the arithmetic that
// made it, then every value of the subset it belongs to, then the steps and
// the descriptors the walk followed.
//
// Values are written as the core writes them, to the decimal places their
// scale gives, so a value can be compared with ecCodes' by eye. Counts are
// grouped.

import type { BufrCursor, BufrInfo, BufrValue, TypeInfo } from "./doc.ts";
import { countText } from "./strings.ts";
import { line, problemLine, span, stepList } from "./steplist.ts";

/** A descriptor the way the tables write it: six digits, `012101`. */
export function code6(code: number): string {
  return String(code).padStart(6, "0");
}

/** How many subsets, and whether they are compressed, for the note beside
 *  the heading. */
export function bufrNote(info: TypeInfo): string {
  const b = info.bufr;
  if (b === null || b.edition === 0) return "";
  return `${countText(b.subsets, "subset")}${b.compressed ? ", compressed" : ""}`;
}

/** The edition and the tables version, and the version that was used where
 *  that is a different one. Null for a message whose header would not read. */
export function tablesLine(b: BufrInfo): string | null {
  if (b.edition === 0) return null;
  const named = `Edition ${b.edition}, WMO tables version ${b.master_table_version}`;
  if (b.tables_version === b.master_table_version) return named;
  return `${named} (not bundled: read with version ${b.tables_version})`;
}

/** A value as the list shows it: the value and its unit, or `missing`. */
export function valueText(v: BufrValue): string {
  if (v.missing) return "missing";
  return v.unit === "" ? v.text : `${v.text} ${v.unit}`;
}

/** What a value is, as the list labels it: the descriptor and its name. */
export function valueLabel(v: BufrValue): string {
  return v.name === "" ? code6(v.code) : `${code6(v.code)} ${v.name}`;
}

/** What a value is about, for the values a bitmap or an associated field
 *  attaches to another element. Null for a value that stands on its own. */
export function aboutText(v: BufrValue): string | null {
  if (v.about === "") return null;
  if (v.role === "associated") return `associated field of ${v.about}`;
  if (v.role === "marker") return `for ${v.about}`;
  if (v.role === "quality") return `for ${v.about}`;
  return null;
}

/** The subhead over the list of values: which subset, where there is more
 *  than one. */
export function subsetHead(b: BufrInfo): string {
  if (b.subsets <= 1) return "Values";
  return `Values of subset ${(b.subset + 1).toLocaleString()} of ${b.subsets.toLocaleString()}`;
}

/** Which of the subset's values are listed, where that is not all of them. */
export function showingLine(b: BufrInfo): string | null {
  if (b.values.length >= b.values_total) return null;
  const from = b.values_start + 1;
  const to = b.values_start + b.values.length;
  return `Showing values ${from.toLocaleString()} to ${to.toLocaleString()} of ${b.values_total.toLocaleString()}.`;
}

/**
 * How the value under the cursor was worked out, as one line: the packed
 * number, the reference and the scale, and where its bits are. For a
 * compressed message the packed number is the smallest of every subset plus
 * this subset's difference, and both are said. Null for text, which is its
 * bytes and nothing else.
 */
export function arithmeticLine(c: BufrCursor, compressed: boolean): string | null {
  const where = `${countText(c.width, "bit")} at bit ${c.bit.toLocaleString()} of section 4`;
  if (!c.numeric) return `${where}.`;
  if (c.packed === null) return `${where}: all ones, which means missing.`;
  const scaled = c.scale === 0 ? "" : ` ÷ 10^${c.scale}`;
  const packed =
    compressed && c.base !== null ? `(${c.base} + ${c.packed - c.base}` : `(${c.packed}`;
  const reference = c.reference < 0 ? ` − ${-c.reference}` : ` + ${c.reference}`;
  return `${packed}${reference})${scaled} = ${c.value.text}, from ${where}.`;
}

/** For a compressed value: the smallest packed number and the difference
 *  width every subset shares. Null for an uncompressed one. */
export function compressionLine(c: BufrCursor): string | null {
  if (c.increment_width === null) return null;
  if (c.increment_width === 0) return "Every subset has the same value: no differences are written.";
  const unit = c.numeric ? "bit" : "character";
  return `Compressed: the smallest value${c.base === null ? "" : ` ${c.base}`}, then a difference of ${countText(c.increment_width, unit)} for each subset.`;
}

function valueRow(v: BufrValue, here: boolean): HTMLElement {
  const row = document.createElement("div");
  row.className = here ? "insp-brow is-here" : "insp-brow";
  const name = document.createElement("span");
  name.className = "insp-brow-name";
  name.append(valueLabel(v));
  const about = aboutText(v);
  if (about !== null) name.append(span("insp-brow-about", ` ${about}`));
  const value = span(v.missing ? "insp-brow-value is-missing" : "insp-brow-value", valueText(v));
  row.append(name, value);
  return row;
}

function cursorSection(b: BufrInfo, c: BufrCursor): DocumentFragment {
  const frag = document.createDocumentFragment();
  frag.append(line("insp-qsubhead","At the cursor"));
  const list = document.createElement("div");
  list.className = "insp-brows";
  list.append(valueRow(c.value, false));
  frag.append(list);
  for (const text of [compressionLine(c), arithmeticLine(c, b.compressed)]) {
    if (text !== null) frag.append(line("insp-qcount", text));
  }
  if (c.across.length > 1) {
    frag.append(line("insp-qsubhead","In each subset"));
    const row = document.createElement("div");
    row.className = "insp-orow is-values";
    row.append(span("insp-orow-text", c.across.map((v) => (v === "" ? "missing" : v)).join("  ")));
    frag.append(row);
    if (c.across.length < b.subsets) {
      frag.append(line("insp-qcount",`Showing the first ${c.across.length.toLocaleString()} of ${countText(b.subsets, "subset")}.`));
    }
  }
  return frag;
}

/**
 * The whole panel: the edition and tables, why the reading stopped if it did,
 * the value at the cursor, the subset's values, the steps, and the
 * descriptors.
 *
 * A message that stopped partway still gets its panel: the values before the
 * descriptor that stopped it are right, and which descriptor that was is how a
 * reader tells a centre's local table from a damaged file.
 */
export function bufrBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  const b = info.bufr;
  if (b === null) return frag;
  const tables = tablesLine(b);
  if (tables !== null) frag.append(line("insp-qcount", tables));
  frag.append(problemLine(info.problem));

  if (b.cursor !== null) frag.append(cursorSection(b, b.cursor));

  if (b.values.length > 0) {
    frag.append(line("insp-qsubhead",subsetHead(b)));
    const list = document.createElement("div");
    list.className = "insp-brows";
    b.values.forEach((v, i) => list.append(valueRow(v, b.cursor !== null && b.cursor.index === i)));
    frag.append(list);
    const showing = showingLine(b);
    if (showing !== null) frag.append(line("insp-qcount",showing));
  }

  frag.append(stepList("Steps", b.steps.map((s, i) => ({ label: String(i + 1), text: s }))));

  if (b.descriptors.length > 0) {
    frag.append(line("insp-qsubhead","Descriptors, expanded through Table D"));
    const list = document.createElement("div");
    list.className = "insp-brows";
    for (const d of b.descriptors) {
      const row = document.createElement("div");
      row.className = "insp-brow";
      const name = span("insp-brow-name", d.name === "" ? code6(d.code) : `${code6(d.code)} ${d.name}`);
      name.style.paddingLeft = `${d.depth}em`;
      row.append(name);
      list.append(row);
    }
    frag.append(list);
    if (b.descriptors.length < b.descriptors_total) {
      frag.append(
        line("insp-qcount",`Showing the first ${b.descriptors.length.toLocaleString()} of ${countText(b.descriptors_total, "descriptor")}.`),
      );
    }
  }
  return frag;
}

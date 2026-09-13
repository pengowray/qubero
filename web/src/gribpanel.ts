// A GRIB2 message's section 7, worked out into the values it stands for. The
// tree shows the packed integers where they are, but what one is worth takes
// section 5's reference value and two scale factors, and under complex packing
// a group's reference, and under spatial differencing every value before it.
// None of that is in the field. This is where it is: which packing, the value
// under the cursor with the arithmetic that made it, the first values, and
// every step in the order it was done.
//
// Values, the packed integers and R are written as the core writes them,
// without digit grouping, so they can be compared by eye; so are the listing's
// own indices. Counts and ordinals are grouped.

import type { GribInfo, GribValue } from "./doc.ts";
import { countText, ordinal } from "./strings.ts";
import { line, problemLine, showingLine, stepList, valuesRow } from "./steplist.ts";

/** How many values, for the note beside the heading. A section that gave fewer
 *  or more than section 5 declares says both. */
export function gribNote(info: GribInfo): string {
  if (info.total === info.declared) return countText(info.declared, "value");
  return `${info.total.toLocaleString()} of ${countText(info.declared, "value")}`;
}

/** Which data representation template packed the values. */
export function packingLine(info: GribInfo): string {
  const head = `Data representation template 5.${info.template}`;
  if (info.template === 0) return `${head}: simple packing`;
  if (info.template === 2) return `${head}: complex packing`;
  return `${head}: complex packing with ${orderWord(info)}-order spatial differencing`;
}

function orderWord(info: GribInfo): string {
  return info.spatial_order === 1 ? "first" : "second";
}

/**
 * What the value under the cursor is: the field, then how its packed integer
 * X relates to what the field holds, then the formula with the numbers in it.
 * Empty where the cursor is not on one value.
 *
 * The field is named as the listing names it, because only simple packing has
 * a `values[i]` to match the message's own count; a complex value is the
 * `j`th of its group, and the count through the message is said separately.
 */
export function cursorLines(info: GribInfo): string[] {
  const at = info.at;
  if (at === null) return [];
  const worth = `decodes to ${at.value}.`;
  const formula = `${at.value} = (${info.reference} + ${at.packed} × 2^${info.binary_scale}) / 10^${info.decimal_scale}`;
  return [...placeLines(info, at, worth), formula];
}

function placeLines(info: GribInfo, at: GribValue, worth: string): string[] {
  const of = countText(info.declared, "value");
  const nth = ordinal(at.index + 1);
  if (at.place === "values") {
    return [`values[${at.index}], the ${nth} of ${of}, ${worth}`, `The packed integer X is ${at.packed}, as written in the field.`];
  }
  if (at.place === "first") {
    const why =
      info.spatial_order === 1
        ? "the first value is written as a value, not a difference, and has no group reference."
        : `the first ${info.spatial_order} values are written as values, not differences, and have no group reference.`;
    return [
      `first_values[${at.index}], the ${nth} of the message's ${of}, ${worth}`,
      `The packed integer X is ${at.packed}, as written in the field: ${why}`,
    ];
  }
  const field = `groups[${at.group}].values[${at.position}], the ${nth} of the message's ${of}, ${worth}`;
  if (info.template === 3) {
    const min = info.minimum ?? 0;
    return [
      field,
      `The field's ${at.written} plus its group's reference plus the overall minimum ${min} is a ${orderWord(info)}-order difference, not a value.`,
      `With the spatial differencing undone, the packed integer X is ${at.packed}.`,
    ];
  }
  return [field, `With its group's reference added, the packed integer X is ${at.packed}; the field's ${at.written} is only the offset above that reference.`];
}

/** The subhead over the values, which says `First` only when some are left
 *  out. */
export function valuesHead(info: GribInfo): string {
  return info.values.length >= info.total ? "Values" : "First values";
}

/**
 * The whole panel: the packing and the size, the value under the cursor, why
 * it stopped if it did, the first values, and then the steps.
 *
 * The value under the cursor comes before the steps because it is what the
 * reader came for, and the steps run to nine sentences, the way the miniSEED
 * panel puts its samples before how they were decoded.
 */
export function gribBody(info: GribInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  frag.append(line("insp-qcount", packingLine(info)), line("insp-qcount", `${countText(info.packed, "byte")} in the file`));
  for (const text of cursorLines(info)) frag.append(line("insp-qcount", text));
  frag.append(problemLine(info.problem));
  frag.append(valuesRow(valuesHead(info), info.values));
  const showing = showingLine(info.values.length, info.total, info.declared, "value");
  if (showing !== null) frag.append(line("insp-qcount", showing));
  const rows = info.steps.map((s) => ({ label: s.label, text: s.what }));
  frag.append(stepList("Steps, in the order they were done", rows, true));
  return frag;
}

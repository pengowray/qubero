// A GWF frame vector, unpacked. A vector whose `compress` field names zero
// suppression, or differences then gzip, stores packed bits or the
// differences between its values, so what the hex view shows there is not the
// values. This is the walk back: what the `compress` number means, each step
// with what went in and what came out, and the first values at the end of it.
// Nothing here can be clicked through to, because none of these values are in
// the file as they stand.
//
// Values are written as the core writes them, without digit grouping, so they
// can be compared by eye with the numbers in another channel. Counts are
// grouped.

import type { VectorInfo, VectorStep } from "./doc.ts";
import { countText } from "./strings.ts";
import { bytesChange, line, problemLine, showingLine, stepList, valuesRow } from "./steplist.ts";

/** How many values the vector holds, for the note beside the heading. A vector
 *  that unpacked fewer than `nData` says both, since the shortfall is the news. */
export function vectorNote(info: VectorInfo): string {
  if (info.total === info.declared) return countText(info.declared, "value");
  return `${info.total.toLocaleString()} of ${countText(info.declared, "value")}`;
}

/** What the `compress` number says, led by the number as the tree shows it so
 *  the two cannot come apart. Only differences then gzip (3) and zero
 *  suppression (5, 8, 10) reach this panel; plain gzip opens as a space of its
 *  own. The rest are here so a number the template sends by mistake still
 *  reads as what it is. */
export function schemeLine(info: VectorInfo): string {
  const order = info.little_endian ? "little-endian" : "big-endian";
  const scheme = info.compress & 0xff;
  const head = `compress = ${info.compress}`;
  if (scheme === 0) return `${head}: not compressed, ${order}`;
  if (scheme === 1) return `${head}: gzip, packed ${order}`;
  if (scheme === 3) return `${head}: gzip, with differencing, packed ${order}`;
  const words = scheme === 5 ? 2 : scheme === 8 ? 4 : scheme === 10 ? 8 : 0;
  if (words > 0) return `${head}: zero suppression of ${words}-byte words, with differencing, packed ${order}`;
  return `${head}: scheme ${scheme}, packed ${order}`;
}

/** The bytes in the file, and what they came to where anything came out. */
export function sizeLine(info: VectorInfo): string {
  const packed = `${countText(info.packed, "byte")} in the file`;
  return info.decoded > 0 ? `${packed}, ${countText(info.decoded, "byte")} unpacked` : packed;
}

/** What one step did. Differencing and the pairing of complex parts leave the
 *  size as it was, so those say what they did as well as the bytes. */
export function vectorStepText(step: VectorStep): string {
  const bytes = bytesChange(step.in_bytes, step.out_bytes);
  if (step.what === "differencing") return `${bytes}, running sum of the differences`;
  if (step.what === "interleave") {
    return `${bytes}, the run of real parts and the run of imaginary parts interleaved into (real, imaginary) pairs`;
  }
  return bytes;
}

/** The subhead over the values, which says `First` only when some are left
 *  out. */
export function valuesHead(info: VectorInfo): string {
  const all = info.values.length >= info.total;
  return `${all ? "Values" : "First values"}, as ${info.element_type}`;
}

/**
 * The whole panel: what the `compress` number means, the sizes, the steps in
 * the order they were done, why it stopped if it did, and the first values.
 *
 * A vector that would not unpack still gets a panel: the steps done before the
 * one that stopped it are how a reader tells an unusual file from a gap in this
 * program.
 */
export function vectorBody(info: VectorInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  frag.append(line("insp-qcount", schemeLine(info)), line("insp-qcount", sizeLine(info)));
  const rows = info.steps.map((s) => ({ label: s.what, text: vectorStepText(s) }));
  frag.append(stepList("Steps, in the order they were done", rows, true));
  frag.append(problemLine(info.problem));
  frag.append(valuesRow(valuesHead(info), info.values));
  const showing = showingLine(info.values.length, info.total, info.declared, "value");
  if (showing !== null) frag.append(line("insp-qcount", showing));
  return frag;
}

// Reading a value that does not sit on byte boundaries: where its bits are, and
// the shift-and-mask that lifts them out. Both the value panel's own section
// and a weight inside a quantised block show this, so it is written once here.

import { bitFormula, formatLength, formatOffset } from "./doc.js";
import { BYTE_NOTE } from "./strings.js";

/** Where a run of bits starts and how far it runs, both in the editor's own
 *  notation: `0x27ba64d3, len: 0+4b`. */
export function whereLine(bit: number, width: number): HTMLElement {
  const e = document.createElement("div");
  e.className = "insp-formula-where";
  const at = document.createElement("span");
  at.className = "addr";
  at.textContent = formatOffset(bit);
  e.append(at, `, len: ${formatLength(width)}`);
  return e;
}

/** The place and the expression together, in that order: what is being read
 *  before how to read it. */
export function extraction(bit: number, width: number): DocumentFragment {
  const frag = document.createDocumentFragment();
  const code = document.createElement("code");
  code.className = "insp-formula-code";
  code.textContent = bitFormula(bit, width);
  code.title = BYTE_NOTE;
  frag.append(whereLine(bit, width), code);
  return frag;
}

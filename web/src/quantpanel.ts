// A block of packed weights, taken apart. A quantised tensor is not a run of
// numbers: it is a run of blocks, each holding a scale and then thirty-two or
// two hundred and fifty-six weights of four, five or six bits, in an order that
// is not the order they are read in. The hex view can only show the bytes, so
// this is where the numbers are.

import { bitFormula } from "./doc.js";
import type { QuantWeight, TypeInfo } from "./doc.js";
import { BYTE_NOTE } from "./strings.js";

/** Asked for when a weight is clicked, so the views go to its bits. */
export type GoTo = (bitOffset: number) => void;

/** Whether the grid shows the stored integers or what they scale to. Kept
 *  across renders, because it is a way of reading rather than a fact about the
 *  block under the cursor. */
let showValues = false;

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

/**
 * A weight, short enough to line up in a grid. Four significant digits is more
 * than a scale stored as a half float can justify, and the exponent form keeps
 * the very small ones from reading as zero.
 */
function num(x: number): string {
  if (!Number.isFinite(x)) return String(x);
  if (x === 0) return "0";
  const size = Math.abs(x);
  if (size >= 1e5 || size < 1e-4) return x.toExponential(2);
  return String(Number(x.toPrecision(4)));
}

/** `d 0.003876` and whatever the layout pairs with it. */
function scaleRow(info: TypeInfo): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-qscales";
  row.append(span("insp-qscale-name", "d"), span("insp-qscale-value", num(info.scale)));
  if (info.second_name !== "") {
    row.append(span("insp-qscale-name", info.second_name), span("insp-qscale-value", num(info.second)));
  }
  return row;
}

/**
 * The weight the cursor is standing on, both ways round, and how to lift its
 * stored integer out of the file.
 *
 * The formula is only for a weight that keeps all of its bits in one run. A
 * five- or six-bit type puts the top bits in a separate byte, and a formula
 * that quietly left them out would be worse than none.
 */
function cursorRow(w: QuantWeight, index: number, info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  const row = document.createElement("div");
  row.className = "insp-qcursor";
  row.append(
    span("insp-qcursor-index", `#${index}`),
    span("insp-qcursor-note", `stored ${w.q}`),
    span("insp-qcursor-value", `scaled ${num(w.value)}`),
  );
  frag.append(row);
  if (w.width === info.width) {
    const code = document.createElement("code");
    code.className = "insp-formula-code";
    code.textContent = bitFormula(info.block_bits + w.bit, w.width);
    code.title = BYTE_NOTE;
    frag.append(code);
  }
  return frag;
}

/** Stored, or what it scales to: the same weights read two ways. */
function toggle(onPick: () => void): HTMLElement {
  const seg = document.createElement("div");
  seg.className = "seg insp-qseg";
  seg.setAttribute("role", "radiogroup");
  seg.setAttribute("aria-label", "Show each weight as");
  for (const [values, label] of [
    [false, "Stored"],
    [true, "Scaled"],
  ] as const) {
    const b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "radio");
    b.setAttribute("aria-checked", String(values === showValues));
    b.textContent = label;
    b.addEventListener("click", () => {
      showValues = values;
      onPick();
    });
    seg.append(b);
  }
  return seg;
}

/**
 * Every weight in the block, in the order the tensor reads them, which is not
 * the order they are written in: a `q4_0` block holds weight 0 in the low half
 * of its first byte and weight 16 in the high half of the same byte. Clicking
 * one goes to the bits it came from.
 */
function grid(info: TypeInfo, goTo: GoTo): HTMLElement {
  const g = document.createElement("div");
  g.className = showValues ? "insp-qgrid is-wide" : "insp-qgrid";
  info.weights.forEach((w, i) => {
    const cell = document.createElement("button");
    cell.type = "button";
    cell.className = "insp-qcell";
    if (i === info.at) cell.classList.add("is-here");
    cell.textContent = showValues ? num(w.value) : String(w.q);
    cell.title = `#${i} · stored ${w.q} · scaled ${num(w.value)}`;
    cell.addEventListener("click", () => goTo(info.block_bits + w.bit));
    g.append(cell);
  });
  return g;
}

/**
 * The whole section under the value editor for a block of packed weights.
 * `redraw` is asked for when the reader switches how the grid reads, since the
 * panel is rebuilt rather than updated in place.
 */
export function quantBody(info: TypeInfo, goTo: GoTo, redraw: () => void): DocumentFragment {
  const frag = document.createDocumentFragment();
  const here = info.at >= 0 ? info.weights[info.at] : undefined;
  if (here !== undefined) frag.append(cursorRow(here, info.at, info));
  frag.append(scaleRow(info));
  const head = document.createElement("div");
  head.className = "insp-qhead";
  head.append(span("insp-qcount", `${info.weights.length} weights, ${info.width} bits each`), toggle(redraw));
  frag.append(head, grid(info, goTo));
  return frag;
}

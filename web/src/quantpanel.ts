// A block of packed weights, taken apart. A quantised tensor is not a run of
// numbers: it is a run of blocks, each holding a scale and then thirty-two or
// two hundred and fifty-six weights of four, five or six bits, in an order that
// is not the order they are read in. The hex view can only show the bytes, so
// this is where the numbers are.

import { extraction } from "./bitextract.js";
import type { BitRange } from "./hexview.js";
import type { QuantPart, QuantWeight, TypeInfo } from "./doc.js";
import { countText } from "./strings.js";

/** Asked for when a weight is clicked, so the views go to its bits and mark
 *  them. A weight the layout keeps in two places gives two runs. */
export type GoTo = (bitOffset: number, ranges: readonly BitRange[]) => void;

/** Whether the grid shows the stored integers or what they scale to. Kept
 *  across renders, because it is a way of reading rather than a fact about the
 *  block under the cursor. */
let showValues = false;

/** Where the grid had got to, and which block it was showing. The panel is
 *  rebuilt on every cursor move, more than once for some of them, so this is
 *  kept here rather than read back off a grid that may already have been
 *  replaced. */
let scrolled = { block: -1, top: 0 };

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

/** The weight the cursor is standing on, both ways round. */
function cursorRow(w: QuantWeight, index: number): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-qcursor";
  row.append(
    span("insp-qcursor-index", `#${index}`),
    span("insp-qcursor-note", `stored ${w.q}`),
    span("insp-qcursor-value", `scaled ${num(w.value)}`),
  );
  return row;
}

/**
 * The scale each run of weights keeps for itself. A K type spends twelve or
 * sixteen bytes on these, six bits apiece and split across bytes, so the field
 * they live in reads as nothing but hex until they are taken apart. The block's
 * own `d` is what they are measured in: a weight is `d * scale * stored`, less
 * `dmin * min` where the type has one.
 */
function groupRow(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  if (info.groups.length === 0) return frag;
  const per = info.group_weights;
  const head = document.createElement("div");
  head.className = "insp-qhead";
  // Where the type has a minimum, every cell carries two numbers, and a
  // heading saying only "scales" would have the reader take both for scales.
  const mins = info.groups[0]?.min !== null;
  head.append(
    span("insp-qsubhead", mins ? "Scales and mins" : "Scales"),
    span("insp-qcount", `${countText(info.groups.length, "group")} of ${countText(per, "weight")}`),
  );
  const g = document.createElement("div");
  g.className = "insp-qgroups";
  const at = info.at >= 0 ? Math.floor(info.at / per) : -1;
  info.groups.forEach((group, i) => {
    const cell = document.createElement("span");
    cell.className = "insp-qgroup";
    if (i === at) cell.classList.add("is-here");
    cell.append(span("insp-qgroup-scale", String(group.scale)));
    if (group.min !== null) cell.append(span("insp-qgroup-min", String(group.min)));
    const covers = `weights ${i * per} to ${(i + 1) * per - 1}`;
    cell.title = group.min === null ? `${covers} · scale ${group.scale}` : `${covers} · scale ${group.scale} · min ${group.min}`;
    g.append(cell);
  });
  frag.append(head, g);
  return frag;
}

/** Every run of bits the weight is made of, low part first, as places in the
 *  file rather than in the block. */
function runs(info: TypeInfo, w: QuantWeight): BitRange[] {
  const parts = w.high === null ? [w.bits] : [w.bits, w.high];
  return parts.map((p) => ({ startBit: info.block_bits + p.bit, endBit: info.block_bits + p.bit + p.width }));
}

/** A number inside a product, bracketed where its sign would otherwise run
 *  into the operator before it. */
function term(x: number): string {
  const t = num(x);
  return t.startsWith("-") ? `(${t})` : t;
}

/**
 * How the stored integer at the cursor becomes the number the model reads,
 * named first and then worked out.
 *
 * The two levels of scaling are the thing to see here: a K type multiplies the
 * block's own scale by the group's, and takes away the block's minimum through
 * the group's. Nothing else on the panel says how the numbers above it are
 * connected.
 */
function recipe(info: TypeInfo, w: QuantWeight, index: number): DocumentFragment {
  const frag = document.createDocumentFragment();
  const group = info.groups.length > 0 ? info.groups[Math.floor(index / info.group_weights)] : undefined;
  if (info.groups.length > 0 && group === undefined) return frag;
  const names = ["d"];
  const values = [info.scale];
  if (group !== undefined) {
    names.push("scale");
    values.push(group.scale);
  }
  names.push("stored");
  values.push(w.q);
  let named = names.join(" × ");
  let worked = values.map(term).join(" × ");
  if (info.second_name !== "") {
    const sign = info.second_subtract ? " − " : " + ";
    const min = info.second_per_group ? group?.min ?? null : null;
    named += sign + (min === null ? info.second_name : `${info.second_name} × min`);
    // The whole of what is taken away goes in one bracket, so that a negative
    // `dmin` reads as a number being subtracted rather than as two signs.
    worked += sign + (min === null ? term(info.second) : `(${num(info.second)} × ${num(min)})`);
  }
  frag.append(
    span("insp-qrecipe-named", named),
    span("insp-qrecipe", `${worked} = ${num(w.value)}`),
  );
  return frag;
}

/**
 * Where each run of the weight's bits is, and how to lift it out.
 *
 * A five-bit weight is four bits of `qs` and one bit of `qh` a dozen bytes
 * away, and the nibble on its own does not show where the fifth came from. Each
 * run is named by the field it is in, so the two lines can be matched to the
 * fields listed above.
 */
function sources(info: TypeInfo, w: QuantWeight): DocumentFragment {
  const frag = document.createDocumentFragment();
  const parts = w.high === null ? [w.bits] : [w.bits, w.high];
  // One run needs no naming: the field is the one the cursor is already in.
  const named = parts.length > 1;
  for (const p of parts) {
    frag.append(extraction(info.block_bits + p.bit, p.width, named ? p.field : undefined));
  }
  return frag;
}

/**
 * How the runs add up to the packed value, and what is taken off it to get the
 * stored one. Only shown where there is something to say: a single unbiased run
 * is its own answer.
 */
function packing(info: TypeInfo, w: QuantWeight): HTMLElement | null {
  if (w.high === null && info.bias === 0 && !info.signed) return null;
  let text = w.bits.field;
  if (w.high !== null) text = `${w.high.field} << ${w.high.shift} | ${text}`;
  // The bracket is only needed once there is more than one term to bind.
  if (info.bias !== 0) text = w.high === null ? `${text} − ${info.bias}` : `(${text}) − ${info.bias}`;
  if (info.signed) text = `${text} read signed`;
  const e = document.createElement("div");
  e.className = "insp-qrecipe-named";
  e.textContent = `stored = ${text}`;
  return e;
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
  // A grid that started at the top on every rebuild would take the weight just
  // clicked off the screen. Another block starts at the top, since nothing has
  // been read there yet.
  const wasAt = scrolled.block === info.block_bits ? scrolled.top : 0;
  const g = document.createElement("div");
  g.className = showValues ? "insp-qgrid is-wide" : "insp-qgrid";
  info.weights.forEach((w, i) => {
    const cell = document.createElement("button");
    cell.type = "button";
    cell.className = "insp-qcell";
    if (i === info.at) cell.classList.add("is-here");
    cell.textContent = showValues ? num(w.value) : String(w.q);
    cell.title = `#${i} · stored ${w.q} · scaled ${num(w.value)}`;
    cell.addEventListener("click", () => goTo(info.block_bits + w.bits.bit, runs(info, w)));
    g.append(cell);
  });
  const remember = (): void => {
    if (g.isConnected) scrolled = { block: info.block_bits, top: g.scrollTop };
  };
  g.addEventListener("scroll", remember);
  // The panel is built and put in place within one task, so by the time this
  // runs the grid is in the document and can be measured. A grid built for a
  // render that was replaced before it got there is left alone: setting the
  // scroll of a detached element would only record a zero.
  queueMicrotask(() => {
    if (!g.isConnected) return;
    g.scrollTop = wasAt;
    const cell = g.querySelector(".insp-qcell.is-here");
    if (cell instanceof HTMLElement) {
      // Just far enough to bring the weight under the cursor into view, so a
      // grid the reader has already put where they want it stays there.
      const c = cell.getBoundingClientRect();
      const r = g.getBoundingClientRect();
      if (c.top < r.top) g.scrollTop += c.top - r.top;
      else if (c.bottom > r.bottom) g.scrollTop += c.bottom - r.bottom;
    }
    remember();
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
  if (here !== undefined) {
    // What it is, then why that number, then how it is packed and where each
    // run of its bits sits.
    frag.append(cursorRow(here, info.at), recipe(info, here, info.at));
    const how = packing(info, here);
    if (how !== null) frag.append(how);
    frag.append(sources(info, here));
  }
  frag.append(scaleRow(info), groupRow(info));
  const head = document.createElement("div");
  head.className = "insp-qhead";
  head.append(span("insp-qcount", `${info.weights.length} weights, ${info.width} bits each`), toggle(redraw));
  frag.append(head, grid(info, goTo));
  return frag;
}

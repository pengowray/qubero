// Bit-packing ribbon for any field stored least significant bit first.
//
// Candidate `bit-pack-ribbon` in docs/DESIGN-diagrams.md, section 4.2; built
// by agent A of the first batch (section 5.3). An inspector section, which
// took over from the deflate-only figure `inspector.ts` used to draw under a
// code's digits.
//
// A format that packs its fields from the low end of each byte (deflate, GIF's
// LZW, FLAC, JPEG XL's headers) writes a field's first bit into bit 0 of the
// byte, the rightmost digit when the byte is written out. So the bits of a
// field come out of the hex view in an order that is not the order of its
// value, and a field that crosses into the next byte takes the low bits of
// that byte, at the far end of it. Along the top this draws every byte the
// field touches as the hex view shows it, high bit first, with the bits of
// other fields faint; along the bottom, the field as the number it is read
// as; and a band from each stored bit to its place. Bands cross where the
// packing mirrors the order. A deflate code is two or four parts, each read
// its own way (`bitorder.ts` has the arithmetic, and says why).
//
// Explored by pointing: a band, or either of its ends, lights the bit at both
// ends, says which digit it is and where it is stored, and marks it in the hex
// view through the shared hover (`linked.ts`). A click on any bit, a faint one
// included, goes to that bit in the file, which for a faint one is the
// neighbouring field. The left and right arrow keys step to the previous and
// next field, so a reader can walk a deflate block code by code with the
// figure open. A toggle redraws the bottom row read the wrong way, high bit
// first straight off the hex, beside the right number.
//
// No zoom: the figure is at most 64 bits across and is drawn at a fixed pitch
// no wider than the panel, so the wheel scrolls the panel as usual.

import "./bits.css";
import { bitOrderLayout, partValues, readOrderDigits, type BitOrderLayout, type BitPart, type BitReading } from "../bitorder.ts";
import type { Doc, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { ribbon } from "../report/ribbon.ts";
import { VIZ } from "../strings.ts";
import { leadingInteger } from "./bitlayout.ts";
import { digitName, packRibbon, PART_COLORS, type PackInput } from "./bitpacklayout.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { linkedFor } from "./linked.ts";
import { goToBits, goToField } from "./pick.ts";
import { hideTip, tipAt, tipCard } from "./tip.ts";

const T = VIZ.bitPack;

/** The widest field drawn. A ribbon of more bits than this has bands too
 *  thin to follow, and no format packs a single value that wide. */
const MAX_BITS = 64;
/** The most a digit of the figure is drawn across, in pixels. */
const MAX_DIGIT_PX = 16;
/** How tall a bit box is drawn, which is also the height of what a pointer
 *  aims at: the boxes of a row touch, so every point along it is one. */
const BOX_PX = 24;

/** Whether a field is a code of a deflate stream: its parent counts codes. */
function isCode(doc: Doc, n: TemplateNode): boolean {
  if (n.composite || n.path.length === 0) return false;
  const up = doc.templateNode(n.path.slice(0, -1));
  return up.status === "ok" && up.node.unit === "code";
}

/** Whether a field packed low bit first touches more than one byte, which is
 *  when its order against the hex is worth a figure. A field inside one byte
 *  keeps the byte's order and is the bit ruler's to draw (`bitruler.ts`). */
function crossesBytes(n: TemplateNode): boolean {
  return Math.floor(n.offset_bits / 8) !== Math.floor((n.offset_bits + n.size_bits - 1) / 8);
}

/** The parts of a deflate code under the cursor, from the decoder's own
 *  reading of it (`Doc.decodedCode`), or null where the stream above it is
 *  not one the decoder can step through. */
function codeParts(doc: Doc, n: TemplateNode): { parts: BitPart[]; colors: string[] } | null {
  for (let i = n.path.length - 1; i >= 1; i--) {
    const at = n.path.slice(0, i);
    const up = doc.templateNode(at);
    if (up.status !== "ok" || !up.node.decoded) continue;
    const found = doc.decodedCode(at, up.node.offset_bits, n.offset_bits);
    if (found === null) return null;
    const parts: BitPart[] = [];
    const colors: string[] = [];
    // A match is two codes, each with its own extra bits, and the panel's
    // Decoded section calls them the length and the distance; so does this.
    const match = found.step.distance !== undefined;
    const add = (c: { readonly code_bits: number; readonly extra_bits: number }, code: string, extra: string, distance: boolean): void => {
      parts.push({ bits: c.code_bits, lowFirst: false, label: code });
      colors.push(distance ? PART_COLORS.distanceCode : PART_COLORS.code);
      if (c.extra_bits > 0) {
        parts.push({ bits: c.extra_bits, lowFirst: true, label: extra });
        colors.push(distance ? PART_COLORS.distanceExtra : PART_COLORS.extra);
      }
    };
    add(found.step.symbol, match ? T.lengthCode : T.huffman, match ? T.lengthExtra : T.extra, false);
    if (found.step.distance !== undefined) add(found.step.distance, T.distanceCode, T.distanceExtra, true);
    return { parts, colors };
  }
  return null;
}

export type BitPackData = PackInput & { readonly node: TemplateNode };

/** Codes whose parts did not add up to their digits, by document, so a
 *  figure that can never be drawn is not worked out again on every render. */
const UNDRAWABLE = new WeakMap<Doc, Set<string>>();

/** What the figure needs for `n`, or null: while its bytes are on the way,
 *  and for a code whose parts cannot be matched to its bits. */
export function bitPackData(doc: Doc, n: TemplateNode): BitPackData | null {
  const count = n.size_bits;
  if (count === 0 || count > MAX_BITS) return null;
  const key = `${n.path.join("/")}|${n.offset_bits}`;
  if (UNDRAWABLE.get(doc)?.has(key) === true) return null;
  const code = isCode(doc, n);
  const found = code ? codeParts(doc, n) : { parts: [{ bits: count, lowFirst: true, label: n.name }], colors: [PART_COLORS.extra] };
  if (found === null) return null;
  // A band drawn to the wrong digit is worse than none.
  if (found.parts.reduce((t, p) => t + p.bits, 0) !== count) {
    if (!UNDRAWABLE.has(doc)) UNDRAWABLE.set(doc, new Set());
    UNDRAWABLE.get(doc)?.add(key);
    return null;
  }
  const keep = found.parts.map((p) => p.bits > 0);
  const parts = found.parts.filter((_, i) => keep[i]);
  const colors = found.colors.filter((_, i) => keep[i]);
  const first = Math.floor(n.offset_bits / 8);
  const last = Math.floor((n.offset_bits + count - 1) / 8);
  const read = doc.read(first, last - first + 1);
  if (!read.complete) return null;
  const digits = code && n.edit_text.length === count ? n.edit_text : readOrderDigits(read.bytes, n.offset_bits, count);
  return { node: n, parts, colors, digits, bytes: read.bytes, code };
}

/** Whether the comparison with the wrong reading is on. Kept across fields,
 *  so a reader stepping through codes keeps the reading they chose. */
let comparing = false;

export type BitPack = { readonly el: HTMLElement; destroy(): void };

/** Draw the figure no wider than `width` pixels. */
export function drawBitPack(doc: Doc, data: BitPackData, opts: { readonly width: number }): BitPack {
  const n = data.node;
  const count = n.size_bits;
  const fig = document.createElement("figure");
  fig.className = "vb-fig vb-pack";
  fig.tabIndex = 0;
  // Said to assistive technology only: a title on the figure would put the
  // browser's own tooltip over the hover card.
  fig.setAttribute("aria-keyshortcuts", "ArrowLeft ArrowRight");
  const head = document.createElement("figcaption");
  head.className = "vb-head";
  const title = document.createElement("span");
  title.className = "vb-title";
  title.textContent = T.title(n.name);
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "vb-toggle";
  head.append(title, toggle);
  const body = document.createElement("div");
  const note = document.createElement("div");
  note.className = "vb-note";
  note.textContent = data.code ? T.caption : T.captionNumber;
  fig.append(head, body, note);

  const bus = linkedFor(doc);
  /** The bit of the file a top-row slot is, in the hex view's numbering. */
  let layout: BitOrderLayout = bitOrderLayout(n.offset_bits, count, data.parts);
  const fileBit = (slot: number): number => layout.firstByte * 8 + slot;

  const render = (): void => {
    const reading: BitReading = comparing ? "hex" : "format";
    layout = bitOrderLayout(n.offset_bits, count, data.parts, reading);
    // The label says what a press does, so it changes with the state and is
    // not also marked pressed.
    toggle.textContent = comparing ? T.compareOff : T.compare;
    toggle.title = comparing ? "" : T.compareTitle;
    const rib = packRibbon(data, layout);
    const units = rib.top.to;
    const width = Math.max(80, Math.min(opts.width, units * MAX_DIGIT_PX));
    const svg = ribbon(rib, {
      width,
      box: BOX_PX,
      gap: 34,
      charWidth: 5,
      strand: 0.3,
      captions: false,
      onHover: (band, e) => hover(band, e),
    });
    svg.setAttribute("aria-label", T.label);
    svg.style.width = `${width}px`;
    svg.style.maxWidth = "100%";
    svg.classList.add("vb-svg");
    const line = (text: string): HTMLElement => {
      const d = document.createElement("div");
      d.className = "vb-row";
      d.textContent = text;
      return d;
    };
    body.replaceChildren(line(rib.top.caption), svg, line(comparing ? T.readHex : rib.bottom.caption), legend());
    svg.addEventListener("click", click);
  };

  /** One entry per part, in the part's colour, with what a number reads as:
   *  the right value, and beside it the wrong one while comparing. */
  const legend = (): HTMLElement => {
    const box = document.createElement("div");
    box.className = "vb-legend";
    const right = partValues(bitOrderLayout(n.offset_bits, count, data.parts), data.digits, data.parts);
    const shown = partValues(layout, data.digits, data.parts);
    const seen = new Set<string>();
    data.parts.forEach((p, i) => {
      // A Huffman code is a path through a tree, not a number; the legend
      // names it and says nothing it would have to make up.
      const r = String(right[i] ?? 0);
      // A field the format adjusts after reading (deflate's HCLEN is stored
      // four short) reads as a number its bits do not make; say both, so the
      // legend and the panel's value do not seem to disagree.
      const said = data.code ? null : leadingInteger(n.value);
      const own = said !== null && String(said) !== r ? T.partStored(p.label, r, n.value) : T.partValue(p.label, r);
      const text = !p.lowFirst ? p.label : comparing ? T.partCompare(p.label, r, String(shown[i] ?? 0)) : own;
      if (seen.has(text)) return;
      seen.add(text);
      const sw = document.createElement("span");
      sw.className = "vb-swatch";
      sw.style.background = data.colors[i] ?? PART_COLORS.code;
      const item = document.createElement("span");
      item.append(sw, text);
      box.append(item);
    });
    return box;
  };

  const hover = (band: number | null, e: PointerEvent): void => {
    if (band === null) {
      // A faint bit along the top has no band: it is another field's.
      const slot = Number((e.target as SVGElement | null)?.dataset?.top ?? Number.NaN);
      if (!Number.isFinite(slot) || layout.top.includes(slot)) {
        hideTip();
        bus.publish({ hover: null });
        return;
      }
      const bit = fileBit(slot);
      const digit = ((data.bytes[Math.floor(slot / 8)] ?? 0) >> (7 - (slot % 8))) & 1;
      const other = neighbour(slot);
      const head = other === null ? T.otherBit : T.otherBitOf(other);
      tipAt(tipCard(head, [T.bitValue(digit), T.storedAs(7 - (slot % 8)), T.goToOther], formatOffset(Math.floor(bit / 8) * 8)), e.clientX, e.clientY);
      bus.publish({ hover: { startBit: bit, endBit: bit + 1 } });
      return;
    }
    const slot = layout.top[band] ?? 0;
    const bit = fileBit(slot);
    const lines = [T.bitValue(data.digits[band] === "1" ? 1 : 0), T.storedAs(7 - (slot % 8))];
    tipAt(tipCard(digitName(layout, data.parts, band, n.name), lines, formatOffset(Math.floor(bit / 8) * 8)), e.clientX, e.clientY);
    bus.publish({ hover: { startBit: bit, endBit: bit + 1 } });
  };

  /** The name of the field a faint bit of the top row belongs to, found the
   *  first time the pointer rests on it. `Doc.locate` counts bits as the hex
   *  view does, high bit first, which is how the top row counts them too. */
  const names = new Map<number, string | null>();
  const neighbour = (slot: number): string | null => {
    const known = names.get(slot);
    if (known !== undefined) return known;
    const path = doc.locate(fileBit(slot));
    const node = path.status === "ok" ? doc.templateNode(path.node) : null;
    const name = node !== null && node.status === "ok" && !samePath(node.node.path, n.path) ? node.node.name : null;
    names.set(slot, name);
    return name;
  };

  /** A click on a bit, either end of a band or the band, goes to that bit. */
  const click = (e: MouseEvent): void => {
    const d = (e.target as SVGElement | null)?.dataset;
    if (d === undefined) return;
    let slot: number | undefined;
    if (d.top !== undefined) slot = Number(d.top);
    else if (d.bottom !== undefined) slot = layout.top[Number(d.bottom)];
    else if (d.band !== undefined) slot = layout.top[Number(d.band)];
    if (slot === undefined || !Number.isFinite(slot)) return;
    const bit = fileBit(slot);
    goToBits(doc, [{ startBit: bit, endBit: bit + 1 }]);
  };

  toggle.addEventListener("click", (e) => {
    e.stopPropagation();
    comparing = !comparing;
    render();
  });
  /** The arrow keys step to the field before or after this one in its
   *  structure: the previous or next code of a deflate block. */
  const key = (e: KeyboardEvent): void => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    const last = n.path[n.path.length - 1];
    if (last === undefined) return;
    const next = last + (e.key === "ArrowLeft" ? -1 : 1);
    if (next < 0) return;
    const sib = doc.templateNode([...n.path.slice(0, -1), next]);
    if (sib.status !== "ok") return;
    e.preventDefault();
    const s = sib.node;
    goToField(doc, { path: s.path, startBit: s.offset_bits, endBit: s.offset_bits + s.size_bits });
  };
  fig.addEventListener("keydown", key);
  const leave = (): void => {
    hideTip();
    bus.publish({ hover: null });
  };
  fig.addEventListener("pointerleave", leave);
  // A press is not a hover, and a click on a bit moves the cursor, which
  // draws the figure again under the pointer: the card goes now, not when
  // the pointer next moves.
  fig.addEventListener("pointerdown", hideTip);

  render();
  return {
    el: fig,
    destroy: () => {
      hideTip();
      fig.removeEventListener("keydown", key);
      fig.removeEventListener("pointerleave", leave);
      fig.removeEventListener("pointerdown", hideTip);
    },
  };
}

/** Put the ribbon in the At cursor panel for every code of a deflate stream
 *  and every field packed low bit first that crosses a byte boundary. */
export function register(): void {
  registerInspectorSection({
    id: "bit-pack",
    applies: (doc, n) =>
      // A code is read low bit first too, so most fields are turned away
      // on the flag before the parent is asked whether it counts codes.
      !n.composite && n.lsb_first && n.space === 0 && n.size_bits >= 2 && n.size_bits <= MAX_BITS && (crossesBytes(n) || isCode(doc, n)),
    key: (_doc, n, path) => `${path.join("/")}|${n.offset_bits}|${n.size_bits}|${n.edit_text}`,
    draw: ({ doc, node, width }) => {
      const data = bitPackData(doc, node);
      return data === null ? null : drawBitPack(doc, data, { width }).el;
    },
  });
}

function samePath(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((s, i) => s === b[i]);
}

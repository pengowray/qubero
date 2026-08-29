// The bytes behind one item, laid out under it.
//
// A column per field: an address wherever the bytes are not a continuation of
// the column before, the hex digits, and the field's name beneath them in the
// field's hue. The digits are deliberately dim: their job here is how many
// bytes there are and where they sit. What a field means lives in the item's
// own rows, in the same order; the hue is the link between the two, so the
// strip never restates a value the row already carries.
//
// The fields come from `Doc.spans`, which answers "what covers these bytes"
// including the stretches nothing covers. That is the same question the hex
// view's annotation column asks.

import { formatOffset } from "./doc.js";
import type { Doc, Span } from "./doc.js";
import { byteDump } from "./bytedump.js";
import { fieldHue } from "./fieldstyle.js";
import { REPORT } from "./strings.js";

/** Fields asked for in one strip. Past this the strip is not the thing to be
 *  reading, and the item's own rows are. */
const MAX_FIELDS = 24;
/** Bytes of one field shown before the run is cut short. A strip is for
 *  seeing where the fields are, not for reading a kilobyte of payload. */
const BYTES_SHOWN = 12;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** A field's bytes as hex, cut short when the field runs longer than a strip
 *  should show, with how many bytes are not drawn. Bits when it does not fill
 *  bytes. */
function digits(doc: Doc, span: Span): { readonly text: string; readonly rest: number } {
  const whole = span.offset_bits % 8 === 0 && span.size_bits % 8 === 0;
  if (!whole && span.size_bits <= BYTES_SHOWN * 8) {
    const { bytes, complete } = doc.readBits(span.offset_bits, span.size_bits);
    if (!complete) return { text: "", rest: 0 };
    let bits = "";
    for (let i = 0; i < span.size_bits; i++) bits += ((bytes[i >> 3] ?? 0) >> (7 - (i % 8))) & 1 ? "1" : "0";
    return { text: bits, rest: 0 };
  }
  const at = Math.floor(span.offset_bits / 8);
  const len = Math.ceil(((span.offset_bits % 8) + span.size_bits) / 8);
  const take = Math.min(len, BYTES_SHOWN);
  const { bytes, complete } = doc.read(at, take);
  if (!complete) return { text: "", rest: 0 };
  const hex = Array.from(bytes.subarray(0, take), (b) => b.toString(16).padStart(2, "0")).join(" ");
  return { text: hex, rest: len - take };
}

/**
 * The strip: a caption the caller supplies, and the fields' bytes.
 *
 * `map` is the file map for this stretch, made by the caller so that every
 * strip's is the same picture as every heading's.
 */
export function byteStrip(
  doc: Doc,
  offsetBits: number,
  sizeBits: number,
  caption: string,
  map: HTMLElement,
  onClose: () => void,
  /** The field the reader has selected elsewhere, so its column here is the
   *  same field rather than a coincidence of position. */
  selected?: { readonly offsetBits: number; readonly sizeBits: number } | null,
  /** Which fields have been opened out into a dump of all their bytes, by the
   *  bit they start at, and where the reader has each one scrolled to. The
   *  strip is redrawn from scratch on every change, so neither can live in
   *  it. */
  dumps?: {
    readonly open: ReadonlySet<number>;
    readonly toggle: (offsetBits: number) => void;
    readonly scroll: (offsetBits: number) => { get: () => number; set: (top: number) => void };
  },
): HTMLElement {
  const strip = el("div", "bstrip");
  const cap = el("div", "bs-cap");
  cap.append(el("span", "bs-cap-text", caption), map);
  const close = el("button", "bs-close", REPORT.hideBytes);
  close.type = "button";
  close.addEventListener("click", (e) => {
    e.stopPropagation();
    onClose();
  });
  cap.append(close);
  strip.append(cap);

  const reply = doc.spans(offsetBits, offsetBits + sizeBits, MAX_FIELDS);
  if (reply.status !== "ok") {
    strip.append(el("div", "bs-wait", REPORT.reading));
    return strip;
  }
  const spans = reply.node.filter((s) => s.size_bits > 0);
  const fields = el("div", "bs-fields");
  /** One past the last bit the previous column drew, and whether it stopped
   *  short of its field's end. A column that does not pick up exactly where
   *  the drawn bytes left off says its own address; the rest stay quiet, so
   *  the addresses mark exactly the places where left-to-right stops being
   *  the truth. */
  let prevEnd = -1;
  let prevCut = false;
  spans.forEach((span, i) => {
    // A stretch nothing covers takes no hue: there is no field for it to be
    // the colour of.
    const hue = span.gap ? null : fieldHue(i);
    const isOn = selected !== undefined && selected !== null
      && selected.offsetBits === span.offset_bits && selected.sizeBits === span.size_bits;
    const column = el("div", `bs-fld${span.gap ? " is-gap" : ""}${isOn ? " is-on" : ""}`);
    if (hue !== null) column.style.setProperty("--hue", hue);
    const { text, rest } = digits(doc, span);
    const jump = prevCut || span.offset_bits !== prevEnd;
    // The slot is there either way so every column's rows line up.
    column.append(el("span", "bs-at", jump ? formatOffset(span.offset_bits) : ""));
    const hex = el("span", "bs-by", text);
    if (rest > 0 && dumps !== undefined) {
      // The count is the way in to the rest of them: the reader has just been
      // told bytes exist that are not here, and this is where they are.
      const more = el("button", "bs-more", REPORT.bytesCut(rest));
      more.type = "button";
      const shown = dumps.open.has(span.offset_bits);
      const label = shown ? REPORT.bytesCutClose : REPORT.bytesCutOpen(span.size_bits);
      more.title = label;
      more.setAttribute("aria-label", label);
      more.setAttribute("aria-expanded", String(shown));
      more.addEventListener("click", (e) => {
        e.stopPropagation();
        dumps.toggle(span.offset_bits);
      });
      hex.append(" ", more);
    } else if (rest > 0) {
      hex.append(` ${REPORT.bytesCut(rest)}`);
    }
    column.append(hex);
    // The label is the field's name whether or not its bytes fitted, whole:
    // with no chip to carry the full name, the label is the only place it is
    // said, and a name clipped to its bytes' width reads as a different name.
    // The hue bar sits under the hex, not under the label, so what marks the
    // field's extent is still exactly its bytes. A stretch no field covers is
    // named for what it is, not for the structure it happens to sit inside.
    column.append(el("span", "bs-lb", span.gap ? REPORT.gap : span.name));
    fields.append(column);
    prevEnd = span.offset_bits + span.size_bits;
    prevCut = rest > 0;
  });
  strip.append(fields);
  // Every field the reader has opened out, under the map that named it, in the
  // order the strip drew them.
  if (dumps !== undefined) {
    for (const span of spans) {
      if (!dumps.open.has(span.offset_bits)) continue;
      if (span.offset_bits % 8 !== 0 || span.size_bits % 8 !== 0) continue;
      strip.append(byteDump(doc, span.offset_bits / 8, span.size_bits / 8, span.gap ? REPORT.gap : span.name, dumps.scroll(span.offset_bits)));
    }
  }
  return strip;
}

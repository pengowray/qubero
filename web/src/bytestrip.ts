// The bytes behind one item, laid out under it.
//
// A column per field: its hex digits on top, a coloured bracket beneath them
// clipped to exactly the field's width, and a chip below carrying what the
// field says. The digits are deliberately dim. Their job here is how many
// bytes there are and where they sit; the value is in the chip, in the same
// hue, in the same order, so the eye goes from a stretch of bytes to its
// meaning without a legend in between.
//
// The fields come from `Doc.spans`, which answers "what covers these bytes"
// including the stretches nothing covers. That is the same question the hex
// view's annotation column asks, so the chips are built on `chipfit`'s
// vocabulary rather than a third one.

import { formatOffset } from "./doc.js";
import type { Doc, Span } from "./doc.js";
import { chipDetail } from "./chipfit.js";
import { fieldHue } from "./fieldstyle.js";
import { bitSizeText, REPORT } from "./strings.js";

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

/** A field's bytes as hex, cut short with the mockup's own word when the field
 *  runs longer than a strip should show. Bits when it does not fill bytes. */
function digits(doc: Doc, span: Span): { readonly text: string; readonly cut: boolean } {
  const whole = span.offset_bits % 8 === 0 && span.size_bits % 8 === 0;
  if (!whole && span.size_bits <= BYTES_SHOWN * 8) {
    const { bytes, complete } = doc.readBits(span.offset_bits, span.size_bits);
    if (!complete) return { text: "", cut: false };
    let bits = "";
    for (let i = 0; i < span.size_bits; i++) bits += ((bytes[i >> 3] ?? 0) >> (7 - (i % 8))) & 1 ? "1" : "0";
    return { text: bits, cut: false };
  }
  const at = Math.floor(span.offset_bits / 8);
  const len = Math.ceil(((span.offset_bits % 8) + span.size_bits) / 8);
  const take = Math.min(len, BYTES_SHOWN);
  const { bytes, complete } = doc.read(at, take);
  if (!complete) return { text: "", cut: false };
  const hex = Array.from(bytes.subarray(0, take), (b) => b.toString(16).padStart(2, "0")).join(" ");
  return { text: hex, cut: len > take };
}

/** What the chip says about a field, beyond its name. `chipDetail` is the hex
 *  view's answer to the same question, so a field reads the same in both. */
function chipValue(span: Span): string {
  const detail = chipDetail(span);
  return detail === "" ? span.value : detail;
}

/**
 * The strip: a caption the caller supplies, the fields' bytes, and a chip each.
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
  const grid = el("div", "bs-grid");
  grid.append(el("span", "bs-addr", formatOffset(offsetBits)));
  const fields = el("div", "bs-fields");
  const chips = el("div", "bs-chips");
  spans.forEach((span, i) => {
    // A stretch nothing covers takes no hue: there is no field for it to be
    // the colour of, and tinting it would promise a chip that is not there.
    const hue = span.gap ? null : fieldHue(i);
    const column = el("div", `bs-fld${span.gap ? " is-gap" : ""}`);
    if (hue !== null) column.style.setProperty("--hue", hue);
    const { text, cut } = digits(doc, span);
    column.append(el("span", "bs-by", cut ? `${text} â€¦` : text));
    column.append(el("span", "bs-lb", cut ? REPORT.moreBytes : span.name));
    fields.append(column);
    if (hue === null) return;
    const chip = el("span", "bs-chip");
    chip.style.setProperty("--hue", hue);
    chip.append(el("span", "bs-k", span.name));
    const value = chipValue(span);
    if (value !== "") chip.append(el("span", "bs-v", value));
    chip.append(el("span", "bs-k", bitSizeText(span.size_bits)));
    chips.append(chip);
  });
  grid.append(fields);
  strip.append(grid);
  if (chips.childElementCount > 0) strip.append(chips);
  return strip;
}

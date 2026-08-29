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
import { byteDump } from "./bytedump.js";
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
  const grid = el("div", "bs-grid");
  grid.append(el("span", "bs-addr", formatOffset(offsetBits)));
  const fields = el("div", "bs-fields");
  const chips = el("div", "bs-chips");
  spans.forEach((span, i) => {
    // A stretch nothing covers takes no hue: there is no field for it to be
    // the colour of, and tinting it would promise a chip that is not there.
    const hue = span.gap ? null : fieldHue(i);
    const isOn = selected !== undefined && selected !== null
      && selected.offsetBits === span.offset_bits && selected.sizeBits === span.size_bits;
    const column = el("div", `bs-fld${span.gap ? " is-gap" : ""}${isOn ? " is-on" : ""}`);
    if (hue !== null) column.style.setProperty("--hue", hue);
    const { text, rest } = digits(doc, span);
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
    // The label is the field's name whether or not its bytes fitted. Putting
    // the cut mark here instead cost a 3,970-byte run of free space and a
    // 40-byte SQL string the same name, which was neither of theirs. A
    // stretch no field covers is named for what it is, not for the structure
    // it happens to sit inside.
    column.append(el("span", "bs-lb", span.gap ? REPORT.gap : span.name));
    fields.append(column);
    if (hue === null) return;
    const chip = el("span", `bs-chip${isOn ? " is-on" : ""}`);
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

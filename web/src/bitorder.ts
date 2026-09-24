// A deflate code's bits in the order they are stored against the order they
// are read, as a ribbon, for the panel at the cursor.
//
// Deflate fills each byte from its low bit, so the first bit of a code is bit
// 0 of its byte, the rightmost digit of the byte written out. A Huffman code
// is then read high bit first, the order the stream gives its bits, while the
// extra bits that follow it are a number stored low bit first. Written out as
// numbers, the Huffman code comes out mirrored against the byte and the extra
// bits come out in the byte's own order. That is what this draws: along the
// top, every byte the code touches written high bit first as a hex editor
// shows it, with the bits of other codes faint; along the bottom, each part of
// the code written as the number it is read as; and a band from each bit to
// where it lands.
//
// The arithmetic is `bitOrderLayout`, with no document and no DOM, so the
// tests can check it.

import type { Doc } from "./doc.ts";
import { formatOffset } from "./format.ts";
import { ribbon, type RibbonBox, type RibbonData } from "./report/ribbon.ts";

/** One part of a code in the order it is read: the Huffman code, or the
 *  extra bits after it. */
export type BitPart = {
  readonly bits: number;
  /** True for extra bits, which are a number stored low bit first. */
  readonly lowFirst: boolean;
  readonly label: string;
};

export type BitOrderLayout = {
  readonly firstByte: number;
  readonly byteCount: number;
  /** For each bit of the code, in the order it is read: its place along the
   *  top, counting the bytes' digits written high bit first. */
  readonly top: readonly number[];
  /** For each bit of the code: its place along the bottom, counting the
   *  parts' digits written as numbers, high digit first. */
  readonly bottom: readonly number[];
  /** Which part each bit of the code belongs to. */
  readonly partOf: readonly number[];
};

/**
 * Where each bit of a code sits in the two orders. `offsetBits` counts the
 * stream's own way, low bit of each byte first, which is how the core places
 * a trace's codes: bit `i` of the code is bit `(offsetBits + i) % 8` of byte
 * `(offsetBits + i) / 8`, counted from the low end.
 */
export function bitOrderLayout(offsetBits: number, count: number, parts: readonly BitPart[]): BitOrderLayout {
  const firstByte = Math.floor(offsetBits / 8);
  const lastByte = Math.floor((offsetBits + Math.max(1, count) - 1) / 8);
  const top: number[] = [];
  const bottom: number[] = [];
  const partOf: number[] = [];
  let partStart = 0;
  let inPart = 0;
  let part = 0;
  for (let i = 0; i < count; i++) {
    const bit = offsetBits + i;
    const byte = Math.floor(bit / 8);
    const low = bit % 8;
    top.push((byte - firstByte) * 8 + (7 - low));
    while (part < parts.length && inPart >= (parts[part]?.bits ?? 0)) {
      partStart += parts[part]?.bits ?? 0;
      inPart = 0;
      part++;
    }
    const p = parts[part];
    const width = p?.bits ?? count;
    bottom.push(partStart + (p?.lowFirst === true ? width - 1 - inPart : inPart));
    partOf.push(Math.min(part, Math.max(0, parts.length - 1)));
    inPart++;
  }
  return { firstByte, byteCount: lastByte - firstByte + 1, top, bottom, partOf };
}

/** Text for the figure, kept with it since only the panel shows it. */
const TEXT = {
  stored: (from: string, to: string): string => (from === to ? `As stored: byte ${from}, high bit first` : `As stored: bytes ${from} to ${to}, each high bit first`),
  read: "As read: each part as the number it is",
  caption: "Deflate takes each byte's bits from the low end. A Huffman code is read in that order, so its digits come out mirrored; extra bits are a number stored low bit first, so their digits keep the byte's order.",
  label: "How this code's bits are reordered between the bytes and the code",
};

/** Colours of the parts: Huffman codes and extra bits, told apart. */
const HUFFMAN = "#5b82cf";
const EXTRA = "#c47f22";

/** The figure for one code, or null where the parts do not account for its
 *  bits: a band drawn to the wrong digit is worse than none. */
export function bitOrderFigure(doc: Doc, offsetBits: number, bits: string, parts: readonly BitPart[]): HTMLElement | null {
  const count = bits.length;
  if (count === 0 || parts.reduce((n, p) => n + p.bits, 0) !== count) return null;
  const layout = bitOrderLayout(offsetBits, count, parts);
  const read = doc.read(layout.firstByte, layout.byteCount);
  if (!read.complete) return null;
  // A half-digit gap between bytes, and between parts, so each reads as one.
  const byteBox = (slot: number): { from: number; to: number } => {
    const byte = Math.floor(slot / 8);
    const at = byte * 8.5 + (slot % 8);
    return { from: at, to: at + 1 };
  };
  const inCode = new Map<number, number>();
  layout.top.forEach((slot, i) => inCode.set(slot, i));
  const topBoxes: RibbonBox[] = [];
  const topIndex = new Map<number, number>();
  for (let slot = 0; slot < layout.byteCount * 8; slot++) {
    const byte = read.bytes[Math.floor(slot / 8)] ?? 0;
    const digit = (byte >> (7 - (slot % 8))) & 1;
    const i = inCode.get(slot);
    const part = i === undefined ? undefined : parts[layout.partOf[i] ?? 0];
    topIndex.set(slot, topBoxes.length);
    topBoxes.push({ ...byteBox(slot), label: String(digit), color: part === undefined ? "var(--muted)" : part.lowFirst ? EXTRA : HUFFMAN, dim: i === undefined });
  }
  const bottomStart: number[] = [];
  let at = 0;
  for (const p of parts) {
    bottomStart.push(at);
    at += p.bits + 0.5;
  }
  const bottomBoxes: RibbonBox[] = [];
  const bottomIndex = new Map<number, number>();
  const slotsBottom: { slot: number; i: number }[] = layout.bottom.map((slot, i) => ({ slot, i })).sort((a, b) => a.slot - b.slot);
  for (const { slot, i } of slotsBottom) {
    const partNo = layout.partOf[i] ?? 0;
    const part = parts[partNo];
    const partBase = parts.slice(0, partNo).reduce((n, p) => n + p.bits, 0);
    const x = (bottomStart[partNo] ?? 0) + (slot - partBase);
    bottomIndex.set(i, bottomBoxes.length);
    bottomBoxes.push({ from: x, to: x + 1, label: bits[i] ?? "", color: part?.lowFirst === true ? EXTRA : HUFFMAN });
  }
  const topEnd = layout.byteCount * 8.5 - 0.5;
  const data: RibbonData = {
    top: {
      from: 0,
      to: topEnd,
      caption: TEXT.stored(formatOffset(layout.firstByte * 8), formatOffset((layout.firstByte + layout.byteCount - 1) * 8)),
      boxes: topBoxes,
    },
    bottom: { from: 0, to: Math.max(topEnd, at - 0.5), caption: TEXT.read, boxes: bottomBoxes },
    bands: layout.top.map((slot, i) => ({ top: topIndex.get(slot) ?? 0, bottom: bottomIndex.get(i) ?? 0 })),
  };
  // Scaled to the bytes: a digit is ten units wide at most, so a one-byte
  // code does not fill the panel.
  const unitsWide = Math.max(topEnd, at - 0.5);
  const width = Math.min(360, Math.max(120, unitsWide * 10));
  // The rows' captions are written above and below the figure, where the
  // panel can wrap them; the figure is only as wide as its bits.
  const fig = document.createElement("figure");
  fig.className = "bit-order";
  const svg = ribbon(data, { width, box: 14, gap: 34, charWidth: 5, strand: 0.3, captions: false });
  svg.setAttribute("aria-label", TEXT.label);
  svg.style.width = `${width}px`;
  svg.style.maxWidth = "100%";
  const legend = document.createElement("div");
  legend.className = "bit-order-legend";
  const names = [...new Set(parts.map((p) => p.label))];
  for (const name of names) {
    const p = parts.find((x) => x.label === name);
    const sw = document.createElement("span");
    sw.className = "bit-order-swatch";
    sw.style.background = p?.lowFirst === true ? EXTRA : HUFFMAN;
    const item = document.createElement("span");
    item.append(sw, name);
    legend.append(item);
  }
  const cap = document.createElement("figcaption");
  cap.textContent = TEXT.caption;
  const line = (text: string): HTMLElement => {
    const d = document.createElement("div");
    d.className = "bit-order-row";
    d.textContent = text;
    return d;
  };
  fig.append(line(data.top.caption), svg, line(data.bottom.caption), legend, cap);
  return fig;
}

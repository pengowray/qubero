// The bit-order ribbon's rows and bands, with no document and no DOM.
//
// `bitpack.ts` draws the figure; this says what goes in it, so a test can
// check it: one box per bit of every byte the field touches along the top,
// one box per bit of the field along the bottom in the order it is read, and
// a band from each stored bit to its place. Where each bit lands is
// `bitOrderLayout`'s arithmetic (`bitorder.ts`); this turns that into the
// boxes `report/ribbon.ts` draws, with the gaps between bytes and between
// parts and the colour of each part.

import type { BitOrderLayout, BitPart } from "../bitorder.ts";
import { formatOffset } from "../format.ts";
import type { RibbonBox, RibbonData } from "../report/ribbon.ts";
import { VIZ } from "../strings.ts";

const T = VIZ.bitPack;

/**
 * The colour of each kind of part (`bits.css`). A match code has four parts,
 * the length code and its extra bits and the distance code and its extra
 * bits, and each has its own hue so the legend can say which is which: blue
 * and orange for the length (or the one code of a literal), aqua and violet
 * for the distance. A field that is one number is orange, as extra bits are:
 * both are a number stored low bit first.
 */
export const PART_COLORS = {
  code: "var(--vb-code)",
  extra: "var(--vb-extra)",
  distanceCode: "var(--vb-dist)",
  distanceExtra: "var(--vb-dist-extra)",
} as const;

/** The same colour as a tint of the page, for a box with a digit on it: the
 *  digit is in the page's own colour and reads at 4.5:1 in either theme. */
export function partTint(color: string): string {
  return `color-mix(in srgb, ${color} 32%, var(--bg))`;
}

/** What the rows are drawn from. */
export type PackInput = {
  readonly parts: readonly BitPart[];
  /** A CSS colour per part. */
  readonly colors: readonly string[];
  /** The field's digits in the order it reads them. */
  readonly digits: string;
  /** The bytes it touches. */
  readonly bytes: Uint8Array;
  /** True for a deflate code, whose bottom row is several parts. */
  readonly code: boolean;
};

/** Where each part starts along the bottom row, in digits: each part after
 *  the first starts half a digit after the one before ends. */
export function partStarts(parts: readonly BitPart[]): { readonly starts: readonly number[]; readonly end: number } {
  const starts: number[] = [];
  let at = 0;
  for (const p of parts) {
    starts.push(at);
    at += p.bits + 0.5;
  }
  return { starts, end: Math.max(0, at - 0.5) };
}

/** The ribbon's two rows and the bands between them, for one reading. */
export function packRibbon(data: PackInput, layout: BitOrderLayout): RibbonData {
  const { parts, colors, digits, bytes } = data;
  const color = (p: number): string => colors[p] ?? PART_COLORS.code;
  // A half-digit gap between bytes, and between parts, so each reads as one.
  const byteBox = (slot: number): { from: number; to: number } => {
    const at = Math.floor(slot / 8) * 8.5 + (slot % 8);
    return { from: at, to: at + 1 };
  };
  const inField = new Map<number, number>();
  layout.top.forEach((slot, i) => inField.set(slot, i));
  const topBoxes: RibbonBox[] = [];
  for (let slot = 0; slot < layout.byteCount * 8; slot++) {
    const byte = bytes[Math.floor(slot / 8)] ?? 0;
    const digit = (byte >> (7 - (slot % 8))) & 1;
    const i = inField.get(slot);
    topBoxes.push({ ...byteBox(slot), label: String(digit), color: i === undefined ? "var(--muted)" : partTint(color(layout.partOf[i] ?? 0)), dim: i === undefined });
  }
  const { starts, end } = partStarts(parts);
  const base: number[] = [];
  let sum = 0;
  for (const p of parts) {
    base.push(sum);
    sum += p.bits;
  }
  // One bottom box per bit of the field, in the field's own read order, so
  // bottom box `i` and band `i` are both bit `i`.
  const bottomBoxes: RibbonBox[] = layout.bottom.map((slot, i) => {
    const p = layout.partOf[i] ?? 0;
    const x = (starts[p] ?? 0) + (slot - (base[p] ?? 0));
    return { from: x, to: x + 1, label: digits[i] ?? "", color: partTint(color(p)) };
  });
  const topEnd = layout.byteCount * 8.5 - 0.5;
  const from = formatOffset(layout.firstByte * 8);
  const to = formatOffset((layout.firstByte + layout.byteCount - 1) * 8);
  const width = Math.max(topEnd, end);
  return {
    top: { from: 0, to: width, caption: T.stored(from, to), boxes: topBoxes },
    bottom: { from: 0, to: width, caption: data.code ? T.read : T.readOne, boxes: bottomBoxes },
    bands: layout.top.map((slot, i) => ({ top: slot, bottom: i, color: color(layout.partOf[i] ?? 0) })),
  };
}

/** What the hover card on bit `band` of the field calls it: for a number,
 *  which bit of the number it is, counted from the lowest; for a Huffman
 *  code, which bit it is in the order the code is read. */
export function digitName(layout: BitOrderLayout, parts: readonly BitPart[], band: number, fallback: string): string {
  const p = layout.partOf[band] ?? 0;
  const part = parts[p];
  const base = parts.slice(0, p).reduce((t, q) => t + q.bits, 0);
  const fromLeft = (layout.bottom[band] ?? 0) - base;
  const width = part?.bits ?? 0;
  const label = part?.label ?? fallback;
  return part?.lowFirst === false ? T.codeBit(label, fromLeft + 1, width) : T.numberBit(label, width - 1 - fromLeft);
}

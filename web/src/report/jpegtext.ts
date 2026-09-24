// The words of the JPEG scan section (`jpeg.ts`), kept apart from `text.ts`
// only so the two can change without treading on each other; the rules are
// the same. Headings state a fact with its number, captions lead with their
// point, and T.81's own terms are used as they are (MCU, DC, AC, EOB, ZRL,
// zigzag), since a reader of a JPEG report looks for those words.

import { bytesText, counted, listText } from "./text.ts";


/** `+3`, `-5`, `0`. */
export function signed(n: number): string {
  return n > 0 ? `+${n}` : String(n);
}

/** `3 bits`, `1 bit`. Always bits: a code's width is never whole bytes. */
export function bitCount(n: number): string {
  return n === 1 ? "1 bit" : `${n.toLocaleString()} bits`;
}

/** `42%`, or `<1%` for a share that rounds to nothing but is not nothing. */
export function share(part: number, whole: number): string {
  if (whole <= 0 || part <= 0) return "0%";
  const p = (part / whole) * 100;
  return p < 1 ? "<1%" : `${Math.round(p)}%`;
}

export const JPEG = {
  heading: (bytes: number): string => `Where the scan's ${bytesText(bytes)} go in the picture`,
  headingOf: (channels: readonly string[], bytes: number): string =>
    `Where the ${listText(channels)} scan's ${bytesText(bytes)} go in the picture`,

  /** The lede of a scan that carries several channels. `parts` are "4 of Y". */
  ledeInterleaved: (w: number, h: number, mcus: number, across: number, down: number, mw: number, mh: number, blocks: number, parts: readonly string[]): string =>
    `The scan codes the ${w} × ${h} picture as ${counted(mcus, "MCU")}, ${across} across and ${down} down. ` +
    `Each MCU covers ${mw} × ${mh} pixels and holds ${blocks} blocks of 8 × 8 samples: ${listText(parts)}.`,
  ledePart: (n: number, channel: string): string => `${n} of ${channel}`,
  /** The lede of a scan that carries one channel, one block to an MCU. */
  ledeOne: (channel: string, w: number, h: number, blocks: number, across: number, down: number): string =>
    `The scan codes the ${channel} channel of the ${w} × ${h} picture as ${counted(blocks, "block")} of 8 × 8 samples, ` +
    `${across} across and ${down} down, one block to an MCU.`,
  ledeRestart: (n: number): string => (n === 1 ? " A restart marker follows every MCU." : ` A restart marker follows every ${n} MCUs.`),

  // ----- the map -----
  pictureLabel: "The picture",
  mapLabel: "Bits per MCU",
  mapAlt: (across: number, down: number): string => `A map of ${across} by ${down} MCUs, each shaded by the bits it takes`,
  /** A class of the legend: `0–99`. */
  legendClass: (from: number, to: number): string => `${from.toLocaleString()}–${to.toLocaleString()}`,
  captionLead: (x: number, y: number, max: number, median: number): string =>
    `MCU ${x}, ${y} costs the most: ${bitCount(max)}, against a median of ${median.toLocaleString()}.`,
  caption:
    "Each square is one MCU, shaded by the bits its codes take. An interval's padding and restart marker count in the MCU before them. " +
    "Hover an MCU for its numbers, and click it to trace its blocks below.",
  tipHead: (x: number, y: number): string => `MCU ${x}, ${y}`,
  tipBits: (bits: number, whole: number): string => `${bitCount(bits)}, ${share(bits, whole)} of the scan`,
  tipChannel: (channel: string, bits: number, blocks: number): string =>
    `${channel}: ${bitCount(bits)} in ${counted(blocks, "block")}`,
  coarse: (n: number): string =>
    `This scan has more codes than Qubero names one by one, so ${counted(n, "block")} past that limit are counted whole, and their codes are not shown.`,

  // ----- where the bits go -----
  channelsLead: (channel: string, bitShare: string, blockShare: string): string =>
    `${channel} takes ${bitShare} of the bits for ${blockShare} of the blocks.`,
  channelCol: "Channel",
  blocksCol: "Blocks",
  bitsCol: "Bits",
  shareCol: "Share of bits",
  perBlockCol: "Bits per block",
  kindsLead: (s: string): string => `AC coefficients take ${s} of the bits, their codes and value bits together.`,
  kindCol: "What the bits are",
  kind: {
    dc_code: "DC Huffman codes",
    dc_value: "DC value bits",
    ac_code: "AC Huffman codes",
    ac_value: "AC value bits",
    eob: "EOB codes",
    zrl: "ZRL codes",
    padding: "Padding to the end of a byte",
    stuffed: "Stuffed 00 bytes after each ff of data",
    markers: "Restart markers",
    unnamed: "Codes not named one by one",
  } as Readonly<Record<string, string>>,
  total: "Total",

  // ----- one block -----
  blockHeading: (channel: string, x: number, y: number, codes: number): string =>
    `${channel} block ${x}, ${y}, from its ${counted(codes, "code")} to its 64 samples`,
  blockWhere: (mx: number, my: number, median: boolean): string =>
    median ? `In MCU ${mx}, ${my}, the MCU of median cost.` : `In MCU ${mx}, ${my}.`,
  blockPick: "Other blocks of this MCU:",
  blockButton: (channel: string, x: number, y: number): string => `${channel} ${x}, ${y}`,
  blockButtonLabel: (channel: string, x: number, y: number): string => `${channel} block ${x}, ${y}`,
  inListing: "Show in the listing",
  unnamedBlock: "This block is past the limit on codes named one by one, so only its extent is known.",
  stripCaptionLead: (codes: number, bits: number): string => `${counted(codes, "code")} in ${bitCount(bits)}.`,
  stripCaption: "Each box is one code, as its bits: the Huffman code first, then its value bits in lighter type. Hover a box for what it says.",
  codeNo: "#",
  codeBits: "Bits",
  codeSays: "What it says",
  codeAt: "At",
  bitsTitle: (code: number, value: number): string =>
    value === 0 ? `${bitCount(code)} of Huffman code` : `${bitCount(code)} of Huffman code, then ${bitCount(value)} of value`,
  saysDc: (diff: number, dc: number): string => `DC difference ${signed(diff)}, so DC is ${dc}`,
  saysAc: (value: number, k: number, run: number): string =>
    `AC ${value} at zigzag position ${k}` + (run === 0 ? "" : run === 1 ? ", after 1 zero" : `, after ${run} zeros`),
  saysZrl: (k: number): string => `ZRL: 16 zeros, zigzag positions ${k} to ${k + 15}`,
  saysEob: (k: number): string => `EOB: zigzag positions ${k} to 63 are zero`,
  saysUnnamed: "Codes not named",
  stuffedNote: "a stuffed 00 byte sits inside these bits, and is left out of them",

  // ----- the grids -----
  zigzagLabel: "As coded, in zigzag order",
  rowsLabel: "Coefficients, in rows",
  stepsLabel: (id: number): string => `× Quantization steps, table ${id}`,
  dequantLabel: "= Dequantized",
  samplesLabel: "Inverse DCT, + 128 = Samples",
  gridsLead: (nonzero: number): string => `${nonzero} of the 64 coefficients are not zero.`,
  gridsCaption:
    "The scan codes a block in zigzag order, from the top left toward the bottom right, and the block is stored in rows. " +
    "Each coefficient is multiplied by its quantization step, and the inverse DCT turns the 64 results into 64 samples from 0 to 255. " +
    "Hover a cell for its row, column, and zigzag position.",
  noQuant: "No quantization table for this channel is defined before the scan, so the samples are not shown.",
  cellTip: (row: number, col: number, k: number, value: number): string => `Row ${row}, column ${col}, zigzag position ${k}: ${value}`,
};

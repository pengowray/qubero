// A tile of a FITS compressed image, decompressed. A compressed image is a
// binary table with a row per tile, and what the hex view shows under a row or
// in the heap is compressed bytes: Rice codes, a deflate stream, or integers a
// float image was quantized to. This is the walk back to pixels, for whichever
// tile the cursor is in: which tile it is and where it sits in the image, each
// step with what went in and what came out, and the first pixels. Nothing here
// can be clicked through to, because none of these pixels are in the file.
//
// Pixel values and the scale and zero point in the steps are written as the
// core writes them, without digit grouping, so a value can be compared by eye.
// Counts are grouped.

import type { PageStep, TileInfo } from "./doc.ts";
import { countText, ordinal } from "./strings.ts";
import { bytesChange, firstValues, line, problemLine, stepList } from "./steplist.ts";

/** How many pixels the tile has, for the note beside the heading. From the
 *  tile's shape, so it shows even when nothing was decompressed. */
export function tileNote(info: TileInfo): string {
  if (info.count === 0) return "";
  return countText(info.pixels, "pixel");
}

/** Which tile, and where it sits in the image. The index is the listing's
 *  own, counted from 0 as the rows are; the ordinal beside it counts from 1,
 *  as do the pixel coordinates. Null for an image whose header would not
 *  read. */
export function tileLine(info: TileInfo): string | null {
  if (info.count === 0) return null;
  const shape = info.shape.map((n) => n.toLocaleString()).join(" × ");
  const at = info.start.map((n) => (n + 1).toLocaleString()).join(", ");
  const image = info.image_shape.map((n) => n.toLocaleString()).join(" × ");
  return (
    `Tile [${info.index}], the ${ordinal(info.index + 1)} of ${info.count.toLocaleString()}: ` +
    `${shape} pixels, first pixel at (${at}) in the ${image} image (pixel coordinates: axis 1 first, counting from 1)`
  );
}

/** What the bytes were and how large, before and after: the algorithm that
 *  ran, which for a tile in one of the fallback columns is not the one
 *  `ZCMPTYPE` names. */
export function sizeLine(info: TileInfo): string | null {
  if (info.count === 0) return null;
  const algorithm =
    info.column === "GZIP_COMPRESSED_DATA" ? "gzip" : info.column === "UNCOMPRESSED_DATA" ? "uncompressed" : info.algorithm;
  const packed = `${countText(info.packed, "byte")} in the file`;
  const unpacked =
    info.decoded > 0 && info.decoded !== info.packed ? `, ${countText(info.decoded, "byte")} unpacked` : "";
  return `${algorithm}, ${packed}${unpacked}`;
}

/** Why this tile's bytes were not in COMPRESSED_DATA, where they were not. */
export function columnLine(info: TileInfo): string | null {
  const zcmptype = `ZCMPTYPE (${info.algorithm}) does not apply to this tile.`;
  if (info.column === "GZIP_COMPRESSED_DATA") {
    return `Stored in GZIP_COMPRESSED_DATA, not COMPRESSED_DATA: the raw floats, gzipped, which is the column for a float tile that would not quantize. ${zcmptype}`;
  }
  if (info.column === "UNCOMPRESSED_DATA") {
    return `Stored in UNCOMPRESSED_DATA, not COMPRESSED_DATA: the raw floats as they are, which is the column for a float tile that would not quantize. ${zcmptype}`;
  }
  return null;
}

/** What one step did: the bytes in and out where it produced bytes, and its
 *  note. A step that stopped before producing anything says how many bytes it
 *  was handed, and a step that works on numbers rather than bytes says only
 *  its note. */
export function tileStepText(step: PageStep): string {
  const parts: string[] = [];
  if (step.out_bytes > 0) parts.push(bytesChange(step.in_bytes, step.out_bytes));
  if (step.note !== "") parts.push(step.note);
  if (parts.length === 0 && step.in_bytes > 0) parts.push(countText(step.in_bytes, "byte"));
  return parts.join(", ");
}

/** The subhead over the pixels, which says `First` only when some are left
 *  out: a tile of one pixel is not the first of anything. */
export function pixelsHead(info: TileInfo): string {
  const all = info.values.length >= info.total;
  return `${all ? "Pixels" : "First pixels"}, as ${info.element_type}`;
}

/**
 * The whole panel: which tile, its sizes, the steps in the order they were
 * done, why it stopped if it did, and the first pixels.
 *
 * A tile that would not decompress still gets a panel: the steps done before
 * the one that stopped it are how a reader tells an unusual file from a gap in
 * this program.
 */
export function tileBody(info: TileInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  for (const text of [tileLine(info), sizeLine(info), columnLine(info)]) {
    if (text !== null) frag.append(line("insp-qcount", text));
  }
  const rows = info.steps.map((s) => ({ label: s.what, text: tileStepText(s) }));
  frag.append(stepList("Steps, in the order they were done", rows, true));
  frag.append(problemLine(info.problem));
  frag.append(firstValues(pixelsHead(info), info.values, info.total, "pixels"));
  return frag;
}

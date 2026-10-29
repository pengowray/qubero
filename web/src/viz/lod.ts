// How much a zoomable picture draws at a given zoom: the pixel sizes at which
// it starts asking for a node's children, writes names, writes each byte in
// hex, and splits a byte into its eight bits.
//
// One table for every piece that zooms from the file down to the bit (the
// position icicle, the pixel grid, the stride raster, the file-to-bit view),
// so they turn the same details on at the same zoom and feel like one
// instrument rather than four. A piece that wants a different threshold
// argues for changing it here, where every other piece changes with it.
//
// DOM-free. The numbers are CSS pixels of the drawn thing, after the zoom.

export const LOD = {
  /** A bar or box this wide or wider is worth asking the core for its
   *  children. Narrower, the children would be slivers nobody can point at,
   *  and asking for them costs a walk for nothing. */
  childrenPx: 24,
  /** A byte cell this wide or wider has room for its two hex digits. The zoom
   *  map in the report has used this threshold since it was written
   *  (`HEX_PPB` in `report/zoommap.ts`). */
  hexPx: 14,
  /** A byte cell this wide or wider is drawn as eight bit cells, three pixels
   *  each, enough to see a run of set bits and to point at one. */
  bitsPx: 24,
  /** The smallest a hit target may be. Matches `pick.ts`: a target drawn
   *  narrower than this is still picked within this many pixels. */
  targetPx: 24,
} as const;

/** What a byte cell of a given width shows: its bits, its hex digits, or only
 *  its colour. */
export type ByteDetail = "bits" | "hex" | "colour";

export function byteDetail(bytePx: number): ByteDetail {
  if (bytePx >= LOD.bitsPx) return "bits";
  if (bytePx >= LOD.hexPx) return "hex";
  return "colour";
}

/** Whether a drawn bar is wide enough to open into its children. */
export function opensChildren(barPx: number): boolean {
  return barPx >= LOD.childrenPx;
}

/** How wide one byte is drawn when `bytes` bytes fill `px` pixels: what a
 *  piece over an address axis passes to `byteDetail`. Zero for an empty
 *  stretch rather than infinity, so an empty file draws colour only. */
export function bytePx(px: number, bytes: number): number {
  return bytes > 0 ? px / bytes : 0;
}

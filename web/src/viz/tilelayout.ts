// The arithmetic behind the record tiles (`recordtiles.ts`): which colour each
// record gets, how big the tiles are and how many fit in a row, which tile a
// pointer is on, and how much of a tile its size fills.
//
// The tiles are a unit chart: one square per element of a list, in file
// order, all the same size, so a list's length is the area it covers and a
// run of one kind of record is a run of one colour. The size of each record is
// the fill, measured against the largest record in the list, so the one IDAT
// chunk that is shorter than the three hundred before it is the one tile that
// is not full.
//
// Kept apart from the drawing so that `node --test` can run it with no page.

/** Hues the tiles have before the rest of the kinds go grey. Seven and not
 *  eight: the dataviz palette's eighth hue is red, and red on this screen
 *  means a wrong value, which is what a problem tile's edge says. */
export const HUE_SLOTS = 7;

/** The slot every kind past the first `HUE_SLOTS` shares: grey. */
export const OTHER_SLOT = HUE_SLOTS;

export type LegendEntry = {
  /** The kind's name, or null for the entry standing for several kinds that
   *  had no hue left. */
  readonly name: string | null;
  readonly count: number;
  readonly slot: number;
  /** The kinds this entry covers. One, except for the grey entry. */
  readonly kinds: readonly string[];
};

export type Categories = {
  /** Each record's slot, in the order given. */
  readonly slot: readonly number[];
  /** One entry per hue, in slot order, then the grey one if any record is
   *  grey. */
  readonly legend: readonly LegendEntry[];
};

/**
 * Give each kind of record a slot of the palette. The kinds with the most
 * records get the hues, so a file with a hundred IDAT chunks after ten
 * ancillary ones still shows its IDAT run in colour; among those, the hues go
 * in order of first appearance in the file, so a kind keeps its colour when a
 * list grows by records of other kinds. The rest share grey. When only one
 * kind is left over, the grey entry carries its name, since "other kinds"
 * would then be a name for one thing.
 */
export function assignCategories(kinds: readonly string[], slots: number = HUE_SLOTS): Categories {
  const count = new Map<string, number>();
  const first = new Map<string, number>();
  kinds.forEach((k, i) => {
    count.set(k, (count.get(k) ?? 0) + 1);
    if (!first.has(k)) first.set(k, i);
  });
  const all = [...count.keys()];
  const byCount = [...all].sort((a, b) => (count.get(b) ?? 0) - (count.get(a) ?? 0) || (first.get(a) ?? 0) - (first.get(b) ?? 0));
  const hued = byCount.slice(0, slots).sort((a, b) => (first.get(a) ?? 0) - (first.get(b) ?? 0));
  const slotOf = new Map(hued.map((k, i) => [k, i]));
  const rest = all.filter((k) => !slotOf.has(k)).sort((a, b) => (first.get(a) ?? 0) - (first.get(b) ?? 0));
  const legend: LegendEntry[] = hued.map((k, i) => ({ name: k, count: count.get(k) ?? 0, slot: i, kinds: [k] }));
  if (rest.length > 0) {
    legend.push({
      name: rest.length === 1 ? (rest[0] as string) : null,
      count: rest.reduce((n, k) => n + (count.get(k) ?? 0), 0),
      slot: OTHER_SLOT,
      kinds: rest,
    });
  }
  return { slot: kinds.map((k) => slotOf.get(k) ?? OTHER_SLOT), legend };
}

export type TileGrid = {
  readonly count: number;
  readonly cols: number;
  readonly rows: number;
  /** A tile's side, and the space between two tiles, in layout pixels. */
  readonly cell: number;
  readonly gap: number;
  /** `cell + gap`: from one tile's left edge to the next one's. */
  readonly pitch: number;
  readonly width: number;
  readonly height: number;
};

/** Tile sides to try, largest first. The largest is about the height of two
 *  lines of the listing's text, which a list of six chunks fills a row of
 *  without looking like buttons; the smallest still shows a colour. */
export const TILE_SIDES: readonly number[] = [28, 22, 18, 14, 11, 8, 6, 4, 3, 2];

/** The space between two tiles of this side: the dataviz skill's 2px gap
 *  between fills, and 1px once tiles are too small to spare two. */
export function gapFor(cell: number): number {
  return cell >= 8 ? 2 : 1;
}

/**
 * The largest tiles that put `count` of them in rows `width` pixels wide no
 * taller than `maxHeight`. When none is short enough the smallest is used and
 * the grid is as tall as it has to be: every record still has its tile.
 */
export function tileGrid(count: number, width: number, maxHeight: number): TileGrid {
  const n = Math.max(0, count);
  let grid: TileGrid | null = null;
  for (const cell of TILE_SIDES) {
    const gap = gapFor(cell);
    const pitch = cell + gap;
    const cols = Math.max(1, Math.floor((width + gap) / pitch));
    const rows = Math.ceil(n / cols);
    // A list that fits one row is as wide as its tiles, not the whole width.
    const usedCols = Math.min(cols, Math.max(1, n));
    grid = { count: n, cols, rows, cell, gap, pitch, width: usedCols * pitch - gap, height: Math.max(0, rows * pitch - gap) };
    if (grid.height <= maxHeight) return grid;
  }
  return grid as TileGrid;
}

/** Where tile `i` is, in layout pixels. */
export function tileBox(g: TileGrid, i: number): { x0: number; y0: number; x1: number; y1: number } {
  const col = i % g.cols;
  const row = Math.floor(i / g.cols);
  const x0 = col * g.pitch;
  const y0 = row * g.pitch;
  return { x0, y0, x1: x0 + g.cell, y1: y0 + g.cell };
}

/**
 * The tile a pointer at layout (`x`, `y`) is on, or null. Grid arithmetic
 * rather than a scan, so fifty thousand tiles cost the same as six. A point in
 * the gap between tiles takes the nearest. `reach` is the smallest target in
 * layout pixels (`LOD.targetPx` divided by the zoom): a tile drawn smaller
 * than that is still picked within half of it of its centre.
 */
export function tileAt(g: TileGrid, x: number, y: number, reach: number = 0): number | null {
  if (g.count === 0) return null;
  const col = Math.round((x - g.cell / 2) / g.pitch);
  const row = Math.round((y - g.cell / 2) / g.pitch);
  if (col < 0 || col >= g.cols || row < 0 || row >= g.rows) return null;
  const i = row * g.cols + col;
  if (i >= g.count) return null;
  const b = tileBox(g, i);
  const slack = Math.max(g.gap, (reach - g.cell) / 2);
  const dx = Math.max(b.x0 - x, 0, x - b.x1);
  const dy = Math.max(b.y0 - y, 0, y - b.y1);
  return dx <= slack && dy <= slack ? i : null;
}

/** How many pixels of a tile `cell` pixels tall a record fills, against the
 *  largest record. Linear, so twice the bytes is twice the fill; a record of
 *  any size gets at least one pixel, so it is never drawn as empty. */
export function fillPx(sizeBits: number, maxBits: number, cell: number): number {
  if (sizeBits <= 0 || maxBits <= 0) return 0;
  return Math.max(1, Math.min(cell, Math.round((cell * sizeBits) / maxBits)));
}

/** What a list's elements are called, in the singular: the list's own word
 *  for them, else the element type's name as a word (`Chunk[]` is `chunk`),
 *  else `item`. */
export function recordWord(list: { readonly unit?: string; readonly type: string }): string {
  if (list.unit !== undefined && list.unit !== "") return list.unit;
  const element = list.type.replace(/^(?:offsets|chain|descriptors) → /, "").replace(/\[\]$/, "");
  if (/^[A-Z][a-z]+$/.test(element)) return element.toLowerCase();
  return "item";
}

/**
 * Records in a row that are the same are one tile with a count on it, the
 * repeat glyph of the design: three hundred IDAT chunks of 8,192 bytes are a
 * run that the eye reads as one thing, and drawing it once leaves room for the
 * records that differ. `keys` says what makes two records the same (their
 * type and their size); a null key is a record that is never folded into a
 * run, such as one with a wrong value inside it, which has to stay a tile of
 * its own to be found.
 */
export type Cells = {
  /** The first record of each cell, in file order. */
  readonly first: readonly number[];
  /** How many records each cell stands for: 1, or the length of the run. */
  readonly count: readonly number[];
};

export function collapseRuns(keys: readonly (string | null)[]): Cells {
  const first: number[] = [];
  const count: number[] = [];
  for (let i = 0; i < keys.length; i++) {
    const k = keys[i] ?? null;
    const last = first.length - 1;
    if (k !== null && last >= 0 && keys[first[last] as number] === k) {
      count[last] = (count[last] ?? 0) + 1;
      continue;
    }
    first.push(i);
    count.push(1);
  }
  return { first, count };
}

/** The cell record `record` is in, or null for a record past the last. */
export function cellOf(cells: Cells, record: number): number | null {
  const { first, count } = cells;
  let lo = 0;
  let hi = first.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const f = first[mid] as number;
    if (record < f) hi = mid - 1;
    else if (record >= f + (count[mid] as number)) lo = mid + 1;
    else return mid;
  }
  return null;
}

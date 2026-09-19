// Where a row of the table is stored, said in words.
//
// Most tables have a row that is a run of the file, and the two address
// columns say where it starts and how long it is. The tables the core works
// out -- a pandas frame, a torch tensor, a checkpoint's summary -- do not: a
// row of a frame is one value out of each of several blocks, and a cell of it
// can be in an unpacked stream or nowhere at all. So a row says where its
// first cell is and that the rest are elsewhere, and every cell carries its
// own address on its hover.
//
// The words are here and not in `tableview.ts` because three things ask for
// them and they have to agree: the columns on screen, a copy of the selected
// rows, and an exported file. `tabletext.ts` is the one that must stay free of
// the DOM, so nothing here reaches for `document` outside `drawAddress`.

import { el } from "./dom.ts";
import { formatAddress } from "./format.ts";
import type { CellPlace, RecordCell } from "./records.ts";
import { bitSizeText, DECODED_PLUS_TITLE, TABLE } from "./strings.ts";
import { rowRun, type TableRow } from "./tableplan.ts";

/** Whether a row has any bytes behind it.
 *
 * Only a table whose cells carry their own addresses can have a row with
 * none, and such a row is told apart from an ordinary table's by its cells
 * saying anything about where they are at all. A summary row whose every
 * column is a fact the pickle states is the row this rules out: it has no
 * offset, so there is nothing to send the file tab and nothing to draw in the
 * address column but the reason. */
export function rowHasBytes(row: TableRow): boolean {
  const placed = row.cells.some((cell) => cell.at !== undefined);
  if (placed) return true;
  return !row.cells.some((cell) => cell.noBytes !== undefined);
}

/** Why a row has no bytes, from the first of its cells that says. */
function whyNoBytes(row: TableRow): "counted" | "nowhere" {
  return row.cells.find((cell) => cell.noBytes !== undefined)?.noBytes ?? "nowhere";
}

/** The two address cells of one row as text: what the columns show, what a
 *  copy holds, and what an export writes. One answer for all three, so a
 *  pasted row says what the row it was pasted from said. */
export function copiedAddress(row: TableRow): [address: string, size: string] {
  if (!rowHasBytes(row)) return [TABLE.noBytes(whyNoBytes(row)), ""];
  const at = formatAddress(row.offsetBits, row.space ?? 0);
  return [at, row.apart === true ? TABLE.sizePerCell : bitSizeText(row.sizeBits)];
}

/**
 * The two address cells of one row, drawn.
 *
 * A row that is one run of bytes shows where it starts and how long it is,
 * which is what every table with a row of its own in the file shows. A row
 * whose cells are in several places shows the first cell's address and says
 * so; a row with no bytes at all says why it has none rather than showing
 * an address of nought.
 */
export function drawAddress(row: TableRow): HTMLElement[] {
  const [address, size] = copiedAddress(row);
  if (!rowHasBytes(row)) {
    const said = el("span", { className: "tbl-cell tbl-at", textContent: address });
    said.title = TABLE.noBytesWhy(whyNoBytes(row));
    return [said, el("span", { className: "tbl-cell tbl-size" })];
  }
  const at = el("span", { className: "tbl-cell tbl-at", textContent: address });
  const sized = el("span", { className: "tbl-cell tbl-size tbl-num", textContent: size });
  const plus = (row.space ?? 0) === 0 ? [] : [DECODED_PLUS_TITLE];
  if (row.apart === true) {
    at.title = [TABLE.cellsApart, ...plus].join("\n");
    sized.title = TABLE.cellsApart;
  } else if (plus.length > 0) {
    at.title = DECODED_PLUS_TITLE;
  }
  return [at, sized];
}

/** Where one cell's bytes are, or why it has none, for the lines under its
 *  text on hover. Empty for a cell of a table that says nothing about where
 *  its cells are, which is every table whose rows are runs of the file.
 *
 *  An address inside an unpacked stream gets a line saying what the `+` in
 *  front of it counts from. The listing puts those words on the `+` itself;
 *  a hover cannot carry a hover, so it says them outright. */
export function whereLines(cell: RecordCell): string[] {
  // A masked entry is shown empty, and a reader hovering an empty cell is
  // asking why. Said before the address, because it is what the cell is: the
  // address under it is the number the array is not counting.
  const why = cell.masked === true ? [TABLE.maskedCell] : [];
  const at = cell.at;
  if (at === undefined) return cell.noBytes === undefined ? why : [...why, TABLE.noBytesWhy(cell.noBytes)];
  const said = TABLE.cellAt(formatAddress(at.offsetBits, at.space), bitSizeText(at.sizeBits));
  return at.space === 0 ? [...why, said] : [...why, said, DECODED_PLUS_TITLE];
}

/**
 * Where the cells of one column are, record by record.
 *
 * A table drawn turned has a column in each of its rows, so this is what the
 * address of a drawn row is worked out from there. A cell that carries its
 * own address answers with it; a row that says where its fields are answers
 * from those, in the tab's own space, which is where a field it read from
 * lives. Null for a cell nothing can be said about.
 */
export function columnPlaces(records: readonly TableRow[], c: number): (CellPlace | null)[] {
  return records.map((row) => cellPlaceIn(row, c));
}

/** Where cell `c` of one row is. A cell that carries its own address answers
 *  with it; otherwise the field the row read that cell from says, in the tab's
 *  own space. Null for a cell neither can place. */
export function cellPlaceIn(row: TableRow, c: number): CellPlace | null {
  const at = row.cells[c]?.at;
  if (at !== undefined) return at;
  const span = row.spans?.[c];
  return span === undefined ? null : { space: 0, offsetBits: span.offsetBits, sizeBits: span.sizeBits };
}

/**
 * Where a drawn row of a turned table is, for its heading's hover.
 *
 * Turned, a drawn row is one column of the table: the same field of every
 * record. Those cells are often a run of their own even when the records are
 * not -- a frame keeps a column's values together in one block, so the column
 * the unturned table could only call `per cell` has a real address and a real
 * length read down the page. Where they are scattered it says so, in the same
 * words the address column uses.
 *
 * Empty for a table that says nothing about where its cells are: turned or
 * not, a row of a SQLite page is a run of the file and the heading lines above
 * already say where each record is.
 */
export function fieldWhereLines(records: readonly TableRow[], c: number): string[] {
  const places = columnPlaces(records, c);
  if (places.every((place) => place === null)) {
    const why = records.find((row) => row.cells[c]?.noBytes !== undefined)?.cells[c]?.noBytes;
    return why === undefined ? [] : [TABLE.noBytesWhy(why)];
  }
  const run = rowRun(places);
  // A column some of whose cells are stored nowhere is not one run either,
  // whatever the rest of it looks like: an address and a length over the
  // placed cells alone would be read as covering the column.
  if (run.apart || places.some((place) => place === null)) return [TABLE.cellsApart];
  const said = TABLE.cellAt(formatAddress(run.offsetBits, run.space), bitSizeText(run.sizeBits));
  return run.space === 0 ? [said] : [said, DECODED_PLUS_TITLE];
}

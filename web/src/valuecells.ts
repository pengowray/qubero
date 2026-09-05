// The cells the value table is made of, and how a row's block of them is
// filled.
//
// What each one says is decided in `valuetable.ts`; this is only the writing
// of it. Like the chips, every cell is written over rather than rebuilt: a
// redraw usually wants the cells that are already there, and on a touch screen
// a finger may be resting on one, which the browser reads as the touch being
// called off if the element leaves the document — which stops the drag that is
// scrolling the view. Spare cells are hidden, never taken away.
//
// `.ts` on the imports for the reason `chipplan.ts` gives.
import type { ChipMeasure } from "./chipfit.ts";
import { fieldClass } from "./fieldstyle.ts";
import { setText } from "./hexcell.ts";
import { VALUES } from "./strings.ts";
import type { PlacedCell, RowValues } from "./valuetable.ts";

/** The block of cells for one row, with the click handler that reads which
 *  element was pressed off the cell itself. */
export type ValsEl = HTMLElement;

/** How a cell says which element it is: the run's path and the index, so one
 *  handler on the block serves every cell for as long as the block lives. */
function pickPath(el: HTMLElement): number[] | null {
  const path = el.dataset["path"];
  const index = el.dataset["index"];
  if (path === undefined || index === undefined) return null;
  const steps = path === "" ? [] : path.split(",").map(Number);
  const i = Number(index);
  if (!Number.isFinite(i) || steps.some((s) => !Number.isFinite(s))) return null;
  return [...steps, i];
}

/** An empty block of value cells, ready to be filled. One click handler for
 *  the block rather than one per cell: a screenful of 24-bit samples is a
 *  thousand cells, and a thousand listeners is a thousand things to take off
 *  again. */
export function newVals(onPick: (path: readonly number[]) => void): ValsEl {
  const el = document.createElement("div");
  el.className = "hv-vals";
  el.addEventListener("click", (e) => {
    const target = e.target;
    if (!(target instanceof HTMLElement)) return;
    const cell = target.closest<HTMLElement>(".hv-val");
    if (cell === null || cell.parentElement !== el) return;
    const path = pickPath(cell);
    if (path === null) return;
    e.stopPropagation();
    onPick(path);
  });
  return el;
}

/** What one cell says, drawn into the element already there. */
function fillCell(el: HTMLElement, c: PlacedCell, layout: RowValues["layout"]): void {
  const aligned = layout === "aligned";
  let cls = `hv-val ${c.copy ? "field-marker" : fieldClass(c.kind)}`;
  if (c.carried !== null) cls += " hv-val-continued";
  if (c.carried === "below") cls += " hv-val-before";
  if (c.numeric) cls += " hv-val-num";
  if (c.cut && c.carried === null) cls += " hv-val-cut";
  if (el.className !== cls) el.className = cls;
  setText(el, c.text);
  const path = c.path.join(",");
  if (el.dataset["path"] !== path) el.dataset["path"] = path;
  const index = String(c.index);
  if (el.dataset["index"] !== index) el.dataset["index"] = index;
  const title =
    c.carried === "above"
      ? VALUES.continued(c.run, c.index)
      : c.carried === "below"
        ? VALUES.continues(c.run, c.index)
        : c.symbol
          ? VALUES.symbol(c.index, c.tip, c.sizeBits)
          : VALUES.cell(c.run, c.index, c.type, c.tip);
  if (el.title !== title) el.title = title;
  // The tint says a cell is a piece of one whose value is on another row; a
  // screen reader has only the words.
  if (c.carried === "above") el.setAttribute("aria-label", VALUES.continuedLabel);
  else if (c.carried === "below") el.setAttribute("aria-label", VALUES.continuesLabel);
  else el.removeAttribute("aria-label");
  const column = aligned ? `${c.from} / ${c.to}` : "";
  if (el.style.gridColumn !== column) el.style.gridColumn = column;
  // Flow cells are each as wide as their own text, and the width is set here
  // rather than left to the browser so that the wrap the plan counted lines
  // from is the wrap the browser does. Cleared on the other two layouts: the
  // element may have been a flow cell a draw ago.
  const width = layout === "flow" ? `${c.width}px` : "";
  if (el.style.width !== width) el.style.width = width;
}

/** The `+N` that counts what the condensed cap left over. */
function fillRest(el: HTMLElement, n: number): void {
  const cls = "hv-val hv-val-rest";
  if (el.className !== cls) el.className = cls;
  setText(el, VALUES.rest(n));
  const tip = VALUES.restTip(n);
  if (el.title !== tip) el.title = tip;
  el.removeAttribute("data-path");
  el.removeAttribute("data-index");
  el.removeAttribute("aria-label");
  if (el.style.gridColumn !== "") el.style.gridColumn = "";
  if (el.style.width !== "") el.style.width = "";
}

/**
 * Put a row's values into its block, reusing the cells already there.
 *
 * `width` is how wide the aligned table is drawn — a byte of it at the pitch
 * of a hex cell where the column is wide enough for that — and is ignored by
 * the uniform layout, whose cells are all one measured width.
 */
export function fillVals(el: ValsEl, plan: RowValues, width: number, bpr: number): void {
  const aligned = plan.layout === "aligned";
  const want = plan.cells.length + (plan.rest > 0 ? 1 : 0);
  let cls = "hv-vals";
  if (want === 0) cls += " hv-empty";
  else cls += ` hv-vals-${plan.layout}`;
  if (el.className !== cls) el.className = cls;
  while (el.childElementCount < want) el.append(document.createElement("span"));
  for (let i = 0; i < el.childElementCount; i++) {
    const c = el.children[i] as HTMLElement;
    if (c.hidden !== i >= want) c.hidden = i >= want;
  }
  if (want === 0) return;
  // The grid is as many columns as the row has bits, so a cell can span
  // exactly the bits its element is stored in; the width is what makes a byte
  // of it the pitch of a byte of the bytes.
  // `max-width` rather than `width`: the block is a line of its own inside the
  // note, which it is by taking the note's whole width, and a flex item's
  // basis wins over its width. What is capped here is how far the grid
  // stretches, which is what gives a byte of it the pitch of a hex cell.
  const w = aligned ? `${Math.round(width)}px` : "";
  if (el.style.maxWidth !== w) el.style.maxWidth = w;
  const columns = aligned ? `repeat(${bpr * 8}, minmax(0, 1fr))` : "";
  if (el.style.gridTemplateColumns !== columns) el.style.gridTemplateColumns = columns;
  // Uniform's one width for every cell. Aligned takes its widths from the
  // grid and flow from each cell's own text, so neither wants this.
  const cell = plan.layout === "uniform" ? `${Math.round(plan.cellWidth)}px` : "";
  if (el.style.getPropertyValue("--hv-val-w") !== cell) el.style.setProperty("--hv-val-w", cell);
  for (const [i, c] of plan.cells.entries()) fillCell(el.children[i] as HTMLElement, c, plan.layout);
  if (plan.rest > 0) fillRest(el.children[plan.cells.length] as HTMLElement, plan.rest);
}

/**
 * How a value cell's own text is measured, read off a cell that has been
 * drawn. The chips' font is a size larger, and measuring these against it says
 * a run does not fit the aligned layout when it does.
 *
 * Null until there is a cell to read a font from; the caller keeps the chips'
 * font until then and draws once more when this arrives.
 */
export function readValFont(root: HTMLElement): ChipMeasure | null {
  const cell = root.querySelector(".hv-val");
  if (!(cell instanceof HTMLElement)) return null;
  const ctx = document.createElement("canvas").getContext("2d");
  if (ctx === null) return null;
  const s = getComputedStyle(cell);
  const font = `${s.fontStyle} ${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;
  const width = (text: string): number => {
    ctx.font = font;
    return ctx.measureText(text).width;
  };
  return { name: width, value: width };
}

/**
 * Mark the cell the cursor is in, and clear the one it has left.
 *
 * Its own pass, not part of what a block says: a cursor key moves the mark two
 * cells and would otherwise rewrite every value on the screen, and the mark is
 * where the reader is rather than what the file holds.
 */
export function markVals(el: ValsEl, plan: RowValues, bitOffset: number): void {
  for (const [i, c] of plan.cells.entries()) {
    const at = el.children[i];
    if (!(at instanceof HTMLElement)) continue;
    const on = bitOffset >= c.startBit && bitOffset < c.endBit;
    if (at.classList.contains("hv-val-at") !== on) at.classList.toggle("hv-val-at", on);
  }
}

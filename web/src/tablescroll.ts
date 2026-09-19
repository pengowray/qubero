// Which rows of a long table are on screen, and where to draw them.
//
// The rows are virtual the way `listpane.ts`'s are: one height each, so which
// ones are on screen is division rather than search. What is different here is
// the mapping past the cap. Ten minutes of stereo sound is twenty-six million
// rows, which is six hundred million pixels of canvas, and browsers stop
// honouring an element's height long before that. Past the cap the canvas
// stops growing and the scroll bar is read as a ratio instead: where it sits
// in its travel is where the reader is in the rows, and the rows on screen are
// drawn against the viewport rather than against the canvas.
//
// The header is inside the scroller, stuck to its top edge, so it is part of
// what is scrolled through and part of what the rows have to share the height
// with. Every measurement below allows for it.

/** Height of one row, which must match `--tbl-row` in the stylesheet: the rows
 *  are placed by arithmetic on it, so a row that drew taller would slide out
 *  from under its own place. */
export const ROW = 22;
/** Rows drawn above and below the window, so a wheel notch has somewhere to go
 *  before the next paint. */
export const OVERSCAN = 8;
/** How tall the canvas is allowed to get. Past this the scroll bar stands for
 *  the rows by ratio rather than by pixels. */
const MAX_CANVAS = 16_000_000;

/** The rows to draw, and where the first of them goes. */
export type RowWindow = { readonly from: number; readonly to: number; readonly base: number };

export class RowScroll {
  private readonly scroller: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly head: HTMLElement;
  /** How many rows there are to scroll through: the records, or the fields
   *  when the table is drawn turned. */
  private count = 0;

  constructor(scroller: HTMLElement, canvas: HTMLElement, head: HTMLElement) {
    this.scroller = scroller;
    this.canvas = canvas;
    this.head = head;
  }

  /** How many rows there are now, which is also how tall the canvas is. */
  setCount(count: number): void {
    this.count = count;
    this.canvas.style.height = `${Math.min(MAX_CANVAS, count * ROW)}px`;
  }

  /** Back to the start, both ways: a table drawn the other way round is a
   *  different table to be at the top of. */
  home(): void {
    this.scroller.scrollTop = 0;
    this.scroller.scrollLeft = 0;
  }

  /** How many rows fit on screen, at least one so a short tab still draws.
   *  The header is stuck over the top of the scroller, so what is left for the
   *  rows is the scroller less the header. */
  onScreen(): number {
    return Math.max(1, Math.floor((this.scroller.clientHeight - this.head.offsetHeight) / ROW));
  }

  /** How far the scroll bar can go. The header is in the scroller with the
   *  canvas, so it is part of what is scrolled through. */
  private travel(): number {
    return this.head.offsetHeight + this.canvas.clientHeight - this.scroller.clientHeight;
  }

  /** True once the rows are taller than a canvas is allowed to be, which is
   *  where the scroll bar stops standing for pixels. */
  private get capped(): boolean {
    return this.count * ROW > MAX_CANVAS;
  }

  /** The first row the scroll position is asking for. Under the cap that is
   *  division; past it the bar's place in its own travel is the reader's place
   *  in the rows, which is the only mapping left once the pixels run out. */
  firstVisible(): number {
    if (!this.capped) return Math.floor(this.scroller.scrollTop / ROW);
    const travel = this.travel();
    const ratio = travel <= 0 ? 0 : this.scroller.scrollTop / travel;
    return Math.round(ratio * Math.max(0, this.count - this.onScreen()));
  }

  /**
   * The rows to draw and where the first of them sits.
   *
   * Under the cap a row sits at its own place on the canvas. Past it the
   * canvas is shorter than the rows would need, so the window is drawn where
   * the reader is looking: at the top of the viewport, wherever that is.
   */
  window(): RowWindow {
    const firstVisible = this.firstVisible();
    const from = Math.max(0, firstVisible - OVERSCAN);
    const to = Math.min(this.count, firstVisible + this.onScreen() + OVERSCAN);
    const base = this.capped ? this.scroller.scrollTop - (firstVisible - from) * ROW : from * ROW;
    return { from, to, base };
  }

  /** Bring row `i` into view, unless it is there already. */
  scrollToRow(i: number): void {
    const onScreen = this.onScreen();
    const first = this.firstVisible();
    if (i >= first && i < first + onScreen) return;
    const want = Math.max(0, i - Math.floor(onScreen / 2));
    if (!this.capped) {
      this.scroller.scrollTop = want * ROW;
      return;
    }
    const travel = this.travel();
    const rows = Math.max(1, this.count - onScreen);
    this.scroller.scrollTop = Math.max(0, Math.min(travel, (want / rows) * travel));
  }
}

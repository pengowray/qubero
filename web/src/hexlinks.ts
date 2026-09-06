// Arrows over the hex grid, from the fields that decided a field's shape to
// the field itself.
//
// The sidebar already names them: a length here, a count there, the switch
// value that chose this type. What it cannot say is how far away they are.
// A GGUF tensor's name is as long as the number four bytes in front of it and
// a PNG chunk's data is as long as the number at the top of the chunk, and
// those are two very different distances; read as a list of names they look
// alike. Drawn over the bytes they do not.
//
// Nothing here touches the grid. The overlay is one absolutely positioned SVG
// inside the box the rows are clipped to, with `pointer-events: none`, drawn
// after the rows have been laid out and measured. It cannot change a row's
// height, which is the one thing the hex view will not tolerate.

/** Where a byte is drawn, in the overlay's own coordinates. Null when that
 *  byte is not on screen. */
export type ByteBox = (byte: number) => { x: number; y: number; w: number; h: number } | null;

/** One field the selected field depends on, as the overlay needs it. */
export type LinkEnd = {
  /** `Role.as_str()` from the core: length, count, type, position, value,
   *  name, width, points. Drawn as the word on the arrow. */
  readonly role: string;
  readonly label: string;
  readonly startBit: number;
  readonly endBit: number;
  /**
   * The first bit of what this field decided about.
   *
   * Not always the field at the cursor. A run of packed weights is as long as
   * a number three levels up said the record was, and the arrow that says so
   * has to land on the record: pointing it at the weights would claim the
   * number sized them, which is one deduction further than the core made.
   */
  readonly decidesBit: number;
};

/** Everything the overlay draws for one selected field. */
export type LinkPlan = {
  /** The field the cursor is on, which every arrow points at. */
  readonly target: { readonly startBit: number; readonly endBit: number } | null;
  /** The structure the field is part of, outlined so that a length four rows
   *  up reads as a length of *this* record and not of the file. */
  readonly parent: { readonly startBit: number; readonly endBit: number; readonly name: string } | null;
  readonly from: readonly LinkEnd[];
};

/** A run of bytes on one line of the grid. */
type Box = { x: number; y: number; w: number; h: number };

const NS = "http://www.w3.org/2000/svg";

/** How many bytes of one field the overlay will probe for boxes. A field of a
 *  hundred megabytes covers every row on screen, and the rows on screen are
 *  what bound the work: past a screenful of boxes the outline is the whole
 *  view and one more box adds nothing. */
const PROBE_LIMIT = 4096;

function svg<K extends keyof SVGElementTagNameMap>(name: K, attrs: Record<string, string>): SVGElementTagNameMap[K] {
  const e = document.createElementNS(NS, name);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
}

/**
 * The lines drawn over the grid for the field at the cursor.
 *
 * Held apart from the hex view because the view is already the largest file
 * here and because the overlay is optional: switched off, this class is asked
 * for nothing and the SVG holds no children.
 */
export class HexLinks {
  readonly el: SVGSVGElement;
  private plan: LinkPlan = { target: null, parent: null, from: [] };
  private on = false;
  /** Bytes the grid is showing, so a field far off screen is known to be off
   *  screen without probing for a box that cannot exist. */
  private window = { start: 0, end: 0 };
  /** The field the pointer is on in the sidebar. See `setHover`. */
  private hover: { readonly startBit: number; readonly endBit: number } | null = null;
  /** Which fields on the plan were nowhere on screen, for the line the status
   *  bar shows. Kept so the line is written again only when it would say
   *  something else. */
  private missed = "";

  /** Told when the off-screen ends change, so the status bar can say which
   *  fields they are without the view polling for them. A field is above the
   *  screen when it starts before the first byte drawn. */
  onOffScreen: (ends: readonly { end: LinkEnd; above: boolean }[]) => void = () => {};

  constructor(private readonly boxOf: ByteBox) {
    this.el = svg("svg", { class: "hv-links" });
    this.el.setAttribute("aria-hidden", "true");
    this.el.append(arrowDefs());
  }

  /** Whether the overlay draws at all. Switched off it clears itself, so no
   *  stale arrow is left over the bytes. */
  setEnabled(on: boolean): void {
    if (this.on === on) return;
    this.on = on;
    if (!on) {
      this.clearShapes();
      this.missed = "";
      this.onOffScreen([]);
    }
  }

  get enabled(): boolean {
    return this.on;
  }

  /** The field the cursor moved to, and what it depends on. */
  setPlan(plan: LinkPlan): void {
    this.plan = plan;
  }

  /**
   * The field the pointer is resting on in the sidebar, marked over the bytes.
   *
   * Drawn here rather than as a mark on the cells because a mark on the cells
   * is a class per byte computed on every frame, and the hex view already
   * carries three of those. A box drawn over the top costs one rectangle and
   * nothing at all while nothing is hovered. Answered whether or not the
   * arrows are switched on: pointing at a row in the sidebar is a question
   * about where that field is, and it deserves an answer either way.
   */
  setHover(range: { readonly startBit: number; readonly endBit: number } | null): boolean {
    const same =
      (range === null && this.hover === null) ||
      (range !== null && this.hover !== null && range.startBit === this.hover.startBit && range.endBit === this.hover.endBit);
    if (same) return false;
    this.hover = range;
    return true;
  }

  /** Which bytes the grid is showing, from the view's own viewport event. */
  setWindow(startBit: number, endBit: number): void {
    this.window = { start: Math.floor(startBit / 8), end: Math.ceil(endBit / 8) };
  }

  /**
   * Draw, having been told the rows are laid out and measured.
   *
   * Every box is read back from the grid rather than worked out from a row
   * height, because a heading, a wrapped line of chips or a value table can
   * make any row taller than the one above it. Reading the boxes is one forced
   * layout for the whole overlay, after the view has already taken its own.
   */
  draw(width: number, height: number): void {
    if (!this.on && this.hover === null) {
      if (this.el.firstElementChild?.nextElementSibling !== undefined) this.clearShapes();
      return;
    }
    this.el.setAttribute("viewBox", `0 0 ${Math.round(width)} ${Math.round(height)}`);
    this.el.setAttribute("width", String(Math.round(width)));
    this.el.setAttribute("height", String(Math.round(height)));
    const parts: SVGElement[] = [];
    if (this.hover !== null) {
      for (const b of this.boxes(this.hover.startBit, this.hover.endBit)) parts.push(this.outline(b, "hv-link-hover"));
    }
    if (!this.on) {
      this.el.replaceChildren(arrowDefs(), ...parts);
      return;
    }
    const target = this.plan.target === null ? [] : this.boxes(this.plan.target.startBit, this.plan.target.endBit);
    // The structure as one box round the lot of it. A record of six hundred
    // bytes covers twenty rows, and twenty separate outlines read as twenty
    // things rather than as the edge of one.
    if (this.plan.parent !== null) {
      const bounds = bound(this.boxes(this.plan.parent.startBit, this.plan.parent.endBit));
      if (bounds !== null) parts.push(this.outline(bounds, "hv-link-parent"));
    }
    // Only where the arrows land. The field at the cursor is already marked
    // cell by cell by the view itself; outlining every row of it again says
    // nothing the reader cannot already see, and buries the arrowheads.
    const head = target[0] ?? null;
    if (head !== null) parts.push(this.outline(head, "hv-link-target"));
    const missed: { end: LinkEnd; above: boolean }[] = [];
    for (const end of this.plan.from) {
      const boxes = this.boxes(end.startBit, end.endBit);
      const source = boxes[0] ?? null;
      // Where the arrow lands: the first byte of whatever this field decided
      // about, which is the field at the cursor for its own dependencies and
      // the enclosing record for an ancestor's.
      const lands = this.boxes(end.decidesBit, end.decidesBit + 8)[0] ?? head;
      if (source === null || lands === null) {
        missed.push({ end, above: Math.floor(end.startBit / 8) < this.window.start });
        continue;
      }
      for (const b of boxes) parts.push(this.outline(b, "hv-link-source"));
      parts.push(...this.arrow(source, lands, end.role, height));
    }
    this.el.replaceChildren(arrowDefs(), ...parts);
    // Written again only when the answer changed. Every scroll redraws the
    // overlay, and a status line rewritten on every frame is a status line the
    // reader cannot finish reading.
    const key = missed.map((m) => `${m.end.startBit}${m.above ? "^" : "v"}`).join(",");
    if (key !== this.missed) {
      this.missed = key;
      this.onOffScreen(missed);
    }
  }

  /** One rounded box per line the run covers. A run of a hundred rows that is
   *  mostly off screen yields the boxes for the lines that are on it. */
  private boxes(startBit: number, endBit: number): Box[] {
    const first = Math.max(Math.floor(startBit / 8), this.window.start);
    const last = Math.min(Math.ceil(endBit / 8), this.window.end);
    if (last <= first) return [];
    const out: Box[] = [];
    let run: Box | null = null;
    const stop = Math.min(last, first + PROBE_LIMIT);
    for (let byte = first; byte < stop; byte++) {
      const b = this.boxOf(byte);
      if (b === null) {
        if (run !== null) out.push(run);
        run = null;
        continue;
      }
      // A new line, or a jump backwards: either way this byte starts a box.
      if (run === null || Math.abs(b.y - run.y) > 1) {
        if (run !== null) out.push(run);
        run = { ...b };
        continue;
      }
      run.w = b.x + b.w - run.x;
      run.h = Math.max(run.h, b.h);
    }
    if (run !== null) out.push(run);
    return out;
  }

  /** Everything but the arrowhead, which every arrow shares and which costs
   *  nothing to leave in place. */
  private clearShapes(): void {
    this.el.replaceChildren(arrowDefs());
  }

  private outline(b: Box, cls: string): SVGElement {
    return svg("rect", {
      class: cls,
      x: String(Math.round(b.x) - 1),
      y: String(Math.round(b.y) - 1),
      width: String(Math.round(b.w) + 2),
      height: String(Math.round(b.h) + 2),
      rx: "3",
    });
  }

  /**
   * One arrow from the field that decided to the field it decided about.
   *
   * Two cases, because a grid of bytes puts the two ends either on one line or
   * on two, and one curve cannot serve both. On one line the arrow arches over
   * the bytes between them, which is where there is room and where it cannot
   * be read as a run of marked cells. On two it swings out into the address
   * column to the left, which is the only vertical whitespace the view has;
   * the further apart the ends, the wider the swing, so a length reaching
   * across the screen looks like it does.
   */
  private arrow(from: Box, to: Box, role: string, height: number): SVGElement[] {
    const sameLine = Math.abs(from.y - to.y) < from.h / 2;
    const y1 = from.y + from.h / 2;
    const y2 = to.y + to.h / 2;
    let d: string;
    let lx: number;
    let ly: number;
    if (sameLine) {
      // Over the middle of each field rather than between their facing edges.
      // A length is very often the field immediately in front of what it
      // sizes, and an arrow drawn edge to edge between neighbours has nowhere
      // to go: it comes out as a dot on the boundary. Over the middles it
      // always spans something, and it spans the two fields it is about.
      const x1 = from.x + from.w / 2;
      const x2 = to.x + to.w / 2;
      const lift = Math.min(16, from.h * 0.7);
      const top = from.y - lift;
      const mid = (x1 + x2) / 2;
      d = `M ${r(x1)} ${r(from.y)} Q ${r(mid)} ${r(top - lift)}, ${r(x2)} ${r(to.y)}`;
      lx = mid;
      ly = top - lift + 3;
    } else {
      const x1 = from.x;
      const x2 = to.x;
      const bow = Math.min(70, 20 + Math.abs(y2 - y1) * 0.3);
      const cx = Math.max(2, Math.min(x1, x2) - bow);
      d = `M ${r(x1)} ${r(y1)} C ${r(cx)} ${r(y1)}, ${r(cx)} ${r(y2)}, ${r(x2)} ${r(y2)}`;
      lx = cx + 3;
      ly = (y1 + y2) / 2;
    }
    const path = svg("path", { class: "hv-link-arrow", d, "marker-end": "url(#hv-arrowhead)" });
    const label = svg("text", {
      class: "hv-link-role",
      x: String(r(lx)),
      y: String(r(Math.max(9, Math.min(height - 2, ly)))),
      "text-anchor": sameLine ? "middle" : "start",
    });
    label.textContent = role;
    return [path, label];
  }
}

/** One box round a run that covers more than one line. */
function bound(boxes: readonly Box[]): Box | null {
  const first = boxes[0];
  if (first === undefined) return null;
  let x = first.x;
  let y = first.y;
  let right = first.x + first.w;
  let bottom = first.y + first.h;
  for (const b of boxes) {
    x = Math.min(x, b.x);
    y = Math.min(y, b.y);
    right = Math.max(right, b.x + b.w);
    bottom = Math.max(bottom, b.y + b.h);
  }
  return { x, y, w: right - x, h: bottom - y };
}

/** Halves and quarter-pixels come out of `getBoundingClientRect`, and a path
 *  drawn on one is a blurred path. */
function r(n: number): number {
  return Math.round(n);
}

/** The one arrowhead every arrow shares. Put in the SVG once, since a marker
 *  per arrow is the same shape defined twenty times. */
export function arrowDefs(): SVGElement {
  const defs = svg("defs", {});
  const marker = svg("marker", {
    id: "hv-arrowhead",
    viewBox: "0 0 8 8",
    refX: "7",
    refY: "4",
    markerWidth: "6",
    markerHeight: "6",
    orient: "auto-start-reverse",
  });
  marker.append(svg("path", { class: "hv-link-head", d: "M 0 0 L 8 4 L 0 8 z" }));
  defs.append(marker);
  return defs;
}

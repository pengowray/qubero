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
  /** How many of the fields on the plan were nowhere on screen, for the line
   *  the status bar shows. */
  private missed = 0;

  /** Told when the count of off-screen ends changes, so the status bar can
   *  say so without the view polling it. */
  onOffScreen: (n: number) => void = () => {};

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
      this.el.replaceChildren();
      this.missed = 0;
      this.onOffScreen(0);
    }
  }

  get enabled(): boolean {
    return this.on;
  }

  /** The field the cursor moved to, and what it depends on. */
  setPlan(plan: LinkPlan): void {
    this.plan = plan;
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
    if (!this.on) return;
    this.el.setAttribute("viewBox", `0 0 ${Math.round(width)} ${Math.round(height)}`);
    this.el.setAttribute("width", String(Math.round(width)));
    this.el.setAttribute("height", String(Math.round(height)));
    const target = this.plan.target === null ? [] : this.boxes(this.plan.target.startBit, this.plan.target.endBit);
    const parts: SVGElement[] = [];
    if (this.plan.parent !== null) {
      const p = this.boxes(this.plan.parent.startBit, this.plan.parent.endBit);
      for (const b of p) parts.push(this.outline(b, "hv-link-parent"));
    }
    for (const b of target) parts.push(this.outline(b, "hv-link-target"));
    // Where the arrows land: the left edge of the field, which is the side the
    // gutter is on and so the side an arrow can reach without crossing bytes.
    const head = target[0] ?? null;
    let missed = 0;
    for (const end of this.plan.from) {
      const boxes = this.boxes(end.startBit, end.endBit);
      const source = boxes[0] ?? null;
      if (source === null || head === null) {
        missed++;
        continue;
      }
      for (const b of boxes) parts.push(this.outline(b, "hv-link-source"));
      parts.push(...this.arrow(source, head, end.role, height));
    }
    this.el.replaceChildren(...parts);
    if (missed !== this.missed) {
      this.missed = missed;
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
   * One arrow, bowed out to the left of the bytes.
   *
   * Straight lines between two boxes in the same column of a grid lie on top
   * of the bytes between them and on top of each other. Bowing every arrow the
   * same way into the space left of the first column keeps them off the bytes
   * and keeps two arrows to the same field apart, since they leave from
   * different rows.
   */
  private arrow(from: Box, to: Box, role: string, height: number): SVGElement[] {
    const x1 = from.x;
    const y1 = from.y + from.h / 2;
    const x2 = to.x;
    const y2 = to.y + to.h / 2;
    // How far left the curve swings: enough to clear the bytes, more when the
    // two ends are far apart, so a long arrow is a visibly long way round.
    const bow = Math.min(64, 12 + Math.abs(y2 - y1) * 0.25);
    const cx = Math.max(2, Math.min(x1, x2) - bow);
    const d = `M ${x1} ${y1} C ${cx} ${y1}, ${cx} ${y2}, ${x2} ${y2}`;
    const path = svg("path", { class: "hv-link-arrow", d, "marker-end": "url(#hv-arrowhead)" });
    const label = svg("text", {
      class: "hv-link-role",
      x: String(Math.round(cx + 4)),
      y: String(Math.round(Math.max(8, Math.min(height - 2, (y1 + y2) / 2)))),
    });
    label.textContent = role;
    return [path, label];
  }
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

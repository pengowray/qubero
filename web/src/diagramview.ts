/**
 * The format as boxes and arrows: one table per type, one row per field, and
 * an arrow from the field that decides to the field it decides about.
 *
 * The fifth main view, and the only one that is not about the open file. Hex,
 * Listing, Text and Strings all read bytes; this reads the template, so the
 * same picture is drawn for every PNG ever opened and nothing in it moves when
 * the file is edited.
 *
 * **Why not the graph view.** That one is force-directed, which is right for
 * "what is near what" and wrong for a structure: a format is read top to
 * bottom, its types depend on each other in one direction, and a layout that
 * settles wherever the springs stop puts the first field of the file wherever
 * it likes. So the boxes are placed in layers, left to right, which is the
 * shape the Kaitai diagrams have.
 *
 * **Why ELK.** The first version placed boxes with dagre and routed the arrows
 * by hand, giving each a lane of its own. It was not enough: two arrows in
 * different lanes still shared long stretches of one line, and a crossing read
 * as a join. Routing orthogonal edges without collisions is a solved problem
 * and solving it again by hand was the mistake. ELK does the placing and the
 * routing together, with a port per field row so an arrow leaves exactly at
 * its row, and hands back the corners; this draws them, and puts a small arc
 * wherever one line steps over another.
 *
 * **Why the boxes are HTML.** An arrow has to land on one row of one box, so
 * the rows have to be measurable, and a row has to be coloured the way the
 * listing colours the same field. Both are free with a `<table>` and
 * `fieldClass`, and both would be reimplemented on a canvas. The arrows are the
 * one thing a table cannot draw, so they are an SVG overlay under the boxes,
 * the way `hexlinks.ts` draws its own.
 */
import type { DiagramBox, DiagramCensus, DiagramEdge, TemplateDiagram } from "./doc.ts";
import { plan as stripPlan, type Item, type Strip } from "./strips.ts";
import { fieldClass } from "./fieldstyle.ts";
import { DIAGRAM, roleLabel } from "./strings.ts";
import { folds, shownBeforeFold } from "./fold.ts";
import {
  boxBadge,
  caseCountTitle,
  countStatus,
  countTitle,
  goTarget,
  isUnused,
  joinTitle,
  onlyUsedTitle,
  redrawIn,
  rowBadge,
} from "./diagramcounts.ts";

/**
 * How many fields a box shows before the rest are folded behind one row.
 *
 * A box is read in one glance or not at all. Past a couple of dozen rows it is
 * a column of text that happens to have a border, and every box beside it has
 * been pushed off the screen to make room for fields nobody is reading. The
 * fold is one click, and what it hides is counted rather than hinted at. A box
 * only a few rows over is shown whole: see `FOLD_SLACK`.
 */
export const ROW_CAP = 24;

/** How many cases a choice's box in a strip lists before the rest are
 *  counted. */
const CASE_CAP = 8;

/** How far the drawing may be scaled by the wheel, either way. */
const MIN_SCALE = 0.15;
const MAX_SCALE = 3;

/** Room round the whole drawing. */
const MARGIN = 44;

/**
 * What ELK is told to leave between things, in stage units.
 *
 * The numbers that matter are the two `edgeEdge` ones: two arrows running the
 * same way have to be far enough apart to be told apart at life size, and eight
 * pixels is where that starts. The rest is room for them to do it in, which is
 * why the gap between layers is wide: a layer feeding ten arrows into the next
 * needs ten lanes to put them in, and ELK will happily overlap them if it has
 * nowhere else to go.
 */
const SPACING = {
  edgeEdge: "10",
  edgeEdgeBetweenLayers: "10",
  edgeNode: "16",
  edgeNodeBetweenLayers: "20",
  nodeNode: "28",
  nodeNodeBetweenLayers: "120",
} as const;

/** The radius of the little arc a line makes where it steps over another, and
 *  how far apart two crossings have to be before both get one. */
const HOP = 4;

/** Which way the view is drawing. Remembered, because a reader who prefers one
 *  prefers it next time. */
export type Mode = "arrows" | "strips";
const MODE_KEY = "qubero.diagram.mode";

/** Room in a strip drawing: between two strips on one row, and between one row
 *  of strips and the next. The second is wide because the funnels live in it.
 *  (What separates two boxes of one strip is the stylesheet's, since that is
 *  the gap the boxes' own border-collapsing look depends on.) */
const STRIP_APART = 56;
const STRIP_DROP = 64;

/** How many strips one box may open onto before the funnels give way to a rail
 *  over the lot of them. Two funnels side by side read as two; three already
 *  cross each other, and a dozen read as hatching. */
const BUS = 3;

/** A point in stage units. */
type Pt = { x: number; y: number };

/** How far the pointer may travel and still count as a click rather than a pan,
 *  in CSS pixels of total movement. */
const PAN_SLOP = 4;

type Placed = {
  box: DiagramBox;
  el: HTMLElement;
  rows: HTMLElement[];
  x: number;
  y: number;
  w: number;
  h: number;
  /** The vertical middle of each row, in stage units, measured off the page
   *  rather than added up from heights. See `DiagramView.measure`. */
  rowMid: number[];
  /** The same for the title bar, where an arrow about the whole type lands. */
  headMid: number;
  /** Each row's middle as a distance from the top of its own box, which is what
   *  a port is placed at: a port is a point on a node, and the node has not been
   *  put anywhere yet when the layout is asked for. */
  rowTop: number[];
  /** The same for the title bar. */
  headTop: number;
};

/** One arrow as ELK routed it: the corners it turns, in order, and which edge
 *  of the diagram it stands for. */
type Route = { e: DiagramEdge; pts: Pt[] };

/** One straight piece of one arrow, for finding where two of them cross. */
type Seg = { owner: number; x1: number; y1: number; x2: number; y2: number };

/** One row of one box, as a key into the census's row counts. */
function rowKey(key: string, row: number): string {
  return `${key}\u{0}${row}`;
}

/**
 * A routed path with every corner square.
 *
 * ELK routes orthogonally, so its corners are already square to within what
 * floating point does to them; this drops the rounding and the points that
 * carry no turn. A path of exactly horizontal and vertical pieces is what the
 * hop-over below needs, and what lets the browser test say a crossing is at a
 * right angle rather than nearly one.
 */
function square(pts: Pt[]): Pt[] {
  const out: Pt[] = [];
  for (const raw of pts) {
    const p = { x: Math.round(raw.x * 2) / 2, y: Math.round(raw.y * 2) / 2 };
    const last = out[out.length - 1];
    if (last === undefined) {
      out.push(p);
      continue;
    }
    // A step that is neither one thing nor the other is squared off through the
    // corner nearest the way it was already going.
    if (Math.abs(p.x - last.x) > 0.5 && Math.abs(p.y - last.y) > 0.5) out.push({ x: p.x, y: last.y });
    if (Math.abs(p.x - last.x) > 0.5 || Math.abs(p.y - last.y) > 0.5) out.push(p);
  }
  // Three points in a line are two points.
  const trimmed: Pt[] = [];
  for (const p of out) {
    const a = trimmed[trimmed.length - 2];
    const b = trimmed[trimmed.length - 1];
    if (a !== undefined && b !== undefined) {
      const straight =
        (Math.abs(a.x - b.x) < 0.5 && Math.abs(b.x - p.x) < 0.5) || (Math.abs(a.y - b.y) < 0.5 && Math.abs(b.y - p.y) < 0.5);
      if (straight) trimmed.pop();
    }
    trimmed.push(p);
  }
  return trimmed;
}

/**
 * One arrow as an SVG path, with a small arc wherever it steps over another.
 *
 * The hop goes on the horizontal piece, always, so that two arrows crossing
 * never both hop and the reader learns one rule. Its own verticals are not
 * stepped over, and neither is a crossing so near a corner that the arc would
 * swallow the turn.
 */
function hopped(pts: Pt[], verticals: Seg[], self: number): string {
  let d = "";
  for (let k = 0; k < pts.length; k++) {
    const p = pts[k];
    if (p === undefined) continue;
    if (k === 0) {
      d = `M ${p.x} ${p.y}`;
      continue;
    }
    const q = pts[k - 1];
    if (q === undefined) continue;
    if (Math.abs(p.y - q.y) > 0.5) {
      d += ` L ${p.x} ${p.y}`;
      continue;
    }
    // Rightwards or leftwards, the crossings are taken in the order they are
    // met, and only the ones with room for an arc on both sides.
    const lo = Math.min(q.x, p.x) + 1;
    const hi = Math.max(q.x, p.x) - 1;
    const over = verticals
      .filter((v) => v.owner !== self && v.x1 > lo && v.x1 < hi && v.y1 < p.y - 0.5 && v.y2 > p.y + 0.5)
      .map((v) => v.x1)
      .sort((a, b) => a - b);
    const going = p.x > q.x ? 1 : -1;
    const order = going === 1 ? over : over.reverse();
    let last = q.x;
    for (const x of order) {
      // Two crossings closer together than one arc is wide are one arc: two
      // overlapping semicircles are a scribble, and what the mark is for is
      // saying "these do not join", which one of them says for both.
      if (Math.abs(x - last) < HOP * 2 && last !== q.x) continue;
      d += ` L ${x - HOP * going} ${p.y}`;
      // Bulging upwards whichever way the line is travelling, which in a
      // y-down space is a clockwise sweep going right and anticlockwise going
      // left.
      d += ` A ${HOP} ${HOP} 0 0 ${going === 1 ? 1 : 0} ${x + HOP * going} ${p.y}`;
      last = x;
    }
    d += ` L ${p.x} ${p.y}`;
  }
  return d;
}

/**
 * Nudge apart the runs that ELK left lying on top of each other.
 *
 * Two rows in two different boxes can sit at the same y, and an arrow leaving
 * each of them runs horizontally at that y; where their x ranges overlap the
 * two are one line on the page, going to different places. ELK will not fix
 * that, because as far as it is concerned they are in different layers and
 * never compared.
 *
 * This is not the lane allocator come back. It decides no routes: it takes the
 * ones ELK decided and moves a middle piece of one of them a few pixels where
 * it would otherwise be invisible under another. The first and last pieces are
 * never moved, since those are what hold an arrow to its row.
 */
function separate(routes: Route[]): void {
  const step = 4;
  for (const axis of ["h", "v"] as const) {
    // Every middle piece, by the line it sits on.
    const lanes = new Map<number, { r: number; k: number; lo: number; hi: number }[]>();
    for (const [r, route] of routes.entries()) {
      for (let k = 1; k < route.pts.length - 1; k++) {
        const a = route.pts[k - 1];
        const b = route.pts[k];
        if (a === undefined || b === undefined) continue;
        if (k === 1 || k === route.pts.length - 1) continue;
        const horizontal = Math.abs(a.y - b.y) < 0.5;
        if ((axis === "h") !== horizontal) continue;
        const at = Math.round(horizontal ? a.y : a.x);
        const lo = horizontal ? Math.min(a.x, b.x) : Math.min(a.y, b.y);
        const hi = horizontal ? Math.max(a.x, b.x) : Math.max(a.y, b.y);
        const list = lanes.get(at);
        if (list === undefined) lanes.set(at, [{ r, k, lo, hi }]);
        else list.push({ r, k, lo, hi });
      }
    }
    for (const list of lanes.values()) {
      if (list.length < 2) continue;
      list.sort((a, b) => a.lo - b.lo);
      // Each piece is pushed off the line by as many steps as there are pieces
      // already overlapping it, so a lane of four becomes four lines.
      const put: { lo: number; hi: number; at: number }[] = [];
      for (const seg of list) {
        let shift = 0;
        for (let tries = 0; tries < 8; tries++) {
          const clash = put.some((q) => q.at === shift && q.lo < seg.hi - 1 && seg.lo < q.hi - 1);
          if (!clash) break;
          shift = shift > 0 ? -shift : -shift + 1;
        }
        put.push({ lo: seg.lo, hi: seg.hi, at: shift });
        if (shift === 0) continue;
        const route = routes[seg.r];
        const a = route?.pts[seg.k - 1];
        const b = route?.pts[seg.k];
        if (a === undefined || b === undefined) continue;
        if (axis === "h") {
          a.y += shift * step;
          b.y += shift * step;
        } else {
          a.x += shift * step;
          b.x += shift * step;
        }
      }
    }
  }
}

const SVG_NS = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(name: K, attrs: Record<string, string> = {}): SVGElementTagNameMap[K] {
  const node = document.createElementNS(SVG_NS, name);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  return node;
}

export class DiagramView {
  readonly el: HTMLElement;
  /** What the wheel and the drag act on. Clips; the stage inside it moves. */
  private readonly board: HTMLElement;
  /** Everything that is drawn, moved and scaled as one. */
  private readonly stage: HTMLElement;
  private readonly lines: SVGSVGElement;
  private readonly note: HTMLElement;
  private diagram: TemplateDiagram | null = null;
  /** The line over the drawing, kept rather than read back off the element: a
   *  relayout rebuilds the note, and reading it back would fold the last note
   *  into the next one. */
  private about: string = DIAGRAM.noTemplate;
  private placed: Placed[] = [];
  /** Boxes whose folded fields the reader has asked to see. Kept across a
   *  relayout, cleared with the diagram, since the indexes are its own. */
  private opened = new Set<number>();
  private scale = 1;
  private tx = 0;
  private ty = 0;
  /** True once the pointer has moved far enough for the gesture to be a pan
   *  rather than a click. A drag that starts and ends inside one row still
   *  fires a click, and a reader who dragged the picture did not ask to be
   *  taken to a field. */
  private dragged = false;
  /** The whole drawing's extent in stage units, for `fit`. */
  private extent = { w: 0, h: 0 };
  /** Whether the one rebuild that waits for the real fonts has been booked. */
  private fontsSettled = false;
  /** A pending re-measure, so a wheel spun through twenty notches redraws the
   *  arrows once rather than twenty times. */
  private pending = 0;
  /** What the open file holds, against the format the boxes are drawn from.
   *  Null until it has been counted, which is not the same as counted and
   *  empty: before it arrives no box is muted and no badge is shown, because
   *  the honest thing to say about a count nobody has taken is nothing. */
  private census: DiagramCensus | null = null;
  /** How many of each box and row the file holds, by the core's own key. */
  private boxCount = new Map<string, number>();
  private rowCount = new Map<string, number>();
  /** Whether the reader has asked for only the types this file has. */
  private onlyUsed = false;
  /** The arrows as the layout routed them, kept so a zoom can redraw them
   *  without laying the boxes out again. */
  private routes: Route[] = [];
  /** Which build is the current one. The layout is asked for over a promise, so
   *  a second build started while the first is still running would otherwise
   *  place boxes from a diagram nobody is looking at any more. */
  private generation = 0;
  /** How long the last layout took, in milliseconds. Read by the browser test
   *  rather than shown: it is a fact about the machine, not about the file. */
  layoutMs = 0;
  /** The layout engine, once it has been fetched. */
  private elk: { layout: (g: unknown) => Promise<unknown> } | null = null;
  /** Which way the view is drawing. */
  private mode: Mode = "arrows";
  private readonly modeBtn: HTMLSelectElement;
  /** The strips on screen, in step with the plan, for the funnels to measure
   *  against once they are laid out. */
  private laid: { strip: Strip; el: HTMLElement; boxes: HTMLElement[] }[] = [];
  private readonly onlyBtn: HTMLInputElement;
  private readonly onlyLabel: HTMLLabelElement;
  /** What the count is doing, at the right of the toolbar, and the button that
   *  carries a stopped count on. */
  private readonly status: HTMLElement;
  private readonly keepBtn: HTMLButtonElement;
  /** Whether a count has been asked for, so the toolbar can say it is counting
   *  before the first answer is drawn. */
  private counting = false;
  /** When the count began and when the drawing was last laid out for it, and
   *  the layout booked for later, so a running count redraws once a second
   *  rather than once a step. See `redrawIn`. */
  private countingSince = 0;
  private lastDrawn = 0;
  private redrawTimer = 0;
  /** The row a single click marked, as `box:row`, kept across a rebuild. */
  private selected: string | null = null;

  /** The reader double-clicked a field of the first box. `main.ts` puts the
   *  cursor on it where the open file has one. */
  onPick: (box: number, row: number) => void = () => {};

  /** The reader double-clicked a box title or a row and wants the first one
   *  this file holds. `main.ts` has the file and does the going. */
  onGo: (path: readonly number[], space: number) => void = () => {};

  /** The reader asked a stopped count to carry on. */
  onKeepCounting: () => void = () => {};

  /**
   * Whether a double click on this row would reach the open file.
   *
   * Asked of every row as the box is built, because a row that does nothing
   * must not say it does: a pointer and a tooltip promising to move the cursor,
   * on seventy of a format's seventy-two types, is seventy-one lies. Only
   * `main.ts` knows, since only it has the file.
   */
  canPick: (box: number, row: number) => boolean = () => false;

  constructor() {
    this.el = document.createElement("section");
    this.el.className = "diagramview";
    this.el.hidden = true;
    this.el.tabIndex = 0;
    this.el.setAttribute("aria-label", DIAGRAM.regionLabel);

    this.note = document.createElement("div");
    this.note.className = "dv-note";

    const bar = document.createElement("div");
    bar.className = "dv-bar";
    const fit = document.createElement("button");
    fit.type = "button";
    fit.className = "dv-fit";
    fit.textContent = DIAGRAM.fit;
    fit.title = DIAGRAM.fitTitle;
    fit.addEventListener("click", () => this.fit());
    // Off by default: the view's job is the format, and narrowing it to one
    // file is a thing the reader asks for rather than a thing that happens to
    // them.
    const only = document.createElement("label");
    only.className = "dv-only";
    only.title = DIAGRAM.onlyUsedTitle;
    this.onlyLabel = only;
    this.onlyBtn = document.createElement("input");
    this.onlyBtn.type = "checkbox";
    this.onlyBtn.addEventListener("change", () => {
      this.onlyUsed = this.onlyBtn.checked;
      void this.build();
    });
    only.append(this.onlyBtn, document.createTextNode(DIAGRAM.onlyUsed));
    // Two pictures of one format, and the reader picks. Arrows say what decides
    // what; strips say what comes after what. Neither is the other's summary.
    this.modeBtn = document.createElement("select");
    this.modeBtn.className = "dv-mode";
    this.modeBtn.title = DIAGRAM.modeTitle;
    this.modeBtn.setAttribute("aria-label", DIAGRAM.modeLabel);
    for (const [value, text] of [
      ["arrows", DIAGRAM.modeArrows],
      ["strips", DIAGRAM.modeStrips],
    ] as const) {
      const opt = document.createElement("option");
      opt.value = value;
      opt.textContent = text;
      this.modeBtn.append(opt);
    }
    const saved = localStorage.getItem(MODE_KEY);
    this.mode = saved === "arrows" ? "arrows" : "strips";
    this.modeBtn.value = this.mode;
    this.modeBtn.addEventListener("change", () => {
      this.mode = this.modeBtn.value === "strips" ? "strips" : "arrows";
      localStorage.setItem(MODE_KEY, this.mode);
      void this.build().then(() => this.home());
    });
    // What the count is doing, in the toolbar rather than on a line of its own
    // under it: a line that comes and goes moves the drawing as it does, and
    // the button that carries a stopped count on belongs beside what it acts
    // on. Every badge already says the count is a floor; this says why.
    this.status = document.createElement("span");
    this.status.className = "dv-status";
    this.status.setAttribute("role", "status");
    this.keepBtn = document.createElement("button");
    this.keepBtn.type = "button";
    this.keepBtn.className = "dv-keep";
    this.keepBtn.textContent = DIAGRAM.keepCounting;
    this.keepBtn.title = DIAGRAM.keepCountingTitle;
    this.keepBtn.hidden = true;
    this.keepBtn.addEventListener("click", () => this.onKeepCounting());
    const counts = document.createElement("span");
    counts.className = "dv-counts";
    counts.append(this.status, this.keepBtn);
    bar.append(fit, this.modeBtn, only, counts);

    this.board = document.createElement("div");
    this.board.className = "dv-board";
    this.stage = document.createElement("div");
    this.stage.className = "dv-stage";
    this.lines = svg("svg", { class: "dv-lines" });
    this.stage.append(this.lines);
    this.board.append(this.stage);
    this.el.append(this.note, bar, this.board);
    this.bindPanZoom();
    // A press on the picture that lands on no row lets go of the marked one,
    // and so does Escape, the way a selection is let go of anywhere else.
    this.board.addEventListener("click", (ev) => {
      if (this.dragged) return;
      if (ev.target instanceof Element && ev.target.closest(".dv-row, .dv-sbox") !== null) return;
      this.select(null);
    });
    this.el.addEventListener("keydown", (ev) => {
      if (ev.key === "Escape" && this.selected !== null) this.select(null);
    });
  }

  /**
   * What the open file holds, laid over the picture of what the format can.
   *
   * Kept apart from `show` because the two change at different times: the
   * drawing changes when the template does, the counts whenever the file does.
   * Null clears them, which is what "not counted yet" means and is not the same
   * as a census that found nothing: before one arrives no box is muted and no
   * badge is shown, since the honest thing to say about a count nobody has
   * taken is nothing.
   */
  setCensus(c: DiagramCensus | null): void {
    const now = performance.now();
    if (c !== null && c.state !== "done" && !this.counting) this.countingSince = now;
    this.counting = c !== null && c.state !== "done";
    // The toolbar's line moves with every step; the drawing only as often as a
    // reader can follow, since each layout moves every box a badge widens.
    this.showStatus(c);
    const wait = redrawIn(this.census, c, now, this.lastDrawn, this.countingSince);
    window.clearTimeout(this.redrawTimer);
    this.redrawTimer = 0;
    if (wait === null) return;
    const apply = (): void => {
      this.redrawTimer = 0;
      this.census = c;
      this.boxCount = new Map(c?.boxes.map((b) => [b.key, b.count]) ?? []);
      this.rowCount = new Map(c?.rows.map((r) => [rowKey(r.key, r.row), r.count]) ?? []);
      this.lastDrawn = performance.now();
      if (this.diagram !== null) void this.build();
    };
    if (wait === 0) apply();
    else this.redrawTimer = window.setTimeout(apply, wait);
  }

  /** What the count is doing, said in the toolbar. Asked of the newest count,
   *  which may be ahead of the one the drawing shows. */
  private showStatus(c: DiagramCensus | null): void {
    const s = countStatus(c, this.counting);
    this.status.textContent = s.text;
    this.keepBtn.hidden = !s.keep;
  }

  /** Mark one row as the one the reader clicked, or none. */
  private select(key: string | null): void {
    this.selected = key;
    for (const el of this.stage.querySelectorAll(".is-selected")) el.classList.remove("is-selected");
    if (key === null) return;
    for (const el of this.stage.querySelectorAll(`[data-row="${key}"]`)) el.classList.add("is-selected");
  }

  /** Put a diagram on screen, replacing whatever was there. */
  show(d: TemplateDiagram | null, formatName: string): void {
    this.diagram = d;
    this.opened.clear();
    this.about = d === null ? DIAGRAM.noTemplate : DIAGRAM.about(formatName);
    this.note.classList.toggle("is-warn", d === null);
    this.build();
    // A new diagram starts on the root type at life size: that is where the
    // format starts, and life size is the only scale its rows can be read at.
    // Fitting a format of seventy types into a window would land the reader on
    // a wall of four-point text. `Fit` is one click away for the reader who
    // wants the shape of the whole thing first.
    this.home();
  }

  /** Measure, place and draw again, keeping wherever the reader had panned to.
   *  Called when the view comes back on screen, and when a fold opens. */
  relayout(): void {
    if (this.diagram === null) return;
    void this.build();
  }

  private async build(): Promise<void> {
    const gen = ++this.generation;
    for (const p of this.placed) p.el.remove();
    this.placed = [];
    for (const l of this.laid) l.el.remove();
    this.laid = [];
    this.lines.replaceChildren();
    this.note.replaceChildren(this.about);
    const d = this.diagram;
    if (d === null) return;
    if (d.omitted > 0) {
      const omitted = document.createElement("span");
      omitted.className = "dv-omitted";
      omitted.textContent = DIAGRAM.omitted(d.omitted);
      this.note.append(DIAGRAM.noteJoin, omitted);
    }
    // The toggle hides what has not been found. While the count runs that is
    // also what has not been found yet, and the hover says so rather than
    // letting "not found" stand for "not there".
    this.onlyBtn.disabled = this.census === null;
    this.onlyLabel.title = onlyUsedTitle(this.census);
    // Which boxes are drawn at all. With the toggle off, all of them: the view
    // is a picture of the format. With it on, the ones this file has, and the
    // edges between two of those; an edge to a box that is not there would
    // point off the drawing.
    const shown = d.types.map((b, i) => !this.onlyUsed || this.census === null || (this.boxCount.get(b.key) ?? 0) > 0 || i === 0);
    if (this.mode === "strips") {
      this.buildStrips(d, shown);
    } else {
      for (const [i, box] of d.types.entries()) {
        const p = this.buildBox(i, box);
        if (!(shown[i] ?? true)) p.el.remove();
        this.placed.push(p);
      }
      // The transform first, then the measuring, then the layout: `measure`
      // divides out the scale that is in force, and the scale in force is
      // whatever `apply` last wrote. The layout is handed measurements, so it
      // cannot run before them.
      this.apply();
      this.measure();
      await this.layout(d, shown, gen);
      if (gen !== this.generation) return;
      this.drawRoutes();
    }
    // A box measured in a fallback font is the wrong height, and every arrow
    // into it lands on the wrong row. Once, when the real fonts arrive.
    if (!this.fontsSettled) {
      this.fontsSettled = true;
      void document.fonts.ready.then(() => {
        if (this.diagram !== null) void this.build();
      });
    }
  }

  /** Measure and draw the arrows again, without moving a box.
   *
   *  What changes them is the zoom, which is divided out of every measurement
   *  and so should change nothing, and does not quite: a row measured at a
   *  tenth of life size rounds differently from one measured at life size.
   *  Cheap enough to do rather than reason about. */
  private redrawEdges(): void {
    if (this.diagram === null || this.placed.length === 0) return;
    this.lines.replaceChildren();
    this.drawRoutes();
  }

  /** How many of a thing this file holds, small and beside its name. */
  private badge(text: string, title: string): HTMLElement {
    const el = document.createElement("span");
    el.className = "dv-count";
    el.textContent = text;
    el.title = title;
    return el;
  }

  /**
   * What a double click on this does, and the one handler that does it.
   *
   * Double rather than single because going to the hex view switches the
   * reader's view, and the app asks for a double click or a button for that: a
   * reader who clicked a row to read it should not lose the drawing. A single
   * click marks the row instead (see `selectable`).
   *
   * One handler and one title however many reasons a row has to go somewhere:
   * a field of the file's first structure goes to itself, anything the count
   * found goes to the first one found, and a first one inside an unpacked
   * stream goes nowhere and says why, since its offsets are the stream's and
   * the hex cursor cannot be put on it. `said` is what the hover says before
   * that.
   */
  private offerGo(el: HTMLElement, pick: { box: number; row: number } | null, first: { first_path: number[]; space: number } | undefined, said: string): void {
    const target = goTarget(pick !== null, first);
    if (target === null) {
      el.title = said;
      return;
    }
    if (target.kind === "stream") {
      el.title = joinTitle(said, DIAGRAM.inStream);
      return;
    }
    el.classList.add("is-goable");
    el.title = joinTitle(said, DIAGRAM.goTitle);
    el.addEventListener("dblclick", (ev) => {
      ev.preventDefault();
      if (this.dragged) return;
      if (target.kind === "pick" && pick !== null) this.onPick(pick.box, pick.row);
      else if (target.kind === "path") this.onGo(target.path, 0);
    });
  }

  /** A single click marks the row, and does nothing else: no cursor moves and
   *  no view changes, so reading a box never costs the reader their place. */
  private selectable(el: HTMLElement, box: number, row: number): void {
    const key = `${box}:${row}`;
    el.dataset["row"] = key;
    if (this.selected === key) el.classList.add("is-selected");
    // Marks rather than toggles: a double click is two clicks first, and a
    // toggle would leave the row it went from unmarked.
    el.addEventListener("click", () => {
      if (this.dragged) return;
      this.select(key);
    });
  }

  /** One type as a table. The rows carry the listing's field colours, so a
   *  length is the same colour here as it is in every other view. */
  private buildBox(index: number, box: DiagramBox): Placed {
    const el = document.createElement("div");
    el.className = "dv-box";
    if (box.kind === "switch") el.classList.add("is-switch");
    const head = document.createElement("div");
    head.className = "dv-box-name";
    const label = document.createElement("span");
    label.className = "dv-box-label";
    label.textContent = box.name;
    // What the format calls the type on the box; how the reader would get to it
    // on hover. The path can run to six steps in a format whose types are
    // written inside switch cases, and it is the second question.
    label.title = DIAGRAM.boxPath(box.path);
    head.append(label);
    // How many of this type the open file holds. Only once a census has been
    // taken: before that the drawing says nothing about the file, which is
    // what it knows.
    const c = this.census;
    const has = c === null ? 0 : (this.boxCount.get(box.key) ?? 0);
    if (c !== null) {
      el.classList.toggle("is-unused", isUnused(has, c.state));
      if (isUnused(has, c.state)) el.title = DIAGRAM.unusedTitle;
      const said = countTitle(has, box.name, c);
      const text = boxBadge(has, c.state);
      if (text !== null) head.append(this.badge(text, said));
      this.offerGo(head, null, c.boxes.find((b) => b.key === box.key), "");
    }
    // No tag beside the name. A switch says what it is in its own title
    // (`switch on class`) and in its heading colour; whether a structure has a
    // name of its own in the template is on all but a couple of boxes in most
    // formats, so saying it on each of them is a word repeated eleven times
    // that the reader cannot act on. The path on hover carries it.
    const table = document.createElement("table");
    table.className = "dv-table";
    const body = document.createElement("tbody");
    const open = this.opened.has(index);
    const shown = open ? box.rows.length : shownBeforeFold(box.rows.length, ROW_CAP);
    const rows: HTMLElement[] = [];
    for (let i = 0; i < shown; i++) {
      const r = box.rows[i];
      if (r === undefined) continue;
      const tr = document.createElement("tr");
      tr.className = `dv-row ${fieldClass(r.kind)}`;
      // A case has no position and no size of its own: what it has is a value
      // and the type that value picks, which is what the two columns say. A
      // field has all four, in the order a reader scans them: where, how long,
      // what, and what it is called.
      const cells =
        box.kind === "switch"
          ? ([
              ["dv-case", r.name, DIAGRAM.column.caseValue],
              ["dv-type", r.type_text, DIAGRAM.column.type],
            ] as const)
          : ([
              ["dv-pos", r.pos_text, DIAGRAM.column.pos],
              ["dv-size", r.size_text, DIAGRAM.column.size],
              ["dv-type", r.type_text, DIAGRAM.column.type],
              ["dv-name", r.name, DIAGRAM.column.name],
            ] as const);
      for (const [cls, text, label] of cells) {
        const td = document.createElement("td");
        td.className = cls;
        td.textContent = text;
        // The columns hold expressions that can run long, so the cell is
        // clipped and says the whole of itself on hover rather than pushing
        // every box after it further right.
        if (text !== "") td.title = `${label}: ${text}`;
        tr.append(td);
      }
      // The same for one field, over every node of this type the file holds,
      // badged only where it differs from the box: a field that is there once
      // per structure says nothing the box's own count has not.
      const pick = box.kind !== "switch" && this.canPick(index, i) ? { box: index, row: i } : null;
      let said = "";
      if (c !== null) {
        const held = this.rowCount.get(rowKey(box.key, i)) ?? 0;
        tr.classList.toggle("is-unused", isUnused(held, c.state));
        said = box.kind === "switch" ? caseCountTitle(r.name, held, c) : countTitle(held, r.name, c);
        const text = rowBadge(held, has, c.state);
        const cell = tr.lastElementChild;
        if (text !== null && cell !== null) cell.append(this.badge(text, said));
      }
      this.offerGo(tr, pick, c?.rows.find((x) => x.key === box.key && x.row === i), said);
      // The hover is on the row, and every cell has one of its own that would
      // hide it: the last cell, where a badge would be, says both.
      const last = tr.lastElementChild;
      if (last instanceof HTMLElement && tr.title !== "") last.title = joinTitle(last.title, tr.title);
      this.selectable(tr, index, i);
      body.append(tr);
      rows.push(tr);
    }
    // What the fold hides, counted. The row is not a field, so no arrow ever
    // lands on it and it is not in `rows`.
    if (folds(box.rows.length, ROW_CAP)) {
      const tr = document.createElement("tr");
      tr.className = "dv-more";
      const td = document.createElement("td");
      const cases = box.kind === "switch";
      td.colSpan = cases ? 2 : 4;
      const hidden = box.rows.length - shownBeforeFold(box.rows.length, ROW_CAP);
      td.textContent = open ? (cases ? DIAGRAM.lessCases : DIAGRAM.less) : cases ? DIAGRAM.moreCases(hidden) : DIAGRAM.more(hidden);
      td.title = open ? td.textContent : cases ? DIAGRAM.moreCasesTitle : DIAGRAM.moreTitle;
      tr.append(td);
      tr.addEventListener("click", () => {
        if (this.dragged) return;
        if (open) this.opened.delete(index);
        else this.opened.add(index);
        void this.build();
      });
      body.append(tr);
    }
    table.append(body);
    el.append(head, table);
    this.stage.append(el);
    return { box, el, rows, x: 0, y: 0, w: el.offsetWidth, h: el.offsetHeight, rowMid: [], headMid: 0, rowTop: [], headTop: 0 };
  }

  /**
   * Where every box and every row of it is, measured off the page.
   *
   * Two frames, because two things ask. `w`/`h` and `rowTop` are a box's own
   * measurements and are what the layout is handed: a port is a point on a node
   * and the node has not been put anywhere when the layout is asked for.
   * `rowMid` is the same rows once the box has been placed, in the stage's
   * frame, which is what everything after the layout reads.
   *
   * Never added up from heights. An arrow that leaves two rows above the row it
   * is about is worse than no arrow, because it says a connection the format
   * does not have, and arithmetic over heights is one unexamined assumption
   * away from that. `getBoundingClientRect` is what the browser actually laid
   * out; it comes back with the stage's transform applied, so it is taken
   * against the box's own corner and divided by the scale in force.
   */
  private measure(): void {
    const k = this.scale === 0 ? 1 : this.scale;
    for (const p of this.placed) {
      const box = p.el.getBoundingClientRect();
      p.w = box.width / k;
      p.h = box.height / k;
      const from = (el: Element): number => {
        const r = el.getBoundingClientRect();
        return (r.top + r.height / 2 - box.top) / k;
      };
      p.rowTop = p.rows.map(from);
      const head = p.el.querySelector(".dv-box-name");
      p.headTop = head === null ? Math.min(18, p.h / 2) : from(head);
      p.rowMid = p.rowTop.map((y) => p.y + y);
      p.headMid = p.y + p.headTop;
    }
  }

  /** Where one end of an arrow sits: the middle of a row's edge, or the title
   *  bar for an arrow that is about the whole type. */
  private port(box: number, row: number | undefined, side: "left" | "right"): Pt | null {
    const p = this.placed[box];
    if (p === undefined) return null;
    const x = side === "right" ? p.x + p.w : p.x;
    // A row behind a fold has no place of its own, so the arrow lands on the
    // box. Better a true arrow to the type than a false one to a row that is
    // not being shown.
    if (row === undefined) return { x, y: p.headMid };
    const y = p.rowMid[row];
    if (y === undefined) return { x, y: p.headMid };
    return { x, y };
  }

  /**
   * The format as strips: a row of boxes per type, its fields in file order,
   * and each type that a field holds drawn on the row below and joined to it.
   *
   * No graph library and nothing to solve. The tree is the plan's own: the root
   * on top, one row per depth, strips left to right in the order their first
   * use appears. What takes the work is the joining, and that is two dashed
   * lines from a box's bottom corners to the child strip's top corners — a
   * funnel, which says "this box opens into that row" without an arrowhead's
   * claim about direction.
   */
  private buildStrips(d: TemplateDiagram, shown: boolean[]): void {
    this.laid = [];
    const made = stripPlan(d, ROW_CAP, (box) => shown[box] !== false);
    if (made.omitted > 0) {
      const note = document.createElement("span");
      note.className = "dv-omitted";
      note.textContent = DIAGRAM.omitted(made.omitted);
      this.note.append(" ", note);
    }
    for (const strip of made.strips) this.laid.push(this.buildStrip(d, strip));
    this.apply();

    // Measured off the page rather than guessed at: a box is as wide as its
    // words, so only the page can say how wide a strip is. Every strip is
    // measured before any is moved, because moving one would make the next
    // measurement wait on a fresh layout.
    const k = this.scale === 0 ? 1 : this.scale;
    const size = this.laid.map((l) => {
      const r = l.el.getBoundingClientRect();
      return { w: r.width / k, h: r.height / k };
    });

    // A strip hangs under the box that opens it, so the plan is already a tree
    // and the layout is the old tidy one: work out how wide each subtree is,
    // then give each child its share of that width and centre the parent over
    // them. Laying each row out left to right instead would be simpler and
    // would draw every funnel across the whole page.
    const kids: number[][] = this.laid.map(() => []);
    for (const [i, l] of this.laid.entries()) {
      const up = l.strip.from?.strip;
      if (up !== undefined && up !== i && kids[up] !== undefined) kids[up]?.push(i);
    }
    const span: number[] = new Array<number>(this.laid.length).fill(0);
    for (let i = this.laid.length - 1; i >= 0; i--) {
      let below = 0;
      for (const c of kids[i] ?? []) below += (span[c] ?? 0) + STRIP_APART;
      span[i] = Math.max(size[i]?.w ?? 0, Math.max(0, below - STRIP_APART));
    }
    // One height per row, so the strips of a row line up and the funnels below
    // them all start from the same place.
    let deepest = 0;
    for (const l of this.laid) deepest = Math.max(deepest, l.strip.depth);
    const tall: number[] = new Array<number>(deepest + 1).fill(0);
    for (const [i, l] of this.laid.entries()) {
      const at = l.strip.depth;
      tall[at] = Math.max(tall[at] ?? 0, size[i]?.h ?? 0);
    }
    const top: number[] = [];
    let y = MARGIN;
    for (let i = 0; i <= deepest; i++) {
      top.push(y);
      y += (tall[i] ?? 0) + STRIP_DROP;
    }
    let right = 0;
    const place = (i: number, left: number): void => {
      const l = this.laid[i];
      if (l === undefined) return;
      const w = size[i]?.w ?? 0;
      const x = left + Math.max(0, ((span[i] ?? 0) - w) / 2);
      l.el.style.left = `${Math.round(x)}px`;
      l.el.style.top = `${Math.round(top[l.strip.depth] ?? MARGIN)}px`;
      right = Math.max(right, x + w);
      let at = left;
      for (const c of kids[i] ?? []) {
        place(c, at);
        at += (span[c] ?? 0) + STRIP_APART;
      }
    };
    // Every strip the plan made hangs under another except the root, but a plan
    // cut off at the cap can leave one behind; those start a tree of their own
    // rather than going undrawn.
    let loose = MARGIN;
    for (const [i, l] of this.laid.entries()) {
      const up = l.strip.from?.strip;
      if (up !== undefined && up !== i && this.laid[up] !== undefined) continue;
      place(i, loose);
      loose += (span[i] ?? 0) + STRIP_APART;
    }
    this.extent = { w: right + MARGIN, h: y + MARGIN };
    this.stage.style.width = `${this.extent.w}px`;
    this.stage.style.height = `${this.extent.h}px`;
    this.lines.setAttribute("width", String(this.extent.w));
    this.lines.setAttribute("height", String(this.extent.h));
    this.lines.setAttribute("viewBox", `0 0 ${this.extent.w} ${this.extent.h}`);
    this.drawFunnels();
  }

  /** One type as a row of boxes. */
  private buildStrip(d: TemplateDiagram, strip: Strip): { strip: Strip; el: HTMLElement; boxes: HTMLElement[] } {
    const el = document.createElement("div");
    el.className = "dv-strip";
    const head = document.createElement("div");
    head.className = "dv-strip-name";
    const label = document.createElement("span");
    label.className = "dv-box-label";
    label.textContent = strip.name;
    label.title = DIAGRAM.boxPath(strip.path);
    head.append(label);
    const c = this.census;
    if (c !== null) {
      const has = this.boxCount.get(strip.key) ?? 0;
      el.classList.toggle("is-unused", isUnused(has, c.state));
      if (isUnused(has, c.state)) el.title = DIAGRAM.unusedTitle;
      const said = countTitle(has, strip.name, c);
      const text = boxBadge(has, c.state);
      if (text !== null) head.append(this.badge(text, said));
      this.offerGo(head, null, c.boxes.find((b) => b.key === strip.key), "");
    }
    const line = document.createElement("div");
    line.className = "dv-strip-row";
    const boxes: HTMLElement[] = [];
    for (const item of strip.items) boxes.push(this.buildItem(d, strip, item, line));
    el.append(head, line);
    this.stage.append(el);
    return { strip, el, boxes };
  }

  /** One box of one strip: the field's name, and under it what it is. */
  private buildItem(d: TemplateDiagram, strip: Strip, item: Item, into: HTMLElement): HTMLElement {
    const el = document.createElement("div");
    if (item.kind === "band") {
      // The stretch a run stands for. Dotted rather than drawn, because how
      // many there are is a fact about the file and the picture is of the
      // format; the count is on the box before it.
      el.className = "dv-band";
      el.textContent = "· · ·";
      into.append(el);
      return el;
    }
    el.className = `dv-sbox ${fieldClass(item.fieldKind)}`;
    if (item.optional) el.classList.add("is-optional");
    if (item.kind === "more") {
      el.classList.add("is-more");
      el.textContent = DIAGRAM.more(Number(item.name));
      el.title = DIAGRAM.moreTitle;
      into.append(el);
      return el;
    }
    const name = document.createElement("div");
    name.className = "dv-sname";
    name.textContent = item.name;
    if (item.placed) {
      // A field whose contents are somewhere else entirely. The mark says so
      // in one character; where it points is the line out of the box.
      const mark = document.createElement("span");
      mark.className = "dv-at";
      mark.textContent = "@";
      mark.title = DIAGRAM.placedTitle;
      name.append(mark);
    }
    el.append(name);
    // What it is, then how long it runs, then what decides whether it is there
    // at all: the same words the other mode's columns use, stacked because a
    // box is taller than it is wide.
    for (const [cls, text] of [
      ["dv-snote", item.kind === "last" ? DIAGRAM.runLast : item.note],
      ["dv-ssize", item.kind === "last" ? "" : item.size],
    ] as const) {
      if (text === "") continue;
      const line = document.createElement("div");
      line.className = cls;
      line.textContent = item.optional && cls === "dv-ssize" ? DIAGRAM.when(text) : text;
      el.append(line);
    }
    if (item.cases.length > 0) {
      const list = document.createElement("div");
      list.className = "dv-scases";
      list.title = DIAGRAM.caseListTitle;
      const listed = shownBeforeFold(item.cases.length, CASE_CAP);
      for (const one of item.cases.slice(0, listed)) {
        const line = document.createElement("div");
        line.textContent = one;
        list.append(line);
      }
      if (listed < item.cases.length) {
        const rest = document.createElement("div");
        rest.className = "dv-snote";
        rest.textContent = DIAGRAM.moreCases(item.cases.length - listed);
        list.append(rest);
      }
      el.append(list);
    }
    // The count again, on the field this time, and badged only where it
    // differs from the strip's.
    const c = this.census;
    const pick = item.row >= 0 && this.canPick(strip.box, item.row) ? { box: strip.box, row: item.row } : null;
    let said = "";
    if (c !== null && item.row >= 0) {
      const held = this.rowCount.get(rowKey(strip.key, item.row)) ?? 0;
      el.classList.toggle("is-unused", isUnused(held, c.state));
      said = countTitle(held, item.name, c);
      const text = rowBadge(held, this.boxCount.get(strip.key) ?? 0, c.state);
      if (text !== null) name.append(this.badge(text, said));
    }
    if (item.row >= 0) {
      this.offerGo(el, pick, c?.rows.find((x) => x.key === strip.key && x.row === item.row), said);
      this.selectable(el, strip.box, item.row);
    }
    into.append(el);
    return el;
  }

  /**
   * The dashed joins: a funnel from a box down to the strip it opens onto, and
   * a single line to a strip drawn somewhere else.
   *
   * Dashed throughout, and never with an arrowhead. An arrow would be a claim
   * about which way something is read; what these say is only "this is what is
   * in there", which is the same fact whichever end you start from.
   */
  private drawFunnels(): void {
    const k = this.scale === 0 ? 1 : this.scale;
    const stage = this.stage.getBoundingClientRect();
    const at = (el: Element): { x: number; y: number; w: number; h: number } => {
      const r = el.getBoundingClientRect();
      return { x: (r.left - stage.left) / k, y: (r.top - stage.top) / k, w: r.width / k, h: r.height / k };
    };
    for (const l of this.laid) {
      for (const [i, item] of l.strip.items.entries()) {
        const from = l.boxes[i];
        if (from === undefined) continue;
        const a = at(from);
        const foot = { x: a.x, y: a.y + a.h, w: a.w };
        // A mention is one line to the title of the strip that holds it, since
        // the strip itself is drawn under whichever box got to it first.
        for (const link of item.links) {
          if (!link.reference) continue;
          const to = this.laid[link.strip];
          if (to === undefined) continue;
          const b = at(to.el);
          const g = svg("g", { class: "dv-funnel is-reference" });
          g.append(svg("path", { d: `M ${foot.x + foot.w / 2} ${foot.y} L ${b.x + Math.min(40, b.w / 2)} ${b.y}` }));
          this.lines.append(g);
        }
        const own = item.links
          .filter((x) => !x.reference)
          .map((x) => this.laid[x.strip])
          .filter((x): x is (typeof this.laid)[number] => x !== undefined)
          .map((x) => at(x.el));
        if (own.length === 0) continue;
        const g = svg("g", { class: "dv-funnel" });
        if (own.length < BUS) {
          // One or two: the funnel itself, corner to corner.
          for (const b of own) {
            g.append(
              svg("path", { d: `M ${foot.x} ${foot.y} L ${b.x} ${b.y}` }),
              svg("path", { d: `M ${foot.x + foot.w} ${foot.y} L ${b.x + b.w} ${b.y}` }),
            );
          }
        } else {
          // A choice with a dozen cases would draw a dozen funnels across each
          // other and read as a scribble. One funnel down to a rail over the
          // lot of them, and a tick from the rail into each, says the same
          // thing: any one of these, and nothing else.
          let left = Infinity;
          let right = -Infinity;
          let rail = Infinity;
          for (const b of own) {
            left = Math.min(left, b.x);
            right = Math.max(right, b.x + b.w);
            rail = Math.min(rail, b.y);
          }
          rail = Math.max(foot.y + 6, rail - STRIP_DROP / 3);
          g.append(
            svg("path", { d: `M ${foot.x} ${foot.y} L ${left} ${rail}` }),
            svg("path", { d: `M ${foot.x + foot.w} ${foot.y} L ${right} ${rail}` }),
            svg("path", { d: `M ${left} ${rail} L ${right} ${rail}` }),
          );
          for (const b of own) {
            const mid = b.x + b.w / 2;
            g.append(svg("path", { d: `M ${mid} ${rail} L ${mid} ${b.y}` }));
          }
        }
        this.lines.append(g);
      }
    }
  }

  /**
   * The layout engine, fetched once and kept.
   *
   * In a worker of its own, because it is not fast: three hundred boxes and six
   * hundred arrows take most of a second, and a second of a frozen page is a
   * page that looks broken. Off the main thread the boxes are on screen while
   * the arrows are still being worked out, which is the order a reader wants
   * them in anyway. It also keeps the engine's own megabyte and a half out of
   * the view's chunk.
   */
  private async engine(): Promise<{ layout: (g: unknown) => Promise<unknown> } | null> {
    if (this.elk !== null) return this.elk;
    try {
      const [{ default: ELK }, { default: Worker }] = await Promise.all([
        import("elkjs/lib/elk-api.js"),
        import("elkjs/lib/elk-worker.min.js?worker"),
      ]);
      this.elk = new ELK({ workerFactory: () => new Worker() }) as never;
    } catch {
      return null;
    }
    return this.elk;
  }

  /**
   * Place the boxes and route the arrows, with ELK.
   *
   * The hand-written router this replaces put each arrow in a lane of its own
   * and that was not enough: two arrows in different lanes still shared long
   * stretches of the same line, and where one crossed another the two read as
   * joined. Laying out orthogonal edges without collisions is a solved problem
   * and solving it again by hand was the mistake.
   *
   * Every field row that an arrow leaves or lands on gets a port of its own, at
   * the y it was measured at, fixed there. That is what makes an arrow leave
   * exactly at its row rather than somewhere near it, and it is why ports are
   * made only for the rows that need one: a format of three hundred types has
   * thousands of rows and ELK would be asked to place a port for every one.
   *
   * `mergeEdges` is off, so two arrows are one line only where they genuinely
   * end at the same port.
   */
  private async layout(d: TemplateDiagram, shown: boolean[], gen: number): Promise<void> {
    const elk = await this.engine();
    if (gen !== this.generation || elk === null) return;
    // Only the ports an arrow actually uses. `p{box}:{side}:{row}`, with `h`
    // for the title bar an arrow about the whole type lands on.
    const wanted = new Map<number, Set<string>>();
    const need = (box: number, side: "e" | "w", row: number | undefined): string | null => {
      const p = this.placed[box];
      if (p === undefined || shown[box] === false) return null;
      const at = row !== undefined && p.rowTop[row] !== undefined ? String(row) : "h";
      const id = `p${box}:${side}:${at}`;
      const set = wanted.get(box);
      if (set === undefined) wanted.set(box, new Set([id]));
      else set.add(id);
      return id;
    };
    type Wire = { e: DiagramEdge; id: string; from: string; to: string };
    const wires: Wire[] = [];
    for (const [i, e] of d.edges.entries()) {
      if (shown[e.from[0]] === false || shown[e.to] === false) continue;
      if (e.from[0] >= this.placed.length || e.to >= this.placed.length) continue;
      const from = need(e.from[0], "e", e.from[1]);
      const to = need(e.to, "w", e.to_row);
      if (from === null || to === null) continue;
      wires.push({ e, id: `e${i}`, from, to });
    }
    const portY = (box: number, at: string): number => {
      const p = this.placed[box];
      if (p === undefined) return 0;
      return at === "h" ? p.headTop : (p.rowTop[Number(at)] ?? p.headTop);
    };
    const children = this.placed
      .map((p, i) => ({ p, i }))
      .filter(({ i }) => shown[i] !== false)
      .map(({ p, i }) => ({
        id: `n${i}`,
        width: p.w,
        height: p.h,
        layoutOptions: { "elk.portConstraints": "FIXED_POS" },
        ports: [...(wanted.get(i) ?? [])].map((id) => {
          const [, side, at] = id.split(":");
          return {
            id,
            x: side === "e" ? p.w : 0,
            y: portY(i, at ?? "h"),
            width: 1,
            height: 1,
            layoutOptions: { "elk.port.side": side === "e" ? "EAST" : "WEST" },
          };
        }),
      }));
    const graph = {
      id: "root",
      layoutOptions: {
        "elk.algorithm": "layered",
        "elk.direction": "RIGHT",
        "elk.edgeRouting": "ORTHOGONAL",
        // The rows of a type are in the order the format writes them, and the
        // cases of a switch in the order it lists them. A layout free to
        // reorder them would be answering a question nobody asked.
        "elk.layered.considerModelOrder.strategy": "NODES_AND_EDGES",
        "elk.layered.crossingMinimization.forceNodeModelOrder": "false",
        "elk.layered.mergeEdges": "false",
        "elk.layered.nodePlacement.strategy": "BRANDES_KOEPF",
        "elk.spacing.edgeEdge": SPACING.edgeEdge,
        "elk.spacing.edgeNode": SPACING.edgeNode,
        "elk.spacing.nodeNode": SPACING.nodeNode,
        "elk.layered.spacing.edgeEdgeBetweenLayers": SPACING.edgeEdgeBetweenLayers,
        "elk.layered.spacing.edgeNodeBetweenLayers": SPACING.edgeNodeBetweenLayers,
        "elk.layered.spacing.nodeNodeBetweenLayers": SPACING.nodeNodeBetweenLayers,
        "elk.padding": `[top=${MARGIN},left=${MARGIN},bottom=${MARGIN},right=${MARGIN}]`,
      },
      children,
      edges: wires.map((w) => ({ id: w.id, sources: [w.from], targets: [w.to] })),
    };
    const began = performance.now();
    let out: typeof graph & { children?: { id: string; x?: number; y?: number }[]; edges?: unknown[] };
    try {
      out = (await elk.layout(graph)) as never;
    } catch {
      // A layout that will not run leaves the boxes where they are rather than
      // taking the view with it; the reader still has the rows and the counts.
      return;
    }
    if (gen !== this.generation) return;
    this.layoutMs = Math.round(performance.now() - began);

    const at = new Map<string, { x: number; y: number }>();
    for (const c of out.children ?? []) at.set(c.id, { x: c.x ?? 0, y: c.y ?? 0 });
    let right = 0;
    let bottom = 0;
    for (const [i, p] of this.placed.entries()) {
      if (shown[i] === false) continue;
      const spot = at.get(`n${i}`) ?? { x: MARGIN, y: MARGIN };
      p.x = Math.round(spot.x);
      p.y = Math.round(spot.y);
      p.el.style.left = `${p.x}px`;
      p.el.style.top = `${p.y}px`;
      right = Math.max(right, p.x + p.w);
      bottom = Math.max(bottom, p.y + p.h);
    }
    // The rows again, now that the boxes are somewhere.
    for (const p of this.placed) {
      p.rowMid = p.rowTop.map((y) => p.y + y);
      p.headMid = p.y + p.headTop;
    }

    // The corners ELK turned, in order. An edge it routed with no section is
    // drawn between its two ports, which is what it would have been anyway.
    type Section = { startPoint: Pt; endPoint: Pt; bendPoints?: Pt[] };
    const byId = new Map<string, Section[]>();
    for (const raw of (out.edges ?? []) as { id: string; sections?: Section[] }[]) {
      byId.set(raw.id, raw.sections ?? []);
    }
    this.routes = [];
    for (const w of wires) {
      const sections = byId.get(w.id) ?? [];
      const pts: Pt[] = [];
      for (const sec of sections) {
        if (pts.length === 0) pts.push({ x: sec.startPoint.x, y: sec.startPoint.y });
        for (const b of sec.bendPoints ?? []) pts.push({ x: b.x, y: b.y });
        pts.push({ x: sec.endPoint.x, y: sec.endPoint.y });
      }
      if (pts.length < 2) {
        const from = this.port(w.e.from[0], w.e.from[1], "right");
        const to = this.port(w.e.to, w.e.to_row, "left");
        if (from === null || to === null) continue;
        pts.push(from, { x: (from.x + to.x) / 2, y: from.y }, { x: (from.x + to.x) / 2, y: to.y }, to);
      }
      this.routes.push({ e: w.e, pts: square(pts) });
    }
    separate(this.routes);

    const reach = this.routes.flatMap((r) => r.pts);
    for (const q of reach) {
      right = Math.max(right, q.x);
      bottom = Math.max(bottom, q.y);
    }
    this.extent = { w: right + MARGIN, h: bottom + MARGIN };
    this.stage.style.width = `${this.extent.w}px`;
    this.stage.style.height = `${this.extent.h}px`;
    this.lines.setAttribute("width", String(this.extent.w));
    this.lines.setAttribute("height", String(this.extent.h));
    this.lines.setAttribute("viewBox", `0 0 ${this.extent.w} ${this.extent.h}`);
  }

  /**
   * Draw the routed arrows, stepping one over another where they cross.
   *
   * A crossing drawn as two lines meeting is a crossing that reads as a join,
   * and on a diagram whose whole subject is what joins to what that is the one
   * mistake worth going out of the way to avoid. So the horizontal line hops:
   * a small arc over the vertical one, the way a circuit diagram has done it
   * since before any of this.
   */
  private drawRoutes(): void {
    const defs = svg("defs");
    const marker = svg("marker", {
      id: "dv-arrow",
      viewBox: "0 0 10 10",
      refX: "9",
      refY: "5",
      markerWidth: "6",
      markerHeight: "6",
      orient: "auto-start-reverse",
    });
    marker.append(svg("path", { d: "M 0 0 L 10 5 L 0 10 z", class: "dv-arrowhead" }));
    defs.append(marker);
    this.lines.append(defs);

    // Every vertical piece of every arrow, so a horizontal one can find what it
    // has to step over.
    const verticals: Seg[] = [];
    for (const [i, r] of this.routes.entries()) {
      for (let k = 1; k < r.pts.length; k++) {
        const a = r.pts[k - 1];
        const b = r.pts[k];
        if (a === undefined || b === undefined) continue;
        if (Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) > 0.5) {
          verticals.push({ owner: i, x1: a.x, y1: Math.min(a.y, b.y), x2: b.x, y2: Math.max(a.y, b.y) });
        }
      }
    }

    const words: { text: SVGTextElement; ly: number }[] = [];
    for (const [i, r] of this.routes.entries()) {
      const group = svg("g", { class: `dv-edge dv-role-${r.e.role}` });
      const line = svg("path", { d: hopped(r.pts, verticals, i), "marker-end": "url(#dv-arrow)" });
      const title = svg("title");
      // What the arrow stands for, in the words the relations panel uses and
      // then the expression the template holds. A role alone leaves the reader
      // asking "which length"; an expression alone leaves them asking what it
      // decided.
      title.textContent = r.e.label === "" ? roleLabel(r.e.role) : `${roleLabel(r.e.role)}: ${r.e.label}`;
      group.append(line, title);
      // The word goes on the last run, beside where the arrow lands: that is
      // the row the reader is looking at when they follow it back.
      const last = r.pts[r.pts.length - 1];
      const before = r.pts[r.pts.length - 2] ?? last;
      if (last !== undefined && before !== undefined) {
        const lx = (last.x + before.x) / 2;
        const ly = (Math.abs(last.y - before.y) < 0.5 ? last.y : (last.y + before.y) / 2) - 3;
        const text = svg("text", { x: String(lx), y: String(ly), class: "dv-edge-label" });
        text.textContent = roleLabel(r.e.role).toLowerCase();
        group.append(text);
        words.push({ text, ly });
      }
      this.lines.append(group);
    }

    // Where every role word goes, once they are all on the page and can be
    // measured rather than guessed at. Moved a line at a time upwards, and
    // dropped where nothing near is clear: a word printed over another is less
    // readable than no word, and the arrow still says what it is on hover.
    const labels: { x1: number; y1: number; x2: number; y2: number }[] = [];
    for (const w of words) {
      const b = w.text.getBBox();
      let put = false;
      for (let step = 0; step < 6; step++) {
        const dy = -step * 10;
        const r = { x1: b.x - 1, y1: b.y + dy - 1, x2: b.x + b.width + 1, y2: b.y + b.height + dy + 1 };
        if (labels.some((l) => l.x1 < r.x2 && r.x1 < l.x2 && l.y1 < r.y2 && r.y1 < l.y2)) continue;
        labels.push(r);
        w.text.setAttribute("y", String(w.ly + dy));
        put = true;
        break;
      }
      if (!put) w.text.remove();
    }
  }

  /** Redraw the arrows on the next frame, at most once however many times this
   *  is asked for in between. */
  private later(): void {
    if (this.pending !== 0) return;
    this.pending = requestAnimationFrame(() => {
      this.pending = 0;
      this.redrawEdges();
    });
  }

  /**
   * Life size, looking at the root type.
   *
   * Left is easy: the root is the leftmost box, so the drawing starts at its
   * left edge. Up is not, because the layout centres each layer, so the root of
   * a seventy-type format sits halfway down a drawing three thousand pixels
   * tall with nothing of its own above it. So the root is centred in the window,
   * and then pulled back down far enough that no gap opens over the top of the
   * drawing: on a format that fits, the whole of it is on screen.
   */
  private home(): void {
    const root = this.placed[0];
    this.scale = 1;
    const strip = this.laid[0];
    if (strip !== undefined) {
      // The root strip at the top of the window and across the middle of it:
      // what hangs under it spreads both ways, so neither edge is where to
      // start reading. Measured off the layout rather than the screen, since
      // the screen has the transform this is about to set in it.
      this.tx = Math.round(this.board.clientWidth / 2 - (strip.el.offsetLeft + strip.el.offsetWidth / 2));
      this.ty = 12 - strip.el.offsetTop;
      this.apply();
      return;
    }
    if (root === undefined) {
      this.tx = 0;
      this.ty = 0;
      this.apply();
      return;
    }
    const top = Math.min(...this.placed.map((p) => p.y));
    this.tx = 12 - root.x;
    this.ty = Math.min(12 - top, this.board.clientHeight / 2 - (root.y + root.h / 2));
    this.apply();
  }

  /** Scale and centre so the whole drawing is in the window. */
  fit(): void {
    const w = this.board.clientWidth;
    const h = this.board.clientHeight;
    if (w === 0 || h === 0 || this.extent.w === 0 || this.extent.h === 0) return;
    // Never magnified past life size: a format with three fields blown up to
    // fill a wide window reads as a mistake.
    this.scale = Math.min(1, w / this.extent.w, h / this.extent.h);
    this.tx = Math.max(0, (w - this.extent.w * this.scale) / 2);
    this.ty = Math.max(0, (h - this.extent.h * this.scale) / 2);
    this.apply();
  }

  private apply(): void {
    this.stage.style.transform = `translate(${this.tx}px, ${this.ty}px) scale(${this.scale})`;
  }

  private bindPanZoom(): void {
    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    let moved = 0;
    this.board.addEventListener("pointerdown", (ev) => {
      // A click on a row picks a field; a drag anywhere moves the drawing. The
      // two are the same gesture until the pointer has gone a few pixels, so
      // that is where the line is drawn: past `PAN_SLOP` it is a pan, and the
      // click that follows it is not a pick.
      dragging = true;
      this.dragged = false;
      moved = 0;
      lastX = ev.clientX;
      lastY = ev.clientY;
    });
    this.board.addEventListener("pointermove", (ev) => {
      if (!dragging) return;
      const dx = ev.clientX - lastX;
      const dy = ev.clientY - lastY;
      if (dx === 0 && dy === 0) return;
      lastX = ev.clientX;
      lastY = ev.clientY;
      moved += Math.abs(dx) + Math.abs(dy);
      if (moved > PAN_SLOP && !this.dragged) {
        this.dragged = true;
        // Taken only once the gesture is a pan, never on the press. Capturing
        // on `pointerdown` would retarget the click that a plain press ends
        // with to the board, and no row would ever be picked again.
        this.board.setPointerCapture(ev.pointerId);
      }
      this.tx += dx;
      this.ty += dy;
      this.board.classList.add("is-panning");
      this.apply();
    });
    const stop = (ev: PointerEvent): void => {
      dragging = false;
      if (this.board.hasPointerCapture(ev.pointerId)) this.board.releasePointerCapture(ev.pointerId);
      this.board.classList.remove("is-panning");
    };
    this.board.addEventListener("pointerup", stop);
    this.board.addEventListener("pointercancel", stop);
    this.board.addEventListener(
      "wheel",
      (ev) => {
        ev.preventDefault();
        const rect = this.board.getBoundingClientRect();
        const px = ev.clientX - rect.left;
        const py = ev.clientY - rect.top;
        const next = Math.min(MAX_SCALE, Math.max(MIN_SCALE, this.scale * Math.exp(-ev.deltaY / 500)));
        // Zoom about the pointer, so the box under it stays under it.
        this.tx = px - ((px - this.tx) / this.scale) * next;
        this.ty = py - ((py - this.ty) / this.scale) * next;
        this.scale = next;
        this.apply();
        this.later();
      },
      { passive: false },
    );
    // The board changing size moves nothing in stage units, but the rows are
    // measured off the page and a reflow is where a measurement goes stale.
    new ResizeObserver(() => this.later()).observe(this.board);
  }
}

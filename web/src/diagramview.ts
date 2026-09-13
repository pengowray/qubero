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
 * it likes. So the boxes are placed in layers by dagre with `rankdir: "LR"`,
 * which is the same shape the Kaitai diagrams have.
 *
 * **Why the boxes are HTML.** An arrow has to land on one row of one box, so
 * the rows have to be measurable, and a row has to be coloured the way the
 * listing colours the same field. Both are free with a `<table>` and
 * `fieldClass`, and both would be reimplemented on a canvas. The arrows are the
 * one thing a table cannot draw, so they are an SVG overlay under the boxes,
 * the way `hexlinks.ts` draws its own.
 */
import dagre from "@dagrejs/dagre";

import type { DiagramBox, DiagramEdge, TemplateDiagram } from "./doc.ts";
import { fieldClass } from "./fieldstyle.ts";
import { DIAGRAM, roleLabel } from "./strings.ts";

/**
 * How many fields a box shows before the rest are folded behind one row.
 *
 * A box is read in one glance or not at all. Past a couple of dozen rows it is
 * a column of text that happens to have a border, and every box beside it has
 * been pushed off the screen to make room for fields nobody is reading. The
 * fold is one click, and what it hides is counted rather than hinted at.
 */
export const ROW_CAP = 24;

/** How far the drawing may be scaled by the wheel, either way. */
const MIN_SCALE = 0.15;
const MAX_SCALE = 3;

/** Room round the whole drawing, and between the boxes dagre places. Wide
 *  enough on the left for the brackets that join two rows of one box, which
 *  run down the outside of it. */
const MARGIN = 44;
const NODE_SEP = 28;
const RANK_SEP = 90;

/** How far under the boxes a backwards arrow runs before it turns back. */
const BACK_LANE = 22;

/** How the vertical runs between two layers of boxes are shared out.
 *
 *  `LANE_STUB` is how far clear of a box a line turns; `LANE_STEP` is the gap
 *  between one edge's vertical run and the next, wide enough to tell two lines
 *  apart at life size and narrow enough that ten of them fit in a rank gap.
 *  `LANE_BUCKET` is how close two boxes' right edges have to be for their
 *  arrows to be shared out together, since a rank is boxes of several widths
 *  rather than one column. `MIN_GAP` is the room an edge needs before it is
 *  worth routing forwards at all; anything tighter goes round. */
const LANE_STUB = 10;
const LANE_STEP = 9;
const LANE_BUCKET = 24;
const MIN_GAP = 28;

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
};

/** One drawn arrow: the path, and where its role word goes. `end` puts the word
 *  before the point rather than centred on it, for a run of arrows that all
 *  leave the same edge. */
type Route = { d: string; lx: number; ly: number; end?: boolean };

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

  /** The reader clicked a field. `main.ts` puts the cursor on it where the open
   *  file has one. */
  onPick: (box: number, row: number) => void = () => {};

  /**
   * Whether a click on this row would reach the open file.
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
    bar.append(fit);

    this.board = document.createElement("div");
    this.board.className = "dv-board";
    this.stage = document.createElement("div");
    this.stage.className = "dv-stage";
    this.lines = svg("svg", { class: "dv-lines" });
    this.stage.append(this.lines);
    this.board.append(this.stage);
    this.el.append(this.note, bar, this.board);
    this.bindPanZoom();
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
    this.build();
  }

  private build(): void {
    for (const p of this.placed) p.el.remove();
    this.placed = [];
    this.lines.replaceChildren();
    this.note.replaceChildren(this.about);
    const d = this.diagram;
    if (d === null) return;
    if (d.omitted > 0) {
      const omitted = document.createElement("span");
      omitted.className = "dv-omitted";
      omitted.textContent = DIAGRAM.omitted(d.omitted);
      this.note.append(" ", omitted);
    }
    for (const [i, box] of d.types.entries()) this.placed.push(this.buildBox(i, box));
    this.place(d);
    // The transform first, then the measuring, then the arrows: `measure`
    // divides out the scale that is in force, and the scale in force is
    // whatever `apply` last wrote.
    this.apply();
    this.measure();
    this.drawEdges(d);
    // A box measured in a fallback font is the wrong height, and every arrow
    // into it lands on the wrong row. Once, when the real fonts arrive.
    if (!this.fontsSettled) {
      this.fontsSettled = true;
      void document.fonts.ready.then(() => {
        if (this.diagram !== null) this.build();
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
    const d = this.diagram;
    if (d === null || this.placed.length === 0) return;
    this.lines.replaceChildren();
    this.measure();
    this.drawEdges(d);
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
    // No tag beside the name. A switch says what it is in its own title
    // (`switch on class`) and in its heading colour; whether a structure has a
    // name of its own in the template is on all but a couple of boxes in most
    // formats, so saying it on each of them is a word repeated eleven times
    // that the reader cannot act on. The path on hover carries it.
    const table = document.createElement("table");
    table.className = "dv-table";
    const body = document.createElement("tbody");
    const open = this.opened.has(index);
    const shown = open ? box.rows.length : Math.min(box.rows.length, ROW_CAP);
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
      if (box.kind !== "switch" && this.canPick(index, i)) {
        tr.classList.add("is-pickable");
        tr.title = DIAGRAM.pickTitle;
        tr.addEventListener("click", () => {
          if (this.dragged) return;
          this.onPick(index, i);
        });
      }
      body.append(tr);
      rows.push(tr);
    }
    // What the fold hides, counted. The row is not a field, so no arrow ever
    // lands on it and it is not in `rows`.
    if (box.rows.length > ROW_CAP) {
      const tr = document.createElement("tr");
      tr.className = "dv-more";
      const td = document.createElement("td");
      td.colSpan = box.kind === "switch" ? 2 : 4;
      td.textContent = open ? DIAGRAM.less : DIAGRAM.more(box.rows.length - ROW_CAP);
      td.title = open ? DIAGRAM.less : DIAGRAM.moreTitle;
      tr.append(td);
      tr.addEventListener("click", () => {
        if (this.dragged) return;
        if (open) this.opened.delete(index);
        else this.opened.add(index);
        this.build();
      });
      body.append(tr);
    }
    table.append(body);
    el.append(head, table);
    this.stage.append(el);
    return { box, el, rows, x: 0, y: 0, w: el.offsetWidth, h: el.offsetHeight, rowMid: [], headMid: 0 };
  }

  /** Where each box goes: dagre in layers, left to right. */
  private place(d: TemplateDiagram): void {
    const g = new dagre.graphlib.Graph({ multigraph: false, compound: false });
    g.setGraph({ rankdir: "LR", nodesep: NODE_SEP, ranksep: RANK_SEP, marginx: MARGIN, marginy: MARGIN });
    g.setDefaultEdgeLabel(() => ({}));
    for (const [i, p] of this.placed.entries()) g.setNode(String(i), { width: p.w, height: p.h });
    // One dagre edge per pair of boxes, whatever the rows say. Ranking is about
    // which box comes before which; a second arrow between the same two boxes
    // decides nothing and would weight the pair twice. A box that reaches
    // itself is left out entirely: dagre would rank it after itself.
    const seen = new Set<string>();
    for (const e of d.edges) {
      const from = e.from[0];
      if (from === e.to || from >= this.placed.length || e.to >= this.placed.length) continue;
      const key = `${from}>${e.to}`;
      if (seen.has(key)) continue;
      seen.add(key);
      g.setEdge(String(from), String(e.to));
    }
    dagre.layout(g);
    let right = 0;
    let bottom = 0;
    for (const [i, p] of this.placed.entries()) {
      const node = g.node(String(i)) as { x?: number; y?: number } | undefined;
      // A box nothing reaches still has a node; a layout that somehow lost one
      // puts it at the origin rather than at NaN, which would take the whole
      // drawing with it.
      p.x = Math.round((node?.x ?? MARGIN) - p.w / 2);
      p.y = Math.round((node?.y ?? MARGIN) - p.h / 2);
      p.el.style.left = `${p.x}px`;
      p.el.style.top = `${p.y}px`;
      right = Math.max(right, p.x + p.w);
      bottom = Math.max(bottom, p.y + p.h);
    }
    // Room for the arrows that run round the outside.
    this.extent = { w: right + MARGIN * 2, h: bottom + MARGIN * 2 + BACK_LANE * 3 };
    this.stage.style.width = `${this.extent.w}px`;
    this.stage.style.height = `${this.extent.h}px`;
    this.lines.setAttribute("width", String(this.extent.w));
    this.lines.setAttribute("height", String(this.extent.h));
    this.lines.setAttribute("viewBox", `0 0 ${this.extent.w} ${this.extent.h}`);
  }

  /**
   * Where every row actually is, measured off the page.
   *
   * Not added up from row heights and not `offsetTop`: an arrow that leaves two
   * rows above the row it is about is worse than no arrow, because it says a
   * connection the format does not have, and any arithmetic over heights is one
   * unexamined assumption away from that. `getBoundingClientRect` is what the
   * browser actually laid out. It comes back in screen pixels with the stage's
   * own transform applied, so it is taken relative to the stage's origin and
   * divided by the scale to get back to the units the boxes are placed in.
   *
   * Called after the transform is applied, so the scale divided out is the one
   * in force, and again whenever the scale changes or the fonts arrive.
   */
  private measure(): void {
    const stage = this.stage.getBoundingClientRect();
    const k = this.scale === 0 ? 1 : this.scale;
    const mid = (el: Element): number => {
      const r = el.getBoundingClientRect();
      return (r.top + r.height / 2 - stage.top) / k;
    };
    for (const p of this.placed) {
      p.rowMid = p.rows.map(mid);
      const head = p.el.querySelector(".dv-box-name");
      p.headMid = head === null ? p.y + Math.min(18, p.h / 2) : mid(head);
    }
  }

  /** Where one end of an arrow sits: the middle of a row's edge, or the title
   *  bar for an arrow that is about the whole type. */
  private port(box: number, row: number | undefined, side: "left" | "right"): { x: number; y: number } | null {
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

  private drawEdges(d: TemplateDiagram): void {
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

    // Both ends of every edge first, so the vertical runs can be shared out
    // before any of them is drawn. Drawn one at a time, each edge knows only
    // about itself and they all take the same channel.
    type Plan = { e: DiagramEdge; from: Pt; to: Pt; lane: number };
    const forward: Plan[] = [];
    const other: Plan[] = [];
    for (const e of d.edges) {
      const from = this.port(e.from[0], e.from[1], e.from[0] === e.to && e.to_row !== undefined ? "left" : "right");
      const to = this.port(e.to, e.to_row, "left");
      if (from === null || to === null) continue;
      const plan = { e, from, to, lane: 0 };
      if (e.from[0] !== e.to && to.x > from.x + MIN_GAP) forward.push(plan);
      else other.push(plan);
    }

    // One lane per edge across the gap it crosses, so ten arrows out of one box
    // are ten lines a reader can follow rather than one bundle. Edges leaving
    // at about the same x share a channel, and within it they are ordered by
    // where they land: taking the lanes in target order means two arrows only
    // cross where the format itself crosses.
    const channels = new Map<number, Plan[]>();
    for (const p of forward) {
      const key = Math.round(p.from.x / LANE_BUCKET);
      const list = channels.get(key);
      if (list === undefined) channels.set(key, [p]);
      else list.push(p);
    }
    const laneX = new Map<Plan, number>();
    for (const list of channels.values()) {
      list.sort((a, b) => a.to.y - b.to.y || a.from.y - b.from.y);
      const start = Math.max(...list.map((p) => p.from.x)) + LANE_STUB;
      // Never past the nearest box the channel feeds: a lane inside a box is a
      // line drawn through somebody's field names.
      const limit = Math.min(...list.map((p) => p.to.x)) - LANE_STUB;
      const room = Math.max(1, Math.floor((limit - start) / LANE_STEP) + 1);
      for (const [i, p] of list.entries()) laneX.set(p, start + (i % room) * LANE_STEP);
    }

    // The same for the ones that do not go forwards. A bracket down the left of
    // a box, a loop round a type that holds itself and an arrow that turns back
    // under everything all leave from one edge, so without lanes of their own a
    // box with eight of them draws eight lines on one x. Numbered per box, in
    // the order they land, for the reason the forward lanes are.
    const byBox = new Map<number, Plan[]>();
    for (const p of other) {
      const list = byBox.get(p.e.from[0]);
      if (list === undefined) byBox.set(p.e.from[0], [p]);
      else list.push(p);
    }
    for (const list of byBox.values()) {
      list.sort((a, b) => a.to.y - b.to.y || a.from.y - b.from.y);
      for (const [i, p] of list.entries()) p.lane = i;
    }

    // Every label that has been placed, so a word is moved rather than printed
    // over one already there, and every one waiting to be placed.
    const labels: { x1: number; y1: number; x2: number; y2: number }[] = [];
    const words: { text: SVGTextElement; ly: number }[] = [];
    for (const p of [...forward, ...other]) {
      const { e, from, to } = p;
      const path =
        e.from[0] !== e.to
          ? this.route(from, to, e, laneX.get(p), p.lane)
          : e.to_row === undefined
            ? this.loop(e, p.lane)
            : this.inside(e, p.lane);
      if (path === null) continue;
      const group = svg("g", { class: `dv-edge dv-role-${e.role}` });
      const line = svg("path", { d: path.d, "marker-end": "url(#dv-arrow)" });
      const title = svg("title");
      // What the arrow stands for, in the words the relations panel uses and
      // then the expression the template holds. A role alone leaves the reader
      // asking "which length"; an expression alone leaves them asking what it
      // decided.
      title.textContent = e.label === "" ? roleLabel(e.role) : `${roleLabel(e.role)}: ${e.label}`;
      group.append(line, title);
      const text = svg("text", {
        x: String(path.lx),
        y: String(path.ly),
        class: path.end === true ? "dv-edge-label is-end" : "dv-edge-label",
      });
      text.textContent = roleLabel(e.role).toLowerCase();
      group.append(text);
      words.push({ text, ly: path.ly });
      this.lines.append(group);
    }

    // Where every role word goes, once they are all on the page and can be
    // measured rather than guessed at.
    //
    // Moved a line at a time upwards, which runs along its own arrow rather
    // than across it, and dropped where nothing near is clear. A word printed
    // over another is less readable than no word, and an arrow whose word was
    // dropped still says what it is when the pointer is on it.
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

  /** An arrow between two boxes: out of the right of the row, across the gap,
   *  and into the left of the row it decides about. A target to the left of the
   *  source turns round under both boxes rather than running back through
   *  them. */
  private route(from: Pt, to: Pt, e: DiagramEdge, mid: number | undefined, lane: number): Route | null {
    if (mid !== undefined) {
      // Forwards: out of the row, down its own lane, and in along the row it is
      // about. The label goes on the last run, beside where the arrow lands,
      // because that is the row the reader is looking at when they follow it
      // back.
      return {
        d: `M ${from.x} ${from.y} H ${mid} V ${to.y} H ${to.x}`,
        lx: (mid + to.x) / 2,
        ly: to.y - 3,
      };
    }
    const a = this.placed[e.from[0]];
    const b = this.placed[e.to];
    if (a === undefined || b === undefined) return null;
    // Backwards, or two boxes too close together to get a lane between them:
    // round the outside rather than through whatever is in the way, each on its
    // own line out, its own line under and its own line back.
    const under = Math.max(a.y + a.h, b.y + b.h) + BACK_LANE + lane * LANE_STEP;
    const out = from.x + LANE_STUB + lane * LANE_STEP;
    const back = to.x - LANE_STUB - lane * LANE_STEP;
    return {
      d: `M ${from.x} ${from.y} H ${out} V ${under} H ${back} V ${to.y} H ${to.x}`,
      lx: (out + back) / 2,
      ly: under - 4,
    };
  }

  /**
   * Two rows of one type: a bracket down the left of the box, from the field
   * that decided to the field it decided about.
   *
   * The commonest arrow there is, since a length and the run it sizes are
   * nearly always neighbours. Routed round the outside like every other edge it
   * would be a loop the width of the box for two rows an inch apart, and a box
   * with five lengths in it would be five of them.
   */
  private inside(e: DiagramEdge, lane: number): Route | null {
    const p = this.placed[e.from[0]];
    const from = this.port(e.from[0], e.from[1], "left");
    const to = this.port(e.to, e.to_row, "left");
    if (p === undefined || from === null || to === null) return null;
    if (from.y === to.y) return null;
    const out = p.x - LANE_STUB - lane * 6;
    return {
      d: `M ${from.x} ${from.y} H ${out} V ${to.y} H ${to.x}`,
      // At the end the arrow leaves from, not at its middle: a box with eight
      // of these has eight brackets side by side, and eight words stacked over
      // the middle of them is a smudge. One word per row is one word per line.
      lx: out - 3,
      ly: from.y - 2,
      end: true,
    };
  }

  /** A type that holds itself: a loop off the right of the row and back into
   *  the left of the same box. Never an expansion, which would not end. */
  private loop(e: DiagramEdge, lane: number): Route | null {
    const p = this.placed[e.from[0]];
    const from = this.port(e.from[0], e.from[1], "right");
    const to = this.port(e.to, e.to_row, "left");
    if (p === undefined || from === null || to === null) return null;
    const out = p.x + p.w + 16 + lane * LANE_STEP;
    const under = p.y + p.h + 10 + lane * LANE_STEP;
    const back = p.x - 16 - lane * LANE_STEP;
    return {
      d: `M ${from.x} ${from.y} H ${out} V ${under} H ${back} V ${to.y} H ${to.x}`,
      lx: (out + back) / 2,
      ly: under - 4,
    };
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
   * left edge. Up is not, because dagre centres each layer, so the root of a
   * seventy-type format sits halfway down a drawing three thousand pixels tall
   * with nothing of its own above it. So the root is centred in the window,
   * and then pulled back down far enough that no gap opens over the top of the
   * drawing: on a format that fits, the whole of it is on screen.
   */
  private home(): void {
    const root = this.placed[0];
    this.scale = 1;
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

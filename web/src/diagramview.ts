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

type Placed = { box: DiagramBox; el: HTMLElement; rows: HTMLElement[]; x: number; y: number; w: number; h: number };

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

/**
 * A box's name, shortened to the part that tells it from its neighbours.
 *
 * The core names a type written inside a field for the whole path it was found
 * down, so a PNG chunk's packed payload is
 * `png.chunks.data.0x49444154.code.0x7070b1.packed`. Every box in that layer
 * shares the front of that, and the last two steps are what differ. The whole
 * name is on the element's title, so nothing is lost.
 */
function shortName(name: string): string {
  const parts = name.split(".");
  if (parts.length <= 3) return name;
  return `…${parts.slice(-2).join(".")}`;
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
  /** The whole drawing's extent in stage units, for `fit`. */
  private extent = { w: 0, h: 0 };

  /** The reader clicked a field. `main.ts` puts the cursor on it where the open
   *  file has one. */
  onPick: (box: number, row: number) => void = () => {};

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
    this.drawEdges(d);
    this.apply();
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
    label.textContent = shortName(box.name);
    // A type written inside a switch case inside a chunk is named for the whole
    // path it was found down, which is honest and longer than the fields under
    // it. Shortened to the part that tells it from its neighbours, with the
    // whole of it on hover.
    label.title = box.name;
    head.append(label);
    if (box.kind === "switch") {
      const tag = document.createElement("span");
      tag.className = "dv-tag";
      tag.textContent = DIAGRAM.switchTag;
      head.append(tag);
    } else if (box.parent !== undefined) {
      const tag = document.createElement("span");
      tag.className = "dv-tag";
      tag.textContent = DIAGRAM.inlineTag;
      head.append(tag);
    }
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
      tr.title = DIAGRAM.pickTitle;
      tr.addEventListener("click", () => this.onPick(index, i));
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
        if (open) this.opened.delete(index);
        else this.opened.add(index);
        this.build();
      });
      body.append(tr);
    }
    table.append(body);
    el.append(head, table);
    this.stage.append(el);
    return { box, el, rows, x: 0, y: 0, w: el.offsetWidth, h: el.offsetHeight };
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

  /** Where one end of an arrow sits: the middle of a row's edge, or the top of
   *  a box for an arrow that is about the whole type. */
  private port(box: number, row: number | undefined, side: "left" | "right"): { x: number; y: number } | null {
    const p = this.placed[box];
    if (p === undefined) return null;
    const x = side === "right" ? p.x + p.w : p.x;
    if (row === undefined) return { x, y: p.y + Math.min(18, p.h / 2) };
    const tr = p.rows[row];
    // A row behind a fold has no place of its own yet, so the arrow lands on
    // the box. Better a true arrow to the type than a false one to a row that
    // is not being shown.
    if (tr === undefined) return { x, y: p.y + Math.min(18, p.h / 2) };
    return { x, y: p.y + tr.offsetTop + tr.offsetHeight / 2 };
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
    // How many arrows have already left this gap, so the vertical runs fan out
    // instead of lying on top of one another.
    const lanes = new Map<string, number>();
    for (const e of d.edges) {
      const from = this.port(e.from[0], e.from[1], "right");
      const to = this.port(e.to, e.to_row, "left");
      if (from === null || to === null) continue;
      const lane = lanes.get(`${e.from[0]}>${e.to}`) ?? 0;
      lanes.set(`${e.from[0]}>${e.to}`, lane + 1);
      const path =
        e.from[0] !== e.to
          ? this.route(from, to, e, lane)
          : e.to_row === undefined
            ? this.loop(e, lane)
            : this.inside(e, lane);
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
      this.lines.append(group);
    }
  }

  /** An arrow between two boxes: out of the right of the row, across the gap,
   *  and into the left of the row it decides about. A target to the left of the
   *  source turns round under both boxes rather than running back through
   *  them. */
  private route(
    from: { x: number; y: number },
    to: { x: number; y: number },
    e: DiagramEdge,
    lane: number,
  ): Route | null {
    const stub = 10 + (lane % 4) * 5;
    if (to.x > from.x + stub * 2) {
      // Forwards: one turn out, one turn in, both in the gap between the boxes.
      const mid = from.x + stub + (to.x - from.x - stub * 2) * ((lane % 5) + 1) / 6;
      return {
        d: `M ${from.x} ${from.y} H ${mid} V ${to.y} H ${to.x}`,
        lx: mid,
        ly: (from.y + to.y) / 2,
      };
    }
    const a = this.placed[e.from[0]];
    const b = this.placed[e.to];
    if (a === undefined || b === undefined) return null;
    const under = Math.max(a.y + a.h, b.y + b.h) + BACK_LANE + (lane % 3) * 8;
    const out = from.x + stub;
    const back = to.x - stub;
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
    const out = p.x - 8 - (lane % 4) * 6;
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
    const out = p.x + p.w + 16 + (lane % 3) * 8;
    const under = p.y + p.h + 10 + (lane % 3) * 8;
    const back = p.x - 16 - (lane % 3) * 8;
    return {
      d: `M ${from.x} ${from.y} H ${out} V ${under} H ${back} V ${to.y} H ${to.x}`,
      lx: (out + back) / 2,
      ly: under - 4,
    };
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
    this.board.addEventListener("pointerdown", (ev) => {
      // A click on a row picks a field; a drag anywhere moves the drawing. The
      // two are told apart by whether the pointer moved, which is what the
      // browser's own click already does: a pointermove of a few pixels still
      // fires a click, so the pan starts only once it is worth starting.
      dragging = true;
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
      this.tx += dx;
      this.ty += dy;
      this.board.classList.add("is-panning");
      this.apply();
    });
    const stop = (): void => {
      dragging = false;
      this.board.classList.remove("is-panning");
    };
    this.board.addEventListener("pointerup", stop);
    this.board.addEventListener("pointerleave", stop);
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
      },
      { passive: false },
    );
  }
}

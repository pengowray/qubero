// The file's fields as a graph, laid out by what pulls them together.
//
// Every other view here is ordered by address: the hex grid, the listing, the
// text. Address is the one thing a binary format is guaranteed to have and the
// last thing that says what it means. A GGUF's tensor offsets are next to each
// other in the file and belong to records scattered through it; a PNG's chunk
// lengths are as far apart as the chunks are and are all the same thing.
//
// So this view drops address as the organising fact and lets four other facts
// compete for it: which field decided another's shape, which fields are read
// as the same type, which are near each other in the file, and which share a
// parent. fCoSe settles the argument. Turning one force up and the others down
// is how a reader asks a different question, which is why the weights are on
// screen rather than baked in.
//
// It is experimental and says so. A file with a million fields will not lay
// out, and rather than trying, the view roots itself at the part of the file
// the cursor is in and says what it left out.

import cytoscape from "cytoscape";
import fcose from "cytoscape-fcose";
import { fieldClass } from "./fieldstyle.js";
import { GRAPH } from "./strings.js";

cytoscape.use(fcose);

/**
 * The graph as the core hands it over.
 *
 * Declared here by shape rather than imported as a name, so the view depends
 * on what the core says and not on where it says it from. `parent` and the
 * edge ends are indexes into `nodes`.
 */
export type GraphInput = {
  readonly nodes: readonly {
    readonly path: readonly number[];
    readonly name: string;
    readonly kind: string;
    readonly offset_bits: number;
    readonly size_bits: number;
    readonly parent: number;
    readonly child_count: number;
  }[];
  readonly edges: readonly { readonly from: number; readonly to: number; readonly role: string }[];
  readonly omitted: number;
};

/** The four forces, and how hard each one pulls. 0 switches one off. */
export type Weights = {
  /** One field decided another's length, count, type or place. */
  depends: number;
  /** Both are read as the same type. */
  kind: number;
  /** They are next to each other in the file. */
  near: number;
  /** They are fields of the same structure. */
  sibling: number;
};

export const DEFAULT_WEIGHTS: Weights = { depends: 1, kind: 0.5, near: 0.35, sibling: 0.6 };

/** The forces, in the order the panel offers them. */
export const FORCES: readonly (keyof Weights)[] = ["depends", "kind", "near", "sibling"];

/** How many nodes the view will lay out. Past this fCoSe takes longer than a
 *  reader will wait and the picture is a hairball either way, so the walk
 *  stops and the view says how many fields it did not draw. */
export const NODE_CAP = 1200;

/** Past this many nodes fCoSe is asked for a rougher answer. Its "default"
 *  quality on a thousand nodes takes the thread for several seconds, and a
 *  picture nobody can pan while it settles is worse than a looser one they
 *  can. Under it the good layout is quick enough not to be noticed. */
const DRAFT_ABOVE = 300;

/** Fields of one kind below this many are not worth a boundary of their own:
 *  two uint32s that happened to drift together are not a cluster. */
const HULL_MIN = 3;

/** How hard the same-type pull has to be working before a boundary round a
 *  type says anything. See `drawHulls`. */
const HULL_PULL_MIN = 0.25;

/** How much of the picture one type's boundary may cover before it stops being
 *  a group and starts being the picture. See `measureHulls`. */
const HULL_SPREAD_MAX = 0.4;

/** How far in the view will zoom to fill the board with a small graph. Life
 *  size: a node drawn at the size it was measured at. */
const MAX_ZOOM = 1;

/** Ideal edge length per force, in layout units. The dependency edges are the
 *  shortest because they are the only ones that are a fact about this file
 *  rather than about the type system: a length and the field it sizes belong
 *  next to each other, and everything else is a preference. */
const IDEAL = { depends: 40, kind: 140, near: 70, sibling: 55 } as const;

type Cy = cytoscape.Core;

/** One boundary drawn round the fields of one kind, worked out after the
 *  layout has run rather than imposed on it. */
type Hull = { kind: string; points: [number, number][]; label: string };

/**
 * The graph view: a cytoscape canvas, a canvas behind it for the boundaries
 * round each kind, and the sliders.
 */
export class GraphView {
  readonly el: HTMLElement;
  private readonly board: HTMLElement;
  /** What cytoscape draws into, which it is free to empty. */
  private readonly canvas: HTMLElement;
  private readonly hullCanvas: HTMLCanvasElement;
  private readonly note: HTMLElement;
  private readonly controls: HTMLElement;
  private cy: Cy | null = null;
  private weights: Weights = { ...DEFAULT_WEIGHTS };
  private hulls: Hull[] = [];
  private input: GraphInput | null = null;
  /** Whether the next layout is this graph's first, which is the only one that
   *  should start from random positions. See `relayout`. */
  private first = true;
  /** The colours, resolved once per graph. See `readPalette`. */
  private palette: Palette | null = null;
  /** Paths by cytoscape node id, so a tap can be answered with a field. */
  private paths = new Map<string, readonly number[]>();

  /** The reader tapped a field. The rest of the app puts the cursor there. */
  onPick: (path: readonly number[]) => void = () => {};

  constructor() {
    this.el = document.createElement("section");
    this.el.className = "graphview";
    this.el.hidden = true;
    this.note = document.createElement("div");
    this.note.className = "gv-note";
    this.board = document.createElement("div");
    this.board.className = "gv-board";
    this.hullCanvas = document.createElement("canvas");
    this.hullCanvas.className = "gv-hulls";
    // Cytoscape empties the element it is given, so it gets one of its own.
    // The boundaries go beside it rather than inside it, or the first graph
    // drawn takes the canvas they are painted on with it.
    this.canvas = document.createElement("div");
    this.canvas.className = "gv-cy";
    this.board.append(this.hullCanvas, this.canvas);
    this.controls = document.createElement("div");
    this.controls.className = "gv-controls";
    this.buildControls();
    this.el.append(this.note, this.board, this.controls);
  }

  /** The sliders, one per force. Changing one relays out rather than redrawing:
   *  the whole point of the weight is where it puts things. */
  private buildControls(): void {
    const head = document.createElement("div");
    head.className = "gv-controls-head";
    head.textContent = GRAPH.forcesHeading;
    this.controls.append(head);
    for (const force of FORCES) {
      const row = document.createElement("label");
      row.className = "gv-slider";
      const name = document.createElement("span");
      name.className = "gv-slider-name";
      name.textContent = GRAPH.force[force];
      const input = document.createElement("input");
      input.type = "range";
      input.min = "0";
      input.max = "2";
      input.step = "0.05";
      input.value = String(this.weights[force]);
      input.addEventListener("input", () => {
        this.weights[force] = Number(input.value);
        this.relayout();
      });
      row.append(name, input);
      this.controls.append(row);
    }
  }

  /**
   * Put a graph on screen, replacing whatever was there.
   *
   * The elements are built once per graph and the layout is run over them;
   * moving a slider re-runs the layout on the same elements, so the reader is
   * watching one picture settle differently rather than a new one appear.
   */
  show(graph: GraphInput, rootName: string | null): void {
    this.input = graph;
    this.paths.clear();
    const cut = graph.omitted > 0;
    // What was left out replaces the experimental line rather than joining it:
    // a reader looking at a tenth of a file needs to know that before they
    // need to know the view is new.
    this.note.textContent = cut
      ? GRAPH.omitted(graph.nodes.length, graph.nodes.length + graph.omitted, rootName)
      : GRAPH.experimental;
    this.note.classList.toggle("is-warn", cut);
    this.palette = readPalette(this.el);
    this.cy?.destroy();
    this.cy = cytoscape({
      container: this.canvas,
      elements: this.elements(graph),
      style: stylesheet(this.palette),
      // The reader is looking for shape, and a hundred labels drawn at once
      // while the layout is still moving is the slowest part of the frame.
      textureOnViewport: true,
      wheelSensitivity: 0.2,
    });
    this.cy.on("tap", "node[path]", (e) => {
      const id = String(e.target.id());
      const p = this.paths.get(id);
      if (p !== undefined) this.onPick(p);
    });
    this.cy.on("viewport render", () => this.drawHulls());
    this.first = true;
    // One frame late, so the note above has been painted before the layout
    // takes the thread. A warning about a slow layout that arrives after the
    // slow layout has finished is not a warning.
    requestAnimationFrame(() => this.relayout());
  }

  /** Which field the rest of the app is on, marked here too. Nothing moves:
   *  a layout that jumped every time the cursor did would be unreadable. */
  setPath(path: readonly number[] | null): void {
    const cy = this.cy;
    if (cy === null) return;
    cy.batch(() => {
      cy.elements().removeClass("is-here is-dim");
      if (path === null) return;
      const node = cy.getElementById(idOf(path));
      if (node.empty()) return;
      // Everything but this field and what it touches steps back. Marking the
      // one node would be lost in two thousand; dimming the rest is the same
      // fact said in a way that survives the crowd.
      const keep = node.closedNeighborhood();
      cy.elements().difference(keep).addClass("is-dim");
      node.addClass("is-here");
    });
  }

  /** Lay the graph out again at the current weights. */
  private relayout(): void {
    const cy = this.cy;
    if (cy === null) return;
    const w = this.weights;
    // Only the first layout of a graph starts from nowhere. A slider moved
    // afterwards is a question about what changes, and starting from random
    // positions again would throw away the picture being compared against.
    const randomize = this.first;
    this.first = false;
    cy
      .layout({
        name: "fcose",
        quality: cy.nodes().length > DRAFT_ABOVE ? "draft" : "default",
        animate: false,
        randomize,
        // More room between nodes the more of them there are. At a dozen fields
        // the default spreads them nicely; at a thousand it packs them into a
        // ball, because every node is pushing every other node from further
        // away and the springs win.
        nodeRepulsion: () => 6000 + Math.min(40000, cy.nodes().length * 40),
        // A force turned down does not vanish; it gets long and slack, which
        // is what "pulls less" means to a spring layout. At zero the spring is
        // left in but made long enough to be beyond anything else on screen,
        // which is as close to absent as a solver that must see every edge
        // can get.
        idealEdgeLength: (e: cytoscape.EdgeSingular) => {
          const cls = String(e.data("force")) as keyof Weights;
          const base = IDEAL[cls] ?? 80;
          const pull = w[cls] ?? 1;
          return pull <= 0 ? 400 : base / pull;
        },
        edgeElasticity: (e: cytoscape.EdgeSingular) => {
          const cls = String(e.data("force")) as keyof Weights;
          return 0.45 * (w[cls] ?? 1);
        },
      } as cytoscape.LayoutOptions)
      .run();
    // A frame later, because the first graph of a session is laid out in the
    // same turn as the element it lives in was put on the page, and a board
    // the browser has not sized yet gives the boundaries a canvas of nothing
    // to be drawn on.
    requestAnimationFrame(() => {
      // Fitted to the fields, not to the hubs. A hub sits wherever the pull of
      // its type put it, which is often well outside the fields it gathered,
      // and fitting to one would leave the graph in a corner with empty space
      // beside it.
      const real = cy.nodes().filter((n) => n.data("hub") !== 1);
      if (real.length > 0) {
        cy.fit(real, 40);
        // Fitting a graph of fifteen fields to a screen blows each node up to
        // the size of a coin, which says the file is enormous. Past life size
        // the picture is recentred instead of magnified.
        if (cy.zoom() > MAX_ZOOM) {
          const at = { x: cy.width() / 2, y: cy.height() / 2 };
          cy.zoom({ level: MAX_ZOOM, renderedPosition: at });
          cy.center(real);
        }
      }
      this.measureHulls();
      this.drawHulls();
    });
  }

  /**
   * The elements cytoscape lays out: one node per field, and an edge per force
   * that has anything to say about a pair.
   *
   * Same-kind is not a clique. Twelve uint32 fields joined to each other is
   * sixty-six edges and a thousand of them is half a million; joined instead
   * to one hub of their own it is twelve, and the hub is what the layout pulls
   * them around. The hub is not drawn.
   */
  private elements(g: GraphInput): cytoscape.ElementDefinition[] {
    const out: cytoscape.ElementDefinition[] = [];
    const byKind = new Map<string, number>();
    g.nodes.forEach((n, i) => {
      const id = idOf(n.path);
      this.paths.set(id, n.path);
      byKind.set(n.kind, (byKind.get(n.kind) ?? 0) + 1);
      out.push({
        data: {
          id,
          path: n.path.join("/"),
          label: n.name,
          kind: n.kind,
          index: i,
          // How big the field is, as the node's own size. A fact about the
          // field that costs the layout nothing, unlike a fifth force.
          weight: sizeOf(n.size_bits, n.child_count),
        },
        classes: fieldClass(family(n.kind)),
      });
    });
    // One hub per kind that has enough fields to be worth grouping.
    for (const [kind, count] of byKind) {
      if (count < HULL_MIN) continue;
      out.push({ data: { id: hubOf(kind), label: kind, hub: 1 }, classes: "gv-hub" });
    }
    for (const e of g.edges) {
      const a = g.nodes[e.from];
      const b = g.nodes[e.to];
      if (a === undefined || b === undefined) continue;
      out.push({
        data: { id: `d${e.from}-${e.to}-${e.role}`, source: idOf(a.path), target: idOf(b.path), force: "depends", role: e.role },
        classes: "gv-dep",
      });
    }
    g.nodes.forEach((n, i) => {
      const id = idOf(n.path);
      if ((byKind.get(n.kind) ?? 0) >= HULL_MIN) {
        out.push({ data: { id: `k${i}`, source: id, target: hubOf(n.kind), force: "kind" }, classes: "gv-hidden" });
      }
      // Its neighbour in the file, and its parent. One edge each, so both
      // forces cost one edge per field however big the file is.
      const prev = g.nodes[i - 1];
      if (prev !== undefined && prev.parent === n.parent) {
        out.push({ data: { id: `n${i}`, source: idOf(prev.path), target: id, force: "near" }, classes: "gv-soft" });
      }
      const up = g.nodes[n.parent];
      if (up !== undefined) {
        out.push({ data: { id: `s${i}`, source: idOf(up.path), target: id, force: "sibling" }, classes: "gv-soft" });
      }
    });
    return out;
  }

  /** Where each kind's fields ended up, as a boundary round them. Worked out
   *  once per layout; the draw itself runs on every pan. */
  private measureHulls(): void {
    const cy = this.cy;
    const g = this.input;
    this.hulls = [];
    if (cy === null || g === null) return;
    const byKind = new Map<string, [number, number][]>();
    for (const n of cy.nodes()) {
      if (n.data("hub") === 1) continue;
      const kind = String(n.data("kind"));
      const p = n.position();
      const list = byKind.get(kind);
      if (list === undefined) byKind.set(kind, [[p.x, p.y]]);
      else list.push([p.x, p.y]);
    }
    // How big the whole picture is, so a boundary can be judged against it. A
    // type that ended up spread over most of the graph did not gather, and a
    // boundary round it is a shape that crosses everything else and says the
    // opposite of what a boundary means.
    const all = [...byKind.values()].flat();
    const whole = area(convexHull(all));
    for (const [kind, points] of byKind) {
      if (points.length < HULL_MIN) continue;
      const hull = convexHull(points);
      if (hull.length < 3) continue;
      if (whole > 0 && area(hull) / whole > HULL_SPREAD_MAX) continue;
      this.hulls.push({ kind, points: expand(hull, 26), label: GRAPH.hull(kind, points.length) });
    }
  }

  /**
   * The boundaries, drawn behind the graph in the graph's own coordinates.
   *
   * A canvas rather than SVG because it is repainted on every frame of a pan
   * and there is nothing in it to click: shapes that only ever get looked at
   * do not need to be elements.
   */
  private drawHulls(): void {
    const cy = this.cy;
    const ctx = this.hullCanvas.getContext("2d");
    if (cy === null || ctx === null) return;
    const w = this.board.clientWidth;
    const h = this.board.clientHeight;
    if (this.hullCanvas.width !== w || this.hullCanvas.height !== h) {
      this.hullCanvas.width = w;
      this.hullCanvas.height = h;
    }
    ctx.clearRect(0, 0, w, h);
    // A boundary is only true while the same-type pull is strong enough to
    // have gathered that type. Below this the fields of one kind are spread
    // over the whole graph and a hull round them is a shape that crosses
    // everything, saying the opposite of what it means.
    if (this.weights.kind < HULL_PULL_MIN) return;
    const p = this.palette;
    if (p === null) return;
    const zoom = cy.zoom();
    const pan = cy.pan();
    // Read once when the graph was built, not here: this runs on every frame
    // of a pan, and `getComputedStyle` is a forced style recalculation.
    ctx.strokeStyle = p.hull;
    ctx.fillStyle = p.hullFill;
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.font = `11px ${p.sans}`;
    for (const hull of this.hulls) {
      ctx.beginPath();
      hull.points.forEach(([x, y], i) => {
        const sx = x * zoom + pan.x;
        const sy = y * zoom + pan.y;
        if (i === 0) ctx.moveTo(sx, sy);
        else ctx.lineTo(sx, sy);
      });
      ctx.closePath();
      ctx.fill();
      ctx.stroke();
      // Above the boundary's highest point and clear of it, where it names the
      // group without landing on a field. Overlapping labels are the one
      // failure this view cannot afford: a boundary the reader cannot read the
      // name of is a shape, and a shape says nothing.
      const top = hull.points.reduce((a, b) => (b[1] < a[1] ? b : a));
      const tx = top[0] * zoom + pan.x;
      const ty = top[1] * zoom + pan.y - 6;
      ctx.fillStyle = p.hullLabel;
      // A short bar of the ground colour under the words, so a label that ends
      // up over an edge is still readable. The node labels are drawn by
      // cytoscape on the canvas above this one, so they win where the two
      // meet, which is the right way round: a field's own name matters more
      // than the name of the crowd it is in.
      ctx.textAlign = "center";
      ctx.fillText(hull.label, tx, ty);
      ctx.textAlign = "start";
      ctx.fillStyle = p.hullFill;
    }
  }

  /** Cytoscape measures its container, and a container that is hidden measures
   *  nothing. Called when the view comes to the front. */
  relayoutForShow(): void {
    this.cy?.resize();
    this.drawHulls();
  }
}

/** A node's id: the path, which is what everything else here addresses a field
 *  by. `r` for the root, whose path is empty. */
function idOf(path: readonly number[]): string {
  return path.length === 0 ? "r" : `f${path.join("_")}`;
}

function hubOf(kind: string): string {
  return `hub:${kind}`;
}

/** The type family a kind belongs to, for `fieldClass`. The core's kinds are
 *  finer than the five colours, and the colours are shared with every other
 *  view, so they are mapped rather than added to. */
function family(kind: string): string {
  if (/^[ui]\d/.test(kind) || kind === "uint" || kind === "int") return "uint";
  if (/^f\d/.test(kind) || kind === "float") return "float";
  if (kind === "str" || kind === "text") return "str";
  if (kind === "struct" || kind === "array" || kind === "repeat" || kind === "composite") return "composite";
  if (kind === "magic") return "magic";
  if (kind === "enum") return "enum";
  return "bytes";
}

/** How big to draw a field: by how many bytes it holds, and by how many
 *  children it has. Logarithmic, because a file holds four-byte headers and
 *  four-hundred-megabyte tensors and both have to be on the same screen. */
function sizeOf(sizeBits: number, children: number): number {
  const bytes = Math.max(1, sizeBits / 8);
  return Math.log2(bytes + 1) + Math.log2(children + 1) * 0.5;
}

/** Andrew's monotone chain. The points are node centres, so a few thousand at
 *  most, and this is O(n log n) over them. */
function convexHull(points: readonly [number, number][]): [number, number][] {
  const pts = [...points].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  if (pts.length < 3) return pts;
  const cross = (o: [number, number], a: [number, number], b: [number, number]): number =>
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const half = (list: [number, number][]): [number, number][] => {
    const out: [number, number][] = [];
    for (const p of list) {
      while (out.length >= 2) {
        const a = out[out.length - 2];
        const b = out[out.length - 1];
        if (a === undefined || b === undefined || cross(a, b, p) > 0) break;
        out.pop();
      }
      out.push(p);
    }
    out.pop();
    return out;
  };
  return [...half(pts), ...half([...pts].reverse())];
}

/** The area a closed polygon encloses, by the shoelace formula. Used only to
 *  compare one boundary against another, so the sign does not matter. */
function area(points: readonly [number, number][]): number {
  if (points.length < 3) return 0;
  let sum = 0;
  for (let i = 0; i < points.length; i++) {
    const a = points[i];
    const b = points[(i + 1) % points.length];
    if (a === undefined || b === undefined) continue;
    sum += a[0] * b[1] - b[0] * a[1];
  }
  return Math.abs(sum) / 2;
}

/** Push a hull outwards so the boundary is round the nodes rather than through
 *  their centres. */
function expand(hull: readonly [number, number][], by: number): [number, number][] {
  const cx = hull.reduce((s, p) => s + p[0], 0) / hull.length;
  const cy = hull.reduce((s, p) => s + p[1], 0) / hull.length;
  return hull.map(([x, y]) => {
    const dx = x - cx;
    const dy = y - cy;
    const d = Math.hypot(dx, dy) || 1;
    return [x + (dx / d) * by, y + (dy / d) * by] as [number, number];
  });
}

/**
 * The colours the graph is drawn in, read off the page.
 *
 * Cytoscape draws to a canvas and parses its own style language, in which
 * `var(--accent)` means nothing: a stylesheet written in CSS variables comes
 * out silently grey. So the five field colours and the interface ones are
 * looked up once, by putting an element of each class in the view and asking
 * the browser what it made of it. Doing that at `show` is also what gets dark
 * mode right without this file having to know there is such a thing.
 */
type Palette = {
  readonly field: Readonly<Record<string, string>>;
  readonly accent: string;
  readonly fg: string;
  readonly line: string;
  readonly hull: string;
  readonly hullFill: string;
  readonly hullLabel: string;
  readonly sans: string;
};

/** The classes `fieldClass` can hand back, which are the ones worth asking
 *  about. */
const FIELD_CLASSES = ["field-number", "field-text", "field-marker", "field-category", "field-structure", "field-binary"] as const;

function readPalette(host: HTMLElement): Palette {
  const probe = document.createElement("span");
  probe.style.display = "none";
  host.append(probe);
  const field: Record<string, string> = {};
  for (const cls of FIELD_CLASSES) {
    probe.className = cls;
    field[cls] = getComputedStyle(probe).getPropertyValue("--field-color").trim() || "#888888";
  }
  probe.remove();
  const own = getComputedStyle(host);
  const at = (name: string, fallback: string): string => own.getPropertyValue(name).trim() || fallback;
  return {
    field,
    accent: at("--accent", "#2457c5"),
    fg: at("--fg", "#1b1b1f"),
    line: at("--line", "#dcdfe4"),
    hull: at("--gv-hull", "#8a90993d"),
    hullFill: at("--gv-hull-fill", "#8a909914"),
    hullLabel: at("--gv-hull-label", "#6b6f76"),
    sans: at("--sans", "sans-serif"),
  };
}

/** How the graph is drawn, in colours already resolved to what this page
 *  shows. A field is the colour here that it is in the listing and in the hex
 *  view, because both read the same variables. */
function stylesheet(p: Palette): cytoscape.StylesheetJson {
  const perClass = FIELD_CLASSES.map((cls) => ({
    selector: `node.${cls}`,
    style: { "background-color": p.field[cls] ?? "#888888" },
  }));
  return [
    {
      selector: "node",
      style: {
        "background-color": "#888888",
        width: "mapData(weight, 0, 24, 10, 46)",
        height: "mapData(weight, 0, 24, 10, 46)",
        label: "data(label)",
        "font-size": "9px",
        color: p.fg,
        "text-valign": "center",
        "text-halign": "right",
        "text-margin-x": 3,
        "min-zoomed-font-size": 8,
      },
    },
    ...perClass,
    { selector: "node.gv-hub", style: { width: 1, height: 1, opacity: 0, label: "" } },
    {
      selector: "edge.gv-dep",
      style: {
        width: 1.2,
        "line-color": p.accent,
        "target-arrow-color": p.accent,
        "target-arrow-shape": "triangle",
        "arrow-scale": 0.6,
        "curve-style": "straight",
      },
    },
    { selector: "edge.gv-soft", style: { width: 0.5, "line-color": p.line, "curve-style": "haystack", opacity: 0.5 } },
    // The same-type edges run to a hub nobody can see. Drawn, they are lines
    // with one end in empty space, which reads as a stray rather than as the
    // pull it is. The boundary round the type is what shows that.
    { selector: "edge.gv-hidden", style: { opacity: 0, events: "no" } },
    { selector: "node.is-here", style: { "border-width": 2, "border-color": p.accent, "z-index": 10 } },
    // Everything that is neither the field at the cursor nor joined to it
    // steps back, so "what is this connected to" is answered by looking
    // instead of by tracing.
    { selector: "node.is-dim", style: { opacity: 0.45 } },
    { selector: "edge.is-dim", style: { opacity: 0.25 } },
  ];
}

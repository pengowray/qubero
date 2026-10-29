// Pointer arcs over the file's address axis.
//
// Candidate `pointer-arcs-axis` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent C1 of the first batch (section 5.3). Directory structure is
// the least visible thing in a hex dump: a ZIP's central directory reaching
// back to each local header, a PDF's xref at the end giving the offset of
// every object, a length in front of each chunk. This draws each of those as
// an arc over a bar of the whole file, from the field that holds the number
// to the field the number is about, so the shape of the file's references is
// one picture.
//
// Drawn in two places:
// - the Pointers tab in the rail (`overviewtabs.ts`), every arc in the file;
// - a card under a listing heading (`headingbuttons.ts` offers it, and
//   `formatcards.ts` draws it in place of the heading's fields), only the
//   arcs with an end inside that part, still over the whole file so the far
//   end is where it is.
// The report hook named in the design comes later.
//
// What it is made of:
// - the arcs come from `pointerwalk.ts`, which asks the core about every
//   field (`Doc.origins`) a slice at a time. One walk per document, shared by
//   the tab and every card (`pointersFor`), started by the first figure that
//   is shown and carried on as bytes arrive;
// - the layout, bundling and hit testing are `arcmath.ts`, with no DOM;
// - the bar under the arcs is the file's top-level parts (`fileParts`), with
//   names fitted inside each part and dropped where they do not fit
//   (`fit.ts`), and bytes no part covers hatched;
// - the wheel zooms the address axis about the pointer and a drag pans it
//   (`board.ts`, x only), and offsets along the bottom come from d3-axis,
//   spaced by `hexTicks` from the worked example;
// - hover goes by an arc's ends first and its curve second, shows the field
//   that holds the number, what it holds, the expression the template writes
//   for it with the values put in, and how far it reaches (`tip.ts`), and
//   publishes the end under the pointer so the hex view marks it
//   (`linked.ts`); a click on an end goes to that field, and on the curve to
//   the field it points to (`pick.ts`);
// - the field the cursor is in lights its own arcs and dims the rest, as
//   does a hover from any other view;
// - chips above the figure turn each role on and off, and are its legend:
//   each says the role's word, its count and its colour (`roleColor`).
//
// Drawn on a canvas, since a file can have tens of thousands of arcs: past
// `MAX_SINGLE` in view they are bundled (`arcmath.ts`). The offsets axis is
// SVG, drawn by d3-axis.

import type { Doc, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { ROLES, roleColor } from "../fieldstyle.ts";
import { ROLE_GROUP, VIZ } from "../strings.ts";
import { askedCardKey, registerKindCard } from "../formatcards.ts";
import { arcShapes, arcsTouching, sideRoom, arcOverlaps, arcTouches, nearestArc, reach, roleCounts, uncovered, type ArcEnd, type ArcHit, type ArcShape, type PointerArc } from "./arcmath.ts";
import { board, type Board } from "./board.ts";
import { applyX, invertX, rescaleDomain, type View } from "./boardmath.ts";
import { hexTicks } from "./childstrip.ts";
import { canvasMeasure, ellipsise, monoMeasure, type Measure } from "./fit.ts";
import { headingButton, registerHeadingButton } from "./headingbuttons.ts";
import { linkedFor, subscribeWhileConnected, type BitRange } from "./linked.ts";
import { registerOverviewTab, type OverviewTabPanel } from "./overviewtabs.ts";
import { goToBits, goToField } from "./pick.ts";
import { fileParts, PointerWalk, type FilePart } from "./pointerwalk.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";
import "./pointerarcs.css";

const T = VIZ.pointerArcs;

// ----- the walk, one per document -----

/** How long one slice of the walk may run before the page gets to draw. */
const SLICE_MS = 8;
/** How often a running walk tells the figures it found more. Drawing every
 *  slice would spend the walk's time on pictures of half a walk. */
const NOTIFY_MS = 120;

export type PointerState = "working" | "pending" | "done";

/** The arcs of one document, found by a walk that runs in slices. */
export class Pointers {
  private walk: PointerWalk | null = null;
  private key = "";
  private timer: ReturnType<typeof setTimeout> | null = null;
  private lastNotify = 0;
  private readonly listeners = new Set<() => void>();
  parts: readonly FilePart[] | null = null;
  state: PointerState = "working";

  private readonly doc: Doc;

  constructor(doc: Doc) {
    this.doc = doc;
    // Bytes arriving are what a waiting walk waits for; an edit or a new
    // template is a different file, walked again from the start.
    doc.onChange(() => {
      if (this.walk === null) return;
      if (this.keyNow() !== this.key) this.restart();
      else if (this.state === "pending") this.schedule();
    });
  }

  get arcs(): readonly PointerArc[] {
    return this.walk?.arcs ?? [];
  }
  get visited(): number {
    return this.walk?.visited ?? 0;
  }
  get capped(): boolean {
    return this.walk?.capped ?? false;
  }
  get fileBits(): number {
    return this.doc.lengthBits;
  }
  /** True once a walk has been asked for. */
  get started(): boolean {
    return this.walk !== null;
  }

  /** Start the walk if nothing has, and carry on with it if it is waiting. */
  ensure(): void {
    if (this.walk === null || this.keyNow() !== this.key) this.restart();
    else if (this.state !== "done") this.schedule();
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** What makes this the same file as when the walk started: the template,
   *  the length and the edits. */
  private keyNow(): string {
    return `${this.doc.template ?? ""}|${this.doc.lengthBits}|${this.doc.pieceCount}|${this.doc.modified}`;
  }

  private restart(): void {
    this.key = this.keyNow();
    this.walk = new PointerWalk(this.doc);
    this.parts = null;
    this.state = "working";
    this.schedule();
  }

  private schedule(): void {
    if (this.timer !== null) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.run();
    }, 0);
  }

  private run(): void {
    const walk = this.walk;
    if (walk === null) return;
    this.parts ??= fileParts(this.doc, this.doc.lengthBits);
    const r = walk.step(performance.now() + SLICE_MS, () => performance.now());
    const was = this.state;
    this.state = r === "done" && this.parts !== null ? "done" : r === "pending" ? "pending" : "working";
    if (this.state === "working") this.schedule();
    const now = performance.now();
    if (this.state !== was || this.state === "done" || now - this.lastNotify >= NOTIFY_MS) {
      this.lastNotify = now;
      for (const fn of [...this.listeners]) fn();
    }
  }
}

const walks = new WeakMap<Doc, Pointers>();

/** The arcs of `doc`, shared by every figure of it. */
export function pointersFor(doc: Doc): Pointers {
  let p = walks.get(doc);
  if (p === undefined) {
    p = new Pointers(doc);
    walks.set(doc, p);
  }
  return p;
}

// ----- the figure -----

/** The bar the arcs stand on. */
const BAR_H = 16;
/** The offsets axis under the figure. */
const AXIS_H = 18;
/** Names on the bar, the size of the listing's small text less one. */
const NAME_PX = 10;
const NAME_PAD = 3;
const NAME_MIN_CHARS = 3;
/** How wide one byte may get at the deepest zoom. */
const MAX_BYTE_PX = 48;
/** Room per offset tick, so d3 is asked for as many as fit. */
const TICK_SPACING_PX = 90;
const TICK_PX = 10;
const TICK_MEASURE = monoMeasure();
/** The roles drawn until the reader turns them off. The others (a type, a
 *  value, a name worked out from another field) are dependencies but not
 *  pointers, and drawn only when asked for. */
const DEFAULT_ROLES: ReadonlySet<string> = new Set(["position", "length", "count"]);
/** Roles in the order the chips list them. `points` is folded into
 *  `position` by the walk. */
const CHIP_ORDER = ROLES.filter((r) => r !== "points");

export type ArcFigureOptions = {
  readonly width: number;
  /** Room for arcs on each side of the bar. */
  readonly arcPx: number;
  /** Only the arcs with an end inside this field, for a card under its
   *  heading. Null for every arc in the file. */
  readonly within: readonly number[] | null;
  /** Keep the full `arcPx` on both sides whatever the arcs need, for a host
   *  that measures the figure once and expects it to stay that height. */
  readonly fixedRoom?: boolean;
  /** Where the reader left the zoom last time this figure was drawn, as a
   *  share of the file's width, and where to say when it moves. A card is
   *  built again on every paint of the listing, so it keeps this outside. */
  readonly zoom?: { readonly get: () => readonly [number, number] | null; readonly set: (range: readonly [number, number] | null) => void };
};

export type ArcFigure = {
  readonly el: HTMLElement;
  readonly board: Board;
  resize(width: number): void;
  destroy(): void;
};

let measure: Measure | null = null;
function measurer(): Measure {
  measure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return measure;
}

type Axis = {
  readonly scale: typeof import("d3-scale");
  readonly axis: typeof import("d3-axis");
  readonly selection: typeof import("d3-selection");
};
let axisLoading: Promise<Axis> | null = null;
function loadAxis(): Promise<Axis> {
  axisLoading ??= Promise.all([import("d3-scale"), import("d3-axis"), import("d3-selection")]).then(([scale, axis, selection]) => ({ scale, axis, selection }));
  return axisLoading;
}

const SVG = "http://www.w3.org/2000/svg";

/** A CSS colour as the canvas can take it, since the canvas does not read
 *  custom properties. */
function resolveColor(probe: HTMLElement, css: string): string {
  probe.style.color = css;
  return getComputedStyle(probe).color;
}

/** `share` of `a` over `b`, both as the browser computes them (`rgb(...)`).
 *  Worked out here: the browser computes a `color-mix` to `color(srgb ...)`,
 *  which Chrome's canvas does not take. */
function mix(a: string, b: string, share: number): string {
  const rgb = (c: string): number[] => (c.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
  const x = rgb(a);
  const y = rgb(b);
  if (x.length < 3 || y.length < 3) return a;
  const m = x.map((v, i) => Math.round(v * share + (y[i] ?? 0) * (1 - share)));
  return `rgb(${m.join(", ")})`;
}

/** The theme's colours, read when the figure paints. */
type Ink = { fg: string; muted: string; accent: string; partA: string; partB: string; ground: string; hatch: CanvasPattern | string };

function readInk(probe: HTMLElement, ctx: CanvasRenderingContext2D): Ink {
  const muted = resolveColor(probe, "var(--muted)");
  const tile = document.createElement("canvas");
  tile.width = 6;
  tile.height = 6;
  const t = tile.getContext("2d");
  let hatch: CanvasPattern | string = muted;
  if (t !== null) {
    t.strokeStyle = muted;
    t.lineWidth = 1;
    t.beginPath();
    t.moveTo(0, 6);
    t.lineTo(6, 0);
    t.moveTo(-1, 1);
    t.lineTo(1, -1);
    t.moveTo(5, 7);
    t.lineTo(7, 5);
    t.stroke();
    hatch = ctx.createPattern(tile, "repeat") ?? muted;
  }
  const fg = resolveColor(probe, "var(--fg)");
  const bg = resolveColor(probe, "var(--bg)");
  return {
    fg,
    muted,
    accent: resolveColor(probe, "var(--accent)"),
    // Two quiet shades for neighbouring parts, so the reader sees where one
    // ends and the next begins without either meaning anything.
    partA: mix(fg, bg, 0.12),
    partB: mix(fg, bg, 0.22),
    ground: mix(fg, bg, 0.04),
    hatch,
  };
}

function isDark(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-color-scheme: dark)").matches;
}

/** Draw the arcs `opts.width` pixels wide. */
export function drawPointerArcs(doc: Doc, source: Pointers, opts: ArcFigureOptions): ArcFigure {
  let width = Math.max(80, opts.width);
  const arcPx = opts.arcPx;
  // The room above and below the bar, cut down to what the arcs need
  // (`sideRoom`) whenever the arcs drawn change.
  let upH = arcPx;
  let downH = arcPx;
  let height = upH + BAR_H + downH;
  let barTop = upH;
  let barBottom = upH + BAR_H;

  const host = document.createElement("figure");
  host.className = "viz-arcs";
  const head = document.createElement("div");
  head.className = "viz-arcs-head";
  const chips = document.createElement("div");
  chips.className = "viz-arcs-chips";
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  // The chips get the head row to themselves, since a narrow rail fits two
  // to a line; the status and the zoom button share the line under them.
  head.append(chips);
  const tools = document.createElement("div");
  tools.className = "viz-arcs-tools";
  const status = document.createElement("div");
  status.className = "viz-arcs-status";
  tools.append(status, reset);
  const stage = document.createElement("div");
  stage.className = "viz-arcs-stage";
  stage.setAttribute("role", "img");
  stage.setAttribute("aria-label", T.figureLabel);
  const canvas = document.createElement("canvas");
  canvas.className = "viz-arcs-canvas";
  const axisSvg = document.createElementNS(SVG, "svg");
  axisSvg.setAttribute("class", "viz-arcs-axis");
  const axisG = document.createElementNS(SVG, "g");
  axisG.setAttribute("class", "viz-axis");
  axisSvg.append(axisG);
  stage.append(canvas, axisSvg);
  const empty = document.createElement("p");
  empty.className = "viz-arcs-empty";
  empty.hidden = true;
  const foot = document.createElement("figcaption");
  foot.className = "viz-arcs-foot";
  const swatch = document.createElement("span");
  swatch.className = "viz-arcs-hatch";
  swatch.setAttribute("aria-hidden", "true");
  const unclaimed = document.createElement("span");
  unclaimed.className = "viz-arcs-key";
  unclaimed.append(swatch, T.unclaimed);
  const legend = document.createElement("span");
  legend.textContent = T.legend;
  foot.append(legend, " ", unclaimed);
  // A colour probe for the canvas, inside the figure so it inherits the
  // figure's theme.
  const probe = document.createElement("span");
  probe.hidden = true;
  host.append(head, tools, stage, empty, foot, probe);

  const ctx = canvas.getContext("2d");
  const bus = linkedFor(doc);
  const enabled = new Set<string>(DEFAULT_ROLES);
  let arcs: readonly PointerArc[] = [];
  let shown: readonly PointerArc[] = [];
  let shapes: ArcShape[] = [];
  let hover: ArcHit | null = null;
  let myHover: BitRange | null = null;
  let view: View = { k: 1, x: 0, y: 0 };
  let axis: Axis | null = null;
  let ink: Ink | null = null;
  let chipsKey = "";
  let zoom: Board | null = null;
  /** True once the room above and below may no longer change. */
  let fixed = opts.fixedRoom === true && source.state !== "done";
  /** The bits of the field a card is about. */
  const withinSpan = ((): { startBit: number; endBit: number } | null => {
    if (opts.within === null) return null;
    const r = doc.templateNode(opts.within);
    return r.status === "ok" ? { startBit: r.node.offset_bits, endBit: r.node.offset_bits + r.node.size_bits } : null;
  })();

  const fileBits = (): number => Math.max(8, source.fileBits);
  /** A bit's place across the figure at no zoom. */
  const layoutX = (bit: number): number => (bit / fileBits()) * width;
  const xOf = (bit: number): number => applyX(view, layoutX(bit));

  const sizeCanvas = (): void => {
    const dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    axisSvg.setAttribute("width", String(width));
    axisSvg.setAttribute("height", String(AXIS_H));
    ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
  };

  const refilter = (): void => {
    arcs = opts.within === null ? source.arcs : arcsTouching(source.arcs, opts.within, withinSpan ?? undefined);
    shown = arcs.filter((a) => enabled.has(a.role));
    const room = fixed ? { up: arcPx, down: arcPx } : sideRoom(shown, fileBits(), arcPx);
    // A figure that must keep its height is still cut to its arcs when they
    // are all known before it is first drawn.
    if (opts.fixedRoom === true) fixed = true;
    if (room.up === upH && room.down === downH) return;
    upH = room.up;
    downH = room.down;
    height = upH + BAR_H + downH;
    barTop = upH;
    barBottom = upH + BAR_H;
    sizeCanvas();
    zoom?.resize(width, height + AXIS_H);
  };

  const drawChips = (): void => {
    const counts = roleCounts(arcs, CHIP_ORDER);
    const key = counts.map((c) => `${c.role}:${c.count}:${enabled.has(c.role)}`).join(",");
    if (key === chipsKey) return;
    chipsKey = key;
    const dark = isDark();
    chips.replaceChildren(
      ...counts.map(({ role, count }) => {
        const on = enabled.has(role);
        const word = ROLE_GROUP[role] ?? role;
        const b = document.createElement("button");
        b.type = "button";
        b.className = `viz-arcs-chip${on ? " is-on" : ""}`;
        b.setAttribute("aria-pressed", String(on));
        b.title = T.chipTitle(word.toLowerCase(), count);
        b.style.setProperty("--role", roleColor(role, dark));
        const dot = document.createElement("span");
        dot.className = "viz-arcs-swatch";
        dot.setAttribute("aria-hidden", "true");
        const n = document.createElement("span");
        n.className = "viz-arcs-count";
        n.textContent = count.toLocaleString();
        b.append(dot, word, " ", n);
        b.addEventListener("click", () => {
          if (enabled.has(role)) enabled.delete(role);
          else enabled.add(role);
          refilter();
          drawChips();
          paint();
        });
        return b;
      }),
    );
  };

  const drawStatus = (): void => {
    let text = "";
    if (source.state !== "done") text = T.finding(source.visited);
    else if (source.capped) text = T.capped(source.visited);
    status.textContent = text;
    const none = source.state === "done" && arcs.length === 0;
    empty.hidden = !none;
    if (none) empty.textContent = opts.within === null ? T.none : T.noneHere(withinName());
    stage.hidden = none;
    foot.hidden = none;
    head.hidden = none;
    tools.hidden = none;
  };

  const withinName = (): string => {
    if (opts.within === null) return "";
    const r = doc.templateNode(opts.within);
    return r.status === "ok" ? r.node.name : "";
  };

  /** The arcs to draw brighter than the rest, or an empty set when none is. */
  const emphasis = (): Set<PointerArc> => {
    const out = new Set<PointerArc>();
    if (hover !== null) for (const a of hover.shape.arcs) out.add(a);
    const sel = bus.state.selection;
    if (sel !== null && sel.path.length > 0) for (const a of shown) if (arcTouches(a, sel.path, sel)) out.add(a);
    const h = bus.state.hover;
    if (h !== null && h !== myHover) for (const a of shown) if (arcOverlaps(a, h.startBit, h.endBit)) out.add(a);
    return out;
  };

  const paintBar = (c: CanvasRenderingContext2D, k: Ink): void => {
    c.fillStyle = k.ground;
    c.fillRect(0, barTop, width, BAR_H);
    const parts = source.parts ?? [];
    for (const gap of uncovered(parts, fileBits())) {
      const x0 = Math.max(0, xOf(gap.startBit));
      const x1 = Math.min(width, xOf(gap.endBit));
      if (x1 <= 0 || x0 >= width) continue;
      c.fillStyle = k.hatch;
      c.fillRect(x0, barTop, Math.max(1, x1 - x0), BAR_H);
    }
    c.font = `${NAME_PX}px ${getComputedStyle(host).fontFamily}`;
    c.textBaseline = "middle";
    parts.forEach((p, i) => {
      const x0 = xOf(p.startBit);
      const x1 = xOf(p.endBit);
      if (x1 < 0 || x0 > width) return;
      const left = Math.max(0, x0);
      const right = Math.min(width, x1);
      c.fillStyle = i % 2 === 0 ? k.partA : k.partB;
      c.fillRect(left, barTop, Math.max(1, right - left), BAR_H);
      // A name only inside its own part, so two can never meet.
      const text = ellipsise(p.name, right - left - 2 * NAME_PAD, NAME_PX, measurer(), NAME_MIN_CHARS);
      if (text === null) return;
      c.fillStyle = k.fg;
      c.fillText(text, left + NAME_PAD, barTop + BAR_H / 2);
    });
    // The field this card is about, and the ends of the arc under the
    // pointer, marked on the bar in the accent the cursor is drawn in.
    const lit: { startBit: number; endBit: number }[] = [];
    if (opts.within !== null) {
      const r = doc.templateNode(opts.within);
      if (r.status === "ok") lit.push({ startBit: r.node.offset_bits, endBit: r.node.offset_bits + r.node.size_bits });
    }
    if (hover !== null && hover.shape.arcs.length === 1) {
      const a = hover.shape.arcs[0] as PointerArc;
      lit.push(a.from, a.to);
    }
    c.strokeStyle = k.accent;
    c.lineWidth = 2;
    for (const s of lit) {
      const x0 = xOf(s.startBit);
      const x1 = Math.max(x0 + 2, xOf(s.endBit));
      if (x1 < 0 || x0 > width) continue;
      c.strokeRect(x0, barTop + 1, x1 - x0, BAR_H - 2);
    }
  };

  const paintArcs = (c: CanvasRenderingContext2D, dark: boolean): void => {
    shapes = arcShapes(shown, layoutX, { left: invertX(view, 0) - 1, right: invertX(view, width) + 1, spanBits: fileBits(), maxPx: arcPx - 4 }).map((s) => ({
      ...s,
      x0: applyX(view, s.x0),
      x1: applyX(view, s.x1),
    }));
    const emph = emphasis();
    const anyEmph = emph.size > 0;
    // Faint ones first, so the bright ones are drawn over them.
    const ordered = anyEmph ? [...shapes].sort((a, b) => Number(a.arcs.some((x) => emph.has(x))) - Number(b.arcs.some((x) => emph.has(x)))) : shapes;
    for (const s of ordered) {
      const bright = s.arcs.some((a) => emph.has(a));
      const bundle = s.arcs.length > 1;
      c.strokeStyle = roleColor(s.role, dark);
      c.globalAlpha = anyEmph ? (bright ? 1 : 0.15) : bundle ? 0.55 : 0.8;
      c.lineWidth = (bright ? 2 : 1.25) + (bundle ? Math.min(4, Math.log2(s.arcs.length)) : 0);
      const cx = (s.x0 + s.x1) / 2;
      const rx = Math.max(0.5, (s.x1 - s.x0) / 2);
      c.beginPath();
      if (s.up) c.ellipse(cx, barTop, rx, s.ry, 0, Math.PI, 2 * Math.PI);
      else c.ellipse(cx, barBottom, rx, s.ry, 0, 0, Math.PI);
      c.stroke();
    }
    c.globalAlpha = 1;
  };

  const drawAxis = (): void => {
    if (axis === null) return;
    const bytes = rescaleDomain(view, [0, fileBits() / 8], [0, width]);
    const x = axis.scale.scaleLinear().domain(bytes).range([0, width]);
    const ticks = hexTicks(bytes, Math.max(2, Math.floor(width / TICK_SPACING_PX)), (b) => x(b), width, (b) => TICK_MEASURE(formatOffset(b * 8), TICK_PX));
    const a = axis.axis
      .axisBottom(x)
      .tickValues(ticks.map((t) => t.byte))
      .tickSize(3)
      .tickFormat((d) => formatOffset(Number(d) * 8));
    const g = axis.selection.select(axisG);
    g.call(a);
    const anchors = new Map(ticks.map((t) => [t.byte, t.anchor]));
    g.selectAll<SVGTextElement, number>(".tick text").attr("text-anchor", (d) => anchors.get(Number(d)) ?? "middle");
  };

  const paint = (): void => {
    drawStatus();
    if (ctx === null || stage.hidden) return;
    // The theme's colours can only be read once the figure is on the page;
    // until then there is nothing to see anyway.
    if (ink === null && host.isConnected) ink = readInk(probe, ctx);
    if (ink === null) return;
    ctx.clearRect(0, 0, width, height);
    paintArcs(ctx, isDark());
    paintBar(ctx, ink);
    drawAxis();
  };

  // Read before the board is made: the board reports its starting zoom at
  // once, which would clear what was kept.
  const kept = opts.zoom?.get() ?? null;
  const pan = board(stage, {
    width,
    height: height + AXIS_H,
    xOnly: true,
    maxScale: Math.max(1, (fileBits() / 8) * (MAX_BYTE_PX / width)),
    onZoom: (v) => {
      view = v;
      reset.disabled = v.k === 1 && v.x === 0;
      opts.zoom?.set(v.k === 1 && v.x === 0 ? null : [invertX(v, 0) / width, invertX(v, width) / width]);
      hover = null;
      paint();
    },
  });
  zoom = pan;
  reset.addEventListener("click", () => pan.home(true));
  if (kept !== null) pan.zoomTo({ x0: kept[0] * width, y0: 0, x1: kept[1] * width, y1: height + AXIS_H });

  /** What is under a pointer event. */
  const hitAt = (e: MouseEvent): ArcHit | null => {
    const r = stage.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    if (y > height) return null;
    const above = y < (barTop + barBottom) / 2;
    const side = shapes.filter((s) => s.up === above);
    return nearestArc(side, x, y, above ? barTop : barBottom);
  };

  /** The end a hit stands for: the one under the pointer, or the field the
   *  arc points to when the pointer is on the curve. */
  const endOf = (h: ArcHit): ArcEnd | null => {
    const a = h.shape.arcs[0];
    if (a === undefined) return null;
    return h.at === "from" ? a.from : a.to;
  };

  const stopTip = hoverTip(stage, (e) => {
    const h = hitAt(e);
    // Shapes are made again on every paint, so the same arc is told by what
    // it draws rather than by the object.
    const changed = h?.shape.arcs[0] !== hover?.shape.arcs[0] || h?.shape.arcs.length !== hover?.shape.arcs.length;
    hover = h;
    const end = h === null ? null : endOf(h);
    myHover = end === null ? null : { startBit: end.startBit, endBit: Math.max(end.endBit, end.startBit + 8) };
    bus.publish({ hover: myHover });
    if (changed) paint();
    return h === null ? null : card(h);
  });
  const leave = (): void => {
    if (hover !== null) {
      hover = null;
      paint();
    }
    myHover = null;
    bus.publish({ hover: null });
  };
  stage.addEventListener("pointerleave", leave);
  const click = (e: MouseEvent): void => {
    const h = hitAt(e);
    if (h === null) return;
    const end = endOf(h);
    if (end === null) return;
    if (end.path.length > 0) goToField(doc, { path: end.path, startBit: end.startBit, endBit: end.endBit });
    else goToBits(doc, [{ startBit: end.startBit, endBit: Math.max(end.endBit, end.startBit + 8) }]);
  };
  stage.addEventListener("click", click);

  /** The hover card for what the pointer is on. */
  const card = (h: ArcHit): HTMLElement => {
    const s = h.shape;
    const word = ROLE_GROUP[s.role] ?? s.role;
    if (s.arcs.length > 1) {
      const from = s.arcs.map((a) => a.from.startBit);
      const to = s.arcs.map((a) => a.to.startBit);
      const where = T.bundleWhere(formatOffset(Math.min(...from)), formatOffset(Math.max(...from)), formatOffset(Math.min(...to)), formatOffset(Math.max(...to)));
      return tipCard(T.bundleTitle(s.arcs.length, word.toLowerCase()), [where], T.bundleHint);
    }
    const a = s.arcs[0] as PointerArc;
    // The core leaves the value out where it could not read it.
    const lines = [a.value === "" ? a.from.label : T.holds(a.from.label, a.value)];
    const worked = workedOut(doc, a);
    if (worked !== null) lines.push(worked);
    const r = reach(a);
    return tipCard(T.arcTitle(word, a.to.label), lines, T.reach(formatOffset(a.from.startBit), formatOffset(a.to.startBit), r.bytes, r.forward));
  };

  const stopLinked = subscribeWhileConnected(bus, host, (state, changed) => {
    if (changed === "hover" && state.hover === myHover) return;
    paint();
  });
  // A figure in a listing card is thrown away with the card, untold; it lets
  // go of the walk the first time it hears from it off the page.
  let seen = false;
  const stopSource = source.subscribe(() => {
    if (host.isConnected) seen = true;
    else if (seen) {
      destroy();
      return;
    }
    refilter();
    drawChips();
    paint();
  });
  const media = typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)") : null;
  const themeChanged = (): void => {
    ink = null;
    chipsKey = "";
    drawChips();
    paint();
  };
  media?.addEventListener("change", themeChanged);

  void loadAxis().then((loaded) => {
    axis = loaded;
    drawAxis();
  });

  function destroy(): void {
    pan.destroy();
    stopTip();
    stopLinked();
    stopSource();
    hideTip();
    media?.removeEventListener("change", themeChanged);
    stage.removeEventListener("pointerleave", leave);
    stage.removeEventListener("click", click);
  }

  sizeCanvas();
  refilter();
  drawChips();
  paint();
  // The caller puts the figure on the page after this returns.
  setTimeout(paint, 0);
  source.ensure();

  return {
    el: host,
    board: pan,
    resize: (w) => {
      const next = Math.max(80, w);
      if (next === width) return;
      width = next;
      sizeCanvas();
      pan.resize(width, height + AXIS_H);
      paint();
    },
    destroy,
  };
}

/** The expression the template writes for the arc, with the values put in,
 *  when it says more than the field and its value already do: `len - 4 =
 *  20 - 4 = 16`. Null for a plain offset. */
function workedOut(doc: Doc, a: PointerArc): string | null {
  if (a.to.path.length === 0) return null;
  const r = doc.relations(a.to.path);
  if (r.status !== "ok") return null;
  const rel = r.node.find((x) => x.role === a.role);
  if (rel === undefined) return null;
  const steps: string[] = [];
  for (const step of [rel.written, rel.substituted, rel.result]) if (step !== "" && step !== steps[steps.length - 1]) steps.push(step);
  // `entries[2].offset = 15` is the line above already.
  if (steps.length <= 1 || (steps.length === 2 && steps[0] === a.from.label && steps[1] === a.value)) return null;
  return steps.join(" = ");
}

// ----- the rail tab -----

/** Room for arcs on each side of the bar, in the rail. */
const RAIL_ARC_PX = 96;
/** And in a card under a heading, which sits among rows of text. */
const CARD_ARC_PX = 56;

function mountTab(doc: Doc, width: number): OverviewTabPanel {
  const el = document.createElement("div");
  el.className = "viz-arcs-panel";
  let fig: ArcFigure | null = null;
  let w = width;
  return {
    el,
    setShown: (shown) => {
      if (!shown) return;
      if (fig === null) {
        fig = drawPointerArcs(doc, pointersFor(doc), { width: w, arcPx: RAIL_ARC_PX, within: null });
        el.replaceChildren(fig.el);
      } else pointersFor(doc).ensure();
    },
    resize: (next) => {
      w = next;
      fig?.resize(next);
    },
    pump: () => {
      if (fig !== null) pointersFor(doc).ensure();
    },
  };
}

// ----- the card under a heading -----

const CARD_ID = "pointer-arcs";

/** Where each card's zoom was left, by document and path, since the listing
 *  builds its cards again on every paint. */
const cardZooms = new WeakMap<Doc, Map<string, readonly [number, number]>>();

function cardOffered(n: TemplateNode): boolean {
  return n.composite && n.size_bits > 0 && !n.decoded && n.path.length > 0;
}

/** How wide the last card was, to draw the next one at before it is on the
 *  page and can be asked. */
let lastCardWidth = 800;

function drawCard(doc: Doc, node: TemplateNode): HTMLElement {
  const el = document.createElement("div");
  el.className = "viz-arcs-card";
  const zooms = cardZooms.get(doc) ?? new Map<string, readonly [number, number]>();
  cardZooms.set(doc, zooms);
  const key = node.path.join(".");
  // Drawn whole before the listing places it, since the listing measures a
  // card's height as soon as it is up, and at a fixed height (`fixedRoom`),
  // so arcs found after that do not make it taller than it was measured.
  const fig = drawPointerArcs(doc, pointersFor(doc), {
    width: lastCardWidth,
    arcPx: CARD_ARC_PX,
    fixedRoom: true,
    within: node.path,
    zoom: { get: () => zooms.get(key) ?? null, set: (r) => (r === null ? zooms.delete(key) : zooms.set(key, r)) },
  });
  el.append(fig.el);
  // The listing does not say how wide its cards are, so the card asks its
  // own box once it is on the page, and again when that changes.
  const watch = new ResizeObserver((entries) => {
    if (!el.isConnected) {
      watch.disconnect();
      fig.destroy();
      return;
    }
    // The content box: the listing indents its items with padding.
    const w = Math.floor(entries[0]?.contentRect.width ?? 0);
    if (w <= 0) return;
    lastCardWidth = w;
    fig.resize(w);
  });
  watch.observe(el);
  return el;
}

/** Put the arcs in the rail and under listing headings. */
export function register(): void {
  registerOverviewTab({
    id: "pointer-arcs",
    label: T.tab,
    title: T.tabTitle,
    offers: (doc) => doc.template !== null,
    mount: mountTab,
  });
  registerKindCard({
    id: CARD_ID,
    offers: (_doc, n) => cardOffered(n),
    draw: (c, item) => drawCard(c.doc, item.node),
  });
  registerHeadingButton({
    id: CARD_ID,
    // Offered on any structure until the file's walk has finished, and after
    // that only where an arc has an end. The button does not start the walk:
    // the card it opens does.
    applies: (c, node) => {
      if (!cardOffered(node)) return false;
      const p = pointersFor(c.doc);
      if (!p.started || p.state !== "done") return true;
      const span = { startBit: node.offset_bits, endBit: node.offset_bits + node.size_bits };
      return p.arcs.some((a) => arcTouches(a, node.path, span));
    },
    draw: (c, node) => {
      const key = askedCardKey(CARD_ID, node.path);
      return headingButton(T.button, T.buttonTitle, c.cards.has(key), () => c.toggleCard(key));
    },
  });
}

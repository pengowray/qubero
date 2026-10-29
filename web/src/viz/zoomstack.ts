// Zoom stack: where the field under the cursor is, from the whole file down,
// as a stack of strips in the At cursor panel.
//
// Candidate `locator-family` in docs/DESIGN-diagrams.md, section 4.2; built
// by agent D of the first batch (section 5.3), with the live file map
// (`filemap.ts`), the bars under the trail's crumbs and the report's position
// map. Test samples: zip/streamed-data-descriptors.zip, pdf/linearized.pdf.
//
// The top strip is the file, its parts across their bytes and the part the
// cursor is in lit. Each strip under it is the lit part of the one above,
// drawn the full width with its own fields, joined to it by lens lines, down
// to the structure the field is in. Which steps of the path get a strip is
// `stacklevels.ts`. A field inside a compressed stream is at an offset of the
// bytes the stream unpacks to, which is nowhere in the file: its strip hangs
// from the compressed run's bar by a dashed lens and is captioned as unpacked,
// so the two axes are never read as one.
//
// Each strip is its own axis, so each zooms on its own: the wheel zooms about
// the pointer and a drag pans, across only (`board.ts`), and the lens lines
// follow both ends. Names go inside their bars where they fit at the zoom
// they are drawn at (`fit.ts`) and are otherwise left to the hover card,
// which names the field, its type, its bytes and its share of the structure.
// A click goes to the field (`pick.ts`). Pointing at a bar of a strip in the
// file marks its bytes in the hex view (`linked.ts`); the field the cursor is
// in is lit from the shared selection, so moving between the fields of one
// structure keeps the stack and its zoom.

import type { Doc, TemplateNode } from "../doc.ts";
import { fieldClass, sectionColor } from "../fieldstyle.ts";
import { rangeWords, shareWords } from "../filemap.ts";
import { bitSizeText, childWord, countText, VIZ } from "../strings.ts";
import { board, type Board } from "./board.ts";
import { applyX, invertX, type View } from "./boardmath.ts";
import { canvasMeasure, ellipsise, type Measure } from "./fit.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { linkedFor, subscribeWhileConnected } from "./linked.ts";
import { LOD } from "./lod.ts";
import { mapCells, type MapCell } from "./mapcells.ts";
import { goToField, nearestOnAxis, type AxisTarget } from "./pick.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";
import { levelDomain, stackPlan, type StackLevel } from "./stacklevels.ts";

/** Most children read for one strip. A list of fifty thousand records is cut
 *  to its first ones; the strip says so in its caption. */
const MAX_CHILDREN = 2000;

/** The strips' parts, top to bottom: the caption line, the bars, and the
 *  lens down to the next strip. */
const CAPTION_H = 15;
const BAR_H = 16;
const LENS_H = 12;
const LABEL_PX = 11;
const LABEL_PAD = 3;
const LABEL_MIN_CHARS = 3;
/** How wide one byte may get at the deepest zoom. */
const MAX_BYTE_PX = 48;

const SVG = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

let measure: Measure | null = null;
function measurer(): Measure {
  measure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return measure;
}

/** What a crumb of the trail would call a structure. */
function labelOf(n: TemplateNode): string {
  return n.composite && n.type !== n.name && !n.inline ? `${n.name} (${n.type})` : n.name;
}

/** One strip's data: the plan's level and the children drawn on it. */
export type StackStrip = {
  readonly level: StackLevel<TemplateNode>;
  readonly kids: readonly TemplateNode[];
  /** The children in the strip's own space with bytes to draw, cut into
   *  cells in file order (`mapCells`): a part the others sit inside is drawn
   *  as the pieces around them. */
  readonly cells: readonly MapCell[];
  readonly domain: readonly [number, number];
};

/** The strips for the field at `path`, or null while any of them is still
 *  being read. */
export type ZoomStackData = { readonly strips: StackStrip[]; readonly skipped: number; readonly focus: readonly number[] };

export function zoomStackData(doc: Doc, path: readonly number[]): ZoomStackData | null {
  const chain: TemplateNode[] = [];
  for (let i = 0; i <= path.length; i++) {
    const r = doc.templateNode(path.slice(0, i));
    if (r.status !== "ok") return null;
    chain.push(r.node);
  }
  const { levels, skipped } = stackPlan(chain);
  if (levels.length === 0) return null;
  const strips: StackStrip[] = [];
  for (const level of levels) {
    const p = level.parent;
    const reply = doc.templateChildren(p.path, 0, Math.min(p.child_count, MAX_CHILDREN));
    if (reply.status !== "ok") return null;
    // A field that points at what it reads has no bytes of its own; what it
    // points at is drawn in its place, where its bytes are, as the listing
    // draws it (`hop` in `flatten`).
    const kids: TemplateNode[] = [];
    for (const k of reply.node) {
      let n = k;
      while (n.size_bits === 0 && n.composite && n.child_count === 1) {
        const to = doc.templateNode([...n.path, 0]);
        if (to.status !== "ok") return null;
        n = to.node;
      }
      if (n.space === p.space) kids.push(n);
    }
    const domain = levelDomain(p, kids);
    strips.push({ level, kids, cells: mapCells(kids.map((k) => ({ offsetBits: k.offset_bits, sizeBits: k.size_bits })), domain[0], domain[1]), domain });
  }
  return { strips, skipped, focus: path };
}

export type ZoomStack = { readonly el: HTMLElement; destroy(): void };

/** One strip as drawn: its SVG, its board and the view it is zoomed to. */
type Drawn = {
  readonly strip: StackStrip;
  readonly svg: SVGSVGElement;
  readonly bars: SVGGElement;
  readonly names: SVGGElement;
  readonly lens: SVGSVGElement | null;
  board: Board | null;
  view: View;
  /** The cells as hit targets, in layout pixels. */
  readonly targets: AxisTarget<number>[];
};

/** Draw the stack `width` pixels wide. */
export function drawZoomStack(doc: Doc, data: ZoomStackData, opts: { readonly width: number }): ZoomStack {
  const width = Math.max(60, opts.width);
  const host = document.createElement("figure");
  host.className = "viz-strip viz-stack";
  const head = document.createElement("figcaption");
  head.className = "viz-strip-head";
  const title = document.createElement("span");
  title.className = "viz-strip-title";
  title.textContent = VIZ.locator.title;
  const reset = document.createElement("button");
  reset.type = "button";
  reset.className = "viz-reset";
  reset.textContent = VIZ.resetZoom;
  reset.disabled = true;
  head.append(title, reset);
  host.append(head);

  const bus = linkedFor(doc);
  let selected: readonly number[] = data.focus;
  const drawn: Drawn[] = [];

  const x = (d: Drawn, bit: number): number => {
    const [a, b] = d.strip.domain;
    return applyX(d.view, ((bit - a) / (b - a)) * width);
  };

  const drawLens = (j: number): void => {
    const below = drawn[j];
    const above = drawn[j - 1];
    if (below === undefined || above === undefined || below.lens === null) return;
    const src = below.strip.level.source;
    if (src === null) return;
    const a0 = Math.max(0, Math.min(width, x(above, src.offsetBits)));
    const a1 = Math.max(0, Math.min(width, x(above, src.offsetBits + src.sizeBits)));
    const b0 = Math.max(0, Math.min(width, applyX(below.view, 0)));
    const b1 = Math.max(0, Math.min(width, applyX(below.view, width)));
    const poly = svg("polygon", { points: `${a0},0 ${Math.max(a1, a0 + 1)},0 ${b1},${LENS_H} ${b0},${LENS_H}`, class: below.strip.level.unpackedFrom !== null ? "viz-lens is-unpacked" : "viz-lens" });
    below.lens.replaceChildren(poly);
  };

  const drawStrip = (j: number): void => {
    const d = drawn[j] as Drawn;
    const { strip } = d;
    const top = j === 0;
    const rects: SVGRectElement[] = [];
    const names: SVGTextElement[] = [];
    d.targets.length = 0;
    strip.cells.forEach((cell, i) => {
      const x0 = x(d, cell.start);
      const x1 = x(d, cell.end);
      d.targets.push({ start: invertX(d.view, x0), end: invertX(d.view, x1), value: i });
      if (x1 < 0 || x0 > width) return;
      const kid = cell.owner >= 0 ? (strip.kids[cell.owner] as TemplateNode) : null;
      const w = Math.max(1, x1 - x0);
      const r = svg("rect", { x: x0, y: 0, width: w, height: BAR_H });
      if (kid === null) r.setAttribute("class", "viz-stack-hole");
      else {
        // The top strip goes by the listing's part colours, so a part is the
        // same colour here as on its heading; the strips under it by kind.
        const onPath = startsWith(selected, kid.path);
        r.setAttribute("class", `viz-strip-bar ${top ? "" : fieldClass(kid.kind)}${onPath ? " is-on" : ""}`);
        if (top) r.style.setProperty("--field-color", sectionColor(cell.owner));
        if (w - 2 * LABEL_PAD >= LABEL_PX) {
          const left = Math.max(0, x0);
          const right = Math.min(width, x1);
          const text = ellipsise(kid.name, right - left - 2 * LABEL_PAD, LABEL_PX, measurer(), LABEL_MIN_CHARS);
          if (text !== null) {
            const t = svg("text", { x: left + LABEL_PAD, y: BAR_H / 2, "dominant-baseline": "central" });
            t.textContent = text;
            names.push(t);
          }
        }
      }
      rects.push(r);
    });
    d.bars.replaceChildren(...rects);
    d.names.replaceChildren(...names);
    d.targets.sort((p, q) => p.start - q.start);
  };

  const anyZoomed = (): boolean => drawn.some((d) => d.view.k !== 1 || d.view.x !== 0);

  data.strips.forEach((strip, j) => {
    const level = strip.level;
    const row = document.createElement("div");
    row.className = "viz-stack-level";
    let lens: SVGSVGElement | null = null;
    if (j > 0) {
      lens = svg("svg", { width, height: LENS_H, class: "viz-stack-lens", "aria-hidden": "true" });
      row.append(lens);
    }
    const caption = document.createElement("div");
    caption.className = "viz-stack-caption";
    const name = level.unpackedFrom !== null ? VIZ.locator.unpacked(level.unpackedFrom.name) : labelOf(level.parent);
    // The levels a deep path leaves out are between the first strip and the
    // second, and said where the trail says them, before the second's name.
    const said = j === 1 && data.skipped > 0 ? VIZ.locator.levelsBetween(data.skipped, name) : name;
    const more = level.parent.child_count > strip.kids.length && level.parent.child_count > MAX_CHILDREN;
    caption.textContent = more ? `${said} · ${VIZ.childStrip.firstOf(MAX_CHILDREN, level.parent.child_count, childWord(level.parent))}` : said;
    caption.style.height = `${CAPTION_H}px`;
    row.append(caption);
    const root = svg("svg", { width, height: BAR_H, class: "viz-strip-svg viz-stack-svg", role: "img" });
    root.setAttribute("aria-label", name);
    const bars = svg("g");
    const names = svg("g", { class: "viz-strip-names" });
    root.append(bars, names);
    row.append(root);
    host.append(row);
    const d: Drawn = { strip, svg: root, bars, names, lens, board: null, view: { k: 1, x: 0, y: 0 }, targets: [] };
    drawn.push(d);
    const bytes = Math.max(1, (strip.domain[1] - strip.domain[0]) / 8);
    d.board = board(root, {
      width,
      height: BAR_H,
      xOnly: true,
      maxScale: Math.max(1, bytes * (MAX_BYTE_PX / width)),
      onZoom: (v) => {
        d.view = v;
        drawStrip(j);
        drawLens(j);
        drawLens(j + 1);
        reset.disabled = !anyZoomed();
      },
    });

    const kidAt = (e: MouseEvent): TemplateNode | null => {
      const r = root.getBoundingClientRect();
      const at = invertX(d.view, e.clientX - r.left);
      const hit = nearestOnAxis(d.targets, at, LOD.targetPx / d.view.k);
      if (hit === null) return null;
      const cell = strip.cells[hit.value] as MapCell;
      return cell.owner >= 0 ? (strip.kids[cell.owner] as TemplateNode) : null;
    };
    hoverTip(root, (e) => {
      const kid = kidAt(e);
      // The hex view draws the file's bytes; a stream's offsets are not those.
      if (level.parent.space === 0) bus.publish({ hover: kid === null ? null : { startBit: kid.offset_bits, endBit: kid.offset_bits + kid.size_bits } });
      if (kid === null) return null;
      const value = kid.composite ? countText(kid.child_count, childWord(kid)) : kid.value;
      const share = shareWords(kid.size_bits, level.parent.size_bits);
      return tipCard(kid.name, [`${kid.type} · ${value}`, `${rangeWords({ offsetBits: kid.offset_bits, sizeBits: kid.size_bits }, kid.space)} · ${bitSizeText(kid.size_bits)}`, ...(share !== "" ? [VIZ.locator.shareOf(share, labelOf(level.parent))] : [])]);
    });
    root.addEventListener("pointerleave", () => bus.publish({ hover: null }));
    root.addEventListener("click", (e) => {
      const kid = kidAt(e);
      if (kid === null) return;
      hideTip();
      goToField(doc, { path: kid.path, startBit: kid.offset_bits, endBit: kid.offset_bits + kid.size_bits });
    });
  });
  reset.addEventListener("click", () => {
    for (const d of drawn) d.board?.home(true);
  });
  // The field the cursor is in is lit, whichever view moved the cursor.
  const stopLinked = subscribeWhileConnected(bus, host, (state, changed) => {
    if (changed !== "selection") return;
    if (state.selection === null) return;
    selected = state.selection.path;
    drawn.forEach((_, j) => drawStrip(j));
  });
  drawn.forEach((_, j) => {
    drawStrip(j);
    drawLens(j);
  });
  return {
    el: host,
    destroy: () => {
      for (const d of drawn) d.board?.destroy();
      stopLinked();
      hideTip();
    },
  };
}

/** Whether the path `a` goes through `b`: the cursor's field is `b` or is
 *  inside it. */
function startsWith(a: readonly number[], b: readonly number[]): boolean {
  return a.length >= b.length && b.every((s, i) => s === a[i]);
}

/** Put the stack in the At cursor panel, for every field below the file. */
export function register(): void {
  registerInspectorSection({
    id: "zoom-stack",
    applies: (_doc, n) => n.path.length > 0,
    // The structure the field is in, so moving between its fields keeps the
    // stack and its zoom; the new field is lit from the shared selection.
    key: (_doc, n, path) => (n.composite && n.child_count > 1 ? path : path.slice(0, -1)).join("/"),
    draw: ({ doc, path, width }) => {
      const data = zoomStackData(doc, path);
      return data === null ? null : drawZoomStack(doc, data, { width }).el;
    },
  });
}

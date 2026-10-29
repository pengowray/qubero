// Sparkline of one field across a run of records, beside the field's value in
// the listing.
//
// Candidate `record-small-multiples` in docs/DESIGN-diagrams.md, section 4.2,
// the run sparkline of its first-batch slice (section 5.3, agent I). The
// heading tile grid is `recordtiles.ts`.
//
// A list of records repeats the same fields in every record: each PNG chunk
// has a `length`, each MIDI event a `delta`, each ZIP entry its sizes. The
// listing shows one record's value per row, so seeing how the value moves
// across the list means reading every record. The sparkline puts the whole
// run on one line beside the first record's value: one point per record, in
// file order, with the smallest and largest value written at the two ends. A
// run longer than the line is pixels wide is drawn as the smallest and
// largest value in each pixel's stretch of records (`sparkmath.ts`), so a
// spike in a run of thousands is still there.
//
// Where it is drawn. On the rows of the first record of a list only
// (`viz/rowglyphs.ts`): the run belongs to the list, and repeating it on every
// record's row would be the same line two hundred times. The point for the
// record the cursor is in is marked, whichever view moved the cursor, so the
// line still says where a reader is in the run. Only for a number that varies
// across at least three records, and not for a checksum, whose values are
// meant to look like noise.
//
// Explorable: hovering the line gives the record and its value (`tip.ts`) and
// marks the field's bytes in the hex view (`linked.ts`); a click goes to that
// record's field (`pick.ts`). It is word sized, so it takes no wheel: the
// listing scrolls under it. The line is drawn by uPlot, loaded the first time
// a sparkline is drawn.

import "uplot/dist/uPlot.min.css";
import type uPlotType from "uplot";
import type { Doc, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { isElementName } from "../elementname.ts";
import { bitSizeText, VIZ } from "../strings.ts";
import { linkedFor } from "./linked.ts";
import { goToField } from "./pick.ts";
import { registerRowGlyph } from "./rowglyphs.ts";
import { binOf, binRecords, envelope, parseNumber, shortNumber, worthDrawing, type Envelope, type Step } from "./sparkmath.ts";
import { recordWord } from "./tilelayout.ts";
import { hideTip, tipAt, tipCard } from "./tip.ts";

/** Records read for one run. */
const READ_MAX = 4096;
const READ_PAGE = 512;
/** How far above the field the record may be: a ZIP entry's sizes are in its
 *  `body`, two steps down. */
const RECORD_DEPTH_MAX = 3;
/** The line's size, in CSS pixels: as tall as the row's text, and about as
 *  wide as a short value. */
const W = 72;
const H = 14;

/** One record's value. */
type Point = { readonly record: number; readonly name: string; readonly node: TemplateNode; readonly value: number };

export type Run = {
  readonly list: TemplateNode;
  readonly field: TemplateNode;
  readonly points: readonly Point[];
  readonly min: number;
  readonly max: number;
  readonly env: Envelope;
};

/** Whether a row could have a sparkline, from the node alone: a number with
 *  bytes, with a first element of a list a few steps above it. */
function candidate(n: TemplateNode): boolean {
  if (n.composite || n.size_bits === 0) return false;
  if (n.kind !== "uint" && n.kind !== "int" && n.kind !== "float") return false;
  const p = n.path;
  for (let d = 1; d <= RECORD_DEPTH_MAX && p.length - d - 1 >= 0; d++) if (p[p.length - d - 1] === 0) return true;
  return false;
}

/** Every child of `path`, or null while some are being read. */
function children(doc: Doc, path: readonly number[], count: number): TemplateNode[] | null {
  const out: TemplateNode[] = [];
  for (let from = 0; from < count; from += READ_PAGE) {
    const reply = doc.templateChildren(path, from, Math.min(count, from + READ_PAGE));
    if (reply.status === "error") return out;
    if (reply.status !== "ok") return null;
    out.push(...reply.node);
  }
  return out;
}

/** Waiting on bytes, so ask again when they land. */
const WAIT = Symbol("wait");

/**
 * The run the field at `n` is one value of: the list whose first record holds
 * it, and the same field in every record, found by the names and types of the
 * steps from the record down to it. Null where there is no run worth a line.
 */
function runOf(doc: Doc, n: TemplateNode): Run | null | typeof WAIT {
  const p = n.path;
  for (let d = 1; d <= RECORD_DEPTH_MAX && p.length - d - 1 >= 0; d++) {
    const recordPath = p.slice(0, p.length - d);
    if (recordPath[recordPath.length - 1] !== 0) continue;
    const rec = doc.templateNode(recordPath);
    if (rec.status !== "ok") return WAIT;
    if (!isElementName(rec.node.name)) continue;
    const up = doc.templateNode(recordPath.slice(0, -1));
    if (up.status !== "ok") return WAIT;
    const list = up.node;
    if (!list.list || list.child_count < 3) continue;
    // The steps from the record down to the field.
    const steps: Step[] = [];
    for (let s = recordPath.length + 1; s <= p.length; s++) {
      const at = doc.templateNode(p.slice(0, s));
      if (at.status !== "ok") return WAIT;
      steps.push({ name: at.node.name, type: at.node.type });
    }
    const records = children(doc, list.path, Math.min(list.child_count, READ_MAX));
    if (records === null) return WAIT;
    const points: Point[] = [];
    for (const [i, r] of records.entries()) {
      let at: TemplateNode | undefined = r;
      for (const step of steps) {
        if (at === undefined || !at.composite) {
          at = undefined;
          break;
        }
        const kids = doc.templateChildren(at.path, 0, Math.min(at.child_count, 64));
        if (kids.status === "pending") return WAIT;
        at = kids.status === "ok" ? kids.node.find((k) => k.name === step.name && k.type === step.type) : undefined;
      }
      if (at === undefined) continue;
      const value = parseNumber(at.edit_text);
      if (value !== null) points.push({ record: i, name: r.name, node: at, value });
    }
    const values = points.map((q) => q.value);
    if (!worthDrawing(values)) return null;
    return { list, field: n, points, min: Math.min(...values), max: Math.max(...values), env: envelope(values, W) };
  }
  return null;
}

type UPlot = typeof uPlotType;
let loading: Promise<UPlot> | null = null;
function loadUPlot(): Promise<UPlot> {
  loading ??= import("uplot").then((m) => m.default);
  return loading;
}

type Drawn = { readonly el: HTMLElement; destroy(): void };

/** A `#rrggbb` colour at a third of its strength, for a canvas, which takes
 *  no `color-mix`. */
function faint(hex: string): string {
  return /^#[0-9a-f]{6}$/i.test(hex) ? `${hex}55` : hex;
}

/** Draw the line for a run. */
export function drawSparkline(doc: Doc, run: Run): Drawn {
  const word = recordWord(run.list);
  const host = document.createElement("span");
  host.className = "viz-spark";
  host.setAttribute("role", "img");
  host.setAttribute("aria-label", VIZ.spark.label(run.field.name, run.points.length, word, shortNumber(run.min), shortNumber(run.max)));
  const lo = document.createElement("span");
  lo.className = "viz-spark-end";
  lo.textContent = shortNumber(run.min);
  const plot = document.createElement("span");
  plot.className = "viz-spark-plot";
  plot.style.width = `${W}px`;
  plot.style.height = `${H}px`;
  const hi = document.createElement("span");
  hi.className = "viz-spark-end";
  hi.textContent = shortNumber(run.max);
  host.append(lo, plot, hi);

  const env = run.env;
  const bins = env.min.length;
  const n = run.points.length;
  const listDepth = run.list.path.length;
  const bus = linkedFor(doc);
  /** Which point the cursor's record is, from the shared selection. */
  const markedOf = (path: readonly number[] | undefined): number | null => {
    if (path === undefined || path.length <= listDepth) return null;
    for (let i = 0; i < listDepth; i++) if (path[i] !== run.list.path[i]) return null;
    const record = path[listDepth] as number;
    const at = run.points.findIndex((q) => q.record === record);
    return at < 0 ? null : at;
  };
  let marked = markedOf(bus.state.selection?.path);
  let hovered: number | null = null;
  let u: uPlotType | null = null;
  let gone = false;

  const colour = (name: string, fallback: string): string => getComputedStyle(host).getPropertyValue(name).trim() || fallback;
  /** The dot for a point, drawn over the line after uPlot has drawn it. */
  const dot = (plotU: uPlotType, point: number | null, fill: string, r: number): void => {
    if (point === null) return;
    const j = binOf(point, n, bins);
    const v = run.points[point]?.value;
    if (v === undefined) return;
    const ctx = plotU.ctx;
    const x = plotU.valToPos(j, "x", true);
    const y = plotU.valToPos(v, "y", true);
    ctx.save();
    ctx.fillStyle = fill;
    ctx.strokeStyle = colour("--bg", "#fff");
    ctx.lineWidth = devicePixelRatio;
    ctx.beginPath();
    ctx.arc(x, y, r * devicePixelRatio, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    ctx.restore();
  };

  void loadUPlot().then((UPlot) => {
    if (gone) return;
    const xs = Array.from({ length: bins }, (_, j) => j);
    const line = colour("--fg", "#333");
    const pad = run.max === run.min ? 1 : (run.max - run.min) * 0.12;
    const series: uPlotType.Series[] = [{}];
    const data: uPlotType.AlignedData = env.exact ? [xs, [...env.min]] : [xs, [...env.min], [...env.max]];
    series.push({ stroke: line, width: 1, points: { show: false } });
    if (!env.exact) series.push({ stroke: line, width: 1, points: { show: false } });
    u = new UPlot(
      {
        width: W,
        height: H,
        cursor: { show: false },
        select: { show: false, left: 0, top: 0, width: 0, height: 0 },
        legend: { show: false },
        padding: [3, 3, 3, 3],
        scales: { x: { time: false }, y: { range: [run.min - pad, run.max + pad] } },
        axes: [{ show: false }, { show: false }],
        series,
        // The band between the smallest and largest line, faint, so the two
        // lines read as the edges of one shape.
        ...(env.exact ? {} : { bands: [{ series: [2, 1], fill: faint(colour("--muted", "#888888")) }] }),
        hooks: {
          draw: [
            (plotU) => {
              dot(plotU, marked, colour("--accent", "#2457c5"), 2.5);
              if (hovered !== marked) dot(plotU, hovered, line, 2);
            },
          ],
        },
      },
      data,
      plot,
    );
  });

  /** The point under the pointer: the nearest bin, then the first record in
   *  it, which is the one the click goes to. */
  const pointAt = (e: PointerEvent | MouseEvent): number | null => {
    if (u === null) return null;
    const r = u.over.getBoundingClientRect();
    const j = Math.max(0, Math.min(bins - 1, u.posToIdx(e.clientX - r.left)));
    return env.first[j] ?? null;
  };

  const move = (ev: Event): void => {
    const e = ev as PointerEvent;
    const at = pointAt(e);
    if (at !== hovered) {
      hovered = at;
      u?.redraw(false, false);
    }
    const q = at === null ? undefined : run.points[at];
    if (q === undefined || at === null) {
      hideTip();
      return;
    }
    bus.publish({ hover: { startBit: q.node.offset_bits, endBit: q.node.offset_bits + q.node.size_bits } });
    const j = binOf(at, n, bins);
    const span = binRecords(j, n, bins);
    if (env.exact || span.to - span.from <= 1) {
      tipAt(tipCard(VIZ.spark.pointTitle(run.field.name, q.name), [q.node.value], `${formatOffset(q.node.offset_bits)} · ${bitSizeText(q.node.size_bits)}`), e.clientX, e.clientY);
    } else {
      const first = run.points[span.from]?.record ?? 0;
      const last = run.points[span.to - 1]?.record ?? 0;
      tipAt(
        tipCard(VIZ.spark.binTitle(run.field.name, first, last, word), [VIZ.spark.binLine(shortNumber(env.min[j] ?? 0), shortNumber(env.max[j] ?? 0))]),
        e.clientX,
        e.clientY,
      );
    }
  };
  const leave = (): void => {
    hideTip();
    bus.publish({ hover: null });
    if (hovered !== null) {
      hovered = null;
      u?.redraw(false, false);
    }
  };
  const click = (e: Event): void => {
    // The row under the line would otherwise take the click as a pick of
    // its own field.
    e.stopPropagation();
    const at = pointAt(e as MouseEvent);
    const q = at === null ? undefined : run.points[at];
    if (q === undefined) return;
    goToField(doc, { path: q.node.path, startBit: q.node.offset_bits, endBit: q.node.offset_bits + q.node.size_bits });
  };
  host.addEventListener("pointermove", move);
  host.addEventListener("pointerleave", leave);
  host.addEventListener("click", click);
  // Lives as long as the line does, not as long as it is on the page: the
  // listing puts the same element in the row each time it draws the row.
  const stopLinked = bus.subscribe((state, changed) => {
    if (changed !== "selection") return;
    const next = markedOf(state.selection?.path);
    if (next === marked) return;
    marked = next;
    u?.redraw(false, false);
  });
  return {
    el: host,
    destroy: () => {
      gone = true;
      stopLinked();
      u?.destroy();
      host.removeEventListener("pointermove", move);
      host.removeEventListener("pointerleave", leave);
      host.removeEventListener("click", click);
    },
  };
}

/** The lines drawn, by document and field, so a row drawn again on the next
 *  paint gets the same line; and the fields found to have none, so they are
 *  not looked for again. Dropped when the document changes, since an edit can
 *  change any value of the run. */
type Memo = { lines: Map<string, Drawn>; none: Set<string> };
const memo = new WeakMap<Doc, Memo>();

function memoFor(doc: Doc): Memo {
  let m = memo.get(doc);
  if (m === undefined) {
    const fresh: Memo = { lines: new Map(), none: new Set() };
    m = fresh;
    memo.set(doc, fresh);
    // Bytes arriving can complete a run that was cut short, and an edit can
    // change any value in it: either way what was found is stale.
    doc.onChange(() => {
      for (const d of fresh.lines.values()) d.destroy();
      fresh.lines.clear();
      fresh.none.clear();
    });
  }
  return m;
}

/** Add the sparkline to the listing's rows. */
export function register(): void {
  registerRowGlyph({
    id: "run-sparkline",
    applies: (_doc, n) => candidate(n),
    draw: (doc, n) => {
      const key = n.path.join(".");
      const m = memoFor(doc);
      const had = m.lines.get(key);
      if (had !== undefined) return had.el;
      if (m.none.has(key)) return null;
      // A checksum's values are meant to look like noise; a line of them
      // says nothing.
      const check = doc.checkOf(n.path);
      if (check.status === "ok" && check.node !== null) {
        m.none.add(key);
        return null;
      }
      const run = runOf(doc, n);
      if (run === WAIT) return null;
      if (run === null) {
        m.none.add(key);
        return null;
      }
      const drawn = drawSparkline(doc, run);
      m.lines.set(key, drawn);
      return drawn.el;
    },
  });
}

// Sparkline of a list of numbers, beside the list's row in the listing.
//
// Candidate `record-small-multiples` in docs/DESIGN-diagrams.md, section 4.2,
// the run sparkline of its first-batch slice (section 5.3, agent I). The
// heading tile grid is `recordtiles.ts`.
//
// The listing shows one element per row, so seeing how the numbers of a list
// move means reading every row. The sparkline puts the whole list on one
// line on the list's own row: one point per element, in file order, with the
// smallest and largest value written at the two ends. A list longer than the
// line is pixels wide is drawn as the smallest and largest value in each
// pixel's stretch of elements (`sparkmath.ts`), so a spike in a list of
// thousands is still there.
//
// Where it is drawn. On the row of a list whose elements are numbers, at
// least three of them, that vary. Until 2026-11-27 it sat beside a number in
// the first record of a list of records and showed that field across the
// records; the owner read that as a picture of the one integer beside it and
// asked for it on arrays of numbers only. The table view is where one field
// is compared across records. The point for the element the cursor is in is
// marked, whichever view moved the cursor.
//
// The run is read a slice at a time with the page given its turn in between
// (`readahead.ts`), so opening a directory of thousands of records does not
// stop the page; the line appears in the row when the reading is done. It
// sits at the right of the value cell, apart from the field's own value, so
// the run's smallest value is not read as part of it.
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
import { bitSizeText, VIZ } from "../strings.ts";
import { linkedFor } from "./linked.ts";
import { goToField } from "./pick.ts";
import { registerRowGlyph } from "./rowglyphs.ts";
import { recordsOf, queued, walk, WAIT, type Wait, type Walk } from "./readahead.ts";
import { binOf, binRecords, envelope, parseNumber, sameRun, shortNumber, worthDrawing, type Envelope } from "./sparkmath.ts";
import { recordWord } from "./tilelayout.ts";
import { hideTip, tipAt, tipCard } from "./tip.ts";

/** Records read for one run. */
const READ_MAX = 4096;
/** The line's size, in CSS pixels: as tall as the row's text, and about as
 *  wide as a short value. */
const W = 72;
const H = 14;

/** One record's value. */
type Point = { readonly record: number; readonly name: string; readonly node: TemplateNode; readonly value: number };

export type Run = {
  readonly list: TemplateNode;
  readonly field: TemplateNode;
  /** How many records were read. More than the points where some records
   *  are of a type without the field. */
  readonly records: number;
  readonly points: readonly Point[];
  readonly min: number;
  readonly max: number;
  readonly env: Envelope;
};

/** The names of a checksum or hash field, whose values are meant to look
 *  like noise. */
const CHECKSUM_NAME = /^(crc\d*|adler\d*|checksum|hash|md5|sha\d*)(_|$)|_(crc\d*|checksum|hash)$/i;

/** Whether a row could have a sparkline, from the node alone: a list with
 *  bytes and at least three elements. Whether those elements are numbers is
 *  not known until they are read (`runOf`). */
function candidate(n: TemplateNode): boolean {
  return n.list && n.composite && n.size_bits > 0 && n.child_count >= 3;
}

/**
 * The run of numbers the list at `n` holds, one point per element. Null where
 * the elements are not numbers, or there is no run worth a line. Read a slice
 * at a time (`readahead.ts`): a list of thousands takes longer than a press
 * may.
 *
 * Only on a list of numbers. The line used to sit beside a number in the
 * first record of a list and show that field across every record, which read
 * as a picture of the one number it was next to.
 */
async function runOf(w: Walk, n: TemplateNode): Promise<Run | null | Wait> {
  const records = await recordsOf(w, n, Math.min(n.child_count, READ_MAX));
  if (records === WAIT || !w.alive()) return WAIT;
  const points: Point[] = [];
  for (const [i, r] of records.entries()) {
    if (r.composite || (r.kind !== "uint" && r.kind !== "int" && r.kind !== "float")) return null;
    const value = parseNumber(r.edit_text);
    if (value !== null) points.push({ record: i, name: r.name, node: r, value });
  }
  const values = points.map((q) => q.value);
  if (!worthDrawing(values)) return null;
  return { list: n, field: n, records: records.length, points, min: Math.min(...values), max: Math.max(...values), env: envelope(values, W) };
}

type UPlot = typeof uPlotType;
let loading: Promise<UPlot> | null = null;
function loadUPlot(): Promise<UPlot> {
  loading ??= import("uplot").then((m) => m.default);
  return loading;
}

type Drawn = {
  readonly el: HTMLElement;
  /** Draw again once the line is back on the page, for a line the listing
   *  has just put into a new row: the cursor may have moved meanwhile. */
  refresh(): void;
  destroy(): void;
};

/** A `#rrggbb` colour at a third of its strength, for a canvas, which takes
 *  no `color-mix`. */
function faint(hex: string): string {
  return /^#[0-9a-f]{6}$/i.test(hex) ? `${hex}55` : hex;
}

/** Draw the line for a run into `host`, a fresh one unless given. */
export function drawSparkline(doc: Doc, run: Run, host: HTMLElement = document.createElement("span")): Drawn {
  const word = recordWord(run.list);
  host.className = "viz-spark";
  host.hidden = false;
  host.setAttribute("role", "img");
  host.setAttribute("aria-label", VIZ.spark.label(run.field.name, run.points.length, run.records, word, shortNumber(run.min), shortNumber(run.max)));
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
  const values = run.points.map((q) => q.value);
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
      // A flat stretch of the line is said once, on whichever point of it is
      // under the pointer: the run of records that hold the same value.
      const lines = [q.node.value];
      const same = sameRun(values, at);
      if (same.to - same.from > 1) {
        const first = run.points[same.from]?.record ?? 0;
        const last = run.points[same.to - 1]?.record ?? 0;
        lines.push(VIZ.spark.sameRun(same.to - same.from, word, first, last));
      }
      tipAt(tipCard(VIZ.spark.pointTitle(run.field.name, q.name), lines, `${formatOffset(q.node.offset_bits)} · ${bitSizeText(q.node.size_bits)}`), e.clientX, e.clientY);
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
    refresh: () => requestAnimationFrame(() => u?.redraw(false, false)),
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

/** A line still being worked out: an empty, hidden span in the row, filled
 *  when the run has been read. The listing puts the same element in the row
 *  on every paint, so the line appears where the row is without the row being
 *  drawn again. */
function pending(doc: Doc, n: TemplateNode, key: string, m: Memo): Drawn {
  const host = document.createElement("span");
  host.hidden = true;
  let inner: Drawn | null = null;
  let gone = false;
  const w = walk(doc);
  void queued(async () => {
    if (gone || !w.alive()) return;
    const run = await runOf(w, n);
    if (gone || !w.alive()) return;
    if (run === WAIT) {
      // Asked again when the bytes land: that change drops this entry.
      return;
    }
    if (run === null) {
      m.none.add(key);
      return;
    }
    inner = drawSparkline(doc, run, host);
  });
  return {
    el: host,
    refresh: () => inner?.refresh(),
    destroy: () => {
      gone = true;
      inner?.destroy();
    },
  };
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
      if (had !== undefined) {
        had.refresh();
        return had.el;
      }
      if (m.none.has(key)) return null;
      // A checksum's values are meant to look like noise; a line of them
      // says nothing.
      // Not every checksum has a check on it: a ZIP local header's crc32 is
      // checked only where the data descriptor repeats it, so its name says
      // what it is too.
      const check = doc.checkOf(n.path);
      if ((check.status === "ok" && check.node !== null) || CHECKSUM_NAME.test(n.name)) {
        m.none.add(key);
        return null;
      }
      const drawn = pending(doc, n, key, m);
      m.lines.set(key, drawn);
      return drawn.el;
    },
  });
}

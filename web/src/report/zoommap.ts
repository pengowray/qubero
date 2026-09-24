// The whole-file map with semantic zoom.
//
// Three rows over one stretch of the file: the parts, the fields, and a strip
// that says what kind of byte each one is. The map does not magnify, it
// redraws: zoomed out, the strip is the byte-class scan the rail's map is
// drawn from; closer in, the bytes of the window are read and classed one
// pixel at a time; past about 14 pixels a byte, the strip is the bytes in hex.
// The fields row comes from `spans`, which is windowed already, so a zoomed-in
// map of a gigabyte file asks for a screenful of fields and no more.
//
// Wheel zooms around the pointer, drag pans, double-click goes back to the
// whole file. Hover says what is under the pointer; a click puts the cursor on
// the field or part there, the way a byte reference does.

import type { Span } from "../doc.ts";
import { svgEl } from "../dom.ts";
import { byteClassColor, fieldClass } from "../fieldstyle.ts";
import { formatOffset } from "../format.ts";
import { ok } from "./model.ts";
import type { Extent } from "./partrules.ts";
import { WAIT, type ReportCtx } from "./section.ts";
import { hideTip, tipAt } from "./tip.ts";
import { bitsText, RV, shareOfFile } from "./text.ts";

export type MapPart = {
  readonly startBit: number;
  readonly endBit: number;
  readonly label: string;
  readonly color: string;
  readonly path?: readonly number[];
};

/** Rows of the figure, in px. */
const PART_Y = 0;
const PART_H = 24;
const FIELD_Y = 28;
const FIELD_H = 18;
const STRIP_Y = 50;
const STRIP_H = 10;
const BYTE_H = 20;
const AXIS_Y = 74;
const HEIGHT = 92;
/** Pixels a byte from which the strip writes each byte in hex. */
const HEX_PPB = 14;
/** The widest window the strip reads and classes itself. Past this it uses
 *  the whole-file scan, whose buckets are coarser but already paid for. */
const LOCAL_BYTES = 256 * 1024;
/** Fields asked for one window. More than this and the row says to zoom in
 *  rather than drawing the first few hundred as if they were all of them. */
const SPAN_MAX = 600;
/** Classes of the byte strip, as the rail numbers them. */
const ZERO = 0;
const TEXT = 2;
const OTHER = 3;

type SpanCache = { readonly key: string; readonly spans: readonly Span[] | null; readonly truncated: boolean };

export class ZoomMap {
  readonly el: HTMLElement;
  private readonly svg: SVGSVGElement;
  private readonly range: HTMLElement;
  private lo = 0;
  private hi: number;
  private readonly size: number;
  private lit: readonly Extent[] = [];
  private spans: SpanCache = { key: "", spans: null, truncated: false };
  private frame = 0;
  private drag: { x: number; lo: number; hi: number; moved: boolean } | null = null;
  /** True once the byte scan has finished and the window's fields are in. */
  private settled = false;
  /** True once the map has been drawn on the page, so a map not on the page
   *  yet is not taken for one that has left it. */
  private drawnOnce = false;

  constructor(
    private readonly ctx: ReportCtx,
    private readonly parts: readonly MapPart[],
  ) {
    this.size = ctx.doc.lengthBytes;
    this.hi = Math.max(1, this.size);
    this.el = document.createElement("div");
    this.el.className = "rv-map";
    this.svg = svgEl("svg", { class: "rv-map-svg", height: String(HEIGHT), role: "img", "aria-label": RV.mapCaptionLead });
    this.range = document.createElement("div");
    this.range.className = "rv-map-range";
    this.el.append(this.svg, this.range);
    this.wire();
    new ResizeObserver(() => this.redraw()).observe(this.el);
    // A window zoomed into after the map first settled can be one whose bytes
    // or fields have not arrived. Their arrival is a change to the document,
    // and the map draws again for it until it has what it asked for. A map
    // that has left the page stops listening.
    const stop = ctx.doc.onChange(() => {
      if (!this.el.isConnected && this.drawnOnce) {
        stop();
        return;
      }
      if (!this.settled) this.redraw();
    });
  }

  /** Called while the report is live: draws again with whatever the byte scan
   *  and the fields have got to. True once there is nothing more to wait for. */
  update(): boolean {
    this.draw();
    return this.settled;
  }

  /** Light these stretches, or none. */
  highlight(extents: readonly Extent[]): void {
    this.lit = extents;
    this.redraw();
  }

  /** Show `[lo, hi)` bytes. */
  show(lo: number, hi: number): void {
    this.lo = lo;
    this.hi = hi;
    this.clamp();
    this.redraw();
  }

  private clamp(): void {
    const min = Math.min(this.size, Math.max(4, this.width() / 40));
    let span = Math.max(min, Math.min(this.size, this.hi - this.lo));
    if (!Number.isFinite(span) || span <= 0) span = Math.max(1, this.size);
    if (this.lo < 0) this.lo = 0;
    this.hi = this.lo + span;
    if (this.hi > this.size) {
      this.hi = this.size;
      this.lo = Math.max(0, this.hi - span);
    }
  }

  private width(): number {
    return this.el.clientWidth || 800;
  }

  private redraw(): void {
    if (this.frame !== 0) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.draw();
    });
  }

  private draw(): void {
    const W = this.width();
    const { lo, hi } = this;
    const ppb = W / Math.max(1e-9, hi - lo);
    const X = (o: number): number => (o - lo) * ppb;
    this.svg.setAttribute("viewBox", `0 0 ${W} ${HEIGHT}`);
    this.svg.setAttribute("width", String(W));
    const out: SVGElement[] = [];
    // The parts.
    for (const p of this.parts) {
      const a = p.startBit / 8;
      const b = p.endBit / 8;
      if (b <= lo || a >= hi) continue;
      const x0 = Math.max(0, X(a));
      const x1 = Math.min(W, X(b));
      const w = Math.max(0.6, x1 - x0 - (x1 - x0 > 3 ? 1 : 0));
      out.push(svgEl("rect", { x: String(x0), y: String(PART_Y), width: String(w), height: String(PART_H), fill: p.color, class: "rv-map-part" }));
      if (x1 - x0 > p.label.length * 6.6 + 10) {
        const t = svgEl("text", { x: String(x0 + 5), y: String(PART_Y + 16), class: "rv-map-partlabel" });
        t.textContent = p.label;
        out.push(t);
      }
    }
    // The fields of the window.
    const spans = this.windowSpans(lo, hi);
    if (spans.truncated) {
      const t = svgEl("text", { x: "4", y: String(FIELD_Y + 13), class: "rv-map-hint" });
      t.textContent = RV.mapZoomForFields;
      out.push(t);
    } else if (spans.spans !== null) {
      for (const s of spans.spans) {
        const a = s.offset_bits / 8;
        const b = (s.offset_bits + s.size_bits) / 8;
        if (b <= lo || a >= hi || s.size_bits === 0) continue;
        const x0 = Math.max(0, X(a));
        const x1 = Math.min(W, X(b));
        const w = Math.max(0.5, x1 - x0 - (x1 - x0 > 3 ? 1 : 0));
        const r = svgEl("rect", { x: String(x0), y: String(FIELD_Y), width: String(w), height: String(FIELD_H), class: `rv-map-field ${s.gap ? "is-gap" : fieldClass(s.kind)}` });
        out.push(r);
        const label = s.count > 0 ? `${s.name} ×${s.count.toLocaleString()}` : s.name;
        if (x1 - x0 > label.length * 6 + 8) {
          const t = svgEl("text", { x: String(x0 + 3), y: String(FIELD_Y + 13), class: "rv-map-fieldlabel" });
          t.textContent = label;
          out.push(t);
        }
      }
    }
    // The byte strip.
    let stripDone = true;
    if (ppb >= HEX_PPB) stripDone = this.drawBytes(out, lo, hi, X, ppb);
    else if (hi - lo <= LOCAL_BYTES) stripDone = this.drawLocal(out, lo, hi, W, ppb) || this.drawScan(out, lo, hi, X, W);
    else stripDone = this.drawScan(out, lo, hi, X, W);
    // What is lit from elsewhere: a ledger row under the pointer.
    for (const e of this.lit) {
      const a = e.offsetBits / 8;
      const b = (e.offsetBits + e.sizeBits) / 8;
      if (b <= lo || a >= hi) continue;
      const x0 = Math.max(0, X(a));
      const x1 = Math.min(W, X(b));
      out.push(svgEl("rect", { x: String(x0), y: "0", width: String(Math.max(2, x1 - x0)), height: String(AXIS_Y - 2), class: "rv-map-lit" }));
    }
    this.axis(out, lo, hi, X, W, ppb);
    this.svg.replaceChildren(...out);
    this.range.textContent = RV.mapRange(formatOffset(Math.floor(lo) * 8), formatOffset(Math.max(0, Math.ceil(hi) - 1) * 8));
    this.settled = stripDone && (spans.spans !== null || spans.truncated || this.ctx.doc.template === null);
    if (this.el.isConnected) this.drawnOnce = true;
  }

  /** The window's fields, asked once per window. */
  private windowSpans(lo: number, hi: number): SpanCache {
    if (this.ctx.doc.template === null) return { key: "", spans: [], truncated: false };
    const from = Math.floor(lo) * 8;
    const to = Math.ceil(hi) * 8;
    const key = `${from}:${to}`;
    if (this.spans.key === key && this.spans.spans !== null) return this.spans;
    const r = ok(this.ctx.doc.spans(from, to, SPAN_MAX));
    if (r === WAIT) this.spans = { key, spans: null, truncated: false };
    else if (r === null) this.spans = { key, spans: [], truncated: false };
    else this.spans = { key, spans: r, truncated: r.length >= SPAN_MAX };
    return this.spans;
  }

  private drawBytes(out: SVGElement[], lo: number, hi: number, X: (o: number) => number, ppb: number): boolean {
    const a = Math.max(0, Math.floor(lo));
    const b = Math.min(this.size, Math.ceil(hi));
    const r = this.ctx.doc.read(a, b - a);
    for (let o = a; o < b; o++) {
      const v = r.bytes[o - a] ?? 0;
      const x = X(o);
      const cls = !r.complete ? -1 : classOf(v);
      out.push(svgEl("rect", { x: String(x), y: String(STRIP_Y), width: String(Math.max(1, ppb - 1)), height: String(BYTE_H), class: "rv-map-byte", fill: cls < 0 ? "none" : byteClassColor(cls) }));
      const t = svgEl("text", { x: String(x + ppb / 2), y: String(STRIP_Y + 14), class: "rv-map-hex", "text-anchor": "middle" });
      t.textContent = r.complete ? v.toString(16).padStart(2, "0") : "··";
      out.push(t);
    }
    return r.complete;
  }

  /** The window read and classed per pixel column: a column is zero or text
   *  only if every byte in it is. */
  private drawLocal(out: SVGElement[], lo: number, hi: number, W: number, ppb: number): boolean {
    const a = Math.max(0, Math.floor(lo));
    const b = Math.min(this.size, Math.ceil(hi));
    const r = this.ctx.doc.read(a, b - a);
    if (!r.complete) return false;
    const cols = Math.max(1, Math.ceil(W));
    const cls = new Int8Array(cols).fill(-1);
    for (let o = a; o < b; o++) {
      const c = Math.min(cols - 1, Math.max(0, Math.floor((o - lo) * ppb)));
      const k = classOf(r.bytes[o - a] ?? 0);
      const had = cls[c] ?? -1;
      cls[c] = had < 0 || had === k ? k : OTHER;
    }
    this.runs(out, cls, (i) => i, (i) => i + 1);
    return true;
  }

  /** The whole-file byte-class scan over the window. Buckets not read yet are
   *  drawn empty, so a partial scan never looks like a finished one. */
  private drawScan(out: SVGElement[], lo: number, hi: number, X: (o: number) => number, W: number): boolean {
    const scan = this.ctx.data.scan();
    if (scan === null) return false;
    const bb = scan.bucket_bytes;
    const first = Math.max(0, Math.floor(lo / bb));
    const last = Math.min(scan.total_buckets, Math.ceil(hi / bb));
    const cls = new Int8Array(Math.max(0, last - first)).fill(-1);
    for (let i = first; i < last; i++) {
      const ch = scan.classes.charCodeAt(i);
      cls[i - first] = Number.isNaN(ch) ? -1 : ch - 48;
    }
    this.runs(out, cls, (i) => Math.max(0, X((first + i) * bb)), (i) => Math.min(W, X((first + i) * bb)));
    return scan.done;
  }

  /** Runs of one class as one rect each. `-1` is not read yet. */
  private runs(out: SVGElement[], cls: Int8Array, left: (i: number) => number, right: (i: number) => number): void {
    let i = 0;
    while (i < cls.length) {
      const c = cls[i] ?? -1;
      let j = i + 1;
      while (j < cls.length && cls[j] === c) j++;
      const x0 = left(i);
      const x1 = right(j);
      out.push(svgEl("rect", { x: String(x0), y: String(STRIP_Y), width: String(Math.max(0.6, x1 - x0)), height: String(STRIP_H), class: c < 0 ? "rv-map-unread" : "rv-map-class", ...(c < 0 ? {} : { fill: byteClassColor(c) }) }));
      i = j;
    }
  }

  private axis(out: SVGElement[], lo: number, hi: number, X: (o: number) => number, W: number, ppb: number): void {
    out.push(svgEl("line", { x1: "0", y1: String(AXIS_Y), x2: String(W), y2: String(AXIS_Y), class: "rv-map-axis" }));
    const step = 2 ** Math.max(0, Math.ceil(Math.log2(96 / Math.max(1e-9, ppb))));
    for (let t = Math.ceil(lo / step) * step; t <= hi; t += step) {
      const x = X(t);
      out.push(svgEl("line", { x1: String(x), y1: String(AXIS_Y), x2: String(x), y2: String(AXIS_Y + 4), class: "rv-map-axis" }));
      const label = svgEl("text", { x: String(Math.min(W - 40, Math.max(0, x - 14))), y: String(AXIS_Y + 15), class: "rv-map-tick" });
      label.textContent = formatOffset(t * 8);
      out.push(label);
    }
  }

  private byteAt(clientX: number): number {
    const r = this.svg.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (clientX - r.left) / Math.max(1, r.width)));
    return Math.floor(this.lo + f * (this.hi - this.lo));
  }

  private partAt(o: number): MapPart | null {
    const bit = o * 8;
    return this.parts.find((p) => bit >= p.startBit && bit < p.endBit) ?? null;
  }

  private spanAt(o: number): Span | null {
    const bit = o * 8;
    return this.spans.spans?.find((s) => bit >= s.offset_bits && bit < s.offset_bits + s.size_bits) ?? null;
  }

  private wire(): void {
    const el = this.svg;
    el.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        const r = el.getBoundingClientRect();
        const f = Math.min(1, Math.max(0, (e.clientX - r.left) / Math.max(1, r.width)));
        const at = this.lo + f * (this.hi - this.lo);
        const k = Math.exp((e.deltaY || e.deltaX) * 0.0022);
        const span = (this.hi - this.lo) * k;
        this.lo = at - f * span;
        this.hi = this.lo + span;
        this.clamp();
        hideTip();
        this.redraw();
      },
      { passive: false },
    );
    el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      this.drag = { x: e.clientX, lo: this.lo, hi: this.hi, moved: false };
      el.setPointerCapture(e.pointerId);
    });
    el.addEventListener("pointermove", (e) => {
      if (this.drag !== null) {
        const dx = e.clientX - this.drag.x;
        if (Math.abs(dx) > 3) this.drag.moved = true;
        if (!this.drag.moved) return;
        const r = el.getBoundingClientRect();
        const d = (dx / Math.max(1, r.width)) * (this.drag.hi - this.drag.lo);
        this.lo = this.drag.lo - d;
        this.hi = this.drag.hi - d;
        this.clamp();
        hideTip();
        this.redraw();
        return;
      }
      this.hover(e);
    });
    el.addEventListener("pointerup", (e) => {
      const d = this.drag;
      this.drag = null;
      if (d === null || d.moved) return;
      const o = this.byteAt(e.clientX);
      const s = this.spanAt(o);
      if (s !== null && !s.gap) {
        this.ctx.host.pick({ path: s.path, startBit: s.offset_bits, endBit: s.offset_bits + s.size_bits });
        return;
      }
      const p = this.partAt(o);
      if (p !== null) this.ctx.host.pick({ ...(p.path === undefined ? {} : { path: p.path }), startBit: p.startBit, endBit: p.endBit });
      else this.ctx.host.pick({ startBit: o * 8, endBit: o * 8 + 8 });
    });
    el.addEventListener("pointerleave", () => hideTip());
    el.addEventListener("dblclick", (e) => {
      e.preventDefault();
      this.show(0, this.size);
    });
  }

  private hover(e: PointerEvent): void {
    const o = this.byteAt(e.clientX);
    if (o < 0 || o >= this.size) {
      hideTip();
      return;
    }
    const box = document.createElement("div");
    const p = this.partAt(o);
    if (p !== null) {
      const b = document.createElement("b");
      b.textContent = p.label;
      const where = document.createElement("div");
      where.className = "rv-tip-where";
      where.textContent = `${RV.tipRange(formatOffset(p.startBit), formatOffset(Math.max(p.startBit, p.endBit - 8)), bitsText(p.endBit - p.startBit))}, ${shareOfFile(p.endBit - p.startBit, this.size * 8)}`;
      box.append(b, where);
    }
    const s = this.spanAt(o);
    if (s !== null) {
      const line = document.createElement("div");
      const name = document.createElement("code");
      name.textContent = s.gap ? RV.gapName : s.name;
      line.append(name);
      const v = s.line ?? s.value;
      if (!s.gap && v !== "") line.append(`: ${v}`);
      line.append(` (${bitsText(s.size_bits)} at ${formatOffset(s.offset_bits)})`);
      box.append(line);
    }
    const r = this.ctx.doc.read(o, 1);
    const byte = document.createElement("div");
    byte.className = "rv-tip-where";
    if (r.complete) {
      const v = r.bytes[0] ?? 0;
      byte.textContent = RV.mapByte(formatOffset(o * 8), v.toString(16).padStart(2, "0"), classWord(classOf(v)));
    } else byte.textContent = `${formatOffset(o * 8)}: ${RV.tipLoading}`;
    box.append(byte);
    tipAt(box, e.clientX, e.clientY);
  }
}

/** What one byte is on the strip: zero, printable text, or anything else. */
function classOf(v: number): number {
  if (v === 0) return ZERO;
  if ((v >= 0x20 && v < 0x7f) || v === 9 || v === 10 || v === 13) return TEXT;
  return OTHER;
}

function classWord(c: number): string {
  return c === ZERO ? RV.classZero : c === TEXT ? RV.classText : RV.classOther;
}

/** The byte strip's legend: which colour is which class. */
export function stripLegend(): HTMLElement {
  const box = document.createElement("div");
  box.className = "rv-legend";
  const lead = document.createElement("span");
  lead.className = "rv-muted";
  lead.textContent = RV.byteStripLabel;
  box.append(lead);
  const item = (color: string | null, text: string): void => {
    const s = document.createElement("span");
    const sw = document.createElement("span");
    sw.className = color === null ? "rv-swatch is-unread" : "rv-swatch";
    if (color !== null) sw.style.background = color;
    s.append(sw, text);
    box.append(s);
  };
  item(byteClassColor(ZERO), RV.classZero);
  item(byteClassColor(TEXT), RV.classText);
  item(byteClassColor(OTHER), RV.classOther);
  item(byteClassColor(1), RV.classRepeat);
  item(byteClassColor(4), RV.classDense);
  item(null, RV.classUnread);
  return box;
}

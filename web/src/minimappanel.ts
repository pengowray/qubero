/**
 * The minimap: the whole file scaled down to a grid of equal cells, one per
 * bucket of the byte-class scan, each coloured by what its bytes turned out to
 * be. Under it, the parts of the file as a single strip, and under that the
 * block a reader opened out of the map, measured on its own.
 *
 * It answers "where in the file are the bytes of each kind", and it answers it
 * for a file no template covers: a tail of zeros or a compressed middle shows
 * up whether or not anything describes it. Its partner below it in the rail is
 * the treemap, which answers the question this one cannot once the small parts
 * are under a pixel: how much of the file each thing is. The two are drawn
 * from the same scan and take the same two mouse verbs, because two pictures
 * of one file side by side that answered a press differently would be two
 * things to learn rather than one. A press selects a cell and goes to its
 * bytes; a second press opens that block below.
 *
 * A cell stands for a lot of bytes and a bucket is judged as a whole, so
 * opening one is not a zoom on what is already drawn: the block is scanned
 * again on its own, at its own resolution, and reported for what it is.
 */

import { byteText, formatBytes, formatOffset, percentText } from "./doc.ts";
import type { Doc, FocusState, OverviewState, Span } from "./doc.ts";
import { TREEMAP } from "./strings.ts";
import { byteClassColors } from "./fieldstyle.ts";
import { fileMap, markMap, segmentWidths } from "./filemap.ts";
import type { MapMark, MapSegment } from "./filemap.ts";
import type { Viewport } from "./outline.ts";
import { factRow, noneLine } from "./dom.ts";

/** How the map is drawn: a square this wide plus a one-pixel gap. */
const CELL = 5;
const GAP = 1;
/** Room around the cells for the outline of the viewed range, which is drawn
 *  just outside the cells it encloses and would otherwise be clipped at the
 *  edges of the map. */
const MAP_PAD = 3;

/** Cells the whole-file map aims for. Enough that a percent of the file is
 *  several cells, few enough that the map stays a glance rather than a page. */
const MAP_BUCKETS = 1024;
/** Cells one block is divided into when it is looked at closely. */
const FOCUS_BUCKETS = 512;

/** A run of byte classes worth a sentence: at least this share of the file. */
const NOTE_PERCENT = 5;
/** And at least this many bytes. In a small file the map already shows every
 *  byte, and a run of a few dozen printable ones is chance, not a finding. */
const NOTE_MIN_BYTES = 512;
/** At most this many sentences; the map shows the rest. */
const NOTE_LIMIT = 3;

/** Fields fetched to work out what a block leaves undescribed. */
const SPAN_LIMIT = 2048;
/** Undescribed stretches listed for a block. */
const GAP_ROWS = 12;

/** What each class is called, by digit. The fill colours live in
 *  `fieldstyle`, since the treemap of the same scan paints in them too. */
const CLASS_LABEL = TREEMAP.classLabel;
const CLASS_TITLE = [
  "Every byte is 0x00",
  "Every byte is the same value, such as 0xFF padding",
  "Mostly printable characters",
  "Structured bytes: headers, tables, machine code",
  "Bytes using the whole 0-255 range about evenly, typical of compressed or encrypted data",
];
/** The same classes inside a sentence. */
const CLASS_PROSE = ["zeros", "one repeated byte", "text", "data", "high-entropy data, likely compressed or encrypted"];

/** The class map's own heading. The rail holds two pictures of the file and
 *  they answer different questions: this one says where things are, the
 *  treemap under it says how much of the file they are. Each is named by the
 *  word a reader can look up and already has from other tools: this map is a
 *  minimap in the editor sense (the whole file scaled down, in order, the
 *  viewed range outlined on it, a click to go there), and its partner is
 *  "Treemap". "Where" would have named the question, not the thing. */
const MAP_TITLE = "Minimap";
/** The tooltip on the heading and on the map itself. It opens with the
 *  question the picture answers, then what a cell is, then the two mouse
 *  verbs, which are the same two the treemap uses. The double-click names
 *  the Block section by its heading so a reader can find where the block
 *  went. */
const MAP_WHAT =
  "Where each kind of byte is in the file. One cell per equal-sized block, in file order, coloured by the kind of bytes in it; the legend names the colours. Click a cell to go to those bytes. Double-click to open that block in the Block section below.";
const SCALE_LABEL = (cell: string): string => `1 cell = ${cell}`;
/** The strip under the class map, when pointed at. */
const LAYOUT_TITLE = "The parts of the file in order, each as wide as its share of the bytes";
const BLOCK_TITLE = "Block";
const CLOSE_BLOCK = "Close block";
const SCANNING = (percent: number): string => `Scanning the file… ${percent}%`;
/** Keeps the line under the map from collapsing when the pointer leaves it,
 *  which would jump everything below by a row. */
const BLANK = " ";

const ALL_DESCRIBED = "No unmapped bytes in this block.";
const GAPS_FOUND = (n: number, bytes: number): string =>
  `Unmapped: ${n === 1 ? "1 run" : `${n.toLocaleString()} runs`}, ${formatBytes(bytes)} total.`;
const GAPS_MORE = (n: number): string => `${n.toLocaleString()} more not shown.`;
const MEASURE_THIS = "Measure only these unmapped bytes";

/** One maximal run of buckets sharing a class. */
type Run = { readonly cls: number; readonly start: number; readonly len: number };

/** A stretch of a block that no field covers, in bytes. */
type Gap = { readonly from: number; readonly to: number };

function runsOf(classes: string): Run[] {
  const out: Run[] = [];
  for (let i = 0; i < classes.length; i++) {
    const cls = Number(classes[i]);
    const last = out[out.length - 1];
    if (last !== undefined && last.cls === cls) out[out.length - 1] = { cls, start: last.start, len: last.len + 1 };
    else out.push({ cls, start: i, len: 1 });
  }
  return out;
}

/** Runs rejoined across short interruptions, for the notes only: a stretch of
 *  compressed data with a calm bucket or two in it is one stretch, not three.
 *  The map itself stays exact. */
function coalesced(runs: Run[], buckets: number): Run[] {
  const tolerance = Math.max(1, Math.floor(buckets / 64));
  const out: Run[] = [];
  for (const r of runs) {
    const last = out[out.length - 1];
    const before = out[out.length - 2];
    if (last !== undefined && before !== undefined && before.cls === r.cls && last.len <= tolerance) {
      out.pop();
      out[out.length - 1] = { cls: r.cls, start: before.start, len: r.start + r.len - before.start };
    } else {
      out.push(r);
    }
  }
  return out;
}

/** The bucket size as a round unit: `1 byte`, `4 KiB`. It is a power of two,
 *  so the division is exact and needs no decimals. */
function cellText(bytes: number): string {
  if (bytes === 1) return "1 byte";
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${bytes / 1024} KiB`;
  if (bytes < 1024 * 1024 * 1024) return `${bytes / (1024 * 1024)} MiB`;
  return `${bytes / (1024 * 1024 * 1024)} GiB`;
}

/** The sentence one notable run earns. Position carries most of it: a run at
 *  the very end reads differently from one in the middle. */
function noteText(run: Run, buckets: number, bucketBytes: number, fileBytes: number): string {
  const bytes = Math.min(run.len * bucketBytes, fileBytes - run.start * bucketBytes);
  const size = `${formatBytes(bytes)} (${percentText(bytes, fileBytes)})`;
  const what = CLASS_PROSE[run.cls] ?? "data";
  if (run.len === buckets) return `The whole file is ${what}.`;
  // A run that leaves a cell or two over is still the file, and "the first
  // 2.25 MiB of 2.25 MiB" says nothing about where it is.
  if (run.len * 50 >= buckets * 49) return `Nearly all of the file is ${what}.`;
  if (run.start === 0) return `The first ${size} is ${what}.`;
  if (run.start + run.len === buckets) return `The last ${size} is ${what}.`;
  return `${size} at ${formatOffset(run.start * bucketBytes * 8)} is ${what}.`;
}

export class MinimapPanel {
  readonly el: HTMLElement;
  private readonly mapHead: HTMLElement;
  private readonly mapScale: HTMLElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly readout: HTMLElement;
  private readonly legend: HTMLElement;
  private readonly notes: HTMLElement;
  private readonly layout: HTMLElement;
  private layoutStrip: HTMLElement | null = null;

  private readonly focusEl: HTMLElement;
  private readonly focusHead: HTMLElement;
  private readonly focusCanvas: HTMLCanvasElement;
  private readonly focusReadout: HTMLElement;
  private readonly focusStats: HTMLElement;
  private readonly focusGaps: HTMLElement;

  private state: OverviewState | null = null;
  private focusState: FocusState | null = null;
  /** The block being looked at, in bytes, or null when none is. */
  private block: { from: number; to: number } | null = null;
  /** The cell of the whole-file map the reader picked, marked while the rest of
   *  the map stays as it was. Dimming the file to show one cell was the first
   *  try and was the wrong shape of answer: a mark says "this one", and dimming
   *  says "not those", which for one cell out of a thousand throws the picture
   *  away to point at a speck of it. */
  private mark: { from: number; to: number } | null = null;
  /** Which cell of the block map was last picked, so it stays marked while the
   *  rest of the scan fills in around it. */
  private picked: number | null = null;
  /** Another step is already queued, so a burst of notifies runs one. */
  private stepQueued = false;
  private focusQueued = false;
  /** Buckets to draw brighter than the rest, while a part is under the
   *  pointer or a block is picked. */
  private highlight: { from: number; to: number } | null = null;
  /** The stretch of the file the main view is showing, for the layout strip
   *  and for the outline on the map. */
  private viewport: Viewport | null = null;
  /** The run of cells that outline was last drawn round. */
  private viewCellsDrawn = "";
  /** The range someone outside asked to be lit, lit on the map and marked on
   *  the layout strip while it stands. */
  private hovering: MapMark = null;
  /** The top-level parts of the file, already coalesced to as many cells as
   *  the strip can draw. */
  private parts: readonly MapSegment[] = [];

  /** A cell or a listed stretch was picked: go there, and mark the bytes it
   *  stands for. A cell is a stretch of the file, not a place in it, so
   *  picking one selects it rather than only moving the cursor to its front. */
  onJump: (startBit: number, endBit: number) => void = () => {};
  /** A press landed on a map. The Contents list uses this to stop following
   *  the view for a moment: the reader is looking at the map, and the list
   *  moving under their pointer is the wrong thing to notice. */
  onPressed: () => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("div");
    this.el.className = "ov-minimap";

    this.mapHead = document.createElement("div");
    this.mapHead.className = "ov-head";
    this.mapScale = document.createElement("span");
    this.mapScale.className = "ov-scale";
    const mapTitle = document.createElement("h3");
    mapTitle.textContent = MAP_TITLE;
    mapTitle.title = MAP_WHAT;
    this.mapHead.append(mapTitle, this.mapScale);
    this.canvas = document.createElement("canvas");
    this.canvas.className = "ov-map";
    this.canvas.title = MAP_WHAT;
    this.readout = document.createElement("p");
    this.readout.className = "ov-readout";
    this.readout.textContent = BLANK;
    this.legend = document.createElement("div");
    this.legend.className = "ov-legend";
    this.notes = document.createElement("ul");
    this.notes.className = "ov-notes";
    // An empty list is still a row of the column's gap, and between the legend
    // and the strip that gap would read as something missing.
    this.notes.hidden = true;
    this.layout = document.createElement("div");
    this.layout.className = "ov-layout";
    this.layout.hidden = true;

    this.focusEl = document.createElement("section");
    this.focusEl.className = "ov-focus";
    this.focusHead = document.createElement("h3");
    this.focusCanvas = document.createElement("canvas");
    this.focusCanvas.className = "ov-map";
    this.focusReadout = document.createElement("p");
    this.focusReadout.className = "ov-readout";
    this.focusReadout.textContent = BLANK;
    this.focusStats = document.createElement("dl");
    this.focusStats.className = "ov-facts";
    this.focusGaps = document.createElement("div");
    this.focusGaps.className = "ov-gaps";
    this.focusEl.append(
      this.focusHead,
      this.focusCanvas,
      this.focusReadout,
      this.focusStats,
      this.focusGaps,
    );

    // In the order the questions come: what the map is drawn at, the map, the
    // cell under the pointer, what the colours mean, the sentences those
    // colours add up to, the parts of the file the same bytes are divided
    // into, and last the block a reader opened out of the map. The block goes
    // straight under the map the cell was picked on, because the cell and the
    // close look at it are one thing to read.
    this.el.append(
      this.mapHead,
      this.canvas,
      this.readout,
      this.legend,
      this.notes,
      this.layout,
      this.focusEl,
    );

    this.canvas.addEventListener("pointermove", (e) => this.onMapHover(e));
    this.canvas.addEventListener("pointerleave", () => {
      this.readout.textContent = BLANK;
    });
    this.canvas.addEventListener("click", (e) => this.onMapClick(e));
    this.canvas.addEventListener("dblclick", (e) => this.onMapOpen(e));
    this.focusCanvas.addEventListener("pointermove", (e) => this.onFocusHover(e));
    this.focusCanvas.addEventListener("pointerleave", () => {
      this.focusReadout.textContent = BLANK;
    });
    this.focusCanvas.addEventListener("click", (e) => this.onFocusClick(e));
  }

  /** Bytes per cell and the rest of the scan, for anything outside that has to
   *  wait for the file to have been read before it can say anything. */
  get scan(): OverviewState | null {
    return this.state;
  }

  /**
   * The stretch of the file the main view is showing: marked on the layout
   * strip, and outlined on the map. The view reports itself on every scroll
   * frame, so the map is redrawn only when the run of cells it covers changes.
   */
  setViewport(v: Viewport): void {
    this.viewport = v;
    if (this.layoutStrip !== null && !this.hovering) this.markLayout(this.viewportMark());
    const s = this.state;
    const cells = s === null ? null : this.viewCells(s);
    const key = cells === null ? "" : `${cells.from}-${cells.to}`;
    if (key !== this.viewCellsDrawn) {
      this.viewCellsDrawn = key;
      this.drawMain();
    }
  }

  /** The top-level parts of the file, for the layout strip. Already reduced to
   *  as many segments as the strip can draw: what a part is, and how many of
   *  them are worth a cell, is the Contents list's business, not the map's. */
  setParts(parts: readonly MapSegment[]): void {
    this.parts = parts;
    this.drawLayout();
  }

  /**
   * Cells to draw brighter than the rest, and the mark on the layout strip:
   * how a row under the pointer elsewhere in the rail shows where its bytes
   * sit. Null puts both back to the picked block and the viewed range.
   */
  setHighlight(range: { offsetBits: number; sizeBits: number } | null): void {
    this.hovering = range;
    this.highlightRange(range);
    this.markLayout(this.hovering ?? this.viewportMark());
  }

  /** The rail was resized. The cells wrap to its width, so a narrower window
   *  is a different map rather than the same one clipped. */
  relayout(): void {
    this.render();
    this.drawLayout();
  }

  /**
   * Ask for one more step of the scan when there is anything left to do and
   * anyone to see it. A folded-away or hidden map must not pull the whole file
   * through the chunk cache for nobody, which is what the visibility check is
   * for: this is also the way back in from the timeouts below.
   */
  pump(): void {
    if (this.el.offsetParent === null) return;
    this.pumpMap();
    this.pumpFocus();
  }

  private pumpMap(): void {
    if (this.stepQueued) return;
    // As many steps as a frame's worth of time allows, in one go: a chain of
    // one step per timeout would be throttled to a crawl the moment the tab
    // is in the background.
    const start = performance.now();
    let r = this.doc.overviewStep(MAP_BUCKETS);
    while (r.status === "ok" && !r.node.done && performance.now() - start < 10) {
      r = this.doc.overviewStep(MAP_BUCKETS);
    }
    if (r.status !== "ok") return;
    // An edit throws the scan away, so `done` can go back to false here.
    this.state = r.node;
    this.render();
    if (!r.node.done) {
      // Yield so the page draws the partial map and stays usable; the chunk
      // fetches themselves also come back through pump.
      this.stepQueued = true;
      setTimeout(() => {
        this.stepQueued = false;
        this.pump();
      }, 0);
    }
  }

  private pumpFocus(): void {
    const block = this.block;
    if (block === null || this.focusQueued) return;
    const start = performance.now();
    let r = this.doc.overviewFocusStep(block.from, block.to, FOCUS_BUCKETS);
    while (r.status === "ok" && !r.node.done && performance.now() - start < 6) {
      r = this.doc.overviewFocusStep(block.from, block.to, FOCUS_BUCKETS);
    }
    if (r.status !== "ok") return;
    this.focusState = r.node;
    this.renderFocus();
    if (!r.node.done) {
      this.focusQueued = true;
      setTimeout(() => {
        this.focusQueued = false;
        this.pump();
      }, 0);
    }
  }

  // ----- drawing -----

  private render(): void {
    const s = this.state;
    if (s === null) return;
    // What one cell stands for belongs on the map, not in the rail's list of
    // facts about the file: it is a fact about the drawing, and read among
    // Size and Type it looked like one more thing the file was.
    this.mapScale.textContent = SCALE_LABEL(cellText(s.bucket_bytes));
    this.drawMain();
    this.drawLegend(s);
    this.drawNotes(s);
    this.renderFocus();
  }

  /** The whole-file map, with whatever is dimmed dimmed and the viewed range
   *  outlined. Every draw of that map goes through here, so the two marks
   *  never come apart. */
  private drawMain(): void {
    const s = this.state;
    if (s === null) return;
    this.drawMap(this.canvas, s.classes, this.highlight, this.viewCells(s), this.mark);
  }

  /** The run of cells the main view is showing, or null when it shows none of
   *  the file the map has scanned so far. */
  private viewCells(s: OverviewState): { from: number; to: number } | null {
    const v = this.viewport;
    const bits = s.bucket_bytes * 8;
    if (v === null || bits <= 0 || s.classes.length === 0) return null;
    const from = Math.floor(v.startBit / bits);
    if (from >= s.classes.length || from < 0) return null;
    const to = Math.min(s.classes.length, Math.max(from + 1, Math.ceil(v.endBit / bits)));
    return { from, to };
  }

  private colors(): readonly string[] {
    return byteClassColors();
  }

  /** One map, wherever it is drawn. Cells outside `bright` are dimmed, which
   *  is how a part under the pointer or a picked block shows where its bytes
   *  sit. `outline` is the run of cells the main view is showing, drawn round
   *  them rather than over them so every cell keeps the colour of its class. */
  private drawMap(
    canvas: HTMLCanvasElement,
    classes: string,
    bright: { from: number; to: number } | null,
    outline: { from: number; to: number } | null = null,
    mark: { from: number; to: number } | null = null,
  ): void {
    const width = this.innerWidth();
    if (width <= 0) return;
    const cols = Math.max(8, Math.floor((width - MAP_PAD * 2) / (CELL + GAP)));
    const rows = Math.max(1, Math.ceil(Math.max(1, classes.length) / cols));
    const w = cols * (CELL + GAP) + MAP_PAD * 2;
    const h = rows * (CELL + GAP) + MAP_PAD * 2;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
    canvas.dataset["cols"] = String(cols);
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const colors = this.colors();
    for (let i = 0; i < classes.length; i++) {
      const cls = Number(classes[i]);
      ctx.globalAlpha = bright === null || (i >= bright.from && i < bright.to) ? 1 : 0.25;
      ctx.fillStyle = colors[cls] ?? colors[3] ?? "#888";
      ctx.fillRect(MAP_PAD + (i % cols) * (CELL + GAP), MAP_PAD + Math.floor(i / cols) * (CELL + GAP), CELL, CELL);
    }
    ctx.globalAlpha = 1;
    // Two marks, told apart by colour: the accent is the stretch the view is
    // showing, and the foreground is the cell the reader picked, which is the
    // same colour the treemap marks a picked box with.
    if (outline !== null && outline.to > outline.from) this.strokeRun(ctx, cols, outline.from, outline.to, "--accent");
    if (mark !== null && mark.to > mark.from) this.strokeRun(ctx, cols, mark.from, mark.to, "--fg");
  }

  /**
   * Draw the border of a run of cells: a rectangle when the run sits in one
   * row, and the stepped shape of the wrapped run when it does not. A line in
   * the page's background colour goes under a narrower one in the accent
   * colour, so the border reads against a cell of any class.
   */
  private strokeRun(ctx: CanvasRenderingContext2D, cols: number, from: number, to: number, colour: string): void {
    const step = CELL + GAP;
    const last = to - 1;
    const r0 = Math.floor(from / cols);
    const r1 = Math.floor(last / cols);
    const c0 = from % cols;
    const c1 = last % cols;
    // The cell's box, grown by a pixel, so the border sits in the gap between
    // cells rather than over the colour it is marking.
    const left = (c: number): number => MAP_PAD + c * step - 1;
    const right = (c: number): number => MAP_PAD + c * step + CELL + 1;
    const top = (r: number): number => MAP_PAD + r * step - 1;
    const bottom = (r: number): number => MAP_PAD + r * step + CELL + 1;
    const path = new Path2D();
    if (r0 === r1) {
      path.rect(left(c0), top(r0), right(c1) - left(c0), bottom(r0) - top(r0));
    } else {
      const near = left(0);
      const far = right(cols - 1);
      path.moveTo(left(c0), top(r0));
      path.lineTo(far, top(r0));
      path.lineTo(far, top(r1));
      path.lineTo(right(c1), top(r1));
      path.lineTo(right(c1), bottom(r1));
      path.lineTo(near, bottom(r1));
      path.lineTo(near, bottom(r0));
      path.lineTo(left(c0), bottom(r0));
      path.closePath();
    }
    const style = getComputedStyle(this.el);
    const ink = style.getPropertyValue(colour).trim();
    const ground = style.getPropertyValue("--bg").trim();
    ctx.lineJoin = "miter";
    ctx.strokeStyle = ground === "" ? "#fff" : ground;
    ctx.lineWidth = 4;
    ctx.stroke(path);
    ctx.strokeStyle = ink === "" ? "#2457c5" : ink;
    ctx.lineWidth = 2;
    ctx.stroke(path);
  }

  /** The width the cells have to fit inside. The panel is a column of the
   *  rail's body, stretched to it, so its own width is already the width
   *  inside the body's padding. */
  private innerWidth(): number {
    return this.el.clientWidth;
  }

  private drawLegend(s: OverviewState): void {
    const counts = [0, 0, 0, 0, 0];
    for (const c of s.classes) counts[Number(c)] = (counts[Number(c)] ?? 0) + 1;
    const colors = this.colors();
    const chips: HTMLElement[] = [];
    for (let cls = 0; cls < counts.length; cls++) {
      const n = counts[cls] ?? 0;
      if (n === 0) continue;
      const chip = document.createElement("span");
      chip.className = "ov-chip";
      chip.title = CLASS_TITLE[cls] ?? "";
      const swatch = document.createElement("i");
      swatch.className = "ov-swatch";
      swatch.style.background = colors[cls] ?? "";
      const name = document.createElement("span");
      name.className = "ov-chip-name";
      name.textContent = CLASS_LABEL[cls] ?? "";
      const share = document.createElement("span");
      share.className = "ov-chip-share";
      share.textContent = percentText(n, s.classes.length);
      chip.append(swatch, name, share);
      chips.push(chip);
    }
    if (!s.done) {
      const p = document.createElement("span");
      p.className = "ov-progress";
      p.textContent = SCANNING(Math.round((s.read_bytes / Math.max(1, this.doc.lengthBytes)) * 100));
      chips.push(p);
    }
    this.legend.replaceChildren(...chips);
  }

  /** The sentences the colours add up to, under the legend that names them.
   *  Hidden rather than emptied when there is nothing to say, because an empty
   *  list between the legend and the strip is still a gap the reader reads. */
  private drawNotes(s: OverviewState): void {
    if (!s.done) {
      this.notes.replaceChildren();
      this.notes.hidden = true;
      return;
    }
    const threshold = Math.max(2, (s.total_buckets * NOTE_PERCENT) / 100);
    // Data is the norm; the other classes are the ones worth a sentence.
    const notable = coalesced(runsOf(s.classes), s.total_buckets)
      .filter((r) => r.cls !== 3 && r.len >= threshold && r.len * s.bucket_bytes >= NOTE_MIN_BYTES)
      .sort((a, b) => b.len - a.len)
      .slice(0, NOTE_LIMIT)
      .sort((a, b) => a.start - b.start);
    this.notes.replaceChildren(
      ...notable.map((r) => {
        const li = document.createElement("li");
        li.textContent = noteText(r, s.classes.length, s.bucket_bytes, this.doc.lengthBytes);
        return li;
      }),
    );
    this.notes.hidden = notable.length === 0;
  }

  // ----- the layout strip -----

  /** The parts of the file as one row under the class map, every part lit in
   *  its own colour, and the stretch the main view is showing marked on it.
   *  The two maps say different things and sit together so the reader can
   *  see "the zeros are the unused half of page 1". */
  private drawLayout(): void {
    const segments = this.parts;
    const width = this.innerWidth();
    if (segments.length === 0 || width <= 0) {
      this.layout.hidden = true;
      this.layout.replaceChildren();
      this.layoutStrip = null;
      return;
    }
    const strip = fileMap(segments, 0, this.doc.lengthBits, LAYOUT_TITLE);
    // The strip is drawn for a heading's width. Here it has the rail's, so
    // its cells are sized again for that.
    const widths = segmentWidths(segments, width);
    Array.from(strip.children).forEach((cell, i) => {
      if (cell instanceof HTMLElement) cell.style.flex = `0 0 ${(widths[i] ?? 0).toFixed(2)}px`;
    });
    strip.style.width = `${width}px`;
    this.layout.replaceChildren(strip);
    this.layout.hidden = false;
    this.layoutStrip = strip;
    this.markLayout(this.hovering ?? this.viewportMark());
  }

  private viewportMark(): MapMark {
    const v = this.viewport;
    return v === null ? null : { offsetBits: v.startBit, sizeBits: Math.max(0, v.endBit - v.startBit) };
  }

  private markLayout(mark: MapMark): void {
    if (this.layoutStrip !== null) markMap(this.layoutStrip, mark);
  }

  // ----- lighting the map -----

  /**
   * Dim the rest of the map, so the range asked for shows where its bytes sit.
   * Passing null puts the whole map back.
   *
   * Dimming is for one job only: a part of the file under the pointer in the
   * Contents list, where the question is "where does this part live" and the
   * answer can be a run scattered over the whole map. It is the wrong answer
   * for a single cell somebody pressed, which is a mark; the map used to dim
   * for that too, and pressing one cell in a thousand greyed out the file.
   */
  private highlightRange(range: { offsetBits: number; sizeBits: number } | null): void {
    const s = this.state;
    if (s === null) return;
    const bucketBits = s.bucket_bytes * 8;
    if (range === null) this.highlight = null;
    else {
      const from = Math.floor(range.offsetBits / bucketBits);
      this.highlight = { from, to: Math.max(from + 1, Math.ceil((range.offsetBits + range.sizeBits) / bucketBits)) };
    }
    this.drawMain();
  }

  // ----- the block being looked at -----

  /**
   * A press on the map selects the cell and goes to its bytes; a second press
   * opens it and measures it on its own.
   *
   * The same two verbs as the treemap under it, and for the same reason: two
   * pictures of one file side by side that answered a press differently would
   * be two things to learn rather than one. Selecting is the cheap one and it
   * is what a single press does, so a reader can run along the map without
   * setting a scan going at every cell.
   */
  private onMapClick(e: MouseEvent): void {
    const s = this.state;
    const i = this.bucketAt(this.canvas, s?.classes.length ?? 0, e);
    if (s === null || i === null) return;
    const from = i * s.bucket_bytes;
    const to = Math.min(this.doc.lengthBytes, from + s.bucket_bytes);
    // Pressing the marked cell again takes the mark off, the way pressing a lit
    // box on the treemap does.
    const same = this.mark !== null && this.mark.from === i && this.mark.to === i + 1;
    this.mark = same ? null : { from: i, to: i + 1 };
    this.drawMain();
    if (same) return;
    this.onPressed();
    this.onJump(from * 8, to * 8);
  }

  /** The second press: look at that stretch of the file on its own. Pressing
   *  the cell that is already open closes it again. */
  private onMapOpen(e: MouseEvent): void {
    const s = this.state;
    const i = this.bucketAt(this.canvas, s?.classes.length ?? 0, e);
    if (s === null || i === null) return;
    const from = i * s.bucket_bytes;
    const to = Math.min(this.doc.lengthBytes, from + s.bucket_bytes);
    const open = this.block;
    if (open !== null && open.from === from && open.to === to) {
      this.setBlock(0, 0);
      return;
    }
    this.setBlock(from, to);
    this.mark = { from: i, to: i + 1 };
    this.drawMain();
  }

  /** A cell of the block map is a stretch of the block, and picking one marks
   *  those bytes rather than opening a block inside the block: at this
   *  resolution the cell is already the thing to look at. */
  private onFocusClick(e: MouseEvent): void {
    const f = this.focusState;
    const i = this.bucketAt(this.focusCanvas, f?.classes.length ?? 0, e);
    if (f === null || i === null) return;
    const from = f.start + i * f.bucket_bytes;
    const to = Math.min(f.end, from + f.bucket_bytes);
    // Marked, not dimmed, for the reason the whole-file map is: a block map of
    // five hundred cells with one bright cell in it is a picture thrown away.
    this.picked = i;
    this.drawMap(this.focusCanvas, f.classes, null, null, { from: i, to: i + 1 });
    this.onPressed();
    this.onJump(from * 8, to * 8);
  }

  /** Look at one stretch of the file on its own. */
  private setBlock(from: number, to: number): void {
    this.block = to > from ? { from, to } : null;
    this.focusState = null;
    this.picked = null;
    this.highlightRange(null);
    this.renderFocus();
    this.pump();
  }

  private renderFocus(): void {
    const block = this.block;
    // No block open is nothing to say, not a sentence saying so. The line that
    // used to sit here explained a press the map itself now signposts, and it
    // held a section's worth of space open for something that was not there.
    this.focusEl.hidden = block === null;
    if (block === null) {
      this.focusEl.replaceChildren();
      return;
    }
    const close = document.createElement("button");
    close.type = "button";
    close.className = "ov-close";
    close.textContent = "×";
    close.title = CLOSE_BLOCK;
    close.setAttribute("aria-label", CLOSE_BLOCK);
    close.addEventListener("click", () => this.setBlock(0, 0));
    this.focusHead.replaceChildren(
      `${BLOCK_TITLE} ${formatOffset(block.from * 8)} · ${formatBytes(block.to - block.from)}`,
      close,
    );
    this.focusEl.replaceChildren(
      this.focusHead,
      this.focusCanvas,
      this.focusReadout,
      this.focusStats,
      this.focusGaps,
    );
    const f = this.focusState;
    const cell = this.picked;
    if (f !== null) this.drawMap(this.focusCanvas, f.classes, null, null, cell === null ? null : { from: cell, to: cell + 1 });
    this.drawFocusStats(f);
    this.drawGaps(block);
  }

  private drawFocusStats(f: FocusState | null): void {
    if (f === null) {
      this.focusStats.replaceChildren();
      return;
    }
    const read = Math.max(1, f.read_bytes);
    const rows: [string, string][] = [];
    // The pair rather than the number: 7.9 out of 8 means dense, 7.9 out of
    // 7.9 means only that there are not many bytes here to spread out.
    rows.push(["Entropy", `${f.entropy.toFixed(2)} of ${f.entropy_max.toFixed(2)} bits per byte`]);
    rows.push(["Byte values", `${f.distinct.toLocaleString()} of 256`]);
    rows.push(["Zeros", `${percentText(f.zero_bytes, read)} (${f.zero_bytes.toLocaleString()} bytes)`]);
    rows.push(["Printable", `${percentText(f.text_bytes, read)} (${f.text_bytes.toLocaleString()} bytes)`]);
    const common = f.common
      .slice(0, 3)
      .map((c) => `${byteText(c.value)} ${percentText(c.count, read)}`)
      .join(", ");
    if (common !== "") rows.push(["Commonest", common]);
    this.focusStats.replaceChildren(...rows.flatMap(([k, v]) => factRow(k, v)));
  }

  // ----- what the template leaves undescribed -----

  /** The stretches of a block that no field covers. Over one block this is
   *  exact and affordable, which it would not be over a whole file. */
  private gapsIn(block: { from: number; to: number }): Gap[] | null {
    const r = this.doc.spans(block.from * 8, block.to * 8, SPAN_LIMIT);
    if (r.status !== "ok") return null;
    const out: Gap[] = [];
    for (const s of r.node as readonly Span[]) {
      if (!s.gap) continue;
      const from = Math.max(block.from, Math.floor(s.offset_bits / 8));
      const to = Math.min(block.to, Math.ceil((s.offset_bits + s.size_bits) / 8));
      if (to <= from) continue;
      const last = out[out.length - 1];
      if (last !== undefined && last.to >= from) out[out.length - 1] = { from: last.from, to: Math.max(last.to, to) };
      else out.push({ from, to });
    }
    return out;
  }

  private drawGaps(block: { from: number; to: number }): void {
    if (this.doc.template === null) {
      this.focusGaps.replaceChildren();
      return;
    }
    const gaps = this.gapsIn(block);
    if (gaps === null) return;
    if (gaps.length === 0) {
      this.focusGaps.replaceChildren(noneLine(ALL_DESCRIBED));
      return;
    }
    const total = gaps.reduce((n, g) => n + (g.to - g.from), 0);
    const summary = document.createElement("p");
    summary.className = "ov-gap-total";
    summary.textContent = GAPS_FOUND(gaps.length, total);
    const rows = gaps.slice(0, GAP_ROWS).map((g) => {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "ov-gap";
      row.title = MEASURE_THIS;
      const at = document.createElement("span");
      at.textContent = formatOffset(g.from * 8);
      const size = document.createElement("span");
      size.className = "ov-gap-size";
      size.textContent = formatBytes(g.to - g.from);
      row.append(at, size);
      // Measuring a stretch on its own is the only honest way to say what is
      // in it: the numbers above are the whole block's, fields and all.
      row.addEventListener("click", () => {
        this.setBlock(g.from, g.to);
        this.onJump(g.from * 8, g.to * 8);
      });
      return row;
    });
    const extra = gaps.length > GAP_ROWS ? [noneLine(GAPS_MORE(gaps.length - GAP_ROWS))] : [];
    this.focusGaps.replaceChildren(summary, ...rows, ...extra);
  }

  // ----- the map under the pointer -----

  private bucketAt(canvas: HTMLCanvasElement, count: number, e: MouseEvent): number | null {
    const cols = Number(canvas.dataset["cols"] ?? 0);
    if (cols === 0 || count === 0) return null;
    const box = canvas.getBoundingClientRect();
    const col = Math.floor((e.clientX - box.left - MAP_PAD) / (CELL + GAP));
    const row = Math.floor((e.clientY - box.top - MAP_PAD) / (CELL + GAP));
    if (col < 0 || col >= cols || row < 0) return null;
    const i = row * cols + col;
    return i < count ? i : null;
  }

  private onMapHover(e: PointerEvent): void {
    const s = this.state;
    const i = this.bucketAt(this.canvas, s?.classes.length ?? 0, e);
    if (s === null || i === null) {
      this.readout.textContent = BLANK;
      return;
    }
    this.readout.textContent = `${formatOffset(i * s.bucket_bytes * 8)} · ${CLASS_LABEL[Number(s.classes[i])] ?? ""}`;
  }

  private onFocusHover(e: PointerEvent): void {
    const f = this.focusState;
    const i = this.bucketAt(this.focusCanvas, f?.classes.length ?? 0, e);
    if (f === null || i === null) {
      this.focusReadout.textContent = BLANK;
      return;
    }
    const at = (f.start + i * f.bucket_bytes) * 8;
    this.focusReadout.textContent = `${formatOffset(at)} · ${CLASS_LABEL[Number(f.classes[i])] ?? ""}`;
  }
}

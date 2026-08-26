// The file described before it is read: how big it is, what kind of bytes it
// is made of and where they sit, and the major regions a template divides it
// into. It sits beside both the hex grid and the listing, because "what is in
// this file" is the same question from either.
//
// The picture is a map of equal cells, one per bucket of the byte-class scan,
// coloured by what the bucket's bytes are like. The same map answers for a
// file no template covers: a tail of zeros or a compressed middle shows up
// whether or not anything describes it.
//
// A cell of the map stands for a lot of bytes, and a bucket is judged as a
// whole, so a stretch of zeros at the front of one leaves no mark. Picking a
// cell scans that block on its own, at a resolution its own size allows, and
// reports what the whole block's bytes turned out to be.

import { formatBytes, formatOffset } from "./doc.js";
import { NO_TEMPLATE } from "./strings.js";
import type { ContentObject, Doc, FocusState, OverviewState, Span, TemplateNode } from "./doc.js";
import type { FieldPick } from "./listingview.js";

/** Where an object's bytes are, in a phrase: one run, one chunk at a time, or
 *  inside the object's own header. */
function storageText(object: ContentObject): string {
  if (object.storage === "chunked") {
    return `chunks of ${object.chunk_dims.map((d) => d.toLocaleString()).join(" × ")}`;
  }
  if (object.storage === "contiguous") return `${formatBytes(object.bytes)} contiguous`;
  if (object.storage === "compact") return `${formatBytes(object.bytes)} in the object header`;
  return "";
}

/** How the map is drawn: a square this wide plus a one-pixel gap. */
const CELL = 5;
const GAP = 1;
/** The padding either side of the sidebar's contents, which the cells have to
 *  fit inside. Kept in step with `.ov-body` in the stylesheet. */
const BODY_PADDING = 8;

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

/** Top-level fields below this share of the file fold into one row, when
 *  three or more sit together. */
const SMALL_PERCENT = 0.5;
/** Top-level fields fetched for the region list. */
const REGION_LIMIT = 64;
/** Fields fetched to work out what a block leaves undescribed. */
const SPAN_LIMIT = 2048;
/** Undescribed stretches listed for a block. */
const GAP_ROWS = 12;

/** Fill colours for the map cells, light and dark, by class digit. The legend
 *  swatches use the same values, so a colour on the map can be looked up. */
const LIGHT = ["#e9ebee", "#b9bec7", "#4c9a63", "#6b8fd8", "#d08a2e"];
const DARK = ["#23252b", "#4a4f58", "#4f9e63", "#6f93e8", "#cf9440"];

/** What each class is called, by digit. */
const CLASS_LABEL = ["Zeros", "One repeated byte", "Text", "Data", "High entropy"];
const CLASS_TITLE = [
  "Every byte is 0x00",
  "Every byte is the same value, such as 0xFF padding",
  "Mostly printable characters",
  "Structured bytes: headers, tables, machine code",
  "Bytes using the whole 0-255 range about evenly, typical of compressed or encrypted data",
];
/** The same classes inside a sentence. */
const CLASS_PROSE = ["zeros", "one repeated byte", "text", "data", "high-entropy data, likely compressed or encrypted"];

const TITLE = "Overview";
const SIZE_LABEL = "Size";
const TYPE_LABEL = "Type";
const SCALE_LABEL = "Scale";
const UNKNOWN_TYPE = "Not identified";
const REGIONS_TITLE = "Regions";
/** The contents list, for a format that names what it holds. */
const CONTENTS_TITLE = "Objects";
/** Objects listed before the list says how many more there were. */
const CONTENTS_ROWS = 200;
const BLOCK_TITLE = "Block";
const CLOSE_BLOCK = "Close block";
const PICK_BLOCK = "Pick a cell on the map to measure that part of the file on its own.";
const SCANNING = (percent: number): string => `Scanning the file… ${percent}%`;
const SMALL_FIELDS = (n: number): string => `${n} small fields`;
/** Keeps the line under the map from collapsing when the pointer leaves it,
 *  which would jump everything below by a row. */
const BLANK = " ";

/** What the block view measures. */
type FocusMode = "all" | "gaps";
const MODE_LABEL = "Measure";
const MODE_OPTIONS: readonly { readonly value: FocusMode; readonly label: string }[] = [
  { value: "all", label: "All bytes in this block" },
  { value: "gaps", label: "Only bytes no field describes" },
];
const ALL_DESCRIBED = "Every byte in this block belongs to a field.";
const NO_TEMPLATE_GAPS = "No template, so no field describes any of it.";
const GAPS_FOUND = (n: number, bytes: number): string =>
  `${n === 1 ? "1 stretch" : `${n.toLocaleString()} stretches`} no field describes, ${formatBytes(bytes)} in all.`;
const GAPS_MORE = (n: number): string => `${n.toLocaleString()} more not listed.`;
const MEASURE_THIS = "Measure this stretch on its own";

/** One maximal run of buckets sharing a class. */
type Run = { readonly cls: number; readonly start: number; readonly len: number };

/** One row of the region list: a top-level field, or a fold of small ones. */
type Region = {
  readonly path: readonly number[];
  readonly name: string;
  readonly title: string;
  readonly offsetBits: number;
  readonly sizeBits: number;
};

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

/** A share of the whole: `48%`, or `<1%` rather than a zero that reads as
 *  nothing at all. */
function percentText(part: number, whole: number): string {
  if (whole === 0) return "0%";
  const p = (part / whole) * 100;
  if (p > 0 && p < 1) return "<1%";
  if (p < 100 && p > 99) return ">99%";
  return `${Math.round(p)}%`;
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

/** A byte as the hex gutter writes it, with its character where it has one. */
function byteText(v: number): string {
  const hex = `0x${v.toString(16).padStart(2, "0")}`;
  return v >= 0x20 && v < 0x7f ? `${hex} ${String.fromCharCode(v)}` : hex;
}

export class OverviewPanel {
  readonly el: HTMLElement;
  private readonly body: HTMLElement;
  private readonly facts: HTMLElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly readout: HTMLElement;
  private readonly legend: HTMLElement;
  private readonly notes: HTMLElement;
  private readonly contentsEl: HTMLElement;
  private readonly regionsEl: HTMLElement;

  private readonly focusEl: HTMLElement;
  private readonly focusHead: HTMLElement;
  private readonly focusMode: HTMLSelectElement;
  private readonly focusCanvas: HTMLCanvasElement;
  private readonly focusReadout: HTMLElement;
  private readonly focusStats: HTMLElement;
  private readonly focusGaps: HTMLElement;

  private state: OverviewState | null = null;
  private focusState: FocusState | null = null;
  /** The block being looked at, in bytes, or null when none is. */
  private block: { from: number; to: number } | null = null;
  /** Which cell of the block map was last picked, so it stays marked while the
   *  rest of the scan fills in around it. */
  private picked: number | null = null;
  private mode: FocusMode = "all";
  /** Another step is already queued, so a burst of notifies runs one. */
  private stepQueued = false;
  private focusQueued = false;
  /** Buckets to draw brighter than the rest, while a region row is under the
   *  pointer or a block is picked. */
  private highlight: { from: number; to: number } | null = null;
  /** What the toolbar's identification said the file is, and whether it has
   *  answered at all yet. An empty answer and no answer yet are different
   *  things to show. */
  private identity = "";
  private identified = false;

  /** A region row was chosen; same contract as picking a listing row. */
  onPick: (pick: FieldPick) => void = () => {};
  /** A cell or a listed stretch was picked: go there, and mark the bytes it
   *  stands for. A cell is a stretch of the file, not a place in it, so
   *  picking one selects it rather than only moving the cursor to its front. */
  onJump: (startBit: number, endBit: number) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("aside");
    this.el.className = "overview";

    const chevron = document.createElement("span");
    chevron.className = "panel-chevron";
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "ov-toggle";
    toggle.append(chevron, TITLE);
    const header = document.createElement("header");
    header.className = "ov-bar";
    header.append(toggle);

    this.facts = document.createElement("dl");
    this.facts.className = "ov-facts";
    this.canvas = document.createElement("canvas");
    this.canvas.className = "ov-map";
    this.readout = document.createElement("p");
    this.readout.className = "ov-readout";
    this.readout.textContent = BLANK;
    this.legend = document.createElement("div");
    this.legend.className = "ov-legend";
    this.notes = document.createElement("ul");
    this.notes.className = "ov-notes";
    this.contentsEl = document.createElement("div");
    this.contentsEl.className = "ov-regions ov-contents";
    this.regionsEl = document.createElement("div");
    this.regionsEl.className = "ov-regions";

    this.focusEl = document.createElement("section");
    this.focusEl.className = "ov-focus";
    this.focusHead = document.createElement("h3");
    this.focusMode = document.createElement("select");
    this.focusMode.className = "ov-mode";
    this.focusMode.setAttribute("aria-label", MODE_LABEL);
    for (const o of MODE_OPTIONS) {
      const opt = document.createElement("option");
      opt.value = o.value;
      opt.textContent = o.label;
      this.focusMode.append(opt);
    }
    this.focusMode.addEventListener("change", () => {
      this.mode = this.focusMode.value === "gaps" ? "gaps" : "all";
      this.renderFocus();
    });
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
      this.focusMode,
      this.focusCanvas,
      this.focusReadout,
      this.focusStats,
      this.focusGaps,
    );

    this.body = document.createElement("div");
    this.body.className = "ov-body";
    this.body.append(
      this.facts,
      this.canvas,
      this.readout,
      this.legend,
      this.notes,
      this.contentsEl,
      this.regionsEl,
      this.focusEl,
    );
    this.el.append(header, this.body);

    // Folded away to start with: it reads the whole file to fill itself in,
    // and that is worth doing when it is asked for rather than on every open.
    const key = "qubero.overview";
    const apply = (collapsed: boolean): void => {
      this.el.classList.toggle("is-collapsed", collapsed);
      this.body.hidden = collapsed;
      chevron.textContent = collapsed ? "▸" : "▾";
      toggle.setAttribute("aria-expanded", String(!collapsed));
      toggle.title = collapsed ? "Expand" : "Collapse";
    };
    apply(localStorage.getItem(key) !== "open");
    toggle.addEventListener("click", () => {
      const collapsed = !this.el.classList.contains("is-collapsed");
      localStorage.setItem(key, collapsed ? "collapsed" : "open");
      apply(collapsed);
      this.pump();
    });

    this.canvas.addEventListener("pointermove", (e) => this.onMapHover(e));
    this.canvas.addEventListener("pointerleave", () => {
      this.readout.textContent = BLANK;
    });
    this.canvas.addEventListener("click", (e) => this.onMapClick(e));
    this.focusCanvas.addEventListener("pointermove", (e) => this.onFocusHover(e));
    this.focusCanvas.addEventListener("pointerleave", () => {
      this.focusReadout.textContent = BLANK;
    });
    this.focusCanvas.addEventListener("click", (e) => this.onFocusClick(e));

    // The cells wrap to the sidebar's width, so a narrower window is a
    // different map rather than the same one clipped.
    new ResizeObserver(() => this.render()).observe(this.body);
    doc.onChange(() => this.pump());
    this.pump();
  }

  /** The identification's sentence for the file. An empty string means the
   *  rules were asked and had nothing to say, which is worth showing. */
  setIdentity(text: string): void {
    this.identity = text;
    this.identified = true;
    this.render();
  }

  /**
   * Ask for one more step of the scan when there is anything left to do and
   * anyone to see it. A folded-away or hidden map must not pull the whole file
   * through the chunk cache for nobody.
   */
  pump(): void {
    if (this.el.offsetParent === null || this.body.hidden) return;
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
    this.drawFacts(s);
    this.drawMap(this.canvas, s.classes, this.highlight);
    this.drawLegend(s);
    this.drawNotes(s);
    this.drawContents();
    this.drawRegions();
    this.renderFocus();
  }

  private drawFacts(s: OverviewState): void {
    const len = this.doc.lengthBytes;
    const size = len < 1024 ? `${len.toLocaleString()} bytes` : `${formatBytes(len)} (${len.toLocaleString()} bytes)`;
    const rows: [string, string][] = [[SIZE_LABEL, size]];
    const type = this.identity !== "" ? this.identity : this.doc.template ?? (this.identified ? UNKNOWN_TYPE : "");
    if (type !== "") rows.push([TYPE_LABEL, type]);
    rows.push([SCALE_LABEL, `1 cell = ${cellText(s.bucket_bytes)}`]);
    this.facts.replaceChildren(...rows.flatMap(([k, v]) => this.factRow(k, v)));
  }

  private factRow(key: string, value: string): HTMLElement[] {
    const dt = document.createElement("dt");
    dt.textContent = key;
    const dd = document.createElement("dd");
    dd.textContent = value;
    return [dt, dd];
  }

  private colors(): string[] {
    return matchMedia("(prefers-color-scheme: dark)").matches ? DARK : LIGHT;
  }

  /** One map, wherever it is drawn. Cells outside `bright` are dimmed, which
   *  is how a region row or a picked block shows where its bytes sit. */
  private drawMap(canvas: HTMLCanvasElement, classes: string, bright: { from: number; to: number } | null): void {
    const width = this.body.clientWidth - BODY_PADDING * 2;
    if (width <= 0) return;
    const cols = Math.max(8, Math.floor(width / (CELL + GAP)));
    const rows = Math.max(1, Math.ceil(Math.max(1, classes.length) / cols));
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(cols * (CELL + GAP) * dpr);
    canvas.height = Math.round(rows * (CELL + GAP) * dpr);
    canvas.style.width = `${cols * (CELL + GAP)}px`;
    canvas.style.height = `${rows * (CELL + GAP)}px`;
    canvas.dataset["cols"] = String(cols);
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cols * (CELL + GAP), rows * (CELL + GAP));
    const colors = this.colors();
    for (let i = 0; i < classes.length; i++) {
      const cls = Number(classes[i]);
      ctx.globalAlpha = bright === null || (i >= bright.from && i < bright.to) ? 1 : 0.25;
      ctx.fillStyle = colors[cls] ?? colors[3] ?? "#888";
      ctx.fillRect((i % cols) * (CELL + GAP), Math.floor(i / cols) * (CELL + GAP), CELL, CELL);
    }
    ctx.globalAlpha = 1;
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

  private drawNotes(s: OverviewState): void {
    if (!s.done) {
      this.notes.replaceChildren();
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
  }

  // ----- contents -----

  /** What the file says it holds: one row per object, named as the file names
   *  it. This is the question the region list answers from the template, asked
   *  of the file instead, and for a format whose objects are all reached by
   *  address it is the only useful answer: the template's own top level is a
   *  signature and a superblock. */
  private drawContents(): void {
    const reply = this.doc.contents();
    if (reply.status !== "ok" || reply.node.objects.length === 0) {
      this.contentsEl.replaceChildren();
      return;
    }
    const { objects, total, anndata, rows, columns } = reply.node;
    const heading = document.createElement("h3");
    heading.textContent = CONTENTS_TITLE;
    const out: HTMLElement[] = [heading];
    if (anndata) {
      const note = document.createElement("p");
      note.className = "ov-note";
      // What the file says about itself and what its shape gives away are two
      // different claims, and only one of them is the file's own word. The
      // counts are left out where nothing said what they are.
      const said = reply.node.encoding === "anndata" ? "AnnData" : "Looks like AnnData";
      const counts =
        rows > 0 && columns > 0
          ? `: ${rows.toLocaleString()} observations × ${columns.toLocaleString()} variables`
          : "";
      note.textContent = said + counts;
      out.push(note);
    }
    for (const object of objects.slice(0, CONTENTS_ROWS)) out.push(this.contentRow(object));
    if (objects.length < total) {
      const more = document.createElement("p");
      more.className = "ov-note";
      more.textContent = `Showing ${objects.length.toLocaleString()} of ${total.toLocaleString()} objects.`;
      out.push(more);
    }
    this.contentsEl.replaceChildren(...out);
  }

  /** One object: what it is called, and what it holds. */
  private contentRow(object: ContentObject): HTMLElement {
    const row = document.createElement("button");
    row.type = "button";
    row.className = object.group ? "ov-region ov-object is-group" : "ov-region ov-object";
    const name = document.createElement("span");
    name.className = "ov-region-name";
    name.textContent = object.name;
    const about = document.createElement("span");
    about.className = "ov-region-size";
    // The root has no shape and no bytes of its own, and an empty line beside
    // rows that have both reads as "nothing known" rather than "this is the
    // container everything else hangs under".
    about.textContent = [
      object.name === "/" && object.encoding === "" ? "root group" : object.encoding,
      object.shape.map((d) => d.toLocaleString()).join(" × "),
      object.element,
      storageText(object),
      object.filters.join(" then "),
    ]
      .filter((part) => part !== "")
      .join(" · ");
    row.title = `${object.name} · header at ${formatOffset(object.address * 8)}`;
    row.append(name, about);
    row.addEventListener("click", () =>
      this.onPick({ path: object.path, startBit: object.address * 8, endBit: object.address * 8 + 8 }),
    );
    return row;
  }

  // ----- regions -----

  /** Top-level fields folded down to rows worth reading: a run of three or
   *  more tiny ones becomes a single row, so a header's bookkeeping does not
   *  outnumber the parts of the file that have any size to them. */
  private regionRows(children: readonly TemplateNode[]): Region[] {
    const fileBits = this.doc.lengthBits;
    // A structure keeps its row however small it is: the metadata of a model
    // file is a region of the file even at a few hundred bytes. Only plain
    // fields, the counts and signatures of a header, fold together.
    const small = (n: TemplateNode): boolean => !n.composite && n.size_bits < (fileBits * SMALL_PERCENT) / 100;
    const out: Region[] = [];
    let fold: TemplateNode[] = [];
    const flush = (): void => {
      if (fold.length === 0) return;
      if (fold.length < 3) {
        for (const n of fold) out.push(this.regionOf(n));
      } else {
        const first = fold[0];
        const last = fold[fold.length - 1];
        if (first !== undefined && last !== undefined) {
          out.push({
            path: first.path,
            name: SMALL_FIELDS(fold.length),
            title: fold.map((n) => n.name).join(", "),
            offsetBits: first.offset_bits,
            sizeBits: last.offset_bits + last.size_bits - first.offset_bits,
          });
        }
      }
      fold = [];
    };
    for (const n of children) {
      if (small(n)) fold.push(n);
      else {
        flush();
        out.push(this.regionOf(n));
      }
    }
    flush();
    return out;
  }

  private regionOf(n: TemplateNode): Region {
    return { path: n.path, name: n.name, title: n.name, offsetBits: n.offset_bits, sizeBits: n.size_bits };
  }

  private drawRegions(): void {
    const heading = document.createElement("h3");
    heading.textContent = REGIONS_TITLE;
    if (this.doc.template === null) {
      const p = document.createElement("p");
      p.className = "ov-none";
      p.textContent = NO_TEMPLATE;
      this.regionsEl.replaceChildren(heading, p);
      return;
    }
    const r = this.doc.templateChildren([], 0, REGION_LIMIT);
    if (r.status !== "ok") return;
    const fileBits = Math.max(1, this.doc.lengthBits);
    const rows = this.regionRows(r.node).map((region) => {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "ov-region";
      row.title = `${region.title}  ·  ${formatOffset(region.offsetBits)}`;
      const name = document.createElement("span");
      name.className = "ov-region-name";
      name.textContent = region.name;
      const size = document.createElement("span");
      size.className = "ov-region-size";
      size.textContent = `${formatBytes(Math.ceil(region.sizeBits / 8))} · ${percentText(region.sizeBits, fileBits)}`;
      const bar = document.createElement("span");
      bar.className = "ov-region-bar";
      const fill = document.createElement("i");
      fill.style.width = `${Math.max(1, (region.sizeBits / fileBits) * 100)}%`;
      bar.append(fill);
      row.append(name, size, bar);
      row.addEventListener("pointerenter", () => this.highlightRange(region.offsetBits, region.sizeBits));
      row.addEventListener("pointerleave", () => this.highlightRange(null, 0));
      row.addEventListener("click", () =>
        this.onPick({
          path: region.path,
          startBit: region.offsetBits,
          endBit: region.offsetBits + region.sizeBits,
        }),
      );
      return row;
    });
    this.regionsEl.replaceChildren(heading, ...rows);
  }

  /** Dim the rest of the map, so the row under the pointer shows where its
   *  bytes sit. Passing null puts the map back to the picked block, or to all
   *  of it when no block is picked. */
  private highlightRange(offsetBits: number | null, sizeBits: number): void {
    const s = this.state;
    if (s === null) return;
    const bucketBits = s.bucket_bytes * 8;
    const range = offsetBits === null ? this.blockBuckets() : { offsetBits, sizeBits };
    if (range === null) this.highlight = null;
    else {
      const from = Math.floor(range.offsetBits / bucketBits);
      this.highlight = { from, to: Math.max(from + 1, Math.ceil((range.offsetBits + range.sizeBits) / bucketBits)) };
    }
    this.drawMap(this.canvas, s.classes, this.highlight);
  }

  private blockBuckets(): { offsetBits: number; sizeBits: number } | null {
    const b = this.block;
    return b === null ? null : { offsetBits: b.from * 8, sizeBits: (b.to - b.from) * 8 };
  }

  // ----- the block being looked at -----

  private onMapClick(e: MouseEvent): void {
    const s = this.state;
    const i = this.bucketAt(this.canvas, s?.classes.length ?? 0, e);
    if (s === null || i === null) return;
    const from = i * s.bucket_bytes;
    const to = Math.min(this.doc.lengthBytes, from + s.bucket_bytes);
    this.setBlock(from, to);
    this.onJump(from * 8, to * 8);
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
    this.picked = i;
    this.drawMap(this.focusCanvas, f.classes, { from: i, to: i + 1 });
    this.onJump(from * 8, to * 8);
  }

  /** Look at one stretch of the file on its own. */
  private setBlock(from: number, to: number): void {
    this.block = to > from ? { from, to } : null;
    this.focusState = null;
    this.picked = null;
    this.highlightRange(null, 0);
    this.renderFocus();
    this.pump();
  }

  private renderFocus(): void {
    const block = this.block;
    if (block === null) {
      const p = document.createElement("p");
      p.className = "ov-none";
      p.textContent = PICK_BLOCK;
      this.focusEl.replaceChildren(p);
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
    this.focusMode.value = this.mode;
    this.focusEl.replaceChildren(
      this.focusHead,
      this.focusMode,
      this.focusCanvas,
      this.focusReadout,
      this.focusStats,
      this.focusGaps,
    );
    const f = this.focusState;
    const cell = this.picked;
    if (f !== null) this.drawMap(this.focusCanvas, f.classes, cell === null ? null : { from: cell, to: cell + 1 });
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
    this.focusStats.replaceChildren(...rows.flatMap(([k, v]) => this.factRow(k, v)));
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
    if (this.mode !== "gaps") {
      this.focusGaps.replaceChildren();
      return;
    }
    if (this.doc.template === null) {
      this.focusGaps.replaceChildren(this.noneLine(NO_TEMPLATE_GAPS));
      return;
    }
    const gaps = this.gapsIn(block);
    if (gaps === null) return;
    if (gaps.length === 0) {
      this.focusGaps.replaceChildren(this.noneLine(ALL_DESCRIBED));
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
    const extra = gaps.length > GAP_ROWS ? [this.noneLine(GAPS_MORE(gaps.length - GAP_ROWS))] : [];
    this.focusGaps.replaceChildren(summary, ...rows, ...extra);
  }

  private noneLine(text: string): HTMLElement {
    const p = document.createElement("p");
    p.className = "ov-none";
    p.textContent = text;
    return p;
  }

  // ----- the map under the pointer -----

  private bucketAt(canvas: HTMLCanvasElement, count: number, e: MouseEvent): number | null {
    const cols = Number(canvas.dataset["cols"] ?? 0);
    if (cols === 0 || count === 0) return null;
    const box = canvas.getBoundingClientRect();
    const col = Math.floor((e.clientX - box.left) / (CELL + GAP));
    const row = Math.floor((e.clientY - box.top) / (CELL + GAP));
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

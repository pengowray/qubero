// The rail down the left: the file described before it is read, and the way
// around it. How big it is, what kind of bytes it is made of and where they
// sit, the parts a template divides it into and which one the main view is
// looking at. It sits beside the hex grid, the listing and the text view,
// because "what is in this file, and where am I" is the same question from
// any of them.
//
// Top to bottom: the facts, the byte-class map with a layout strip under it,
// the Contents (the listing's headings, one source of truth for the parts of
// the file) with a Logical tab beside it for formats that have their own
// objects, the notes, and the block detail.
//
// The class map is a grid of equal cells, one per bucket of the byte-class
// scan, coloured by what the bucket's bytes are like. The same map answers for
// a file no template covers: a tail of zeros or a compressed middle shows up
// whether or not anything describes it. A cell stands for a lot of bytes, and
// a bucket is judged as a whole, so picking a cell scans that block on its own
// and reports what the block's bytes turned out to be.

import { formatBytes, formatOffset } from "./doc.js";
import { NO_TEMPLATE, REPORT } from "./strings.js";
import type { Doc, FocusState, OverviewState, Span } from "./doc.js";
import type { FieldPick } from "./doc.js";
import { fileMap, markMap, segmentWidths } from "./filemap.js";
import type { MapMark, MapSegment } from "./filemap.js";
import type { OutlineHeading, Viewport } from "./outline.js";
import { hasLogicalOutline, logicalLength, logicalOutline } from "./logicaloutline.js";
import type { LogicalNode, LogicalOutline } from "./logicaloutline.js";

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

/** Top-level parts listed before the list says how many more there are. A
 *  SQLite file of a hundred thousand pages is not a hundred thousand buttons;
 *  the part the view is in is always listed, wherever it falls. */
const PARTS_SHOWN = 200;
/** The layout strip draws one cell per part. Past this many, neighbouring
 *  parts share a cell: a strip of a hundred thousand two-pixel cells would be
 *  neither drawable nor readable. */
const STRIP_SEGMENTS = 256;
/** Fields fetched to work out what a block leaves undescribed. */
const SPAN_LIMIT = 2048;
/** Undescribed stretches listed for a block. */
const GAP_ROWS = 12;
/** Rows a "show more" press adds to a logical section. */
const LOGICAL_PAGE = 80;
/** How far each level of the logical tree steps in. */
const LOGICAL_INDENT_PX = 12;

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
/** The strip under the class map, when pointed at. */
const LAYOUT_TITLE = "The parts of the file in order, each as wide as its share of the bytes";
const CONTENTS_TAB = "Contents";
const LOGICAL_TAB = "Logical";
/** A template is chosen and the listing has not walked it yet. */
const PARTS_PENDING = "Listing the parts of the file…";
/** A template is chosen and walking it found no parts to list. */
const NO_PARTS = "No parts to list. This template describes single fields, not parts of the file.";
const MORE_PARTS = (n: number): string => `${n.toLocaleString()} more not listed`;
const EXPAND = "Expand";
const COLLAPSE = "Collapse";
const LOGICAL_READING = (bytes: number): string => `Reading the objects… ${formatBytes(bytes)} read so far`;
const LOGICAL_FAILED = (message: string): string => `Couldn't read the objects: ${message}`;
const LOGICAL_MORE = (count: number, label: string): string =>
  `Show ${Math.min(LOGICAL_PAGE, count).toLocaleString()} more (${count.toLocaleString()} ${label} not shown)`;
const LOGICAL_UNLISTED = (n: number): string => `${n.toLocaleString()} more objects not listed`;
const BLOCK_TITLE = "Block";
const CLOSE_BLOCK = "Close block";
const PICK_BLOCK = "Pick a cell on the map to measure that part of the file on its own.";
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

/** One top-level part of the file and the named parts inside it. */
type Part = { readonly head: OutlineHeading; readonly subs: readonly OutlineHeading[] };

/** Which part the main view is looking at: an index into the parts, and the
 *  index of the named part inside it, or -1 when the top of the view is not
 *  inside one of those. */
type Place = { readonly part: number; readonly sub: number };

type Tab = "contents" | "logical";

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

function pathKey(path: readonly number[]): string {
  return path.join("/");
}

/** What a heading is called. A run of plain fields has no name of its own,
 *  and the listing names it by where it sits; the rail says the same word. */
function headingName(h: OutlineHeading, fileBits: number): string {
  if (h.name !== "") return h.name;
  const where = h.offsetBits === 0 ? "start" : fileBits > 0 && h.offsetBits + h.sizeBits >= fileBits ? "end" : "middle";
  return REPORT.unnamedPart(where);
}

/** The headings grouped into parts: each level-0 heading with the level-1
 *  headings of its section. The outline is in file order, so a level-1
 *  heading follows the level-0 one it is inside. */
function partsOf(headings: readonly OutlineHeading[]): Part[] {
  const out: { head: OutlineHeading; subs: OutlineHeading[] }[] = [];
  for (const h of headings) {
    if (h.level === 0) out.push({ head: h, subs: [] });
    else {
      const last = out[out.length - 1];
      if (last !== undefined && last.head.section === h.section) last.subs.push(h);
    }
  }
  return out;
}

function sameHeading(a: OutlineHeading, b: OutlineHeading): boolean {
  return (
    a.key === b.key &&
    a.level === b.level &&
    a.section === b.section &&
    a.offsetBits === b.offsetBits &&
    a.sizeBits === b.sizeBits &&
    a.name === b.name &&
    a.color === b.color
  );
}

/** The last index whose heading starts at or before `bit`, or -1. */
function partAt(parts: readonly Part[], bit: number): number {
  let lo = 0;
  let hi = parts.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const head = parts[mid]?.head;
    if (head !== undefined && head.offsetBits <= bit) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

function samePlace(a: Place | null, b: Place | null): boolean {
  if (a === null || b === null) return a === b;
  return a.part === b.part && a.sub === b.sub;
}

/** Neighbouring parts merged until the strip has few enough cells to draw.
 *  Each merged cell keeps the colour of its first part. */
function stripSegments(parts: readonly Part[]): MapSegment[] {
  const per = Math.max(1, Math.ceil(parts.length / STRIP_SEGMENTS));
  const out: MapSegment[] = [];
  for (let i = 0; i < parts.length; i += per) {
    const first = parts[i]?.head;
    const last = parts[Math.min(parts.length, i + per) - 1]?.head;
    if (first === undefined || last === undefined) continue;
    out.push({
      offsetBits: first.offsetBits,
      sizeBits: last.offsetBits + last.sizeBits - first.offsetBits,
      color: first.color,
    });
  }
  return out;
}

export class OverviewPanel {
  readonly el: HTMLElement;
  private readonly body: HTMLElement;
  private readonly facts: HTMLElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly readout: HTMLElement;
  private readonly legend: HTMLElement;
  private readonly layout: HTMLElement;
  private layoutStrip: HTMLElement | null = null;
  private readonly tabs: HTMLElement;
  private readonly contentsTab: HTMLButtonElement;
  private readonly logicalTab: HTMLButtonElement;
  private readonly contentsEl: HTMLElement;
  private readonly logicalEl: HTMLElement;
  private readonly notes: HTMLElement;

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
  /** Which cell of the block map was last picked, so it stays marked while the
   *  rest of the scan fills in around it. */
  private picked: number | null = null;
  /** Another step is already queued, so a burst of notifies runs one. */
  private stepQueued = false;
  private focusQueued = false;
  /** Buckets to draw brighter than the rest, while a part is under the
   *  pointer or a block is picked. */
  private highlight: { from: number; to: number } | null = null;
  /** What the toolbar's identification said the file is, and whether it has
   *  answered at all yet. An empty answer and no answer yet are different
   *  things to show. */
  private identity = "";
  private identified = false;

  // ----- the contents -----

  /** The parts of the file as the listing last named them, and which template
   *  they were named under. A template that differs from the document's is a
   *  walk still owed: the listing has not caught up yet. Null before the
   *  listing has ever answered. */
  private headings: readonly OutlineHeading[] | null = null;
  private headingsTemplate: string | null = null;
  /** The template the list on screen was drawn for, so a change of template
   *  redraws it whether or not the listing has answered yet. */
  private drawnTemplate: string | null = null;
  private parts: readonly Part[] = [];
  /** The part the main view is looking at. */
  private place: Place | null = null;
  /** The stretch of the file the main view is showing, for the layout strip. */
  private viewport: Viewport | null = null;
  /** The part under the pointer, lit on both maps while it is. */
  private hovering: MapMark = null;
  /** The rows on screen, by part index, and the one list of named parts open
   *  under the current part. */
  private partRows = new Map<number, HTMLElement>();
  private subsEl: HTMLElement | null = null;
  private subRows: HTMLElement[] = [];
  /** A standing note about the template, shown above the parts. */
  private note = "";
  private tab: Tab = "contents";

  // ----- the logical outline -----

  private readonly logicalExpanded = new Set<string>(["/"]);
  private readonly logicalShown = new Map<string, number>();
  private logical: LogicalOutline | null = null;
  /** Logical rows can share one storage node while pointing at different
   *  byte extents, as ISO directory entries do, so the selection is the row's
   *  own id and the field's path is only a way to find it. */
  private selectedLogicalId: string | null = null;
  private selectedPath: string | null = null;
  /** The tree on screen is behind the document: it was skipped while the tab
   *  or the rail was hidden. */
  private logicalStale = true;
  private logicalTimer = 0;

  /** A part or an object was chosen; same contract as picking a listing row. */
  onPick: (pick: FieldPick) => void = () => {};
  /** A cell or a listed stretch was picked: go there, and mark the bytes it
   *  stands for. A cell is a stretch of the file, not a place in it, so
   *  picking one selects it rather than only moving the cursor to its front. */
  onJump: (startBit: number, endBit: number) => void = () => {};
  /** Ctrl+click on an object whose field holds an offset: go to where it
   *  points. */
  onGoTo: (bitOffset: number) => void = () => {};

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
    this.layout = document.createElement("div");
    this.layout.className = "ov-layout";
    this.layout.hidden = true;

    this.tabs = document.createElement("div");
    this.tabs.className = "ov-tabs";
    this.tabs.setAttribute("role", "tablist");
    this.contentsTab = this.tabButton(CONTENTS_TAB, "contents");
    this.logicalTab = this.tabButton(LOGICAL_TAB, "logical");
    this.tabs.append(this.contentsTab, this.logicalTab);
    this.contentsEl = document.createElement("div");
    this.contentsEl.className = "ov-parts";
    this.contentsEl.setAttribute("role", "tabpanel");
    this.logicalEl = document.createElement("div");
    this.logicalEl.className = "ov-logical";
    this.logicalEl.setAttribute("role", "tabpanel");
    this.notes = document.createElement("ul");
    this.notes.className = "ov-notes";

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

    this.body = document.createElement("div");
    this.body.className = "ov-body";
    this.body.append(
      this.facts,
      this.canvas,
      this.readout,
      this.legend,
      this.layout,
      this.tabs,
      this.contentsEl,
      this.logicalEl,
      this.notes,
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
      // Nothing could scroll while the body was hidden, so the mark may be
      // anywhere in the list.
      if (!collapsed) this.showPlace();
    });

    const savedTab = localStorage.getItem("qubero.rail.tab");
    this.tab = savedTab === "logical" ? "logical" : "contents";
    this.syncTabs();

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
    this.logicalEl.addEventListener("click", (e) => this.onLogicalClick(e));

    // The cells wrap to the sidebar's width, so a narrower window is a
    // different map rather than the same one clipped.
    new ResizeObserver(() => {
      this.render();
      this.drawLayout();
    }).observe(this.body);
    doc.onChange(() => {
      this.logical = null;
      this.logicalStale = true;
      this.syncTabs();
      // A template chosen since the listing last walked the file is a list of
      // parts that is about to be replaced, and saying so beats showing the
      // old one as if it were the new.
      if (doc.template !== this.drawnTemplate) this.drawContents();
      this.pump();
    });
    this.drawContents();
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
   * A standing note about the template, above the parts. A generated
   * signature template has one part or none, which without a word of
   * explanation reads as a format Qubero supports badly rather than one it
   * only names.
   */
  setNote(text: string): void {
    if (text === this.note) return;
    this.note = text;
    this.drawContents();
  }

  /**
   * Take the parts of the file from the listing. Answers whether they differ
   * from the ones already up: the listing walks the file again for every
   * batch of chunks a streamed file delivers, and most walks name the same
   * parts, so nothing is rebuilt for those.
   */
  setOutline(headings: readonly OutlineHeading[]): boolean {
    const old = this.headings;
    const template = this.doc.template;
    const same =
      old !== null &&
      template === this.headingsTemplate &&
      old.length === headings.length &&
      old.every((h, i) => {
        const other = headings[i];
        return other !== undefined && sameHeading(h, other);
      });
    if (same) return false;
    this.headings = headings;
    this.headingsTemplate = template;
    this.parts = partsOf(headings);
    this.place = this.viewport === null ? null : this.placeOf(this.viewport);
    this.drawContents();
    this.drawLayout();
    return true;
  }

  /**
   * The stretch of the file the main view is showing. The part its top is in
   * is marked, its named parts are unfolded under it, and the rail scrolls to
   * keep the mark on screen. Only the mark moves the rail: a view that says
   * it is still where it was must not fight the reader scrolling the rail.
   */
  setViewport(v: Viewport): void {
    this.viewport = v;
    if (this.layoutStrip !== null && !this.hovering) this.markLayout(this.viewportMark());
    const place = this.placeOf(v);
    if (samePlace(place, this.place)) return;
    const wasPart = this.place?.part;
    this.place = place;
    // The row for the new part may be past the listed ones, and the old
    // part's row may have been listed only because the view was in it.
    if (
      (place !== null && !this.partRows.has(place.part)) ||
      (wasPart !== undefined && wasPart >= PARTS_SHOWN && (place === null || place.part !== wasPart))
    ) {
      this.drawContents();
      return;
    }
    this.markPlace();
    this.showPlace();
  }

  /** Select the object that stands for the field at `path`, in the Logical
   *  tab. The Contents mark follows the view, not the cursor, so it is not
   *  moved here. */
  reveal(path: readonly number[]): void {
    const k = pathKey(path);
    if (k === this.selectedPath) return;
    this.selectedPath = k;
    this.selectedLogicalId = null;
    if (this.logical !== null) {
      const node = this.logical.nodes.find((candidate) => pathKey(candidate.sourcePath) === k);
      if (node !== undefined) {
        this.selectedLogicalId = node.id;
        const byId = new Map(this.logical.nodes.map((candidate) => [candidate.id, candidate]));
        let parent = node.parentId;
        while (parent !== null) {
          this.logicalExpanded.add(parent);
          parent = byId.get(parent)?.parentId ?? null;
        }
      }
    }
    this.scheduleLogical();
  }

  /** Drop the selection in the Logical tab. */
  clearSelection(): void {
    if (this.selectedPath === null && this.selectedLogicalId === null) return;
    this.selectedPath = null;
    this.selectedLogicalId = null;
    this.scheduleLogical();
  }

  /**
   * Ask for one more step of the scan when there is anything left to do and
   * anyone to see it. A folded-away or hidden map must not pull the whole file
   * through the chunk cache for nobody. The Logical tab catches up here too,
   * for the same reason: it is drawn when it can be seen.
   */
  pump(): void {
    if (this.el.offsetParent === null || this.body.hidden) return;
    this.pumpMap();
    this.pumpFocus();
    if (this.logicalStale && this.tab === "logical") this.scheduleLogical();
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
   *  is how a part under the pointer or a picked block shows where its bytes
   *  sit. */
  private drawMap(canvas: HTMLCanvasElement, classes: string, bright: { from: number; to: number } | null): void {
    const width = this.innerWidth();
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

  /** The width the body's contents have, inside its padding. */
  private innerWidth(): number {
    return this.body.clientWidth - BODY_PADDING * 2;
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

  // ----- the layout strip -----

  /** The parts of the file as one row under the class map, every part lit in
   *  its own colour, and the stretch the main view is showing marked on it.
   *  The two maps say different things and sit together so the reader can
   *  see "the zeros are the unused half of page 1". */
  private drawLayout(): void {
    const segments = stripSegments(this.parts);
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

  // ----- the tabs -----

  private tabButton(label: string, tab: Tab): HTMLButtonElement {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "ov-tab";
    b.textContent = label;
    b.setAttribute("role", "tab");
    b.addEventListener("click", () => {
      this.tab = tab;
      localStorage.setItem("qubero.rail.tab", tab);
      this.syncTabs();
      if (tab === "contents") this.showPlace();
      else this.pump();
    });
    return b;
  }

  /** Show the chosen tab, and only offer Logical where a format has objects
   *  of its own to list. The template is sniffed after the document opens, so
   *  the offer can appear, or go, later. */
  private syncTabs(): void {
    const hasLogical = hasLogicalOutline(this.doc);
    this.logicalTab.hidden = !hasLogical;
    if (!hasLogical && this.tab === "logical") this.tab = "contents";
    const logical = this.tab === "logical";
    this.contentsTab.classList.toggle("is-on", !logical);
    this.logicalTab.classList.toggle("is-on", logical);
    this.contentsTab.setAttribute("aria-selected", String(!logical));
    this.logicalTab.setAttribute("aria-selected", String(logical));
    this.contentsEl.hidden = logical;
    this.logicalEl.hidden = !logical;
  }

  // ----- the contents -----

  /** Where the main view is, in the parts: the part whose bytes hold the top
   *  of the view, and the named part inside it that does. A view whose top
   *  is before the first part but which reaches into it is in that one. */
  private placeOf(v: Viewport): Place | null {
    const parts = this.parts;
    let i = partAt(parts, v.startBit);
    if (i < 0) {
      const first = parts[0];
      if (first === undefined || first.head.offsetBits >= v.endBit) return null;
      i = 0;
    }
    const part = parts[i];
    if (part === undefined) return null;
    let sub = -1;
    for (let j = 0; j < part.subs.length; j++) {
      const s = part.subs[j];
      if (s === undefined || s.offsetBits > v.startBit) break;
      if (v.startBit < s.offsetBits + s.sizeBits) sub = j;
    }
    return { part: i, sub };
  }

  /** Build the list of parts again: the first so many, the one the view is
   *  in wherever it falls, and a count for the rest. */
  private drawContents(): void {
    this.drawnTemplate = this.doc.template;
    this.partRows = new Map();
    this.subsEl = null;
    this.subRows = [];
    const out: HTMLElement[] = [];
    if (this.note !== "") out.push(this.noteLine(this.note));
    if (this.doc.template === null) {
      out.push(this.noneLine(NO_TEMPLATE));
      this.contentsEl.replaceChildren(...out);
      return;
    }
    if (this.headings === null || this.headingsTemplate !== this.doc.template) {
      out.push(this.noneLine(PARTS_PENDING));
      this.contentsEl.replaceChildren(...out);
      return;
    }
    const parts = this.parts;
    if (parts.length === 0) {
      out.push(this.noneLine(NO_PARTS));
      this.contentsEl.replaceChildren(...out);
      return;
    }
    const current = this.place?.part ?? -1;
    const shown = Math.min(parts.length, PARTS_SHOWN);
    for (let i = 0; i < shown; i++) out.push(...this.partRow(i));
    if (current >= shown) {
      if (current > shown) out.push(this.noneLine(MORE_PARTS(current - shown)));
      out.push(...this.partRow(current));
      if (parts.length - current - 1 > 0) out.push(this.noneLine(MORE_PARTS(parts.length - current - 1)));
    } else if (parts.length > shown) {
      out.push(this.noneLine(MORE_PARTS(parts.length - shown)));
    }
    this.contentsEl.replaceChildren(...out);
    this.markPlace();
    this.showPlace();
  }

  /** One part's row, and under it the list of its named parts when the view
   *  is in it. */
  private partRow(i: number): HTMLElement[] {
    const part = this.parts[i];
    if (part === undefined) return [];
    const row = this.headingRow(part.head, 0);
    row.dataset["part"] = String(i);
    this.partRows.set(i, row);
    row.addEventListener("click", () => this.pickHeading(part.head));
    const out = [row];
    if (this.place?.part === i) {
      const subs = this.subList(part);
      if (subs !== null) out.push(subs);
    }
    return out;
  }

  /** The named parts inside one part, as rows under its own. Null for a part
   *  with none. */
  private subList(part: Part): HTMLElement | null {
    if (part.subs.length === 0) return null;
    const subs = document.createElement("div");
    subs.className = "ov-subs";
    this.subRows = part.subs.map((h, j) => {
      const sub = this.headingRow(h, 1);
      sub.dataset["sub"] = String(j);
      sub.addEventListener("click", () => this.pickHeading(h));
      return sub;
    });
    subs.append(...this.subRows);
    this.subsEl = subs;
    return subs;
  }

  private headingRow(h: OutlineHeading, level: 0 | 1): HTMLElement {
    const fileBits = Math.max(1, this.doc.lengthBits);
    const row = document.createElement("button");
    row.type = "button";
    row.className = level === 0 ? "ov-part" : "ov-part ov-part-sub";
    const bytes = formatBytes(Math.ceil(h.sizeBits / 8));
    const called = headingName(h, fileBits);
    row.title = `${called} · ${formatOffset(h.offsetBits)} · ${bytes}`;
    const swatch = document.createElement("i");
    swatch.className = "ov-swatch";
    swatch.style.background = h.color;
    const name = document.createElement("span");
    name.className = "ov-part-name";
    name.textContent = called;
    const size = document.createElement("span");
    size.className = "ov-part-size";
    size.textContent = bytes;
    const share = document.createElement("span");
    share.className = "ov-part-share";
    share.textContent = percentText(h.sizeBits, fileBits);
    row.append(swatch, name, size, share);
    row.addEventListener("pointerenter", () => this.hoverPart(h));
    row.addEventListener("pointerleave", () => this.hoverPart(null));
    return row;
  }

  private pickHeading(h: OutlineHeading): void {
    this.onPick({ path: h.path, startBit: h.offsetBits, endBit: h.offsetBits + h.sizeBits });
  }

  /** Where the part under the pointer sits, on both maps. */
  private hoverPart(h: OutlineHeading | null): void {
    this.hovering = h === null ? null : { offsetBits: h.offsetBits, sizeBits: h.sizeBits };
    this.highlightRange(h === null ? null : h.offsetBits, h?.sizeBits ?? 0);
    this.markLayout(this.hovering ?? this.viewportMark());
  }

  /** Put the mark on the row for the part the view is in, and take it off
   *  the others. The named parts under the current part are built in
   *  `drawContents`; this only moves the mark among rows already up. */
  private markPlace(): void {
    const place = this.place;
    for (const [i, row] of this.partRows) {
      const on = place !== null && place.part === i;
      row.classList.toggle("is-current", on && place.sub < 0);
      row.classList.toggle("is-inside", on && place.sub >= 0);
      if (on) row.setAttribute("aria-current", "location");
      else row.removeAttribute("aria-current");
    }
    // The list of named parts is the current part's. A different part is a
    // different list, so it is built again rather than moved.
    const subsFor = this.subsEl?.previousElementSibling;
    const subsPart = subsFor instanceof HTMLElement ? Number(subsFor.dataset["part"]) : NaN;
    if (place === null || subsPart !== place.part) {
      this.subsEl?.remove();
      this.subsEl = null;
      this.subRows = [];
      const row = place === null ? undefined : this.partRows.get(place.part);
      const part = place === null ? undefined : this.parts[place.part];
      if (row !== undefined && part !== undefined) {
        const subs = this.subList(part);
        if (subs !== null) row.after(subs);
      }
    }
    this.subRows.forEach((row, j) => {
      const on = place !== null && place.sub === j;
      row.classList.toggle("is-current", on);
      if (on) row.setAttribute("aria-current", "location");
      else row.removeAttribute("aria-current");
    });
  }

  /** Scroll the rail so the marked row can be seen. */
  private showPlace(): void {
    if (this.body.hidden || this.tab !== "contents") return;
    const place = this.place;
    if (place === null) return;
    const row = place.sub >= 0 ? this.subRows[place.sub] : this.partRows.get(place.part);
    row?.scrollIntoView({ block: "nearest" });
  }

  // ----- the logical outline -----

  /** Draw the tree again once, however many things asked. A timeout rather
   *  than an animation frame: a page in a background tab paints no frames,
   *  and a tab that stops following the cursor until the window is looked at
   *  again is worse than one that draws unseen. */
  private scheduleLogical(): void {
    this.logicalStale = true;
    if (this.logicalTimer !== 0) return;
    this.logicalTimer = window.setTimeout(() => {
      this.logicalTimer = 0;
      this.renderLogical();
    }, 0);
  }

  private renderLogical(): void {
    if (this.tab !== "logical" || this.body.hidden || this.el.offsetParent === null) return;
    const reply = logicalOutline(this.doc, this.logicalExpanded, this.logicalShown);
    if (reply === null) {
      this.logical = null;
      this.logicalStale = false;
      this.syncTabs();
      return;
    }
    if (reply.status !== "ok") {
      this.logical = null;
      // Bytes still to come run this again through `pump` when they land; an
      // error stays until the document changes.
      this.logicalStale = reply.status !== "error";
      this.logicalEl.replaceChildren(
        this.noneLine(reply.status === "error" ? LOGICAL_FAILED(reply.message) : LOGICAL_READING(reply.reachedBytes)),
      );
      return;
    }
    this.logicalStale = false;
    this.logical = reply.node;
    // A selection made by path before the tree was read finds its row now.
    if (this.selectedLogicalId === null && this.selectedPath !== null) {
      this.selectedLogicalId =
        reply.node.nodes.find((node) => pathKey(node.sourcePath) === this.selectedPath)?.id ?? null;
    }
    const out: HTMLElement[] = [this.noteLine(`${reply.node.title} · ${reply.node.summary}`)];
    if (reply.node.progressText !== undefined) out.push(this.noneLine(reply.node.progressText));
    const byId = new Map(reply.node.nodes.map((node) => [node.id, node]));
    const parents = new Set(reply.node.nodes.flatMap((node) => (node.parentId === null ? [] : [node.parentId])));
    const moreByAfter = new Map(reply.node.more?.map((more) => [more.afterId, more]) ?? []);
    for (const node of reply.node.nodes) {
      if (!this.logicalVisible(node, byId)) continue;
      out.push(this.logicalRow(node, node.hasChildren || parents.has(node.id)));
      const more = moreByAfter.get(node.id);
      if (more !== undefined) out.push(this.logicalMoreRow(more.sectionId, more.count, more.label));
    }
    if ((reply.node.more?.length ?? 0) === 0 && reply.node.nodes.length < reply.node.total) {
      out.push(this.noneLine(LOGICAL_UNLISTED(reply.node.total - reply.node.nodes.length)));
    }
    this.logicalEl.replaceChildren(...out);
    this.logicalEl.querySelector(".is-selected")?.scrollIntoView({ block: "nearest" });
  }

  private logicalVisible(node: LogicalNode, byId: ReadonlyMap<string, LogicalNode>): boolean {
    let parent = node.parentId;
    while (parent !== null) {
      if (!this.logicalExpanded.has(parent)) return false;
      parent = byId.get(parent)?.parentId ?? null;
    }
    return true;
  }

  /** One object: what it is called and how big it is on the first line; its
   *  shape, its type and where its header is on the second. */
  private logicalRow(node: LogicalNode, hasChildren: boolean): HTMLElement {
    const row = document.createElement("div");
    row.className = node.group ? "ov-lrow is-group" : "ov-lrow";
    row.dataset["logicalId"] = node.id;
    row.dataset["path"] = pathKey(node.sourcePath);
    if (node.sourceBits !== null) row.dataset["start"] = String(node.sourceBits);
    row.title = node.title;
    row.style.paddingLeft = `${node.depth * LOGICAL_INDENT_PX + 4}px`;
    if (this.selectedLogicalId === node.id) row.classList.add("is-selected");
    const head = document.createElement("div");
    head.className = "ov-lrow-head";
    if (hasChildren) {
      const open = this.logicalExpanded.has(node.id);
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "ov-fold";
      toggle.dataset["fold"] = node.id;
      toggle.textContent = open ? "▾" : "▸";
      toggle.setAttribute("aria-label", open ? COLLAPSE : EXPAND);
      toggle.setAttribute("aria-expanded", String(open));
      head.append(toggle);
    } else {
      const spacer = document.createElement("span");
      spacer.className = "ov-fold ov-fold-leaf";
      head.append(spacer);
    }
    const label = document.createElement("button");
    label.type = "button";
    label.className = "ov-lrow-name";
    label.textContent = node.label;
    label.dataset["pick"] = node.id;
    const size = document.createElement("span");
    size.className = "ov-lrow-size";
    size.textContent = logicalLength(node);
    head.append(label, size);
    row.append(head);
    const about = [node.value, node.type].filter((part) => part !== "").join(" · ");
    if (about !== "" || node.sourceText !== "") {
      const line = document.createElement("div");
      line.className = "ov-lrow-about";
      const what = document.createElement("span");
      what.className = "ov-lrow-what";
      what.textContent = about;
      const where = document.createElement("span");
      where.className = "ov-lrow-where addr";
      where.textContent = node.sourceText;
      line.append(what, where);
      row.append(line);
    }
    return row;
  }

  private logicalMoreRow(sectionId: string, count: number, label: string): HTMLElement {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ov-more";
    button.dataset["logicalMore"] = sectionId;
    button.textContent = LOGICAL_MORE(count, label);
    return button;
  }

  private onLogicalClick(e: MouseEvent): void {
    const t = e.target;
    if (!(t instanceof HTMLElement)) return;
    const more = t.closest<HTMLElement>("[data-logical-more]");
    if (more !== null) {
      const id = more.dataset["logicalMore"] ?? "";
      this.logicalShown.set(id, (this.logicalShown.get(id) ?? LOGICAL_PAGE) + LOGICAL_PAGE);
      this.scheduleLogical();
      return;
    }
    const fold = t.closest<HTMLElement>("[data-fold]");
    if (fold !== null) {
      const id = fold.dataset["fold"] ?? "";
      if (this.logicalExpanded.has(id)) this.logicalExpanded.delete(id);
      else this.logicalExpanded.add(id);
      this.scheduleLogical();
      return;
    }
    const row = t.closest<HTMLElement>(".ov-lrow");
    if (row === null) return;
    const path = (row.dataset["path"] ?? "") === "" ? [] : (row.dataset["path"] ?? "").split("/").map(Number);
    if (e.ctrlKey || e.metaKey) {
      const to = this.pointsAt(path);
      if (to !== null) {
        this.onGoTo(to);
        return;
      }
    }
    const start = Number(row.dataset["start"]);
    this.selectedLogicalId = row.dataset["logicalId"] ?? null;
    this.selectedPath = row.dataset["path"] ?? null;
    for (const other of this.logicalEl.querySelectorAll(".ov-lrow.is-selected")) other.classList.remove("is-selected");
    row.classList.add("is-selected");
    if (!Number.isFinite(start)) return;
    this.onPick({ path, startBit: start, endBit: start + 8 });
  }

  /** The bit this field's value points at, for a field holding an offset. */
  private pointsAt(path: readonly number[]): number | null {
    const r = this.doc.origins(path);
    if (r.status !== "ok") return null;
    const to = r.node.find((o) => o.role === "points" && o.target_bits !== null);
    return to?.target_bits ?? null;
  }

  // ----- lighting the map -----

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
      this.focusEl.replaceChildren(this.noneLine(PICK_BLOCK));
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
    if (this.doc.template === null) {
      this.focusGaps.replaceChildren();
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

  private noteLine(text: string): HTMLElement {
    const p = document.createElement("p");
    p.className = "ov-note";
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

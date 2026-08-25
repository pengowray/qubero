// The file described before it is read: how big it is, what kind of bytes it
// is made of and where they sit, and the major regions a template divides it
// into. Sits at the top of the listing, where "what is in this file" begins.
//
// The picture is a map of equal cells, one per bucket of the byte-class scan,
// colored by what the bucket's bytes are like. The same map answers for a file
// no template covers: a tail of zeros or a compressed middle shows up whether
// or not anything describes it.

import { formatBytes, formatOffset } from "./doc.js";
import type { Doc, OverviewState, TemplateNode } from "./doc.js";
import type { FieldPick } from "./listingview.js";

/** How the map is drawn: a square this wide plus a one-pixel gap. */
const CELL = 6;
const GAP = 1;

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

/** Fill colors for the map cells, light and dark, by class digit. The legend
 *  chips use the same values, so a color on the map can be looked up. */
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

const SCANNING = (percent: number): string => `Scanning the file… ${percent}%`;
const SMALL_FIELDS = (n: number): string => `${n} small fields`;

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

/** A share of the file: `48%`, or `<1%` rather than a zero that reads as
 *  nothing at all. */
function percentText(part: number, whole: number): string {
  if (whole === 0) return "0%";
  const p = (part / whole) * 100;
  return p >= 1 ? `${Math.round(p)}%` : "<1%";
}

/** The sentence one notable run earns. Position carries most of it: a run at
 *  the very end reads differently from one in the middle, and saying "the
 *  last half is zeros" is the whole point of the exercise. */
function noteText(run: Run, buckets: number, bucketBytes: number, fileBytes: number): string {
  const bytes = Math.min(run.len * bucketBytes, fileBytes - run.start * bucketBytes);
  const size = `${formatBytes(bytes)} (${percentText(bytes, fileBytes)})`;
  const what = CLASS_PROSE[run.cls] ?? "data";
  if (run.len === buckets) return `The whole file is ${what}.`;
  if (run.start === 0) return `The first ${size} is ${what}.`;
  if (run.start + run.len === buckets) return `The last ${size} is ${what}.`;
  return `${size} at ${formatOffset(run.start * bucketBytes * 8)} is ${what}.`;
}

export class OverviewPanel {
  readonly el: HTMLElement;
  private readonly body: HTMLElement;
  private readonly summary: HTMLElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly readout: HTMLElement;
  private readonly legend: HTMLElement;
  private readonly notes: HTMLElement;
  private readonly regionsEl: HTMLElement;

  private state: OverviewState | null = null;
  /** Another step is already queued, so a burst of notifies runs one. */
  private stepQueued = false;
  /** Buckets to draw brighter than the rest, while a region row is under the
   *  pointer. */
  private highlight: { from: number; to: number } | null = null;
  /** What the toolbar's identification said the file is, when it said. */
  private identity = "";

  /** A region row was chosen; same contract as picking a listing row. */
  onPick: (pick: FieldPick) => void = () => {};
  /** The map was clicked at this bit offset. */
  onJump: (bit: number) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "overview";

    const chevron = document.createElement("span");
    chevron.className = "panel-chevron";
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "ov-toggle";
    toggle.append(chevron, "Overview");
    const header = document.createElement("header");
    header.className = "ov-bar";
    header.append(toggle);

    this.summary = document.createElement("p");
    this.summary.className = "ov-summary";
    this.canvas = document.createElement("canvas");
    this.canvas.className = "ov-map";
    this.readout = document.createElement("p");
    this.readout.className = "ov-readout";
    this.legend = document.createElement("div");
    this.legend.className = "ov-legend";
    this.notes = document.createElement("ul");
    this.notes.className = "ov-notes";
    this.regionsEl = document.createElement("div");
    this.regionsEl.className = "ov-regions";
    this.body = document.createElement("div");
    this.body.className = "ov-body";
    this.body.append(this.summary, this.canvas, this.legend, this.readout, this.notes, this.regionsEl);
    this.el.append(header, this.body);

    const key = "qubero.overview";
    const apply = (collapsed: boolean): void => {
      this.el.classList.toggle("is-collapsed", collapsed);
      this.body.hidden = collapsed;
      chevron.textContent = collapsed ? "▸" : "▾";
      toggle.setAttribute("aria-expanded", String(!collapsed));
      toggle.title = collapsed ? "Expand" : "Collapse";
    };
    apply(localStorage.getItem(key) === "collapsed");
    toggle.addEventListener("click", () => {
      const collapsed = !this.el.classList.contains("is-collapsed");
      localStorage.setItem(key, collapsed ? "collapsed" : "open");
      apply(collapsed);
      this.pump();
    });

    this.canvas.addEventListener("pointermove", (e) => this.onMapHover(e));
    this.canvas.addEventListener("pointerleave", () => {
      this.readout.textContent = "";
    });
    this.canvas.addEventListener("click", (e) => {
      const b = this.bucketAt(e);
      if (b !== null && this.state !== null) this.onJump(b * this.state.bucket_bytes * 8);
    });

    doc.onChange(() => this.pump());
    this.pump();
  }

  /** The identification's sentence for the file, once there is one. */
  setIdentity(text: string): void {
    this.identity = text;
    this.render();
  }

  /**
   * Ask for one more step of the scan when there is anything left to do and
   * anyone to see it. A hidden or folded-away map must not pull the whole
   * file through the chunk cache for nobody.
   */
  pump(): void {
    if (this.el.offsetParent === null || this.body.hidden) return;
    if (this.stepQueued) return;
    // As many steps as a frame's worth of time allows, in one go: a chain of
    // one step per timeout would be throttled to a crawl the moment the tab
    // is in the background.
    const start = performance.now();
    let r = this.doc.overviewStep();
    while (r.status === "ok" && !r.node.done && performance.now() - start < 12) {
      r = this.doc.overviewStep();
    }
    if (r.status === "ok") {
      // An edit throws the scan away, so `done` can go back to false here.
      this.state = r.node;
      this.render();
      if (!r.node.done) {
        // Yield so the page draws the partial map and stays usable; the
        // chunk fetches themselves also come back through pump.
        this.stepQueued = true;
        setTimeout(() => {
          this.stepQueued = false;
          this.pump();
        }, 0);
      }
    }
    // Pending: the chunks are on their way, and their arrival calls pump.
  }

  // ----- drawing -----

  private render(): void {
    const s = this.state;
    if (s === null) return;
    this.drawSummary(s);
    this.drawMap(s);
    this.drawLegend(s);
    this.drawNotes(s);
    this.drawRegions();
  }

  private drawSummary(s: OverviewState): void {
    const len = this.doc.lengthBytes;
    const size = len < 1024 ? `${len.toLocaleString()} bytes` : `${formatBytes(len)} (${len.toLocaleString()} bytes)`;
    const parts = [size];
    if (this.identity !== "") parts.push(this.identity);
    parts.push(`1 cell = ${cellText(s.bucket_bytes)}`);
    this.summary.textContent = parts.join("  ·  ");
  }

  private colors(): string[] {
    return matchMedia("(prefers-color-scheme: dark)").matches ? DARK : LIGHT;
  }

  private drawMap(s: OverviewState): void {
    const width = this.body.clientWidth;
    if (width <= 0) return;
    const cols = Math.max(16, Math.floor(width / (CELL + GAP)));
    const rows = Math.max(1, Math.ceil(s.total_buckets / cols));
    const dpr = window.devicePixelRatio || 1;
    this.canvas.width = Math.round(cols * (CELL + GAP) * dpr);
    this.canvas.height = Math.round(rows * (CELL + GAP) * dpr);
    this.canvas.style.width = `${cols * (CELL + GAP)}px`;
    this.canvas.style.height = `${rows * (CELL + GAP)}px`;
    this.canvas.dataset["cols"] = String(cols);
    const ctx = this.canvas.getContext("2d");
    if (ctx === null) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cols * (CELL + GAP), rows * (CELL + GAP));
    const colors = this.colors();
    const h = this.highlight;
    for (let i = 0; i < s.classes.length; i++) {
      const cls = Number(s.classes[i]);
      ctx.globalAlpha = h === null || (i >= h.from && i < h.to) ? 1 : 0.25;
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
      chip.append(swatch, `${CLASS_LABEL[cls]} ${percentText(n, s.classes.length)}`);
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
    if (this.doc.template === null) {
      this.regionsEl.replaceChildren();
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
      size.textContent = formatBytes(Math.ceil(region.sizeBits / 8));
      const share = document.createElement("span");
      share.className = "ov-region-share";
      share.textContent = percentText(region.sizeBits, fileBits);
      const bar = document.createElement("span");
      bar.className = "ov-region-bar";
      const fill = document.createElement("i");
      fill.style.width = `${Math.max(1, (region.sizeBits / fileBits) * 100)}%`;
      bar.append(fill);
      row.append(name, size, share, bar);
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
    this.regionsEl.replaceChildren(...rows);
  }

  /** Dim the rest of the map, so the row under the pointer shows where its
   *  bytes sit. Passing null puts the map back. */
  private highlightRange(offsetBits: number | null, sizeBits: number): void {
    const s = this.state;
    if (s === null) return;
    if (offsetBits === null) this.highlight = null;
    else {
      const bucketBits = s.bucket_bytes * 8;
      this.highlight = {
        from: Math.floor(offsetBits / bucketBits),
        to: Math.max(Math.floor(offsetBits / bucketBits) + 1, Math.ceil((offsetBits + sizeBits) / bucketBits)),
      };
    }
    this.drawMap(s);
  }

  // ----- the map under the pointer -----

  private bucketAt(e: MouseEvent): number | null {
    const s = this.state;
    if (s === null) return null;
    const cols = Number(this.canvas.dataset["cols"] ?? 0);
    if (cols === 0) return null;
    const box = this.canvas.getBoundingClientRect();
    const col = Math.floor((e.clientX - box.left) / (CELL + GAP));
    const row = Math.floor((e.clientY - box.top) / (CELL + GAP));
    if (col < 0 || col >= cols || row < 0) return null;
    const i = row * cols + col;
    return i < s.classes.length ? i : null;
  }

  private onMapHover(e: PointerEvent): void {
    const s = this.state;
    const i = this.bucketAt(e);
    if (s === null || i === null) {
      this.readout.textContent = "";
      return;
    }
    const cls = Number(s.classes[i]);
    this.readout.textContent = `${formatOffset(i * s.bucket_bytes * 8)}  ·  ${CLASS_LABEL[cls] ?? ""}`;
  }
}

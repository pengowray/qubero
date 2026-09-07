/**
 * The treemap with its controls: the picker that says what the boxes stand
 * for, the trail that says which part of the file is being looked at, and the
 * line under it that says what the picture cannot.
 *
 * One component, mounted twice. In the rail it is 256px wide and shows the
 * shape of the file at a glance; as a view of its own it gets the whole
 * workspace and can be read. Nothing here knows which it is except how much
 * room it was given, which is the only difference that should matter: a
 * treemap that behaved differently in the two places would be two treemaps.
 */

import type { Doc, FieldPick } from "./doc.js";
import type { OutlineHeading } from "./outline.js";
import { TREEMAP } from "./strings.js";
import { boxAt, drawTreemap, nodeAt } from "./treemap.js";
import { bitsLine, bitsTree, boxTitle, bytesTree, kindsTree, poolNoun, POOL_UNDER, structureTree, TREEMAP_MODES, type TreemapMode, type TreemapTree } from "./treemapdata.js";

/** The whole-file scan's resolution. The same number the rail's byte-class map
 *  asks for, so both are answered by one scan: the core keeps one per sheet
 *  and carries it on only while the range and the resolution match. */
const SCAN_BUCKETS = 1024;
/** How long one turn of the scan may run before the page gets a frame. The
 *  rail's map uses the same budget for the same scan. */
const SCAN_MS = 10;

/** How tall the map is drawn in the rail, where it shares the column with
 *  everything else. As a view it takes what it is given. */
const RAIL_HEIGHT = 168;

export class TreemapPanel {
  readonly el: HTMLElement;
  private readonly pick: HTMLSelectElement;
  private readonly trail: HTMLElement;
  private readonly plot: HTMLElement;
  private readonly line: HTMLElement;
  private readonly note: HTMLElement;
  private mode: TreemapMode = "structure";
  /** Which node the map is rooted at, in structure mode. Null is the file. */
  private root: readonly number[] | null = null;
  /** The trail back out, kept as the nodes themselves so a crumb can name what
   *  it leads to without asking the template again. */
  private crumbs: { readonly name: string; readonly path: readonly number[] | null }[] = [];
  private tree: TreemapTree | null = null;
  private pumping = false;
  private frame = 0;

  onPick: (pick: FieldPick) => void = () => {};
  onJump: (startBit: number, endBit: number) => void = () => {};

  constructor(
    private readonly doc: Doc,
    private readonly compact: boolean,
  ) {
    this.el = document.createElement("div");
    this.el.className = compact ? "tmp tmp-rail" : "tmp tmp-view";

    const bar = document.createElement("div");
    bar.className = "tmp-bar";
    this.pick = document.createElement("select");
    this.pick.className = compact ? "insp-reading-pick" : "tb-mode";
    this.pick.setAttribute("aria-label", TREEMAP.groupBy);
    this.pick.title = TREEMAP.groupBy;
    for (const mode of TREEMAP_MODES) {
      const option = document.createElement("option");
      option.value = mode;
      option.textContent = TREEMAP.modes[mode];
      this.pick.append(option);
    }
    this.pick.addEventListener("change", () => {
      this.mode = (TREEMAP_MODES as readonly string[]).includes(this.pick.value) ? (this.pick.value as TreemapMode) : "structure";
      // Which node the map is rooted at is a fact about the structure, and the
      // other three modes have no structure to be rooted in.
      this.root = null;
      this.crumbs = [];
      rememberMode(this.mode);
      this.draw();
      // Each mode reads the file its own way, so switching starts whichever
      // reading the new one needs rather than waiting for the next nudge.
      this.pump();
    });
    // The rail's own heading row: the name on the left and the picker on the
    // right, the way the block section shares its row with its close button.
    // A view has room to write the question the options answer instead.
    if (compact) {
      const head = document.createElement("h3");
      head.textContent = TREEMAP.title;
      bar.append(head, this.pick);
    } else {
      const label = document.createElement("label");
      label.className = "tmp-label";
      label.textContent = TREEMAP.groupBy;
      label.append(this.pick);
      bar.append(label);
    }

    this.trail = document.createElement("div");
    this.trail.className = "tmp-trail insp-crumbs";
    this.trail.hidden = true;
    this.trail.addEventListener("click", (e) => {
      const t = e.target;
      if (!(t instanceof HTMLElement)) return;
      const at = t.dataset["crumb"];
      if (at === undefined) return;
      this.crumbs = this.crumbs.slice(0, Number(at) + 1);
      this.root = this.crumbs[this.crumbs.length - 1]?.path ?? null;
      this.draw();
    });

    this.plot = document.createElement("div");
    this.plot.className = "tmp-plot";
    this.plot.addEventListener("click", (e) => this.hit(e, false));
    this.plot.addEventListener("dblclick", (e) => this.hit(e, true));

    this.line = document.createElement("div");
    this.line.className = "tmp-line";
    this.note = document.createElement("div");
    this.note.className = "tmp-note";

    this.el.append(bar, this.trail, this.plot, this.line, this.note);
    // Backspace is the way back out, which is what it means everywhere else a
    // reader has gone into something.
    this.el.tabIndex = -1;
    this.el.addEventListener("keydown", (e) => {
      if (e.key !== "Backspace" || this.crumbs.length === 0) return;
      e.preventDefault();
      this.crumbs.pop();
      this.root = this.crumbs[this.crumbs.length - 1]?.path ?? null;
      this.draw();
    });

    const saved = localStorage.getItem(MODE_KEY);
    if (saved !== null && (TREEMAP_MODES as readonly string[]).includes(saved)) this.mode = saved as TreemapMode;
    this.pick.value = this.mode;
    doc.onChange(() => this.draw());
  }

  /** The listing has walked more of the template. Nothing here is built from
   *  the outline any more, but a new one means there is more of the file to
   *  draw than there was. */
  setOutline(_headings: readonly OutlineHeading[]): void {
    this.draw();
  }

  /** Called when the panel is shown or resized. Measuring is the caller's cue
   *  rather than the panel's own, because a panel that measured itself while
   *  hidden would draw a map one pixel wide and keep it. */
  relayout(): void {
    this.draw();
  }

  /**
   * Read more of the file, if this mode needs the whole of it read.
   *
   * The scan is the one the rail's byte-class map runs, at the same
   * resolution, so opening the treemap on a file the rail has already scanned
   * costs nothing and the two never disagree about how far it has got.
   */
  pump(): void {
    if (this.el.offsetParent === null || this.mode === "structure") return;
    if (this.pumping) return;
    this.pumping = true;
    const until = performance.now() + SCAN_MS;
    let more = false;
    do {
      // Both of these hand back what they have and say whether there is more,
      // so neither is driven by the document's own wake-up: the panel loops
      // them under a budget the way the byte map does.
      const step = this.mode === "kinds" ? this.doc.kindTotalsStep() : this.doc.overviewStep(SCAN_BUCKETS);
      if (step.status !== "ok") break;
      more = !step.node.done;
    } while (more && performance.now() < until);
    this.pumping = false;
    this.draw();
    if (more) setTimeout(() => this.pump(), 0);
  }

  /** Everything the map is made of, worked out afresh. Cheap enough to do on
   *  every change: the trees are tens of nodes, and the alternative is knowing
   *  which of a dozen inputs each mode depends on. */
  private draw(): void {
    if (this.frame !== 0) return;
    // A timeout rather than an animation frame: a hidden tab is given no
    // frames at all, and a panel whose redraw is waiting on one would sit at
    // whatever it last drew until the tab came back, including the empty box
    // it starts as.
    this.frame = window.setTimeout(() => {
      this.frame = 0;
      this.paint();
    }, 0);
  }

  private paint(): void {
    const width = Math.floor(this.plot.clientWidth || this.el.clientWidth);
    const height = this.compact ? RAIL_HEIGHT : Math.floor(this.plot.clientHeight);
    this.tree = this.build();
    const t = this.tree;
    this.trail.hidden = this.crumbs.length === 0;
    if (!this.trail.hidden) this.drawTrail();
    this.line.textContent = t.none ?? t.progress ?? this.underLine();
    this.line.hidden = this.line.textContent === "";
    this.note.textContent = this.crumbs.length === 0 ? TREEMAP.hint : `${TREEMAP.hint} ${TREEMAP.hintBack}`;
    this.note.hidden = t.none !== null;
    if (t.none !== null || width < 8 || height < 8) {
      this.plot.replaceChildren();
      return;
    }
    const parent = this.root === null ? null : nodeOf(this.doc, this.root);
    const noun = poolNoun(this.mode, parent);
    // What the shares on the boxes are shares of: the file once the reading is
    // done, and what has been read while it is not.
    const read = t.progress === null ? null : this.readSoFar(t.unit);
    const map = drawTreemap(t.root, {
      width,
      height,
      poolUnder: POOL_UNDER,
      title: (node, share) => boxTitle(node, share, t.unit, read),
      poolName: TREEMAP.pooled,
      poolDetail: (n) => TREEMAP.pooledTitle(n, noun, "", ""),
    });
    this.plot.replaceChildren(map.el);
  }

  /** The line under a map that has finished: what two boxes cannot say. */
  private underLine(): string {
    if (this.mode !== "bits") return "";
    const h = this.histogram();
    return h === null ? "" : bitsLine(h);
  }

  private build(): TreemapTree {
    if (this.mode === "structure") return structureTree(this.doc, this.root);
    // Field type is a walk of the template, not a read of the bytes, so it
    // must not be held up behind the byte scan or report the byte scan's
    // progress as its own.
    if (this.mode === "kinds") {
      const walk = this.doc.kindTotalsStep();
      if (walk.status === "error") return blank(TREEMAP.failed(walk.message));
      if (walk.status !== "ok") return blank(TREEMAP.reading(0));
      return kindsTree(walk.node, this.doc.lengthBits);
    }
    const scanned = this.scanned();
    const total = this.doc.lengthBytes;
    const h = this.histogram();
    // The scan is still on its way. The share it has reached is the honest
    // number to show, and it is the one that moves: a line fixed at 0% reads
    // as a scan that has stalled rather than one that has started.
    if (h === null) return blank(SCANNING(percent(scanned, total)));
    if (this.mode === "bytes") return bytesTree(h, scanned, total);
    return bitsTree(h, scanned, total);
  }

  /** The 256 byte counts the whole-file scan has accumulated, or null while it
   *  has not reported them. A core built before the counts were put on the
   *  wire reports none at all, which is the same answer as far as this is
   *  concerned: there is nothing yet to draw. */
  private histogram(): readonly number[] | null {
    const step = this.doc.overviewStep(SCAN_BUCKETS);
    if (step.status !== "ok") return null;
    const h = (step.node as { histogram?: readonly number[] }).histogram;
    return h === undefined || h.length !== 256 ? null : h;
  }

  /** How much of the file the current mode has taken in, in bytes. */
  private readSoFar(unit: "bits" | "count"): number {
    if (unit === "count") return this.scanned();
    const walk = this.doc.kindTotalsStep();
    return walk.status === "ok" ? Math.ceil(walk.node.reached_bits / 8) : 0;
  }

  private scanned(): number {
    const step = this.doc.overviewStep(SCAN_BUCKETS);
    return step.status === "ok" ? step.node.read_bytes : 0;
  }

  private drawTrail(): void {
    const items = this.crumbs.map((c, i) => {
      const b = document.createElement("button");
      b.type = "button";
      b.className = i === this.crumbs.length - 1 ? "insp-crumb insp-crumb-here" : "insp-crumb";
      b.dataset["crumb"] = String(i);
      b.textContent = c.name;
      return b;
    });
    this.trail.replaceChildren(...items);
  }

  /** A click goes to the bytes; a double click makes them the whole picture. */
  private hit(e: MouseEvent, into: boolean): void {
    const t = this.tree;
    if (t === null) return;
    const key = boxAt(e.target);
    if (key === null) return;
    const found = nodeAt(t.root, key);
    if (found === null) return;
    const node = found.node;
    if (into) {
      const path = node.path;
      // Only a box that is a node of the template can be opened: a group the
      // treemap invented has nothing under it to go to.
      if (this.mode !== "structure" || path === undefined) return;
      this.crumbs.push({ name: node.name, path });
      this.root = path;
      this.draw();
      return;
    }
    const range = node.range;
    if (range === undefined) return;
    this.onJump(range.offsetBits, range.offsetBits + range.sizeBits);
  }
}

/** A map with nothing in it yet, and the line saying why. */
function blank(progress: string): TreemapTree {
  return { root: { key: "file", name: TREEMAP.root, value: 0, color: "var(--line)" }, unit: "count", progress, none: null };
}

/** The whole-file scan's own line, so the treemap and the byte map above it
 *  never report one scan two different ways. */
const SCANNING = (share: number): string => `Scanning the file… ${share}%`;

function percent(part: number, whole: number): number {
  return whole === 0 ? 0 : Math.round((part / whole) * 100);
}

const MODE_KEY = "qubero.treemap.mode";

function rememberMode(mode: TreemapMode): void {
  try {
    localStorage.setItem(MODE_KEY, mode);
  } catch {
    // A browser that will not remember is a browser that opens on Structure.
  }
}

function nodeOf(doc: Doc, path: readonly number[]): import("./doc.js").TemplateNode | null {
  const r = doc.templateNode(path);
  return r.status === "ok" ? r.node : null;
}


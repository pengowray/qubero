/**
 * The treemap with its controls: the picker that says what the boxes stand
 * for, the trail that says which part of the file is being looked at, and the
 * line under it that says what the picture cannot.
 *
 * It lives in the rail, where it is a column wide and shows the shape of the
 * file at a glance, and it can be thrown over the whole window when a reader
 * wants to read it rather than glance at it. One instance either way: it was
 * two, one in the rail and one as a view of its own, and the two never agreed
 * about which mode was showing or how far into the file a reader had gone.
 *
 * The two mouse verbs are the ones the rail's byte map uses, and they mean the
 * same here: press a box to select it and go to its bytes, press it twice to
 * make it the whole picture. Going back is a crumb in the trail or Backspace,
 * and the trail is on screen the whole time a reader is inside something,
 * because the way out of a zoom has to be visible from inside it.
 */

import type { Doc, FieldPick } from "./doc.js";
import type { OutlineHeading } from "./outline.js";
import { formatBytes, formatOffset, percentText } from "./doc.js";
import { TREEMAP } from "./strings.js";
import { boxAt, drawTreemap, nodeAt, type TreeNode } from "./treemap.js";
import { bitsLine, bitsTree, boxTitle, bytesTree, classesTree, kindsTree, poolNoun, POOL_UNDER, structureTree, TREEMAP_MODES, type StructureAt, type TreemapMode, type TreemapTree } from "./treemapdata.js";

/** The whole-file scan's resolution. The same number the rail's byte-class map
 *  asks for, so both are answered by one scan: the core keeps one per sheet
 *  and carries it on only while the range and the resolution match. */
const SCAN_BUCKETS = 1024;
/** How long one turn of the scan may run before the page gets a frame. The
 *  rail's map uses the same budget for the same scan. */
const SCAN_MS = 10;

/** How tall the map is drawn in the rail, where it shares the column with
 *  everything else. Full screen it takes what it is given. */
const RAIL_HEIGHT = 168;

/** One step of the way in. `path` is the template node the step stands for,
 *  where the mode has one; `key` is the box's identity in the drawn tree,
 *  which every mode has. */
type Crumb = { readonly name: string; readonly key: string; readonly at: StructureAt };

export class TreemapPanel {
  readonly el: HTMLElement;
  private readonly pick: HTMLSelectElement;
  private readonly grow: HTMLButtonElement;
  private readonly trail: HTMLElement;
  private readonly plot: HTMLElement;
  private readonly where: HTMLElement;
  private readonly line: HTMLElement;
  private readonly note: HTMLElement;
  private mode: TreemapMode = "structure";
  /** The way in, outermost first. Empty is the whole file. */
  private crumbs: Crumb[] = [];
  private tree: TreemapTree | null = null;
  /** The box the reader last pressed, by its identity in the drawn tree, so a
   *  redraw puts the mark back where it was. */
  private selected: string | null = null;
  private big = false;
  private pumping = false;
  private frame = 0;

  onPick: (pick: FieldPick) => void = () => {};
  onJump: (startBit: number, endBit: number) => void = () => {};
  /** The panel was thrown over the window or put back, so the rest of the page
   *  can stop drawing what is now behind it. */
  onResize: () => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("div");
    this.el.className = "tmp";

    const bar = document.createElement("div");
    bar.className = "tmp-bar";
    const head = document.createElement("h3");
    head.textContent = TREEMAP.title;
    // The minimap's heading carries the question it answers, so this one does
    // too. Two pictures of one file where only one says what it is for is two
    // pictures a reader has to work out the difference between.
    head.title = TREEMAP.what;
    this.pick = document.createElement("select");
    this.pick.className = "insp-reading-pick";
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
      // Where a reader had got to is a fact about one way of dividing the
      // file, and the next way divides it somewhere else entirely.
      this.crumbs = [];
      this.selected = null;
      rememberMode(this.mode);
      this.draw();
      // Each mode reads the file its own way, so switching starts whichever
      // reading the new one needs rather than waiting for the next nudge.
      this.pump();
    });
    this.grow = document.createElement("button");
    this.grow.type = "button";
    this.grow.className = "tmp-grow";
    this.grow.addEventListener("click", () => this.setBig(!this.big));
    bar.append(head, this.pick, this.grow);

    this.trail = document.createElement("div");
    this.trail.className = "tmp-trail insp-crumbs";
    this.trail.hidden = true;
    this.trail.addEventListener("click", (e) => {
      const t = e.target;
      if (!(t instanceof HTMLElement)) return;
      const at = t.dataset["crumb"];
      if (at === undefined) return;
      this.crumbs = this.crumbs.slice(0, Number(at));
      this.selected = null;
      this.draw();
    });

    this.plot = document.createElement("div");
    this.plot.className = "tmp-plot";
    // The map is drawn at the size the box happens to be, so it has to be
    // drawn again when the box changes size. Throwing the panel over the
    // window is the loud case and a redraw is asked for there anyway, but it
    // lands a frame before the layout settles, and what got drawn was a map of
    // a box that was about to be ten times bigger.
    new ResizeObserver(() => this.draw()).observe(this.plot);
    this.plot.addEventListener("click", (e) => this.hit(e, false));
    this.plot.addEventListener("dblclick", (e) => this.hit(e, true));

    this.where = document.createElement("div");
    this.where.className = "tmp-where";
    this.where.hidden = true;
    this.line = document.createElement("div");
    this.line.className = "tmp-line";
    this.note = document.createElement("div");
    this.note.className = "tmp-note";

    this.el.append(bar, this.trail, this.where, this.plot, this.line, this.note);
    // Backspace is the way back out, which is what it means everywhere else a
    // reader has gone into something, and Escape puts a full-screen map back
    // in the rail.
    this.el.tabIndex = -1;
    this.el.addEventListener("keydown", (e) => this.key(e));
    // Over the window the panel is the page, so its keys have to work wherever
    // the focus went. Pressing a box moves focus to nothing, since the boxes
    // are divs, and Escape on a panel nobody is focused on would be a
    // full-screen map with no way out but the mouse.
    document.addEventListener("keydown", (e) => {
      if (!this.big || this.el.contains(document.activeElement)) return;
      this.key(e);
    });

    const saved = localStorage.getItem(MODE_KEY);
    if (saved !== null && (TREEMAP_MODES as readonly string[]).includes(saved)) this.mode = saved as TreemapMode;
    this.pick.value = this.mode;
    this.setBig(false);
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

  /** Escape puts a full-screen map back in the rail, and Backspace is the way
   *  back out of a box, which is what it means everywhere else a reader has
   *  gone into something. */
  private key(e: KeyboardEvent): void {
    if (e.key === "Escape" && this.big) {
      e.preventDefault();
      this.setBig(false);
      return;
    }
    if (e.key !== "Backspace") return;
    // Swallowed whether or not there is anywhere to go back to. A browser that
    // still reads Backspace as Back would otherwise leave the file, which is a
    // long way to fall for a key that means "up one level" here.
    e.preventDefault();
    if (this.crumbs.length === 0) return;
    this.crumbs.pop();
    this.selected = null;
    this.draw();
  }

  /** Whether the map is over the window rather than in the rail. */
  get maximised(): boolean {
    return this.big;
  }

  private setBig(big: boolean): void {
    this.big = big;
    this.el.classList.toggle("tmp-big", big);
    this.grow.textContent = big ? TREEMAP.shrinkIcon : TREEMAP.growIcon;
    this.grow.title = big ? TREEMAP.shrink : TREEMAP.grow;
    this.grow.setAttribute("aria-label", this.grow.title);
    if (big) this.el.focus();
    this.onResize();
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
    const height = this.big ? Math.floor(this.plot.clientHeight) : RAIL_HEIGHT;
    const t = this.zoomed(this.build(width * Math.max(height, 1)));
    this.tree = t;
    // The picker says what is on screen, which is not always what the reader
    // chose: without a template two of the modes cannot be drawn, so they are
    // greyed and the picker moves to the one that is. A picker reading
    // "Structure" over a map of byte classes is the panel telling the reader
    // something untrue about its own state.
    const showing = this.showing();
    const noTemplate = this.doc.template === null;
    for (const option of this.pick.options) {
      option.disabled = noTemplate && (option.value === "structure" || option.value === "kinds");
    }
    if (this.pick.value !== showing) this.pick.value = showing;
    this.trail.hidden = this.crumbs.length === 0;
    if (!this.trail.hidden) this.drawTrail();
    this.drawWhere(t.root);
    this.line.textContent = t.none ?? t.progress ?? this.underLine();
    this.line.hidden = this.line.textContent === "";
    this.note.textContent = this.crumbs.length === 0 ? TREEMAP.hint : `${TREEMAP.hint} ${TREEMAP.hintBack}`;
    this.note.hidden = t.none !== null;
    if (t.none !== null || width < 8 || height < 8) {
      this.plot.replaceChildren();
      return;
    }
    const rooted = this.rootedAt();
    const parent = rooted === null ? null : nodeOf(this.doc, rooted.path);
    const noun = poolNoun(this.mode, parent);
    // What the shares on the boxes are shares of. Null while the picture is
    // the file; once a reader has opened something it is that, named, because
    // a share of an opened box that said "of file" was a wrong number with
    // confident wording on it.
    const inside = this.crumbs.length === 0 ? null : t.root.name;
    const map = drawTreemap(t.root, {
      width,
      height,
      poolUnder: POOL_UNDER,
      title: (node, share) => boxTitle(node, share, t.unit, inside),
      poolName: TREEMAP.pooled,
      poolDetail: (n) => TREEMAP.pooledTitle(n, noun, "", ""),
      ...(t.ordered === true ? { ordered: true } : {}),
      selected: this.selected,
    });
    this.plot.replaceChildren(map.el);
  }

  /**
   * Where the picture is rooted, written out.
   *
   * A zoomed treemap fills its box whatever it is a picture of, so nothing in
   * the boxes themselves says whether a reader is looking at the file or at
   * half a per cent of it. The trail says which part; this says how much of
   * the file that part is and where it starts, which is what connects the
   * zoomed picture back to the one it came out of.
   */
  private drawWhere(root: TreeNode): void {
    if (this.crumbs.length === 0 || root.value <= 0) {
      this.where.hidden = true;
      return;
    }
    const whole = this.doc.lengthBits;
    const bits = this.tree?.unit === "bits" ? root.value : root.value * 8;
    const size = formatBytes(Math.ceil(bits / 8));
    // Counted in bytes or in bits, a box is still some share of the file, and
    // the share is the number that says whether a zoomed picture is most of
    // the file or a corner of it.
    const share = whole > 0 ? percentText(bits, whole) : null;
    const at = root.range === undefined ? null : formatOffset(root.range.offsetBits);
    this.where.textContent = TREEMAP.zoomedTo(root.name, size, share, at);
    this.where.hidden = false;
  }

  /** The line under a map that has finished: what two boxes cannot say. */
  private underLine(): string {
    if (this.mode !== "bits") return "";
    const h = this.histogram();
    return h === null ? "" : bitsLine(h);
  }

  /** Where in the template the reader has gone, for the modes that have a
   *  template to go into. */
  private rootedAt(): StructureAt {
    for (let i = this.crumbs.length - 1; i >= 0; i--) {
      const at = this.crumbs[i]?.at;
      if (at !== undefined && at !== null) return at;
    }
    return null;
  }

  /**
   * The mode actually drawn.
   *
   * Structure and Field type both need a template, and plenty of files worth
   * opening have none. Rather than spend the panel on a sentence saying so,
   * they fall through to the one division that always has an answer: what the
   * bytes are like. What the reader chose is kept, so a template arriving
   * later brings their mode back with it.
   */
  private showing(): TreemapMode {
    const needsTemplate = this.mode === "structure" || this.mode === "kinds";
    return this.doc.template === null && needsTemplate ? "classes" : this.mode;
  }

  private build(pixels: number): TreemapTree {
    const mode = this.showing();
    if (mode === "structure") return structureTree(this.doc, this.rootedAt(), pixels);
    // What the map above this one is coloured by. It comes from the same scan
    // and needs nothing else, so it answers as soon as the first buckets land.
    if (mode === "classes") {
      const step = this.doc.overviewStep(SCAN_BUCKETS);
      if (step.status === "error") return blank(TREEMAP.failed(step.message));
      if (step.status !== "ok") return blank(SCANNING(0));
      const s = step.node;
      return classesTree(s.classes, s.bucket_bytes, this.doc.lengthBytes, s.read_bytes);
    }
    // Field type is a walk of the template, not a read of the bytes, so it
    // must not be held up behind the byte scan or report the byte scan's
    // progress as its own.
    if (mode === "kinds") {
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
    if (mode === "bytes") return bytesTree(h, scanned, total);
    return bitsTree(h, scanned, total);
  }

  /**
   * The tree cut down to what the reader has opened.
   *
   * Structure re-reads from the template at the opened node, so its tree
   * arrives already rooted there and there is nothing to cut. The other three
   * are built whole and cut here, which is what lets a reader open a byte
   * group or a field kind and get its parts drawn large: the same two mouse
   * verbs in every mode, rather than one mode that opens and three that do
   * nothing when pressed twice.
   */
  private zoomed(tree: TreemapTree): TreemapTree {
    if (this.mode === "structure" || this.crumbs.length === 0) return tree;
    let at: TreeNode = tree.root;
    for (const crumb of this.crumbs) {
      const next = (at.children ?? []).find((k) => k.key === crumb.key);
      // The tree changed under the trail, which a scan filling in can do. The
      // honest answer is the deepest node that is still there.
      if (next === undefined) break;
      at = next;
    }
    return at === tree.root ? tree : { ...tree, root: at };
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

  private scanned(): number {
    const step = this.doc.overviewStep(SCAN_BUCKETS);
    return step.status === "ok" ? step.node.read_bytes : 0;
  }

  /** The way back out. The file is always the first crumb, so a reader who has
   *  gone in three levels can get all the way out in one press. */
  private drawTrail(): void {
    const names = [TREEMAP.root, ...this.crumbs.map((c) => c.name)];
    const items = names.map((name, i) => {
      const b = document.createElement("button");
      b.type = "button";
      b.className = i === names.length - 1 ? "insp-crumb insp-crumb-here" : "insp-crumb";
      b.dataset["crumb"] = String(i);
      b.textContent = name;
      return b;
    });
    this.trail.replaceChildren(...items);
  }

  /** A press goes to the bytes and marks the box; a second press makes it the
   *  whole picture. */
  private hit(e: MouseEvent, into: boolean): void {
    const t = this.tree;
    if (t === null) return;
    const key = boxAt(e.target);
    if (key === null) return;
    const found = nodeAt(t.root, key);
    if (found === null) return;
    const node = found.node;
    if (into) {
      // Only a box with something under it can be opened. A byte value has
      // nothing inside it, and a map that zoomed into one would be a map of
      // one box.
      const inside = (node.children?.length ?? 0) > 0 || node.openable === true;
      if (!inside) return;
      // The trail is the keys below the root the map is already drawn at, so
      // opening a box two levels down adds both levels.
      for (const step of found.trail.slice(1)) {
        const at: StructureAt = step.path === undefined ? null : { path: step.path, span: step.span ?? null };
        this.crumbs.push({ name: step.name, key: step.key, at });
      }
      this.selected = null;
      this.draw();
      return;
    }
    // Lit here rather than by a redraw. Going to the bytes moves the cursor,
    // and nothing a cursor move sets off comes back to this panel, so a box
    // that waited for a redraw to be marked was a box that never got marked.
    this.selected = key;
    for (const on of this.plot.querySelectorAll(".tm-box.is-on")) on.classList.remove("is-on");
    const box = this.plot.querySelector(`[data-key="${CSS.escape(key)}"]`);
    box?.classList.add("is-on");
    const range = node.range;
    if (range !== undefined) this.onJump(range.offsetBits, range.offsetBits + range.sizeBits);
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

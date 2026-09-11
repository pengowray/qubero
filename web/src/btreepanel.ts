/**
 * The B-trees tab: one of an HDF5 file's version 1 B-trees, drawn in the shape
 * the file gives it.
 *
 * None of the other views shows a tree as a tree. The Listing is a containment
 * outline in address order, the hex grid is a line, the treemap is areas, and
 * the graph view settles its positions by a force balance, which is the right
 * tool when the organising fact is contested and the wrong one for a shape the
 * file states outright: this many children, this deep, these keys in this
 * order. What a reader wants from a B-tree is its fanout, its depth, how full
 * its nodes are, which keys sit where, and where the nodes landed in the file.
 *
 * So there are two pictures, one above the other, with the same rows:
 *
 * - The tree, as bands: one band per row of the tree, root at the top, each
 *   node a box sitting under its parent's own span. A box's width is how many
 *   things are under it, which for the bottom row is the node's own count and
 *   above it is the total beneath. That is not the same fact as a node's own
 *   entry count, so the caption says which one the widths are.
 *
 *   Under the last row of boxes is one more band, dashed and undivided, for
 *   the links or chunks the tree indexes. Those are not nodes: a group's links
 *   sit inside the link tables above, a dataset's chunks sit somewhere else in
 *   the file entirely, and neither has a mark on the address picture. Drawing
 *   the band anyway is what makes the picture say where a chunk tree stops,
 *   which a caption used to have to say in prose, and what makes the number
 *   printed on a box readable: every box's number is how many things are in
 *   the band directly below it, which at the root is two or three boxes a
 *   reader can count.
 * - The same nodes by file address, zoomed to the tree's own span. Tree order
 *   runs left to right in the first picture and address order in the second,
 *   so whether a tidy tree is scattered through the file is a thing to see
 *   rather than a thing to work out.
 *
 * The two mouse verbs are the treemap's and the byte map's, and mean the same
 * here: press a box to go to its bytes, press it twice to open it in the
 * Listing. The panel's corner button throws it over the window, which is the
 * treemap's mechanism and the answer to a tree too deep to glance at in a
 * 256px rail.
 *
 * Which tree is shown is settled by the core from where the cursor is: the
 * tree the cursor is inside, else the tree named by the object header it is
 * inside, else the root group's. So picking a dataset in the Logical tab draws
 * that dataset's chunk tree, without this panel knowing anything about object
 * headers.
 */

import type { Doc, FieldPick, Tree, TreeNode } from "./doc.ts";
import { formatBytes, formatOffset } from "./doc.ts";
import { BTREES, TREEMAP } from "./strings.ts";

/** How many nodes one walk draws. A group of a million links has a quarter of
 *  a million link tables, and a picture of a quarter of a million boxes is not
 *  a picture; past this the walk says how many children it left out. */
const NODE_LIMIT = 4096;

/** How tall one band of the tree is, in the rail and over the window. */
const ROW_H = 16;
const ROW_H_BIG = 30;
/** The gap between bands. Small enough that a child still reads as sitting
 *  under its parent, wide enough that the two rows are two rows. */
const ROW_GAP = 3;
/** How tall one band of the address strip is. Shorter than a tree band: it
 *  carries marks rather than labelled boxes. */
const STRIP_H = 7;
const STRIP_H_BIG = 11;

/** A box narrower than this cannot be pressed, so neighbouring siblings are
 *  drawn as one until they reach it. */
const MIN_W = 3;

/** How long after the cursor lands somewhere with no node of the drawn tree
 *  under it before another tree is asked for. A cursor moving through a file
 *  crosses plenty of places that are in no tree at all. */
const REWALK_MS = 150;

/** One drawn box: the nodes it stands for and where it sits. A box stands for
 *  more than one node only when they are siblings too narrow to draw apart. */
type Box = {
  readonly key: string;
  readonly row: number;
  readonly nodes: readonly number[];
  readonly x: number;
  readonly w: number;
};

/** Where each node sits in the tree picture, before neighbours are merged. */
type Placed = { x: number; w: number; weight: number };

export class BTreePanel {
  readonly el: HTMLElement;
  private readonly grow: HTMLButtonElement;
  private readonly head: HTMLElement;
  private readonly job: HTMLElement;
  private readonly plot: HTMLElement;
  private readonly widths: HTMLElement;
  private readonly keyLine: HTMLElement;
  /** The words beside the drawn box in the key, which are the one part of it
   *  that depends on the tree: a version 1 node's number is entries and a
   *  version 2 node's is records, and every other line on the panel says
   *  whichever of the two it is. Held so that a repaint can set the text
   *  rather than build the row again. */
  private readonly keyLabel: HTMLElement;
  private readonly rows: HTMLElement;
  private readonly stripCap: HTMLElement;
  private readonly strip: HTMLElement;
  private readonly span: HTMLElement;
  private readonly readout: HTMLElement;
  private readonly note: HTMLElement;

  /** The tree last walked, and the path it was walked from. */
  private tree: Tree | null = null;
  /** Why there is nothing to draw, when there is nothing to draw. */
  private empty = "";
  /** What the heading says, worked out once a tree. Null until it has been
   *  asked for: naming the tree's owner means walking the file's object list,
   *  which is not work to repeat on every redraw. */
  private ownerName: string | null = null;
  /** Where the cursor was when this tree was asked for. */
  private from: readonly number[] = [];
  /** The box the reader pressed, by its identity in the drawn picture. */
  private selected: string | null = null;
  /** The node the cursor is in, as an index into the tree's nodes. */
  private lit = -1;
  private boxes: readonly Box[] = [];
  private big = false;
  private shown = false;
  private frame = 0;
  private rewalk = 0;
  /** The tree on screen is behind the document: it was skipped while the tab
   *  or the rail was hidden. */
  private stale = true;

  onPick: (pick: FieldPick) => void = () => {};
  onJump: (startBit: number, endBit: number) => void = () => {};
  /** The panel was thrown over the window or put back, so the rest of the page
   *  can stop drawing what is now behind it. */
  onResize: () => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("div");
    this.el.className = "btp";

    const bar = document.createElement("div");
    bar.className = "btp-bar";
    this.head = document.createElement("div");
    this.head.className = "btp-head";
    this.grow = document.createElement("button");
    this.grow.type = "button";
    this.grow.className = "tmp-grow";
    this.grow.addEventListener("click", () => this.setBig(!this.big));
    bar.append(this.head, this.grow);

    this.job = document.createElement("div");
    this.job.className = "btp-job";
    this.plot = document.createElement("div");
    this.plot.className = "btp-plot";
    this.plot.addEventListener("click", (e) => this.hit(e, false));
    this.plot.addEventListener("dblclick", (e) => this.hit(e, true));
    this.widths = document.createElement("div");
    this.widths.className = "btp-line";
    const key = keyRow();
    this.keyLine = key.row;
    this.keyLabel = key.label;
    this.rows = document.createElement("div");
    this.rows.className = "btp-rows";
    this.stripCap = document.createElement("div");
    this.stripCap.className = "btp-line";
    this.strip = document.createElement("div");
    this.strip.className = "btp-strip";
    this.strip.addEventListener("click", (e) => this.hit(e, false));
    this.span = document.createElement("div");
    this.span.className = "btp-span";
    this.readout = document.createElement("div");
    this.readout.className = "btp-readout";
    this.note = document.createElement("div");
    this.note.className = "btp-note";
    this.note.textContent = BTREES.hint;

    this.el.append(bar, this.job, this.plot, this.keyLine, this.widths, this.rows, this.stripCap, this.strip, this.span, this.readout, this.note);

    // The picture is drawn at the size the box happens to be, so it has to be
    // drawn again when the box changes size. Throwing the panel over the
    // window asks for a redraw anyway, but that lands a frame before the
    // layout settles, and what got drawn was a picture of a box that was about
    // to be four times wider.
    new ResizeObserver(() => this.draw()).observe(this.plot);

    // Over the window the panel is the page, so its keys have to work wherever
    // the focus went. The two keys are the treemap's and mean the same.
    this.el.tabIndex = -1;
    this.el.addEventListener("keydown", (e) => this.key(e));
    document.addEventListener("keydown", (e) => {
      if (!this.big || this.el.contains(document.activeElement)) return;
      this.key(e);
    });

    this.setBig(false);
    doc.onChange(() => {
      this.stale = true;
      this.draw();
    });
  }

  /** Whether the picture is over the window rather than in the rail. */
  get maximised(): boolean {
    return this.big;
  }

  /**
   * Whether anyone can see this.
   *
   * Not `offsetParent !== null` alone, which is how the rest of the rail asks.
   * A panel thrown over the window is positioned against the window itself and
   * so has no offset parent, and that test would answer "nobody is looking" for
   * exactly the state a reader asked for out loud. Being told which tab is up
   * covers the rest: the caller says so when the tab changes and when the rail
   * is folded away.
   */
  private get visible(): boolean {
    return this.shown && (this.big || this.el.offsetParent !== null);
  }

  /** The tab was shown or hidden. Nothing is walked while nobody can see it:
   *  a tree of a thousand nodes is a walk of a thousand nodes, and a hidden
   *  tab must not pay for one. */
  setShown(on: boolean): void {
    if (this.shown === on) return;
    this.shown = on;
    if (on) this.pump();
  }

  /** Called when the panel is shown or resized. Measuring is the caller's cue
   *  rather than the panel's own, because a panel that measured itself while
   *  hidden would draw a picture one pixel wide and keep it. */
  relayout(): void {
    this.draw();
  }

  /** Draw again, walking the tree first if the walk is owed. The walk itself
   *  happens in the redraw, which is a turn later: a document change is one of
   *  the things that asks for this, and a chunk landing during a scroll must
   *  not turn into a walk inside the handler that noticed it. */
  pump(): void {
    if (!this.visible) return;
    this.draw();
  }

  /**
   * The cursor moved to `path`. A node of the drawn tree under it is lit, and
   * nothing else happens: the common case is a reader moving about inside the
   * tree they are already looking at, and re-walking it for every step would
   * be a walk of a thousand nodes per keypress.
   *
   * A cursor that lands outside the drawn tree asks for another one, after a
   * pause: a cursor crossing a file passes through plenty of places that are
   * in no tree at all, and each of those is not worth a walk.
   */
  reveal(path: readonly number[]): void {
    this.from = path;
    // Nothing is looked up while nobody is looking. A cursor move is cheap and
    // frequent, and the panel catches up in one walk when it is next shown.
    if (!this.visible) {
      this.stale = true;
      return;
    }
    const found = this.nodeAt(path);
    if (found >= 0) {
      this.from = path;
      if (found !== this.lit) {
        this.lit = found;
        this.light();
      }
      return;
    }
    this.from = path;
    if (this.rewalk !== 0) clearTimeout(this.rewalk);
    this.rewalk = window.setTimeout(() => {
      this.rewalk = 0;
      if (!this.visible) {
        this.stale = true;
        return;
      }
      this.walk(this.from);
      this.draw();
    }, REWALK_MS);
  }

  /** Which node of the drawn tree holds the field at `path`, or -1. A node's
   *  own path is a prefix of every path inside it, and the deepest such node
   *  is the one the cursor is actually in.
   *
   *  A node with no path of its own is passed over rather than matched. Every
   *  version 2 node below the root has one, because the template places only
   *  the root, and an empty path is a prefix of every path there is: matched,
   *  it would light whichever of those boxes came first wherever the cursor
   *  stood. */
  private nodeAt(path: readonly number[]): number {
    const tree = this.tree;
    if (tree === null) return -1;
    let best = -1;
    let deepest = -1;
    for (let i = 0; i < tree.nodes.length; i++) {
      const p = tree.nodes[i]?.path;
      if (p === undefined || p.length === 0 || p.length > path.length) continue;
      if (!p.every((step, k) => step === path[k])) continue;
      if (p.length > deepest) {
        deepest = p.length;
        best = i;
      }
    }
    return best;
  }

  private walk(path: readonly number[]): void {
    const reply = this.doc.btree(path, NODE_LIMIT);
    if (reply.status === "error") {
      this.tree = null;
      this.empty = BTREES.failed(reply.message);
      this.stale = false;
      return;
    }
    // The bytes are on their way, or the walk ran out of its turn. Either way
    // it is asked again when they land, and what is already drawn stays up.
    // Only a panel with nothing up at all says so, because a line reading
    // "Reading the B-tree…" over a drawn tree reads as the wrong tree.
    if (reply.status !== "ok") {
      if (this.tree === null) this.empty = BTREES.reading;
      return;
    }
    this.stale = false;
    if (reply.node === null || reply.node.nodes.length === 0) {
      this.tree = null;
      this.empty = BTREES.none;
      return;
    }
    this.tree = reply.node;
    this.empty = "";
    this.ownerName = null;
    this.lit = this.nodeAt(path);
    this.selected = null;
  }

  private setBig(big: boolean): void {
    this.big = big;
    this.el.classList.toggle("btp-big", big);
    this.grow.textContent = big ? TREEMAP.shrinkIcon : TREEMAP.growIcon;
    this.grow.title = big ? TREEMAP.shrink : TREEMAP.grow;
    this.grow.setAttribute("aria-label", this.grow.title);
    if (big) this.el.focus();
    this.onResize();
    this.draw();
  }

  /** Escape puts a full-window picture back in the rail and takes the mark off
   *  a box, in that order, which is what Escape means everywhere else here. */
  private key(e: KeyboardEvent): void {
    if (e.key !== "Escape") return;
    if (this.selected !== null) {
      e.preventDefault();
      this.selected = null;
      this.light();
      this.drawReadout();
      return;
    }
    if (!this.big) return;
    e.preventDefault();
    this.setBig(false);
  }

  private draw(): void {
    if (this.frame !== 0) return;
    // A timeout rather than an animation frame: a hidden tab is given no
    // frames at all, and a panel whose redraw is waiting on one would sit at
    // whatever it last drew until the tab came back.
    this.frame = window.setTimeout(() => {
      this.frame = 0;
      this.paint();
    }, 0);
  }

  private paint(): void {
    if (!this.visible) return;
    if (this.stale) this.walk(this.from);
    const tree = this.tree;
    // Nothing to throw over the window when there is no picture. A corner
    // button over a sentence is a control that does nothing, which a reader
    // has to press to find out.
    this.grow.hidden = tree === null;
    if (tree === null) {
      this.head.textContent = "";
      this.job.textContent = this.empty;
      this.plot.replaceChildren();
      this.keyLine.hidden = true;
      this.widths.textContent = "";
      this.rows.replaceChildren();
      this.stripCap.textContent = "";
      this.strip.replaceChildren();
      this.span.textContent = "";
      this.readout.textContent = "";
      this.note.hidden = true;
      return;
    }
    this.note.hidden = false;
    const width = Math.max(1, Math.floor(this.plot.clientWidth || this.el.clientWidth));
    this.head.textContent = this.ownerName ?? this.owner(tree);
    this.job.textContent = jobCaption(tree);
    this.keyLine.hidden = false;
    this.keyLabel.textContent = tree.version === 2 ? BTREES.recordsKey : BTREES.entriesKey;
    this.widths.textContent = widthCaption(tree);
    this.stripCap.textContent = BTREES.stripCaption;
    const placed = place(tree, width);
    this.boxes = boxesOf(tree, placed, width);
    this.plot.replaceChildren(this.drawTree(tree, width));
    this.drawRows(tree);
    this.drawStrip(tree, width);
    this.drawReadout();
    this.light();
  }

  /** The object whose tree this is, by its own path in the file.
   *
   * Read from the contents list rather than from the walk: the tree hangs
   * under the object header's messages, so the object is the one whose
   * template path is the longest prefix of the tree's. The list is capped, so
   * a file of thousands of objects can draw a tree whose owner is past the
   * cap; the heading says so rather than going blank. */
  private owner(tree: Tree): string {
    const root = tree.nodes[0];
    if (root === undefined) return BTREES.unnamed("");
    const at = formatOffset(root.address * 8);
    const reply = this.doc.contents();
    // Not kept while the list is still being read: the bytes it is waiting on
    // arrive and paint again, and an answer cached from a half-read list would
    // outlive the reason it was wrong.
    if (reply.status !== "ok") return BTREES.unnamed(at);
    let name = "";
    let deepest = -1;
    for (const object of reply.node.objects) {
      if (object.path.length > root.path.length || object.path.length <= deepest) continue;
      if (!object.path.every((step, k) => step === root.path[k])) continue;
      deepest = object.path.length;
      name = object.name;
    }
    this.ownerName = name === "" ? BTREES.unnamed(at) : name;
    return this.ownerName;
  }

  /** The tree itself: one band per row, each box under its parent's span, and
   *  under the last of them the band of links or chunks the tree indexes. */
  private drawTree(tree: Tree, width: number): SVGElement {
    const rowH = this.big ? ROW_H_BIG : ROW_H;
    const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
    const leaves = leafBand(tree);
    const bands = depth + (leaves === null ? 0 : 1);
    const height = bands * rowH + (bands - 1) * ROW_GAP;
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("class", "btp-svg");
    svg.setAttribute("width", String(width));
    svg.setAttribute("height", String(height));
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    for (const box of this.boxes) {
      const rect = document.createElementNS(SVG, "rect");
      rect.setAttribute("x", String(box.x));
      rect.setAttribute("y", String(box.row * (rowH + ROW_GAP)));
      rect.setAttribute("width", String(Math.max(1, box.w - 1)));
      rect.setAttribute("height", String(rowH));
      rect.setAttribute("class", "btp-box");
      rect.dataset["key"] = box.key;
      const title = document.createElementNS(SVG, "title");
      title.textContent = this.boxTitle(tree, box);
      rect.append(title);
      svg.append(rect);
      // The count on the box, where the box is wide enough to hold it. A box
      // too narrow for its own number is not given a clipped one: the readout
      // under the picture says it for whichever box is pressed.
      const one = box.nodes.length === 1 ? tree.nodes[box.nodes[0] ?? -1] : undefined;
      if (one !== undefined && box.w > 26 && rowH >= ROW_H) {
        const text = document.createElementNS(SVG, "text");
        text.setAttribute("x", String(box.x + box.w / 2));
        text.setAttribute("y", String(box.row * (rowH + ROW_GAP) + rowH / 2));
        text.setAttribute("class", "btp-count");
        text.textContent = one.entries.toLocaleString();
        svg.append(text);
      }
    }
    // The links or chunks themselves, as one dashed band across the full
    // width. Undivided because they are not nodes and have no place of their
    // own here: a group's links are inside the link tables in the row above,
    // and a dataset's chunks are elsewhere in the file. Drawn all the same,
    // because a picture that stops at the last row of boxes leaves a chunk
    // tree looking like a group tree with its bottom row missing, and leaves
    // the number printed on a box with nothing to count against.
    if (leaves !== null) {
      const y = depth * (rowH + ROW_GAP);
      const band = document.createElementNS(SVG, "rect");
      band.setAttribute("x", "0.5");
      band.setAttribute("y", String(y + 0.5));
      band.setAttribute("width", String(Math.max(1, width - 2)));
      band.setAttribute("height", String(rowH - 1));
      band.setAttribute("class", "btp-leaf");
      // No `data-key`, so a press on it falls out of `hit` before it looks for
      // a node. There is nowhere to go: the walk never gave these an address.
      const title = document.createElementNS(SVG, "title");
      title.textContent = leaves.title;
      band.append(title);
      svg.append(band);
      const text = document.createElementNS(SVG, "text");
      text.setAttribute("x", String(width / 2));
      text.setAttribute("y", String(y + rowH / 2));
      text.setAttribute("class", "btp-leafcount");
      text.textContent = leaves.label;
      svg.append(text);
    }
    return svg;
  }

  /** One box's tooltip: what it is, how much it holds, and where it is. */
  private boxTitle(tree: Tree, box: Box): string {
    if (box.nodes.length > 1) {
      const first = tree.nodes[box.nodes[0] ?? -1];
      const noun = first === undefined ? BTREES.kindIndex : kindWord(first);
      return BTREES.pooled(box.nodes.length, noun);
    }
    const node = tree.nodes[box.nodes[0] ?? -1];
    if (node === undefined) return "";
    return nodeLines(tree, node).join("\n");
  }

  /** The summary under the picture: one line a row, so how wide the tree is at
   *  each level is a number as well as a picture. */
  private drawRows(tree: Tree): void {
    const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
    const out: HTMLElement[] = [];
    for (let d = 0; d < depth; d++) {
      const row = tree.nodes.filter((n) => n.depth === d);
      const first = row[0];
      if (first === undefined) continue;
      const held = row.reduce((sum, n) => sum + n.entries, 0);
      const line = document.createElement("div");
      line.className = "btp-row";
      // A version 2 node holds records, both kinds of it, and writes no level
      // of its own. So neither of the version 1 rows fits and neither does
      // `levelTitle`, which names a field those nodes do not have.
      if (first.kind === "leaf") line.textContent = BTREES.rowLeaves(row.length, held);
      else if (tree.version === 2) line.textContent = BTREES.rowInternal(row.length, first.level);
      else if (first.kind === "links") line.textContent = BTREES.rowLinks(row.length, held);
      else if (tree.job === "chunk" && first.level === 0) line.textContent = BTREES.rowChunks(row.length, held);
      else {
        line.textContent = BTREES.rowIndex(row.length, first.level);
        // HDF5 counts levels up from the leaves, the opposite of the way the
        // bands are stacked. Said once, where a reader who wondered will look,
        // rather than on every row.
        line.title = BTREES.levelTitle;
      }
      out.push(line);
    }
    if (tree.omitted > 0) {
      const line = document.createElement("div");
      line.className = "btp-row btp-row-short";
      line.textContent = BTREES.omitted(tree.omitted);
      out.push(line);
    }
    // What the drawing does not cover. A version 2 tree is typed and this
    // reads two of the twelve types; the shape of one comes out of its header
    // and its child pointers rather than out of its records, so the picture
    // stands for a type whose records were not read and says so here. A tree
    // drawn as though its records were understood when they were not is the
    // one thing this must never do.
    for (const text of caveats(tree)) {
      const line = document.createElement("div");
      line.className = "btp-row btp-row-short";
      line.textContent = text;
      out.push(line);
    }
    this.rows.replaceChildren(...out);
  }

  /**
   * The same nodes by address, in the same rows.
   *
   * Zoomed to the tree's own span rather than to the file: a tree confined to
   * one small region of a file of gigabytes would otherwise put every mark on
   * the same pixel. Marks closer together than a pixel are pooled, which is
   * the treemap's answer to the same problem and keeps the count honest.
   */
  private drawStrip(tree: Tree, width: number): void {
    const from = Math.min(...tree.nodes.map((n) => n.address));
    const to = Math.max(...tree.nodes.map((n) => n.address + Math.ceil(n.size_bits / 8)));
    const span = Math.max(1, to - from);
    const rowH = this.big ? STRIP_H_BIG : STRIP_H;
    const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
    const height = depth * rowH + (depth - 1) * 2;
    const svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("class", "btp-svg");
    svg.setAttribute("width", String(width));
    svg.setAttribute("height", String(height));
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    for (let d = 0; d < depth; d++) {
      // One bucket a pixel: two nodes that would land on the same column are
      // one mark saying how many, rather than one mark drawn twice.
      const buckets = new Map<number, number[]>();
      for (let i = 0; i < tree.nodes.length; i++) {
        const node = tree.nodes[i];
        if (node === undefined || node.depth !== d) continue;
        const at = Math.min(width - 1, Math.floor(((node.address - from) / span) * width));
        const pool = buckets.get(at);
        if (pool === undefined) buckets.set(at, [i]);
        else pool.push(i);
      }
      for (const [at, pool] of buckets) {
        const mark = document.createElementNS(SVG, "rect");
        mark.setAttribute("x", String(at));
        mark.setAttribute("y", String(d * (rowH + 2)));
        mark.setAttribute("width", "1");
        mark.setAttribute("height", String(rowH));
        mark.setAttribute("class", "btp-mark");
        const key = keyOf(pool);
        mark.dataset["key"] = key;
        const title = document.createElementNS(SVG, "title");
        const one = pool.length === 1 ? tree.nodes[pool[0] ?? -1] : undefined;
        title.textContent =
          one === undefined
            ? BTREES.stripPooled(pool.length)
            : BTREES.selectedAt(kindWord(one), signWord(one), formatOffset(one.address * 8));
        mark.append(title);
        svg.append(mark);
      }
    }
    this.strip.replaceChildren(svg);
    this.span.textContent = BTREES.stripSpan(formatOffset(from * 8), formatOffset(to * 8));
  }

  /** What the pressed box is, written out under the picture. */
  private drawReadout(): void {
    const tree = this.tree;
    const box = this.boxes.find((b) => b.key === this.selected);
    if (tree === null || box === undefined) {
      this.readout.replaceChildren();
      this.readout.removeAttribute("title");
      return;
    }
    let lines: string[] = [];
    if (box.nodes.length > 1) {
      const first = tree.nodes[box.nodes[0] ?? -1];
      lines = [BTREES.pooled(box.nodes.length, first === undefined ? BTREES.kindIndex : kindWord(first))];
    } else {
      const node = tree.nodes[box.nodes[0] ?? -1];
      if (node === undefined) return;
      lines = nodeLines(tree, node);
      // How many bytes the node takes, which only the readout has room for.
      // On the line with the address, not the line with the count: "points at
      // 28 link tables · 480 B" reads as 480 bytes of link tables at least as
      // readily as 480 bytes of this node, and the address line has nothing
      // else on it a size could belong to.
      const first = lines[0];
      if (first !== undefined) lines[0] = `${first} · ${formatBytes(Math.ceil(node.size_bits / 8))}`;
      // What the numbers in a chunk key are. The visible line says "element
      // offset"; this says how many of the numbers are dimensions, so that
      // nobody counts them and gets one too many.
      // A version 1 key ends with an offset within an element that is always
      // zero and a version 2 record does not, so the two notes count the
      // numbers differently. `coords_pad` is which, from the walk, because
      // reading it off the version would be a rule this file made up.
      if (tree.job === "chunk" && node.first_key !== "" && tree.coords > 0) {
        lines.push(tree.coords_pad ? BTREES.chunkRangeNote(tree.coords) : BTREES.chunkRangeNoteV2(tree.coords));
      }
      // A node the template never placed has nowhere in the Listing to open,
      // and the reader has just read a hint that says double-click opens one.
      if (node.path.length === 0) lines.push(BTREES.notInListing);
    }
    this.readout.replaceChildren(
      ...lines.map((text) => {
        const line = document.createElement("div");
        line.textContent = text;
        return line;
      }),
    );
  }

  /** A press goes to the bytes and marks the box; a second press opens it in
   *  the Listing. The same two verbs as the treemap and the byte map. */
  private hit(e: MouseEvent, into: boolean): void {
    const tree = this.tree;
    const target = e.target;
    if (tree === null || !(target instanceof Element)) return;
    const key = (target as SVGElement & { dataset: DOMStringMap }).dataset["key"];
    if (key === undefined) return;
    const nodes = key.split(",").map(Number);
    const node = tree.nodes[nodes[0] ?? -1];
    if (node === undefined) return;
    if (into) {
      // The two presses that make a double-click have already marked the box
      // and taken the mark off again, which leaves the reader looking at the
      // node in the Listing and at nothing in the readout. Put it back: the
      // box they opened is the box they are on.
      this.selected = key;
      this.light();
      this.drawReadout();
      // A node with no path is not in the Listing to be opened: the template
      // places only the root of a version 2 tree, and the rest are read from
      // their bytes. So the second press does nothing beyond putting the mark
      // back. Not the jump again: the first of the two presses already made
      // it, and a second would record a step from where the reader is to where
      // they already are. An empty path handed on would send the Listing to
      // the root of the file, which is worse than nothing.
      if (node.path.length === 0) return;
      this.onPick({ path: node.path, startBit: node.address * 8, endBit: node.address * 8 + node.size_bits });
      return;
    }
    // Pressing the box that is already marked takes the mark off. Something a
    // reader turned on has to be something they can turn off.
    this.selected = this.selected === key ? null : key;
    this.light();
    this.drawReadout();
    if (this.selected === null) return;
    this.onJump(node.address * 8, node.address * 8 + node.size_bits);
  }

  /**
   * Mark the pressed box and the node the cursor is in.
   *
   * Done to the elements rather than by redrawing. Going to the bytes moves
   * the cursor, and what a cursor move sets off comes back here as `reveal`,
   * so a box that waited for a redraw would be marked a frame late and a
   * redraw would be run for a mark.
   */
  private light(): void {
    for (const on of this.el.querySelectorAll(".is-on, .is-here")) on.classList.remove("is-on", "is-here");
    if (this.selected !== null) {
      for (const el of this.el.querySelectorAll(`[data-key="${CSS.escape(this.selected)}"]`)) el.classList.add("is-on");
    }
    if (this.lit < 0) return;
    for (const box of this.boxes) {
      if (!box.nodes.includes(this.lit)) continue;
      for (const el of this.el.querySelectorAll(`[data-key="${CSS.escape(box.key)}"]`)) el.classList.add("is-here");
    }
  }
}

const SVG = "http://www.w3.org/2000/svg";

/** What kind of node this is, in words, and the signature written at it.
 *
 *  Three words for three structures. "Internal node" rather than "index node"
 *  for a `BTIN`, because that is the specification's own word for one and the
 *  summary row uses it; "index node" was coined here for a `TREE`, whose
 *  contents needed explaining. */
function kindWord(node: TreeNode): string {
  if (node.kind === "links") return BTREES.kindLinks;
  if (node.kind === "leaf") return BTREES.kindLeaf;
  return node.sign === "BTIN" ? BTREES.kindInternal : BTREES.kindIndex;
}

/** The four bytes written at the node's address.
 *
 *  The core sends them, because it is the core that checked for them: it reads
 *  a version 2 node by its signature and refuses the node when the signature
 *  is not the one the level called for. Worked out here instead, from the kind
 *  and the version, this would be a claim about bytes nothing in this file has
 *  seen. The two constants are the fallback for a walk that sent none. */
function signWord(node: TreeNode): string {
  if (node.sign !== "") return node.sign;
  return node.kind === "links" ? BTREES.signLinks : BTREES.signIndex;
}

/**
 * One node written out: what it is and where, what it points at or holds, and
 * the keys at its ends.
 *
 * One function for the tooltip and the readout, so the two never say the same
 * node two different ways. A node whose children were not all read says so
 * instead of a range, because that is the same fact: the core clears a range
 * it could only settle at one end.
 */
function nodeLines(tree: Tree, node: TreeNode): string[] {
  const lines = [BTREES.selectedAt(kindWord(node), signWord(node), formatOffset(node.address * 8))];
  if (tree.version === 2) {
    // Both kinds of version 2 node hold records, so the count is a count of
    // records and the verb is "holds" for both. An internal node points at one
    // more child than that, which is what a B-tree is, and the second line is
    // where a reader who noticed the mismatch finds out why.
    lines.push(BTREES.holds(node.entries, "record"));
    if (node.kind === "index") lines.push(BTREES.pointsAtChildren(node.entries + 1));
  } else {
    const noun = holdWord(tree, node);
    lines.push(node.kind === "links" ? BTREES.holds(node.entries, noun) : BTREES.pointsAt(node.entries, noun));
  }
  if (node.truncated) {
    // The clause about a missing range only belongs where a range was going to
    // be shown. A version 2 group tree has none to begin with, and a tree whose
    // records were not read has none either; there the clause would read as a
    // second thing gone wrong.
    const ranged = tree.job === "chunk" && tree.records === "read";
    lines.push(ranged ? BTREES.truncated("chunk") : tree.version === 1 ? BTREES.truncated("link") : BTREES.truncatedNoRange);
    return lines;
  }
  if (node.first_key === "") return lines;
  lines.push(
    tree.job === "chunk"
      ? BTREES.selectedChunkRange(node.first_key, node.last_key)
      : BTREES.selectedRange(node.first_key, node.last_key),
  );
  return lines;
}

/**
 * What this node's own count counts.
 *
 * Four different nouns on purpose, because the file's one number means four
 * different things: a link table's entries are names, and an index node's are
 * pointers at whatever the next row down holds, which is index nodes above the
 * bottom row, link tables under a group tree's bottom row and chunks under a
 * chunk tree's. A single word for all four would be a label whose value is not
 * the thing the label names.
 */
function holdWord(tree: Tree, node: TreeNode): string {
  if (node.kind === "links") return "link";
  if (node.level > 0) return BTREES.kindIndex;
  return tree.job === "chunk" ? "chunk" : BTREES.kindLinks;
}

function keyOf(nodes: readonly number[]): string {
  return nodes.join(",");
}

/** The line under the heading: what this tree indexes, and which of the two
 *  structures it is.
 *
 *  The version is on the line for every tree and not only for a version 2 one.
 *  The two have different silhouettes, and a reader comparing a file to
 *  another file cannot tell from a picture of three rows of boxes which kind
 *  they are looking at unless it says. */
function jobCaption(tree: Tree): string {
  // On `records` and not on the name being empty, because the two cases are
  // not the same fact: a type the specification names is a thing this tree
  // indexes and can be said outright, and a type byte nothing names is a
  // number and a caveat.
  const other = tree.records === "unknown" ? BTREES.jobUnknown(tree.record_type) : BTREES.jobOther(tree.record_type_name);
  const job = tree.job === "group" ? BTREES.jobGroup : tree.job === "chunk" ? BTREES.jobChunk : other;
  return job + BTREES.versionTag(tree.version);
}

/** The line under the picture: what a box's width is and what the number on it
 *  is, which are two different facts about the same box.
 *
 *  A version 2 tree gets its own, because both of those facts are records
 *  rather than links or chunks: a version 2 tree is a B-tree, both kinds of
 *  node hold records, and what a record refers to is not in the tree at all. */
function widthCaption(tree: Tree): string {
  if (tree.version === 2) return BTREES.widthRecords;
  return tree.job === "group" ? BTREES.widthGroup : BTREES.widthChunk;
}

/**
 * What the drawing does not cover, in the order a reader meets it: what the
 * records are, and then why there are no keys.
 *
 * A version 2 B-tree is typed and the core reads two of the twelve types. The
 * shape of one does not depend on the type: it comes out of the header's node
 * size, record size and depth and out of the counts in the child pointers, so
 * a tree of a type nobody here decoded still has a true picture. What it does
 * not have is anything to say about the contents, and a picture that stayed
 * silent about that would be a picture a reader took for a full account.
 */
function caveats(tree: Tree): string[] {
  const out: string[] = [];
  if (tree.records === "unread") out.push(BTREES.recordsUnread(tree.record_type_name));
  if (tree.records === "unknown") out.push(BTREES.recordsUnknown(tree.record_type));
  // Why a version 2 group tree shows no first or last link anywhere: its
  // records hold a hash and a heap id, and the names are in the heap. Only
  // where the records were read, because `recordsUnread` has already accounted
  // for the missing range otherwise, and two notes about one absence read as
  // two problems.
  if (tree.version === 2 && tree.job === "group" && tree.records === "read") out.push(BTREES.groupRecordsNote);
  return out;
}

/**
 * The band under the last row of boxes, or null where there is no honest one
 * to draw.
 *
 * The count is the same sum `drawRows` puts in words on the last row, so the
 * picture and the line under it never disagree, and both are a floor when the
 * walk stopped early. Null when the last row drawn is index nodes that should
 * have had children: the walk ran out before it reached the bottom of the
 * tree, so what is below that row was never counted and a band would be a
 * number from nowhere. `omitted` says that happened.
 */
function leafBand(tree: Tree): { label: string; title: string } | null {
  // A version 2 tree's count is the header's own `record_count`, so the band
  // does not go null when the walk stopped above the leaves and does not say
  // "or more": what it prints is the file's number, not a sum of the last row
  // drawn. It could not be a sum of that row anyway: a version 2 tree is a
  // B-tree, its internal nodes hold records the leaves do not repeat, and the
  // bottom row adds up to less than the whole.
  if (tree.version === 2) {
    const n = tree.records_total;
    const capped = tree.omitted > 0;
    if (tree.records === "read" && tree.record_type === 5) {
      return { label: BTREES.leafLinksV2(n), title: BTREES.leafLinksV2Title(n, capped) };
    }
    if (tree.records === "read" && tree.record_type === 10) {
      return { label: BTREES.leafChunksV2(n), title: BTREES.leafChunksV2Title(n, capped) };
    }
    // A type whose records were not read. The count is still the header's, so
    // the band is drawn; what a record refers to is not claimed.
    const name = tree.records === "unknown" ? null : tree.record_type_name;
    return { label: BTREES.leafRecords(n), title: BTREES.leafRecordsTitle(n, name, tree.record_type, capped) };
  }
  const depth = Math.max(...tree.nodes.map((n) => n.depth));
  const row = tree.nodes.filter((n) => n.depth === depth);
  const first = row[0];
  if (first === undefined) return null;
  const held = row.reduce((sum, n) => sum + n.entries, 0);
  const capped = tree.omitted > 0;
  if (first.kind === "links") {
    return { label: BTREES.leafLinks(held, capped), title: BTREES.leafLinksTitle(row.length, held, capped) };
  }
  if (tree.job === "chunk" && first.level === 0) {
    return { label: BTREES.leafChunks(held, capped), title: BTREES.leafChunksTitle(row.length, held, capped) };
  }
  return null;
}

/**
 * The key to the number printed on a box: one box drawn the way the picture
 * draws them, with `N` where the count goes, and four words saying what the
 * count counts.
 *
 * Built once and never rebuilt: a legend rebuilt on every paint is work done
 * during a scroll for a picture that did not change. One word of it does
 * depend on the tree, because a version 1 node's number is entries and a
 * version 2 node's is records, so the row is handed back with that word's
 * element and a repaint sets its text. Its own class rather than `.btp-box`,
 * which is pressable and answers the mouse; a key that lights up under the
 * cursor is a control that does nothing.
 */
function keyRow(): { row: HTMLElement; label: HTMLElement } {
  const row = document.createElement("div");
  row.className = "btp-key";
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("class", "btp-svg btp-keysvg");
  svg.setAttribute("width", "26");
  svg.setAttribute("height", "14");
  svg.setAttribute("viewBox", "0 0 26 14");
  svg.setAttribute("aria-hidden", "true");
  const box = document.createElementNS(SVG, "rect");
  box.setAttribute("x", "0.5");
  box.setAttribute("y", "0.5");
  box.setAttribute("width", "25");
  box.setAttribute("height", "13");
  box.setAttribute("class", "btp-chip");
  const text = document.createElementNS(SVG, "text");
  text.setAttribute("x", "13");
  text.setAttribute("y", "7");
  text.setAttribute("class", "btp-count");
  text.textContent = BTREES.entriesChip;
  svg.append(box, text);
  const label = document.createElement("span");
  label.textContent = BTREES.entriesKey;
  row.append(svg, label);
  return { row, label };
}

/**
 * Where every node sits across the width, by what is under it.
 *
 * A node's weight is the number of things at the bottom of its own subtree:
 * the links in a link table, the chunks a bottom-row chunk index node points
 * at, or the sum of its children's weights. So a box spans exactly its
 * children, which is what makes the picture a tree without a line drawn
 * between any two boxes, and the bottom row's widths are each node's own count.
 *
 * A node with no weight at all still gets a sliver: an empty node is a fact
 * about the file, and a box of no width says nothing.
 */
function place(tree: Tree, width: number): Placed[] {
  const kids: number[][] = tree.nodes.map(() => []);
  for (let i = 1; i < tree.nodes.length; i++) {
    const parent = tree.nodes[i]?.parent ?? -1;
    if (parent >= 0) kids[parent]?.push(i);
  }
  const weight = new Array<number>(tree.nodes.length).fill(0);
  // Backwards over a breadth-first list: every child is settled before its
  // parent is asked about.
  for (let i = tree.nodes.length - 1; i >= 0; i--) {
    const own = kids[i] ?? [];
    if (own.length === 0) weight[i] = Math.max(1, tree.nodes[i]?.entries ?? 1);
    else weight[i] = own.reduce((sum, k) => sum + (weight[k] ?? 0), 0);
  }
  const placed: Placed[] = tree.nodes.map(() => ({ x: 0, w: 0, weight: 0 }));
  const root = placed[0];
  if (root !== undefined) {
    root.x = 0;
    root.w = width;
    root.weight = weight[0] ?? 1;
  }
  for (let i = 0; i < tree.nodes.length; i++) {
    const here = placed[i];
    const own = kids[i] ?? [];
    if (here === undefined || own.length === 0) continue;
    const total = own.reduce((sum, k) => sum + (weight[k] ?? 0), 0) || 1;
    let at = here.x;
    for (const k of own) {
      const child = placed[k];
      if (child === undefined) continue;
      child.x = at;
      child.w = (here.w * (weight[k] ?? 0)) / total;
      child.weight = weight[k] ?? 0;
      at += child.w;
    }
    // Rounding a thousand widths down leaves the last child short of its
    // parent's right edge, which reads as a parent wider than its children.
    const last = placed[own[own.length - 1] ?? -1];
    if (last !== undefined) last.w = here.x + here.w - last.x;
  }
  return placed;
}

/**
 * The boxes actually drawn: one a node, except where neighbouring siblings are
 * too narrow to press and are drawn as one.
 *
 * Only siblings are merged, and only ones next to each other, so a merged box
 * still sits inside its parent's span and the picture stays a true tree. What
 * a merged box says is how many nodes it stands for, which the readout and the
 * tooltip both carry.
 */
function boxesOf(tree: Tree, placed: readonly Placed[], width: number): Box[] {
  const out: Box[] = [];
  const depth = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
  for (let d = 0; d < depth; d++) {
    let pool: number[] = [];
    let from = 0;
    let parent = -2;
    const flush = (end: number): void => {
      if (pool.length === 0) return;
      out.push({ key: keyOf(pool), row: d, nodes: pool, x: from, w: Math.max(MIN_W, end - from) });
      pool = [];
    };
    for (let i = 0; i < tree.nodes.length; i++) {
      const node = tree.nodes[i];
      const at = placed[i];
      if (node === undefined || at === undefined || node.depth !== d) continue;
      if (pool.length > 0 && node.parent !== parent) flush(at.x);
      if (pool.length === 0) {
        from = at.x;
        parent = node.parent;
      }
      pool.push(i);
      if (at.x + at.w - from >= MIN_W) flush(at.x + at.w);
    }
    flush(width);
  }
  return out;
}

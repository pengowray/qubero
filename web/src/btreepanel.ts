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
 *
 * What either picture looks like is `btreedraw.ts`: this file is the state,
 * the walk, the events and the document, and it asks that one for geometry,
 * words and elements. The seam is where it is because what a box weighs is a
 * fact about a tree rather than about a panel, and while it lived here it
 * could only be checked by looking at the screen.
 */

import type { Doc, FieldPick, Tree } from "./doc.ts";
import { formatOffset } from "./doc.ts";
import {
  boxesOf,
  drawStrip,
  drawTree,
  entriesOf,
  entryNoun,
  entryReadout,
  readEntryKey,
  jobCaption,
  keyRow,
  place,
  readoutLines,
  rowLines,
  widthCaption,
  ROW_H,
  ROW_H_BIG,
  STRIP_H,
  STRIP_H_BIG,
} from "./btreedraw.ts";
import type { Box, Entry } from "./btreedraw.ts";
import { BTREES, TREEMAP } from "./strings.ts";

/** How many nodes one walk draws. A group of a million links has a quarter of
 *  a million link tables, and a picture of a quarter of a million boxes is not
 *  a picture; past this the walk says how many children it left out. */
const NODE_LIMIT = 4096;

/** How long after the cursor lands somewhere with no node of the drawn tree
 *  under it before another tree is asked for. A cursor moving through a file
 *  crosses plenty of places that are in no tree at all. */
const REWALK_MS = 150;

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
  /** The slices drawn inside the boxes that have nothing under them, one an
   *  entry of a node. Empty wherever `entriesOf` refused to divide a node. */
  private entries: readonly Entry[] = [];
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
      this.widths.hidden = true;
      this.widths.textContent = "";
      this.rows.replaceChildren();
      this.stripCap.hidden = true;
      this.stripCap.textContent = "";
      this.strip.hidden = true;
      this.strip.replaceChildren();
      this.span.hidden = true;
      this.span.textContent = "";
      this.readout.textContent = "";
      this.note.hidden = true;
      return;
    }
    this.note.hidden = false;
    const width = Math.max(1, Math.floor(this.plot.clientWidth || this.el.clientWidth));
    this.head.textContent = this.ownerName ?? this.owner(tree);
    this.job.textContent = jobCaption(tree);
    this.keyLabel.textContent = tree.version === 2 ? BTREES.recordsKey : BTREES.entriesKey;
    this.widths.textContent = widthCaption(tree);
    this.stripCap.textContent = BTREES.stripCaption;
    const placed = place(tree, width);
    this.boxes = boxesOf(tree, placed, width);
    this.entries = entriesOf(tree, this.boxes);
    this.plot.replaceChildren(drawTree(tree, this.boxes, this.entries, width, this.big ? ROW_H_BIG : ROW_H));
    this.drawRows(tree);
    // Two of the lines under the picture are about telling boxes apart, and a
    // picture with one box in it has nothing to tell apart: a width is only a
    // proportion against another width, and a key to the number on a box is a
    // second drawn box beside the only real one. Both go rather than stand
    // there saying nothing about the tree on screen.
    // A width is a proportion against another width, so the caption waits for
    // a row with two boxes on it. The key to the number on a box is a second
    // drawn box beside the real ones, which is worth its line once there is
    // more than one row of them to read it against.
    const rows = Math.max(...tree.nodes.map((n) => n.depth)) + 1;
    this.widths.hidden = !this.boxes.some((box) => this.boxes.some((other) => other.row === box.row && other !== box));
    this.keyLine.hidden = rows < 2;
    // What a press does, which depends on the tree: a version 1 node is placed
    // in the template and a version 2 node below the root is not.
    this.note.textContent = BTREES.hint(entryNoun(tree), tree.version === 2 && tree.nodes.length > 1);
    // The address picture is the tree's nodes in file order against its own
    // span. One node is in one order and spans its own bytes, so the strip is
    // a single mark against a scale it defines, its caption promises a second
    // reading of something, and the span line prints the node's own address
    // twice. The readout carries where that node is.
    const placedApart = tree.nodes.length > 1;
    this.stripCap.hidden = !placedApart;
    this.strip.hidden = !placedApart;
    this.span.hidden = !placedApart;
    if (placedApart) {
      const strip = drawStrip(tree, width, this.big ? STRIP_H_BIG : STRIP_H);
      this.strip.replaceChildren(strip.svg);
      this.span.textContent = strip.span;
    } else {
      this.strip.replaceChildren();
      this.span.textContent = "";
    }
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
    // The kind for an owner the list could not name, from the job the tree
    // does: a group tree belongs to a group and a chunk tree to a dataset,
    // which the walk settled by what the tree hangs under. A version 2 tree
    // indexing anything else leaves the noun at "object", which is the only
    // thing true of all twelve record types.
    const kind = tree.job === "group" ? BTREES.headGroupWord : tree.job === "chunk" ? BTREES.headDatasetWord : BTREES.headObjectWord;
    if (root === undefined) return BTREES.unnamed(kind, "");
    const at = formatOffset(root.address * 8);
    const reply = this.doc.contents();
    // Not kept while the list is still being read: the bytes it is waiting on
    // arrive and paint again, and an answer cached from a half-read list would
    // outlive the reason it was wrong.
    if (reply.status !== "ok") return BTREES.unnamed(kind, at);
    let name = "";
    let group = false;
    let deepest = -1;
    for (const object of reply.node.objects) {
      if (object.path.length > root.path.length || object.path.length <= deepest) continue;
      if (!object.path.every((step, k) => step === root.path[k])) continue;
      deepest = object.path.length;
      name = object.name;
      group = object.group;
    }
    // The path alone was the whole heading, and the root group's path is one
    // slash: a line a reader cannot tell from a separator, under a job line
    // that used to point at it with "this group". The kind word in front is
    // what makes the shortest path in the file read as a thing.
    this.ownerName = name === "" ? BTREES.unnamed(kind, at) : group ? BTREES.headGroup(name) : BTREES.headDataset(name);
    return this.ownerName;
  }

  /** The summary under the picture, as rows in the document: one line a row,
   *  so how wide the tree is at each level is a number as well as a picture,
   *  and then the caveats under them. What each line says is `rowLines`; this
   *  is only where it lands and which class it takes. */
  private drawRows(tree: Tree): void {
    this.rows.replaceChildren(
      ...rowLines(tree).map((row) => {
        const line = document.createElement("div");
        line.className = row.muted === true ? "btp-row btp-row-muted" : row.short ? "btp-row btp-row-short" : "btp-row";
        line.textContent = row.text;
        if (row.title !== undefined) line.title = row.title;
        return line;
      }),
    );
  }

  /** What the pressed box is, written out under the picture, one line a div. */
  private drawReadout(): void {
    const tree = this.tree;
    const entry = this.entries.find((e) => e.key === this.selected);
    if (tree !== null && entry !== undefined) {
      this.readout.replaceChildren(
        ...entryReadout(tree, entry).map((text) => {
          const line = document.createElement("div");
          line.textContent = text;
          return line;
        }),
      );
      return;
    }
    const box = this.boxes.find((b) => b.key === this.selected);
    if (tree === null || box === undefined) {
      this.readout.replaceChildren();
      this.readout.removeAttribute("title");
      return;
    }
    this.readout.replaceChildren(
      ...readoutLines(tree, box).map((text) => {
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
    // A slice inside a node. It goes to its own bytes and is marked like a
    // box, and a second press does no more than a first: an entry is a run of
    // bytes the template places under the node, with no field of its own for
    // the Listing to open.
    if (readEntryKey(key) !== null) {
      this.selected = this.selected === key ? null : key;
      this.light();
      this.drawReadout();
      const entry = this.entries.find((e) => e.key === key);
      if (this.selected === null || entry === undefined) return;
      this.onJump(entry.startBit, entry.endBit);
      return;
    }
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

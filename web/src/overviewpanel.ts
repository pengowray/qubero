// The rail down the left: the file described before it is read, and the way
// around it. How big it is, what kind of bytes it is made of and where they
// sit, the parts a template divides it into and which one the main view is
// looking at. It sits beside the hex grid, the listing and the text view,
// because "what is in this file, and where am I" is the same question from
// any of them.
//
// Top to bottom: the facts, the minimap with its layout strip and the block
// picked on it (`minimappanel`), the treemap (`treemappanel`), and the
// Contents (the listing's headings, one source of truth for the parts of the
// file) with a Logical tab beside it for formats that have their own objects.
//
// What is left here is the rail itself and the Contents: the panel owns the
// heading that folds it away, the two facts at the top, and the outlines. The
// two pictures of the file are panels of their own, and this one hands each
// what it needs: where the main view is looking, and which parts the listing
// last named.

import { formatBytes, formatOffset, percentText } from "./doc.ts";
import { BTREES, NO_TEMPLATE, REPORT } from "./strings.ts";
import type { Doc } from "./doc.ts";
import type { FieldPick } from "./doc.ts";
import { factRow, noneLine, noteLine } from "./dom.ts";
import { MinimapPanel } from "./minimappanel.ts";
import { TreemapPanel } from "./treemappanel.ts";
import { BTreePanel } from "./btreepanel.ts";
import type { MapSegment } from "./filemap.ts";
import type { OutlineHeading, Viewport } from "./outline.ts";
import { hasLogicalOutline, logicalLength, logicalOutline } from "./logicaloutline.ts";
import type { LogicalNode, LogicalOutline } from "./logicaloutline.ts";

/** Top-level parts listed before the list says how many more there are. A
 *  SQLite file of a hundred thousand pages is not a hundred thousand buttons;
 *  the part the view is in is always listed, wherever it falls. */
const PARTS_SHOWN = 200;
/** How long after a press on a map the Contents list holds still. */
const MAP_PRESS_QUIET_MS = 1000;
/** The layout strip draws one cell per part. Past this many, neighbouring
 *  parts share a cell: a strip of a hundred thousand two-pixel cells would be
 *  neither drawable nor readable. */
const STRIP_SEGMENTS = 256;
/** Rows a "show more" press adds to a logical section. */
const LOGICAL_PAGE = 80;
/** How far each level of the logical tree steps in. */
const LOGICAL_INDENT_PX = 12;

const TITLE = "Overview";
const SIZE_LABEL = "Size";
const TYPE_LABEL = "Type";
const UNKNOWN_TYPE = "Not identified";
const CONTENTS_TAB = "Contents";
const LOGICAL_TAB = "Logical";
const BTREES_TAB = BTREES.tab;
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

/** One top-level part of the file and the named parts inside it. */
type Part = { readonly head: OutlineHeading; readonly subs: readonly OutlineHeading[] };

/** Which part the main view is looking at: an index into the parts, and the
 *  index of the named part inside it, or -1 when the top of the view is not
 *  inside one of those. */
type Place = { readonly part: number; readonly sub: number };

type Tab = "contents" | "logical" | "btrees";

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
  private readonly tabs: HTMLElement;
  private readonly contentsTab: HTMLButtonElement;
  private readonly logicalTab: HTMLButtonElement;
  private readonly btreesTab: HTMLButtonElement;
  private readonly contentsEl: HTMLElement;
  private readonly logicalEl: HTMLElement;
  /** The third tab's whole panel: one B-tree drawn in the shape the file gives
   *  it. A panel of its own rather than a list, the way the treemap is. */
  private readonly btrees: BTreePanel;
  /** The two pictures of the file, in the order the questions come: where the
   *  bytes of each kind are, then how much of the file each part is. */
  private readonly minimap: MinimapPanel;
  private readonly treemap: TreemapPanel;

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
  /** The stretch of the file the main view is showing, for working out which
   *  part that is. */
  private viewport: Viewport | null = null;
  /** The rows on screen, by part index, and the named parts listed under each
   *  part the reader has unfolded. */
  private partRows = new Map<number, HTMLElement>();
  private subRowsByPart = new Map<number, HTMLElement[]>();
  /** The parts the reader has unfolded, by heading key. Folding is the
   *  reader's, not the view's: rows must not appear and disappear under
   *  someone reading the list while the main view scrolls. Kept for the
   *  session, and started again for a different template. */
  private readonly openParts = new Set<string>();
  private seededTemplate: string | null = null;
  private seeded = false;
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
  /** When a map was last pressed, so the list does not scroll to follow the
   *  view the press moved. */
  private mapPressAt = -Infinity;
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

    this.tabs = document.createElement("div");
    this.tabs.className = "ov-tabs";
    this.tabs.setAttribute("role", "tablist");
    this.contentsTab = this.tabButton(CONTENTS_TAB, "contents");
    this.logicalTab = this.tabButton(LOGICAL_TAB, "logical");
    this.btreesTab = this.tabButton(BTREES_TAB, "btrees");
    this.btreesTab.title = BTREES.what;
    this.tabs.append(this.contentsTab, this.logicalTab, this.btreesTab);
    this.contentsEl = document.createElement("div");
    this.contentsEl.className = "ov-parts";
    this.contentsEl.setAttribute("role", "tabpanel");
    this.logicalEl = document.createElement("div");
    this.logicalEl.className = "ov-logical";
    this.logicalEl.setAttribute("role", "tabpanel");
    // The same two verbs as the two maps above: a press goes to the node's
    // bytes, a second press opens it in the listing.
    this.btrees = new BTreePanel(this.doc);
    this.btrees.el.setAttribute("role", "tabpanel");
    this.btrees.onJump = (startBit, endBit) => this.onJump(startBit, endBit);
    this.btrees.onPick = (pick) => this.onPick(pick);
    this.btrees.onResize = () => this.pump();

    // Where the bytes of each kind are, with the parts of the file as a strip
    // under it and the block a reader opened out of it under that.
    this.minimap = new MinimapPanel(this.doc);
    this.minimap.onJump = (startBit, endBit) => this.onJump(startBit, endBit);
    this.minimap.onPressed = () => {
      this.mapPressAt = performance.now();
    };
    // The same widget the Treemap view mounts, given a column instead of the
    // workspace. The map above says where things are; this says how much of
    // the file they are, which is the question the map cannot answer once the
    // small things are under a pixel.
    this.treemap = new TreemapPanel(this.doc);
    // A box is a stretch of the file, so pressing one does what pressing a
    // cell of the minimap does. It was never wired: a press marked the box and
    // went nowhere, which made the two pictures answer the same gesture two
    // different ways, and the hint under the map promised the one that did not
    // happen.
    this.treemap.onJump = (startBit, endBit) => this.onJump(startBit, endBit);
    this.treemap.onPick = (pick) => this.onPick(pick);
    this.treemap.onResize = () => this.pump();

    this.body = document.createElement("div");
    this.body.className = "ov-body";
    // In the order the questions come: what it is, where the bytes of each
    // kind are, then the parts by name. The treemap, how much of the file each
    // part is, waits folded at the foot while it is rough.
    this.body.append(
      this.facts,
      this.minimap.el,
      this.tabs,
      this.contentsEl,
      this.logicalEl,
      this.btrees.el,
      this.treemap.el,
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
    this.tab = savedTab === "logical" || savedTab === "btrees" ? savedTab : "contents";
    this.syncTabs();

    this.logicalEl.addEventListener("click", (e) => this.onLogicalClick(e));

    // The cells wrap to the sidebar's width, so a narrower window is a
    // different map rather than the same one clipped.
    new ResizeObserver(() => {
      this.drawFacts();
      this.minimap.relayout();
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
    this.drawFacts();
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
    this.treemap.setOutline(headings);
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
    this.minimap.setParts(stripSegments(this.parts));
    return true;
  }

  /**
   * The stretch of the file the main view is showing. The part its top is in
   * is marked, and the rail scrolls to keep the mark on screen. Scrolling
   * marks; it never folds or unfolds, so the list keeps the shape the reader
   * gave it. Only the mark moves the rail: a view that says it is still where
   * it was must not fight the reader scrolling the rail.
   */
  setViewport(v: Viewport): void {
    this.viewport = v;
    this.minimap.setViewport(v);
    const place = this.placeOf(v);
    if (samePlace(place, this.place) && this.seeded) return;
    const wasPart = this.place?.part;
    this.place = place;
    // The row for the new part may be past the listed ones, and the old
    // part's row may have been listed only because the view was in it. The
    // first place of all also settles which section starts open.
    if (
      !this.seeded ||
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
    // The B-trees tab marks the node the cursor landed in, the same way the
    // graph view marks the field it was drawn for.
    this.btrees.reveal(path);
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
    this.treemap.pump();
    this.minimap.pump();
    // A streamed file grows under the rail, so the size is worth saying again
    // whenever the scan is nudged. The facts wait on that scan the way they
    // always have: a rail that says "0 bytes" of a file nothing has read yet
    // is stating a number it does not have.
    this.drawFacts();
    if (this.logicalStale && this.tab === "logical") this.scheduleLogical();
    this.btrees.setShown(this.tab === "btrees" && !this.body.hidden);
    this.btrees.pump();
  }

  private drawFacts(): void {
    if (this.minimap.scan === null) return;
    const len = this.doc.lengthBytes;
    const size = len < 1024 ? `${len.toLocaleString()} bytes` : `${formatBytes(len)} (${len.toLocaleString()} bytes)`;
    const rows: [string, string][] = [[SIZE_LABEL, size]];
    const type = this.identity !== "" ? this.identity : this.doc.template ?? (this.identified ? UNKNOWN_TYPE : "");
    if (type !== "") rows.push([TYPE_LABEL, type]);
    this.facts.replaceChildren(...rows.flatMap(([k, v]) => factRow(k, v)));
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

  /**
   * Show the chosen tab, and only offer the two that not every format has:
   * Logical where a format has objects of its own to list, B-trees where one
   * keeps its structure in a tree of nodes scattered through the file. The
   * template is sniffed after the document opens, so either offer can appear,
   * or go, later.
   *
   * The B-trees tab is offered for the one format that has them at all rather
   * than for a file already known to have one, because knowing that is the
   * walk itself, and a walk run to decide whether to show a hidden tab is a
   * walk nobody asked for. A file of that format with no version 1 tree in it
   * says so in the panel.
   */
  private syncTabs(): void {
    const hasLogical = hasLogicalOutline(this.doc);
    const hasTrees = this.doc.template === "hdf5";
    this.logicalTab.hidden = !hasLogical;
    this.btreesTab.hidden = !hasTrees;
    if (!hasLogical && this.tab === "logical") this.tab = "contents";
    if (!hasTrees && this.tab === "btrees") this.tab = "contents";
    for (const [button, panel, tab] of [
      [this.contentsTab, this.contentsEl, "contents"],
      [this.logicalTab, this.logicalEl, "logical"],
      [this.btreesTab, this.btrees.el, "btrees"],
    ] as const) {
      const on = this.tab === tab;
      button.classList.toggle("is-on", on);
      button.setAttribute("aria-selected", String(on));
      panel.hidden = !on;
    }
    // The panel walks the tree only while it can be seen, so it has to be told
    // both ways round rather than working it out from its own visibility.
    this.btrees.setShown(this.tab === "btrees" && !this.body.hidden);
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

  /** Which part's fold state is which, across a list built again. The
   *  listing's own key for the heading, so a streamed file that gains parts
   *  keeps the ones the reader opened open. */
  private foldKey(part: Part): string {
    return part.head.key !== "" ? part.head.key : `section:${part.head.section}`;
  }

  /**
   * Fold every section, except the one the file was opened at. Done once per
   * template: after that the list is the reader's to fold and unfold, and
   * nothing else touches it.
   */
  private seedFolds(): void {
    if (this.seeded && this.seededTemplate === this.doc.template) return;
    if (this.parts.length === 0 || this.place === null) return;
    this.seeded = true;
    this.seededTemplate = this.doc.template;
    this.openParts.clear();
    const part = this.parts[this.place.part];
    if (part !== undefined) this.openParts.add(this.foldKey(part));
  }

  /** Build the list of parts again: the first so many, the one the view is
   *  in wherever it falls, and a count for the rest. `scroll` is false when
   *  the reader pressed a caret, since the list should stay where they are
   *  looking rather than jump back to the mark. */
  private drawContents(scroll = true): void {
    this.drawnTemplate = this.doc.template;
    this.partRows = new Map();
    this.subRowsByPart = new Map();
    const out: HTMLElement[] = [];
    if (this.note !== "") out.push(noteLine(this.note));
    if (this.doc.template === null) {
      out.push(noneLine(NO_TEMPLATE));
      this.contentsEl.replaceChildren(...out);
      return;
    }
    if (this.headings === null || this.headingsTemplate !== this.doc.template) {
      out.push(noneLine(PARTS_PENDING));
      this.contentsEl.replaceChildren(...out);
      return;
    }
    const parts = this.parts;
    if (parts.length === 0) {
      out.push(noneLine(NO_PARTS));
      this.contentsEl.replaceChildren(...out);
      return;
    }
    this.seedFolds();
    const current = this.place?.part ?? -1;
    const shown = Math.min(parts.length, PARTS_SHOWN);
    for (let i = 0; i < shown; i++) out.push(...this.partRow(i));
    if (current >= shown) {
      if (current > shown) out.push(noneLine(MORE_PARTS(current - shown)));
      out.push(...this.partRow(current));
      if (parts.length - current - 1 > 0) out.push(noneLine(MORE_PARTS(parts.length - current - 1)));
    } else if (parts.length > shown) {
      out.push(noneLine(MORE_PARTS(parts.length - shown)));
    }
    this.contentsEl.replaceChildren(...out);
    this.markPlace();
    if (scroll) this.showPlace();
  }

  /** One part's row, and under it the list of its named parts while the
   *  reader has it unfolded. */
  private partRow(i: number): HTMLElement[] {
    const part = this.parts[i];
    if (part === undefined) return [];
    const open = this.openParts.has(this.foldKey(part));
    const row = this.headingRow(part.head, 0, part.subs.length > 0 ? open : null);
    row.dataset["part"] = String(i);
    this.partRows.set(i, row);
    row.addEventListener("click", (e) => {
      if (e.target instanceof HTMLElement && e.target.closest(".ov-fold") !== null) return;
      this.pickHeading(part.head);
    });
    const caret = row.querySelector(".ov-fold");
    if (caret instanceof HTMLButtonElement) caret.addEventListener("click", () => this.toggleFold(i));
    const out = [row];
    if (open) {
      const subs = this.subList(i, part);
      if (subs !== null) out.push(subs);
    }
    return out;
  }

  /** Fold or unfold one part. The mark does not move, so the rail is left
   *  where the reader's eye is. */
  private toggleFold(i: number): void {
    const part = this.parts[i];
    if (part === undefined) return;
    const key = this.foldKey(part);
    if (this.openParts.has(key)) this.openParts.delete(key);
    else this.openParts.add(key);
    this.drawContents(false);
  }

  /** The named parts inside one part, as rows under its own. Null for a part
   *  with none. */
  private subList(i: number, part: Part): HTMLElement | null {
    if (part.subs.length === 0) return null;
    const subs = document.createElement("div");
    subs.className = "ov-subs";
    const rows = part.subs.map((h, j) => {
      const sub = this.headingRow(h, 1, null);
      sub.dataset["sub"] = String(j);
      sub.addEventListener("click", () => this.pickHeading(h));
      return sub;
    });
    this.subRowsByPart.set(i, rows);
    subs.append(...rows);
    return subs;
  }

  /**
   * One row of the Contents. `fold` is null for a row with nothing to fold,
   * and otherwise says whether it is open: a level-0 row carries the caret
   * that opens it, which is why it is a div with a button in it rather than a
   * button of its own.
   */
  private headingRow(h: OutlineHeading, level: 0 | 1, fold: boolean | null): HTMLElement {
    const fileBits = Math.max(1, this.doc.lengthBits);
    const row = document.createElement(level === 0 ? "div" : "button");
    if (row instanceof HTMLButtonElement) row.type = "button";
    else {
      row.setAttribute("role", "button");
      row.tabIndex = 0;
      row.addEventListener("keydown", (e) => {
        if (e.key !== "Enter" && e.key !== " ") return;
        e.preventDefault();
        row.click();
      });
    }
    row.className = level === 0 ? "ov-part" : "ov-part ov-part-sub";
    if (level === 0) {
      if (fold === null) {
        const spacer = document.createElement("span");
        spacer.className = "ov-fold ov-fold-leaf";
        row.append(spacer);
      } else {
        const caret = document.createElement("button");
        caret.type = "button";
        caret.className = "ov-fold";
        caret.textContent = fold ? "▾" : "▸";
        caret.setAttribute("aria-label", fold ? COLLAPSE : EXPAND);
        caret.setAttribute("aria-expanded", String(fold));
        row.append(caret);
      }
    }
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

  /** Where the part under the pointer sits, said on the minimap: its cells lit
   *  and the rest dimmed, and the same stretch marked on the layout strip. */
  private hoverPart(h: OutlineHeading | null): void {
    this.minimap.setHighlight(h === null ? null : { offsetBits: h.offsetBits, sizeBits: h.sizeBits });
  }

  /** Put the mark on the row for the part the view is in, and take it off the
   *  others. Marking only: which rows are on screen is the reader's, so a
   *  part that is folded carries the mark itself, and an open one hands it to
   *  the named part inside. */
  private markPlace(): void {
    const place = this.place;
    for (const [i, row] of this.partRows) {
      const on = place !== null && place.part === i;
      const inside = on && place.sub >= 0 && this.subRowsByPart.has(i);
      row.classList.toggle("is-current", on && !inside);
      row.classList.toggle("is-inside", inside);
      if (on) row.setAttribute("aria-current", "location");
      else row.removeAttribute("aria-current");
    }
    for (const [i, rows] of this.subRowsByPart) {
      rows.forEach((row, j) => {
        const on = place !== null && place.part === i && place.sub === j;
        row.classList.toggle("is-current", on);
        if (on) row.setAttribute("aria-current", "location");
        else row.removeAttribute("aria-current");
      });
    }
  }

  /** Scroll the rail so the marked row can be seen. Not after a press on one
   *  of the maps: the reader is looking at the map, and the list moving under
   *  their pointer is the wrong thing to notice. The view moves in its own
   *  time (a glide can report several viewports), so this is a window rather
   *  than a one-shot flag. */
  private showPlace(): void {
    if (this.body.hidden || this.tab !== "contents") return;
    if (performance.now() - this.mapPressAt < MAP_PRESS_QUIET_MS) return;
    const place = this.place;
    if (place === null) return;
    const subs = this.subRowsByPart.get(place.part);
    const row = (place.sub >= 0 ? subs?.[place.sub] : undefined) ?? this.partRows.get(place.part);
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
        noneLine(reply.status === "error" ? LOGICAL_FAILED(reply.message) : LOGICAL_READING(reply.reachedBytes)),
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
    const out: HTMLElement[] = [noteLine(`${reply.node.title} · ${reply.node.summary}`)];
    if (reply.node.progressText !== undefined) out.push(noneLine(reply.node.progressText));
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
      out.push(noneLine(LOGICAL_UNLISTED(reply.node.total - reply.node.nodes.length)));
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
    // The B-trees tab follows whichever object the reader is standing on, and
    // picking one here is standing on it. Told directly rather than through
    // the cursor: picking sets the cursor with the round trip suppressed, so
    // nothing comes back this way to say where the reader went.
    this.btrees.reveal(path);
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
}

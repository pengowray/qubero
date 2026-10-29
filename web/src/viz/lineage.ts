// Lineage breadcrumb: which streams a tab, or the field under the cursor, was
// unpacked or joined from, from the file down.
//
// Candidate `space-nesting-components` in docs/DESIGN-diagrams.md, section
// 4.2; this is the slice agent J of the first batch built (section 5.3): the
// inline form, a breadcrumb of the streams between the file and the bytes on
// show, in two places.
// - The tab strip. A tab over a stream is titled by the streams it is inside,
//   `image › compressed › scanlines` for a PNG's pixels, fitted to the room a
//   tab has (`fitCrumbs`), and its hover card says how each was made and how
//   big it is. The title used to be `scanlines unpacked from file.png`, which
//   left out the two streams in between.
// - The At cursor panel, as a section (`inspectorsections.ts`): the same
//   streams for the field under the cursor, one row each, with how the stream
//   was made and its sizes, in the file's tab as much as in a stream's. The
//   cursor on a pixel of a PNG's own tab already shows all three, none of
//   them opened yet. A stream's name brings up its tab, opening it when it is
//   not open yet; a run's address goes to the run's bytes in the tab of the
//   stream one row up, opening that one the same way. The run's offsets are
//   bits of that stream, so no other tab has bytes to put the cursor on.
//   Hovering a run of the file marks its bytes in the hex view.
//
// The component diagram the design draws around this (a box per stream, to
// scale, with a strip of where it sat in its parent) is the later part of the
// candidate and is not here. Nothing in a breadcrumb zooms or pans, so the
// board is not used; hover, click and the shared hover are the whole of it.
//
// What a tab is inside is two things put together. Which tab a stream tab was
// opened from is known only here, told by `main.ts` as each tab opens
// (`noteOpened`). The path from that tab's reading down to the stream is the
// editor's (`Doc.spaceOrigin`), and walking it (`spaceSteps`) finds every
// stream on the way, including those never opened as tabs. The reading a
// path is walked in is the file's or a recognised stream's, a tar inside a
// gzip file say: a stream read by the template it declared has no reading of
// its own, and its paths are its home's.

import type { Doc, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { bitSizeText, VIZ } from "../strings.ts";
import { canvasMeasure, type Measure } from "./fit.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { cutMiddle, fitCrumbs, spaceSteps, type SpaceStep } from "./lineagemath.ts";
import { linkedFor } from "./linked.ts";
import { goToField } from "./pick.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

/** One crumb: the file, or one stream. */
export type Crumb = {
  readonly label: string;
  /** The document `step`'s paths are paths of, and whose tab opens the
   *  stream. For the file, the file. */
  readonly from: Doc;
  /** How the stream was made. Null for the file. */
  readonly step: SpaceStep | null;
  /** The stream's or the file's size, in bits. */
  readonly sizeBits: number;
};

/** Which document each stream tab was opened from. */
const parents = new WeakMap<Doc, Doc>();

/** Say that `child`, a stream's document, was opened from `parent`'s tab.
 *  Called by `main.ts` as the tab opens. */
export function noteOpened(child: Doc, parent: Doc): void {
  if (child !== parent) parents.set(child, parent);
}

/** What the tabs let a crumb do, set by `main.ts`. */
export type LineageRoutes = {
  /** Bring the tab showing `doc` to the front. False when it is not open. */
  readonly focus: (doc: Doc) => boolean;
  /** The documents of the open document tabs. */
  readonly open: () => readonly Doc[];
};
let routes: LineageRoutes = { focus: () => false, open: () => [] };
export function setLineageRoutes(r: LineageRoutes): void {
  routes = r;
}

/** Each document tab's Open unpacked, which opens a stream at a path of that
 *  document as a tab, or brings its tab up when it is already open, and says
 *  which document that tab shows. Null while the stream is still being
 *  unpacked (the tab opens by itself when it is done) or when it will not
 *  open. */
export type LineageOpener = (path: readonly number[]) => Doc | null;
const openers = new WeakMap<Doc, LineageOpener>();
export function setLineageOpener(doc: Doc, open: LineageOpener): () => void {
  openers.set(doc, open);
  return () => {
    if (openers.get(doc) === open) openers.delete(doc);
  };
}

/** The document whose reading `doc`'s paths and its streams' origins are
 *  paths of: itself when it has a reading of its own, and otherwise the one
 *  its tab was opened from, or that one's. Null when that is not known. */
function homeOf(doc: Doc): Doc | null {
  if (doc.isFile || doc.recognised) return doc;
  const up = parents.get(doc);
  return up === undefined ? null : homeOf(up);
}

function nodeOf(doc: Doc): (path: readonly number[]) => TemplateNode | null {
  return (p) => {
    const r = doc.templateNode(p);
    return r.status === "ok" ? r.node : null;
  };
}

/**
 * The crumbs of a tab: the file, then every stream down to the one the tab
 * shows, which is last. Just the file for the file's tab. Null when the tab
 * was not opened by `main.ts`'s Open unpacked, or its reading is still on its
 * way; the caller keeps the title it had.
 */
export function tabLineage(doc: Doc): Crumb[] | null {
  if (doc.isFile) return [{ label: doc.name, from: doc, step: null, sizeBits: doc.lengthBits }];
  const parent = parents.get(doc);
  if (parent === undefined) return null;
  const home = homeOf(parent);
  if (home === null) return null;
  const above = tabLineage(home);
  if (above === null) return null;
  const origin = doc.spaceOrigin();
  if (origin.length === 0) return null;
  // The stream's own root is one below the node it was opened from, which
  // is where the last step ends.
  const steps = spaceSteps([...origin, 0], nodeOf(home));
  if (steps === null || steps.length === 0) return null;
  return [...above, ...steps.map((step) => crumbOf(home, step))];
}

/**
 * The crumbs for the field at `path` of `doc`: the tab's, then every stream
 * inside the tab the path goes into. Null where the tab's are not known.
 */
export function fieldLineage(doc: Doc, path: readonly number[]): Crumb[] | null {
  const above = tabLineage(doc);
  if (above === null) return null;
  const steps = spaceSteps(path, nodeOf(doc));
  if (steps === null) return null;
  return [...above, ...steps.map((step) => crumbOf(doc, step))];
}

function crumbOf(from: Doc, step: SpaceStep): Crumb {
  return { label: step.label, from, step, sizeBits: step.outBits };
}

/** How many parts a joined stream is joined from, by asking which part its
 *  first byte is in. Null when the core does not say. */
function partsOf(c: Crumb): { readonly count: number; readonly first: string } | null {
  if (c.step === null || !c.step.joined) return null;
  const part = c.from.partOf(c.step.root);
  return part === null ? null : { count: part.parts, first: part.label };
}

/** The line under a crumb's name: its size, and how it was made. */
function howText(c: Crumb): string {
  if (c.step === null) return bitSizeText(c.sizeBits);
  if (c.step.joined) return VIZ.lineage.joined(c.step.outBits, partsOf(c)?.count ?? null);
  const at = formatOffset(c.step.atBits);
  return isStored(c.step) ? VIZ.lineage.stored(c.step.inBits, at) : VIZ.lineage.unpacked(c.step.inBits, c.step.codec, at, c.step.outBits);
}

/** A run kept as it is, whose bytes are the stream's: nothing to say about
 *  unpacking. Recognised by the codec's name and by the sizes agreeing, so a
 *  codec that only happens to be called that is still worded as unpacked. */
function isStored(step: SpaceStep): boolean {
  return step.codec === "stored" && step.inBits === step.outBits;
}

/** The hover card of one crumb. `parent` is the crumb one up, whose bytes or
 *  stream the run is in. */
export function crumbCard(c: Crumb, parent: Crumb | null, action: string | null = null): HTMLElement {
  const lines = [howText(c)];
  const parts = partsOf(c);
  if (parts !== null) lines.push(VIZ.lineage.firstPart(parts.first));
  if (action !== null) lines.push(action);
  const where = c.step !== null && !c.step.joined && parent !== null ? VIZ.lineage.where(formatOffset(c.step.atBits), parent.label) : "";
  return tipCard(c.label, lines, where);
}

/** The hover card of a whole tab: one line per crumb, the file first. */
export function lineageCard(crumbs: readonly Crumb[]): HTMLElement {
  const last = crumbs[crumbs.length - 1];
  return tipCard(last?.label ?? "", crumbs.map((c) => `${c.label}: ${howText(c)}`));
}

/** Bring up the tab a crumb stands for, the file's or the stream's, opening
 *  the stream's if it is not open yet, and say which document it shows. Null
 *  when it cannot be brought up now. */
function showCrumb(c: Crumb): Doc | null {
  if (c.step === null) return routes.focus(c.from) ? c.from : null;
  return openers.get(c.from)?.(c.step.opener) ?? null;
}

/** Where a stream's root is in its home's reading, for telling whether a tab
 *  shows it. Null when that is not known. */
function rootInHome(c: Crumb): { readonly home: Doc; readonly path: readonly number[] } | null {
  if (c.step === null) return null;
  if (c.from.isFile || c.from.recognised) return { home: c.from, path: c.step.root };
  const home = homeOf(c.from);
  if (home === null) return null;
  // A tab with no reading of its own has its stream's root as its `[]`.
  return { home, path: [...c.from.spaceOrigin(), 0, ...c.step.root] };
}

/** Whether a tab showing a crumb's stream, or the file, is open now. */
function isOpen(c: Crumb): boolean {
  if (c.step === null) return routes.open().includes(c.from);
  const want = rootInHome(c);
  if (want === null) return false;
  return routes.open().some((d) => {
    if (d === c.from || d.isFile) return false;
    const up = parents.get(d);
    if (up === undefined || homeOf(up) !== want.home) return false;
    const root = [...d.spaceOrigin(), 0];
    return root.length === want.path.length && root.every((v, i) => v === want.path[i]);
  });
}

/** How long a run's address waits for its stream to finish unpacking before
 *  giving up: tries, and the time between them. */
const RUN_TRIES = 40;
const RUN_WAIT_MS = 100;

/**
 * Go to a stream's run in the tab of the stream it is in, `parent`, opening
 * that tab if it is not open yet. The run's offsets are bits of the parent
 * stream, so that tab is the only one with bytes to put the cursor on.
 *
 * The run's path is a path of `c.from`. The parent's tab is either that same
 * document (a run in a tab's own bytes) or a tab over the parent stream,
 * whose `[]` is the stream's root, so the path is cut to start below it.
 */
function goToRun(c: Crumb, parent: Crumb, tries = 0): void {
  const step = c.step;
  if (step === null || step.joined) return;
  const there = showCrumb(parent);
  if (there === null) {
    // Still unpacking, most likely: the tab opens by itself when it is done,
    // and the run is gone to then.
    if (tries < RUN_TRIES) setTimeout(() => goToRun(c, parent, tries + 1), RUN_WAIT_MS);
    return;
  }
  let path: readonly number[];
  if (there === c.from) path = step.opener;
  else if (parent.step !== null && parent.from === c.from && startsWith(step.opener, parent.step.root)) path = step.opener.slice(parent.step.root.length);
  else return;
  goToField(there, { path, startBit: step.atBits, endBit: step.atBits + step.inBits });
}

function startsWith(path: readonly number[], prefix: readonly number[]): boolean {
  return prefix.length <= path.length && prefix.every((v, i) => v === path[i]);
}

// ----- the tab strip -----

/** The tab title's text size (`.tab-pick`). */
const TAB_PX = 12;
/** The widest a tab's title is drawn. The strip scrolls sideways past that,
 *  so this is what keeps three stream tabs in view at once. */
const TAB_ROOM = 300;
/** Gap and arrow between two crumbs, and the mark for those left out. */
const SEP = "›";
/** The padding either side of the arrow, `.viz-lineage-sep`. */
const SEP_PAD = 4;
const MORE = "…";
/** Fewer characters than this is not a name worth cutting down to. */
const MIN_CHARS = 4;

let measure: Measure | null = null;
function measurer(): Measure {
  measure ??= canvasMeasure(getComputedStyle(document.body).fontFamily);
  return measure;
}

/**
 * The crumbs a tab's title is made of: every one but the file, which is the
 * strip's first tab and need not be said again. A stream out of a run of
 * bytes lifted into a tab of its own keeps the name of that tab first.
 */
export function titleCrumbs(crumbs: readonly Crumb[], windowFile: Doc | null): readonly Crumb[] {
  const first = crumbs[0];
  return crumbs.length > 1 && first !== undefined && first.step === null && first.from === windowFile ? crumbs.slice(1) : crumbs;
}

/** A tab's title as plain text, for the toolbar, the close button and the
 *  reader of a screen. Every crumb, the file's too. */
export function tabTitleText(crumbs: readonly Crumb[]): string {
  return VIZ.lineage.tabTitle(crumbs.map((c) => c.label));
}

/** A tab's title as drawn: the names, fitted to the tab, the tab's own stream
 *  last and in the strong colour. */
export function tabTitleLabel(crumbs: readonly Crumb[], room = TAB_ROOM): HTMLElement {
  const m = measurer();
  const widths = crumbs.map((c) => m(c.label, TAB_PX));
  const fit = fitCrumbs(widths, room, m(SEP, TAB_PX) + 2 * SEP_PAD, () => m(MORE, TAB_PX));
  const out = document.createElement("span");
  out.className = "viz-lineage-tab";
  fit.forEach((f, i) => {
    if (i > 0) out.append(sep());
    if (f.kind === "more") {
      out.append(span("viz-lineage-tab-more", MORE));
      return;
    }
    const c = crumbs[f.index] as Crumb;
    const text = f.room === null ? c.label : (cutMiddle(c.label, f.room, (t) => m(t, TAB_PX), MIN_CHARS) ?? MORE);
    out.append(span(f.index === crumbs.length - 1 ? "viz-lineage-tab-here" : "viz-lineage-tab-up", text));
  });
  return out;
}

function sep(): HTMLElement {
  const s = span("viz-lineage-sep", SEP);
  s.setAttribute("aria-hidden", "true");
  return s;
}

function span(className: string, text: string): HTMLElement {
  const s = document.createElement("span");
  s.className = className;
  s.textContent = text;
  return s;
}

// ----- the At cursor section -----

/** The section for the field at `path`: one row per crumb, indented by how
 *  deep it is, the file first. Null while the reading is on its way. */
export function drawLineage(doc: Doc, path: readonly number[]): HTMLElement | null {
  const crumbs = fieldLineage(doc, path);
  if (crumbs === null || crumbs.length < 2) return null;
  const tabCrumbs = tabLineage(doc)?.length ?? 1;
  const host = document.createElement("figure");
  host.className = "viz-lineage";
  const head = document.createElement("figcaption");
  head.className = "viz-lineage-title";
  head.textContent = VIZ.lineage.title;
  const list = document.createElement("ol");
  list.className = "viz-lineage-list";
  // Nothing here is ever unhooked: the listeners are on the section's own
  // elements and go with them when the panel draws another field.
  const bus = linkedFor(doc);
  crumbs.forEach((c, i) => {
    const parent = i > 0 ? (crumbs[i - 1] as Crumb) : null;
    const here = i === tabCrumbs - 1;
    const li = document.createElement("li");
    li.className = "viz-lineage-row";
    li.style.setProperty("--depth", String(i));
    // The tab already showing is named in plain text, having nothing to bring
    // up; every other name is a button.
    const name = here ? span("viz-lineage-name is-here", c.label) : document.createElement("button");
    const nameAction = here ? null : c.step === null || isOpen(c) ? VIZ.lineage.goToTab(c.label) : VIZ.lineage.open(c.label);
    if (name instanceof HTMLButtonElement) {
      name.type = "button";
      name.className = "viz-lineage-name";
      name.textContent = c.label;
      // The tab it would be opened from has been closed.
      if (c.step !== null && !openers.has(c.from)) name.disabled = true;
      if (nameAction !== null) name.setAttribute("aria-label", nameAction);
      name.addEventListener("click", () => {
        hideTip();
        showCrumb(c);
      });
    }
    const top = document.createElement("div");
    top.className = "viz-lineage-top";
    top.append(name);
    if (here) top.append(span("viz-lineage-note", VIZ.lineage.thisTab));
    const how = document.createElement("div");
    how.className = "viz-lineage-how";
    let run: HTMLButtonElement | null = null;
    if (c.step !== null && !c.step.joined) {
      run = document.createElement("button");
      run.type = "button";
      run.className = "viz-lineage-at";
      run.textContent = formatOffset(c.step.atBits);
      if (parent !== null) {
        run.setAttribute("aria-label", VIZ.lineage.goToRun(parent.label));
        run.addEventListener("click", () => {
          hideTip();
          goToRun(c, parent);
        });
      } else run.disabled = true;
      if (isStored(c.step)) how.append(VIZ.lineage.storedBefore(c.step.inBits), run);
      else how.append(VIZ.lineage.unpackedBefore(c.step.outBits, c.step.inBits, c.step.codec), run);
    } else how.textContent = howText(c);
    li.append(top, how);
    list.append(li);

    const runAction = parent === null ? null : VIZ.lineage.goToRun(parent.label);
    hoverTip(li, (e) => {
      const t = e.target instanceof Element ? e.target : null;
      const action = t !== null && run !== null && run.contains(t) ? runAction : t !== null && name instanceof HTMLButtonElement && !name.disabled && name.contains(t) ? nameAction : null;
      return crumbCard(c, parent, action);
    });
    // A run of this tab's own bytes is marked in the hex view while the
    // pointer is on its row. Runs inside a stream, or of another tab, have
    // nothing here to mark.
    const marks = c.step !== null && c.from === doc && c.step.topLevel && c.step.inBits > 0;
    if (marks && c.step !== null) {
      const bits = { startBit: c.step.atBits, endBit: c.step.atBits + c.step.inBits };
      li.addEventListener("pointerenter", () => bus.publish({ hover: bits }));
      li.addEventListener("pointerleave", () => bus.publish({ hover: null }));
    }
  });
  host.append(head, list);
  return host;
}

/** The deepest stream root on `path`, as the section's key: moving between
 *  fields of one stream keeps the section as drawn. */
function deepestRoot(doc: Doc, path: readonly number[]): string {
  const steps = spaceSteps(path, nodeOf(doc));
  if (steps === null) return `pending|${path.join("/")}`;
  return steps[steps.length - 1]?.root.join("/") ?? "";
}

/** Which tabs are open, as part of the section's key: each name says whether
 *  its tab is open, and a name whose tab has closed cannot be followed. */
function openKey(): string {
  return routes
    .open()
    .map((d) => d.space)
    .join(",");
}

/** Put the section in the At cursor panel, for every field inside a stream
 *  and every field of a stream's tab. */
export function register(): void {
  registerInspectorSection({
    id: "lineage",
    applies: (doc, n) => doc.space !== 0 || n.space !== 0,
    key: (doc, _n, path) => `lineage|${doc.space}|${deepestRoot(doc, path)}|${openKey()}`,
    draw: ({ doc, path }) => drawLineage(doc, path),
  });
}

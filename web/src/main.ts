import { Doc, EditorMissing, bytesSource, formatBytes, formatOffset, glyphColumn, prefetchMagic, type DiagramBox, type MapStep } from "./doc.ts";
import * as nav from "./navhistory.ts";
import { HexView, isRightColumn, type BitRange, type RightColumn } from "./hexview.ts";
import type { LinkEnd, LinkPlan } from "./hexlinks.ts";
import type { GraphView } from "./graphview.ts";
import type { DiagramView } from "./diagramview.ts";
import type { ReportView } from "./reportview.ts";
import type { ReportHost } from "./report/section.ts";
import { RV } from "./report/text.ts";
import { COUNT_TURN_MS, countLimit } from "./diagramcounts.ts";
import { Inspector } from "./inspector.ts";
import { saveBlob, saveDoc, type SaveOutcome } from "./save.ts";
import { askSaveAs, type SaveAsChoice } from "./saveasdialog.ts";
import { FolderList } from "./folderview.ts";
import { parseSize, syntheticFile } from "./synthetic.ts";
import { ListingReport } from "./listingreport.ts";
import { ListPane } from "./listpane.ts";
import { TextView } from "./textview.ts";
import { StringsView, ENCODINGS, MIN_CHARS_DEFAULT, MIN_CHARS_KEY, ENCODINGS_KEY, HIDE_INSIDE_KEY } from "./stringsview.ts";
import { Crystal } from "./crystal.ts";
import { OverviewPanel } from "./overviewpanel.ts";
import { Tabs, type Page, type Tab } from "./tabs.ts";
import { isTable, tablePlan } from "./tableplan.ts";
import { TableView } from "./tableview.ts";
import { markFromRange, markFromStep, stepBits } from "./unpackedlink.ts";
import { SearchBar } from "./searchbar.ts";
import { el, svgEl } from "./dom.ts";
import { fileType, builtinTemplate, rememberKaitaiTitles, SIGNATURE_TEMPLATE, templateLabel, templateSentence, templateSignatureMismatch, templateTypeName } from "./filetype.ts";
import { DATASET_MEMBER, DIAGRAM, DUMP, EDITOR_WONT_LOAD, FOLDER, GRAPH, HEXGLYPHS, HEXPAT, HEXPAT_TEMPLATE, JOINED, KAITAI_TEMPLATE, KSY, LINKS, PAGE_OUT_OF_DATE, SAVE_AS, SETTINGS, strideSegment, STRINGSVIEW, TEXTVIEW, UNPACKED, unpackedOrigin, childWord, TABLE } from "./strings.ts";
import { CRC_AT_OPEN_MAX_BYTES, datasetIn, dropIsFolder, leafOf, missingFromDataset, orderForArchive, readDrop, readPicked, Stopped, storedZip, type BuiltZip, type Dropped, type FolderFile } from "./folderzip.ts";
import { ArchiveSums, SumJob } from "./sumjob.ts";
import { KsyPanel } from "./ksypanel.ts";
import { HexpatPanel } from "./hexpatpanel.ts";
import { reloadForStaleAssets, watchForStaleAssets } from "./staleassets.ts";
import {
  CODEPAGES_A,
  CODEPAGES_B,
  HEX_GLYPH_SETS,
  HEX_GLYPHS_ASCII,
  HEX_GLYPHS_DEFAULT,
  HEX_GLYPHS_KEY,
  SCREEN_GLYPHS,
  UNICODE_ENCODINGS,
} from "./encodings.ts";
import { rememberChoice, storedChoice, storedNumber, storedText } from "./stored.ts";
import { ASCII_GLYPHS } from "./hexcell.ts";
import { gearIcon } from "./icons.ts";
import { SettingsDialog, type ExtraTemplate } from "./settingsdialog.ts";
import { magicPattern, type TemplateEntry } from "./templatesearch.ts";
import { templateExtensions } from "./identity.ts";

const appEl = document.getElementById("app");
if (!appEl) throw new Error("missing #app");
const app: HTMLElement = appEl;

const formatSize = formatBytes;

/** The main views: one reading of the file at a time, in the same area. The
 *  graph is behind `?graph` and is not offered until it has been unlocked.
 *  `ksy` and `hexpat` are the two converters, which take the same area without
 *  being a reading of the file: they are tools, opened from the template
 *  menu. */
type View = "hex" | "listing" | "text" | "strings" | "graph" | "diagram" | "report" | "ksy" | "hexpat";

/** Whether the graph view is on offer. Set by `?graph` and kept, so the URL is
 *  needed once rather than every time. Read at startup, before any page is
 *  built, since the switch is built with the rest of the toolbar. */
const graphUnlocked = ((): boolean => {
  if (new URLSearchParams(location.search).has("graph")) rememberChoice("qubero.graph", "1");
  return storedText("qubero.graph") === "1";
})();

/**
 * The open documents. The first is a file the reader chose; the rest were
 * opened out of it: a compressed stream unpacked and read in its own right, a
 * decompressed zip entry, a field's run of bytes read as a file of its own.
 * One is showing at a time; the strip above the toolbar swaps between them,
 * and only appears once there are two. See `tabs.ts`.
 */
const tabs = new Tabs(app, build);
tabs.onEmpty = () => welcome();
tabs.onConfirmClose = (tab) => confirm(`Discard unsaved edits to ${tab.doc.name}?`);

function activeDoc(): Doc | null {
  return tabs.doc;
}

/**
 * What one tab lets the others do to it, so that the cursor in one can mark the
 * bytes it answers to in another. Filled in as each page is built and dropped
 * when the tab closes.
 */
type Link = {
  /** Mark the stretch the other tab's cursor answers to, or clear it. */
  mark: (startBit: number | null, endBit?: number) => void;
  /** Bring this tab to the front with the cursor at this bit. */
  goTo: (bit: number) => void;
};
const links = new Map<Tab, Link>();

/**
 * Which tab put the mark on which, and where in that tab to go back to.
 *
 * The bits of a mark are the bits of the tab it is drawn on, so they are no use
 * as a destination: the mark on the file is a stretch of the file, and following
 * it means going to the unpacked byte that stretch produced. That byte is
 * wherever the cursor was in the tab that set the mark, which is what is kept
 * here. Two streams open from one file also means the first of them is not
 * necessarily the one to go back to.
 */
const markedBy = new Map<Tab, { by: Tab; bit: number }>();

/** The tab showing the file every space was unpacked out of. */
function fileTab(): Tab | undefined {
  return tabs.all.find((t) => t.doc.isFile);
}

/**
 * The cursor moved in `from`. Mark what it answers to in every other tab.
 *
 * The two directions are different questions with different answers. From an
 * unpacked stream the question is which bits of the compressed run produced
 * this byte, which `mapOut` answers; from the file it is which unpacked bytes
 * these bits came to, which each open space answers for itself with `mapIn`.
 * Both are null until a codec keeps a trace, and a null mark is no mark.
 */
function linkCursor(from: Tab, bitOffset: number): MapStep | null {
  if (!from.doc.isFile) {
    const step = from.doc.mapOut(Math.floor(bitOffset / 8));
    const mark = markFromStep(step);
    const file = fileTab();
    if (file !== undefined) {
      links.get(file)?.mark(mark?.startBit ?? null, mark?.endBit);
      if (mark === null) markedBy.delete(file);
      else markedBy.set(file, { by: from, bit: bitOffset });
    }
    return step;
  }
  for (const t of tabs.all) {
    // A table has no bits of its own: its rows are the file's, and it is told
    // where the cursor is by `markTables` rather than by mapping between two
    // address spaces.
    if (t.doc.isFile || t.kind.view !== "document") continue;
    const mark = markFromRange(t.doc.mapIn(bitOffset));
    links.get(t)?.mark(mark?.startBit ?? null, mark?.endBit);
    if (mark === null) markedBy.delete(t);
    else markedBy.set(t, { by: from, bit: bitOffset });
  }
  return null;
}

/** Where a byte of an unpacked stream came from, in one line, or nothing when
 *  the codec kept no trace of that byte. */
function originLine(doc: Doc, step: MapStep | null): string {
  if (step === null) return "";
  const { start, end } = stepBits(step);
  return unpackedOrigin(doc.name, start, end, step.kind, step.len, step.dist, step.field);
}

/** Follow the mark on `tab` back to the tab that put it there. */
function followLink(tab: Tab): void {
  const home = markedBy.get(tab);
  if (home === undefined) return;
  const i = tabs.all.indexOf(home.by);
  if (i < 0) return;
  tabs.focus(i);
  links.get(home.by)?.goTo(home.bit);
}

/** The first open document with unsaved edits, which is what a replacement or
 *  a page close would throw away. */
function modifiedTab(): Tab | null {
  return tabs.modified();
}
/** Writes to the toolbar's message slot, once there is one. */
let say: (text: string, warn?: boolean) => void = () => {};

/** Where a dropped `.ksy` goes: the showing tab's converter. A `.ksy` says how
 *  to read a file rather than being one, so it opens the converter instead of
 *  taking the place of what is open. Null while no file is open, and set to a
 *  refusal for a tab of unpacked bytes, which no template of the reader's
 *  choosing reads. */
let dropKsy: ((text: string, name: string) => void) | null = null;
/** The same for a dropped `.hexpat` or `.pat`. */
let dropHexpat: ((text: string, name: string) => void) | null = null;

const DROP_TITLE = "Drop to open";
const discardMsg = (open: string, next: string): string =>
  `Discard unsaved edits to ${open} and open ${next}?`;
/** Says what letting go costs: the open file closes. Not "replaces", which
 *  during a drag reads as overwriting that file on disk, which never happens. */
const closesMsg = (): string => {
  const doc = activeDoc();
  if (doc === null) return "";
  const what = tabs.all.length > 1 ? `all ${tabs.all.length} open documents` : doc.name;
  return `Closes ${what}${modifiedTab() !== null ? " (unsaved edits)" : ""}`;
};
const selectedBytes = (n: number): string => (n === 1 ? "1 byte" : `${n.toLocaleString()} bytes`);

/** Above the parts in the rail, for the generated template: it marks the
 *  bytes that name the format and describes nothing else. */
const SIGNATURE_NOTE = "Signature only. This template marks the bytes that identify the format and nothing else.";


/** The chooser's value for the generated template. Not a built-in name, so it
 *  can never collide with one. */
const SIGNATURE_VALUE = "generated-signature";
const signatureOption = (name: string): string => (name === "" ? "Signature only" : `${name} (signature only)`);

/** What a bundled Kaitai format's template name starts with. */
const KAITAI_PREFIX = "ksy:";
/** The menu entry for the template a converted `.ksy` produced, added once one
 *  is in use. Neither value can collide with a built-in's name. */
const KSY_VALUE = "converted-ksy";
/** What a bundled ImHex pattern's template name starts with. */
const IMHEX_PREFIX = "hexpat:";
/** The menu entry for the template a converted ImHex pattern produced. */
const HEXPAT_VALUE = "converted-hexpat";

/**
 * Open a file from disk, in place of everything already open. Unsaved edits
 * live only in this page, so replacing a document that has them asks first.
 */
let welcomeCrystal: Crystal | null = null;
let welcomeStatus: HTMLElement | null = null;

function openFile(f: File, note?: string): void {
  const edited = modifiedTab();
  if (edited !== null && !confirm(discardMsg(edited.doc.name, f.name))) return;
  welcomeCrystal?.setBusy(true);
  if (welcomeStatus !== null) welcomeStatus.textContent = `Opening ${f.name}…`;
  // Let the opening screen paint its loading state before starting WASM work.
  void new Promise<void>(resolve => requestAnimationFrame(() => setTimeout(resolve, 0))).then(() => Doc.open(f)).then((doc) => {
    welcomeCrystal?.dispose();
    welcomeCrystal = null;
    welcomeStatus = null;
    hideFolderList();
    folder = null;
    mount(doc);
    if (note !== undefined) say(note);
  }).catch(openFailed);
}

/**
 * Say why a file did not open, and answer the one reason there is an answer
 * to.
 *
 * The editor's own code being missing means, after a deployment, that this
 * page is naming files that were replaced: asking for the page again finds the
 * new ones. A tab that has already asked gets the message instead, since a
 * page that reloads forever takes the tab away from the reader before they can
 * read what went wrong.
 *
 * Every way of opening a file goes through here, and that is the point: a
 * dropped file, a `?url=`, and a synthetic one all fail the same way when the
 * editor will not load, and one of them silently doing nothing is how this was
 * found.
 */
function openFailed(error: unknown): void {
  if (error instanceof EditorMissing && reloadForStaleAssets()) {
    if (welcomeStatus !== null) welcomeStatus.textContent = PAGE_OUT_OF_DATE;
    else say(PAGE_OUT_OF_DATE, true);
    return;
  }
  welcomeCrystal?.setBusy(false);
  const message =
    error instanceof EditorMissing
      ? EDITOR_WONT_LOAD
      : error instanceof Error
        ? error.message
        : "Could not open this file.";
  if (welcomeStatus !== null) welcomeStatus.textContent = message;
  else say(message, true);
  console.error("open", error);
}

/** Open bytes lifted out of the showing document as a tab of their own. */
function openEmbedded(bytes: Uint8Array, name: string, origin: string): void {
  void Doc.open(bytesSource(bytes, name)).then((doc) => {
    tabs.add({ doc, title: name, origin });
  });
}

/** A section that can be folded down to its title bar, and remembers whether
 *  it was. */
function panel(title: string, content: HTMLElement, onToggle: () => void): HTMLElement {
  const key = "qubero.panel.right";
  const chevron = el("span", { className: "panel-chevron" });
  const toggle = el("button", { type: "button", className: "panel-toggle" }, chevron, title);
  const section = el("section", { className: "panel panel-right" }, el("header", { className: "panel-bar" }, toggle), content);
  const apply = (collapsed: boolean): void => {
    section.classList.toggle("is-collapsed", collapsed);
    chevron.textContent = collapsed ? "\u25c2" : "\u25be";
    toggle.setAttribute("aria-expanded", String(!collapsed));
    toggle.title = collapsed ? "Expand" : "Collapse";
  };
  apply(storedText(key) === "collapsed");
  toggle.addEventListener("click", () => {
    const collapsed = !section.classList.contains("is-collapsed");
    rememberChoice(key, collapsed ? "collapsed" : "open");
    apply(collapsed);
    onToggle();
  });
  return section;
}

/** Show this file on its own, closing whatever was open. */
function mount(doc: Doc): void {
  tabs.only({ doc, title: doc.name });
}

/** Build the page a tab wants: the whole of a document, or one list inside
 *  one read as a table. */
function build(tab: Tab): Page {
  const kind = tab.kind;
  return kind.view === "table" ? buildTable(tab, kind.path) : buildDocument(tab);
}

/**
 * The tab a table opens in: the view and nothing else.
 *
 * No toolbar and no status bar, because neither would have anything to say
 * here. A table is a reading of somebody else's bytes, so every control that
 * is about the file -- the template, the encoding, saving -- belongs to the
 * document tab this one was opened from, and the two are tied together by the
 * cursor rather than by a second copy of the controls.
 */
function buildTable(tab: Tab, path: readonly number[]): Page {
  const doc = tab.doc;
  const page = el("div", { className: "tabpage tabpage-table" });
  const reply = doc.templateNode(path);
  const plan = reply.status === "ok" ? tablePlan(doc, reply.node) : null;
  if (reply.status !== "ok" || plan === null) {
    // The field is gone, or the template that named it is. Nothing to draw and
    // nothing to fix from in here: the reader closes the tab.
    page.append(el("p", { className: "rp-empty", textContent: TABLE.gone }));
    return { el: page, shown: () => {} };
  }
  const view = new TableView(doc, plan, { title: reply.node.name });
  page.append(view.el);
  /** The document tab these rows are stored in, which is where a pick goes. */
  const home = (): Tab | undefined => tabs.all.find((t) => t.kind.view === "document" && t.doc === doc);
  const goHome = (startBit: number, endBit: number): void => {
    const at = home();
    if (at === undefined) return;
    links.get(at)?.goTo(startBit);
    links.get(at)?.mark(startBit, endBit);
  };
  view.onPick = ({ startBit, endBit }) => goHome(startBit, endBit);
  // A fact, or a link in a cell, names another field: the cursor goes to it
  // the same way picking a row goes to the row's own bytes.
  view.onFactPick = (at) => {
    const n = doc.templateNode(at);
    if (n.status !== "ok") return;
    goHome(n.node.offset_bits, n.node.offset_bits + n.node.size_bits);
  };
  // What the document tab may do to this one: put the selection on the row its
  // cursor is in, and bring the table up with a row selected.
  links.set(tab, {
    mark: (startBit) => {
      if (startBit === null) view.clearSelection();
      else view.setBit(startBit);
    },
    goTo: (bit) => {
      view.setBit(bit);
      view.el.focus();
    },
  });
  tab.release.push(() => links.delete(tab));
  return {
    el: page,
    shown: () => {
      // The status slot is the document page's. Nothing here writes to it, so
      // it is left saying nothing rather than saying whatever the page before
      // this one left in it.
      say = () => {};
      dropKsy = () => {};
      dropHexpat = () => {};
      view.shown();
    },
  };
}

/** Every table open on one document follows its cursor: a cursor inside the
 *  table selects the row holding it, and one outside leaves the table alone. */
function markTables(doc: Doc, bitOffset: number): void {
  for (const t of tabs.all) if (t.kind.view === "table" && t.doc === doc) links.get(t)?.mark(bitOffset);
}

/**
 * Build the page for one tab: the toolbar, the three views, the rail, the
 * inspector and the status bar, all over that tab's document.
 *
 * Every open tab's page stays in the document, so anything registered on
 * `document` rather than inside the page has to ask whether this tab is the one
 * showing before it acts, and has to come off again when the tab closes. `key`
 * is how a page does both.
 */
function buildDocument(tab: Tab): Page {
  const doc = tab.doc;
  const key = (fn: (e: KeyboardEvent) => void): void => {
    const on = (e: KeyboardEvent): void => {
      if (tabs.showing(tab)) fn(e);
    };
    document.addEventListener("keydown", on);
    tab.release.push(() => document.removeEventListener("keydown", on));
  };
  // A change to an unpacked stream has nowhere to go, so every way of asking
  // for one is refused and says so where the other messages go.
  doc.onRefuseEdit = (why) => say(why, true);
  const view = new HexView(doc);
  const inspector = new Inspector(doc);
  const structure = new ListingReport(doc);
  // One long list, read on its own beside the listing. Empty and hidden until
  // a list is opened in it.
  const listPane = new ListPane(doc);
  // The listing and the pane share a row: the pane takes its half only while
  // a list is open in it, so the listing has the whole width until then.
  const listRow = el("div", { className: "listrow" }, structure.el, listPane.el);
  // The file as the text it is, for the files that were written to be read.
  const text = new TextView(doc);
  // The other reading of a file: not the text it is, but the text inside it.
  const strings = new StringsView(doc);
  // A text file that turns out to be a dump of another file. The offer sits
  // above the views rather than inside one, because it is a fact about the
  // whole file and not about a field or a place in it.
  const dumpBar = el("div", { className: "dumpbar" });
  dumpBar.hidden = true;
  void doc.dumpScan().then((scan) => {
    if (scan === null) return;
    const open = el("button", { type: "button", className: "dumpbar-open", textContent: DUMP.open });
    open.addEventListener("click", () => {
      open.disabled = true;
      void doc.dumpBytes().then((bytes) => {
        open.disabled = false;
        if (bytes.length === 0) return;
        const named = scan.names[0];
        const name = named === undefined ? DUMP.fallbackName(doc.name) : named.replace(/^.*[\\/]/, "");
        openEmbedded(bytes, name, DUMP.origin(doc.name, scan.tool));
      });
    });
    const facts: HTMLElement[] = [el("strong", { textContent: DUMP.heading })];
    facts.push(el("span", { textContent: DUMP.summary(scan.tool, scan.covered) }));
    if (scan.from > 0) facts.push(el("span", { textContent: DUMP.startsAt(scan.from) }));
    if (scan.holes.length > 0) {
      const mark = el("span", { className: "is-warn", textContent: DUMP.holes(scan.holes.length / 2) });
      mark.title = DUMP.holesTitle;
      facts.push(mark);
    }
    if (scan.conflicts.length > 0)
      facts.push(el("span", { className: "is-warn", textContent: DUMP.conflicts(scan.conflicts.length) }));
    dumpBar.replaceChildren(el("div", { className: "dumpbar-facts" }, ...facts), open);
    dumpBar.hidden = false;
  });
  // One file of a BP5 dataset, which reads less by itself than with the rest
  // of its folder: said above the views, with the way to open the folder. A
  // file lifted out of a folder already open says where it reads instead.
  const memberBar = el("div", { className: "dumpbar" });
  memberBar.hidden = true;
  // Sums still being taken for an archive no one can save any more are not
  // worth reading the folder for.
  const sums = doc.isFile ? doc.archiveSums : null;
  if (sums !== null) tab.release.push(() => sums.job.stop());
  const showMember = (template: string | null): void => {
    const fact = template === null ? undefined : DATASET_MEMBER.facts[template];
    if (fact === undefined) return;
    const facts = el("div", { className: "dumpbar-facts" }, el("strong", { textContent: DATASET_MEMBER.heading }));
    if (doc.isFile) {
      facts.append(el("span", { textContent: fact }));
      const open = el("button", { type: "button", className: "dumpbar-open", textContent: DATASET_MEMBER.open, title: DATASET_MEMBER.openTitle });
      open.addEventListener("click", () => pickFolder(doc.name));
      memberBar.replaceChildren(facts, open);
    } else {
      const from = fileTab()?.title ?? doc.name;
      facts.append(el("span", { textContent: DATASET_MEMBER.lifted(from) }));
      memberBar.replaceChildren(facts);
    }
    memberBar.hidden = false;
  };
  const overview = new OverviewPanel(doc);
  const search = new SearchBar(doc);
  // The views share one position: the hex cursor. Picking a field moves
  // it; moving it picks the field it lands in. `picking` stops that going round.
  let picking = false;
  let followWhenLoaded: number | null = null;
  let followedBit: number | null = null;

  /** Which field the arrows are drawn for. The cursor's field, or a field
   *  picked by name in the listing, which is the same thing every other panel
   *  is showing. */
  let linkPath: readonly number[] | null = null;

  /**
   * Work out what the overlay should draw for the field the panels are on.
   *
   * The core is asked the same question the sidebar asks: which fields settled
   * this one's shape. Nothing new is inferred here, so an arrow can never say
   * something the properties list does not.
   *
   * Only fields whose offsets are bits of the file are drawn. A field read out
   * of a compressed stream is at an offset of that stream, and pointing at the
   * byte of the file with the same number would point at some other field.
   */
  const refreshLinks = (): void => {
    if (!view.links.enabled) return;
    const path = linkPath;
    if (path === null || !inFile(path)) {
      view.links.setPlan({ target: null, parent: null, from: [] });
      view.render();
      return;
    }
    const self = doc.templateNode(path);
    const target =
      self.status === "ok"
        ? { startBit: self.node.offset_bits, endBit: self.node.offset_bits + self.node.size_bits }
        : null;
    // The structure the field is part of, outlined so that a length four rows
    // up reads as a length of this record rather than of the file. The root is
    // not a structure the reader can see the edges of, so it is left out.
    let parent: LinkPlan["parent"] = null;
    if (path.length > 0) {
      const up = doc.templateNode(path.slice(0, -1));
      if (up.status === "ok" && up.node.size_bits > 0) {
        parent = {
          startBit: up.node.offset_bits,
          endBit: up.node.offset_bits + up.node.size_bits,
          name: up.node.name,
        };
      }
    }
    // Every step of the path is asked, not only the field itself, for the same
    // reason the sidebar asks: 128 bytes of packed weights are 128 bytes
    // because of a record three levels up, and that record is the answer. The
    // walk stops at the nearest step above the field that has anything to say,
    // which is what the sidebar shows without being unfolded. Past that the
    // grid would carry arrows the panel beside it has folded away.
    const from: LinkEnd[] = [];
    const seen = new Set<string>();
    for (let i = path.length; i >= 0 && from.length === 0; i--) {
      const at = path.slice(0, i);
      const step = doc.templateNode(at);
      if (step.status !== "ok") continue;
      const reply = doc.origins(at);
      if (reply.status !== "ok") continue;
      for (const o of reply.node) {
        // A `points` row is the other direction and names no field at its far
        // end: the arrow would leave from the field it is already drawn on.
        if (o.role === "points" || o.path.length === 0) continue;
        const key = `${o.role}/${o.path.join("/")}`;
        if (seen.has(key)) continue;
        seen.add(key);
        if (!inFile(o.path)) continue;
        const n = doc.templateNode(o.path);
        if (n.status !== "ok") continue;
        from.push({
          role: o.role,
          label: o.label,
          startBit: n.node.offset_bits,
          endBit: n.node.offset_bits + n.node.size_bits,
          decidesBit: step.node.offset_bits,
        });
      }
    }
    view.links.setPlan({ target, parent, from });
    view.render();
  };

  const followCursor = (bitOffset: number): void => {
    if (doc.template === null) return;
    // Only an actual move picks a field, so Escape can clear the highlight
    // without the cursor event putting it straight back.
    if (bitOffset === followedBit) return;
    followedBit = bitOffset;
    const at = doc.locate(bitOffset);
    if (at.status === "pending" || at.status === "working") {
      followWhenLoaded = bitOffset;
      return;
    }
    followWhenLoaded = null;
    if (at.status !== "ok") return;
    const n = doc.templateNode(at.node);
    if (n.status === "ok") {
      view.setHighlight({ startBit: n.node.offset_bits, endBit: n.node.offset_bits + n.node.size_bits });
    }
    linkPath = at.node;
    refreshLinks();
    // Inside the part the graph was drawn for, the cursor only marks a node.
    // Outside it, the graph is about somewhere else and is drawn again, which
    // is what the note in the view promises when it says to put the cursor in
    // a smaller part of the file.
    if (graph !== null && !graph.el.hidden) {
      if (sameRoot(graphRoot, graphRootFor(bitOffset))) graph.setPath(at.node);
      else void showGraph();
    }
    overview.reveal(at.node);
    structure.setBit(bitOffset);
    listPane.setBit(bitOffset);
  };

  /** `throughBit` marks a stretch rather than one field: the end of the run a
   *  folded chip stands for. The cursor and the inspector still go to the
   *  field the path names, which is the first of the run and the byte the chip
   *  sits over; what widens is the mark over the bytes. `atBit` is where a
   *  pressed chip sits when that is not the field's first bit: the rest of a
   *  structure after its last child, or a field carried in from above the
   *  view. */
  const goToField = (path: readonly number[], throughBit?: number, atBit?: number): void => {
    const n = doc.templateNode(path);
    if (n.status !== "ok") return;
    const end = n.node.offset_bits + n.node.size_bits;
    view.setHighlight({ startBit: n.node.offset_bits, endBit: Math.max(end, throughBit ?? end) });
    picking = true;
    view.setBitCursor(atBit ?? n.node.offset_bits, { pane: "hex" });
    picking = false;
    inspector.setPath(path);
    linkPath = path;
    refreshLinks();
  };

  /** Whether a field's offsets are bits of the file, so the hex view and the
   *  overview can follow them. A field read out of a compressed stream is at
   *  an offset of that stream: moving the cursor there would land on the byte
   *  of the file with the same number, which is some other field entirely. */
  const inFile = (path: readonly number[]): boolean => {
    const n = doc.templateNode(path);
    return n.status !== "ok" || n.node.space === 0;
  };

  structure.onPick = ({ path, startBit, endBit }) => {
    // The inspector always follows: it can say what a decoded field holds and
    // which stream it came out of. The hex view and the overview cannot, and
    // are left where they are rather than sent somewhere wrong.
    if (inFile(path)) {
      view.setHighlight({ startBit, endBit });
      picking = true;
      view.setBitCursor(startBit, { pane: "hex" });
      picking = false;
      overview.reveal(path);
      linkPath = path;
      refreshLinks();
    }
    inspector.setPath(path);
  };
  // Following an offset: put the cursor where it points, and let the views
  // catch up the same way they do for a search hit.
  const goToBit = (bitOffset: number): void => {
    view.setBitCursor(bitOffset, { pane: "hex" });
    inspector.setOffset(bitOffset);
    followCursor(bitOffset);
  };
  // Landing on a run of bits marks them, so that a four-bit weight shows as
  // four bits and not as the byte it shares. The mark goes on after
  // `followCursor`, whose own mark is the whole field the cursor landed in.
  const showBits = (bitOffset: number, ranges?: readonly BitRange[]): void => {
    goToBit(bitOffset);
    if (ranges !== undefined && ranges.length > 0) view.setHighlight(ranges);
  };
  // The same move, with the place being left kept so that Back returns to it.
  // Going back calls `showBits` instead, which records nothing: retracing a
  // step is not a step of its own.
  const jumpToBit = (bitOffset: number, ranges?: readonly BitRange[]): void => {
    nav.recordJump(view.cursorState.bitOffset, bitOffset, ranges);
    showBits(bitOffset, ranges);
  };
  nav.startFile(view.cursorState.bitOffset);
  nav.onGo(showBits);
  overview.onGoTo = jumpToBit;
  inspector.onGoTo = jumpToBit;
  inspector.onPick = (path) => {
    const n = doc.templateNode(path);
    // Following a crumb into a stream moves the listing to it and nothing
    // else: there is no bit of the file to put the cursor on.
    if (!inFile(path)) {
      structure.reveal(path);
      return;
    }
    if (n.status === "ok") nav.recordJump(view.cursorState.bitOffset, n.node.offset_bits);
    goToField(path);
    overview.reveal(path);
    structure.reveal(path);
  };
  // Pointing at a row in the properties list marks that field over the bytes.
  // The sidebar names the field and the grid says where it is; between them
  // that is the whole answer, and neither has to be clicked for it.
  inspector.onHoverField = (path) => {
    if (path === null || !inFile(path)) {
      view.markHover(null);
      return;
    }
    const n = doc.templateNode(path);
    if (n.status !== "ok") {
      view.markHover(null);
      return;
    }
    view.markHover({ startBit: n.node.offset_bits, endBit: n.node.offset_bits + n.node.size_bits });
  };
  inspector.onOpenTab = openEmbedded;
  /**
   * Open a compressed run as a document of its own, or bring its tab to the
   * front if it is already open. The run is unpacked once and kept, so a second
   * ask costs nothing.
   */
  const openUnpacked = (path: readonly number[]): void => {
    const n = doc.templateNode(path);
    if (n.status !== "ok") return;
    const unpacked = doc.openSpace(path);
    if (unpacked === null) return;
    // A compressed run that will not open says why on its own row and is not
    // offered. A joined stream is offered while its length is under the cap,
    // and a part that will not read is found only now, so this is where it is
    // said, for the listing's button and the inspector's alike.
    if ("refused" in unpacked) {
      const first = doc.templateNode([...path, 0]);
      const joined = first.status === "ok" && first.node.joined && first.node.space_root;
      if (joined) say(unpacked.refused === "too-large" ? JOINED.tooLarge : JOINED.failed(JOINED.tabTitle(n.node.name, doc.name)), true);
      return;
    }
    const already = tabs.forSpace(unpacked.space);
    if (already >= 0) {
      tabs.focus(already);
      return;
    }
    tabs.add({
      doc: unpacked,
      title: unpacked.joined ? JOINED.tabTitle(n.node.name, doc.name) : UNPACKED.tabTitle(n.node.name, doc.name),
      origin: UNPACKED.openTitle(n.node.name),
    });
  };
  structure.onOpenUnpacked = openUnpacked;
  inspector.onOpenUnpacked = openUnpacked;
  strings.onOpenUnpacked = openUnpacked;
  /** Open one list as a table of its own, or bring the table already open on
   *  it to the front. The rows are read where they are asked for, so a second
   *  ask for the same list is the same tab rather than the same work twice. */
  const openTable = (path: readonly number[]): void => {
    const n = doc.templateNode(path);
    if (n.status !== "ok") return;
    const already = tabs.forTable(doc, path);
    if (already >= 0) {
      tabs.focus(already);
      return;
    }
    const rows = tablePlan(doc, n.node)?.rowWord ?? childWord(n.node);
    tabs.add({
      doc,
      title: TABLE.tabTitle(n.node.name, doc.name),
      origin: TABLE.tabTooltip(rows, n.node.name, doc.name),
      kind: { view: "table", path },
    });
  };
  structure.onOpenTable = openTable;
  inspector.onOpenTable = openTable;
  view.onOpenTable = openTable;
  // Whether a run of bytes reads as a table, for the chips beside them: the
  // view draws the mark, and what it means is worked out here.
  view.tableAt = (path) => {
    const n = doc.templateNode(path);
    return n.status === "ok" && isTable(doc, n.node);
  };
  // And from the bytes themselves: a chip marked as holding a file opens it on
  // a second press, which is what a second press means on every other picture
  // in this app.
  view.onOpenUnpacked = openUnpacked;
  view.onPickField = (path, throughBit, atBit) => {
    goToField(path, throughBit, atBit);
    overview.reveal(path);
  };
  // What the other tabs may do to this one: mark the stretch their cursor
  // answers to, and send the cursor there when the reader clicks that mark.
  links.set(tab, {
    mark: (startBit, endBit) => view.setLinkedRange(startBit, endBit ?? 0),
    goTo: (bit) => {
      view.setBitCursor(bit, { pane: "hex" });
      view.el.focus();
    },
  });
  tab.release.push(() => {
    links.delete(tab);
    markedBy.delete(tab);
    for (const [on, home] of markedBy) if (home.by === tab) markedBy.delete(on);
  });
  // Clicking the mark goes to the tab it came from: from an unpacked stream to
  // the compressed bits it was made of, and back the other way.
  view.onLinkedPick = () => followLink(tab);
  view.onHighlightClear = () => {
    followedBit = null;
    overview.clearSelection();
    structure.clearSelection();
  };

  const kind = fileType();
  // The overview shows the same name the toolbar does, whatever named it.
  kind.onIdentity = (name) => overview.setIdentity(name);
  // Every template the chooser offers, with what the search looks for in each:
  // a built-in's extensions, and a bundled Kaitai format's own extensions and
  // magic bytes.
  const choices = doc.templateChoices;
  rememberKaitaiTitles(choices);
  const templateEntries: TemplateEntry[] = choices.map((c) => {
    const magic = c.source === "builtin" ? null : magicPattern(c.magic ?? []);
    return {
      value: c.name,
      label: templateLabel(c.name),
      kind: c.source,
      ext: c.source === "builtin" ? templateExtensions(c.name) : [...(c.ext ?? [])],
      sigs: magic === null ? [] : [magic],
    };
  });
  /** The templates that are not in the list above: the file's own signature
   *  template, and a converted `.ksy`, each once there is one. */
  let extraTemplates: ExtraTemplate[] = [];
  const setExtra = (x: ExtraTemplate): void => {
    extraTemplates = [...extraTemplates.filter((e) => e.kind !== x.kind), x];
    settings?.setExtras(extraTemplates);
    if (tmplValue === x.value) setTemplateValue(x.value);
  };
  const dropExtra = (kind: ExtraTemplate["kind"]): void => {
    extraTemplates = extraTemplates.filter((e) => e.kind !== kind);
    settings?.setExtras(extraTemplates);
  };
  /** The settings dialog, built once the hex view's controls it holds are. */
  let settings: SettingsDialog | null = null;
  /** Which template the chooser says is reading the file. */
  let tmplValue = "";
  // The toolbar names the template and opens the chooser: it is a button, not
  // a menu, since what it opens is the dialog.
  const tmpl = el("button", { type: "button", className: "tb-tmpl", title: SETTINGS.chipTitle });
  const setTemplateValue = (value: string): void => {
    tmplValue = value;
    tmpl.dataset.template = value;
    const name = value === "" ? null : (extraTemplates.find((x) => x.value === value)?.label ?? templateLabel(value));
    tmpl.textContent = name === null ? SETTINGS.chipNone : SETTINGS.chip(name);
    tmpl.title = `${tmpl.textContent}. ${SETTINGS.chipTitle}`;
    settings?.setCurrent(value, name);
  };
  setTemplateValue("");
  tmpl.addEventListener("click", () => settings?.open());
  // The generated template is not one of the built-ins, so switching back to it
  // rebuilds it rather than looking it up by name.
  let reapplySignature: (() => Promise<void>) | null = null;
  /** What the toolbar says about the template now reading the file. Both ways
   *  in go through this: a template picked from the menu changes the answer
   *  exactly as much as one Qubero sniffed, and picking the wrong one is the
   *  case where saying so matters most. */
  const sayTemplate = (name: string, label: string): void => {
    kind.setTemplate({ name, label, sentence: templateSentence(doc, name), signatureMismatch: templateSignatureMismatch(doc) });
    kind.setNote(builtinTemplate(name));
  };

  const chooseTemplate = (value: string): void => {
    setTemplateValue(value);
    overview.setNote("");
    if (value === SIGNATURE_VALUE) {
      void reapplySignature?.();
      return;
    }
    if (value === KSY_VALUE) {
      applyKsy();
      return;
    }
    if (value === HEXPAT_VALUE) {
      applyHexpat();
      return;
    }
    doc.setTemplate(value === "" ? null : value);
    // The toolbar answered for the template Qubero sniffed and then said
    // nothing when the reader picked another, so the name above the file was
    // the old one until the page was reloaded.
    if (value === "") {
      kind.setTemplate(null);
      kind.setNote(null);
    } else {
      sayTemplate(value, extraTemplates.find((x) => x.value === value)?.label ?? templateTypeName(value));
    }
    // A bundled Kaitai format says where it came from, and says so again with
    // a count when its description holds things the template does not: those
    // fields are missing or read another way, and a reader who is not told
    // has no way to know which.
    if (value.startsWith(KAITAI_PREFIX)) showKaitaiNote(value);
    else if (value.startsWith(IMHEX_PREFIX)) showImhexNote(value);
    // Picking a template is asking to read fields, so the panel goes back to
    // them. It is left on the raw reading only for a file that has none.
    if (value !== "") inspector.setMode("structure");
  };
  // An unpacked stream was named by the stream that held it, so none of the
  // work below applies: nothing sniffs it, nothing identifies it, and its
  // template is not one of the menu's.
  // Nothing here says what the unpacked bytes are. The template reading them is
  // the one the stream was declared with, and naming the tab after it would
  // claim the unpacked bytes are another Zstandard stream. Until the unpacked
  // bytes are recognised in their own right, the tab's own title is the only
  // honest answer, and it is already above.
  if (!doc.isFile) {
    structure.setMatched(doc.template !== null);
    showMember(doc.template);
  } else
  void doc.sniffTemplate().then(async (name) => {
    const templated = name !== null;
    structure.setMatched(templated);
    if (name !== null) {
      setTemplateValue(name);
      doc.setTemplate(name);
      showMember(name);
      // A file recognised as a bundled Kaitai format says so as much as one
      // picked from the menu does, and offers the same way to the description.
      if (name.startsWith(KAITAI_PREFIX)) showKaitaiNote(name);
      // The template's answer goes up at once: it is the one source that
      // has read the file, and the one that answers before anything else.
      sayTemplate(name, templateTypeName(name));
    } else {
      // Nothing to read a field from, so start on the raw reading instead.
      inspector.setMode("le");
    }
    // The rules describe a file in a sentence, and a sentence saying a PNG is
    // 1280 by 720 and 8-bit RGBA is worth having whether or not a template
    // covers the format. Only a file without one waits on it, though: a
    // templated file is already readable, so it says nothing until it knows.
    const waiting = templated
      ? null
      : setTimeout(() => {
          kind.identifying();
        }, 300);
    try {
      const id = await doc.identify();
      // Stop the timer the moment there is an answer, and before the rule file
      // is fetched for the template: that second wait must not be able to
      // write "identifying" over a name already on screen.
      if (waiting !== null) clearTimeout(waiting);
      // Which answer names the file is decided in one place, from every
      // source that has spoken: see identity.ts.
      kind.setFile(id);
      void kind.addMatches(doc);
      if (id === null || name !== null) return;
      // The rule that named the format also says where its signature is. That
      // is one field, but it is a field: clickable, highlighted, and true.
      const signature = await doc.signatureTemplate(id);
      if (signature === null) return;
      setExtra({ value: SIGNATURE_VALUE, label: signatureOption(signature), kind: "signature" });
      setTemplateValue(SIGNATURE_VALUE);
      overview.setNote(SIGNATURE_NOTE);
      kind.setNote(SIGNATURE_TEMPLATE);
      reapplySignature = async (): Promise<void> => {
        await doc.signatureTemplate(id);
        overview.setNote(SIGNATURE_NOTE);
      };
    } catch (e) {
      console.error("identify", e);
      kind.failed();
    } finally {
      if (waiting !== null) clearTimeout(waiting);
    }
  });

  const fileLabel = el("span", { className: "tb-file" });
  // Where the tab came from, as its tab says it; a page opened alone has no
  // tab strip to say it on.
  if (tab.origin !== null) fileLabel.title = tab.origin;
  const posLabel = el("span", { className: "tb-pos" });
  // Icons, not words, and only once there is an edit to go back over: two
  // buttons that do nothing yet are room taken from the controls that do.
  const historyIcon = (flip: boolean): SVGSVGElement => {
    const svg = svgEl("svg", { viewBox: "0 0 16 16", width: "16", height: "16", "aria-hidden": "true", class: "history-icon" });
    if (flip) svg.style.transform = "scaleX(-1)";
    svg.append(
      svgEl("path", { d: "M5.5 3.5 2.5 6.5l3 3", fill: "none", stroke: "currentColor", "stroke-width": "1.6", "stroke-linecap": "round", "stroke-linejoin": "round" }),
      svgEl("path", { d: "M2.5 6.5h7a4 4 0 0 1 0 8H7", fill: "none", stroke: "currentColor", "stroke-width": "1.6", "stroke-linecap": "round" }),
    );
    return svg;
  };
  const undoBtn = el("button", { type: "button", title: "Undo (Ctrl+Z)", className: "icon-btn" }, historyIcon(false));
  const redoBtn = el("button", { type: "button", title: "Redo (Ctrl+Y)", className: "icon-btn" }, historyIcon(true));
  undoBtn.setAttribute("aria-label", "Undo");
  redoBtn.setAttribute("aria-label", "Redo");
  const saveBtn = el("button", { type: "button", textContent: "Save as", title: "Save as a new file (Ctrl+S)" });
  const saveMsg = el("span", { className: "tb-msg" });
  saveMsg.setAttribute("role", "status");
  say = (text, warn) => {
    saveMsg.textContent = text;
    saveMsg.classList.toggle("warn", warn === true);
  };
  if (folderDocs.has(doc)) saveBtn.title = SAVE_AS.buttonTitle;
  const save = async (): Promise<void> => {
    if (saveBtn.disabled) return;
    // What came from a folder asks first whether to save it or a ZIP of the
    // folder. Anything else saves as it always has.
    const built = builtFor.get(doc);
    const from = folderDocs.has(doc) ? folder : null;
    let choice: SaveAsChoice = { whole: false };
    if (built !== undefined || (from !== null && from.at !== null)) {
      const files = built?.files ?? from?.files ?? [];
      const total = files.reduce((n, f) => n + f.file.size, 0);
      const asked = await askSaveAs({
        file: built !== undefined ? null : { name: doc.name, size: formatSize(doc.lengthBytes) },
        zip: from?.zip ?? doc.name,
        files: FOLDER.files(files.length),
        size: formatSize(total),
        edited: doc.modified ? doc.name : null,
        sums: built !== undefined ? (doc.archiveSums?.job ?? null) : null,
        formatSize,
      });
      if (asked === null) return;
      choice = asked;
    }
    saveBtn.disabled = true;
    saveMsg.textContent = "Saving";
    let unwatch = (): void => {};
    let r: SaveOutcome;
    let zipName: string | null = null;
    if (built !== undefined) {
      // A dataset saves as the ZIP it is read from. A large one opened without
      // its CRC-32s waits for any still being read, saying how far they are.
      zipName = doc.name;
      const sums = doc.archiveSums;
      const source = async (): Promise<Blob> => {
        if (sums === null) return built.blob;
        const job = sums.job;
        const progress = (): void => {
          if (!job.finished) saveMsg.textContent = SAVE_AS.progress(doc.name, formatSize(job.read), formatSize(job.total));
        };
        progress();
        unwatch = job.onChange(progress);
        try {
          return await sums.summed();
        } finally {
          unwatch();
          saveMsg.textContent = "Saving";
        }
      };
      r = await saveDoc(doc, source);
    } else if (choice.whole && from !== null) {
      // The whole folder, with this file as it is now rather than as it is on
      // disk.
      zipName = from.zip;
      const at = from.at;
      r = await saveBlob(from.zip, async () => {
        const edited = doc.modified ? await doc.buildOutput() : null;
        const files = from.files.map((f, i) => (i === at && edited !== null ? { ...f, file: edited } : f));
        const zip = await storedZip(orderForArchive(files), {
          progress: (p) => (saveMsg.textContent = SAVE_AS.progress(from.zip, formatSize(p.done), formatSize(p.total))),
        });
        return zip.blob;
      });
    } else {
      r = await saveDoc(doc);
    }
    unwatch();
    saveBtn.disabled = false;
    const saved = (bytes: number): string =>
      zipName === null ? `Saved ${formatSize(bytes)}` : SAVE_AS.done(zipName, formatSize(bytes));
    saveMsg.textContent = r.kind === "saved" ? saved(r.bytes) : r.kind === "cancelled" ? "" : `Save failed: ${r.message}`;
    saveMsg.classList.toggle("warn", r.kind === "failed");
  };
  saveBtn.addEventListener("click", () => void save());
  key((e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
    }
  });
  undoBtn.addEventListener("click", () => doc.undo());
  redoBtn.addEventListener("click", () => doc.redo());

  const goto = el("input", { type: "text", placeholder: "Go to offset (hex)", className: "tb-goto" });
  goto.setAttribute("aria-label", "Go to offset, hexadecimal");
  goto.addEventListener("keydown", (e) => {
    if (e.key !== "Enter") return;
    // An address copied off the screen arrives as `@0x40`, so the box takes
    // the mark off before it takes the `0x` off. Neither is required: what a
    // reader types by hand is still bare digits.
    const t = goto.value.trim().replace(/^@/, "").replace(/^0x/i, "");
    if (!/^[0-9a-f]+$/i.test(t)) return goto.classList.add("invalid");
    goto.classList.remove("invalid");
    const to = parseInt(t, 16);
    nav.recordJump(view.cursorState.bitOffset, to * 8);
    view.setCursor(to, { pane: "hex" });
    view.el.focus();
  });
  goto.addEventListener("input", () => goto.classList.remove("invalid"));

  // The hex view's settings, which the settings dialog shows and changes.
  const narrow = window.innerWidth < 700;
  let bytesPerRow = narrow ? 8 : 16;
  view.setBytesPerRow(bytesPerRow);
  let byteMode: "hex" | "binary" = "hex";
  /** The width the stride button was offered at, kept while it is the row
   *  width in use: the button the reader chose has to go on saying what they
   *  chose. */
  let strideOffered: { bytes: number; label: string } | null = null;
  // A stream of records read sixteen bytes to a row is read across the grain:
  // every record starts in a different column. This offers the row that is one
  // record long, where the screen is holding records of one length. It is
  // worked out as the dialog opens rather than kept up to date behind it: a
  // button that changed while the reader was deciding would be worse than one
  // that is not there.
  const offerStride = (): { bytes: number; label: string } | null => {
    if (strideOffered !== null && bytesPerRow === strideOffered.bytes) return strideOffered;
    const found = view.recordStride();
    strideOffered = found === null || [8, 16, 32].includes(found.bytes) ? null : { bytes: found.bytes, label: strideSegment(found.bytes, found.every, found.unit) };
    return strideOffered;
  };

  let column: RightColumn = "text";
  const columnKey = (): string => (doc.template === null ? "qubero.column.plain" : "qubero.column.template");
  const syncColumn = (): void => {
    const saved = storedText(columnKey());
    // Anything else saved is from an older build, or from nowhere: fall back
    // to what a file of this kind starts with.
    column = isRightColumn(saved) ? saved : doc.template === null ? "text" : "both";
    view.setRightColumn(column);
    settings?.syncHex();
  };
  syncColumn();

  // Which characters the text column is drawn in. ASCII is what a hex dump's
  // text column has always been, and is still the default; the rest are for
  // the files where the printable ninety-five leave most of the column as
  // full stops, which is every file from DOS.
  const glyphs = el("select", { className: "tb-glyphs" });
  glyphs.setAttribute("aria-label", HEXGLYPHS.label);
  glyphs.title = HEXGLYPHS.title;
  glyphs.append(el("option", { value: HEX_GLYPHS_ASCII, textContent: HEXGLYPHS.ascii }));
  /** What each set is called on screen, which is what a refusal names: the
   *  core's own name for the screen rule is a sentence, not a label. */
  const glyphLabel = (name: string): string =>
    name === SCREEN_GLYPHS ? HEXGLYPHS.screen : name === HEX_GLYPHS_ASCII ? HEXGLYPHS.ascii : name;
  // Grouped the way the text view's chooser is, and for the same reason:
  // twelve entries in one list is a list nobody reads to the end of. The last
  // group holds one entry because what is in it is not a page at all.
  for (const [label, entries] of [
    [HEXGLYPHS.groupPages, CODEPAGES_A.map((n) => [n, n] as const)],
    [HEXGLYPHS.groupDos, CODEPAGES_B.map((n) => [n, n] as const)],
    [HEXGLYPHS.groupScreen, [[SCREEN_GLYPHS, HEXGLYPHS.screen] as const]],
  ] as const) {
    const group = el("optgroup");
    group.label = label;
    for (const [value, text] of entries) group.append(el("option", { value, textContent: text }));
    glyphs.append(group);
  }
  /** Hand the view a set, falling back to ASCII for a name the core has never
   *  heard of, which is what a choice saved by an older build can be. */
  const useGlyphs = (name: string): void => {
    const table = glyphColumn(name);
    const chosen = table === null ? HEX_GLYPHS_ASCII : name;
    view.setGlyphs(table ?? ASCII_GLYPHS, glyphLabel(chosen));
    glyphs.value = chosen;
  };
  useGlyphs(storedChoice(HEX_GLYPHS_KEY, HEX_GLYPH_SETS, HEX_GLYPHS_DEFAULT));
  glyphs.addEventListener("change", () => {
    rememberChoice(HEX_GLYPHS_KEY, glyphs.value);
    useGlyphs(glyphs.value);
  });

  const dialog = new SettingsDialog(
    templateEntries,
    {
      mode: () => byteMode,
      setMode: (m) => {
        byteMode = m;
        view.setMode(m);
        // Eight binary digits per byte: a wide row has to narrow to stay readable.
        if (m === "binary" && bytesPerRow > 8) {
          bytesPerRow = 8;
          view.setBytesPerRow(8);
        }
      },
      bytesPerRow: () => bytesPerRow,
      setBytesPerRow: (n) => {
        bytesPerRow = n;
        view.setBytesPerRow(n);
      },
      stride: offerStride,
      column: () => column,
      setColumn: (c) => {
        column = c;
        rememberChoice(columnKey(), c);
        view.setRightColumn(c);
      },
    },
    glyphs,
  );
  settings = dialog;
  dialog.setTemplatesOffered(doc.isFile, doc.isFile);
  dialog.setCurrent(tmplValue, tmplValue === "" ? null : (extraTemplates.find((x) => x.value === tmplValue)?.label ?? templateLabel(tmplValue)));
  dialog.setExtras(extraTemplates);
  dialog.onPickTemplate = chooseTemplate;
  dialog.onConvertKsy = () => openKsyPanel();
  dialog.onConvertHexpat = () => openHexpatPanel();
  tab.release.push(() => dialog.el.remove());
  const gear = el("button", { type: "button", className: "icon-btn tb-settings", title: SETTINGS.gearTitle }, gearIcon());
  gear.setAttribute("aria-label", SETTINGS.gearLabel);
  gear.addEventListener("click", () => dialog.open());

  // The arrows from a field's dependencies to the field, over the bytes. Only
  // over the hex grid: the listing already draws the structure as a tree and
  // the text view has no fields to point at.
  // How many arrows could not be drawn because the field they leave from is
  // off screen. Lives in the status bar rather than over the grid: it is a
  // fact about the view, and the grid is already carrying the arrows.
  const linksNote = el("span", { className: "tb-linksnote" });
  const linksBtn = el("button", { type: "button", textContent: LINKS.button, className: "tb-links" });
  linksBtn.title = LINKS.title;
  linksBtn.setAttribute("aria-pressed", "false");
  const setLinks = (on: boolean): void => {
    view.links.setEnabled(on);
    linksBtn.setAttribute("aria-pressed", String(on));
    linksBtn.classList.toggle("is-on", on);
    rememberChoice("qubero.links", on ? "1" : "0");
    if (on) refreshLinks();
    else {
      linksNote.textContent = "";
      view.render();
    }
  };
  linksBtn.addEventListener("click", () => setLinks(linksBtn.getAttribute("aria-pressed") !== "true"));
  if (storedText("qubero.links") === "1") setLinks(true);
  // How many of the fields it would have drawn were nowhere on screen. Said
  // rather than left out: an arrow that is not there because the field is a
  // thousand rows away looks exactly like no dependency at all.
  view.links.onOffScreen = (ends) => {
    linksNote.textContent = LINKS.offScreen(
      ends.map((e) => ({ name: e.end.label, at: formatOffset(e.end.startBit), above: e.above })),
    );
  };

  // Hex and Listing are two readings of the same file, so they share the
  // cursor and swap in the same place rather than sitting side by side. The
  // listing carries its own bytes, so showing both would say it twice.
  // Which encoding the text view reads in. First entry lets the file decide,
  // which is right nearly always; the rest are for the files where nothing in
  // the file says, which is every capture of a DOS screen.
  const encoding = el("select", { className: "tb-enc" });
  encoding.setAttribute("aria-label", TEXTVIEW.encodingLabel);
  encoding.append(el("option", { value: "", textContent: TEXTVIEW.encodingAuto }));
  // Grouped, because eleven single-byte pages in one list is a list nobody
  // reads to the end of, and which family a page belongs to is the thing a
  // reader already knows about the file.
  for (const [label, names] of [
    [TEXTVIEW.encodingUnicode, UNICODE_ENCODINGS],
    [TEXTVIEW.encodingWindows, CODEPAGES_A],
    [TEXTVIEW.encodingDos, CODEPAGES_B],
  ] as const) {
    const group = el("optgroup");
    group.label = label;
    for (const name of names) group.append(el("option", { value: name, textContent: name }));
    encoding.append(group);
  }
  encoding.addEventListener("change", () => {
    // The panel reads a selection every way text can be read; which way the
    // reader is reading the file decides only which one it names first.
    inspector.textEncoding = encoding.value;
    void text.setEncoding(encoding.value);
    inspector.render();
  });
  // What the file was read as, beside the chooser, so a guess is never passed
  // off as a fact.
  const reading = el("span", { className: "tb-reading" });
  const wrapping = el("select", { className: "tb-enc" });
  wrapping.setAttribute("aria-label", "Text wrapping");
  wrapping.title = "Visual wrapping preserves file bytes and line endings. Offsets identify original lines; scrollbar positions in unindexed regions are estimates.";
  for (const [value, label] of [["off", "No wrap"], ["line", "Wrap anywhere"], ["word", "Word wrap"]] as const) {
    wrapping.append(el("option", { value, textContent: label }));
  }
  wrapping.addEventListener("change", () => {
    void text.setWrap(wrapping.value as "off" | "line" | "word");
  });
  // Which line ending the file uses, which is a fact about the whole file and
  // not about the screen, so it belongs beside the encoding rather than in the
  // margin of every line.
  const endings = el("span", { className: "tb-endings" });
  text.onReading = (r, counts) => {
    endings.textContent = TEXTVIEW.lineEndings(counts);
    reading.textContent = encoding.value === "" ? TEXTVIEW.readAs(r.encoding, r.guessed) : "";
    // Which reading of a selection the panel names first is whichever the file
    // is being read in, chosen or settled.
    inspector.textEncoding = encoding.value === "" ? r.encoding : encoding.value;
  };

  // What the string scan looks for. Kept between visits: a reader who works on
  // one kind of file sets these once.
  const minChars = el("input", { className: "tb-min", type: "number" });
  minChars.min = "1";
  minChars.max = "1024";
  minChars.title = STRINGSVIEW.minimumTitle;
  minChars.value = String(storedNumber(MIN_CHARS_KEY, MIN_CHARS_DEFAULT));
  strings.setMinimum(Number(minChars.value));
  minChars.addEventListener("change", () => {
    strings.setMinimum(Number(minChars.value));
    minChars.value = String(strings.minimum);
    rememberChoice(MIN_CHARS_KEY, minChars.value);
  });
  const minBox = el(
    "label",
    { className: "tb-minbox", title: STRINGSVIEW.minimumTitle },
    el("span", { textContent: STRINGSVIEW.minimumLabel }),
    minChars,
    el("span", { textContent: STRINGSVIEW.minimumUnit }),
  );
  // Which readings to look for. Checkboxes rather than a menu: they are not
  // alternatives, and one file holds all three at once.
  const readingBox = el("div", { className: "tb-lookfor" });
  readingBox.setAttribute("role", "group");
  readingBox.setAttribute("aria-label", STRINGSVIEW.lookForGroup);
  readingBox.append(el("span", { className: "tb-lookfor-label", textContent: STRINGSVIEW.lookForLabel }));
  const savedEncodings = storedText(ENCODINGS_KEY);
  const wanted = new Set(savedEncodings === null ? ENCODINGS : savedEncodings.split(",").filter((e) => e !== ""));
  const readingBoxes = ENCODINGS.map((name) => {
    const box = el("input", { type: "checkbox" });
    box.checked = wanted.has(name);
    const label = el(
      "label",
      { className: "tb-check", title: STRINGSVIEW.encodingToggleTitle[name] ?? "" },
      box,
      el("span", { textContent: STRINGSVIEW.encodingToggle[name] ?? name }),
    );
    box.addEventListener("change", () => {
      const picked = ENCODINGS.filter((_, i) => readingBoxes[i]?.checked === true);
      strings.setReading(picked);
      rememberChoice(ENCODINGS_KEY, picked.join(","));
    });
    readingBox.append(label);
    return box;
  });
  strings.setReading(ENCODINGS.filter((e) => wanted.has(e)));
  // Whether to leave out the strings a template reads as numbers, code or
  // packed data. Off until asked: text inside what a template calls samples
  // can be the template reading the wrong bytes, and hiding it would hide that.
  const hideInside = el("input", { type: "checkbox" });
  hideInside.checked = storedText(HIDE_INSIDE_KEY) === "1";
  strings.setHideInside(hideInside.checked);
  hideInside.addEventListener("change", () => {
    strings.setHideInside(hideInside.checked);
    rememberChoice(HIDE_INSIDE_KEY, hideInside.checked ? "1" : "0");
  });
  const hideInsideBox = el(
    "label",
    { className: "tb-check", title: STRINGSVIEW.hideInsideTitle },
    hideInside,
    el("span", { textContent: STRINGSVIEW.hideInsideToggle }),
  );
  const filter = el("input", { className: "tb-filter", type: "search" });
  filter.placeholder = STRINGSVIEW.filterPlaceholder;
  filter.title = STRINGSVIEW.filterTitle;
  filter.setAttribute("aria-label", STRINGSVIEW.filterLabel);
  filter.addEventListener("input", () => strings.setFilter(filter.value));

  // Where the main views live. The graph is put in here when it arrives, so
  // it takes the same area as the hex grid and the listing rather than a
  // corner of its own.
  const workspaceLeft = el("div", { className: "left" }, dumpBar, memberBar, search.el, view.el, text.el, strings.el, listRow);
  // The folder this file, or this dataset, was opened from: its name, the way
  // to its list, and the files either side of this one.
  if (doc.isFile) {
    const bar = folderBarFor(doc, workspaceLeft);
    if (bar !== null) workspaceLeft.prepend(bar);
  }

  const hexBtn = el("button", { type: "button", textContent: "Hex", className: "tb-view" });
  const listBtn = el("button", { type: "button", textContent: "Listing", className: "tb-view" });
  const textBtn = el("button", { type: "button", textContent: TEXTVIEW.viewButton, className: "tb-view" });
  const stringsBtn = el("button", { type: "button", textContent: STRINGSVIEW.viewButton, className: "tb-view" });
  // Behind ?graph, and built only when it has been unlocked: an experiment
  // with a button in the main switch would read as a finished view.
  const graphBtn = el("button", { type: "button", textContent: GRAPH.button, className: "tb-view" });
  const diagramBtn = el("button", { type: "button", textContent: DIAGRAM.button, className: "tb-view" });
  const reportBtn = el("button", { type: "button", textContent: RV.button, className: "tb-view", title: RV.buttonTitle });
  const views = el("div", { className: "tb-views" }, hexBtn, listBtn, textBtn, stringsBtn, diagramBtn, reportBtn);
  if (graphUnlocked) views.append(graphBtn);
  // The graph and everything it needs is a third of a megabyte of layout
  // engine. Fetched when the view is first asked for, so a reader who never
  // unlocks it never pays for it.
  let graph: GraphView | null = null;
  /** Which part of the file the graph is rooted at, so a cursor move inside
   *  the same part marks a node instead of laying the whole thing out again. */
  let graphRoot: readonly number[] | null = null;
  /** How many fields the graph will lay out, from the module once it is here. */
  let graphCap = 2000;
  /** True while the graph is waiting on bytes it asked for. See `showGraph`. */
  let graphWaiting = false;
  /** The diagram and its layout engine, fetched when the view is first opened.
   *  The same lazy chunk the graph uses, for the same reason: nobody pays for a
   *  view they never open. */
  let diagram: DiagramView | null = null;
  /** The boxes the diagram is showing, so a click on a row can be checked
   *  against the file before it moves the cursor. */
  let diagramTypes: readonly DiagramBox[] = [];
  /** Which template the diagram on screen was drawn for, so reopening the view
   *  does not throw away where the reader had panned to. Null until the first
   *  drawing. */
  let diagramFor: string | null = null;
  /** The report, fetched and built when the view is first opened, like the
   *  diagram: a reader who never opens it never pays for it. */
  let report: ReportView | null = null;
  /** True once the reader has asked a count that stopped at its limit to carry
   *  on. See `AUTO_COUNT`. */
  let keepCounting = false;
  /** True while the next step of a running count is booked, so two changes to
   *  the document in one turn do not book two. */
  let countBooked = false;
  views.setAttribute("role", "group");
  views.setAttribute("aria-label", "View");
  /** Controls that only mean anything over the hex rows. */
  const hexOnly = [linksBtn];
  /** Controls that only mean anything over the text. */
  const textOnly = [encoding, wrapping, reading, endings];
  /** Controls that only mean anything over the strings list. */
  const stringsOnly = [minBox, readingBox, hideInsideBox, filter];
  /** True while the listing is showing, which is also while the hex grid's
   *  editing state is not the user's to act on. */
  let listingShowing = false;
  /**
   * What to draw the graph of, for a cursor sitting on one field.
   *
   * Not the whole file: a WAV holds one sample per field and there are
   * twenty-four thousand of them, and a graph of the first twelve hundred of
   * those says nothing about anything. Not the field itself either, which has
   * no connections of its own to show.
   *
   * So: down the path from the root towards the field, and stop at the first
   * ancestor whose subtree fits under the cap. That is the largest piece of
   * the file that can be drawn whole, which is the most context the reader can
   * be given without the picture being a lie about what is in it. The file
   * root wins outright on a small file, which is the right answer there.
   *
   * Null means the whole file, which is also what a file with no template
   * comes back as.
   */
  const graphRootFor = (bit: number): readonly number[] | null => {
    const at = doc.locate(bit);
    const path = at.status === "ok" ? at.node : [];
    for (let i = 0; i < path.length; i++) {
      const here = path.slice(0, i);
      const reply = doc.graph(here, graphCap);
      // Not readable yet, or nothing to say: neither is a reason to go deeper.
      if (reply.status !== "ok") break;
      if (reply.node.omitted === 0) return i === 0 ? null : here;
    }
    // Every ancestor overflows, so the field's own parent is as small as this
    // gets. What it leaves out the view says out loud.
    return path.length > 1 ? path.slice(0, -1) : null;
  };

  /** Whether two roots are the same part of the file. Null is the whole file,
   *  which is a root like any other and is the same as itself. */
  const sameRoot = (a: readonly number[] | null, b: readonly number[] | null): boolean => {
    if (a === null || b === null) return a === b;
    return a.length === b.length && a.every((x, i) => x === b[i]);
  };

  /**
   * Build the graph for wherever the cursor is, fetching the layout engine the
   * first time it is asked for.
   *
   * Called when the view is opened and when the cursor leaves the part the
   * graph was drawn for. Inside one part the cursor only marks a node, since
   * laying the graph out again would move every field the reader was reading.
   */
  const showGraph = async (): Promise<void> => {
    if (graph === null) {
      const { GraphView, NODE_CAP } = await import("./graphview.ts");
      graph = new GraphView();
      graphCap = NODE_CAP;
      // A double tap, since it switches the view: see `DiagramView.offerGo`.
      graph.onPick = (path) => {
        setView("hex");
        goToField(path);
      };
      graph.el.hidden = false;
      workspaceLeft.append(graph.el);
    }
    const root = graphRootFor(view.cursorState.bitOffset);
    const reply = doc.graph(root ?? [], graphCap);
    // Bytes on their way, or a walk that ran out of time. Either is answered
    // by asking again once the document says something changed, which is how
    // every other panel here waits.
    if (reply.status !== "ok") {
      graphWaiting = true;
      return;
    }
    graphWaiting = false;
    graphRoot = root;
    const named = root === null ? null : doc.templateNode(root);
    const rootName = named !== null && named.status === "ok" ? named.node.name : null;
    graph.show(reply.node, rootName);
    graph.relayoutForShow();
    graph.setPath(linkPath);
  };

  /**
   * Build the diagram, fetching the layout engine the first time it is asked
   * for.
   *
   * Not rebuilt as the cursor moves: the picture is of the format, and the
   * cursor is in the file. It is rebuilt when the template changes, which is
   * the one thing that changes what it shows.
   */
  /**
   * Where a row of the diagram's first box is in the open file, if it is there.
   *
   * The format's first type is what the root of the file is read as, so its
   * field `i` is the path `[i]` — but only when the root is that type outright.
   * A format whose root is a list of its type has an element at `[i]`, not a
   * field, so the field's name is checked against what the file's node is
   * called and a mismatch is no path at all.
   *
   * TODO: a core query for "the first path whose type is X" would let a click
   * on any box reach the file. Until then a click on any other box is a click
   * on the format, and the cursor stays where it was.
   */
  const diagramFieldPath = (box: number, row: number): readonly number[] | null => {
    if (box !== 0) return null;
    const name = diagramTypes[0]?.rows[row]?.name;
    if (name === undefined) return null;
    const n = doc.templateNode([row]);
    if (n.status !== "ok" || n.node.name !== name) return null;
    return [row];
  };

  const showDiagram = async (): Promise<void> => {
    if (diagram === null) {
      const { DiagramView } = await import("./diagramview.ts");
      diagram = new DiagramView();
      diagram.canPick = (box, row) => diagramFieldPath(box, row) !== null;
      // Both of these are a double click on the drawing: a single click only
      // marks a row, and going to the hex view is a view switch.
      diagram.onKeepCounting = () => {
        keepCounting = true;
        countForDiagram();
      };
      diagram.onPick = (box, row) => {
        const path = diagramFieldPath(box, row);
        if (path === null) return;
        setView("hex");
        goToField(path);
      };
      diagram.onGo = (path, space) => {
        // Only the file's own reading has offsets the hex view understands; a
        // field inside an unpacked stream is not offered one, and the view says
        // so rather than moving the cursor to the wrong byte.
        if (space !== 0) return;
        setView("hex");
        goToField(path);
      };
      diagram.el.hidden = false;
      workspaceLeft.append(diagram.el);
    }
    // Coming back to a picture that has not changed is coming back to the same
    // picture: the boxes the reader opened are still open and the view is
    // still where they panned it. Only a new template is a new drawing.
    const format = doc.template ?? "";
    if (diagramFor === format) {
      diagram.relayout();
      // A count left running when the view was hidden carries on from where
      // it stopped.
      countForDiagram();
      return;
    }
    diagramFor = format;
    const reply = doc.templateDiagram();
    // No template is an answer, not a failure: the view says so rather than
    // showing the last format's picture over an unrecognised file.
    diagramTypes = reply.status === "ok" ? reply.node.types : [];
    diagram.show(reply.status === "ok" ? reply.node : null, templateLabel(format));
    countForDiagram();
  };

  /**
   * Carry the count of the open file against the diagram's boxes on, and hand
   * over what it has so far.
   *
   * The count is kept by the core between calls, so each call is the next
   * stretch of one walk. As many goes as fit in a turn, then the page gets the
   * turn back. What asks again depends on why it stopped: a go that ran out
   * books the next turn itself, since nothing else will wake it; a count
   * waiting on bytes has asked for them, and their arrival changes the
   * document, which calls this; a count stopped at its limit waits for the
   * reader. An edit or a new template throws the count away in the core, and
   * the change it makes starts it again here.
   */
  const countForDiagram = (): void => {
    if (diagram === null || diagram.el.hidden) return;
    const limit = countLimit(doc.lengthBytes, keepCounting);
    const until = performance.now() + COUNT_TURN_MS;
    let reply = doc.diagramCensus(limit);
    while (reply.status === "ok" && reply.node.state === "working" && performance.now() < until) {
      reply = doc.diagramCensus(limit);
    }
    // An error is an answer: there is nothing to count against. A reply that is
    // not ready leaves the counts already drawn where they are, and the
    // document asks again when it is.
    if (reply.status === "ok") diagram.setCensus(reply.node);
    else if (reply.status === "error") diagram.setCensus(null);
    if (reply.status === "ok" && reply.node.state === "working" && !countBooked) {
      countBooked = true;
      setTimeout(() => {
        countBooked = false;
        countForDiagram();
      }, 0);
    }
  };

  /**
   * What the report asks of the page. Every one of these is what another view
   * already does: a single click on a byte reference or a part moves the
   * cursor, so the panel at the cursor says what it is, and a double click
   * goes to the hex view as the Diagram view's does.
   */
  const reportHost: ReportHost = {
    pick: (t) => {
      // A field read out of a stream has no place in the file to go to: the
      // panel follows it, as it does from the listing, and the hex view stays.
      if (t.path !== undefined && !inFile(t.path)) {
        inspector.setPath(t.path);
        return;
      }
      if (t.path !== undefined && inFile(t.path)) {
        const n = doc.templateNode(t.path);
        if (n.status === "ok") {
          nav.recordJump(view.cursorState.bitOffset, n.node.offset_bits);
          goToField(t.path);
          overview.reveal(t.path);
          return;
        }
      }
      jumpToBit(t.startBit);
      if (t.endBit - t.startBit > 8) view.selectRange(t.startBit, t.endBit, t.startBit);
    },
    go: (t) => {
      setView("hex");
      reportHost.pick(t);
    },
    openTable: (path) => openTable(path),
    showInListing: (path) => {
      setView("listing");
      structure.reveal(path);
    },
    openUnpacked: (path) => openUnpacked(path),
  };

  /** Build the report the first time it is asked for, and bring it up to date
   *  each time it is shown. */
  const showReport = async (): Promise<void> => {
    if (report === null) {
      const { ReportView } = await import("./reportview.ts");
      const r = new ReportView(doc, reportHost);
      report = r;
      // Only the view on screen says where the reader is, as for the hex view
      // and the listing.
      r.onViewport = (v) => {
        if (!r.el.hidden) overview.setViewport(v);
      };
      r.el.hidden = showingView !== "report";
      workspaceLeft.append(r.el);
      tab.release.push(() => r.dispose());
    }
    if (!report.el.hidden) report.show();
  };

  /** The `.ksy` converter, built the first time it is opened. It keeps its text
   *  while it is closed, so reopening comes back to what was being worked on. */
  let ksyPanel: KsyPanel | null = null;
  /** Which view the converter was opened over, so closing it goes back there. */
  let ksyCameFrom: View = "hex";
  /** What is in the main pane now. Only the converter needs to ask, and only so
   *  that closing it can put back what it covered. */
  let showingView: View = "hex";

  /** Apply the template in the converter to the open file. Also what the menu's
   *  own entry for it does, for a reader who read the file with something else
   *  and has come back. */
  const applyKsy = (): void => ksyPanel?.apply();

  /**
   * Open the converter over the main pane, with a `.ksy` in it where one was
   * dropped or picked.
   *
   * Nothing about how the file is being read changes here. The converter is a
   * tool: it converts as it is typed into, on a scratch basis, and only "Use
   * this template" reaches the document.
   */
  const openKsyPanel = (load?: { text: string; name: string | null } | { bundled: string }): void => {
    if (ksyPanel === null) {
      const panel = new KsyPanel(doc);
      ksyPanel = panel;
      panel.onMessage = (message, warn) => say(message, warn);
      panel.onClose = () => setView(ksyCameFrom === "ksy" ? "hex" : ksyCameFrom);
      panel.onApply = (id, bundled) => {
        if (bundled) {
          // A shipped description the reader did not change is the format the
          // menu already lists, so it is applied by name and the menu goes on
          // showing its title.
          const name = `${KAITAI_PREFIX}${id}`;
          if (doc.setTemplate(name)) {
            setTemplateValue(name);
            // The entry for a pasted `.ksy` named a template that is no longer
            // what the converter holds, and picking it would have applied this
            // one under that name. It goes, and comes back with the next
            // pasted template.
            dropExtra("ksy");
            say(KSY.applied(id));
            showKaitaiNote(name);
          } else say(KSY.cannotApply(id), true);
        } else {
          // The chooser names whatever is reading the file, and that is now this.
          setExtra({ value: KSY_VALUE, label: KSY.menuApplied(id), kind: "ksy" });
          setTemplateValue(KSY_VALUE);
          overview.setNote("");
        }
        structure.setMatched(true);
        inspector.setMode("structure");
        // The diagram is a picture of the template, and the template is a new
        // one, so the drawing on hand is of the last format.
        diagramFor = null;
        keepCounting = false;
        setView("listing");
      };
      panel.el.hidden = true;
      workspaceLeft.append(panel.el);
      tab.release.push(() => panel.dispose());
    }
    if (load !== undefined && "bundled" in load) {
      const text = doc.bundledKsyText(load.bundled);
      if (text !== "") ksyPanel.loadBundled(load.bundled, text);
    } else if (load !== undefined) ksyPanel.load(load.text, load.name);
    if (showingView !== "ksy") ksyCameFrom = showingView;
    setView("ksy");
  };

  /**
   * The note under a bundled Kaitai format: where the description came from,
   * how much of it the template leaves out, and the way to read it.
   *
   * The count is only useful next to the description it counts, so the note
   * carries the way there: the converter opens with the shipped `.ksy` in it
   * and lists each part beside the line it is written on.
   */
  const showKaitaiNote = (name: string): void => {
    const id = name.slice(KAITAI_PREFIX.length);
    overview.setNote(KAITAI_TEMPLATE.note(doc.ksyReport()?.gaps.length ?? 0), {
      label: KAITAI_TEMPLATE.source,
      title: KAITAI_TEMPLATE.sourceTitle,
      run: () => openKsyPanel({ bundled: id }),
    });
  };

  /** The ImHex pattern converter, built the first time it is opened. */
  let hexpatPanel: HexpatPanel | null = null;
  /** Which view it was opened over, so closing it goes back there. */
  let hexpatCameFrom: View = "hex";
  const applyHexpat = (): void => hexpatPanel?.apply();

  /**
   * Open the ImHex pattern converter over the main pane, with a pattern in it
   * where one was dropped or picked. The same tool as the `.ksy` converter and
   * the same rules: nothing reaches the document until it is applied.
   */
  const openHexpatPanel = (load?: { text: string; name: string | null } | { bundled: string }): void => {
    if (hexpatPanel === null) {
      const panel = new HexpatPanel(doc);
      hexpatPanel = panel;
      panel.onMessage = (message, warn) => say(message, warn);
      panel.onClose = () => setView(hexpatCameFrom === "hexpat" ? "hex" : hexpatCameFrom);
      panel.onApply = (id, bundled) => {
        if (bundled) {
          // A shipped pattern nobody has edited is the format the menu already
          // lists, so it is applied by name and the menu goes on showing it.
          const name = `${IMHEX_PREFIX}${id}`;
          if (doc.setTemplate(name)) {
            setTemplateValue(name);
            dropExtra("hexpat");
            say(HEXPAT.applied(id));
            showImhexNote(name);
          } else say(HEXPAT.cannotApply(id), true);
        } else {
          setExtra({ value: HEXPAT_VALUE, label: HEXPAT.menuApplied(id), kind: "hexpat" });
          setTemplateValue(HEXPAT_VALUE);
          overview.setNote("");
        }
        structure.setMatched(true);
        inspector.setMode("structure");
        diagramFor = null;
        keepCounting = false;
        setView("listing");
      };
      panel.el.hidden = true;
      workspaceLeft.append(panel.el);
      tab.release.push(() => panel.dispose());
    }
    if (load !== undefined && "bundled" in load) {
      const text = doc.bundledHexpatText(load.bundled);
      if (text !== "") hexpatPanel.loadBundled(load.bundled, text);
    } else if (load !== undefined) hexpatPanel.load(load.text, load.name);
    if (showingView !== "hexpat") hexpatCameFrom = showingView;
    setView("hexpat");
  };

  /** The note under a template converted from a bundled ImHex pattern: where it
   *  came from, how much of it the template leaves out, and the way there. */
  const showImhexNote = (name: string): void => {
    const id = name.slice(IMHEX_PREFIX.length);
    overview.setNote(HEXPAT_TEMPLATE.note(doc.hexpatReport()?.gaps.length ?? 0), {
      label: HEXPAT_TEMPLATE.source,
      title: HEXPAT_TEMPLATE.sourceTitle,
      run: () => openHexpatPanel({ bundled: id }),
    });
  };

  const setView = (which: View): void => {
    showingView = which;
    const ksyOn = which === "ksy";
    const hexpatOn = which === "hexpat";
    // Either converter takes the main pane the same way, so everything that
    // asks whether a tool is showing asks this.
    const toolOn = ksyOn || hexpatOn;
    const listingOn = which === "listing";
    const textOn = which === "text";
    const stringsOn = which === "strings";
    const graphOn = which === "graph";
    const diagramOn = which === "diagram";
    const reportOn = which === "report";
    listingShowing = listingOn;
    view.el.hidden = which !== "hex";
    structure.el.hidden = !listingOn;
    listRow.hidden = !listingOn;
    text.el.hidden = !textOn;
    strings.el.hidden = !stringsOn;
    if (graph !== null) graph.el.hidden = !graphOn;
    if (diagram !== null) diagram.el.hidden = !diagramOn;
    if (report !== null) report.el.hidden = !reportOn;
    if (ksyPanel !== null) ksyPanel.el.hidden = !ksyOn;
    if (hexpatPanel !== null) hexpatPanel.el.hidden = !hexpatOn;
    for (const c of hexOnly) c.hidden = which !== "hex";
    for (const c of textOnly) c.hidden = !textOn;
    for (const c of stringsOnly) c.hidden = !stringsOn;
    // Only a template says which bytes are numbers or code.
    hideInsideBox.hidden = !stringsOn || doc.template === null;
    for (const [btn, on] of [
      [hexBtn, which === "hex"],
      [listBtn, listingOn],
      [textBtn, textOn],
      [stringsBtn, stringsOn],
      [graphBtn, graphOn],
      [diagramBtn, diagramOn],
      [reportBtn, reportOn],
    ] as const) {
      btn.setAttribute("aria-pressed", String(on));
      btn.classList.toggle("is-on", on);
    }
    // The converter is not a reading of the file, so it is not what a reader
    // meant to come back to next time.
    if (!toolOn) rememberChoice("qubero.view", which);
    // A hidden view ignores the cursor, since scrolling something nobody is
    // looking at only loses their place in it. So when it comes back it has
    // wherever the cursor was left to catch up on.
    if (listingOn) {
      structure.relayout();
      listPane.relayout();
      structure.setBit(view.cursorState.bitOffset);
    } else if (textOn) {
      void text.setByte(Math.floor(view.cursorState.bitOffset / 8));
    } else if (stringsOn) {
      strings.enter();
      strings.setByte(Math.floor(view.cursorState.bitOffset / 8));
    } else if (graphOn) {
      void showGraph();
    } else if (diagramOn) {
      void showDiagram();
    } else if (reportOn) {
      void showReport();
    } else if (!toolOn) view.relayout();
    if (ksyOn) ksyPanel?.focus();
    else if (hexpatOn) hexpatPanel?.focus();
    else
      (listingOn
        ? structure.el
        : textOn
          ? text.el
          : stringsOn
            ? strings.el
            : graphOn && graph !== null
              ? graph.el
              : diagramOn && diagram !== null
                ? diagram.el
                : reportOn && report !== null
                  ? report.el
                  : view.el
      ).focus();
    refresh();
  };
  // Escape leaves the converter and Ctrl+Enter applies it, from anywhere inside
  // it: Tab in the text box indents rather than moving the focus out, so the
  // keys have to work while the caret is in there.
  key((e) => {
    if (showingView === "ksy" && ksyPanel !== null) {
      if (ksyPanel.handleKey(e)) e.preventDefault();
      return;
    }
    if (showingView === "hexpat" && hexpatPanel !== null) {
      if (hexpatPanel.handleKey(e)) e.preventDefault();
    }
  });
  hexBtn.addEventListener("click", () => setView("hex"));
  listBtn.addEventListener("click", () => setView("listing"));
  textBtn.addEventListener("click", () => setView("text"));
  stringsBtn.addEventListener("click", () => setView("strings"));
  // Picking a string is the same as putting the cursor on its first byte,
  // which is what every other view is looking at.
  strings.onPick = (at, len) => {
    nav.recordJump(view.cursorState.bitOffset, at * 8);
    // Selected as well as pointed at, so the panel says what the bytes are
    // every way they can be read. That is the check on the reading this view
    // picked, in the place that already spells such things out.
    //
    // `selectRange` puts the cursor at the front of the run itself. Setting it
    // again here dropped the selection on its way out, which is why the panel
    // used to answer about whatever field the string sat inside.
    view.selectRange(at * 8, (at + len) * 8, at * 8);
  };
  strings.onPickPrefix = (at, len) => {
    nav.recordJump(view.cursorState.bitOffset, at * 8);
    // The number's own bytes, for the same reason the string gets its own:
    // the panel is where a reader checks a reading, and the reading here is
    // that these bytes are a length.
    view.selectRange(at * 8, (at + len) * 8, at * 8);
  };
  graphBtn.addEventListener("click", () => setView("graph"));
  diagramBtn.addEventListener("click", () => setView("diagram"));
  reportBtn.addEventListener("click", () => setView("report"));
  // Picking a character in the text is the same as putting the cursor on its
  // first byte, which is what every other view is looking at.
  text.onPick = (at) => {
    view.setBitCursor(at * 8);
  };
  // Typing in the text writes bytes, so the rest of the page has to catch up
  // the way it does after any other edit.
  text.onEdit = () => refresh();
  text.onRefuse = (char, encodingName) => say(TEXTVIEW.refused(char, encodingName), true);
  text.onMessage = (msg) => say(msg, true);
  // The hex view owns the selection, so the text view writes through it and
  // reads it back. One selection, the way there is one cursor.
  text.onSelect = (from, to, caret) => view.selectRange(from * 8, to * 8, caret * 8);

  const openBtn = el("button", { type: "button", textContent: "Open" });
  openBtn.addEventListener("click", () => pick());

  // An unpacked stream is read-only and has no file of its own to save to, and
  // its template came with the stream rather than being chosen. Controls that
  // would do nothing are not shown rather than shown disabled: a row of dead
  // buttons is a row of questions.
  if (!doc.isFile) {
    for (const c of [saveBtn, undoBtn, redoBtn, tmpl]) c.hidden = true;
  }

  const toolbar = el(
    "header",
    { className: "toolbar" },
    openBtn,
    // Before anything whose width changes with the file or the view, so the
    // switch stays put when it is pressed.
    views,
    fileLabel,
    kind.label,
    kind.info,
    encoding,
    wrapping,
    reading,
    endings,
    minBox,
    readingBox,
    hideInsideBox,
    filter,
    saveMsg,
    el("span", { className: "tb-spacer" }),
    goto,
    linksBtn,
    tmpl,
    gear,
    undoBtn,
    redoBtn,
    // Last and apart from the view switch, so that switching views cannot
    // land on it by mistake.
    saveBtn,
    dialog.el,
  );

  // The origin sits after the offset, because it answers a question about the
  // offset: this byte, and where it came from.
  const originLabel = el("span", { className: "tb-origin" });
  const statusbar = el("footer", { className: "statusbar" }, posLabel, originLabel, linksNote);

  const refresh = (): void => {
    // What the dump said was read off the file as it was opened. Once it has
    // been edited that reading is about bytes that are no longer there, so the
    // offer goes rather than saying something that stopped being true.
    if (doc.modified) dumpBar.hidden = true;
    // The name of what is being read, which for an unpacked stream is what the
    // tab calls it rather than the file it came out of.
    fileLabel.textContent = `${tab.title}${doc.modified ? " (edited)" : ""}  ${formatSize(doc.lengthBytes)}`;
    if (tabs.showing(tab)) app.querySelector(".tab.is-active")?.classList.toggle("is-edited", doc.modified);
    undoBtn.disabled = !doc.canUndo;
    redoBtn.disabled = !doc.canRedo;
    const history = doc.isFile && (doc.canUndo || doc.canRedo);
    undoBtn.hidden = !history;
    redoBtn.hidden = !history;
    const c = view.cursorState;
    const at = document.createElement("span");
    at.className = "addr";
    at.textContent = formatOffset(c.bitOffset);
    // Inside a byte the decimal counts bits, so the two halves agree.
    const decimal =
      c.bitOffset % 8 === 0 ? `(${c.offset.toLocaleString()})` : `(bit ${c.bitOffset.toLocaleString()})`;
    // Overwrite/Insert and the pane are the hex grid's, so they go with it:
    // reading `· Hex` under a listing says the wrong thing about both.
    const pane = c.pane === "ascii" ? "Text" : c.mode === "binary" ? "Binary" : "Hex";
    const editing = `  ·  ${c.insertMode ? "Insert" : "Overwrite"}  ·  ${pane}`;
    // How much is selected, where there is a selection to say it about. Whole
    // bytes are counted in bytes; a run that does not fill them is counted in
    // bits, since "3 bytes" for 28 bits would be wrong in both directions.
    const sel = listingShowing ? null : view.selectionRange;
    const bits = sel === null ? 0 : sel.endBit - sel.startBit;
    const selected =
      bits === 0 ? "" : `  ·  ${bits % 8 === 0 ? selectedBytes(bits / 8) : `${bits.toLocaleString()} bits`} selected`;
    posLabel.replaceChildren("Offset ", at, ` ${decimal}${listingShowing ? "" : editing}${selected}`);
    // Where the byte under the cursor came from. Only an unpacked stream has an
    // answer, and only once a codec keeps a trace of what it did; until then
    // there is nothing to say and nothing is said.
    // Asked whichever tab this is: from the file it marks the unpacked tabs,
    // and only an unpacked tab has a line of its own to show.
    const step = linkCursor(tab, c.bitOffset);
    originLabel.textContent = doc.isFile ? "" : originLine(doc, step);
  };
  view.onSelectionChange = (r) => {
    text.setSelection(r === null ? null : r.startBit / 8, r === null ? 0 : r.endBit / 8);
    refresh();
    inspector.setSelection(r === null ? [] : [r]);
  };
  view.onCursorChange = (c) => {
    inspector.setOffset(c.bitOffset);
    markTables(doc, c.bitOffset);
    if (!text.el.hidden) void text.setByte(Math.floor(c.bitOffset / 8));
    if (!picking) {
      followCursor(c.bitOffset);
      // Moving the cursor by hand starts the next search from there rather
      // than carrying on from the last match.
      search.reset();
    }
    refresh();
  };
  let hadTemplate = doc.template;
  doc.onChange(() => {
    if (doc.template !== hadTemplate) {
      hadTemplate = doc.template;
      // A count carried past its limit was of the last template. The new one
      // stops at the limit again until the reader asks.
      keepCounting = false;
      syncColumn();
      strings.templateChanged();
      hideInsideBox.hidden = strings.el.hidden || doc.template === null;
      // The diagram is a picture of the template and of nothing else, so a new
      // template is the one thing that changes it. Only while it is showing:
      // laying it out behind a hidden view costs a layout nobody is looking at.
      if (diagram !== null && !diagram.el.hidden) void showDiagram();
    }
    // The counts are of the file, so they are taken again whenever it changes:
    // bytes arriving, an edit, a stream opening.
    if (diagram !== null && !diagram.el.hidden) countForDiagram();
    refresh();
    if (followWhenLoaded !== null) {
      // The first try was turned away for want of bytes, and `followCursor`
      // skips a bit it has already seen. Forget it, so this one goes through.
      followedBit = null;
      followCursor(followWhenLoaded);
    }
    // The graph asked for a part of the file whose bytes had not arrived. They
    // have now, or some of them have, so it asks again rather than sitting
    // blank.
    if (graphWaiting && graph !== null && !graph.el.hidden) void showGraph();
  });

  // A match moves the cursor and marks its bytes in whichever view is showing.
  search.onCursor = () => view.cursorState.offset;
  search.onFound = ({ at, len }) => {
    picking = true;
    view.setHighlight({ startBit: at * 8, endBit: (at + len) * 8 });
    view.setCursor(at, { pane: "hex" });
    structure.setBit(at * 8);
    inspector.setOffset(at * 8);
    picking = false;
  };
  const relayout = (): void => {
    view.relayout();
    structure.relayout();
    listPane.relayout();
    overview.pump();
  };
  // Picking a part moves the cursor everywhere, the same as picking a row in
  // the listing, and brings the listing to it: a part is usually off screen.
  overview.onPick = ({ path, startBit, endBit }) => {
    view.setHighlight({ startBit, endBit });
    picking = true;
    view.setBitCursor(startBit, { pane: "hex" });
    picking = false;
    inspector.setPath(path);
    structure.reveal(path);
  };
  // The listing works out what the parts of the file are; the rail lists them
  // and the hex view draws their headings. The rail says whether they changed,
  // so a walk that named the same parts again redraws nothing.
  structure.onOutline = (headings) => {
    if (overview.setOutline(headings)) view.setSections(headings);
    report?.setOutline(headings);
  };
  // Only the view on screen says where the reader is. A hidden listing still
  // walks the file and would otherwise drag the rail's mark to wherever it
  // happened to be left.
  view.onViewport = (v) => {
    if (!view.el.hidden) overview.setViewport(v);
  };
  structure.onViewport = (v) => {
    if (!structure.el.hidden) overview.setViewport(v);
  };
  // A cell of the map stands for a stretch of the file, so picking one marks
  // that stretch: the panel at the cursor then reads it as a number, which is
  // most of what picking a few bytes out of a map is for.
  overview.onJump = (startBit, endBit) => {
    jumpToBit(startBit);
    view.selectRange(startBit, endBit);
  };
  structure.onOpenList = (path) => {
    listPane.open(path);
    listPane.setBit(view.cursorState.bitOffset);
    relayout();
  };
  listPane.onPick = ({ path, startBit, endBit }) => {
    structure.onPick({ path, startBit, endBit });
  };
  listPane.onClose = () => relayout();
  const right = panel("At cursor", inspector.el, relayout);
  // One element per tab, so switching is a matter of hiding one and showing
  // another: everything inside keeps where it was scrolled to and what it had
  // folded away.
  const page = el(
    "div",
    { className: "tabpage" },
    toolbar,
    el(
      "main",
      { className: "workspace" },
      overview.el,
      workspaceLeft,
      right,
    ),
    statusbar,
    kind.dialog,
  );
  const startView = storedText("qubero.view");
  key((e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    const pressed = e.key.toLowerCase();
    // Ctrl+H opens the bar with the replace row already open, since asking to
    // replace is asking for both halves of it.
    if (pressed === "f" || pressed === "h") {
      e.preventDefault();
      search.show(pressed === "h");
    }
  });
  // The next file dropped may be one no template covers. Fetch the rules while
  // nothing is waiting on them, so that file is named as soon as it opens.
  prefetchMagic();
  let started = false;
  /** The page is in the document now, so the views can measure themselves.
   *  Also run each time this tab comes back to the front, since a view laid
   *  out while it was hidden measured nothing. */
  const shown = (): void => {
    // The status slot belongs to whichever page is showing.
    say = (text, warn) => {
      saveMsg.textContent = text;
      saveMsg.classList.toggle("warn", warn === true);
    };
    // So does a dropped `.ksy`.
    dropKsy = doc.isFile ? (text, name) => openKsyPanel({ text, name }) : () => say(KSY.notHere, true);
    dropHexpat = doc.isFile ? (text, name) => openHexpatPanel({ text, name }) : () => say(HEXPAT.notHere, true);
    if (!started) {
      started = true;
      // A saved "graph" from a browser where it was once unlocked is not a
      // reason to open a view that is no longer on offer.
      const start: View =
        startView === "listing" ||
        startView === "text" ||
        startView === "strings" ||
        startView === "diagram" ||
        startView === "report" ||
        (startView === "graph" && graphUnlocked)
          ? startView
          : "hex";
      setView(start);
      view.relayout();
      refresh();
      inspector.setOffset(0);
      view.el.focus();
      return;
    }
    relayout();
    refresh();
  };
  if (import.meta.env.DEV) {
    Object.assign(window, {
      __qubero: {
        doc,
        view,
        inspector,
        overview,
        structure,
        listPane,
        text,
        strings,
        setView,
        tabs,
        graph: () => graph,
        diagram: () => diagram,
        report: () => report,
        ksy: () => ksyPanel,
        openKsy: openKsyPanel,
        hexpat: () => hexpatPanel,
        openHexpat: openHexpatPanel,
      },
    });
  }
  return { el: page, shown };
}

function pick(): void {
  const input = el("input", { type: "file" });
  input.addEventListener("change", () => {
    const f = input.files?.[0];
    if (f) openFile(f);
  });
  input.click();
}

/** Datasets read from a folder, and the ZIP each is read from, which Save as
 *  writes. */
const builtFor = new WeakMap<Doc, BuiltZip>();
/** Documents opened from the folder that is open, plain files and datasets. */
const folderDocs = new WeakSet<Doc>();

/** The folder the open document came from, while it did. */
type OpenedFolder = {
  /** The folder's name, or a count of the items dropped together. */
  readonly name: string;
  /** What a ZIP of it is called. */
  readonly zip: string;
  /** Every file, in path order. */
  readonly files: readonly FolderFile[];
  readonly dropped: Dropped;
  readonly list: FolderList;
  /** Whether the folder holds a dataset, which opens as one. */
  readonly dataset: boolean;
  /** The file open from the list, or null for the dataset or none yet. */
  at: number | null;
};
let folder: OpenedFolder | null = null;

/** Says where a message goes before anything is open, and after. */
function report(text: string, warn = false): void {
  if (welcomeStatus !== null) welcomeStatus.textContent = text;
  else say(text, warn);
}

/** How far opening a folder has got, over whatever is showing, with the way
 *  to stop it. Shown only once opening has taken long enough to wonder. */
const busyText = el("span");
const busyCancel = el("button", { type: "button", textContent: FOLDER.cancel });
const busyCard = el("div", { className: "busycard" }, busyText, busyCancel);
busyCard.setAttribute("role", "status");
busyCard.hidden = true;
document.body.append(busyCard);

/** Run `work` with the busy card up once it has taken a moment. `show` sets
 *  what the card says; it is written out a few times a second rather than on
 *  every piece of every file read. */
async function busy(name: string, work: (show: (text: string) => void, signal: AbortSignal) => Promise<void>): Promise<void> {
  const stop = new AbortController();
  let latest = "";
  let ticker: ReturnType<typeof setInterval> | undefined;
  const reveal = setTimeout(() => {
    busyText.textContent = latest;
    busyCard.hidden = false;
    ticker = setInterval(() => (busyText.textContent = latest), 200);
  }, 400);
  busyCancel.onclick = () => stop.abort();
  welcomeCrystal?.setBusy(true);
  try {
    await work((text) => (latest = text), stop.signal);
  } catch (error) {
    if (error instanceof Stopped) report(FOLDER.stopped(name));
    else openFailed(error);
  } finally {
    clearTimeout(reveal);
    clearInterval(ticker);
    busyCard.hidden = true;
    welcomeCrystal?.setBusy(false);
  }
}

/**
 * Open a folder, or several items dropped together. A folder holding a
 * dataset opens as that dataset; a folder of one file opens that file; any
 * other opens as a list of its files, to open one from.
 *
 * `read` is called before anything is awaited, since a drop's items are gone
 * once the event is over. Replacing a document with unsaved edits is asked
 * about by the caller, before the folder is read.
 */
async function openFolder(name: string, read: (seen: (count: number) => void) => Promise<Dropped | null>): Promise<void> {
  let show: (text: string) => void = () => {};
  const reading = read((count) => show(FOLDER.listing(name, count)));
  await busy(name, async (status) => {
    show = status;
    const dropped = await reading;
    if (dropped === null) return report(FOLDER.unreadable, true);
    if (dropped.files.length === 0) return report(FOLDER.empty(name), true);
    const byPath = (a: FolderFile, b: FolderFile): number => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
    const files = [...dropped.files].sort(byPath);
    const root = dropped.folder === null ? "" : `${dropped.folder}/`;
    const title = dropped.folder ?? FOLDER.dropped(files.length);
    const dataset = datasetIn(files) !== null;
    const list = new FolderList(
      title,
      files.map((f) => ({ label: f.path.startsWith(root) ? f.path.slice(root.length) : f.path, size: f.file.size })),
      formatSize,
    );
    const opened: OpenedFolder = { name: title, zip: dropped.name, files, dropped, list, dataset, at: null };
    list.onOpen = (i) => openFromFolder(opened, i);
    if (dataset) list.onDataset = () => void busy(title, (s, signal) => openDataset(opened, s, signal));
    list.onDismiss = () => hideFolderList();
    list.render();
    if (dataset) return openDataset(opened, status, new AbortController().signal);
    folder = opened;
    if (files.length === 1) return openFromFolder(opened, 0, false);
    showFolderPage(opened);
  });
}

/** The list of a folder in place of the start screen, before any of its files
 *  is open. */
function showFolderPage(opened: OpenedFolder): void {
  welcomeCrystal?.dispose();
  welcomeCrystal = null;
  tabs.clear();
  welcomeStatus = el("p", { className: "welcome-status" });
  welcomeStatus.setAttribute("role", "status");
  opened.list.el.className = "folderlist is-page";
  app.replaceChildren(el("div", { className: "folderpage" }, opened.list.el, welcomeStatus));
  opened.list.focus();
}

/** Lay the folder's list over the views, or take it away. */
function toggleFolderList(host: HTMLElement): void {
  if (folder === null) return;
  const list = folder.list.el;
  if (list.parentElement === host) return hideFolderList();
  list.className = "folderlist is-overlay";
  host.classList.add("has-folderlist");
  host.append(list);
  folder.list.focus();
}

function hideFolderList(): void {
  const list = folder?.list.el;
  if (list === undefined || !list.classList.contains("is-overlay")) return;
  list.parentElement?.classList.remove("has-folderlist");
  list.remove();
  document.querySelector<HTMLElement>(".folderbar-files[aria-pressed='true']")?.setAttribute("aria-pressed", "false");
}

/** Open one file of the folder, in place of what is open. */
function openFromFolder(opened: OpenedFolder, i: number, ask = true): void {
  const f = opened.files[i];
  if (f === undefined) return;
  const label = leafOf(f.path);
  const edited = modifiedTab();
  if (ask && edited !== null && !confirm(discardMsg(edited.doc.name, label))) return;
  hideFolderList();
  const file = f.file instanceof File ? f.file : new File([f.file], label);
  void Doc.open(file)
    .then((doc) => {
      welcomeCrystal?.dispose();
      welcomeCrystal = null;
      welcomeStatus = null;
      folder = opened;
      opened.at = i;
      opened.list.setCurrent(i);
      folderDocs.add(doc);
      tabs.only({ doc, title: doc.name, origin: f.path });
      say(FOLDER.openedFile(label, opened.name));
    })
    .catch(openFailed);
}

/** Open the folder as the dataset it holds: its files written into a ZIP that
 *  stores them, which the templates read as one space. */
async function openDataset(opened: OpenedFolder, show: (text: string) => void, signal: AbortSignal): Promise<void> {
  const edited = folder === opened ? modifiedTab() : null;
  if (edited !== null && !confirm(discardMsg(edited.doc.name, opened.name))) return;
  hideFolderList();
  const files = orderForArchive(opened.files);
  const bytes = files.reduce((n, f) => n + f.file.size, 0);
  const total = formatBytes(bytes);
  // A small folder is summed before it opens, which takes no time worth
  // mentioning. A large one opens now and is summed after, out of the way.
  const sums = bytes <= CRC_AT_OPEN_MAX_BYTES;
  if (sums) show(FOLDER.reading(opened.name, formatBytes(0), total));
  const built = await storedZip(files, { sums, progress: (p) => show(FOLDER.reading(opened.name, formatBytes(p.done), total)), signal });
  const doc = await Doc.open(new File([built.blob], opened.zip, { type: "application/zip" }));
  builtFor.set(doc, built);
  folderDocs.add(doc);
  if (!built.summed) doc.archiveSums = new ArchiveSums(built, new SumJob(files.map((f) => f.file)));
  welcomeCrystal?.dispose();
  welcomeCrystal = null;
  welcomeStatus = null;
  folder = opened;
  opened.at = null;
  opened.list.setCurrent(null, true);
  const count = FOLDER.files(files.length);
  const kind = datasetIn(files) === "bp5" ? FOLDER.kinds.bp5 : doc.template === "omezarr" ? FOLDER.kinds.omezarr : FOLDER.kinds.zarr;
  tabs.only({ doc, title: opened.name, origin: FOLDER.datasetOrigin(kind, count, opened.name) });
  say(openedMessage(opened, kind, count, formatSize(bytes)));
  // The sums start once the page has settled, so the first reads of the
  // archive are not queued behind a read of all of it.
  const job = doc.archiveSums?.job;
  if (job !== undefined) {
    const request = (window as { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => void }).requestIdleCallback;
    if (request !== undefined) request(() => job.start(), { timeout: 5000 });
    else setTimeout(() => job.start(), 2000);
  }
}

/** What opening a dataset came to: its files, and for a BP5 dataset what its
 *  folder lacks and whether a second dataset beside it went unread. */
function openedMessage(opened: OpenedFolder, kind: string, count: string, size: string): string {
  const indexes = opened.files.filter((f) => leafOf(f.path) === "md.idx").map((f) => f.path.slice(0, -"/md.idx".length));
  if (indexes.length > 1) {
    const [read, rest] = [leafOf(indexes[0] as string), leafOf(indexes[1] as string)];
    return FOLDER.severalDatasets(indexes.length, opened.name, read, rest);
  }
  return FOLDER.openedDataset(opened.name, kind, count, size, missingFromDataset(opened.files));
}

/** The bar above the views for a document opened from the folder: the
 *  folder's name, the way to its list, and the files either side of this
 *  one. Null for a document that did not come from the folder open now. */
function folderBarFor(doc: Doc, host: HTMLElement): HTMLElement | null {
  const opened = folder;
  if (opened === null || !folderDocs.has(doc)) return null;
  const files = el("button", { type: "button", className: "folderbar-files", textContent: FOLDER.barFiles });
  files.setAttribute("aria-pressed", "false");
  files.addEventListener("click", () => {
    toggleFolderList(host);
    files.setAttribute("aria-pressed", String(opened.list.el.parentElement === host));
  });
  const bar = el("div", { className: "dumpbar folderbar" }, el("div", { className: "dumpbar-facts" }, el("strong", { textContent: FOLDER.bar(opened.name, FOLDER.files(opened.files.length)) })), files);
  if (opened.files.length > 1 && opened.at !== null) {
    const at = opened.at;
    const step = (by: number, text: string, title: string): HTMLButtonElement => {
      const b = el("button", { type: "button", textContent: text, title });
      b.setAttribute("aria-label", title);
      b.disabled = opened.files[at + by] === undefined;
      b.addEventListener("click", () => openFromFolder(opened, at + by));
      return b;
    };
    bar.append(step(-1, "‹", FOLDER.barPrev(opened.name)), step(1, "›", FOLDER.barNext(opened.name)));
  }
  return bar;
}

/**
 * Pick a folder and open it. `holding` is the file the folder is
 * meant to have, when the pick is to read a file that was opened by itself
 * together with the rest of its folder.
 */
function pickFolder(holding?: string): void {
  const input = el("input", { type: "file" });
  input.setAttribute("webkitdirectory", "");
  input.addEventListener("change", () => {
    const list = input.files;
    if (list === null || list.length === 0) return;
    const picked = readPicked(list);
    const name = picked.folder ?? FOLDER.files(picked.files.length);
    if (holding !== undefined && !picked.files.some((f) => leafOf(f.path) === holding)) {
      return report(DATASET_MEMBER.wrongFolder(name, holding), true);
    }
    const edited = modifiedTab();
    if (edited !== null && !confirm(discardMsg(edited.doc.name, name))) return;
    void openFolder(name, () => Promise.resolve(picked));
  });
  input.click();
}

function welcome(): void {
  welcomeCrystal?.dispose();
  welcomeCrystal = new Crystal();
  welcomeStatus = el("p", { className: "welcome-status" });
  welcomeStatus.setAttribute("role", "status");
  const openBtn = el("button", { type: "button", textContent: "Open a file", className: "primary" });
  openBtn.addEventListener("click", pick);
  // Opening a folder is a rare way in, so it is a link at the end of the hint
  // under the one button rather than a choice beside it. A button still, for
  // the keyboard and a screen reader: it opens a picker, it goes nowhere.
  const openFolderLink = el("button", { type: "button", textContent: FOLDER.open, className: "welcome-link" });
  openFolderLink.addEventListener("click", () => pickFolder());
  const drop = el(
    "div",
    { className: "welcome" },
    el("div", { className: "welcome-brand" }, welcomeCrystal.el, el("h1", { textContent: "Qubero" })),
    el("p", { className: "welcome-tagline", textContent: "A hex editor and scientific file viewer." }),
    openBtn,
    el("p", { className: "hint" }, FOLDER.hintBefore, openFolderLink, FOLDER.hintAfter),
    el("p", { className: "welcome-privacy", textContent: "Your files stay on your device. Opens files of any size." }),
    welcomeStatus,
  );
  app.replaceChildren(drop);
}

window.addEventListener("resize", () => document.querySelector(".hexview")?.dispatchEvent(new Event("relayout")));

// ----- dropping a file on the page -----

/** Shown over the whole window while a file is being dragged onto it. */
const dropSub = el("span", { className: "hint" });
const dropzone = el(
  "div",
  { className: "dropzone" },
  el("div", { className: "dropzone-card" }, el("strong", { textContent: DROP_TITLE }), dropSub),
);
dropzone.setAttribute("aria-hidden", "true");
document.body.append(dropzone);

/** True when what is being dragged is a file, rather than selected text or an
 *  image dragged out of another page. */
function draggingFile(e: DragEvent): boolean {
  return Array.from(e.dataTransfer?.types ?? []).includes("Files");
}

// Drag events fire on the element under the pointer and bubble, so entering a
// child counts as leaving its parent. Counting them keeps the overlay steady
// while the pointer crosses the toolbar, the rows and the panels.
let dragDepth = 0;
function showDropzone(on: boolean): void {
  dragDepth = on ? dragDepth : 0;
  dropzone.classList.toggle("is-over", on);
}

document.addEventListener("dragenter", (e) => {
  if (!draggingFile(e)) return;
  dragDepth += 1;
  // On the start screen there is nothing to close, so the card says only what
  // the drop does.
  dropSub.textContent = closesMsg();
  dropzone.classList.add("is-over");
});
document.addEventListener("dragleave", (e) => {
  if (!draggingFile(e)) return;
  dragDepth -= 1;
  if (dragDepth <= 0) showDropzone(false);
});
document.addEventListener("dragover", (e) => {
  if (!draggingFile(e)) return;
  // Without this the browser opens the file itself, leaving the page.
  e.preventDefault();
  if (e.dataTransfer !== null) e.dataTransfer.dropEffect = "copy";
});
document.addEventListener("drop", (e) => {
  if (!draggingFile(e)) return;
  e.preventDefault();
  showDropzone(false);
  // A folder, or several items dropped together, opens as a folder. A folder
  // arrives as an item with no usable file behind it, so it has to be told
  // apart before reaching for the file.
  const items = e.dataTransfer?.items;
  if (items !== undefined && dropIsFolder(items)) {
    const entries = Array.from(items).filter((item) => item.kind === "file");
    const only = entries.length === 1 ? entries[0]?.webkitGetAsEntry() : null;
    const name = only?.isDirectory === true ? only.name : FOLDER.files(entries.length);
    const edited = modifiedTab();
    if (edited !== null && !confirm(discardMsg(edited.doc.name, name))) return;
    void openFolder(name, (seen) => readDrop(items, seen));
    return;
  }
  const files = e.dataTransfer?.files;
  const f = files?.[0];
  if (files === undefined || f === undefined) return;
  if (/\.ksy$/i.test(f.name)) {
    openKsy(f);
    return;
  }
  if (/\.(hexpat|pat)$/i.test(f.name)) {
    openHexpat(f);
    return;
  }
  openFile(f);
});

/** A dropped `.ksy` goes into the converter over the open file, not into a tab
 *  of its own: it describes how to read a file, so there has to be one. */
function openKsy(f: File): void {
  if (dropKsy === null) {
    if (welcomeStatus !== null) welcomeStatus.textContent = KSY.needsFile;
    else say(KSY.needsFile, true);
    return;
  }
  void f
    .text()
    .then((text) => dropKsy?.(text, f.name))
    .catch(openFailed);
}

/** A dropped `.hexpat` or `.pat` goes into the ImHex pattern converter over the
 *  open file, for the same reason a `.ksy` does. */
function openHexpat(f: File): void {
  if (dropHexpat === null) {
    if (welcomeStatus !== null) welcomeStatus.textContent = HEXPAT.needsFile;
    else say(HEXPAT.needsFile, true);
    return;
  }
  void f
    .text()
    .then((text) => dropHexpat?.(text, f.name))
    .catch(openFailed);
}
// A drag that ends outside the window, or one the browser abandons, still has
// to clear the overlay.
window.addEventListener("dragend", () => showDropzone(false));
window.addEventListener("blur", () => showDropzone(false));

// A chunk that will not load is this page naming a file the site no longer
// serves, and the answer is to ask for the page again. Only while there is
// nothing open: the rule database is fetched once a file is being read, and
// reloading then would take that file away to fix a sentence about it.
watchForStaleAssets(() => tabs.all.length === 0);

const params = new URLSearchParams(location.search);
const sampleUrl = params.get("url");
if (sampleUrl !== null) {
  void fetch(sampleUrl)
    .then((r) => r.blob())
    .then((b) => Doc.open(new File([b], sampleUrl.split("/").pop() ?? "sample")))
    .then(mount)
    .catch(openFailed);
}
const synthetic = sampleUrl !== null ? null : params.get("synthetic");
const syntheticSize = synthetic === null ? null : parseSize(synthetic);
if (syntheticSize !== null) void Doc.open(syntheticFile(syntheticSize)).then(mount).catch(openFailed);
else welcome();

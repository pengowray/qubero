import { Doc, bytesSource, formatBytes, formatOffset, prefetchMagic } from "./doc.js";
import * as nav from "./navhistory.js";
import { HexView, type BitRange, type RightColumn } from "./hexview.js";
import { Inspector } from "./inspector.js";
import { saveDoc } from "./save.js";
import { parseSize, syntheticFile } from "./synthetic.js";
import { ListingReport } from "./listingreport.js";
import { ListPane } from "./listpane.js";
import { TextView } from "./textview.js";
import { OverviewPanel } from "./overviewpanel.js";
import { SearchBar } from "./searchbar.js";
import { el } from "./dom.js";
import { fileType, builtinTemplate, SIGNATURE_TEMPLATE, templateLabel, templateTypeName } from "./filetype.js";
import { DUMP, TEXTVIEW } from "./strings.js";

const appEl = document.getElementById("app");
if (!appEl) throw new Error("missing #app");
const app: HTMLElement = appEl;

const formatSize = formatBytes;

/**
 * The open documents. The first is a file the reader chose; the rest were
 * opened out of another tab's bytes: a decompressed zip entry, a field's run
 * of bytes read as a file of its own. One is showing at a time; the strip
 * above the toolbar swaps between them, and only appears once there are two.
 */
type Tab = {
  readonly doc: Doc;
  /** Where the bytes came from, for a document opened out of another one.
   *  Null for a file opened from disk. */
  readonly origin: string | null;
};
let tabs: Tab[] = [];
let active = 0;

function activeDoc(): Doc | null {
  return tabs[active]?.doc ?? null;
}

/** The first open document with unsaved edits, which is what a replacement or
 *  a page close would throw away. */
function modifiedTab(): Tab | null {
  return tabs.find((t) => t.doc.modified) ?? null;
}
/** Writes to the toolbar's message slot, once there is one. */
let say: (text: string, warn?: boolean) => void = () => {};

const DROP_TITLE = "Drop to open";
const DROP_HINT = "or drop a file anywhere on this page";
const FOLDER_MSG =
  "Folders can't be opened. Zip the folder and drop the .zip; a Zarr store is read from inside the archive.";
const manyFilesMsg = (name: string, ignored: number): string =>
  `Opened ${name}. Ignored ${ignored} other ${ignored === 1 ? "file" : "files"}.`;
const discardMsg = (open: string, next: string): string =>
  `Discard unsaved edits to ${open} and open ${next}?`;
/** Says what letting go costs: the open file closes. Not "replaces", which
 *  during a drag reads as overwriting that file on disk, which never happens. */
const closesMsg = (): string => {
  const doc = activeDoc();
  if (doc === null) return "";
  const what = tabs.length > 1 ? `all ${tabs.length} open files` : doc.name;
  return `Closes ${what}${modifiedTab() !== null ? " (unsaved edits)" : ""}`;
};
const selectedBytes = (n: number): string => (n === 1 ? "1 byte" : `${n.toLocaleString()} bytes`);

/** Above the parts in the rail, for the generated template: it marks the
 *  bytes that name the format and describes nothing else. */
const SIGNATURE_NOTE = "Signature only. This template marks the bytes that identify the format and nothing else.";


/** The select value that stands for the generated template. Not a built-in
 *  name, so it can never collide with one. */
const SIGNATURE_VALUE = "generated-signature";
const signatureOption = (name: string): string =>
  name === "" ? "Template: signature only" : `Template: ${name} (signature only)`;

/**
 * Open a file from disk, in place of everything already open. Unsaved edits
 * live only in this page, so replacing a document that has them asks first.
 */
function openFile(f: File, note?: string): void {
  const edited = modifiedTab();
  if (edited !== null && !confirm(discardMsg(edited.doc.name, f.name))) return;
  void Doc.open(f).then((doc) => {
    mount(doc);
    if (note !== undefined) say(note);
  });
}

/** Open bytes lifted out of the showing document as a tab of their own. */
function openEmbedded(bytes: Uint8Array, name: string, origin: string): void {
  void Doc.open(bytesSource(bytes, name)).then((doc) => {
    tabs.push({ doc, origin });
    active = tabs.length - 1;
    show();
  });
}

/** Close one tab. Its document is gone for good, so unsaved edits ask first. */
function closeTab(i: number): void {
  const tab = tabs[i];
  if (tab === undefined) return;
  if (tab.doc.modified && !confirm(`Discard unsaved edits to ${tab.doc.name}?`)) return;
  tabs.splice(i, 1);
  if (active >= tabs.length) active = tabs.length - 1;
  else if (i < active) active -= 1;
  if (tabs.length === 0) welcome();
  else show();
}

/** The strip that swaps between tabs. Only built once there are two, so a
 *  single file looks the way it always has. */
function tabStrip(): HTMLElement {
  const strip = el("nav", { className: "tabstrip" });
  strip.setAttribute("aria-label", "Open files");
  const list = el("div", { className: "tabstrip-tabs" });
  list.setAttribute("role", "tablist");
  tabs.forEach((tab, i) => {
    const here = i === active;
    const pick = el("button", { type: "button", className: "tab-pick", textContent: tab.doc.name });
    pick.setAttribute("role", "tab");
    pick.setAttribute("aria-selected", String(here));
    if (tab.origin !== null) pick.title = tab.origin;
    if (!here) pick.addEventListener("click", () => {
      active = i;
      show();
    });
    const close = el("button", { type: "button", className: "tab-close", textContent: "×" });
    close.title = `Close ${tab.doc.name}`;
    close.setAttribute("aria-label", `Close ${tab.doc.name}`);
    close.addEventListener("click", () => closeTab(i));
    const item = el("div", { className: here ? "tab is-active" : "tab" }, pick, close);
    if (tab.doc.modified) item.classList.add("is-edited");
    list.append(item);
  });
  strip.append(list);
  return strip;
}

/** Rebuild the page for the active tab. */
function show(): void {
  const tab = tabs[active];
  if (tab !== undefined) build(tab);
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
  apply(localStorage.getItem(key) === "collapsed");
  toggle.addEventListener("click", () => {
    const collapsed = !section.classList.contains("is-collapsed");
    localStorage.setItem(key, collapsed ? "collapsed" : "open");
    apply(collapsed);
    onToggle();
  });
  return section;
}

/** Show this file on its own, closing whatever was open. */
function mount(doc: Doc): void {
  tabs = [{ doc, origin: null }];
  active = 0;
  show();
}

function build(tab: Tab): void {
  const doc = tab.doc;
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
  const overview = new OverviewPanel(doc);
  const search = new SearchBar(doc);
  // The views share one position: the hex cursor. Picking a field moves
  // it; moving it picks the field it lands in. `picking` stops that going round.
  let picking = false;
  let followWhenLoaded: number | null = null;
  let followedBit: number | null = null;

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
    overview.reveal(at.node);
    structure.setBit(bitOffset);
    listPane.setBit(bitOffset);
  };

  const goToField = (path: readonly number[]): void => {
    const n = doc.templateNode(path);
    if (n.status !== "ok") return;
    view.setHighlight({ startBit: n.node.offset_bits, endBit: n.node.offset_bits + n.node.size_bits });
    picking = true;
    view.setBitCursor(n.node.offset_bits, { pane: "hex" });
    picking = false;
    inspector.setPath(path);
  };

  structure.onPick = ({ path, startBit, endBit }) => {
    view.setHighlight({ startBit, endBit });
    picking = true;
    view.setBitCursor(startBit, { pane: "hex" });
    picking = false;
    inspector.setPath(path);
    overview.reveal(path);
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
    if (n.status === "ok") nav.recordJump(view.cursorState.bitOffset, n.node.offset_bits);
    goToField(path);
    overview.reveal(path);
    structure.reveal(path);
  };
  inspector.onOpenTab = openEmbedded;
  view.onPickField = (path) => {
    goToField(path);
    overview.reveal(path);
  };
  view.onHighlightClear = () => {
    followedBit = null;
    overview.clearSelection();
    structure.clearSelection();
  };

  const kind = fileType();
  const tmpl = el("select", { className: "tb-tmpl" });
  tmpl.setAttribute("aria-label", "Template");
  tmpl.append(el("option", { value: "", textContent: "No template" }));
  for (const n of doc.templateNames) tmpl.append(el("option", { value: n, textContent: `Template: ${templateLabel(n)}` }));
  // The generated template is not one of the built-ins, so switching back to it
  // rebuilds it rather than looking it up by name.
  let reapplySignature: (() => Promise<void>) | null = null;
  tmpl.addEventListener("change", () => {
    overview.setNote("");
    if (tmpl.value === SIGNATURE_VALUE) {
      void reapplySignature?.();
      return;
    }
    doc.setTemplate(tmpl.value === "" ? null : tmpl.value);
    // Picking a template is asking to read fields, so the panel goes back to
    // them. It is left on the raw reading only for a file that has none.
    if (tmpl.value !== "") inspector.setMode("structure");
  });
  void doc.sniffTemplate().then(async (name) => {
    const templated = name !== null;
    structure.setMatched(templated);
    if (name !== null) {
      tmpl.value = name;
      doc.setTemplate(name);
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
      if (id === null) {
        overview.setIdentity("");
        // A full template has stronger structural evidence than the generic
        // rule database. Keep its answer visible when those rules have no
        // signature for the format (as with a Bard's Tale TPW record).
        if (templated && name !== null) {
          const identity = templateTypeName(name);
          kind.named(identity);
          overview.setIdentity(identity);
          kind.details(null, builtinTemplate(name));
          void kind.addTools(doc, null, name);
        } else {
          kind.unknown();
          kind.details(null, null);
          void kind.addTools(doc, null, name);
        }
        return;
      }
      kind.named(id.message);
      overview.setIdentity(id.message);
      void kind.addTools(doc, id, name);
      if (name !== null) {
        kind.details(id, builtinTemplate(name));
        return;
      }
      // The rule that named the format also says where its signature is. That
      // is one field, but it is a field: clickable, highlighted, and true.
      const signature = await doc.signatureTemplate(id);
      if (signature === null) {
        kind.details(id, null);
        return;
      }
      const option = el("option", { value: SIGNATURE_VALUE, textContent: signatureOption(signature) });
      tmpl.append(option);
      tmpl.value = SIGNATURE_VALUE;
      overview.setNote(SIGNATURE_NOTE);
      kind.details(id, SIGNATURE_TEMPLATE);
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
  const posLabel = el("span", { className: "tb-pos" });
  const undoBtn = el("button", { type: "button", textContent: "Undo", title: "Undo (Ctrl+Z)" });
  const redoBtn = el("button", { type: "button", textContent: "Redo", title: "Redo (Ctrl+Y)" });
  const saveBtn = el("button", { type: "button", textContent: "Save as", title: "Save as a new file (Ctrl+S)" });
  const saveMsg = el("span", { className: "tb-msg" });
  saveMsg.setAttribute("role", "status");
  say = (text, warn) => {
    saveMsg.textContent = text;
    saveMsg.classList.toggle("warn", warn === true);
  };
  const save = async (): Promise<void> => {
    saveBtn.disabled = true;
    saveMsg.textContent = "Saving";
    const r = await saveDoc(doc);
    saveBtn.disabled = false;
    saveMsg.textContent =
      r.kind === "saved" ? `Saved ${formatSize(r.bytes)}` : r.kind === "cancelled" ? "" : `Save failed: ${r.message}`;
    saveMsg.classList.toggle("warn", r.kind === "failed");
  };
  saveBtn.addEventListener("click", () => void save());
  document.addEventListener("keydown", (e) => {
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
    const t = goto.value.trim().replace(/^0x/i, "");
    if (!/^[0-9a-f]+$/i.test(t)) return goto.classList.add("invalid");
    goto.classList.remove("invalid");
    const to = parseInt(t, 16);
    nav.recordJump(view.cursorState.bitOffset, to * 8);
    view.setCursor(to, { pane: "hex" });
    view.el.focus();
  });
  goto.addEventListener("input", () => goto.classList.remove("invalid"));

  const width = el("select", { className: "tb-width" });
  width.setAttribute("aria-label", "Bytes per row");
  for (const n of [8, 16, 32]) width.append(el("option", { value: String(n), textContent: `${n} per row` }));
  const narrow = window.innerWidth < 700;
  width.value = narrow ? "8" : "16";
  view.setBytesPerRow(narrow ? 8 : 16);
  width.addEventListener("change", () => view.setBytesPerRow(Number(width.value)));

  const mode = el("select", { className: "tb-mode" });
  mode.setAttribute("aria-label", "Show bytes as");
  for (const [value, label] of [["hex", "Hex"], ["binary", "Binary"]] as const) {
    mode.append(el("option", { value, textContent: label }));
  }
  mode.addEventListener("change", () => {
    const binary = mode.value === "binary";
    view.setMode(binary ? "binary" : "hex");
    // Eight binary digits per byte: a wide row has to narrow to stay readable.
    if (binary && Number(width.value) > 8) {
      width.value = "8";
      view.setBytesPerRow(8);
    }
  });

  const column = el("select", { className: "tb-col" });
  column.setAttribute("aria-label", "Column beside the bytes");
  for (const [value, label] of [
    ["text", "Text column"],
    ["fields", "Field column"],
    ["both", "Text and fields"],
  ] as const) {
    column.append(el("option", { value, textContent: label }));
  }
  const columnKey = (): string => (doc.template === null ? "qubero.column.plain" : "qubero.column.template");
  const syncColumn = (): void => {
    const saved = localStorage.getItem(columnKey());
    const c: RightColumn =
      saved === "fields" || saved === "text" || saved === "both" ? saved : doc.template === null ? "text" : "fields";
    column.value = c;
    view.setRightColumn(c);
  };
  syncColumn();
  column.addEventListener("change", () => {
    const c: RightColumn = column.value === "fields" ? "fields" : column.value === "both" ? "both" : "text";
    localStorage.setItem(columnKey(), c);
    view.setRightColumn(c);
  });

  // Hex and Listing are two readings of the same file, so they share the
  // cursor and swap in the same place rather than sitting side by side. The
  // listing carries its own bytes, so showing both would say it twice.
  // Which encoding the text view reads in. First entry lets the file decide,
  // which is right nearly always; the rest are for the files where nothing in
  // the file says, which is every capture of a DOS screen.
  const encoding = el("select", { className: "tb-enc" });
  encoding.setAttribute("aria-label", TEXTVIEW.encodingLabel);
  encoding.append(el("option", { value: "", textContent: TEXTVIEW.encodingAuto }));
  for (const name of ["UTF-8", "ASCII", "Latin-1", "CP437", "UTF-16 LE", "UTF-16 BE"]) {
    encoding.append(el("option", { value: name, textContent: name }));
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
  text.onReading = (r) => {
    reading.textContent = encoding.value === "" ? TEXTVIEW.readAs(r.encoding, r.guessed) : "";
    // Which reading of a selection the panel names first is whichever the file
    // is being read in, chosen or settled.
    inspector.textEncoding = encoding.value === "" ? r.encoding : encoding.value;
  };

  const hexBtn = el("button", { type: "button", textContent: "Hex", className: "tb-view" });
  const listBtn = el("button", { type: "button", textContent: "Listing", className: "tb-view" });
  const textBtn = el("button", { type: "button", textContent: TEXTVIEW.viewButton, className: "tb-view" });
  const views = el("div", { className: "tb-views" }, hexBtn, listBtn, textBtn);
  views.setAttribute("role", "group");
  views.setAttribute("aria-label", "View");
  /** Controls that only mean anything over the hex rows. */
  const hexOnly = [width, mode, column];
  /** Controls that only mean anything over the text. */
  const textOnly = [encoding, reading];
  /** True while the listing is showing, which is also while the hex grid's
   *  editing state is not the user's to act on. */
  let listingShowing = false;
  const setView = (which: "hex" | "listing" | "text"): void => {
    const listingOn = which === "listing";
    const textOn = which === "text";
    listingShowing = listingOn;
    view.el.hidden = which !== "hex";
    structure.el.hidden = !listingOn;
    listRow.hidden = !listingOn;
    text.el.hidden = !textOn;
    for (const c of hexOnly) c.hidden = which !== "hex";
    for (const c of textOnly) c.hidden = !textOn;
    for (const [btn, on] of [
      [hexBtn, which === "hex"],
      [listBtn, listingOn],
      [textBtn, textOn],
    ] as const) {
      btn.setAttribute("aria-pressed", String(on));
      btn.classList.toggle("is-on", on);
    }
    localStorage.setItem("qubero.view", which);
    // A hidden view ignores the cursor, since scrolling something nobody is
    // looking at only loses their place in it. So when it comes back it has
    // wherever the cursor was left to catch up on.
    if (listingOn) {
      structure.relayout();
      listPane.relayout();
      structure.setBit(view.cursorState.bitOffset);
    } else if (textOn) {
      void text.setByte(Math.floor(view.cursorState.bitOffset / 8));
    } else view.relayout();
    (listingOn ? structure.el : textOn ? text.el : view.el).focus();
    refresh();
  };
  hexBtn.addEventListener("click", () => setView("hex"));
  listBtn.addEventListener("click", () => setView("listing"));
  textBtn.addEventListener("click", () => setView("text"));
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

  const toolbar = el(
    "header",
    { className: "toolbar" },
    openBtn,
    saveBtn,
    fileLabel,
    kind.label,
    kind.info,
    encoding,
    reading,
    saveMsg,
    el("span", { className: "tb-spacer" }),
    views,
    goto,
    width,
    mode,
    column,
    tmpl,
    undoBtn,
    redoBtn,
  );

  const statusbar = el("footer", { className: "statusbar" }, posLabel);

  const refresh = (): void => {
    // What the dump said was read off the file as it was opened. Once it has
    // been edited that reading is about bytes that are no longer there, so the
    // offer goes rather than saying something that stopped being true.
    if (doc.modified) dumpBar.hidden = true;
    fileLabel.textContent = `${doc.name}${doc.modified ? " (edited)" : ""}  ${formatSize(doc.lengthBytes)}`;
    app.querySelector(".tab.is-active")?.classList.toggle("is-edited", doc.modified);
    undoBtn.disabled = !doc.canUndo;
    redoBtn.disabled = !doc.canRedo;
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
  };
  view.onSelectionChange = (r) => {
    text.setSelection(r === null ? null : r.startBit / 8, r === null ? 0 : r.endBit / 8);
    refresh();
    inspector.setSelection(r === null ? [] : [r]);
  };
  view.onCursorChange = (c) => {
    inspector.setOffset(c.bitOffset);
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
      syncColumn();
    }
    refresh();
    if (followWhenLoaded !== null) {
      // The first try was turned away for want of bytes, and `followCursor`
      // skips a bit it has already seen. Forget it, so this one goes through.
      followedBit = null;
      followCursor(followWhenLoaded);
    }
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
  app.replaceChildren(
    ...(tabs.length > 1 ? [tabStrip()] : []),
    toolbar,
    el(
      "main",
      { className: "workspace" },
      overview.el,
      el("div", { className: "left" }, dumpBar, search.el, view.el, text.el, listRow),
      right,
    ),
    statusbar,
    kind.dialog,
  );
  const saved = localStorage.getItem("qubero.view");
  setView(saved === "listing" || saved === "text" ? saved : "hex");
  document.addEventListener("keydown", (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    const key = e.key.toLowerCase();
    // Ctrl+H opens the bar with the replace row already open, since asking to
    // replace is asking for both halves of it.
    if (key === "f" || key === "h") {
      e.preventDefault();
      search.show(key === "h");
    }
  });
  view.relayout();
  refresh();
  inspector.setOffset(0);
  view.el.focus();
  // The next file dropped may be one no template covers. Fetch the rules while
  // nothing is waiting on them, so that file is named as soon as it opens.
  prefetchMagic();
  if (import.meta.env.DEV) Object.assign(window, { __qubero: { doc, view, inspector, overview, structure, listPane, text, setView } });
}

function pick(): void {
  const input = el("input", { type: "file" });
  input.addEventListener("change", () => {
    const f = input.files?.[0];
    if (f) openFile(f);
  });
  input.click();
}

/** Pick an OME-Zarr directory and open its root NGFF metadata document. */
function pickOmeZarr(): void {
  const input = el("input", { type: "file" });
  input.setAttribute("webkitdirectory", "");
  input.addEventListener("change", () => {
    const files = Array.from(input.files ?? []);
    // A root .zattrs carries multiscales for v0.1--0.4; v0.5 uses zarr.json.
    // Nested metadata describes an array, not the OME-Zarr image store.
    const metadata = files.find((f) => {
      const parts = f.webkitRelativePath.replace(/\\/g, "/").split("/");
      return parts.length === 2 && (f.name === ".zattrs" || f.name === "zarr.json");
    });
    if (metadata === undefined) {
      say("This folder has no root OME-Zarr metadata (.zattrs or zarr.json).", true);
      return;
    }
    openFile(metadata, `Opened OME-Zarr metadata from ${metadata.webkitRelativePath}.`);
  });
  input.click();
}

function welcome(): void {
  const openBtn = el("button", { type: "button", textContent: "Open a file", className: "primary" });
  openBtn.addEventListener("click", pick);
  const openOmeZarrBtn = el("button", { type: "button", textContent: "Open OME-Zarr", className: "secondary", hidden: true });
  openOmeZarrBtn.addEventListener("click", pickOmeZarr);
  const drop = el(
    "div",
    { className: "welcome" },
    el("h1", { textContent: "Qubero" }),
    el("p", { textContent: "A hex editor for files of any size. Nothing leaves your device." }),
    openBtn,
    openOmeZarrBtn,
    el("p", { className: "hint", textContent: DROP_HINT }),
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
  // A folder arrives as an item with no usable file behind it, so it has to be
  // told apart before reaching for the file.
  const first = e.dataTransfer?.items[0];
  if (first !== undefined && first.webkitGetAsEntry()?.isDirectory === true) {
    say(FOLDER_MSG, true);
    return;
  }
  const files = e.dataTransfer?.files;
  const f = files?.[0];
  if (files === undefined || f === undefined) return;
  openFile(f, files.length > 1 ? manyFilesMsg(f.name, files.length - 1) : undefined);
});
// A drag that ends outside the window, or one the browser abandons, still has
// to clear the overlay.
window.addEventListener("dragend", () => showDropzone(false));
window.addEventListener("blur", () => showDropzone(false));

const params = new URLSearchParams(location.search);
const sampleUrl = params.get("url");
if (sampleUrl !== null) {
  void fetch(sampleUrl)
    .then((r) => r.blob())
    .then((b) => Doc.open(new File([b], sampleUrl.split("/").pop() ?? "sample")))
    .then(mount);
}
const synthetic = sampleUrl !== null ? null : params.get("synthetic");
const syntheticSize = synthetic === null ? null : parseSize(synthetic);
if (syntheticSize !== null) void Doc.open(syntheticFile(syntheticSize)).then(mount);
else welcome();

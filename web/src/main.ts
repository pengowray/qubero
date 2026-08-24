import { Doc, formatBytes, formatOffset, OWN_SOURCE, prefetchMagic, type Identification, type ToolMatch } from "./doc.js";
import { HexView, type RightColumn } from "./hexview.js";
import { Inspector } from "./inspector.js";
import { saveDoc } from "./save.js";
import { parseSize, syntheticFile } from "./synthetic.js";
import { ListingView } from "./listingview.js";
import { TypeTable } from "./typetable.js";

const appEl = document.getElementById("app");
if (!appEl) throw new Error("missing #app");
const app: HTMLElement = appEl;

const formatSize = formatBytes;

/** The file that is open, so a second one can ask before replacing it. */
let current: Doc | null = null;
/** Writes to the toolbar's message slot, once there is one. */
let say: (text: string, warn?: boolean) => void = () => {};

const DROP_TITLE = "Drop to open";
const DROP_HINT = "or drop a file anywhere on this page";
const FOLDER_MSG = "Can't open folders. Drop a single file.";
const manyFilesMsg = (name: string, ignored: number): string =>
  `Opened ${name}. Ignored ${ignored} other ${ignored === 1 ? "file" : "files"}.`;
const discardMsg = (open: string, next: string): string =>
  `Discard unsaved edits to ${open} and open ${next}?`;
/** Says what letting go costs: the open file closes. Not "replaces", which
 *  during a drag reads as overwriting that file on disk, which never happens. */
const closesMsg = (doc: Doc): string => `Closes ${doc.name}${doc.modified ? " (unsaved edits)" : ""}`;

const IDENTIFYING_MSG = "Identifying file type...";
const IDENTIFY_FAILED_MSG = "Couldn't check the file type";
const IDENTIFY_FAILED_TITLE = "The identification rules failed to download.";
const UNKNOWN_TYPE_MSG = "Unknown file type";
const INFO_LABEL = "File type details";
const DIALOG_TITLE = "File type";
const DIALOG_CLOSE = "Close";
const IDENTIFIED_FROM = `Identified from the file's first bytes, using the rule database of the Unix "file" command.`;
const NO_MATCH_BODY = `No match in the rule database of the Unix "file" command.`;
const SIGNATURE_LINE =
  "The Fields table shows only the format's signature, generated from this rule. Qubero has no full template for this format.";
const SIGNATURE_NOTE = "Signature only.";
const fullTemplateLine = (name: string): string => `The Fields table uses Qubero's full ${name} template.`;
const MATCHED_AGAINST = "Matched against the signature database of the Detect It Easy project.";
const NO_TOOL_MATCH = "No matches in the Detect It Easy signature database.";
const READ_FROM_STUB = "Identified from the loader stub the compiler placed at the end of the program.";

/**
 * The database writes its categories as slugs. Two of them are not words, and
 * one needs saying what it immunised against, so they are written out here.
 * A category not in this list is shown as the database wrote it: inventing a
 * label would claim to know something the rule did not say.
 */
const CATEGORY: Record<string, string> = {
  packer: "Packer",
  cryptor: "Cryptor",
  protector: "Protector",
  compiler: "Compiler",
  converter: "Converter",
  installer: "Installer",
  linker: "Linker",
  archive: "Archive",
  format: "File format",
  data: "Data",
  extender: "DOS extender",
  sfx: "Self-extracting archive",
  "self-displayer": "Self-displaying program",
  immunizer: "Antivirus immunizer",
};

/**
 * What the bytes on screen are wrapped in comes first, then what built them,
 * then the rest. A packed file is showing compressed output rather than the
 * program, which changes how everything else on screen should be read.
 */
const CATEGORY_ORDER = [
  "cryptor",
  "protector",
  "packer",
  "sfx",
  "installer",
  "compiler",
  "linker",
  "converter",
  "extender",
];

/** The categories that change how to read the bytes, and how to say each. */
const WRAPPER: Record<string, (m: ToolMatch) => string> = {
  packer: (m) => `packed with ${nameAndVersion(m)}`,
  protector: (m) => `protected with ${nameAndVersion(m)}`,
  cryptor: (m) => `encrypted with ${nameAndVersion(m)}`,
  sfx: (m) => `self-extracting (${nameAndVersion(m)})`,
};

const categoryLabel = (slug: string): string => CATEGORY[slug] ?? slug;

/** `UPX v3.96`. The v matters: names in this database end in digits. */
const nameAndVersion = (m: ToolMatch): string => (m.version === null ? m.name : `${m.name} v${m.version}`);

/** `Packer: UPX v3.96 (1985)`, with the author's own words in the brackets. */
const toolLine = (m: ToolMatch): string => {
  const head = `${categoryLabel(m.category)}: ${nameAndVersion(m)}`;
  return m.options === null ? head : `${head} (${m.options})`;
};

const sortTools = (found: readonly ToolMatch[]): ToolMatch[] => {
  const rank = (m: ToolMatch): number => {
    const i = CATEGORY_ORDER.indexOf(m.category);
    return i === -1 ? CATEGORY_ORDER.length : i;
  };
  return [...found].sort((a, b) => rank(a) - rank(b));
};

/** What to append to the readout, for a match that changes how to read it. */
const wrapperSuffix = (found: readonly ToolMatch[]): string => {
  const m = sortTools(found).find((x) => WRAPPER[x.category] !== undefined);
  return m === undefined ? "" : ` \u00b7 ${WRAPPER[m.category]?.(m) ?? ""}`;
};
/** The select value that stands for the generated template. Not a built-in
 *  name, so it can never collide with one. */
const SIGNATURE_VALUE = "generated-signature";
const signatureOption = (name: string): string =>
  name === "" ? "Template: signature only" : `Template: ${name} (signature only)`;

/**
 * Open a file, in place of the one already open. Unsaved edits live only in
 * this tab, so replacing a file that has them asks first.
 */
function openFile(f: File, note?: string): void {
  if (current?.modified === true && !confirm(discardMsg(current.name, f.name))) return;
  void Doc.open(f).then((doc) => {
    mount(doc);
    if (note !== undefined) say(note);
  });
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> & { className?: string } = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  Object.assign(e, props);
  e.append(...children);
  return e;
}


/** A section that can be folded down to its title bar, and remembers whether
 *  it was. */
function panel(title: string, side: "bottom" | "right", content: HTMLElement, onToggle: () => void): HTMLElement {
  const key = `qubero.panel.${side}`;
  const chevron = el("span", { className: "panel-chevron" });
  const toggle = el("button", { type: "button", className: "panel-toggle" }, chevron, title);
  const section = el("section", { className: `panel panel-${side}` }, el("header", { className: "panel-bar" }, toggle), content);
  const apply = (collapsed: boolean): void => {
    section.classList.toggle("is-collapsed", collapsed);
    chevron.textContent = collapsed ? (side === "right" ? "\u25c2" : "\u25b8") : "\u25be";
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

function mount(doc: Doc): void {
  current = doc;
  const view = new HexView(doc);
  const inspector = new Inspector(doc);
  const table = new TypeTable(doc);
  const listing = new ListingView(doc);
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
    if (at.status === "pending") {
      followWhenLoaded = bitOffset;
      return;
    }
    followWhenLoaded = null;
    if (at.status !== "ok") return;
    const n = doc.templateNode(at.node);
    if (n.status === "ok") {
      view.setHighlight({ startBit: n.node.offset_bits, endBit: n.node.offset_bits + n.node.size_bits });
    }
    table.reveal(at.node);
    listing.setBit(bitOffset);
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

  table.onPick = ({ path, startBit, endBit }) => {
    view.setHighlight({ startBit, endBit });
    picking = true;
    view.setBitCursor(startBit, { pane: "hex" });
    picking = false;
    inspector.setPath(path);
    listing.reveal(path);
  };
  listing.onPick = ({ path, startBit, endBit }) => {
    view.setHighlight({ startBit, endBit });
    picking = true;
    view.setBitCursor(startBit, { pane: "hex" });
    picking = false;
    inspector.setPath(path);
    table.reveal(path);
  };
  inspector.onPick = (path) => {
    goToField(path);
    table.reveal(path);
    listing.reveal(path);
  };
  view.onPickField = (path) => {
    goToField(path);
    table.reveal(path);
  };
  view.onHighlightClear = () => {
    followedBit = null;
    table.clearSelection();
    listing.clearSelection();
  };

  const tmpl = el("select", { className: "tb-tmpl" });
  tmpl.setAttribute("aria-label", "Template");
  tmpl.append(el("option", { value: "", textContent: "No template" }));
  for (const n of doc.templateNames) tmpl.append(el("option", { value: n, textContent: `Template: ${n}` }));
  // The generated template is not one of the built-ins, so switching back to it
  // rebuilds it rather than looking it up by name.
  let reapplySignature: (() => Promise<void>) | null = null;
  tmpl.addEventListener("change", () => {
    table.setNote("");
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
          kindLabel.textContent = IDENTIFYING_MSG;
        }, 300);
    try {
      const id = await doc.identify();
      // Stop the timer the moment there is an answer, and before the rule file
      // is fetched for the template: that second wait must not be able to
      // write "identifying" over a name already on screen.
      if (waiting !== null) clearTimeout(waiting);
      if (id === null) {
        // A file with a template is not unknown, whatever the rules make of
        // it, so only a file without one says so.
        if (!templated) {
          kindLabel.textContent = UNKNOWN_TYPE_MSG;
          showDetails(null, "");
          void addToolMatches(null, name);
        }
        return;
      }
      kindLabel.textContent = id.message;
      // The toolbar copy is cut short, so the whole sentence stays reachable
      // on hover as well as in the dialog.
      kindLabel.title = id.message;
      void addToolMatches(id, name);
      if (name !== null) {
        showDetails(id, fullTemplateLine(name));
        return;
      }
      // The rule that named the format also says where its signature is. That
      // is one field, but it is a field: clickable, highlighted, and true.
      const signature = await doc.signatureTemplate(id);
      if (signature === null) {
        showDetails(id, "");
        return;
      }
      const option = el("option", { value: SIGNATURE_VALUE, textContent: signatureOption(signature) });
      tmpl.append(option);
      tmpl.value = SIGNATURE_VALUE;
      table.setNote(SIGNATURE_NOTE);
      showDetails(id, SIGNATURE_LINE);
      reapplySignature = async (): Promise<void> => {
        await doc.signatureTemplate(id);
        table.setNote(SIGNATURE_NOTE);
      };
    } catch (e) {
      console.error("identify", e);
      kindLabel.textContent = IDENTIFY_FAILED_MSG;
      kindLabel.title = IDENTIFY_FAILED_TITLE;
    } finally {
      if (waiting !== null) clearTimeout(waiting);
    }
  });

  /**
   * Ask the signature rules what made this file, and fold the answer into what
   * is already on screen. A file nothing else could name is named by this if it
   * can be, since for a .COM there is nothing else to go on.
   */
  const addToolMatches = async (id: Identification | null, template: string | null): Promise<void> => {
    try {
      tools = await doc.detectTools(id !== null);
    } catch (e) {
      console.error("detectTools", e);
      return;
    }
    const templateLine = template === null ? "" : fullTemplateLine(template);
    showDetails(id, id === null && tools.length > 0 ? "" : templateLine);
    if (tools.length === 0) return;
    if (id === null) {
      // Nothing else knew anything, so this is the answer rather than a note
      // beside one.
      const m = sortTools(tools)[0];
      if (m !== undefined) {
        const line = `Signature match: ${nameAndVersion(m)} (${m.category})`;
        kindLabel.textContent = line;
        kindLabel.title = line;
      }
      return;
    }
    const suffix = wrapperSuffix(tools);
    if (suffix !== "") kindLabel.textContent = `${id.message}${suffix}`;
  };

  const fileLabel = el("span", { className: "tb-file" });
  // What the file is, for a file no template covers. Its own element rather
  // than the message slot: a save message is an event and passes, this is a
  // fact about the file and stays.
  const kindLabel = el("span", { className: "tb-kind" });
  // The details behind the readout. A button and a dialog rather than a
  // tooltip: the rule's sentence is long, worth copying, and worth reading at
  // leisure, none of which a title attribute allows.
  const kindInfo = el("button", { type: "button", className: "tb-info", textContent: "i" });
  kindInfo.setAttribute("aria-label", INFO_LABEL);
  kindInfo.hidden = true;
  const dlgBody = el("div", { className: "dlg-body" });
  const dialog = el(
    "dialog",
    { className: "dlg" },
    el("h2", { textContent: DIALOG_TITLE }),
    dlgBody,
    el("form", { method: "dialog", className: "dlg-close" }, el("button", { type: "submit", textContent: DIALOG_CLOSE })),
  );
  kindInfo.addEventListener("click", () => dialog.showModal());
  // The dialog element covers only the middle of the screen, so a click that
  // lands on it rather than on its contents is a click on the backdrop.
  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) dialog.close();
  });

  /** Fill the dialog for one outcome, and show the button that opens it. */
  // Filled in once the signature rules have answered, so reopening the
  // dialog shows them without asking again.
  let tools: ToolMatch[] | null = null;
  const showDetails = (id: Identification | null, templateLine: string): void => {
    const rows: HTMLElement[] = [];
    const row = (label: string, value: string): void => {
      rows.push(el("div", { className: "dlg-row" }, el("span", { className: "dlg-key", textContent: label }), value));
    };
    if (id === null) {
      rows.push(el("p", { textContent: NO_MATCH_BODY }));
    } else {
      rows.push(el("p", { className: "dlg-sentence", textContent: id.message }));
      if (id.mime !== "") row("Media type", id.mime);
      if (id.ext.length > 0) row("Extensions", id.ext.join(", "));
      if (id.source !== "") row("Rule file", id.source);
      rows.push(el("p", { className: "dlg-muted", textContent: IDENTIFIED_FROM }));
    }
    // What made the file, when anything knows. Its own block after the file
    // type's, so each muted credit line sits under the answers it covers.
    if (tools !== null) {
      if (tools.length === 0) {
        rows.push(el("p", { className: "dlg-muted", textContent: NO_TOOL_MATCH }));
      } else {
        for (const m of sortTools(tools)) {
          rows.push(el("p", { className: "dlg-tool", textContent: toolLine(m) }));
        }
        // Each credit covers only the answers it found. An answer the editor
        // read out of the file itself is not the database's to be credited
        // with, and the database's rules are not this editor's.
        if (tools.some((m) => m.source !== OWN_SOURCE)) {
          rows.push(el("p", { className: "dlg-muted", textContent: MATCHED_AGAINST }));
        }
        if (tools.some((m) => m.source === OWN_SOURCE)) {
          rows.push(el("p", { className: "dlg-muted", textContent: READ_FROM_STUB }));
        }
      }
    }
    if (templateLine !== "") rows.push(el("p", { className: "dlg-muted", textContent: templateLine }));
    dlgBody.replaceChildren(...rows);
    kindInfo.hidden = false;
  };
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
    view.setCursor(parseInt(t, 16), { pane: "hex" });
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
  const hexBtn = el("button", { type: "button", textContent: "Hex", className: "tb-view" });
  const listBtn = el("button", { type: "button", textContent: "Listing", className: "tb-view" });
  const views = el("div", { className: "tb-views" }, hexBtn, listBtn);
  views.setAttribute("role", "group");
  views.setAttribute("aria-label", "How to read the file");
  /** Controls that only mean anything over the hex rows. */
  const hexOnly = [width, mode, column];
  const setView = (which: "hex" | "listing"): void => {
    const listingOn = which === "listing";
    view.el.hidden = listingOn;
    listing.el.hidden = !listingOn;
    for (const c of hexOnly) c.hidden = listingOn;
    hexBtn.setAttribute("aria-pressed", String(!listingOn));
    listBtn.setAttribute("aria-pressed", String(listingOn));
    hexBtn.classList.toggle("is-on", !listingOn);
    listBtn.classList.toggle("is-on", listingOn);
    localStorage.setItem("qubero.view", which);
    if (listingOn) listing.relayout();
    else view.relayout();
    (listingOn ? listing.el : view.el).focus();
  };
  hexBtn.addEventListener("click", () => setView("hex"));
  listBtn.addEventListener("click", () => setView("listing"));

  const openBtn = el("button", { type: "button", textContent: "Open" });
  openBtn.addEventListener("click", () => pick());

  const toolbar = el(
    "header",
    { className: "toolbar" },
    openBtn,
    saveBtn,
    fileLabel,
    kindLabel,
    kindInfo,
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
    fileLabel.textContent = `${doc.name}${doc.modified ? " (edited)" : ""}  ${formatSize(doc.lengthBytes)}`;
    undoBtn.disabled = !doc.canUndo;
    redoBtn.disabled = !doc.canRedo;
    const c = view.cursorState;
    // Inside a byte the decimal counts bits, so the two halves agree.
    const where =
      c.bitOffset % 8 === 0
        ? `Offset ${formatOffset(c.bitOffset)} (${c.offset.toLocaleString()})`
        : `Offset ${formatOffset(c.bitOffset)} (bit ${c.bitOffset.toLocaleString()})`;
    const pane = c.pane === "ascii" ? "Text" : c.mode === "binary" ? "Binary" : "Hex";
    posLabel.textContent = `${where}  ·  ${c.insertMode ? "Insert" : "Overwrite"}  ·  ${pane}`;
  };
  view.onCursorChange = (c) => {
    inspector.setOffset(c.bitOffset);
    if (!picking) followCursor(c.bitOffset);
    refresh();
  };
  let hadTemplate = doc.template;
  doc.onChange(() => {
    if (doc.template !== hadTemplate) {
      hadTemplate = doc.template;
      syncColumn();
    }
    refresh();
    if (followWhenLoaded !== null) followCursor(followWhenLoaded);
  });

  const relayout = (): void => {
    view.relayout();
    listing.relayout();
  };
  const bottom = panel("Structure", "bottom", table.el, relayout);
  const right = panel("At cursor", "right", inspector.el, relayout);
  app.replaceChildren(
    toolbar,
    el("main", { className: "workspace" }, el("div", { className: "left" }, view.el, listing.el, bottom), right),
    statusbar,
    dialog,
  );
  setView(localStorage.getItem("qubero.view") === "listing" ? "listing" : "hex");
  view.relayout();
  refresh();
  inspector.setOffset(0);
  view.el.focus();
  // The next file dropped may be one no template covers. Fetch the rules while
  // nothing is waiting on them, so that file is named as soon as it opens.
  prefetchMagic();
  if (import.meta.env.DEV) Object.assign(window, { __qubero: { doc, view, inspector, table, listing } });
}

function pick(): void {
  const input = el("input", { type: "file" });
  input.addEventListener("change", () => {
    const f = input.files?.[0];
    if (f) openFile(f);
  });
  input.click();
}

function welcome(): void {
  const openBtn = el("button", { type: "button", textContent: "Open a file", className: "primary" });
  openBtn.addEventListener("click", pick);
  const drop = el(
    "div",
    { className: "welcome" },
    el("h1", { textContent: "Qubero" }),
    el("p", { textContent: "A hex editor for files of any size. Nothing leaves your device." }),
    openBtn,
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
  dropSub.textContent = current === null ? "" : closesMsg(current);
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

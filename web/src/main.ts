import { Doc, formatBytes, formatOffset, type Identification } from "./doc.js";
import { HexView } from "./hexview.js";
import { Inspector } from "./inspector.js";
import { saveDoc } from "./save.js";
import { parseSize, syntheticFile } from "./synthetic.js";
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
/** The whole sentence, where it came from, and what it does not come with.
 *  The last line is there because "No template" sits in the same toolbar, and
 *  without it, knowing the format but offering no template reads as a fault. */
const identifyTitle = (id: Identification): string => {
  const lines = [
    id.message,
    `Identified from the file's first bytes, using the rule database of the Unix "file" command.`,
  ];
  if (id.mime !== "") lines.push(`Media type: ${id.mime}`);
  if (id.ext.length > 0) lines.push(`Extensions: ${id.ext.join(", ")}`);
  lines.push("Qubero has no field template for this format.");
  return lines.join("\n");
};

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
  // The three views share one position: the hex cursor. Picking a field moves
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
  };
  inspector.onPick = (path) => {
    goToField(path);
    table.reveal(path);
  };
  view.onPickField = (path) => {
    goToField(path);
    table.reveal(path);
  };
  view.onHighlightClear = () => {
    followedBit = null;
    table.clearSelection();
  };

  const tmpl = el("select", { className: "tb-tmpl" });
  tmpl.setAttribute("aria-label", "Template");
  tmpl.append(el("option", { value: "", textContent: "No template" }));
  for (const n of doc.templateNames) tmpl.append(el("option", { value: n, textContent: `Template: ${n}` }));
  tmpl.addEventListener("change", () => doc.setTemplate(tmpl.value === "" ? null : tmpl.value));
  void doc.sniffTemplate().then(async (name) => {
    if (name !== null) {
      tmpl.value = name;
      doc.setTemplate(name);
      return;
    }
    // Nothing to read a field from, so start on the raw reading instead.
    inspector.setMode("le");
    // The rule database is a separate download. Say so once the wait is long
    // enough to notice, so a file answered from cache says nothing at all.
    const waiting = setTimeout(() => {
      kindLabel.textContent = IDENTIFYING_MSG;
    }, 300);
    try {
      const id = await doc.identify();
      if (id === null) {
        kindLabel.textContent = UNKNOWN_TYPE_MSG;
      } else {
        kindLabel.textContent = id.message;
        kindLabel.title = identifyTitle(id);
      }
    } catch (e) {
      console.error("identify", e);
      kindLabel.textContent = IDENTIFY_FAILED_MSG;
      kindLabel.title = IDENTIFY_FAILED_TITLE;
    } finally {
      clearTimeout(waiting);
    }
  });

  const fileLabel = el("span", { className: "tb-file" });
  // What the file is, for a file no template covers. Its own element rather
  // than the message slot: a save message is an event and passes, this is a
  // fact about the file and stays.
  const kindLabel = el("span", { className: "tb-kind" });
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
  ] as const) {
    column.append(el("option", { value, textContent: label }));
  }
  const columnKey = (): string => (doc.template === null ? "qubero.column.plain" : "qubero.column.template");
  const syncColumn = (): void => {
    const saved = localStorage.getItem(columnKey());
    const c: "text" | "fields" = saved === "fields" || saved === "text" ? saved : doc.template === null ? "text" : "fields";
    column.value = c;
    view.setRightColumn(c);
  };
  syncColumn();
  column.addEventListener("change", () => {
    const c = column.value === "fields" ? "fields" : "text";
    localStorage.setItem(columnKey(), c);
    view.setRightColumn(c);
  });

  const openBtn = el("button", { type: "button", textContent: "Open" });
  openBtn.addEventListener("click", () => pick());

  const toolbar = el(
    "header",
    { className: "toolbar" },
    openBtn,
    saveBtn,
    fileLabel,
    kindLabel,
    saveMsg,
    el("span", { className: "tb-spacer" }),
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

  const relayout = (): void => view.relayout();
  const bottom = panel("Structure", "bottom", table.el, relayout);
  const right = panel("At cursor", "right", inspector.el, relayout);
  app.replaceChildren(
    toolbar,
    el("main", { className: "workspace" }, el("div", { className: "left" }, view.el, bottom), right),
    statusbar,
  );
  view.relayout();
  refresh();
  inspector.setOffset(0);
  view.el.focus();
  if (import.meta.env.DEV) Object.assign(window, { __qubero: { doc, view, inspector, table } });
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

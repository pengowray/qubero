import { Doc, formatBytes, formatOffset } from "./doc.js";
import { HexView } from "./hexview.js";
import { Inspector } from "./inspector.js";
import { saveDoc } from "./save.js";
import { parseSize, syntheticFile } from "./synthetic.js";
import { TypeTable } from "./typetable.js";

const appEl = document.getElementById("app");
if (!appEl) throw new Error("missing #app");
const app: HTMLElement = appEl;

const formatSize = formatBytes;

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

function mount(doc: Doc): void {
  const view = new HexView(doc);
  const inspector = new Inspector(doc);
  const table = new TypeTable(doc);
  table.onPick = ({ startBit, endBit }) => {
    view.setHighlight({ startBit, endBit });
    view.setBitCursor(startBit, { pane: "hex" });
  };
  view.onHighlightClear = () => table.clearSelection();

  const tmpl = el("select", { className: "tb-tmpl" });
  tmpl.setAttribute("aria-label", "Template");
  tmpl.append(el("option", { value: "", textContent: "No template" }));
  for (const n of doc.templateNames) tmpl.append(el("option", { value: n, textContent: `Template: ${n}` }));
  tmpl.addEventListener("change", () => doc.setTemplate(tmpl.value === "" ? null : tmpl.value));
  void doc.sniffTemplate().then((name) => {
    if (name !== null) {
      tmpl.value = name;
      doc.setTemplate(name);
    }
  });

  const fileLabel = el("span", { className: "tb-file" });
  const posLabel = el("span", { className: "tb-pos" });
  const undoBtn = el("button", { type: "button", textContent: "Undo", title: "Undo (Ctrl+Z)" });
  const redoBtn = el("button", { type: "button", textContent: "Redo", title: "Redo (Ctrl+Y)" });
  const saveBtn = el("button", { type: "button", textContent: "Save as", title: "Save as a new file (Ctrl+S)" });
  const saveMsg = el("span", { className: "tb-msg" });
  saveMsg.setAttribute("role", "status");
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

  const openBtn = el("button", { type: "button", textContent: "Open" });
  openBtn.addEventListener("click", () => pick());

  const toolbar = el(
    "header",
    { className: "toolbar" },
    openBtn,
    saveBtn,
    fileLabel,
    saveMsg,
    el("span", { className: "tb-spacer" }),
    goto,
    width,
    mode,
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
    inspector.setOffset(c.offset);
    refresh();
  };
  doc.onChange(refresh);

  app.replaceChildren(toolbar, el("main", { className: "workspace" }, el("div", { className: "left" }, view.el, table.el), inspector.el), statusbar);
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
    if (f) void Doc.open(f).then(mount);
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
    el("p", { className: "hint", textContent: "or drop a file anywhere on this page" }),
  );
  app.replaceChildren(drop);
}

window.addEventListener("resize", () => document.querySelector(".hexview")?.dispatchEvent(new Event("relayout")));
document.addEventListener("dragover", (e) => e.preventDefault());
document.addEventListener("drop", (e) => {
  e.preventDefault();
  const f = e.dataTransfer?.files[0];
  if (f) void Doc.open(f).then(mount);
});

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

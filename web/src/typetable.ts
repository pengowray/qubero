// Tree table of the template's fields with live values. Only expanded nodes are
// evaluated; long arrays are paged so a 100k-element array costs nothing until
// the user opens it. Editable leaf values are typed in place: the core encodes
// the text as that field's type and writes only that field's bits.

import { formatBytes, formatOffset } from "./doc.js";
import { bitSizeText, childWord, countText, GAP_LABEL } from "./strings.js";
import type { Doc, TemplateNode } from "./doc.js";
import { fieldClass } from "./fieldstyle.js";
import { appendAnatomy } from "./anatomy.js";
import type { AnatomyPart } from "./anatomy.js";
import { hasLogicalOutline, logicalLength, logicalOutline } from "./logicaloutline.js";
import type { LogicalNode, LogicalOutline } from "./logicaloutline.js";

const PAGE = 200;
/** The Value column shows a preview of a long field, so editing it here would
 * write back what the preview left out. Longer fields are edited at the cursor. */
const INLINE_LIMIT = { bytes: 16, str: 64 } as const;
/** Give up after this many chunk-loading retries on one commit. */
const WRITE_RETRIES = 8;
/** A cell whose value is still being worked out. Not zero, and not nothing. */
const NOT_YET = "…";
/** Preserve the underlying tree, but stop its parser wrappers from consuming
 * the whole Field column. Deeper levels are marked rather than further
 * indented. */
const MAX_INDENT_DEPTH = 5;
/** Opening every visible composite remains an overview rather than an
 * accidental request for thousands of rows. Repeated presses progressively
 * open the next visible level. */
const OVERVIEW_SECTIONS = 24;
const OVERVIEW_CHILDREN = 24;
/** Enough boundaries to show the shape without turning Length into a second
 * tree. The last mark represents the rest of a larger structure. */
const ANATOMY_PARTS = 6;
/** Small composites are cheap enough to preview before they are opened. This
 * is what makes length + value read immediately as two stored components. */
const EAGER_ANATOMY_CHILDREN = 6;

function treeIndent(depth: number, extra: number): string {
  return `${Math.min(depth, MAX_INDENT_DEPTH) * 16 + extra}px`;
}

export type FieldPick = { readonly path: readonly number[]; readonly startBit: number; readonly endBit: number };

type Editing = {
  readonly key: string;
  readonly path: readonly number[];
  text: string;
  caret: readonly [number, number] | null;
  waiting: boolean;
  tries: number;
};

function key(path: readonly number[]): string {
  return path.join("/");
}

function editableHere(n: TemplateNode): boolean {
  // Nothing to edit yet in a field whose bytes have not arrived.
  if (n.kind === "unread") return false;
  if (!n.editable) return false;
  const cap = n.kind === "bytes" ? INLINE_LIMIT.bytes : n.kind === "str" ? INLINE_LIMIT.str : null;
  return cap === null || n.size_bits <= cap * 8;
}

function pathOf(k: string): number[] {
  return k === "" ? [] : k.split("/").map(Number);
}

/// A named value reads as `local.get (0x20)`. Hundreds of instruction rows in a
/// row are easier to scan with the number behind the name played down.
function label(n: TemplateNode): (Node | string)[] {
  const m = n.kind === "enum" ? /^(.*) (\([^()]*\))$/.exec(n.value) : null;
  if (m === null) return [n.value];
  const num = document.createElement("span");
  num.className = "tt-num-note";
  num.textContent = m[2] ?? "";
  return [`${m[1]} `, num];
}

export class TypeTable {
  readonly el: HTMLElement;
  private readonly body: HTMLElement;
  private readonly empty: HTMLElement;
  private readonly status: HTMLElement;
  private readonly note: HTMLParagraphElement;
  /** How far the structure has been worked out, while that is still going on. */
  private readonly progress: HTMLParagraphElement;
  private readonly head: HTMLTableRowElement;
  private readonly modeGroup: HTMLElement;
  private readonly logicalButton: HTMLButtonElement;
  private readonly storageButton: HTMLButtonElement;
  private readonly openButton: HTMLButtonElement;
  private readonly collapseButton: HTMLButtonElement;
  private readonly expanded = new Set<string>();
  private readonly logicalExpanded = new Set<string>(["/"]);
  private readonly logicalShown = new Map<string, number>();
  private readonly shown = new Map<string, number>();
  /** Direct-child extents learned when a composite is opened. Keeping this
   * small preview means collapsing the row does not throw its useful shape
   * away. It is cleared whenever the document changes. */
  private readonly anatomy = new Map<string, readonly AnatomyPart[]>();
  private selected: string | null = null;
  /** Logical rows can share one storage node while pointing at different
   * byte extents, as ISO directory entries do. */
  private selectedLogicalId: string | null = null;
  private editing: Editing | null = null;
  /** Cell to focus once the table has been rebuilt. */
  private focusKey: string | null = null;
  /** True while render() is replacing the rows, so blur is not a cancel. */
  private rebuilding = false;
  private outlineMode: "logical" | "storage" = "storage";
  private outlineModeChosen = false;
  private logical: LogicalOutline | null = null;

  onPick: (pick: FieldPick) => void = () => {};
  /** Ctrl+click on a field holding an offset: go to where it points. */
  onJump: (bitOffset: number) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "typetable";
    this.el.setAttribute("aria-label", "Fields");

    const table = document.createElement("table");
    const actions = document.createElement("div");
    actions.className = "tt-actions";
    this.modeGroup = document.createElement("div");
    this.modeGroup.className = "tt-modes";
    this.modeGroup.setAttribute("role", "group");
    this.modeGroup.setAttribute("aria-label", "Outline");
    this.logicalButton = document.createElement("button");
    this.logicalButton.type = "button";
    this.logicalButton.textContent = "Logical";
    this.storageButton = document.createElement("button");
    this.storageButton.type = "button";
    this.storageButton.textContent = "Storage";
    this.logicalButton.addEventListener("click", () => this.setOutlineMode("logical"));
    this.storageButton.addEventListener("click", () => this.setOutlineMode("storage"));
    this.modeGroup.append(this.logicalButton, this.storageButton);
    this.openButton = document.createElement("button");
    this.openButton.type = "button";
    this.openButton.textContent = "Open visible sections";
    this.openButton.title = "Open one visible level, with a bounded preview of large lists";
    this.openButton.addEventListener("click", () => this.openVisibleSections());
    this.collapseButton = document.createElement("button");
    this.collapseButton.type = "button";
    this.collapseButton.textContent = "Collapse to overview";
    this.collapseButton.addEventListener("click", () => this.collapseOverview());
    actions.append(this.modeGroup, this.openButton, this.collapseButton);
    const head = document.createElement("thead");
    this.head = document.createElement("tr");
    head.append(this.head);
    this.body = document.createElement("tbody");
    table.append(head, this.body);
    this.empty = document.createElement("p");
    this.empty.className = "tt-empty";
    this.status = document.createElement("p");
    this.status.className = "tt-status";
    this.status.setAttribute("role", "status");
    this.note = document.createElement("p");
    this.note.className = "tt-note";
    this.note.hidden = true;
    this.progress = document.createElement("p");
    this.progress.className = "tt-progress";
    this.progress.setAttribute("role", "status");
    this.progress.hidden = true;
    this.el.append(actions, this.empty, table, this.progress, this.note, this.status);
    this.expanded.add("");
    this.body.addEventListener("click", (e) => this.onClick(e));
    this.body.addEventListener("keydown", (e) => this.onKey(e));
    doc.onChange(() => {
      this.anatomy.clear();
      this.logical = null;
      if (this.editing?.waiting) this.commit();
      else this.render();
    });
  }

  private setOutlineMode(mode: "logical" | "storage"): void {
    this.outlineMode = mode;
    this.outlineModeChosen = true;
    this.editing = null;
    if (mode === "storage") this.selectedLogicalId = null;
    this.render();
  }

  private collapseOverview(): void {
    if (this.outlineMode === "logical") {
      this.logicalExpanded.clear();
      this.logicalExpanded.add("/");
      this.logicalShown.clear();
    } else {
      this.expanded.clear();
      this.expanded.add("");
      this.shown.clear();
    }
    this.render();
  }

  /** Open the composites already on screen. This gives a useful broad view in
   * one press without recursively evaluating an unbounded file. */
  private openVisibleSections(): void {
    if (this.outlineMode === "logical") {
      const rows = Array.from(this.body.querySelectorAll<HTMLElement>('tr[data-logical-group="1"]'))
        .filter((row) => !this.logicalExpanded.has(row.dataset["logicalId"] ?? ""))
        .slice(0, OVERVIEW_SECTIONS);
      for (const row of rows) this.logicalExpanded.add(row.dataset["logicalId"] ?? "");
      this.render();
      return;
    }
    const rows = Array.from(this.body.querySelectorAll<HTMLElement>('tr[data-composite="1"]'))
      .filter((row) => !this.expanded.has(row.dataset["path"] ?? ""))
      .slice(0, OVERVIEW_SECTIONS);
    for (const row of rows) {
      const k = row.dataset["path"] ?? "";
      this.expanded.add(k);
      if (!this.shown.has(k)) this.shown.set(k, OVERVIEW_CHILDREN);
    }
    this.render();
  }

  /**
   * A standing note under the rows, for a template that needs one. A generated
   * signature template has a single row, which without a word of explanation
   * reads as a format Qubero supports badly rather than one it only names.
   */
  setNote(text: string): void {
    this.note.textContent = text;
    this.note.hidden = text === "";
  }

  /** Open the path down to `path` and select it, scrolling it into view. */
  reveal(path: readonly number[]): void {
    const k = key(path);
    if (k === this.selected) return;
    if (this.outlineMode === "logical" && this.logical !== null) {
      const node = this.logical.nodes.find((candidate) => key(candidate.sourcePath) === k);
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
    for (let i = 0; i < path.length; i++) this.expanded.add(key(path.slice(0, i)));
    this.selected = k;
    this.editing = null;
    this.render();
    this.body.querySelector(`tr[data-path="${CSS.escape(k)}"]`)?.scrollIntoView({ block: "nearest" });
  }

  /** Drop the selection and any half-typed value. */
  clearSelection(): void {
    this.selected = null;
    this.selectedLogicalId = null;
    this.editing = null;
    this.status.textContent = "";
    this.render();
  }

  private onClick(e: MouseEvent): void {
    const t = e.target;
    if (!(t instanceof HTMLElement)) return;
    if (t.closest(".tt-input")) return;
    const more = t.closest<HTMLElement>("[data-more]");
    if (more) {
      const k = more.dataset["more"] ?? "";
      this.shown.set(k, (this.shown.get(k) ?? PAGE) + PAGE);
      this.render();
      return;
    }
    const logicalMore = t.closest<HTMLElement>("[data-logical-more]");
    if (logicalMore) {
      const id = logicalMore.dataset["logicalMore"] ?? "";
      this.logicalShown.set(id, (this.logicalShown.get(id) ?? 80) + 80);
      this.render();
      return;
    }
    const row = t.closest<HTMLElement>("tr[data-path]");
    if (!row) return;
    const k = row.dataset["path"] ?? "";
    if (t.closest(".tt-toggle")) {
      const logicalId = row.dataset["logicalId"];
      if (logicalId !== undefined) {
        if (this.logicalExpanded.has(logicalId)) this.logicalExpanded.delete(logicalId);
        else this.logicalExpanded.add(logicalId);
        this.render();
        return;
      }
      if (this.expanded.has(k)) this.expanded.delete(k);
      else this.expanded.add(k);
      this.render();
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      const to = this.pointsAt(pathOf(row.dataset["path"] ?? ""));
      if (to !== null) {
        this.onJump(to);
        return;
      }
    }
    const startBit = Number(row.dataset["start"]);
    const endBit = Number(row.dataset["end"]);
    this.selected = k;
    this.selectedLogicalId = row.dataset["logicalId"] ?? null;
    if (t.closest("[data-edit]")) this.beginEdit(k, row.dataset["value"] ?? "");
    else this.editing = null;
    this.render();
    if (!Number.isFinite(startBit) || !Number.isFinite(endBit)) return;
    this.onPick({ path: pathOf(k), startBit, endBit });
  }

  /** The bit this field's value points at, for a field holding an offset. */
  private pointsAt(path: readonly number[]): number | null {
    const r = this.doc.origins(path);
    if (r.status !== "ok") return null;
    const to = r.node.find((o) => o.role === "points" && o.target_bits !== null);
    return to?.target_bits ?? null;
  }

  private onKey(e: KeyboardEvent): void {
    const t = e.target;
    if (!(t instanceof HTMLElement)) return;
    const cell = t.closest<HTMLElement>("[data-edit]");
    if (!cell || this.editing) return;
    if (e.key === "Enter" || e.key === "F2") {
      e.preventDefault();
      const k = cell.dataset["edit"] ?? "";
      this.selected = k;
      this.beginEdit(k, cell.closest("tr")?.dataset["value"] ?? "");
      this.render();
    }
  }

  private beginEdit(k: string, current: string): void {
    this.status.textContent = "";
    this.editing = { key: k, path: pathOf(k), text: current, caret: null, waiting: false, tries: 0 };
  }

  private cancelEdit(refocus: boolean): void {
    this.editing = null;
    this.status.textContent = "";
    this.status.classList.remove("warn");
    if (refocus) this.focusKey = this.selected;
    this.render();
  }

  private commit(): void {
    const e = this.editing;
    if (e === null) return;
    const r = this.doc.writeNode(e.path, e.text);
    if (r.status === "ok") {
      this.editing = null;
      this.focusKey = e.key;
      this.status.textContent = "";
    } else if (r.status === "pending" || r.status === "working") {
      // The field's offset depends on bytes that are not loaded yet. Doc has
      // asked for them; this runs again when they land. An edit is never
      // handed back half-done, so "working" does not arrive here.
      e.waiting = e.tries < WRITE_RETRIES;
      e.tries += 1;
      this.status.textContent = e.waiting
        ? "Loading this part of the file…"
        : "Couldn't read this part of the file. Press Enter to try again.";
      this.status.classList.toggle("warn", !e.waiting);
    } else {
      e.waiting = false;
      this.status.textContent = r.message;
      this.status.classList.add("warn");
    }
    this.render();
  }

  render(): void {
    if (this.doc.template === null) {
      this.empty.textContent = "No template selected. Pick one above to see the file's fields.";
      this.empty.hidden = false;
      this.body.replaceChildren();
      return;
    }
    this.empty.hidden = true;
    const hasLogical = hasLogicalOutline(this.doc);
    this.modeGroup.hidden = !hasLogical;
    if (hasLogical && !this.outlineModeChosen) this.outlineMode = "logical";
    if (!hasLogical) this.outlineMode = "storage";
    this.logicalButton.classList.toggle("is-on", this.outlineMode === "logical");
    this.storageButton.classList.toggle("is-on", this.outlineMode === "storage");
    this.logicalButton.setAttribute("aria-pressed", String(this.outlineMode === "logical"));
    this.storageButton.setAttribute("aria-pressed", String(this.outlineMode === "storage"));
    if (this.outlineMode === "logical") {
      this.renderLogical();
      return;
    }
    this.setHead(["Offset", "Field", "Value", "Type", "Length"]);
    const frag = document.createDocumentFragment();
    const root = this.doc.templateNode([]);
    if (root.status === "ok") {
      this.progress.hidden = true;
      this.addRows(frag, root.node, 0);
    } else if (root.status === "working" || root.status === "pending") {
      // How far a file runs is only known once every field in it has been
      // placed, and on a large file that takes a while. The fields already
      // placed are worth showing meanwhile: the head of a file is what says
      // what the file is.
      if (root.status === "working") this.showProgress(root.reachedBytes);
      this.addPlaceholderRoot(frag);
      this.addReadyChildren(frag, [], 1);
      this.addEstimateRow(frag, root.reachedBytes);
    } else {
      this.progress.hidden = true;
      this.addStatusRow(frag, root, 0, "file");
    }
    this.rebuilding = true;
    this.body.replaceChildren(frag);
    this.rebuilding = false;
    this.restoreFocus();
  }

  private setHead(labels: readonly string[]): void {
    this.head.replaceChildren(
      ...labels.map((label) => {
        const th = document.createElement("th");
        th.textContent = label;
        return th;
      }),
    );
  }

  private renderLogical(): void {
    this.setHead(["Source", "Object", "Shape / contents", "Type", "Size"]);
    const reply = logicalOutline(this.doc, this.logicalExpanded, this.logicalShown);
    const frag = document.createDocumentFragment();
    if (reply === null) {
      this.outlineMode = "storage";
      this.render();
      return;
    }
    if (reply.status !== "ok") {
      this.logical = null;
      this.progress.hidden = reply.status === "error";
      this.progress.textContent =
        reply.status === "error"
          ? ""
          : `Reading the logical outline… ${formatBytes(reply.reachedBytes)} read so far`;
      if (reply.status === "error") this.addStatusRow(frag, reply, 0, "logical outline");
      this.body.replaceChildren(frag);
      return;
    }
    this.progress.hidden = true;
    this.logical = reply.node;
    this.setHead(["Source", "Object", "Shape / contents", "Type", reply.node.sizeLabel ?? "Logical size"]);
    if (reply.node.progressText !== undefined) {
      this.progress.textContent = reply.node.progressText;
      this.progress.hidden = false;
    }
    const summary = document.createElement("tr");
    summary.className = "tt-logical-summary";
    const summaryCell = document.createElement("td");
    summaryCell.colSpan = 5;
    summaryCell.textContent = `${reply.node.title} · ${reply.node.summary}`;
    summary.append(summaryCell);
    frag.append(summary);
    const byId = new Map(reply.node.nodes.map((node) => [node.id, node]));
    const parents = new Set(reply.node.nodes.flatMap((node) => node.parentId === null ? [] : [node.parentId]));
    const moreByAfter = new Map(reply.node.more?.map((more) => [more.afterId, more]) ?? []);
    for (const node of reply.node.nodes) {
      if (!this.logicalVisible(node, byId)) continue;
      this.addLogicalRow(frag, node, node.hasChildren || parents.has(node.id));
      const more = moreByAfter.get(node.id);
      if (more !== undefined) this.addLogicalMoreRow(frag, more.sectionId, more.count, more.label);
    }
    if ((reply.node.more?.length ?? 0) === 0 && reply.node.nodes.length < reply.node.total) {
      const more = document.createElement("tr");
      more.className = "tt-skip";
      const td = document.createElement("td");
      td.colSpan = 5;
      td.textContent = `${(reply.node.total - reply.node.nodes.length).toLocaleString()} more objects not listed`;
      more.append(td);
      frag.append(more);
    }
    this.rebuilding = true;
    this.body.replaceChildren(frag);
    this.rebuilding = false;
  }

  private addLogicalMoreRow(frag: DocumentFragment, sectionId: string, count: number, label: string): void {
    const tr = document.createElement("tr");
    tr.className = "tt-more-row";
    const td = document.createElement("td");
    td.colSpan = 5;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "tt-more";
    button.dataset["logicalMore"] = sectionId;
    button.textContent = `Show ${Math.min(80, count).toLocaleString()} more · ${count.toLocaleString()} ${label} remaining`;
    td.append(button);
    tr.append(td);
    frag.append(tr);
  }

  private logicalVisible(node: LogicalNode, byId: ReadonlyMap<string, LogicalNode>): boolean {
    let parent = node.parentId;
    while (parent !== null) {
      if (!this.logicalExpanded.has(parent)) return false;
      parent = byId.get(parent)?.parentId ?? null;
    }
    return true;
  }

  private addLogicalRow(frag: DocumentFragment, node: LogicalNode, hasChildren: boolean): void {
    const tr = document.createElement("tr");
    const k = key(node.sourcePath);
    tr.dataset["path"] = k;
    tr.dataset["logicalId"] = node.id;
    if (node.sourceBits !== null) {
      tr.dataset["start"] = String(node.sourceBits);
      tr.dataset["end"] = String(node.sourceBits + 8);
    }
    tr.title = node.title;
    if (node.group) tr.dataset["logicalGroup"] = "1";
    if (this.selectedLogicalId === node.id || (this.selectedLogicalId === null && k === this.selected)) tr.classList.add("tt-selected");
    tr.classList.add(fieldClass(node.group ? "composite" : "bytes"));
    const source = document.createElement("td");
    source.className = "tt-num tt-addr";
    source.textContent = node.sourceText;
    const name = document.createElement("td");
    name.style.paddingLeft = treeIndent(node.depth, 4);
    if (hasChildren) {
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "tt-toggle";
      const open = this.logicalExpanded.has(node.id);
      toggle.textContent = open ? "▾" : "▸";
      toggle.setAttribute("aria-label", open ? "Collapse" : "Expand");
      toggle.setAttribute("aria-expanded", String(open));
      name.append(toggle);
    } else {
      const spacer = document.createElement("span");
      spacer.className = "tt-toggle tt-leaf";
      name.append(spacer);
    }
    name.append(document.createTextNode(node.label));
    const value = document.createElement("td");
    value.className = "tt-val";
    value.textContent = node.value;
    const type = document.createElement("td");
    type.className = "tt-type";
    type.textContent = node.type;
    const length = document.createElement("td");
    length.className = "tt-num tt-length";
    length.textContent = logicalLength(node);
    tr.append(source, name, value, type, length);
    frag.append(tr);
  }

  private showProgress(reachedBytes: number): void {
    const estimate = this.doc.extentEstimate();
    this.progress.textContent = estimate === null
      ? `Working out the file's structure… ${formatBytes(reachedBytes)} read so far`
      : `Estimating items… ${estimate.measured_items.toLocaleString()} of ${estimate.total_items.toLocaleString()} · ~${bitSizeText(estimate.estimated_bits)}`;
    this.progress.hidden = false;
  }

  /** The physical file extent is exact even while its internal fields are not. */
  private addPlaceholderRoot(frag: DocumentFragment): void {
    const tr = document.createElement("tr");
    tr.dataset["path"] = "";
    const name = document.createElement("td");
    name.style.paddingLeft = "4px";
    const spacer = document.createElement("span");
    spacer.className = "tt-toggle tt-leaf";
    name.append(spacer, document.createTextNode("file"));
    const value = document.createElement("td");
    const type = document.createElement("td");
    type.className = "tt-type";
    const off = document.createElement("td");
    off.className = "tt-num tt-addr";
    off.textContent = formatOffset(0);
    const size = document.createElement("td");
    size.className = "tt-num";
    size.textContent = bitSizeText(this.doc.lengthBits);
    tr.append(off, name, value, type, size);
    frag.append(tr);
  }

  /** The unparsed physical tail, kept distinct from undefined template gaps. */
  private addEstimateRow(frag: DocumentFragment, reachedBytes: number): void {
    const startBits = Math.min(this.doc.lengthBits, reachedBytes * 8);
    if (startBits >= this.doc.lengthBits) return;
    const estimate = this.doc.extentEstimate();
    const tr = document.createElement("tr");
    tr.className = "tt-estimate";
    const off = document.createElement("td");
    off.className = "tt-num tt-addr";
    off.textContent = formatOffset(startBits);
    const name = document.createElement("td");
    name.textContent = "Structure remaining";
    const value = document.createElement("td");
    value.textContent = estimate === null
      ? "estimating fields"
      : `${estimate.measured_items.toLocaleString()} of ${estimate.total_items.toLocaleString()} items measured`;
    const type = document.createElement("td");
    type.className = "tt-type";
    type.textContent = "estimating";
    const length = document.createElement("td");
    length.className = "tt-num";
    length.textContent = bitSizeText(this.doc.lengthBits - startBits);
    tr.append(off, name, value, type, length);
    frag.append(tr);
  }

  /**
   * The children of `path` that can be placed already, in order, stopping at
   * the first one that cannot. A field is placed by the fields before it, so
   * the ones that are ready are always the ones at the front.
   */
  private addReadyChildren(frag: DocumentFragment, path: readonly number[], depth: number, count = Infinity): number {
    const limit = Math.min(this.shown.get(key(path)) ?? PAGE, count);
    let waiting = 0;
    for (let i = 0; i < limit; i++) {
      const child = this.doc.templateNode([...path, i]);
      if (child.status === "ok") {
        this.addRows(frag, child.node, depth);
        continue;
      }
      waiting += 1;
      // A field is placed by the fields before it, so once one cannot be
      // placed neither can the rest: asking each of them would be a walk each.
      // Bytes are another matter, and a field waiting on bytes says nothing
      // about the next one, which may be here already.
      if (child.status === "working") return waiting;
    }
    return waiting;
  }

  /** Put the caret back where it was: an edit in progress, or the cell just left. */
  private restoreFocus(): void {
    const e = this.editing;
    if (e !== null) {
      const input = this.body.querySelector<HTMLInputElement>(".tt-input");
      if (input === null) {
        // The row is gone (an edit changed the structure around it).
        this.editing = null;
        return;
      }
      input.focus();
      if (e.caret) input.setSelectionRange(e.caret[0], e.caret[1]);
      else input.select();
      return;
    }
    if (this.focusKey !== null) {
      const cell = this.body.querySelector<HTMLElement>(`[data-edit="${CSS.escape(this.focusKey)}"]`);
      this.focusKey = null;
      cell?.focus();
    }
  }

  private addStatusRow(
    frag: DocumentFragment,
    r:
      | { status: "pending"; reachedBytes: number }
      | { status: "working"; reachedBytes: number }
      | { status: "error"; message: string },
    depth: number,
    what: string,
  ): void {
    // Work still going on is said once, under the rows, rather than again on
    // every row waiting for it.
    if (r.status === "working") {
      this.showProgress(r.reachedBytes);
      return;
    }
    const tr = document.createElement("tr");
    tr.className = r.status === "pending" ? "tt-pending" : "tt-error";
    const td = document.createElement("td");
    td.colSpan = 5;
    td.style.paddingLeft = treeIndent(depth, 8);
    td.textContent = r.status === "pending" ? `Loading ${what}` : `${what}: ${r.message}`;
    tr.append(td);
    frag.append(tr);
  }

  private addRows(frag: DocumentFragment, n: TemplateNode, depth: number): void {
    const k = key(n.path);
    const tr = document.createElement("tr");
    tr.dataset["path"] = k;
    tr.dataset["start"] = String(n.offset_bits);
    tr.dataset["end"] = String(n.offset_bits + n.size_bits);
    tr.dataset["value"] = n.edit_text;
    if (k === this.selected) tr.classList.add("tt-selected");
    if (!n.ok) tr.classList.add("tt-bad");
    tr.classList.add(fieldClass(n.kind));
    if (n.composite) tr.dataset["composite"] = "1";

    const name = document.createElement("td");
    name.style.paddingLeft = treeIndent(depth, 4);
    if (depth > MAX_INDENT_DEPTH) {
      const deeper = document.createElement("span");
      deeper.className = "tt-depth-more";
      deeper.textContent = "… ";
      deeper.title = `${depth - MAX_INDENT_DEPTH} deeper internal ${depth - MAX_INDENT_DEPTH === 1 ? "level" : "levels"}`;
      name.append(deeper);
    }
    const open = this.expanded.has(k);
    if (n.composite) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "tt-toggle";
      b.textContent = open ? "▾" : "▸";
      b.setAttribute("aria-label", open ? "Collapse" : "Expand");
      b.setAttribute("aria-expanded", String(open));
      name.append(b);
    } else {
      const spacer = document.createElement("span");
      spacer.className = "tt-toggle tt-leaf";
      name.append(spacer);
    }
    name.append(document.createTextNode(n.name));

    const value = document.createElement("td");
    value.className = `tt-val tt-${n.kind}`;
    if (this.editing?.key === k) {
      value.append(this.editor(n));
    } else if (editableHere(n)) {
      value.classList.add("tt-editable");
      value.dataset["edit"] = k;
      value.tabIndex = 0;
      value.title = "Click to edit";
      value.append(...label(n));
    } else {
      if (n.composite) {
        value.textContent = countText(n.child_count, childWord(n));
      }
      else value.append(...label(n));
    }

    const type = document.createElement("td");
    type.className = "tt-type";
    type.textContent = n.type;
    const off = document.createElement("td");
    off.className = "tt-num tt-addr";
    off.textContent = formatOffset(n.offset_bits);
    const size = document.createElement("td");
    size.className = "tt-num tt-length";
    const total = document.createElement("span");
    total.className = "length-total";
    total.textContent = bitSizeText(n.size_bits);
    size.append(total);
    let knownAnatomy = this.anatomy.get(k);
    if (
      knownAnatomy === undefined &&
      !open &&
      n.composite &&
      n.child_count > 1 &&
      n.child_count <= EAGER_ANATOMY_CHILDREN
    ) {
      const preview = this.doc.templateChildren(n.path, 0, n.child_count);
      if (preview.status === "ok") {
        const parts = this.anatomyParts(n, preview.node, true);
        if (parts.length > 1) {
          this.anatomy.set(k, parts);
          knownAnatomy = parts;
        }
      }
    }
    if (knownAnatomy !== undefined) appendAnatomy(size, knownAnatomy, n.name);
    tr.append(off, name, value, type, size);
    frag.append(tr);

    if (n.composite && open) {
      const limit = this.shown.get(k) ?? PAGE;
      const kids = this.doc.templateChildren(n.path, 0, limit);
      if (kids.status !== "ok") {
        if (kids.status === "working") this.showProgress(kids.reachedBytes);
        // Whatever is holding the rest up, the rows that are ready are worth
        // showing: a page that empties itself while one row waits for bytes
        // reads as the file being reread from the start.
        const waiting = this.addReadyChildren(frag, n.path, depth + 1, n.child_count);
        if (waiting > 0 && kids.status !== "working") this.addStatusRow(frag, kids, depth + 1, n.name);
        this.addSelectedChild(frag, n, depth, limit);
        return;
      }
      const complete = kids.node.length >= n.child_count;
      const parts = this.anatomyParts(n, kids.node, complete);
      if (parts.length > 1) {
        this.anatomy.set(k, parts);
        appendAnatomy(size, parts, n.name);
      }
      this.addChildRows(frag, n, kids.node, depth + 1, complete);
      if (n.child_count > limit) {
        const tr2 = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 5;
        td.style.paddingLeft = treeIndent(depth + 1, 8);
        const b = document.createElement("button");
        b.type = "button";
        b.dataset["more"] = k;
        b.className = "tt-more";
        b.textContent = `Show ${Math.min(PAGE, n.child_count - limit).toLocaleString()} more (${(n.child_count - limit).toLocaleString()} hidden)`;
        td.append(b);
        tr2.append(td);
        frag.append(tr2);
      }
      this.addSelectedChild(frag, n, depth, limit);
    }
  }

  /** A miniature version of the field-anatomy strip: direct children and
   * undefined stretches occupy their real share of the parent's extent. */
  private anatomyParts(parent: TemplateNode, children: readonly TemplateNode[], complete: boolean): AnatomyPart[] {
    const start = parent.offset_bits;
    const end = start + parent.size_bits;
    if (end <= start) return [];
    const runs = children
      .map((child) => ({
        start: Math.max(start, child.offset_bits),
        end: Math.min(end, child.offset_bits + child.size_bits),
        label: child.name,
      }))
      .filter((run) => run.end > run.start)
      .sort((a, b) => a.start - b.start || a.end - b.end);
    const all: AnatomyPart[] = [];
    let at = start;
    for (const run of runs) {
      if (run.start > at) all.push({ sizeBits: run.start - at, label: GAP_LABEL, gap: true, rest: false });
      const from = Math.max(at, run.start);
      if (run.end > from) all.push({ sizeBits: run.end - from, label: run.label, gap: false, rest: false });
      at = Math.max(at, run.end);
      if (at >= end) break;
    }
    if (at < end) {
      all.push({
        sizeBits: end - at,
        label: complete ? GAP_LABEL : `${Math.max(0, parent.child_count - children.length).toLocaleString()} more`,
        gap: complete,
        rest: !complete,
      });
    }
    if (all.length <= ANATOMY_PARTS) return all;
    const head = all.slice(0, ANATOMY_PARTS - 1);
    const tail = all.slice(ANATOMY_PARTS - 1);
    head.push({
      sizeBits: tail.reduce((sum, part) => sum + part.sizeBits, 0),
      label: `${tail.length.toLocaleString()} more parts`,
      gap: false,
      rest: true,
    });
    return head;
  }

  /** Children in file order, with the stretches the template leaves undefined
   * made explicit just as they are in Listing. Trailing slack is only shown
   * when every child is present; a paged list may still fill it. */
  private addChildRows(
    frag: DocumentFragment,
    parent: TemplateNode,
    children: readonly TemplateNode[],
    depth: number,
    complete: boolean,
  ): void {
    let at = parent.offset_bits;
    const end = parent.offset_bits + parent.size_bits;
    for (const child of children) {
      if (child.offset_bits > at && child.offset_bits < end) this.addGapRow(frag, at, Math.min(child.offset_bits, end), depth);
      this.addRows(frag, child, depth);
      const childEnd = child.offset_bits + child.size_bits;
      if (child.offset_bits <= end) at = Math.max(at, Math.min(childEnd, end));
    }
    if (complete && at < end) this.addGapRow(frag, at, end, depth);
  }

  private addGapRow(frag: DocumentFragment, startBit: number, endBit: number, depth: number): void {
    if (endBit <= startBit) return;
    const tr = document.createElement("tr");
    tr.className = "tt-gap";
    const off = document.createElement("td");
    off.className = "tt-num tt-addr";
    off.textContent = formatOffset(startBit);
    const name = document.createElement("td");
    name.style.paddingLeft = treeIndent(depth, 4);
    const spacer = document.createElement("span");
    spacer.className = "tt-toggle tt-leaf";
    name.append(spacer, document.createTextNode(GAP_LABEL));
    const value = document.createElement("td");
    value.textContent = "";
    const type = document.createElement("td");
    type.className = "tt-type";
    type.textContent = "undefined";
    const length = document.createElement("td");
    length.className = "tt-num";
    length.textContent = bitSizeText(endBit - startBit);
    tr.append(off, name, value, type, length);
    frag.append(tr);
  }

  /**
   * The one child on the way to the selected field, when it sits past the end
   * of the page. Opening a list of four million blocks at the block the cursor
   * is in must not mean drawing the four million before it, so that row is
   * fetched on its own and shown under the page with the run between them
   * marked. Only the selected path gets this: one row, not a second page.
   */
  private addSelectedChild(frag: DocumentFragment, n: TemplateNode, depth: number, limit: number): void {
    const i = this.selectedChildIndex(n.path);
    if (i === null || i < limit || i >= n.child_count) return;
    const child = this.doc.templateNode([...n.path, i]);
    const skipped = i - limit;
    if (skipped > 0) {
      const tr = document.createElement("tr");
      tr.className = "tt-skip";
      const td = document.createElement("td");
      td.colSpan = 5;
      td.style.paddingLeft = treeIndent(depth + 1, 8);
      td.textContent = `${countText(skipped, childWord(n))} hidden`;
      tr.append(td);
      frag.append(tr);
    }
    if (child.status === "ok") this.addRows(frag, child.node, depth + 1);
    else this.addStatusRow(frag, child, depth + 1, `${n.name}[${i}]`);
  }

  /** Which child of `path` the selected field is inside, if any. */
  private selectedChildIndex(path: readonly number[]): number | null {
    if (this.selected === null) return null;
    const sel = pathOf(this.selected);
    if (sel.length <= path.length) return null;
    for (let i = 0; i < path.length; i++) if (sel[i] !== path[i]) return null;
    return sel[path.length] ?? null;
  }

  private editor(n: TemplateNode): HTMLInputElement {
    const input = document.createElement("input");
    input.type = "text";
    input.className = "tt-input";
    input.spellcheck = false;
    input.autocomplete = "off";
    input.value = this.editing?.text ?? n.value;
    input.setAttribute("aria-label", `${n.name}, ${n.type}`);
    if (this.status.classList.contains("warn")) input.classList.add("invalid");
    input.addEventListener("input", () => {
      if (this.editing === null) return;
      this.editing.text = input.value;
      this.editing.caret = [input.selectionStart ?? 0, input.selectionEnd ?? 0];
      input.classList.remove("invalid");
      this.status.textContent = "";
      this.status.classList.remove("warn");
    });
    const track = (): void => {
      if (this.editing) this.editing.caret = [input.selectionStart ?? 0, input.selectionEnd ?? 0];
    };
    input.addEventListener("keyup", track);
    input.addEventListener("click", track);
    input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") {
        e.preventDefault();
        if (this.editing) this.editing.tries = 0; // a manual retry gets the full run of attempts
        this.commit();
      } else if (e.key === "Escape") {
        e.preventDefault();
        this.cancelEdit(true);
      }
    });
    input.addEventListener("blur", () => {
      if (this.rebuilding) return;
      // Let the click that moved focus land first: rebuilding the table here
      // would delete the row it is headed for. If that click opened another
      // editor, `editing` is a different object and this cancel is stale.
      const mine = this.editing;
      setTimeout(() => {
        if (this.editing === mine) this.cancelEdit(false);
      }, 0);
    });
    return input;
  }
}

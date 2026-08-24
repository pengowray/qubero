// Tree table of the template's fields with live values. Only expanded nodes are
// evaluated; long arrays are paged so a 100k-element array costs nothing until
// the user opens it. Editable leaf values are typed in place: the core encodes
// the text as that field's type and writes only that field's bits.

import { formatBytes, formatOffset } from "./doc.js";
import { countText } from "./strings.js";
import type { Doc, TemplateNode } from "./doc.js";

const PAGE = 200;
/** The Value column shows a preview of a long field, so editing it here would
 * write back what the preview left out. Longer fields are edited at the cursor. */
const INLINE_LIMIT = { bytes: 16, str: 64 } as const;
/** Give up after this many chunk-loading retries on one commit. */
const WRITE_RETRIES = 8;
/** A cell whose value is still being worked out. Not zero, and not nothing. */
const NOT_YET = "…";

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

function sizeText(bits: number): string {
  if (bits % 8 === 0) {
    const b = bits / 8;
    return b === 1 ? "1 byte" : `${b.toLocaleString()} bytes`;
  }
  return bits === 1 ? "1 bit" : `${bits.toLocaleString()} bits`;
}

export class TypeTable {
  readonly el: HTMLElement;
  private readonly body: HTMLElement;
  private readonly empty: HTMLElement;
  private readonly status: HTMLElement;
  private readonly note: HTMLParagraphElement;
  /** How far the structure has been worked out, while that is still going on. */
  private readonly progress: HTMLParagraphElement;
  private readonly expanded = new Set<string>();
  private readonly shown = new Map<string, number>();
  private selected: string | null = null;
  private editing: Editing | null = null;
  /** Cell to focus once the table has been rebuilt. */
  private focusKey: string | null = null;
  /** True while render() is replacing the rows, so blur is not a cancel. */
  private rebuilding = false;

  onPick: (pick: FieldPick) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "typetable";
    this.el.setAttribute("aria-label", "Fields");

    const table = document.createElement("table");
    const head = document.createElement("thead");
    head.innerHTML = "<tr><th>Offset</th><th>Field</th><th>Value</th><th>Type</th><th>Size</th></tr>";
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
    this.el.append(this.empty, table, this.progress, this.note, this.status);
    this.expanded.add("");
    this.body.addEventListener("click", (e) => this.onClick(e));
    this.body.addEventListener("keydown", (e) => this.onKey(e));
    doc.onChange(() => {
      if (this.editing?.waiting) this.commit();
      else this.render();
    });
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
    for (let i = 0; i < path.length; i++) this.expanded.add(key(path.slice(0, i)));
    this.selected = k;
    this.editing = null;
    this.render();
    this.body.querySelector(`tr[data-path="${CSS.escape(k)}"]`)?.scrollIntoView({ block: "nearest" });
  }

  /** Drop the selection and any half-typed value. */
  clearSelection(): void {
    this.selected = null;
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
    const row = t.closest<HTMLElement>("tr[data-path]");
    if (!row) return;
    const k = row.dataset["path"] ?? "";
    if (t.closest(".tt-toggle")) {
      if (this.expanded.has(k)) this.expanded.delete(k);
      else this.expanded.add(k);
      this.render();
      return;
    }
    const startBit = Number(row.dataset["start"]);
    const endBit = Number(row.dataset["end"]);
    this.selected = k;
    if (t.closest("[data-edit]")) this.beginEdit(k, row.dataset["value"] ?? "");
    else this.editing = null;
    this.render();
    this.onPick({ path: pathOf(k), startBit, endBit });
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
    } else {
      this.progress.hidden = true;
      this.addStatusRow(frag, root, 0, "file");
    }
    this.rebuilding = true;
    this.body.replaceChildren(frag);
    this.rebuilding = false;
    this.restoreFocus();
  }

  private showProgress(reachedBytes: number): void {
    this.progress.textContent = `Working out the file's structure… ${formatBytes(reachedBytes)} read so far`;
    this.progress.hidden = false;
  }

  /** The row for the file itself, before its length is known. */
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
    size.className = "tt-num tt-not-yet";
    size.textContent = NOT_YET;
    tr.append(off, name, value, type, size);
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
    td.style.paddingLeft = `${depth * 16 + 8}px`;
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

    const name = document.createElement("td");
    name.style.paddingLeft = `${depth * 16 + 4}px`;
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
      // A structure has named fields; a list has whatever the format calls
      // its children, and items when it has no word for them. A list reads as
      // `X[]`, or as `offsets → X` when its children sit where an earlier
      // array of offsets says.
      if (n.composite) {
        const list = n.type.endsWith("[]") || n.type.startsWith("offsets ");
        value.textContent = countText(n.child_count, list ? (n.unit ?? "item") : "field");
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
    size.className = "tt-num";
    size.textContent = sizeText(n.size_bits);
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
      for (const c of kids.node) this.addRows(frag, c, depth + 1);
      if (n.child_count > limit) {
        const tr2 = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 5;
        td.style.paddingLeft = `${(depth + 1) * 16 + 8}px`;
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
      td.style.paddingLeft = `${(depth + 1) * 16 + 8}px`;
      const list = n.type.endsWith("[]") || n.type.startsWith("offsets ");
      td.textContent = `${countText(skipped, list ? (n.unit ?? "item") : "field")} between`;
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

// Tree table of the template's fields with live values. Only expanded nodes are
// evaluated; long arrays are paged so a 100k-element array costs nothing until
// the user opens it. Editable leaf values are typed in place: the core encodes
// the text as that field's type and writes only that field's bits.

import type { Doc, TemplateNode } from "./doc.js";

const PAGE = 200;
/** Give up after this many chunk-loading retries on one commit. */
const WRITE_RETRIES = 8;

export type FieldPick = { readonly startByte: number; readonly endByte: number };

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

function pathOf(k: string): number[] {
  return k === "" ? [] : k.split("/").map(Number);
}

function hexOffset(bits: number): string {
  const byte = Math.floor(bits / 8);
  const rem = bits % 8;
  return `0x${byte.toString(16).toUpperCase()}${rem ? `+${rem}b` : ""}`;
}

function countText(n: number, noun: string): string {
  return `${n.toLocaleString()} ${noun}${n === 1 ? "" : "s"}`;
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
    head.innerHTML = "<tr><th>Field</th><th>Value</th><th>Type</th><th>Offset</th><th>Size</th></tr>";
    this.body = document.createElement("tbody");
    table.append(head, this.body);
    this.empty = document.createElement("p");
    this.empty.className = "tt-empty";
    this.status = document.createElement("p");
    this.status.className = "tt-status";
    this.status.setAttribute("role", "status");
    this.el.append(this.empty, table, this.status);
    this.expanded.add("");
    this.body.addEventListener("click", (e) => this.onClick(e));
    this.body.addEventListener("keydown", (e) => this.onKey(e));
    doc.onChange(() => {
      if (this.editing?.waiting) this.commit();
      else this.render();
    });
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
    const start = Number(row.dataset["start"]);
    const end = Number(row.dataset["end"]);
    this.selected = k;
    if (t.closest("[data-edit]")) this.beginEdit(k, row.dataset["value"] ?? "");
    else this.editing = null;
    this.render();
    this.onPick({ startByte: start, endByte: end });
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
    } else if (r.status === "pending") {
      // The field's offset depends on bytes that are not loaded yet. Doc has
      // asked for them; this runs again when they land.
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
    if (root.status === "ok") this.addRows(frag, root.node, 0);
    else this.addStatusRow(frag, root, 0, "file");
    this.rebuilding = true;
    this.body.replaceChildren(frag);
    this.rebuilding = false;
    this.restoreFocus();
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

  private addStatusRow(frag: DocumentFragment, r: { status: "pending" } | { status: "error"; message: string }, depth: number, what: string): void {
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
    tr.dataset["start"] = String(Math.floor(n.offset_bits / 8));
    tr.dataset["end"] = String(Math.ceil((n.offset_bits + n.size_bits) / 8));
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
    } else if (n.editable) {
      value.classList.add("tt-editable");
      value.dataset["edit"] = k;
      value.tabIndex = 0;
      value.title = "Click to edit";
      value.append(...label(n));
    } else {
      if (n.composite) value.textContent = countText(n.child_count, n.type.endsWith("[]") ? "item" : "field");
      else value.append(...label(n));
    }

    const type = document.createElement("td");
    type.className = "tt-type";
    type.textContent = n.type;
    const off = document.createElement("td");
    off.className = "tt-num";
    off.textContent = hexOffset(n.offset_bits);
    const size = document.createElement("td");
    size.className = "tt-num";
    size.textContent = sizeText(n.size_bits);
    tr.append(name, value, type, off, size);
    frag.append(tr);

    if (n.composite && open) {
      const limit = this.shown.get(k) ?? PAGE;
      const kids = this.doc.templateChildren(n.path, 0, limit);
      if (kids.status !== "ok") {
        this.addStatusRow(frag, kids, depth + 1, n.name);
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
    }
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

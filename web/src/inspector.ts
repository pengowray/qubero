// Value inspector: interprets the bytes under the cursor as common primitive
// types and writes edits back. Every row is a small two-way lens: decode bytes
// to text, parse text to bytes.
//
// The cursor is a bit position, so these readings start wherever it is: put the
// cursor three bits into a byte and the rows show what a u16 there would say.

import { formatOffset } from "./doc.js";
import type { Doc, TemplateNode } from "./doc.js";
import { LENSES, type Lens } from "./lenses.js";
import { countText } from "./strings.js";
import { typePanel } from "./typepanel.js";

/** Structure reads the template's field; the other two read raw bytes. */
type Mode = "structure" | "le" | "be";

export class Inspector {
  readonly el: HTMLElement;
  private mode: Mode = "structure";
  /** Absolute bit position of the cursor. */
  private offset = 0;
  private readonly inputs = new Map<Lens, HTMLInputElement>();
  private readonly status: HTMLElement;
  private readonly seg: HTMLElement;
  private readonly table: HTMLElement;
  private readonly struct: HTMLElement;
  private readonly crumbs: HTMLElement;
  private readonly field: HTMLInputElement;
  /** Long values (bytes, text) are edited here instead, wrapped over lines. */
  private readonly area: HTMLTextAreaElement;
  private readonly note: HTMLElement;
  private readonly detail: HTMLElement;
  private readonly fieldRow: HTMLElement;
  private readonly types: HTMLElement;
  /** Path of the field the structure panel is showing, if any. */
  private at: readonly number[] | null = null;
  /** A field picked by name stays shown until the cursor moves off it. */
  private pinned: readonly number[] | null = null;

  /** Asked for when a breadcrumb is clicked, so the views can follow. */
  onPick: (path: readonly number[]) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "inspector";
    this.el.setAttribute("aria-label", "Value at cursor");

    const head = document.createElement("div");
    head.className = "insp-head";
    const seg = document.createElement("div");
    seg.className = "seg";
    seg.setAttribute("role", "radiogroup");
    seg.setAttribute("aria-label", "Interpret the bytes at the cursor as");
    this.seg = seg;
    for (const [value, label] of [["structure", "Field"], ["le", "Little-endian"], ["be", "Big-endian"]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.setAttribute("role", "radio");
      b.setAttribute("aria-checked", String(value === this.mode));
      b.addEventListener("click", () => {
        this.mode = value;
        for (const c of seg.children) c.setAttribute("aria-checked", String(c === b));
        this.render();
      });
      seg.append(b);
    }
    head.append(seg);

    const table = document.createElement("table");
    table.className = "insp-table";
    for (const lens of LENSES) {
      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = lens.label;
      const td = document.createElement("td");
      const input = document.createElement("input");
      input.type = "text";
      input.spellcheck = false;
      input.autocomplete = "off";
      input.setAttribute("aria-label", lens.label);
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") this.commit(lens, input);
        if (e.key === "Escape") {
          input.dataset["dirty"] = "0";
          this.render();
        }
      });
      input.addEventListener("blur", () => {
        if (input.dataset["dirty"] === "1") this.commit(lens, input);
      });
      input.addEventListener("input", () => {
        input.dataset["dirty"] = "1";
        input.classList.remove("invalid");
      });
      td.append(input);
      tr.append(th, td);
      table.append(tr);
      this.inputs.set(lens, input);
    }

    this.table = table;

    // Structure panel: where the cursor is in the template, and that field's value.
    this.struct = document.createElement("div");
    this.struct.className = "insp-struct";
    this.crumbs = document.createElement("div");
    this.crumbs.className = "insp-crumbs";
    this.crumbs.addEventListener("click", (e) => {
      const t = e.target;
      if (!(t instanceof HTMLElement)) return;
      const p = t.dataset["path"];
      if (p !== undefined) this.onPick(p === "" ? [] : p.split("/").map(Number));
    });
    this.field = document.createElement("input");
    this.field.type = "text";
    this.field.spellcheck = false;
    this.field.autocomplete = "off";
    this.field.className = "insp-field";
    this.field.addEventListener("keydown", (e) => {
      if (e.key === "Enter") this.commitField();
      if (e.key === "Escape") {
        this.field.dataset["dirty"] = "0";
        this.clearError();
      }
    });
    this.field.addEventListener("blur", () => {
      if (this.field.dataset["dirty"] === "1") this.commitField();
    });
    this.field.addEventListener("input", () => {
      this.field.dataset["dirty"] = "1";
      this.field.classList.remove("invalid");
    });
    this.area = document.createElement("textarea");
    this.area.className = "insp-area";
    this.area.spellcheck = false;
    this.area.rows = 3;
    this.area.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        this.commitField();
      }
      if (e.key === "Escape") {
        this.area.dataset["dirty"] = "0";
        this.clearError();
      }
      e.stopPropagation();
    });
    this.area.addEventListener("blur", () => {
      if (this.area.dataset["dirty"] === "1") this.commitField();
    });
    this.area.addEventListener("input", () => {
      this.area.dataset["dirty"] = "1";
      this.area.classList.remove("invalid");
    });

    this.note = document.createElement("div");
    this.note.className = "insp-note";
    this.detail = document.createElement("div");
    this.detail.className = "insp-detail";
    this.fieldRow = document.createElement("div");
    this.fieldRow.className = "insp-fieldrow";
    // What the type permits, under the editor: the values an enum names, the
    // bytes a magic field wanted, the meaning of each bit of a flags field.
    // Absent for a type whose value already says everything.
    this.types = document.createElement("div");
    this.types.className = "insp-type";
    this.types.hidden = true;
    this.fieldRow.append(this.field, this.area, this.note, this.detail, this.types);
    this.struct.append(this.crumbs, this.fieldRow);

    this.status = document.createElement("div");
    this.status.className = "insp-status";
    this.status.setAttribute("role", "status");

    this.el.append(head, this.struct, table, this.status);
    doc.onChange(() => this.render());
  }

  /** `bitOffset` is absolute, counting from the top bit of byte 0. */
  setOffset(bitOffset: number): void {
    this.offset = bitOffset;
    this.pinned = null;
    this.render();
  }

  /** Pick which reading is shown. Used once at startup for a file with no
   * template, where the field reading would be empty. */
  setMode(mode: Mode): void {
    this.mode = mode;
    for (const c of this.seg.children) {
      c.setAttribute("aria-checked", String(c instanceof HTMLElement && c.textContent === modeLabel(mode)));
    }
    this.render();
  }

  /** Show this field rather than the innermost one at the cursor. */
  setPath(path: readonly number[]): void {
    this.pinned = path;
    this.render();
  }

  /** Drop a rejection message and put the stored value back. */
  private clearError(): void {
    this.status.textContent = "";
    this.field.classList.remove("invalid");
    this.area.classList.remove("invalid");
    this.render();
  }

  private commitField(): void {
    const widget: HTMLInputElement | HTMLTextAreaElement = this.area.hidden ? this.field : this.area;
    widget.dataset["dirty"] = "0";
    if (this.at === null) return;
    const r = this.doc.writeNode(this.at, widget.value);
    if (r.status === "error") {
      widget.classList.add("invalid");
      this.status.textContent = r.message;
      return;
    }
    if (r.status === "pending" || r.status === "working") {
      this.status.textContent = "Loading this part of the file…";
      return;
    }
    this.status.textContent = "";
  }

  private commit(lens: Lens, input: HTMLInputElement): void {
    input.dataset["dirty"] = "0";
    const bytes = lens.encode(input.value, this.mode === "le");
    if (bytes === null) {
      input.classList.add("invalid");
      this.status.textContent = `Not a valid ${lens.label} value.`;
      return;
    }
    this.doc.overwriteBits(this.offset, bytes, bytes.length * 8);
    this.status.textContent = "";
  }

  render(): void {
    const structure = this.mode === "structure";
    this.struct.hidden = !structure;
    this.table.hidden = structure;
    if (structure) return this.renderStructure();

    const { bytes, complete } = this.doc.readBits(this.offset, 64);
    const avail = Math.max(0, Math.floor((this.doc.lengthBits - this.offset) / 8));
    const view = new DataView(bytes.buffer);
    for (const [lens, input] of this.inputs) {
      if (input.dataset["dirty"] === "1" && document.activeElement === input) continue;
      const fits = lens.size <= avail;
      input.disabled = !fits;
      input.classList.remove("invalid");
      if (!fits) {
        input.value = "";
        input.placeholder = avail === 0 ? "end of file" : `needs ${lens.size} bytes, ${avail} left`;
      } else if (!complete) {
        input.value = "";
        input.placeholder = "loading";
      } else {
        input.placeholder = "";
        input.value = lens.decode(view, this.mode === "le");
      }
    }
  }

  /** Where the cursor is in the template, and what that field holds. */
  private renderStructure(): void {
    if (this.doc.template === null) {
      this.at = null;
      // The Fields table below says where to pick one; saying it twice is noise.
      this.crumbs.textContent = "No template selected.";
      this.fieldRow.hidden = true;
      this.status.textContent = "";
      return;
    }
    const found = this.pinned === null ? this.doc.locate(this.offset) : ({ status: "ok", node: this.pinned } as const);
    if (found.status !== "ok") {
      this.at = null;
      this.crumbs.textContent =
        found.status === "pending" || found.status === "working"
          ? "Loading this part of the file…"
          : "No field at this offset.";
      this.fieldRow.hidden = true;
      return;
    }
    const path: readonly number[] = found.node;
    const node = this.doc.templateNode(path);
    if (node.status !== "ok") {
      this.at = null;
      this.crumbs.textContent =
        node.status === "pending" || node.status === "working" ? "Loading this part of the file…" : node.message;
      this.fieldRow.hidden = true;
      return;
    }
    this.at = path;
    const n = node.node;
    this.crumbs.replaceChildren(...this.trail(path));
    this.fieldRow.hidden = false;
    const at = document.createElement("span");
    at.className = "addr";
    at.textContent = formatOffset(n.offset_bits);
    this.detail.replaceChildren(`${n.type} · `, at, ` · ${sizeText(n.size_bits)}`);
    const long = !n.composite && (n.kind === "bytes" || n.kind === "str");
    this.area.hidden = !long;
    this.field.hidden = long;
    if (long) this.fillArea(n);
    else this.fillField(n);
    this.fillTypes(path, n);
  }

  /** The section below the editor. See `insp-type`. */
  private fillTypes(path: readonly number[], n: TemplateNode): void {
    const reply = this.doc.typeInfo(path);
    if (reply.status !== "ok" || reply.node.kind === "plain") {
      this.types.hidden = true;
      this.types.replaceChildren();
      return;
    }
    this.types.replaceChildren(typePanel(reply.node, path, n, (p, text) => this.applyValue(p, text)));
    this.types.hidden = false;
  }

  /** Write a value chosen from the type section rather than typed. */
  private applyValue(path: readonly number[], text: string): void {
    const r = this.doc.writeNode(path, text);
    if (r.status === "error") this.status.textContent = r.message;
    else this.status.textContent = "";
  }

  private fillField(n: TemplateNode): void {
    this.note.hidden = true;
    if (this.field.dataset["dirty"] === "1" && document.activeElement === this.field) return;
    this.field.disabled = !n.editable;
    this.field.classList.remove("invalid");
    this.field.value = n.composite ? "" : n.edit_text;
    const noun = n.type.endsWith("[]") || n.type.startsWith("offsets ") ? (n.unit ?? "item") : "field";
    this.field.placeholder = n.composite ? countText(n.child_count, noun) : "";
    this.field.setAttribute("aria-label", `${n.name}, ${n.type}`);
  }

  /**
   * The whole value, wrapped over as many lines as it takes: hex pairs for a
   * byte field, the text itself for a text field. Read from the document rather
   * than from the node, whose value is a preview once a field gets long.
   */
  private fillArea(n: TemplateNode): void {
    if (this.area.dataset["dirty"] === "1" && document.activeElement === this.area) return;
    this.area.classList.remove("invalid");
    this.area.setAttribute("aria-label", `${n.name}, ${n.type}`);
    this.area.placeholder = "";
    const shown = n.kind === "str" ? this.readText(n) : this.readHex(n);
    if (shown === null) {
      this.area.value = "";
      this.area.placeholder = "Loading this part of the file…";
      this.area.disabled = true;
      this.note.hidden = true;
      return;
    }
    let note = shown.note ?? "";
    if (shown.truncated) {
      note = `Showing the first ${SHOW_LIMIT.toLocaleString()} bytes of ${n.value_bytes.toLocaleString()}. Too long to edit here; use the hex view.`;
    }
    const editable = n.editable && !shown.truncated;
    this.area.value = shown.text;
    this.area.disabled = !editable;
    this.area.rows = Math.max(2, Math.min(12, Math.ceil(shown.text.length / 30)));
    // Enter puts a newline into the value, so the way to apply has to be said.
    // A note only appears when editing is off, so the two never collide.
    if (editable && note === "") note = "Ctrl+Enter to apply";
    this.note.textContent = note;
    this.note.hidden = note === "";
  }

  /** Text comes decoded from the core, which knows the field's encoding. */
  private readText(n: TemplateNode): { text: string; truncated: boolean; note: string | null } | null {
    const r = this.doc.fieldText(n.path);
    if (r.status === "pending" || r.status === "working") return null;
    if (r.status === "error") return { text: "", truncated: false, note: r.message };
    return { text: r.node.text, truncated: r.node.truncated, note: n.read_as };
  }

  /** A byte field is its own value: hex pairs, wrapped. */
  private readHex(n: TemplateNode): { text: string; truncated: boolean; note: string | null } | null {
    const total = n.value_bytes;
    const shown = Math.min(total, SHOW_LIMIT);
    const { bytes, complete } = this.doc.readBits(n.value_offset_bits, shown * 8);
    if (!complete) return null;
    return { text: hexText(bytes), truncated: total > shown, note: null };
  }

  /**
   * Every step from the root down, each one selectable. A list and the element
   * taken from it are one crumb, `boxes[0]`, because two crumbs for one step
   * doubles the length of a deep path without saying more.
   */
  private trail(path: readonly number[]): HTMLElement[] {
    const out: HTMLElement[] = [];
    for (let i = 0; i <= path.length; i++) {
      const node = this.doc.templateNode(path.slice(0, i));
      if (node.status !== "ok") {
        out.push(this.crumb("?", path.slice(0, i), i === path.length));
        continue;
      }
      const n = node.node;
      const isList = n.composite && n.type.endsWith("[]");
      if (isList && i < path.length) {
        // Fold the element index into the list's own name.
        const to = path.slice(0, i + 1);
        out.push(this.crumb(`${n.name}[${path[i]}]`, to, i + 1 === path.length));
        i += 1;
        continue;
      }
      // A struct field is often called `body`; its type says what it holds.
      const label = n.composite && n.type !== n.name ? `${n.name} (${n.type})` : n.name;
      out.push(this.crumb(label, path.slice(0, i), i === path.length));
    }
    return out;
  }

  private crumb(label: string, path: readonly number[], here: boolean): HTMLElement {
    const b = document.createElement("button");
    b.type = "button";
    b.className = here ? "insp-crumb insp-crumb-here" : "insp-crumb";
    b.dataset["path"] = path.join("/");
    b.textContent = label;
    return b;
  }
}

/** How much of a long field the panel reads; the core stops editing there too. */
const SHOW_LIMIT = 4096;

function hexText(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(" ");
}

function modeLabel(mode: Mode): string {
  return mode === "structure" ? "Field" : mode === "le" ? "Little-endian" : "Big-endian";
}

function sizeText(bits: number): string {
  if (bits % 8 === 0) {
    const b = bits / 8;
    return b === 1 ? "1 byte" : `${b.toLocaleString()} bytes`;
  }
  return bits === 1 ? "1 bit" : `${bits.toLocaleString()} bits`;
}

// Value inspector: interprets the bytes under the cursor as common primitive
// types and writes edits back. Every row is a small two-way lens: decode bytes
// to text, parse text to bytes.
//
// The cursor is a bit position, so these readings start wherever it is: put the
// cursor three bits into a byte and the rows show what a u16 there would say.

import { formatOffset } from "./doc.js";
import type { BitRange } from "./hexview.js";
import type { Doc, Origin, TemplateNode } from "./doc.js";
import { LENSES, type Lens } from "./lenses.js";
import { childWord, countText } from "./strings.js";
import { typePanel } from "./typepanel.js";
import { extraction } from "./bitextract.js";

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
  /** Shift-and-mask for a value that does not start on a byte boundary. */
  private readonly formula: HTMLElement;
  private readonly fieldRow: HTMLElement;
  private readonly types: HTMLElement;
  /** Which other fields settled this one's length, count, type or place. */
  private readonly origins: HTMLElement;
  /** Path of the field the structure panel is showing, if any. */
  private at: readonly number[] | null = null;
  /** A field picked by name stays shown until the cursor moves off it. */
  private pinned: readonly number[] | null = null;

  /** Asked for when a breadcrumb is clicked, so the views can follow. */
  onPick: (path: readonly number[]) => void = () => {};
  /** Asked for when the reader follows an offset, so the views can follow. */
  onGoTo: (bitOffset: number, ranges?: readonly BitRange[]) => void = () => {};

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
    this.origins = document.createElement("div");
    this.origins.className = "insp-origins";
    this.origins.hidden = true;
    this.origins.addEventListener("click", (e) => {
      const t = e.target;
      if (!(t instanceof HTMLElement)) return;
      const to = t.dataset["bit"];
      if (to !== undefined) return this.onGoTo(Number(to));
      const p = t.dataset["path"];
      if (p !== undefined) this.onPick(p === "" ? [] : p.split("/").map(Number));
    });
    this.fieldRow.append(subhead("Value"), this.field, this.area, this.note, this.origins, this.types);
    this.struct.append(this.crumbs, this.fieldRow);

    // How to lift an unaligned run of bits out of the bytes around it. Only
    // shown when the cursor is not on a byte boundary, where reading the value
    // out of a file takes more than an index.
    this.formula = document.createElement("div");
    this.formula.className = "insp-formula";
    this.formula.hidden = true;

    this.status = document.createElement("div");
    this.status.className = "insp-status";
    this.status.setAttribute("role", "status");

    // The address sits above every tab: the field reading and the two raw
    // readings all start at the same place, and that place is the first thing
    // to check.
    this.el.append(head, this.detail, this.struct, table, this.formula, this.status);
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

    // The raw readings all start at the cursor, so the address is the cursor's
    // own and the width the formula explains is one byte of the run.
    this.detail.hidden = false;
    const here = document.createElement("span");
    here.className = "addr";
    here.textContent = formatOffset(this.offset);
    this.detail.replaceChildren(here);
    this.showFormula(this.offset, 8, true);

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
      this.hideField();
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
      this.hideField();
      return;
    }
    const path: readonly number[] = found.node;
    const node = this.doc.templateNode(path);
    if (node.status !== "ok") {
      this.at = null;
      this.crumbs.textContent =
        node.status === "pending" || node.status === "working" ? "Loading this part of the file…" : node.message;
      this.hideField();
      return;
    }
    this.at = path;
    const n = node.node;
    this.crumbs.replaceChildren(...this.trail(path));
    this.fieldRow.hidden = false;
    this.detail.hidden = false;
    const at = document.createElement("span");
    at.className = "addr";
    at.textContent = formatOffset(n.offset_bits);
    this.detail.replaceChildren(at, ` · ${n.type} · ${sizeText(n.size_bits)}`);
    this.showFormula(n.offset_bits, n.size_bits, false);
    const long = !n.composite && (n.kind === "bytes" || n.kind === "str");
    this.area.hidden = !long;
    this.field.hidden = long;
    if (long) this.fillArea(n);
    else this.fillField(n);
    this.fillOrigins(path);
    this.fillTypes(path, n);
  }

  /**
   * The shift-and-mask that lifts the value at the cursor out of the bytes it
   * straddles. Shown for a value that does not start and end on a byte
   * boundary, where "the byte at this address" is not the whole answer.
   *
   * `perByte` is for the raw readings, which run for as many bytes as the type
   * takes: one byte is worked, and the rest follow it.
   */
  private showFormula(bitOffset: number, widthBits: number, perByte: boolean): void {
    const unaligned = bitOffset % 8 !== 0;
    const partial = widthBits % 8 !== 0;
    // Wider than a machine word: a term per byte would run off the panel, and
    // the per-byte shift says the same thing in one line.
    const wide = widthBits > 64;
    if (widthBits === 0 || (!unaligned && !partial) || (wide && !unaligned)) {
      this.formula.hidden = true;
      this.formula.replaceChildren();
      return;
    }
    const width = perByte || wide ? 8 : widthBits;
    const parts: Node[] = [subhead("Bit extraction"), extraction(bitOffset, width)];
    // Where the reading runs on past one byte, the expression is for the first
    // of them and the rest follow it. Where it is a whole field, its value is
    // already in the editor above and needs no second telling.
    if (perByte || wide) {
      const note = document.createElement("div");
      note.className = "insp-formula-note";
      note.textContent = "The first byte at the cursor. Step both indexes up for each byte after it.";
      parts.push(note);
    }
    this.formula.replaceChildren(...parts);
    this.formula.hidden = false;
  }

  /** Nothing to show about a field: no template, no field, or not read yet. */
  private hideField(): void {
    this.fieldRow.hidden = true;
    this.detail.hidden = true;
    this.formula.hidden = true;
  }

  /**
   * Which other fields settled the shape of the one at the cursor, and where it
   * points when it holds an offset.
   *
   * Every step of the path is asked, not only the field itself: 128 bytes of
   * packed weights are 128 bytes because of the tensor record three levels up,
   * and that record is what the reader wants to see. Each step that has an
   * answer is a group under the field it is about.
   */
  private fillOrigins(path: readonly number[]): void {
    const rows: Node[] = [];
    const jumps: Node[] = [];
    for (let i = 0; i <= path.length; i++) {
      const at = path.slice(0, i);
      const reply = this.doc.origins(at);
      if (reply.status !== "ok") continue;
      const from = reply.node.filter((o) => o.role !== "points");
      // Only the field itself can point somewhere: an ancestor's pointer is
      // not what the cursor is on.
      if (i === path.length) {
        for (const o of reply.node) {
          if (o.role === "points" && o.target_bits !== null) jumps.push(pointsRow(o));
        }
      }
      if (from.length === 0) continue;
      if (i < path.length) {
        const node = this.doc.templateNode(at);
        const b = document.createElement("button");
        b.type = "button";
        b.className = "insp-link insp-origin-group";
        b.dataset["path"] = at.join("/");
        b.textContent = node.status === "ok" ? node.node.name : "…";
        rows.push(b);
      }
      for (const o of from) rows.push(originRow(o));
    }
    if (rows.length === 0 && jumps.length === 0) {
      this.origins.hidden = true;
      this.origins.replaceChildren();
      return;
    }
    const all: Node[] = [];
    if (rows.length > 0) all.push(subhead("Depends on"), ...rows);
    if (jumps.length > 0) all.push(subhead("Points to"), ...jumps);
    this.origins.replaceChildren(...all);
    this.origins.hidden = false;
  }

  /** The section below the editor. See `insp-type`. */
  private fillTypes(path: readonly number[], n: TemplateNode): void {
    const reply = this.doc.typeInfo(path, this.offset);
    if (reply.status !== "ok" || reply.node.kind === "plain") {
      this.types.hidden = true;
      this.types.replaceChildren();
      return;
    }
    this.types.replaceChildren(
      typePanel(
        reply.node,
        path,
        n,
        (p, text) => this.applyValue(p, text),
        (bit, ranges) => this.onGoTo(bit, ranges),
        () => this.render(),
      ),
    );
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
    this.field.placeholder = n.composite ? countText(n.child_count, childWord(n)) : "";
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

/** A heading over one part of the panel, so the mode buttons above are not
 *  mistaken for one. */
function subhead(text: string): HTMLElement {
  const h = document.createElement("div");
  h.className = "insp-subhead";
  h.textContent = text;
  return h;
}

/** Where a field holding an offset points. */
function pointsRow(o: Origin): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-origin";
  const b = document.createElement("button");
  b.type = "button";
  b.className = "insp-link addr";
  b.dataset["bit"] = String(o.target_bits);
  b.textContent = formatOffset(o.target_bits ?? 0);
  const what = document.createElement("span");
  what.className = "insp-origin-val";
  what.textContent = o.label;
  row.append(b, what);
  return row;
}

/** A count of four million reads as one. */
function grouped(value: string): string {
  return /^\d{5,}$/.test(value) ? BigInt(value).toLocaleString() : value;
}

/** What one field decided about the one at the cursor: `Length  len = 20`. */
const ROLE_TEXT = { length: "Length", count: "Count", type: "Type", position: "Position", points: "" } as const;

function originRow(o: Origin): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-origin";
  const role = document.createElement("span");
  role.className = "insp-origin-role";
  role.textContent = ROLE_TEXT[o.role];
  const b = document.createElement("button");
  b.type = "button";
  b.className = "insp-link";
  b.dataset["path"] = o.path.join("/");
  b.textContent = o.label;
  row.append(role, b);
  if (o.value !== "") {
    const v = document.createElement("span");
    v.className = "insp-origin-val";
    v.textContent = `= ${grouped(o.value)}`;
    row.append(v);
  }
  return row;
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

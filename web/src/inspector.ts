// Value inspector: interprets the bytes under the cursor as common primitive
// types and writes edits back. Every row is a small two-way lens: decode bytes
// to text, parse text to bytes.
//
// The cursor is a bit position, so these readings start wherever it is: put the
// cursor three bits into a byte and the rows show what a u16 there would say.

import { formatBytes, formatOffset } from "./doc.js";
import type { BitRange } from "./hexview.js";
import type { Doc, Origin, TemplateNode } from "./doc.js";
import { LENSES, type Lens } from "./lenses.js";
import { bitSizeText, childWord, countText } from "./strings.js";
import { typePanel } from "./typepanel.js";
import { fieldNumber, openPlan, type OpenPlan } from "./openplan.js";
import { extraction } from "./bitextract.js";
import { crc32, hex32, hexBytes, lhaCrc16, sha1, sum8 } from "./integrity.js";

const AUTO_CHECK_BYTES = 1024 * 1024;

type IntegrityPlan = {
  readonly label: string;
  readonly bytes: number;
  check(): Promise<{ actual: string; expected: string }>;
};

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
  /** Human-readable dates and checks derived from format fields. */
  private readonly semantics: HTMLElement;
  /** Which other fields settled this one's length, count, type or place. */
  private readonly origins: HTMLElement;
  /** Offer to open this field's bytes as a document in a tab of its own. */
  private readonly openAs: HTMLElement;
  /** What the hex view has selected, in file order. More than one run is
   *  allowed because a value a format does not keep in one piece is more than
   *  one run of bits. Empty when nothing is selected. */
  private selection: readonly BitRange[] = [];
  private readonly selectionEl: HTMLElement;
  private readonly selWhere: HTMLElement;
  private readonly selTable: HTMLElement;
  private readonly selLength: HTMLElement;
  private readonly selStatus: HTMLElement;
  private readonly selRows = new Map<SelKind, SelRow>();
  /** Which reading of the selection is open for typing into. Only one is, so
   *  the rest go on showing what the file says while it is being changed. */
  private editing: SelKind | null = null;
  /** Path of the field the structure panel is showing, if any. */
  private at: readonly number[] | null = null;
  /** A field picked by name stays shown until the cursor moves off it. */
  private pinned: readonly number[] | null = null;
  /** Deep parser-only paths start compact. The omitted middle can be expanded
   * in place when somebody does need to inspect the underlying wrappers. */
  private crumbsExpanded = false;

  /** Asked for when a breadcrumb is clicked, so the views can follow. */
  onPick: (path: readonly number[]) => void = () => {};
  /** Asked for when the reader follows an offset, so the views can follow. */
  onGoTo: (bitOffset: number, ranges?: readonly BitRange[]) => void = () => {};
  /** Asked for when the reader opens a field's bytes as their own document. */
  onOpenTab: (bytes: Uint8Array, name: string, origin: string) => void = () => {};

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
      if (t.dataset["expand"] !== undefined) {
        this.crumbsExpanded = true;
        this.render();
        return;
      }
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
    this.semantics = document.createElement("div");
    this.semantics.className = "insp-semantics";
    this.semantics.hidden = true;
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
    this.openAs = document.createElement("div");
    this.openAs.className = "insp-openas";
    this.openAs.hidden = true;
    this.fieldRow.append(subhead("Value"), this.field, this.area, this.note, this.semantics, this.openAs, this.origins, this.types);
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

    // What is selected, above the readings at the cursor and outside the three
    // tabs, because a selection is its own question and the answer to it does
    // not depend on which reading is showing.
    this.selectionEl = document.createElement("div");
    this.selectionEl.className = "insp-selection";
    this.selectionEl.hidden = true;
    this.selWhere = document.createElement("div");
    this.selWhere.className = "insp-detail";
    this.selTable = document.createElement("table");
    this.selTable.className = "insp-table insp-seltable";
    this.selLength = document.createElement("td");
    const lengthRow = document.createElement("tr");
    const lengthHead = document.createElement("th");
    lengthHead.scope = "row";
    lengthHead.textContent = SEL_LENGTH;
    lengthRow.append(lengthHead, this.selLength);
    this.selTable.append(lengthRow);
    for (const row of SEL_ROWS) this.buildSelRow(row.kind, row.label);
    this.selStatus = document.createElement("div");
    this.selStatus.className = "insp-selstatus";
    this.selStatus.setAttribute("role", "status");
    this.selectionEl.append(subhead(SEL_TITLE), this.selWhere, this.selTable, this.selStatus);

    // The address sits above every tab: the field reading and the two raw
    // readings all start at the same place, and that place is the first thing
    // to check.
    this.el.append(head, this.selectionEl, this.detail, this.struct, table, this.formula, this.status);
    doc.onChange(() => this.render());
  }

  /** `bitOffset` is absolute, counting from the top bit of byte 0. */
  setOffset(bitOffset: number): void {
    this.offset = bitOffset;
    this.pinned = null;
    this.render();
  }

  /** What the hex view has selected. Several runs read as one number, in the
   *  order they are given, which is how a value split across a block is put
   *  back together. */
  setSelection(ranges: readonly BitRange[]): void {
    this.selection = ranges.filter((r) => r.endBit > r.startBit);
    this.renderSelection();
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

  /**
   * One reading of the selection: the number on a single line, cut short
   * rather than wrapped, with a way to take it whole and a way to change it.
   * Both only appear under the pointer or the keyboard, because five rows of
   * buttons would bury the numbers they belong to.
   */
  private buildSelRow(kind: SelKind, label: string): void {
    const tr = document.createElement("tr");
    tr.hidden = true;
    const th = document.createElement("th");
    th.scope = "row";
    th.textContent = label;
    const td = document.createElement("td");
    const wrap = document.createElement("div");
    wrap.className = "insp-valrow";
    const text = document.createElement("span");
    text.className = "insp-val";
    const input = document.createElement("input");
    input.type = "text";
    input.spellcheck = false;
    input.autocomplete = "off";
    input.className = "insp-val-edit";
    input.hidden = true;
    input.setAttribute("aria-label", label);
    const acts = document.createElement("div");
    acts.className = "insp-acts";
    const copy = actionButton(COPY, copyLabel(label));
    const edit = actionButton(EDIT, editLabel(label));
    acts.append(copy, edit);
    wrap.append(text, input, acts);
    td.append(wrap);
    tr.append(th, td);
    this.selTable.append(tr);

    copy.addEventListener("click", () => void this.copyValue(text.textContent ?? ""));
    edit.addEventListener("click", () => this.startEdit(kind));
    // Double-clicking the number is the other way in, for readers who reach
    // for that before they notice the button.
    text.addEventListener("dblclick", () => this.startEdit(kind));
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") this.commitSel(kind);
      if (e.key === "Escape") this.cancelEdit();
    });
    input.addEventListener("blur", () => {
      if (this.editing === kind) this.commitSel(kind);
    });
    input.addEventListener("input", () => {
      input.classList.remove("invalid");
      this.selStatus.textContent = "";
    });
    this.selRows.set(kind, { tr, text, input, edit });
  }

  private startEdit(kind: SelKind): void {
    const row = this.selRows.get(kind);
    if (row === undefined || row.tr.hidden) return;
    this.editing = kind;
    this.selStatus.textContent = "";
    this.renderSelection();
    row.input.focus();
    row.input.select();
  }

  private cancelEdit(): void {
    this.editing = null;
    this.selStatus.textContent = "";
    this.renderSelection();
  }

  /** Write the typed number back over the selected bits. */
  private commitSel(kind: SelKind): void {
    const row = this.selRows.get(kind);
    const ranges = this.selection;
    if (row === undefined || ranges.length === 0) return;
    const bits = ranges.reduce((n, r) => n + (r.endBit - r.startBit), 0);
    const parsed = parseSel(kind, row.input.value, bits);
    if (!parsed.ok) {
      row.input.classList.add("invalid");
      this.selStatus.textContent = parsed.why;
      return;
    }
    // The whole selection is one value, so writing it is one undo step even
    // where it is spread over several runs.
    this.editing = null;
    this.doc.beginBatch();
    writeRanges(this.doc, ranges, parsed.value, reversed(kind));
    this.doc.endBatch();
    this.selStatus.textContent = "";
  }

  private async copyValue(text: string): Promise<void> {
    if (text === "") return;
    try {
      await navigator.clipboard.writeText(text);
      this.selStatus.textContent = COPIED;
    } catch {
      this.selStatus.textContent = COPY_FAILED;
    }
  }

  /** The selection as a number, when there is one and it is short enough to
   *  be one. */
  private renderSelection(): void {
    const ranges = this.selection;
    this.selectionEl.hidden = ranges.length === 0;
    if (ranges.length === 0) {
      this.editing = null;
      return;
    }
    const bits = ranges.reduce((n, r) => n + (r.endBit - r.startBit), 0);
    this.selWhere.textContent = ranges
      .map((r) => `${formatOffset(r.startBit)} to ${formatOffset(r.endBit)}`)
      .join(", ");
    this.selLength.textContent = lengthText(bits);
    // Past the limit the number rows are simply absent. Nothing selects a
    // thousand bytes meaning to read them as one integer, so there is nothing
    // to explain.
    const v = bits <= SELECTION_LIMIT_BITS ? readBits(this.doc, ranges) : null;
    const loading = bits <= SELECTION_LIMIT_BITS && v === null;
    this.selStatus.textContent = loading ? LOADING : this.selStatus.textContent;
    // Reversing bytes only means anything when the selection is made of whole
    // bytes lying together, which is the only case a format would have stored
    // the other way round.
    const one = ranges[0];
    const whole = ranges.length === 1 && one !== undefined && one.startBit % 8 === 0 && bits % 8 === 0 && bits > 8;
    const le = whole && one !== undefined && v !== null ? readBits(this.doc, [one], true) : null;
    for (const [kind, row] of this.selRows) {
      const raw = reversed(kind) ? le : v;
      const show = raw !== null && (!reversed(kind) || whole);
      row.tr.hidden = !show;
      if (!show) continue;
      const value = formatSel(kind, raw, bits);
      row.text.textContent = value;
      row.text.title = value;
      const open = this.editing === kind;
      row.text.hidden = open;
      row.input.hidden = !open;
      row.edit.hidden = open;
      // Only overwrite the box while it is not being typed into.
      if (open && document.activeElement !== row.input) row.input.value = value;
      if (!open) row.input.classList.remove("invalid");
    }
  }

  render(): void {
    this.renderSelection();
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
    this.detail.replaceChildren(at, ` · ${n.type} · ${bitSizeText(n.size_bits)}`);
    this.showFormula(n.offset_bits, n.size_bits, false);
    const long = !n.composite && (n.kind === "bytes" || n.kind === "str");
    this.area.hidden = !long;
    this.field.hidden = long;
    if (long) this.fillArea(n);
    else this.fillField(n);
    this.fillOrigins(path);
    this.fillTypes(path, n);
    this.fillSemantics(path, n);
    this.fillOpenAs(path, n);
  }

  /**
   * A field whose bytes are a whole embedded file, or any plain run of bytes,
   * can be opened as a document in a tab of its own. A run stored compressed
   * is decompressed on the way, which is the point: those bytes exist nowhere
   * in this file, so a tab is the only place to read them.
   */
  private fillOpenAs(path: readonly number[], n: TemplateNode): void {
    const plan = openPlan(this.doc, path, n);
    if (plan === null) {
      this.openAs.hidden = true;
      this.openAs.replaceChildren();
      return;
    }
    const detail = document.createElement("div");
    detail.className = "insp-detail";
    detail.textContent = plan.detail;
    const parts: Node[] = [subhead("Open as a file"), detail];
    if (plan.load !== null) parts.push(this.openButton(plan));
    this.openAs.replaceChildren(...parts);
    this.openAs.hidden = false;
  }

  private openButton(plan: OpenPlan): HTMLElement {
    const load = plan.load;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "insp-check-button";
    button.textContent = `Open ${plan.name}`;
    button.addEventListener("click", () => {
      if (load === null) return;
      button.disabled = true;
      this.status.textContent = "Loading…";
      load()
        .then((bytes) => {
          this.status.textContent = "";
          this.onOpenTab(bytes, plan.name, plan.origin);
        })
        .catch((cause: unknown) => {
          this.status.textContent = cause instanceof Error ? cause.message : "Couldn't open these bytes.";
        })
        .finally(() => {
          button.disabled = false;
        });
    });
    return button;
  }

  /** Date lenses keep the stored integer visible above. Large integrity
   * ranges wait for an explicit click instead of making field selection read
   * the whole file. */
  private fillSemantics(path: readonly number[], n: TemplateNode): void {
    const date = this.dateText(path, n);
    const plan = this.integrityPlan(path, n);
    if (date === null && plan === null) {
      this.semantics.hidden = true;
      this.semantics.replaceChildren();
      return;
    }
    const parts: Node[] = [];
    if (date !== null) {
      const value = document.createElement("div");
      value.className = "insp-semantic-value";
      value.append(subhead("Date & time"), date);
      parts.push(value);
    }
    if (plan !== null) parts.push(this.integrityWidget(plan));
    this.semantics.replaceChildren(...parts);
    this.semantics.hidden = false;
  }

  private dateText(path: readonly number[], n: TemplateNode): string | null {
    const raw = Number(n.edit_text);
    if (!Number.isFinite(raw)) return null;
    if (this.doc.template === "gzip" && n.name === "mtime") return unixDate(raw, raw === 0 ? "not specified" : "UTC");
    if ((this.doc.template === "mp4" || this.doc.template === "braw") && (n.name === "creation_time" || n.name === "modification_time")) {
      return quickTimeDate(raw);
    }
    if (this.doc.template === "utmp" && n.name === "tv_sec") return unixDate(raw, "UTC");
    if (this.doc.template === "cpio" && n.name === "c_mtime") return unixDate(raw, "UTC");
    if ((this.doc.template === "ar" || this.doc.template === "deb") && n.name === "mtime") return unixDate(raw, "UTC");
    // A journal keeps its wall-clock times in microseconds.
    if (this.doc.template === "journal" && n.name.endsWith("realtime")) return unixDate(raw / 1e6, "UTC");
    if (this.doc.template === "mca" && path.length === 2) {
      const parent = this.doc.templateNode(path.slice(0, -1));
      if (parent.status === "ok" && parent.node.name === "timestamps") return unixDate(raw, "UTC");
    }
    if (this.doc.isZip && (n.name === "modified_time" || n.name === "modified_date")) {
      const siblings = this.siblings(path);
      const time = siblings.find((x) => x.name === "modified_time");
      const date = siblings.find((x) => x.name === "modified_date");
      if (time === undefined || date === undefined) return null;
      const t = Number(time.edit_text);
      const d = Number(date.edit_text);
      const year = 1980 + ((d >>> 9) & 0x7f);
      const month = (d >>> 5) & 0x0f;
      const day = d & 0x1f;
      const hour = (t >>> 11) & 0x1f;
      const minute = (t >>> 5) & 0x3f;
      const second = (t & 0x1f) * 2;
      if (month === 0 || day === 0 || month > 12 || day > 31 || hour > 23 || minute > 59 || second > 59) {
        return "Invalid MS-DOS date/time";
      }
      return `${year}-${pad(month)}-${pad(day)} ${pad(hour)}:${pad(minute)}:${pad(second)} (MS-DOS local time)`;
    }
    return null;
  }

  private siblings(path: readonly number[]): TemplateNode[] {
    if (path.length === 0) return [];
    const reply = this.doc.templateChildren(path.slice(0, -1), 0, 128);
    return reply.status === "ok" ? reply.node : [];
  }

  private integrityPlan(path: readonly number[], n: TemplateNode): IntegrityPlan | null {
    const siblings = this.siblings(path);
    if (this.doc.template === "png" && n.name === "crc") {
      const numeric = Number(n.edit_text);
      if (!Number.isFinite(numeric)) return null;
      const expected = numeric >>> 0;
      const type = siblings.find((x) => x.name === "type");
      if (type === undefined || type.offset_bits % 8 !== 0 || n.offset_bits % 8 !== 0) return null;
      const at = type.offset_bits / 8;
      const bytes = n.offset_bits / 8 - at;
      return {
        label: "PNG CRC-32",
        bytes,
        check: async () => ({ actual: hex32(crc32(await this.loadBytes(at, bytes))), expected: hex32(expected) }),
      };
    }
    if (this.doc.isZip && n.name === "crc32") {
      const numeric = Number(n.edit_text);
      if (!Number.isFinite(numeric)) return null;
      const expected = numeric >>> 0;
      const compression = siblings.find((x) => x.name === "compression");
      const data = siblings.find((x) => x.name === "data");
      const uncompressedSize =
        siblings.find((x) => x.name === "unpacked_size") ?? siblings.find((x) => x.name === "uncompressed_size");
      if (compression === undefined || data === undefined || data.offset_bits % 8 !== 0 || data.size_bits % 8 !== 0) return null;
      const method = fieldNumber(compression);
      const packedBytes = data.size_bits / 8;
      const coveredBytes = uncompressedSize === undefined ? packedBytes : Number(uncompressedSize.edit_text);
      if (method !== 0 && method !== 8) return null;
      return {
        label: method === 0 ? "ZIP CRC-32 (stored data)" : "ZIP CRC-32 (deflated data)",
        bytes: Number.isFinite(coveredBytes) ? coveredBytes : packedBytes,
        check: async () => {
          const packed = await this.loadBytes(data.offset_bits / 8, packedBytes);
          const unpacked = method === 0 ? packed : await decompress(packed, "deflate-raw");
          return { actual: hex32(crc32(unpacked)), expected: hex32(expected) };
        },
      };
    }
    if (this.doc.template === "gzip" && n.name === "header_crc" && n.size_bits === 16 && n.offset_bits % 8 === 0) {
      const bytes = n.offset_bits / 8;
      return {
        label: "gzip header CRC-16",
        bytes,
        check: async () => {
          const stored = await this.loadBytes(n.offset_bits / 8, 2);
          const expected = stored[0]! | (stored[1]! << 8);
          return { actual: hex16(crc32(await this.loadBytes(0, bytes)) & 0xffff), expected: hex16(expected) };
        },
      };
    }
    if (this.doc.template === "gzip" && n.name === "crc32") {
      const numeric = Number(n.edit_text);
      if (!Number.isFinite(numeric)) return null;
      const expected = numeric >>> 0;
      const compressed = siblings.find((x) => x.name === "compressed");
      const originalSize = siblings.find((x) => x.name === "original_size");
      if (compressed === undefined || compressed.offset_bits % 8 !== 0 || compressed.size_bits % 8 !== 0) return null;
      const packedBytes = compressed.size_bits / 8;
      const declaredBytes = originalSize === undefined ? packedBytes : Number(originalSize.edit_text);
      // ISIZE is modulo 2^32. A non-empty stream declaring zero may really
      // expand to 4 GiB, so never start that case merely because it says zero.
      const expandedBytes = declaredBytes === 0 && packedBytes > 2 ? 0x1_0000_0000 : declaredBytes;
      return {
        label: "gzip CRC-32 (uncompressed data)",
        bytes: Number.isFinite(expandedBytes) ? expandedBytes : packedBytes,
        check: async () => {
          const packed = await this.loadBytes(compressed.offset_bits / 8, packedBytes);
          const unpacked = await decompress(packed, "deflate-raw");
          return { actual: hex32(crc32(unpacked)), expected: hex32(expected) };
        },
      };
    }
    if ((this.doc.template === "gitindex" || this.doc.template === "gitpackidx") && n.name === "checksum" && path.length === 1 && n.size_bits === 160 && n.offset_bits % 8 === 0) {
      const bytes = n.offset_bits / 8;
      return {
        label: "Git file SHA-1",
        bytes,
        check: async () => ({
          actual: await sha1(await this.loadBytes(0, bytes)),
          expected: hexBytes(await this.loadBytes(n.offset_bits / 8, 20)),
        }),
      };
    }
    if (this.doc.template === "lha" && n.name === "header_checksum" && n.size_bits === 8 && n.offset_bits % 8 === 0) {
      const entry = this.doc.templateChildren(path.slice(0, -2), 0, 8);
      if (entry.status !== "ok") return null;
      const headerSize = entry.node.find((x) => x.name === "header_size");
      const expected = Number(n.edit_text);
      if (headerSize === undefined || !Number.isFinite(expected)) return null;
      const bytes = Number(headerSize.edit_text);
      const at = n.offset_bits / 8 + 1;
      return {
        label: "LHA header checksum",
        bytes,
        check: async () => ({ actual: hex8(sum8(await this.loadBytes(at, bytes))), expected: hex8(expected) }),
      };
    }
    if (this.doc.template === "lha" && n.name === "crc" && n.size_bits === 16) {
      const method = siblings.find((x) => x.name === "method");
      const data = siblings.find((x) => x.name === "data");
      const expected = Number(n.edit_text);
      // -lh0- is the stored method; compressed LHA methods need their own
      // decoders before their CRC of the uncompressed file can be checked.
      if (method === undefined || fieldNumber(method) !== 0x2d_6c_68_30_2d || data === undefined || !Number.isFinite(expected)) return null;
      if (data.offset_bits % 8 !== 0 || data.size_bits % 8 !== 0) return null;
      const bytes = data.size_bits / 8;
      return {
        label: "LHA CRC-16 (stored data)",
        bytes,
        check: async () => ({ actual: hex16(lhaCrc16(await this.loadBytes(data.offset_bits / 8, bytes))), expected: hex16(expected) }),
      };
    }
    return null;
  }

  private integrityWidget(plan: IntegrityPlan): HTMLElement {
    const box = document.createElement("div");
    box.className = "insp-integrity";
    const result = document.createElement("div");
    result.className = "insp-check-result";
    const run = async (): Promise<void> => {
      result.className = "insp-check-result";
      result.textContent = "Checking…";
      try {
        const { actual, expected } = await plan.check();
        const ok = actual === expected;
        result.classList.add(ok ? "ok" : "bad");
        result.textContent = ok ? `Valid · ${actual}` : `Mismatch · calculated ${actual}, stored ${expected}`;
      } catch (cause) {
        result.classList.add("bad");
        result.textContent = cause instanceof Error ? cause.message : "Could not check this data.";
      }
    };
    box.append(subhead("Integrity"));
    if (plan.bytes <= AUTO_CHECK_BYTES) {
      box.append(`${plan.label} · ${formatBytes(plan.bytes)}`, result);
      void run();
    } else {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "insp-check-button";
      button.textContent = `Check ${plan.label} over ${formatBytes(plan.bytes)}`;
      button.addEventListener("click", () => void run());
      box.append(button, result);
    }
    return box;
  }

  private async loadBytes(at: number, len: number): Promise<Uint8Array> {
    await this.doc.ensureRange(at, len);
    const read = this.doc.read(at, len);
    if (!read.complete) throw new Error("Some bytes could not be loaded.");
    return read.bytes;
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
    const items: { label: string; path: readonly number[]; here: boolean }[] = [];
    for (let i = 0; i <= path.length; i++) {
      const node = this.doc.templateNode(path.slice(0, i));
      if (node.status !== "ok") {
        items.push({ label: "?", path: path.slice(0, i), here: i === path.length });
        continue;
      }
      const n = node.node;
      const isList = n.composite && n.type.endsWith("[]");
      if (isList && i < path.length) {
        // Fold the element index into the list's own name.
        const to = path.slice(0, i + 1);
        items.push({ label: `${n.name}[${path[i]}]`, path: to, here: i + 1 === path.length });
        i += 1;
        continue;
      }
      // A struct field is often called `body`; its type says what it holds.
      const label = n.composite && n.type !== n.name ? `${n.name} (${n.type})` : n.name;
      const previous = items[items.length - 1];
      if (previous !== undefined && previous.label === label) {
        // Repeated `object`/`body` wrappers are one logical step. Keep the
        // deepest target so following the crumb still reaches the useful one.
        items[items.length - 1] = { label, path: path.slice(0, i), here: i === path.length };
      } else {
        items.push({ label, path: path.slice(0, i), here: i === path.length });
      }
    }
    const MAX_CRUMBS = 7;
    if (this.crumbsExpanded || items.length <= MAX_CRUMBS) {
      return items.map((item) => this.crumb(item.label, item.path, item.here));
    }
    const head = items.slice(0, 2);
    const tail = items.slice(-3);
    const hidden = items.slice(2, -3);
    const more = document.createElement("button");
    more.type = "button";
    more.className = "insp-crumb insp-crumb-more";
    more.dataset["expand"] = "";
    more.textContent = `… ${hidden.length} internal levels`;
    more.title = hidden.map((item) => item.label).join(" › ");
    return [
      ...head.map((item) => this.crumb(item.label, item.path, item.here)),
      more,
      ...tail.map((item) => this.crumb(item.label, item.path, item.here)),
    ];
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
/** How much of a selection is read as a number. A thousand bytes is already
 *  well past anything a format stores as one, and the whole point of a limit
 *  is that selecting half a file does not lock the page up computing a number
 *  nobody wanted. */
const SELECTION_LIMIT_BYTES = 1024;
const SELECTION_LIMIT_BITS = SELECTION_LIMIT_BYTES * 8;

const SEL_TITLE = "Selection";
const SEL_LENGTH = "Length";
const LOADING = "Loading…";
const COPY = "Copy";
const EDIT = "Edit";
const COPIED = "Copied.";
const COPY_FAILED = "Couldn't copy to the clipboard.";
const copyLabel = (row: string): string => `Copy the ${row.toLowerCase()} value`;
const editLabel = (row: string): string => `Edit the ${row.toLowerCase()} value`;

/** Which readings of the selection are offered, in the order they are shown.
 *  The two reversed ones are only for a selection of whole bytes lying
 *  together. */
type SelKind = "unsigned" | "signed" | "hex" | "unsignedLe" | "signedLe";
const SEL_ROWS: readonly { readonly kind: SelKind; readonly label: string }[] = [
  { kind: "unsigned", label: "Unsigned" },
  { kind: "signed", label: "Signed" },
  { kind: "hex", label: "Hex" },
  { kind: "unsignedLe", label: "Unsigned LE" },
  { kind: "signedLe", label: "Signed LE" },
];

/** The parts of one reading's row that anything outside it touches. */
type SelRow = {
  readonly tr: HTMLElement;
  readonly text: HTMLElement;
  readonly input: HTMLInputElement;
  readonly edit: HTMLButtonElement;
};

function reversed(kind: SelKind): boolean {
  return kind === "unsignedLe" || kind === "signedLe";
}

/** A small button that stays out of the way until it is wanted. */
function actionButton(text: string, label: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "insp-act";
  b.textContent = text;
  b.title = label;
  b.setAttribute("aria-label", label);
  return b;
}

/** One reading of the bits, as text. `raw` is already byte-reversed for the
 *  readings that are. */
function formatSel(kind: SelKind, raw: bigint, bits: number): string {
  if (kind === "hex") return `0x${raw.toString(16).padStart(Math.ceil(bits / 4), "0")}`;
  if (kind === "signed" || kind === "signedLe") return signed(raw, bits).toString();
  return raw.toString();
}

/** Typed text back to the bits it stands for, or why it is not any. */
function parseSel(kind: SelKind, text: string, bits: number): { ok: true; value: bigint } | { ok: false; why: string } {
  const t = text.trim();
  const width = 1n << BigInt(bits);
  if (kind === "hex") {
    const digits = t.replace(/^0x/i, "").replace(/\s+/g, "");
    if (!/^[0-9a-f]+$/i.test(digits)) return { ok: false, why: NOT_HEX };
    const v = BigInt(`0x${digits}`);
    return v < width ? { ok: true, value: v } : { ok: false, why: tooBigHex(bits) };
  }
  const signedKind = kind === "signed" || kind === "signedLe";
  if (!new RegExp(signedKind ? "^[+-]?\\d+$" : "^\\+?\\d+$").test(t)) return { ok: false, why: NOT_A_NUMBER };
  const v = BigInt(t);
  if (signedKind) {
    const half = 1n << BigInt(bits - 1);
    if (v < -half || v >= half) return { ok: false, why: outOfRange(bits, (-half).toString(), (half - 1n).toString()) };
    // Two's complement is the bit pattern; a negative number is the one that
    // wraps to it.
    return { ok: true, value: v < 0n ? v + width : v };
  }
  if (v >= width) return { ok: false, why: outOfRange(bits, "0", (width - 1n).toString()) };
  return { ok: true, value: v };
}

const NOT_A_NUMBER = "Not a whole number.";
const NOT_HEX = "Not hexadecimal.";
const outOfRange = (bits: number, low: string, high: string): string =>
  `Out of range for ${lengthText(bits)}: ${low} to ${high}.`;
const tooBigHex = (bits: number): string => `More than ${lengthText(bits)}: at most ${Math.ceil(bits / 4)} hex digits.`;

/**
 * Write one number back over the runs it was read from, filling them from the
 * last backwards so each run keeps the bits that were its own.
 */
function writeRanges(doc: Doc, ranges: readonly BitRange[], value: bigint, reverseBytes: boolean): void {
  let rest = value;
  for (let i = ranges.length - 1; i >= 0; i--) {
    const r = ranges[i];
    if (r === undefined) continue;
    const len = r.endBit - r.startBit;
    const chunk = rest & ((1n << BigInt(len)) - 1n);
    rest >>= BigInt(len);
    const bytes = bitsToBytes(chunk, len);
    doc.overwriteBits(r.startBit, reverseBytes ? bytes.reverse() : bytes, len);
  }
}

/** A number as the bits of a field: packed from the top, so a run that does
 *  not fill its last byte leaves the padding at the bottom of it. */
function bitsToBytes(v: bigint, bits: number): Uint8Array {
  const n = Math.ceil(bits / 8);
  let x = v << BigInt(n * 8 - bits);
  const out = new Uint8Array(n);
  for (let i = n - 1; i >= 0; i--) {
    out[i] = Number(x & 0xffn);
    x >>= 8n;
  }
  return out;
}

/** `24 bytes`, or `3 bytes 4 bits` where the run does not fill whole bytes. */
function lengthText(bits: number): string {
  const bytes = Math.floor(bits / 8);
  const rest = bits % 8;
  const parts: string[] = [];
  if (bytes > 0) parts.push(`${bytes.toLocaleString()} ${bytes === 1 ? "byte" : "bytes"}`);
  if (rest > 0 || bytes === 0) parts.push(`${rest} ${rest === 1 ? "bit" : "bits"}`);
  return parts.join(" ");
}

/**
 * The selected bits as one number, in the order they are given and MSB first
 * inside each byte, which is how the rest of the editor counts bits. Null
 * while any of the bytes are still on their way.
 *
 * `reverseBytes` reads a single whole-byte run the other way round, for a
 * format that stored it little-endian.
 */
function readBits(doc: Doc, ranges: readonly BitRange[], reverseBytes = false): bigint | null {
  let out = 0n;
  for (const r of ranges) {
    const bits = r.endBit - r.startBit;
    const { bytes, complete } = doc.readBits(r.startBit, bits);
    if (!complete) return null;
    const ordered = reverseBytes ? Array.from(bytes).reverse() : bytes;
    let chunk = 0n;
    for (const b of ordered) chunk = (chunk << 8n) | BigInt(b);
    // `readBits` packs from the top, so a run that does not fill its last byte
    // leaves padding at the bottom of it.
    chunk >>= BigInt(bytes.length * 8 - bits);
    out = (out << BigInt(bits)) | chunk;
  }
  return out;
}

/** The same bits read as two's complement over their own width. */
function signed(v: bigint, bits: number): bigint {
  const top = 1n << BigInt(bits - 1);
  return (v & top) === 0n ? v : v - (1n << BigInt(bits));
}

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
const ROLE_TEXT = { length: "Length", count: "Count", type: "Type", position: "Position", value: "Value", points: "" } as const;

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

function pad(value: number): string {
  return value.toString().padStart(2, "0");
}

function hex8(value: number): string {
  return `0x${(value & 0xff).toString(16).padStart(2, "0")}`;
}

function hex16(value: number): string {
  return `0x${(value & 0xffff).toString(16).padStart(4, "0")}`;
}

function unixDate(seconds: number, suffix: string): string {
  if (seconds === 0 && suffix === "not specified") return "Not specified (stored as 0)";
  const date = new Date(seconds * 1000);
  if (!Number.isFinite(date.getTime())) return "Invalid Unix timestamp";
  return `${date.toISOString().replace("T", " ").replace(".000Z", "")} (${suffix})`;
}

/** ISO base media and QuickTime count seconds from 1904-01-01 UTC. */
function quickTimeDate(seconds: number): string {
  const unixSeconds = seconds - 2_082_844_800;
  const date = new Date(unixSeconds * 1000);
  if (!Number.isFinite(date.getTime())) return "Invalid QuickTime timestamp";
  return `${date.toISOString().replace("T", " ").replace(".000Z", "")} (QuickTime epoch, UTC)`;
}

async function decompress(bytes: Uint8Array, format: "gzip" | "deflate-raw"): Promise<Uint8Array> {
  if (typeof DecompressionStream === "undefined") throw new Error("This browser cannot decompress data for the check.");
  // Current Chromium implements deflate-raw; older DOM typings only name
  // gzip and deflate.
  const stream = new Blob([Uint8Array.from(bytes)]).stream().pipeThrough(new DecompressionStream(format as CompressionFormat));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

// Value inspector: interprets the bytes under the cursor as common primitive
// types and writes edits back. Every row is a small two-way lens: decode bytes
// to text, parse text to bytes.
//
// The cursor is a bit position, so these readings start wherever it is: put the
// cursor three bits into a byte and the rows show what a u16 there would say.

import { formatAddress, formatBytes, formatOffset } from "./doc.js";
import type { BitRange } from "./hexview.js";
import type { Doc, FieldGraph, Origin, Relation, Shape, TemplateNode, TemplateReply } from "./doc.js";
import { LENSES, type Lens } from "./lenses.js";
import { bitSizeText, childWord, childrenHead, countText, INSIDE, PROPERTIES, REPORT, ROLE_GROUP, DECODED_INSIDE, DECODED_REFUSED, DECODED_REFUSED_OTHER, UNPACKED, unpackedOriginRow } from "./strings.js";
import { CHILD_PAGE, insideValue, PREVIEW_ITEMS, type Inside } from "./composite.js";
import { fieldClass } from "./fieldstyle.js";
import { withPictures } from "./textview.js";
import { typePanel } from "./typepanel.js";
import { fieldNumber, openPlan, type OpenPlan } from "./openplan.js";
import { extraction } from "./bitextract.js";
import { crc32, hex32, hexBytes, lhaCrc16, sha1, sum8 } from "./integrity.js";
import {
  CODEPAGE_A_DEFAULT,
  CODEPAGE_A_KEY,
  CODEPAGE_B_DEFAULT,
  CODEPAGE_B_KEY,
  CODEPAGES_A,
  CODEPAGES_B,
  LITERAL_LANG_DEFAULT,
  LITERAL_LANG_KEY,
  LITERAL_LANG_NAMES,
  LITERAL_LANGS,
  rememberChoice,
  storedChoice,
} from "./encodings.js";

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
  /** The type and size of what the box is showing, under the box: the reader's
   *  first question about a value is what it is. */
  private readonly shape: HTMLElement;
  /** What a structure holds, listed under it. */
  private readonly kids: HTMLElement;
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
  /** The selection read as text, every way it can be. */
  private readonly selText: HTMLElement;
  /** What the text view is reading the file in, so the likeliest reading of a
   *  selection is the one at the top. Empty when it settled for itself. */
  textEncoding = "";
  /** Which single-byte page each of the two slots is reading high bytes as.
   *  Nothing in a file says which page it is in, so this is the reader's own
   *  choice and is kept between visits. */
  private pageA = storedChoice(CODEPAGE_A_KEY, CODEPAGES_A, CODEPAGE_A_DEFAULT);
  private pageB = storedChoice(CODEPAGE_B_KEY, CODEPAGES_B, CODEPAGE_B_DEFAULT);
  /** Which language the string literal row is written in. */
  private literalLang = storedChoice(LITERAL_LANG_KEY, LITERAL_LANGS, LITERAL_LANG_DEFAULT);
  /** Which of the choosers to put the keyboard back on after a re-render, so
   *  changing one with the keyboard does not lose it. */
  private refocus: string | null = null;
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
  /** How many of a structure's children the list has been asked to show.
   *  Grows a page at a time and starts over on a different field, so opening
   *  one long list does not make the next one long. */
  private childCap = CHILD_PAGE;
  private childCapFor = "";
  /** Which property rows are unfolded, and which field they belong to, as its
   *  path written the way `data-path` writes it. Kept on the panel rather than
   *  in the DOM so that re-reading the same field does not fold them up again,
   *  and cleared by moving to another field, whose properties are other
   *  questions and start folded. */
  private openProps = new Set<string>();
  private openPropsFor: string | null = null;
  /** A row unfolded from the keyboard, whose head went with the rest of the
   *  section when it was rebuilt. Focus goes back to the row that replaced it. */
  private focusProp: string | null = null;
  /** The dependency row the pointer is resting on, so it can be unmarked when
   *  the pointer moves off it or the section is rebuilt underneath it. */
  private hoverRow: HTMLElement | null = null;

  /** Asked for when a breadcrumb is clicked, so the views can follow. */
  onPick: (path: readonly number[]) => void = () => {};
  /** Asked for when the reader follows an offset, so the views can follow. */
  onGoTo: (bitOffset: number, ranges?: readonly BitRange[]) => void = () => {};
  /** The reader is pointing at a row naming another field, or has moved off it.
   *  Null means nothing is being pointed at. */
  onHoverField: (path: readonly number[] | null) => void = () => {};
  /** Asked for when the reader opens a field's bytes as their own document. */
  onOpenTab: (bytes: Uint8Array, name: string, origin: string) => void = () => {};
  /** A compressed run was asked for as a document of its own. */
  onOpenUnpacked: (path: readonly number[]) => void = () => {};

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
      // The row carries the path, not only the name inside it, so the value
      // beside the name and the space after it lead to the same field.
      const row = t.closest<HTMLElement>("[data-path]");
      const p = row?.dataset["path"];
      if (p !== undefined) this.onPick(pathOf(p));
    });
    // Pointing at a row that names another field says so, so the views can
    // mark that field while the pointer rests here: the two ends of one
    // connection, lit at the same time. A row that names no field of its own,
    // and the space between rows, are pointing at nothing.
    this.origins.addEventListener("mouseover", (e) => {
      const t = e.target;
      this.markHover(t instanceof HTMLElement ? t.closest<HTMLElement>("[data-path]") : null);
    });
    this.origins.addEventListener("mouseleave", () => this.markHover(null));
    this.openAs = document.createElement("div");
    this.openAs.className = "insp-openas";
    this.openAs.hidden = true;
    this.shape = document.createElement("div");
    this.shape.className = "insp-detail insp-shape";
    this.shape.hidden = true;
    // A structure's children, which are the value it does not have one of.
    // The rows behave as the origin rows do: a click goes to the child, and
    // pointing at one lights it in the views while the pointer rests here.
    this.kids = document.createElement("div");
    this.kids.className = "insp-kids";
    this.kids.hidden = true;
    this.kids.addEventListener("click", (e) => {
      const t = e.target;
      if (!(t instanceof HTMLElement)) return;
      if (t.dataset["more"] !== undefined) {
        this.childCap += CHILD_PAGE;
        this.render();
        return;
      }
      const p = t.closest<HTMLElement>("[data-path]")?.dataset["path"];
      if (p !== undefined) this.onPick(pathOf(p));
    });
    this.kids.addEventListener("mouseover", (e) => {
      const t = e.target;
      this.markHover(t instanceof HTMLElement ? t.closest<HTMLElement>("[data-path]") : null);
    });
    this.kids.addEventListener("mouseleave", () => this.markHover(null));
    this.fieldRow.append(subhead("Value"), this.field, this.area, this.shape, this.note, this.kids, this.semantics, this.openAs, this.origins, this.types);
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
    this.selText = document.createElement("div");
    this.selText.className = "insp-seltext";
    this.selectionEl.append(subhead(SEL_TITLE), this.selWhere, this.selTable, this.selText, this.selStatus);

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
    // Whole bytes lying together, which is what characters are made of. Not
    // the `whole` above: that also wants more than one byte, because reversing
    // the bytes of a single one says nothing. One byte is a character.
    const readable = ranges.length === 1 && one !== undefined && one.startBit % 8 === 0 && bits % 8 === 0 && bits >= 8;
    this.renderSelectionText(ranges, bits, readable);
  }

  /**
   * The selection read as text. Only for a run of whole bytes lying together:
   * a selection made over the bits is not characters, which is the same reason
   * the byte-reversed number rows only appear for whole bytes.
   *
   * One row per encoding, always in the same order, so a reading is found in
   * the same place from one selection to the next. The two code-page rows are
   * chosen by the reader: the chooser is the row's label, and what the page
   * makes of the bytes sits beside it, so a change is seen where it was made.
   * A row that agrees with the UTF-8 one is drawn quieter, so the one that
   * differs is the one that stands out.
   */
  private renderSelectionText(ranges: readonly BitRange[], bits: number, readable: boolean): void {
    const one = ranges[0];
    // A render that draws no choosers has none to put the keyboard back on,
    // and a note left standing would take focus off whatever is next.
    if (!readable || one === undefined || bits === 0) {
      this.refocus = null;
      this.selText.hidden = true;
      this.selText.replaceChildren();
      return;
    }
    const atByte = one.startBit / 8;
    const nBytes = bits / 8;
    const got = this.doc.selectionText(atByte, nBytes, this.textEncoding, this.pageA, this.pageB);
    if (got === null || (got.readings.length === 0 && got.refused.length === 0)) {
      this.refocus = null;
      this.selText.hidden = true;
      return;
    }
    const by = new Map<string, string>();
    for (const r of got.readings) for (const enc of r.encodings) by.set(enc, r.text);
    const utf8 = by.get("UTF-8") ?? null;
    const ascii = by.has("ASCII");
    const rows: Node[] = [
      this.readingRow(ascii ? SEL_TEXT_UTF8_ASCII : "UTF-8", "UTF-8", by.get("UTF-8"), null),
      this.readingRow("UTF-16 LE", "UTF-16 LE", by.get("UTF-16 LE"), utf8),
      this.readingRow("UTF-16 BE", "UTF-16 BE", by.get("UTF-16 BE"), utf8),
      this.readingRow(this.pagePicker("a"), this.pageA, by.get(this.pageA), utf8),
      this.readingRow(this.pagePicker("b"), this.pageB, by.get(this.pageB), utf8),
      this.literalRow(atByte, nBytes),
    ];
    const parts: Node[] = [subhead(SEL_TEXT_TITLE), ...rows];
    if (!got.all) {
      const cut = document.createElement("div");
      cut.className = "insp-reading-miss";
      cut.textContent = SEL_TEXT_PARTIAL(got.read);
      parts.push(cut);
    }
    this.selText.replaceChildren(...parts);
    this.selText.hidden = false;
    // Changing a chooser rebuilds these rows and throws the old one away, so
    // the keyboard is put back on the new one in its place.
    if (this.refocus !== null) {
      const back = this.selText.querySelector<HTMLSelectElement>(`select[data-slot="${this.refocus}"]`);
      this.refocus = null;
      back?.focus();
    }
  }

  /**
   * One encoding's row. `label` is what names the row: plain text, or the
   * chooser for a code-page row. `text` undefined means the bytes are not
   * valid in this encoding, which is said in the reading's place rather than
   * by leaving the row out: a missing row reads as a forgotten one.
   * `same` is the UTF-8 reading, for quietening a row that only repeats it.
   */
  private readingRow(label: string | HTMLElement, name: string, text: string | undefined, same: string | null): HTMLElement {
    const row = document.createElement("div");
    row.className = "insp-reading";
    const who = document.createElement("span");
    who.className = "insp-reading-enc";
    who.append(label);
    row.append(who);
    if (text === undefined) {
      const miss = document.createElement("span");
      miss.className = "insp-reading-miss";
      miss.textContent = SEL_TEXT_NOT_VALID;
      row.append(miss);
      return row;
    }
    if (same !== null && text === same) row.classList.add("is-same");
    this.readingCell(row, text, withPictures(text), copyLabel(name), expandLabel(name), collapseLabel(name));
    return row;
  }

  /** The text of a row, with its copy and expand buttons. `shown` is the text
   *  as drawn; `text` is what gets copied. */
  private readingCell(row: HTMLElement, text: string, shown: string, copyTip: string, expandTip: string, collapseTip: string): void {
    const cell = document.createElement("span");
    cell.className = "insp-reading-text";
    // Control characters as their pictures, the way the text view shows
    // them: a selected line feed drawn as a line feed is a row that looks
    // empty, and empty is what "these bytes say nothing" would look like.
    cell.textContent = shown;
    cell.title = text;
    const acts = document.createElement("div");
    acts.className = "insp-acts";
    const copy = actionButton(COPY, copyTip);
    const expand = actionButton(EXPAND, expandTip);
    acts.append(copy, expand);
    row.append(cell, acts);
    copy.addEventListener("click", () => void this.copyValue(text));
    expand.addEventListener("click", () => {
      const open = row.classList.toggle("is-open");
      expand.textContent = open ? COLLAPSE : EXPAND;
      const label = open ? collapseTip : expandTip;
      expand.title = label;
      expand.setAttribute("aria-label", label);
    });
    // A reading that already fits has nothing to expand into.
    row.addEventListener("pointerenter", () => {
      expand.hidden = !row.classList.contains("is-open") && cell.scrollWidth <= cell.clientWidth;
    });
  }

  /** The chooser that is one code-page row's label. Which page a run of high
   *  bytes is read as is the reader's own decision, so it is made on the row
   *  that shows the answer. */
  private pagePicker(slot: "a" | "b"): HTMLSelectElement {
    const pages = slot === "a" ? CODEPAGES_A : CODEPAGES_B;
    const pick = picker(pages, slot === "a" ? this.pageA : this.pageB, SEL_TEXT_PAGE_LABEL, slot);
    pick.addEventListener("change", () => {
      if (slot === "a") this.pageA = pick.value;
      else this.pageB = pick.value;
      rememberChoice(slot === "a" ? CODEPAGE_A_KEY : CODEPAGE_B_KEY, pick.value);
      this.refocus = slot;
      this.renderSelection();
    });
    return pick;
  }

  /** The selection as a string literal, in whichever language is wanted: what
   *  to paste into a parser being written against the file. */
  private literalRow(atByte: number, nBytes: number): HTMLElement {
    const row = document.createElement("div");
    row.className = "insp-reading insp-reading-literal";
    const who = document.createElement("span");
    who.className = "insp-reading-enc";
    const pick = picker(LITERAL_LANGS, this.literalLang, SEL_TEXT_LANG_LABEL, "lang", LITERAL_LANG_NAMES);
    pick.addEventListener("change", () => {
      this.literalLang = pick.value;
      rememberChoice(LITERAL_LANG_KEY, pick.value);
      this.refocus = "lang";
      this.renderSelection();
    });
    who.append(pick);
    row.append(who);
    const lit = this.doc.selectionLiteral(atByte, nBytes, this.literalLang) ?? "";
    this.readingCell(row, lit, lit, SEL_TEXT_LITERAL_COPY, SEL_TEXT_LITERAL_EXPAND, SEL_TEXT_LITERAL_COLLAPSE);
    return row;
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
    at.textContent = formatAddress(n.offset_bits, n.space);
    // A field read out of a compressed stream is at an address of that
    // stream, not of the file, and the two look the same written down. The
    // trail above already says which stream; this says which space the number
    // belongs to, beside the number.
    // The type and the size used to be here too. They moved under the box,
    // beside the value they describe: a reader who wants to know what they
    // are looking at looks at the value first and the line under it next.
    const inside = n.space === 0 ? "" : ` ${DECODED_INSIDE}`;
    this.detail.replaceChildren(at, inside);
    this.shape.textContent = `${n.type} · ${bitSizeText(n.size_bits)}`;
    this.shape.hidden = false;
    // The formula reads bytes of the file by address. There is no address of
    // the file for these bytes, so there is no formula to write.
    if (n.space === 0) this.showFormula(n.offset_bits, n.size_bits, false);
    else {
      this.formula.hidden = true;
      this.formula.replaceChildren();
    }
    // A compressed run nothing could open says so where its value would be.
    if (n.refused !== null) {
      const why = DECODED_REFUSED[n.refused] ?? DECODED_REFUSED_OTHER;
      this.detail.append(` · ${why}`);
    }
    this.fillValue(path, n);
    this.fillProperties(path, n);
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
    const parts: Node[] = [];
    // A compressed run that opened can be read in its own right, in a tab that
    // stays connected to the bytes it came from. A run that would not open has
    // no button; the line saying why is already beside the address above.
    if (n.decoded && n.refused === null) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "insp-check-button";
      button.textContent = UNPACKED.open;
      button.title = UNPACKED.openTitle(n.name);
      button.addEventListener("click", () => this.onOpenUnpacked(path));
      parts.push(button);
    }
    const plan = openPlan(this.doc, path, n);
    if (plan !== null) {
      const detail = document.createElement("div");
      detail.className = "insp-detail";
      detail.textContent = plan.detail;
      parts.push(subhead("Open as a file"), detail);
      if (plan.load !== null) parts.push(this.openButton(plan));
    }
    if (parts.length === 0) {
      this.openAs.hidden = true;
      this.openAs.replaceChildren();
      return;
    }
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
      return this.dosDateText(path, "modified_date", "modified_time");
    }
    if (this.doc.template === "cab" && (n.name === "date" || n.name === "time")) {
      return this.dosDateText(path, "date", "time");
    }
    return null;
  }

  /** The two halves of an MS-DOS timestamp, which a ZIP and a cabinet both
   * keep as a packed date beside a packed time. Either field shows the whole
   * moment, since neither says much alone. */
  private dosDateText(path: readonly number[], dateField: string, timeField: string): string | null {
    const siblings = this.siblings(path);
    const time = siblings.find((x) => x.name === timeField);
    const date = siblings.find((x) => x.name === dateField);
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
    // The dependency rows go out of sight with the rest, and the pointer cannot
    // leave a row it can no longer see, so anything marked is unmarked here.
    this.markHover(null);
    this.fieldRow.hidden = true;
    this.detail.hidden = true;
    this.shape.hidden = true;
    this.kids.hidden = true;
    this.kids.replaceChildren();
    this.formula.hidden = true;
  }

  /**
   * What is true of the field at the cursor, one question to a row.
   *
   * The section used to be arranged the other way about: a heading per *other*
   * field, saying what that field had settled here. That answers a question a
   * reader only has once they know one was asked, and it had nothing at all to
   * say about the fields nobody decided anything about, which is most of a
   * file. What a reader asks first is plainer and is always the same handful of
   * things: where is this, how long is it, why is it this type, what points at
   * it. So each row is one of those questions, the answer is on the row, and
   * the field that settled it is the working behind the answer rather than the
   * heading over it.
   *
   * Every row folds. Its own line carries the fact and one short clause saying
   * how the fact was arrived at, which is what a reader passing through wants;
   * the fields named and the arithmetic over them are one click below. The
   * structures above the field fold away together at the end, because 128 bytes
   * of packed weights are 128 bytes because of a record three levels up, and
   * that record is worth reaching and is still not the field the reader is
   * standing on.
   */
  private fillProperties(path: readonly number[], n: TemplateNode): void {
    // The rows are about to be thrown away, so whatever the pointer was
    // resting on is no longer there to point at. Say so before it goes.
    this.markHover(null);
    const key = path.join("/");
    // Unfolding is about one field's questions, so it is forgotten as soon as
    // the panel is about another field.
    if (this.openPropsFor !== key) {
      this.openPropsFor = key;
      this.openProps.clear();
    }
    const rows = this.properties(path, n, "", false);
    const above = this.aboveBlocks(path);
    if (rows.length === 0 && above.length === 0) {
      this.origins.hidden = true;
      this.origins.replaceChildren();
      return;
    }
    const all: Node[] = [subhead(PROPERTIES.head)];
    for (const p of rows) all.push(this.propertyEl(p));
    if (above.length > 0) {
      const open = this.openProps.has(ABOVE_KEY);
      all.push(this.disclosure(ABOVE_KEY, open, PROPERTIES.above(above.length, above[0]?.name ?? "")));
      if (open) for (const block of above) all.push(...block.rows);
    }
    this.origins.replaceChildren(...all);
    this.origins.hidden = false;
    // The head that was clicked went with the rest of the section. Focus lands
    // on the one that replaced it, so unfolding a row from the keyboard does
    // not put the reader back at the top of the panel.
    if (this.focusProp !== null) {
      this.origins.querySelector<HTMLElement>(`[data-prop="${this.focusProp}"]`)?.focus();
      this.focusProp = null;
    }
  }

  /**
   * The questions worth asking about one field, in the order a reader asks
   * them: where it is, how much of it there is, what it is, and only then what
   * it has to do with other fields.
   *
   * `terse` is for the structures above the field, where a row is worth
   * printing only if another field settled it: that a record's own length is
   * fixed by its type is a fact about the record, and the reader is looking at
   * something inside it.
   */
  private properties(path: readonly number[], n: TemplateNode, prefix: string, terse: boolean): Property[] {
    const from = new Map<OriginRole, Origin[]>();
    const jumps: Origin[] = [];
    // The stream these bytes were unpacked out of, which the core reports as an
    // origin of the field's value. It is not one of the fields a value was
    // worked out from: it is where the bytes came from at all, so it belongs
    // under Position with the rest of the answer to "why is it here". What
    // tells the two apart is that a stream is above the field and a field read
    // by an expression never is.
    const stream: Origin[] = [];
    const reply = this.doc.origins(path);
    // Two accounts of the same window name the same field twice: the length of
    // the sized window around a run of bytes, and the length of the bytes
    // inside it. One fact, so one row. An origin with no field of its own has
    // no identity to compare and is always kept.
    const seen = new Set<string>();
    if (reply.status === "ok") {
      for (const o of reply.node) {
        if (o.role === "points") {
          if (o.target_bits !== null) jumps.push(o);
          continue;
        }
        const at = `${o.role} ${o.path.join("/")}`;
        if (o.path.length > 0) {
          if (seen.has(at)) continue;
          seen.add(at);
        }
        if (o.role === "value" && encloses(o.path, path)) stream.push(o);
        else push(from, o.role, o);
      }
    }
    const how = new Map<OriginRole, Relation[]>();
    const said = this.doc.relations(path);
    if (said.status === "ok") for (const r of said.node) push(how, r.role, r);
    const shape = this.doc.shape(path);
    const said_ = (roles: readonly OriginRole[]): boolean =>
      roles.some((r) => (from.get(r)?.length ?? 0) + (how.get(r)?.length ?? 0) > 0);
    const out: Property[] = [];
    // Where it is, and how long. Both are answered for every field, since both
    // have an answer for every field; the rest of the rows appear only when
    // there is something to put on them.
    if (!terse || said_(["position"])) {
      const detail = this.working(["position"], from, how);
      if (!terse) detail.push(...this.insideRows(path, n), ...this.unpackedRows(path, stream));
      out.push({
        key: `${prefix}position`,
        label: PROPERTIES.position,
        value: formatAddress(n.offset_bits, n.space),
        bit: null,
        how: this.placedHow(path, n, shape, from),
        detail,
      });
    }
    if (!terse || said_(["length", "width"])) {
      out.push({
        key: `${prefix}length`,
        label: PROPERTIES.length,
        value: bitSizeText(n.size_bits),
        bit: null,
        how: this.sizedHow(path, shape, from),
        detail: this.working(["length", "width"], from, how),
      });
    }
    // The type, only where the file rather than the template settled it: the
    // line under the value already says what the type is, and a row repeating
    // it to say the template said so is a row that never varies.
    if (said_(["type"])) {
      const one = only(from.get("type"));
      out.push({
        key: `${prefix}type`,
        label: PROPERTIES.type,
        value: n.type,
        bit: null,
        how: one === null ? null : { text: PROPERTIES.type_from(one.label, one.value), path: one.path },
        detail: this.working(["type"], from, how),
      });
    }
    if (said_(["count"])) {
      const one = only(from.get("count"));
      out.push({
        key: `${prefix}count`,
        label: PROPERTIES.count,
        value: countText(n.child_count, childWord(n)),
        bit: null,
        how: one === null ? null : { text: originClause(one), path: one.path },
        detail: this.working(["count"], from, how),
      });
    }
    // A field of no bytes whose value other fields come to. The editor above
    // shows what it says; this says what it was worked out from.
    if (said_(["value"])) {
      const one = only(from.get("value"));
      out.push({
        key: `${prefix}computed`,
        label: PROPERTIES.computed,
        value: one === null ? "" : one.label,
        bit: null,
        how: null,
        detail: this.working(["value"], from, how),
      });
    }
    // Where the name on the row came from, when the file rather than the
    // template spells it.
    if (said_(["name"])) {
      const one = only(from.get("name"));
      out.push({
        key: `${prefix}name`,
        label: PROPERTIES.name,
        value: one === null ? "" : one.label,
        bit: null,
        how: null,
        detail: this.working(["name"], from, how),
      });
    }
    if (terse) return out;
    // Where this field's own value points, when it holds an offset. One row
    // each: a field that is two pointers is rare, and folding them together
    // would need a heading saying which is which.
    for (const [i, o] of jumps.entries()) {
      out.push({
        key: `${prefix}points:${i}`,
        label: PROPERTIES.points,
        value: formatOffset(o.target_bits ?? 0),
        bit: o.target_bits,
        how: { text: o.label, path: null },
        detail: [],
      });
    }
    const used = this.usedBy(path);
    if (used.length > 0) {
      out.push({
        key: `${prefix}readby`,
        label: PROPERTIES.readBy,
        value: PROPERTIES.readByCount(used.length),
        bit: null,
        how: null,
        detail: used,
      });
    }
    return out;
  }

  /**
   * The working behind one property: the fields it names, and the arithmetic
   * done over them.
   *
   * The fields and the sentence they appear in belong together. The rows name
   * which fields settled this much of the field's shape; the expression under
   * them says what was done with those fields, so the number can be checked
   * rather than taken. A role heading appears only where one property covers
   * two of them, which is Length over a run of packed numbers: how long the run
   * is and how wide one value in it is are different questions with the same
   * unit.
   */
  private working(roles: readonly OriginRole[], from: Map<OriginRole, Origin[]>, how: Map<OriginRole, Relation[]>): Node[] {
    const filled = roles.filter((r) => (from.get(r)?.length ?? 0) + (how.get(r)?.length ?? 0) > 0);
    const out: Node[] = [];
    for (const role of filled) {
      if (filled.length > 1) out.push(roleHead(ROLE_GROUP[role] ?? role));
      for (const o of from.get(role) ?? []) out.push(originRow(o));
      for (const r of how.get(role) ?? []) out.push(relationRow(r));
    }
    return out;
  }

  /**
   * How the field's start was settled, in one clause.
   *
   * Read off the core's word for it rather than worked out here: the panel has
   * no way of telling a field that follows the one before it from one an
   * address put in the same place, and a clause guessed in this position would
   * read exactly like a fact the file gave. Where the core says it does not
   * know, this says nothing.
   */
  private placedHow(path: readonly number[], n: TemplateNode, shape: TemplateReply<Shape>, from: Map<OriginRole, Origin[]>): How | null {
    if (shape.status !== "ok") return null;
    const one = only(from.get("position"));
    const up = path.slice(0, -1);
    const parent = this.nameOf(up);
    const idx = path[path.length - 1] ?? 0;
    switch (shape.node.placed) {
      case "root":
        return { text: PROPERTIES.placed.root, path: null };
      case "first":
        return parent === null ? null : { text: PROPERTIES.placed.first(parent), path: up };
      case "follows": {
        // The field before it, by name: "after the field before it" is a fact
        // the reader can already see in the listing, and the name is what lets
        // them go to it.
        const prev = [...up, idx - 1];
        const before = idx > 0 ? this.nameOf(prev) : null;
        if (before === null) return { text: PROPERTIES.placed.followsPlain, path: null };
        return { text: PROPERTIES.placed.follows(before), path: prev };
      }
      case "element":
        return parent === null
          ? { text: PROPERTIES.placed.elementPlain(idx), path: null }
          : { text: PROPERTIES.placed.element(idx, parent), path: up };
      case "pointer":
        return one === null ? { text: PROPERTIES.placed.pointerPlain, path: null } : { text: PROPERTIES.placed.pointer(one.label), path: one.path };
      case "chain":
        return one === null ? { text: PROPERTIES.placed.chainPlain, path: null } : { text: PROPERTIES.placed.chain(one.label), path: one.path };
      case "address":
        return one === null ? { text: PROPERTIES.placed.addressPlain, path: null } : { text: PROPERTIES.placed.address(one.label), path: one.path };
      case "trace":
        return { text: PROPERTIES.placed.trace, path: null };
      case "stream":
        return parent === null ? { text: PROPERTIES.placed.streamPlain, path: null } : { text: PROPERTIES.placed.stream(parent), path: up };
      default:
        return n.space === 0 ? null : { text: PROPERTIES.placed.streamPlain, path: null };
    }
  }

  /** How the field's length was settled, in one clause. Read off the core's
   *  word for it, for the reason `placedHow` is. */
  private sizedHow(path: readonly number[], shape: TemplateReply<Shape>, from: Map<OriginRole, Origin[]>): How | null {
    if (shape.status !== "ok") return null;
    const parent = this.nameOf(path.slice(0, -1));
    switch (shape.node.sized) {
      case "fixed":
        return { text: PROPERTIES.sized.fixed, path: null };
      case "expression": {
        // Named only where one field settles it. Two fields and an expression
        // over them is a formula, and half of a formula on the row would read
        // as the whole of it.
        const one = only(from.get("length")) ?? only(from.get("width"));
        return one === null ? { text: PROPERTIES.sized.expressionPlain, path: null } : { text: PROPERTIES.sized.expression(one.label), path: one.path };
      }
      case "terminated":
        return { text: PROPERTIES.sized.terminated, path: null };
      case "remaining":
        return parent === null
          ? { text: PROPERTIES.sized.remainingPlain, path: null }
          : { text: PROPERTIES.sized.remaining(parent), path: path.slice(0, -1) };
      case "children":
        return { text: PROPERTIES.sized.children, path: null };
      case "count": {
        const one = only(from.get("count"));
        return one === null ? { text: PROPERTIES.sized.countPlain, path: null } : { text: PROPERTIES.sized.count(one.label), path: one.path };
      }
      case "encoded":
        return { text: PROPERTIES.sized.encoded, path: null };
      case "trace":
        return { text: PROPERTIES.sized.trace, path: null };
      case "nothing":
        return { text: PROPERTIES.sized.nothing, path: null };
      default:
        return null;
    }
  }

  /**
   * Where the field sits inside the structures around it.
   *
   * An absolute address answers where it is in the file, which is not the
   * question a reader of a record has: a local file header's `crc32` is at
   * `+0xe` of that header wherever in the zip the header landed, and that is
   * the number the specification prints. The nearest few are enough; a member
   * eight levels down a JSON tree has eight of these and only the first are
   * about anything the reader can hold in their head.
   */
  private insideRows(path: readonly number[], n: TemplateNode): Node[] {
    const rows: Node[] = [];
    for (let i = path.length - 1; i >= 1 && rows.length < INSIDE_LEVELS; i--) {
      const at = path.slice(0, i);
      const a = this.doc.templateNode(at);
      // A stretch counted in another address space is not a distance from
      // here: the field is at an offset of the unpacked bytes and its stream
      // is at an offset of the file.
      if (a.status !== "ok" || a.node.space !== n.space) continue;
      const delta = n.offset_bits - a.node.offset_bits;
      if (delta < 0) continue;
      rows.push(insideRow(a.node.name, delta, at));
    }
    return rows.length === 0 ? [] : [roleHead(PROPERTIES.inside), ...rows];
  }

  /**
   * Where the bytes came from, for a field that was read out of a compressed
   * run rather than out of the file.
   *
   * Two halves of one answer: which run it was unpacked from, which is a field
   * of this document with a place to go and look at, and, for a stream opened
   * as a document of its own, which step of the decoder produced the byte under
   * the cursor and which bits of the compressed run that step read. The second
   * is the same sentence as the status bar, deliberately: it is the same fact,
   * and a reader who has seen it below should recognise it here.
   */
  private unpackedRows(path: readonly number[], stream: readonly Origin[]): Node[] {
    const rows: Node[] = stream.map(originRow);
    const n = this.doc.templateNode(path);
    if (!this.doc.isFile && n.status === "ok") {
      const step = this.doc.mapOut(Math.floor(n.node.offset_bits / 8));
      if (step !== null) {
        const row = document.createElement("div");
        row.className = "insp-origin";
        const what = document.createElement("span");
        what.textContent = unpackedOriginRow(this.doc.name, step.in_start, step.in_end, step.kind, step.len, step.dist, step.field);
        row.append(what);
        rows.push(row);
      }
    }
    return rows.length === 0 ? [] : [roleHead(UNPACKED.originHead), ...rows];
  }

  /**
   * What the structures above the field settled about it, nearest first, each
   * headed by the field it is about.
   *
   * A reader who did not ask about that field has to be told whose length this
   * is, so the heading is the field's name and leads to it.
   */
  private aboveBlocks(path: readonly number[]): { name: string; rows: Node[] }[] {
    const out: { name: string; rows: Node[] }[] = [];
    for (let i = path.length - 1; i >= 0; i--) {
      const at = path.slice(0, i);
      const node = this.doc.templateNode(at);
      if (node.status !== "ok") continue;
      const rows = this.properties(at, node.node, `${at.join("/")}:`, true);
      if (rows.length === 0) continue;
      out.push({ name: node.node.name, rows: [this.stepHead(at), ...rows.map((p) => this.propertyEl(p))] });
    }
    return out;
  }

  /**
   * One property as it is drawn: the label and the answer on one line, the
   * clause saying how it was settled under it, and the working folded away
   * below that.
   *
   * The head is a button only where there is something behind it, so a row that
   * folds and a row that does not are told apart before the triangle is read.
   * The clause is outside the head rather than in it, because it leads to the
   * field it names and a link inside a button is neither valid nor operable.
   */
  private propertyEl(p: Property): HTMLElement {
    const box = document.createElement("div");
    box.className = "insp-prop";
    const open = this.openProps.has(p.key);
    const foldable = p.detail.length > 0;
    const head = document.createElement(foldable ? "button" : "div");
    head.className = "insp-prop-head";
    const mark = document.createElement("span");
    mark.className = "insp-prop-mark";
    mark.setAttribute("aria-hidden", "true");
    mark.textContent = foldable ? (open ? "▾" : "▸") : "";
    const label = document.createElement("span");
    label.className = "insp-prop-name";
    label.textContent = p.label;
    head.append(mark, label, this.propertyValue(p));
    if (head instanceof HTMLButtonElement) {
      head.type = "button";
      head.setAttribute("aria-expanded", String(open));
      head.dataset["prop"] = p.key;
      head.addEventListener("click", () => {
        if (open) this.openProps.delete(p.key);
        else this.openProps.add(p.key);
        this.focusProp = p.key;
        this.render();
      });
    }
    box.append(head);
    if (p.how !== null) {
      const how = document.createElement("div");
      how.className = "insp-prop-how";
      // The clause names a field the reader can go to and light up, so the
      // whole line carries the path the way an origin row does.
      if (p.how.path !== null) how.dataset["path"] = p.how.path.join("/");
      how.textContent = p.how.text;
      box.append(how);
    }
    if (foldable && open) {
      const detail = document.createElement("div");
      detail.className = "insp-prop-detail";
      detail.append(...p.detail);
      box.append(detail);
    }
    return box;
  }

  /** The answer itself. An answer that is a place in the file is a link to
   *  that place; every other answer is a word. */
  private propertyValue(p: Property): HTMLElement {
    const value = document.createElement(p.bit === null ? "span" : "button");
    value.className = p.bit === null ? "insp-prop-value" : "insp-prop-value insp-link addr";
    if (value instanceof HTMLButtonElement) {
      value.type = "button";
      value.dataset["bit"] = String(p.bit);
    }
    value.textContent = p.value;
    return value;
  }

  /**
   * A control that folds a block away, and the count of what is behind it.
   *
   * The same words open or shut. A disclosure whose label changes asks the
   * reader to read it twice to find out which way it is; the triangle beside it
   * already says that.
   */
  private disclosure(key: string, open: boolean, text: string): HTMLElement {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "insp-link insp-origin-more";
    b.setAttribute("aria-expanded", String(open));
    b.dataset["prop"] = key;
    const mark = document.createElement("span");
    mark.className = "insp-origin-mark";
    mark.textContent = open ? "▾" : "▸";
    mark.setAttribute("aria-hidden", "true");
    b.append(mark, text);
    b.addEventListener("click", () => {
      if (open) this.openProps.delete(key);
      else this.openProps.add(key);
      this.focusProp = key;
      this.render();
    });
    return b;
  }

  /** What a field is called, for a clause that names it. Null when it cannot
   *  be read, which is when the clause has to be worded without it. */
  private nameOf(path: readonly number[]): string | null {
    const node = this.doc.templateNode(path);
    return node.status === "ok" ? node.node.name : null;
  }

  /**
   * The other direction: which fields read this one.
   *
   * The core answers "what decided this field" one field at a time and has no
   * call for the reverse, so the reverse is read off the graph of a subtree
   * holding this field and turned round.
   *
   * Which subtree is the whole question. Only the file gives a complete
   * answer: an expression can reach a field by a path from anywhere
   * (`header.record[2].format`), so a field inside one structure can be read
   * by a field in another. The file is tried first for that reason, with the
   * enclosing structure as the fallback where the file is too big to walk. In
   * the fallback the list is what that structure knows, which is most of the
   * answer and not all of it.
   *
   * Rows stay flat, with the role word on each. Grouped under a heading the
   * word would flip its referent: `Length` above means what decided this
   * field's length, and here it would mean the other field's.
   */
  private usedBy(path: readonly number[]): Node[] {
    if (path.length === 0) return [];
    const found = this.reverseGraph(path);
    if (found === null) return [];
    const { graph, self } = found;
    const rows: Node[] = [];
    const seen = new Set<string>();
    for (const e of graph.edges) {
      // A `points` edge is the pointer this field holds, and the section above
      // has already shown where it points. Under this heading the same fact
      // would read the other way round, as the far end using this field.
      if (e.from !== self || e.to === self || e.role === "points") continue;
      const to = graph.nodes[e.to];
      if (to === undefined) continue;
      const at = `${e.role} ${to.path.join("/")}`;
      if (seen.has(at)) continue;
      seen.add(at);
      rows.push(usedRow(e.role, to.name, to.path));
    }
    return rows;
  }

  /**
   * A graph holding the field at `path` that was walked to the end, and where
   * the field is in it. Null when neither the file nor the enclosing structure
   * could be walked whole.
   *
   * A walk that hit the cap is thrown away rather than used. What it holds is
   * whichever fields it reached first, and a list of dependents that is
   * silently a sample of them is worse than no list, because nothing on screen
   * says which it is.
   */
  private reverseGraph(path: readonly number[]): { graph: FieldGraph; self: number } | null {
    const key = path.join("/");
    for (const root of [[] as readonly number[], path.slice(0, -1)]) {
      // A structure of more fields than the cap cannot come back whole, so it
      // is not asked at all: the walk would be paid for and thrown away, on
      // every move of the cursor.
      const node = this.doc.templateNode(root);
      if (node.status === "ok" && node.node.child_count > USED_BY_LIMIT) continue;
      const reply = this.doc.graph(root, USED_BY_LIMIT);
      if (reply.status !== "ok" || reply.node.omitted > 0) continue;
      const self = reply.node.nodes.findIndex((n) => n.path.join("/") === key);
      if (self >= 0) return { graph: reply.node, self };
    }
    return null;
  }

  /** The field a block of properties is about, when it is not the field at the
   *  cursor, as a link to it. */
  private stepHead(at: readonly number[]): HTMLElement {
    const node = this.doc.templateNode(at);
    const b = document.createElement("button");
    b.type = "button";
    b.className = "insp-link insp-origin-group";
    b.dataset["path"] = at.join("/");
    b.textContent = node.status === "ok" ? node.node.name : "…";
    return b;
  }

  /**
   * Mark the row the pointer is resting on, and say which field it names so the
   * views can mark that field too. Only a row that names a field can be marked:
   * null unmarks whatever was marked and says nothing is being pointed at.
   */
  private markHover(row: HTMLElement | null): void {
    const on = row !== null && (this.origins.contains(row) || this.kids.contains(row)) ? row : null;
    if (on === this.hoverRow) return;
    this.hoverRow?.classList.remove("is-hover");
    this.hoverRow = on;
    if (on === null) {
      this.onHoverField(null);
      return;
    }
    on.classList.add("is-hover");
    const p = on.dataset["path"];
    this.onHoverField(p === undefined ? null : pathOf(p));
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

  /**
   * What goes in the box, and what goes under it.
   *
   * A structure has no value of its own, so the box used to say how many
   * children it had. For the two shapes that are one value written as several
   * fields, a length beside its string and a short row of numbers, the box
   * shows that value instead, read-only: what it is showing belongs to a child
   * with an editor of its own, which is a row in the list underneath.
   */
  private fillValue(path: readonly number[], n: TemplateNode): void {
    const key = path.join("/");
    if (key !== this.childCapFor) {
      this.childCapFor = key;
      this.childCap = CHILD_PAGE;
    }
    const want = Math.min(n.child_count, this.childCap);
    const reply = n.composite && want > 0 ? this.doc.templateChildren(path, 0, want) : null;
    const kids = reply?.status === "ok" ? reply.node : null;
    const inside = kids === null ? null : insideValue(n, kids);
    const shown = inside?.kind === "payload" ? inside.node : n;
    const long = !shown.composite && (shown.kind === "bytes" || shown.kind === "str");
    this.area.hidden = !long;
    this.field.hidden = long;
    if (long) this.fillArea(shown, n, inside);
    else this.fillField(n, inside);
    this.fillKids(n, kids, reply?.status === "pending" || reply?.status === "working");
  }

  /** A structure's children, listed. Each row leads to the child, which is
   *  where it can be read whole and edited. */
  private fillKids(n: TemplateNode, kids: readonly TemplateNode[] | null, waiting: boolean): void {
    if (!n.composite || n.child_count === 0) {
      this.kids.hidden = true;
      this.kids.replaceChildren();
      return;
    }
    // Children that would not read are not an empty list; the error is on the
    // status line, and a heading over nothing would say the structure is empty.
    if (kids === null && !waiting) {
      this.kids.hidden = true;
      this.kids.replaceChildren();
      return;
    }
    const noun = childWord(n);
    const parts: Node[] = [subhead(childrenHead(n))];
    if (kids === null) {
      const row = el("div", "insp-kid insp-kid-waiting");
      row.append(el("span", "insp-kid-value", REPORT.paneWaiting));
      parts.push(row);
    } else {
      for (const kid of kids) parts.push(kidRow(kid, this.kidValue(kid)));
      const rest = n.child_count - kids.length;
      if (rest > 0) parts.push(moreButton(rest, noun));
    }
    this.kids.replaceChildren(...parts);
    this.kids.hidden = false;
  }

  /**
   * What a child's row says it holds. A structure of its own holds a count,
   * except where that structure is one value written as several fields, which
   * is the same question the box above asks and the same answer: the row for a
   * tensor's `name` says the name, not that a name is two fields.
   *
   * Only a short structure is looked into. A row that stood for a page of a
   * database would send the reader's twelve rows after twelve pages.
   */
  private kidValue(kid: TemplateNode): { readonly text: string; readonly count: boolean } {
    if (!kid.composite) return { text: kid.value, count: false };
    if (kid.child_count > 0 && kid.child_count <= PREVIEW_ITEMS) {
      const reply = this.doc.templateChildren(kid.path, 0, kid.child_count);
      const inside = reply.status === "ok" ? insideValue(kid, reply.node) : null;
      if (inside?.kind === "row") return { text: inside.text, count: false };
      if (inside?.kind === "payload") return { text: inside.node.value, count: false };
    }
    return { text: countText(kid.child_count, childWord(kid)), count: true };
  }

  private fillField(n: TemplateNode, inside: Inside | null): void {
    const row = inside?.kind === "row" ? inside.text : null;
    this.note.textContent = "";
    this.note.hidden = true;
    if (this.field.dataset["dirty"] === "1" && document.activeElement === this.field) return;
    // Read-only rather than disabled: a value shown here is still a value to
    // select and copy, which a disabled input in most browsers is not.
    this.field.readOnly = !n.editable || row !== null;
    this.field.classList.remove("invalid");
    this.field.value = row ?? (n.composite ? "" : n.edit_text);
    this.field.placeholder = n.composite && row === null ? countText(n.child_count, childWord(n)) : "";
    this.field.setAttribute("aria-label", `${n.name}, ${n.type}`);
  }

  /**
   * The whole value, wrapped over as many lines as it takes: hex pairs for a
   * byte field, the text itself for a text field. Read from the document rather
   * than from the node, whose value is a preview once a field gets long.
   */
  private fillArea(n: TemplateNode, owner: TemplateNode, inside: Inside | null): void {
    // `n` is the field at the cursor, except where the cursor is on a length
    // and its string together, when it is the string and `owner` is the pair.
    const borrowed = inside?.kind === "payload";
    if (this.area.dataset["dirty"] === "1" && document.activeElement === this.area) return;
    this.area.classList.remove("invalid");
    this.area.setAttribute("aria-label", `${owner.name}, ${n.type}`);
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
    const editable = n.editable && !shown.truncated && !borrowed;
    this.area.value = shown.text;
    this.area.disabled = false;
    this.area.readOnly = !editable;
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

  /** A byte field is its own value: hex pairs, wrapped.
   *
   *  Asked for by path rather than read out of the file at the node's offset:
   *  a field inside a decoded stream is at an offset of that stream, and the
   *  file at the same offset is some other field's bytes. */
  private readHex(n: TemplateNode): { text: string; truncated: boolean; note: string | null } | null {
    const r = this.doc.fieldBytes(n.path, Math.min(n.value_bytes, SHOW_LIMIT));
    if (r.status === "pending" || r.status === "working") return null;
    if (r.status === "error") return { text: "", truncated: false, note: r.message };
    return { text: hexText(Uint8Array.from(r.node.bytes)), truncated: r.node.truncated, note: null };
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
const SEL_TEXT_TITLE = "As text";
const SEL_TEXT_PARTIAL = (n: number): string => `First ${n.toLocaleString()} bytes`;
/** The UTF-8 row when the bytes are all ASCII too: one fact, said once. */
const SEL_TEXT_UTF8_ASCII = "UTF-8 · ASCII";
/** In the reading's place when the bytes do not fit the row's encoding. The
 *  row stays, so every encoding is found where it always is. */
const SEL_TEXT_NOT_VALID = "Not valid";
const SEL_TEXT_PAGE_LABEL = "Codepage for this row";
const SEL_TEXT_LANG_LABEL = "Language for the string literal";
const SEL_TEXT_LITERAL_COPY = "Copy the string literal";
const SEL_TEXT_LITERAL_EXPAND = "Show the whole string literal";
const SEL_TEXT_LITERAL_COLLAPSE = "Show the string literal on one line";
const SEL_LENGTH = "Length";
const LOADING = "Loading…";
const COPY = "Copy";
const EDIT = "Edit";
const COPIED = "Copied.";
const COPY_FAILED = "Couldn't copy to the clipboard.";
const EXPAND = "Expand";
const COLLAPSE = "Collapse";
const copyLabel = (row: string): string => `Copy the ${row.toLowerCase()} value`;
const expandLabel = (row: string): string => `Show the whole ${row} reading`;
const collapseLabel = (row: string): string => `Show the ${row} reading on one line`;
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

/** A chooser dressed as the label it stands in for, so a row reads as a
 *  sentence rather than as a form. `slot` says which one it is, which is how
 *  the keyboard finds it again after the rows are rebuilt. */
function picker(options: readonly string[], value: string, label: string, slot: string, names?: Readonly<Record<string, string>>): HTMLSelectElement {
  const s = document.createElement("select");
  s.className = "insp-reading-pick";
  s.dataset["slot"] = slot;
  s.setAttribute("aria-label", label);
  s.title = label;
  for (const name of options) {
    const o = document.createElement("option");
    o.value = name;
    o.textContent = names?.[name] ?? name;
    s.append(o);
  }
  s.value = value;
  return s;
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

/** A tagged element with a class and its text, the way `listpane.ts` and
 *  `listingdraw.ts` each keep one. Three copies of four lines is two too many;
 *  they want collecting into `dom.ts`, whose own `el` takes properties. */
function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** One child of the structure at the cursor: what it is called, what it holds,
 *  and how long it is. A structure of its own holds a count, drawn as the box
 *  draws one so a count is never read as a value. */
function kidRow(kid: TemplateNode, holds: { readonly text: string; readonly count: boolean }): HTMLElement {
  const row = el("div", "insp-kid");
  row.dataset["path"] = kid.path.join("/");
  row.append(el("span", `insp-kid-name ${fieldClass(kid.kind)}`, kid.name));
  const value = el("span", `insp-kid-value${holds.count ? " insp-kid-count" : ""}`, holds.text);
  value.title = holds.text;
  row.append(value, el("span", "insp-kid-size", bitSizeText(kid.size_bits)));
  return row;
}

/** The rest of a list too long to draw at once. The glyph says the list is cut
 *  short and the number says by how much; the words for what a click delivers
 *  are on the button rather than in it, since a mark and a count read at a
 *  glance and a sentence in a 190px column does not. */
function moreButton(rest: number, noun: string): HTMLElement {
  const b = el("button", "insp-kid-more");
  b.type = "button";
  b.dataset["more"] = "";
  b.textContent = INSIDE.more(rest);
  const said = rest > CHILD_PAGE ? INSIDE.moreTitle(CHILD_PAGE, rest, noun) : INSIDE.moreRest(rest, noun);
  b.title = said;
  b.setAttribute("aria-label", said);
  return b;
}

function subhead(text: string): HTMLElement {
  const h = document.createElement("div");
  h.className = "insp-subhead";
  h.textContent = text;
  return h;
}

/**
 * One question about the field at the cursor, answered on one line.
 *
 * `value` is the answer itself: an address, a size, a type name. `how` is the
 * clause under it saying how that answer was arrived at, and names a field
 * where one field settled it. `detail` is the working, which the row folds
 * away: an empty one is a row with nothing behind it and no triangle.
 */
type Property = {
  /** What the unfolded set remembers, so a row stays open across a redraw. */
  readonly key: string;
  readonly label: string;
  readonly value: string;
  /** For an answer that is a place in the file: the bit it leads to. */
  readonly bit: number | null;
  readonly how: How | null;
  readonly detail: Node[];
};

/** How an answer was arrived at, and the field it names, when it names one. */
type How = { readonly text: string; readonly path: readonly number[] | null };

/** The one thing in a list, or nothing: a clause naming a field can only name
 *  one, and where two fields settled something the row says so without
 *  pretending either of them did it alone. */
function only<T>(list: readonly T[] | undefined): T | null {
  return list !== undefined && list.length === 1 ? (list[0] ?? null) : null;
}

/** Whether `above` is a field this one sits inside. What tells the stream a
 *  field was unpacked from apart from a field an expression read: an
 *  expression reads leaves, and never one of its own ancestors. */
function encloses(above: readonly number[], path: readonly number[]): boolean {
  return above.length < path.length && above.every((step, i) => step === path[i]);
}

/** Where the field sits inside one of the structures around it. */
function insideRow(name: string, delta: number, path: readonly number[]): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-origin";
  row.dataset["path"] = path.join("/");
  const what = document.createElement("span");
  what.className = "insp-origin-name";
  what.textContent = PROPERTIES.insideAt(name, `+${formatOffset(delta)}`);
  row.append(what);
  return row;
}

/** A clause that is only the field's name and what it says: what a count or a
 *  name was taken from needs no verb, since the label supplies it. */
function originClause(o: Origin): string {
  return o.value === "" ? o.label : `${o.label} = ${grouped(o.value)}`;
}

/** How many of the structures a field sits inside are worth an offset each. */
const INSIDE_LEVELS = 3;

/** The one block that is not a property of the field: what the structures
 *  above it settled. */
const ABOVE_KEY = "above";

/** A count of four million reads as one. */
function grouped(value: string): string {
  return /^\d{5,}$/.test(value) ? BigInt(value).toLocaleString() : value;
}

/**
 * What one field decided about another. `points` is not here because it is the
 * other direction: this field holds an offset, and that is where it leads.
 */
const ROLE_ORDER = ["position", "length", "width", "count", "type", "value", "name"] as const;
type OriginRole = (typeof ROLE_ORDER)[number];

/**
 * How many fields of a structure are searched for ones that read this field.
 *
 * A structure of more than this is a structure whose dependents are not worth
 * a wait: the panel is answering a question the reader asked by moving the
 * cursor, and it has to answer before they move it again.
 */
const USED_BY_LIMIT = 400;

/** One field that reads the field at the cursor: what it took from it, and
 *  what it is called. The role stays on the row; see `usedBy`. */
function usedRow(role: string, name: string, path: readonly number[]): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-origin";
  row.dataset["path"] = path.join("/");
  const what = document.createElement("span");
  what.className = "insp-origin-role";
  what.textContent = ROLE_GROUP[role] ?? role;
  const b = document.createElement("button");
  b.type = "button";
  b.className = "insp-link insp-origin-name";
  b.textContent = name;
  row.append(what, b);
  return row;
}

/** A `data-path` attribute read back. The empty string is the root of the
 *  template, which is a place like any other and not the absence of one. */
function pathOf(text: string): readonly number[] {
  return text === "" ? [] : text.split("/").map(Number);
}

/** Collect values under the key they belong to, in the order they arrived. */
function push<K, V>(map: Map<K, V[]>, key: K, value: V): void {
  const had = map.get(key);
  if (had === undefined) map.set(key, [value]);
  else had.push(value);
}

/** What the rows below it settled. Lighter than the section's own heading: it
 *  divides a list rather than opening one. */
function roleHead(text: string): HTMLElement {
  const h = document.createElement("div");
  h.className = "insp-role-head";
  h.textContent = text;
  return h;
}

/**
 * One field that decided something about the one at the cursor, and what that
 * field says: `len = 20`. Which of its shapes was decided is the heading above
 * the row, so the names line up down the group instead of starting after a
 * column of repeated words.
 *
 * The whole row carries the path, so pointing anywhere along it marks the field
 * it names and clicking anywhere along it goes there. An origin with no field
 * of its own carries none: there is nowhere to go, and an empty path would
 * read as the root of the file.
 */
function originRow(o: Origin): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-origin";
  let name: HTMLElement;
  if (o.path.length === 0) {
    name = document.createElement("span");
    name.className = "insp-origin-name";
  } else {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "insp-link";
    row.dataset["path"] = o.path.join("/");
    name = b;
  }
  name.textContent = o.label;
  row.append(name);
  if (o.value !== "") {
    const v = document.createElement("span");
    v.className = "insp-origin-val";
    v.textContent = `= ${grouped(o.value)}`;
    row.append(v);
  }
  return row;
}

/**
 * The relationship itself: the expression the template holds, and under it the
 * same expression with the numbers in it and what they come to. It sits under
 * the rows naming the fields it reads, so the sentence and its words are read
 * together.
 *
 * Both forms come from the core. The panel puts them one above the other so
 * the substitution reads as a substitution: same shape, same length, numbers
 * where the names were.
 */
function relationRow(r: Relation): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-relation";
  const written = document.createElement("code");
  written.className = "insp-rel-written";
  written.textContent = r.written;
  const sums = document.createElement("code");
  sums.className = "insp-rel-sums";
  sums.textContent = `${r.substituted} = ${grouped(r.result)}`;
  row.append(written, sums);
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

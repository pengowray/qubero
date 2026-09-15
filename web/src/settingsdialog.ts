// The settings dialog: the template reading the file, found by searching, and
// the hex view's settings. Every control applies as it is changed, and the
// dialog sits to one side so the view behind it shows what changed.

import { el } from "./dom.ts";
import type { RightColumn } from "./hexview.ts";
import { loadSignatures, type Compiled } from "./signatures.ts";
import { searchTemplates, spacedPattern, type Reason, type TemplateEntry } from "./templatesearch.ts";
import { KAITAI_TEMPLATE, KSY, SETTINGS } from "./strings.ts";

/** What the hex pane reads and sets. The owner keeps the state; the dialog
 *  only shows it. */
export type HexHost = {
  mode(): "hex" | "binary";
  setMode(mode: "hex" | "binary"): void;
  bytesPerRow(): number;
  setBytesPerRow(n: number): void;
  /** A row width that is one record long, when the screen holds records. */
  stride(): { readonly bytes: number; readonly label: string } | null;
  column(): RightColumn;
  setColumn(c: RightColumn): void;
};

/** A template the chooser lists beyond the built-ins and Kaitai formats: the
 *  file's own signature template, and a converted `.ksy`. */
export type ExtraTemplate = { readonly value: string; readonly label: string; readonly kind: "signature" | "ksy" };

/** How many signature-only rows a search lists before it asks to be narrowed. */
const SIGNATURE_ROWS = 50;

const segmented = (label: string, options: readonly { value: string; text: string }[], pick: (value: string) => void): HTMLElement => {
  const group = el("div", { className: "set-seg" });
  group.setAttribute("role", "group");
  group.setAttribute("aria-label", label);
  for (const o of options) {
    const b = el("button", { type: "button", textContent: o.text });
    b.dataset.value = o.value;
    b.addEventListener("click", () => pick(o.value));
    group.append(b);
  }
  return group;
};

const pressSegment = (group: HTMLElement, value: string): void => {
  for (const b of group.querySelectorAll("button")) b.setAttribute("aria-pressed", String(b.dataset.value === value));
};

export class SettingsDialog {
  readonly el: HTMLDialogElement;
  onPickTemplate: (value: string) => void = () => {};
  onConvertKsy: () => void = () => {};

  private readonly entries: TemplateEntry[];
  private extras: ExtraTemplate[] = [];
  private current = "";
  private signatures: readonly Compiled[] = [];
  private readonly search: HTMLInputElement;
  private readonly clear: HTMLButtonElement;
  private readonly list: HTMLElement;
  private readonly currentLine: HTMLElement;
  private readonly templatePane: HTMLElement;
  private readonly convert: HTMLButtonElement;
  private readonly modeGroup: HTMLElement;
  private readonly rowGroup: HTMLElement;
  private readonly textToggle: HTMLButtonElement;
  private readonly fieldsToggle: HTMLButtonElement;
  private readonly condensed: HTMLInputElement;
  private condensedWanted = false;
  private active = -1;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    entries: readonly TemplateEntry[],
    private readonly hex: HexHost,
    private readonly glyphs: HTMLSelectElement,
  ) {
    this.entries = [...entries].sort((a, b) => a.label.localeCompare(b.label));

    // ---- template ----
    this.currentLine = el("p", { className: "set-current" });
    this.search = el("input", { type: "text", className: "set-search", placeholder: SETTINGS.searchPlaceholder });
    this.search.setAttribute("aria-label", SETTINGS.searchLabel);
    const hintId = `set-hint-${Math.random().toString(36).slice(2)}`;
    this.search.setAttribute("aria-describedby", hintId);
    this.search.setAttribute("aria-controls", `${hintId}-list`);
    this.clear = el("button", { type: "button", className: "set-clear", textContent: "×", title: SETTINGS.clearSearch });
    this.clear.setAttribute("aria-label", SETTINGS.clearSearch);
    this.clear.hidden = true;
    this.clear.addEventListener("click", () => {
      this.search.value = "";
      this.render();
      this.search.focus();
    });
    this.list = el("div", { className: "set-results", id: `${hintId}-list` });
    this.list.setAttribute("role", "listbox");
    this.list.setAttribute("aria-label", SETTINGS.templateHeading);
    this.search.addEventListener("input", () => {
      if (this.timer !== null) clearTimeout(this.timer);
      this.timer = setTimeout(() => this.render(), 120);
    });
    this.search.addEventListener("keydown", (e) => this.onSearchKey(e));
    this.convert = el("button", { type: "button", className: "set-convert", textContent: KSY.menuEntry });
    this.convert.addEventListener("click", () => {
      this.el.close();
      this.onConvertKsy();
    });
    this.templatePane = el(
      "section",
      { className: "set-pane set-template" },
      el("h3", { textContent: SETTINGS.templateHeading }),
      this.currentLine,
      el("div", { className: "set-searchbox" }, this.search, this.clear),
      el("p", { className: "set-hint", id: hintId, textContent: SETTINGS.searchHint }),
      this.list,
      el("div", { className: "set-foot" }, this.convert),
    );

    // ---- hex view ----
    this.modeGroup = segmented(
      SETTINGS.bytesAs,
      [
        { value: "hex", text: SETTINGS.hex },
        { value: "binary", text: SETTINGS.binary },
      ],
      (v) => {
        this.hex.setMode(v === "binary" ? "binary" : "hex");
        this.syncHex();
      },
    );
    this.rowGroup = segmented(SETTINGS.perRow, [], () => {});
    this.textToggle = el("button", { type: "button", textContent: SETTINGS.columnText });
    this.fieldsToggle = el("button", { type: "button", textContent: SETTINGS.columnFields });
    this.condensed = el("input", { type: "checkbox" });
    const toggles = el("div", { className: "set-seg set-toggles" }, this.textToggle, this.fieldsToggle);
    toggles.setAttribute("role", "group");
    toggles.setAttribute("aria-label", SETTINGS.column);
    this.textToggle.addEventListener("click", () => this.flipColumn("text"));
    this.fieldsToggle.addEventListener("click", () => this.flipColumn("fields"));
    this.condensed.addEventListener("change", () => {
      this.condensedWanted = this.condensed.checked;
      this.applyColumn(this.textOn(), true);
    });
    const glyphLabel = el("label", { className: "set-field" }, el("span", { className: "set-label", textContent: glyphs.getAttribute("aria-label") ?? "" }), glyphs);
    const hexPane = el(
      "section",
      { className: "set-pane set-hex" },
      el("h3", { textContent: SETTINGS.hexHeading }),
      el("div", { className: "set-field" }, el("span", { className: "set-label", textContent: SETTINGS.bytesAs }), this.modeGroup),
      el("div", { className: "set-field" }, el("span", { className: "set-label", textContent: SETTINGS.perRow }), this.rowGroup),
      el(
        "div",
        { className: "set-field" },
        el("span", { className: "set-label", textContent: SETTINGS.column }),
        el("div", { className: "set-columns" }, toggles, el("label", { className: "set-check" }, this.condensed, SETTINGS.condensed)),
        el("p", { className: "set-note", textContent: SETTINGS.columnNote }),
      ),
      glyphLabel,
    );

    const closeX = el("button", { type: "button", className: "set-x", textContent: "×", title: SETTINGS.closeLabel });
    closeX.setAttribute("aria-label", SETTINGS.closeLabel);
    closeX.addEventListener("click", () => this.el.close());
    this.el = el(
      "dialog",
      { className: "dlg settings-dlg" },
      el("div", { className: "set-head" }, el("h2", { textContent: SETTINGS.title }), closeX),
      el("div", { className: "set-body" }, hexPane, this.templatePane),
      el("form", { method: "dialog", className: "dlg-close" }, el("button", { type: "submit", textContent: SETTINGS.close })),
    );
    this.el.setAttribute("aria-label", SETTINGS.title);
    // The dialog covers only part of the screen, so a click that lands on it
    // rather than on its contents is a click on the backdrop.
    this.el.addEventListener("click", (e) => {
      if (e.target === this.el) this.el.close();
    });
  }

  /** Open it, with the caret in the template search. */
  open(): void {
    this.syncHex();
    this.render();
    if (!this.el.open) this.el.showModal();
    this.search.focus();
    this.search.select();
    if (this.signatures.length === 0) {
      void loadSignatures().then((s) => {
        if (s === null) return;
        this.signatures = s.compiled;
        if (this.el.open && this.search.value.trim() !== "") this.render();
      });
    }
  }

  /** Whether the file can have its template changed at all; an unpacked
   *  stream's template came with it. */
  setTemplatesOffered(offered: boolean, canConvert: boolean): void {
    this.templatePane.hidden = !offered;
    this.convert.hidden = !canConvert;
  }

  setExtras(extras: readonly ExtraTemplate[]): void {
    this.extras = [...extras];
    if (this.el.open) this.render();
  }

  setCurrent(value: string, label: string | null): void {
    this.current = value;
    this.currentLine.textContent = label === null ? SETTINGS.currentNone : SETTINGS.current(label);
    if (this.el.open) this.render();
  }

  // ---- hex pane ----

  private textOn(): boolean {
    const c = this.hex.column();
    return c === "text" || c.startsWith("both");
  }

  private fieldsOn(): boolean {
    return this.hex.column() !== "text";
  }

  private flipColumn(which: "text" | "fields"): void {
    const text = this.textOn();
    const fields = this.fieldsOn();
    if (which === "text" && text && !fields) return;
    if (which === "fields" && fields && !text) return;
    this.applyColumn(which === "text" ? !text : text, which === "fields" ? !fields : fields);
  }

  private applyColumn(text: boolean, fields: boolean): void {
    const c: RightColumn = !fields ? "text" : `${text ? "both" : "fields"}${this.condensedWanted ? "-condensed" : ""}` as RightColumn;
    this.hex.setColumn(c);
    this.syncHex();
  }

  /** Show what the hex view is set to now. */
  syncHex(): void {
    pressSegment(this.modeGroup, this.hex.mode());
    const per = this.hex.bytesPerRow();
    const stride = this.hex.stride();
    const widths = [8, 16, 32].map((n) => ({ value: String(n), text: String(n) }));
    if (stride !== null && ![8, 16, 32].includes(stride.bytes)) widths.push({ value: String(stride.bytes), text: stride.label });
    const fresh = segmented(SETTINGS.perRow, widths, (v) => {
      this.hex.setBytesPerRow(Number(v));
      this.syncHex();
    });
    this.rowGroup.replaceChildren(...fresh.childNodes);
    pressSegment(this.rowGroup, String(per));
    const c = this.hex.column();
    const text = this.textOn();
    const fields = this.fieldsOn();
    if (fields) this.condensedWanted = c.endsWith("-condensed");
    for (const [b, on, alone] of [
      [this.textToggle, text, text && !fields],
      [this.fieldsToggle, fields, fields && !text],
    ] as const) {
      b.setAttribute("aria-pressed", String(on));
      b.setAttribute("aria-disabled", String(alone));
      b.title = alone ? SETTINGS.lastOn : "";
    }
    this.condensed.checked = fields && this.condensedWanted;
    this.condensed.disabled = !fields;
    this.glyphs.disabled = !text;
  }

  // ---- template pane ----

  private rows(): HTMLElement[] {
    return [...this.list.querySelectorAll<HTMLElement>("[role=option]")];
  }

  private onSearchKey(e: KeyboardEvent): void {
    const rows = this.rows();
    // Escape closes the dialog from the field, as it does from the rest of it;
    // the clear button is what empties the field.
    if (e.key === "Escape") {
      e.preventDefault();
      this.el.close();
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (rows.length === 0) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      this.setActive(Math.max(0, Math.min(rows.length - 1, this.active + step)));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const row = rows[this.active] ?? (rows.length === 1 ? rows[0] : undefined);
      if (row !== undefined) this.pick(row.dataset.template ?? "");
    }
  }

  private setActive(i: number): void {
    const rows = this.rows();
    rows.forEach((r, j) => r.classList.toggle("is-active", j === i));
    this.active = i;
    const row = rows[i];
    if (row === undefined) this.search.removeAttribute("aria-activedescendant");
    else {
      this.search.setAttribute("aria-activedescendant", row.id);
      row.scrollIntoView({ block: "nearest" });
    }
  }

  private pick(value: string): void {
    this.onPickTemplate(value);
  }

  private reasonText(r: Reason): string {
    if (r.by === "ext") return SETTINGS.reasonExt(r.ext);
    if (r.by === "name") return "";
    const bytes = spacedPattern(r.pattern);
    if (r.fromEnd) return SETTINGS.reasonEnd(bytes);
    return r.offset === 0 ? SETTINGS.reasonStart(bytes) : SETTINGS.reasonAt(r.offset, bytes);
  }

  private label(text: string, reason: Reason | null): HTMLElement {
    const name = el("span", { className: "set-name" });
    if (reason?.by === "name" && reason.at >= 0 && reason.length > 0) {
      name.append(
        text.slice(0, reason.at),
        el("b", { textContent: text.slice(reason.at, reason.at + reason.length) }),
        text.slice(reason.at + reason.length),
      );
    } else name.textContent = text;
    return name;
  }

  private templateRow(value: string, text: string, opts: { reason?: Reason; tag?: string; sub?: string } = {}): HTMLElement {
    const on = value === this.current;
    const row = el("div", { className: "set-row", id: `set-row-${Math.random().toString(36).slice(2)}` });
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", String(on));
    row.dataset.template = value;
    const main = el("div", { className: "set-rowmain" }, el("span", { className: "set-tick", textContent: on ? "✓" : "" }), this.label(text, opts.reason ?? null));
    if (on) main.append(el("span", { className: "set-tag", textContent: SETTINGS.currentTag }));
    if (opts.tag !== undefined) main.append(el("span", { className: "set-tag", textContent: opts.tag }));
    const why = opts.reason === undefined ? "" : this.reasonText(opts.reason);
    main.append(el("span", { className: "set-why", textContent: why }));
    row.append(main);
    if (opts.sub !== undefined) row.append(el("div", { className: "set-sub", textContent: opts.sub }));
    row.addEventListener("click", () => this.pick(value));
    return row;
  }

  private render(): void {
    const q = this.search.value.trim();
    this.clear.hidden = q === "";
    this.list.replaceChildren();
    this.active = -1;
    const extraRow = (x: ExtraTemplate, reason?: Reason): HTMLElement =>
      x.kind === "signature"
        ? this.templateRow(x.value, x.label, { ...(reason ? { reason } : {}), sub: SETTINGS.fileSignatureSub })
        : this.templateRow(x.value, x.label, { ...(reason ? { reason } : {}), tag: SETTINGS.ksyTag });
    if (q === "") {
      this.list.append(this.templateRow("", SETTINGS.noTemplate));
      for (const x of this.extras.filter((x) => x.kind === "signature")) this.list.append(extraRow(x));
      for (const e of this.entries.filter((e) => e.kind === "builtin")) this.list.append(this.templateRow(e.value, e.label));
      const kaitai = this.entries.filter((e) => e.kind === "kaitai");
      if (kaitai.length > 0) {
        this.list.append(el("div", { className: "set-group", textContent: KAITAI_TEMPLATE.group }));
        for (const e of kaitai) this.list.append(this.templateRow(e.value, e.label));
      }
      for (const x of this.extras.filter((x) => x.kind === "ksy")) this.list.append(extraRow(x));
      this.setActive(this.rows().findIndex((r) => r.dataset.template === this.current));
      return;
    }
    const extraEntries: TemplateEntry[] = this.extras.map((x) => ({ value: x.value, label: x.label, kind: "extra", ext: [], sigs: [] }));
    const found = searchTemplates(q, [...this.entries, ...extraEntries], this.signatures);
    if (found.templates.length === 0 && found.signatures.length === 0) {
      this.list.append(el("p", { className: "set-empty", textContent: SETTINGS.noMatches(q) }));
      return;
    }
    this.list.append(el("div", { className: "set-group", textContent: SETTINGS.templates(found.templates.length) }));
    if (found.templates.length === 0) this.list.append(el("p", { className: "set-empty", textContent: SETTINGS.noTemplateMatches(q) }));
    for (const h of found.templates) {
      const x = this.extras.find((x) => x.value === h.entry.value);
      if (x !== undefined) this.list.append(extraRow(x, h.reason));
      else this.list.append(this.templateRow(h.entry.value, h.entry.label, { reason: h.reason, ...(h.entry.kind === "kaitai" ? { tag: SETTINGS.kaitaiTag } : {}) }));
    }
    if (found.signatures.length > 0) {
      this.list.append(el("div", { className: "set-group", textContent: SETTINGS.signatures(found.signatures.length) }));
      for (const h of found.signatures.slice(0, SIGNATURE_ROWS)) {
        const main = el("div", { className: "set-rowmain" }, el("span", { className: "set-tick" }), this.label(h.format.label, h.reason), el("span", { className: "set-why", textContent: this.reasonText(h.reason) }));
        this.list.append(
          el(
            "div",
            { className: "set-row set-sigonly" },
            main,
            el("div", { className: "set-sub", textContent: h.format.source === "wikidata" ? SETTINGS.fromWikidata : SETTINGS.fromMagic }),
          ),
        );
      }
      if (found.signatures.length > SIGNATURE_ROWS) {
        this.list.append(el("p", { className: "set-empty", textContent: SETTINGS.more(SIGNATURE_ROWS, found.signatures.length) }));
      }
    }
    if (found.templates.length > 0) this.setActive(0);
  }
}

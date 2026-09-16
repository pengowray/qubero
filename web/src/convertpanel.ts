// The shared half of the two format-description converters: the `.ksy` one in
// `ksypanel.ts` and the `.hexpat` one in `hexpatpanel.ts`.
//
// Both are the same tool. Three columns, left to right: the description as
// text, what converting it could not carry across, and the template it
// produced. Both convert on a scratch basis as the text is typed, so the
// document keeps whatever template it had until "Use this template" is pressed.
// Both offer the shipped descriptions, take a file from disk or a drop, and
// send the caret to the line a report line came from.
//
// What differs is the language: which entry of the core does the converting,
// how a report path finds its line, and what each panel offers beside the text
// box. Those are the abstract members below; everything else is here once.

import { el } from "./dom.ts";

/** One line of a conversion report: where in the description it is, the text
 *  there, and what became of it. */
export type PanelLine = {
  readonly path: string;
  readonly source: string;
  readonly message: string;
};

/** What a conversion had to say. `name` is what the template goes by once it
 *  is in use. */
export type PanelReport = {
  readonly name: string;
  readonly fields: readonly PanelLine[];
  readonly gaps: readonly PanelLine[];
  readonly notes: readonly PanelLine[];
};

/** The last conversion, or what was wrong with the text. */
export type PanelResult =
  | { readonly ok: true; readonly report: PanelReport; readonly text: string }
  | { readonly ok: false; readonly message: string };

/** Every string the shared half shows. Each panel passes its own set, so the
 *  two can differ in the words while sharing the layout. */
export type PanelStrings = {
  readonly regionLabel: string;
  readonly openButton: string;
  readonly openTitle: string;
  readonly placeholder: string;
  readonly sourceLabel: string;
  readonly reportLabel: string;
  readonly templateLabel: string;
  readonly templateEmpty: string;
  readonly templateFailed: string;
  readonly gapsHeading: (n: number) => string;
  readonly gapsTitle: string;
  readonly notesHeading: (n: number) => string;
  readonly notesTitle: string;
  readonly fieldsHeading: (n: number) => string;
  readonly lineTitle: string;
  readonly failedHeading: string;
  readonly apply: string;
  readonly applyTitle: string;
  readonly close: string;
  readonly closeTitle: string;
};

/** How long after the last keystroke the text is converted. Long enough that
 *  typing a type name does not convert it five times, short enough that a
 *  pause reads as an answer to what was typed. */
const DEBOUNCE_MS = 300;

export abstract class ConvertPanel {
  readonly el: HTMLElement;
  /**
   * Called when the reader applies the template, with the name it goes by.
   *
   * `bundled` is true when the text is a shipped description the reader has not
   * changed: the file is then read as that bundled format, so the chooser goes
   * on naming it. The page closes the panel, goes back to the listing and
   * updates the menu either way.
   */
  onApply: ((id: string, bundled: boolean) => void) | null = null;
  /** Called when the reader closes the panel without applying anything. */
  onClose: (() => void) | null = null;
  /** Where a message about the document itself goes: the status slot the rest
   *  of the page writes to. */
  onMessage: ((text: string, warn?: boolean) => void) | null = null;

  protected readonly strings: PanelStrings;
  protected readonly source: HTMLTextAreaElement;
  /** The name of the file in the box, where one came from a file. */
  protected readonly fileName: HTMLElement;
  /** The line above the text box. A panel appends its own controls here. */
  protected readonly bar: HTMLElement;
  /** The left column, for anything a panel shows under the text box. */
  protected readonly sourceColumn: HTMLElement;
  protected readonly reportColumn: HTMLElement;
  protected last: PanelResult | null = null;

  private readonly report: HTMLElement;
  private readonly out: HTMLElement;
  private readonly applyBtn: HTMLButtonElement;
  private readonly picker: HTMLInputElement;
  private readonly tab: string;
  private timer: number | null = null;

  /**
   * `accept` is the file picker's extension list, `tab` what the Tab key
   * inserts, and `rootClass` a second class on the panel so a test or a rule
   * can tell the two apart.
   */
  constructor(strings: PanelStrings, accept: string, tab: string, rootClass: string) {
    this.strings = strings;
    this.tab = tab;

    this.picker = el("input", { type: "file", accept, hidden: true });
    this.picker.addEventListener("change", () => {
      const file = this.picker.files?.[0];
      if (file === undefined) return;
      void file.text().then((text) => this.load(text, file.name));
    });
    const openBtn = el("button", { type: "button", className: "kp-open", textContent: strings.openButton, title: strings.openTitle });
    openBtn.addEventListener("click", () => this.picker.click());
    this.fileName = el("span", { className: "kp-name" });
    this.bar = el("div", { className: "kp-bar" }, openBtn);

    this.source = el("textarea", { className: "kp-source", spellcheck: false, placeholder: strings.placeholder });
    this.source.setAttribute("wrap", "off");
    this.source.setAttribute("aria-label", strings.sourceLabel);
    this.source.addEventListener("input", () => this.edited());
    this.source.addEventListener("keydown", (e) => this.onKeyInText(e));

    this.report = el("div", { className: "kp-report" });
    this.report.setAttribute("aria-label", strings.reportLabel);
    this.out = el("pre", { className: "kp-out" });
    this.out.setAttribute("aria-label", strings.templateLabel);

    this.applyBtn = el("button", { type: "button", className: "primary", textContent: strings.apply, title: strings.applyTitle });
    this.applyBtn.disabled = true;
    this.applyBtn.addEventListener("click", () => this.apply());
    const closeBtn = el("button", { type: "button", textContent: strings.close, title: strings.closeTitle });
    closeBtn.addEventListener("click", () => this.onClose?.());

    this.sourceColumn = el("div", { className: "kp-col kp-col-source" }, this.bar, this.source, this.picker);
    this.reportColumn = el(
      "div",
      { className: "kp-col kp-col-report" },
      el("div", { className: "kp-bar" }, el("span", { className: "kp-heading", textContent: strings.reportLabel })),
      this.report,
    );
    this.el = el(
      "section",
      { className: `kp ${rootClass}`, tabIndex: -1 },
      el(
        "div",
        { className: "kp-cols" },
        this.sourceColumn,
        this.reportColumn,
        el(
          "div",
          { className: "kp-col kp-col-out" },
          el("div", { className: "kp-bar" }, el("span", { className: "kp-heading", textContent: strings.templateLabel })),
          this.out,
        ),
      ),
      el("div", { className: "kp-foot" }, this.applyBtn, closeBtn),
    );
    this.el.setAttribute("role", "region");
    this.el.setAttribute("aria-label", strings.regionLabel);
  }

  // ----- what each language fills in -----

  /** Convert the text and change nothing else. */
  protected abstract preview(text: string): PanelResult;

  /** Read the open file with the template the text converted to. Called only
   *  when the last conversion succeeded. */
  protected abstract applyConverted(): void;

  /** Which line of the text a report path points at, or null when the path
   *  cannot be placed. */
  protected abstract lineOfPath(text: string, path: string): number | null;

  /** Anything a panel shows above the report's own groups. Empty by default. */
  protected reportPrelude(): readonly HTMLElement[] {
    return [];
  }

  /** Called after each conversion, before the columns are redrawn. */
  protected converted(): void {}

  /** Called when the text in the box changed under the reader's hands. */
  protected textEdited(): void {}

  // ----- the text in the box -----

  /** The text in the box, so the page can keep it while the panel is closed. */
  get text(): string {
    return this.source.value;
  }

  /** Put a description in the box and convert it at once: a file picked here,
   *  one dropped on the window, or the text the panel was left with. */
  load(text: string, name: string | null): void {
    this.put(text, name);
  }

  protected put(text: string, name: string | null): void {
    this.source.value = text;
    this.fileName.textContent = name ?? "";
    this.fileName.title = name ?? "";
    // At the top of the file it just opened, not wherever the last one was
    // left.
    this.source.setSelectionRange(0, 0);
    this.source.scrollTop = 0;
    this.convert();
  }

  /** Called when the panel is shown. The text box is where the reader acts. */
  focus(): void {
    this.source.focus();
  }

  /** Escape and Ctrl+Enter, wherever the focus is inside the panel. Returns
   *  true when the key was the panel's, so the page can stop it there. */
  handleKey(e: KeyboardEvent): boolean {
    if (e.key === "Escape") {
      this.onClose?.();
      return true;
    }
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      this.apply();
      return true;
    }
    return false;
  }

  /** Stop the pending conversion. The page calls this when the tab closes. */
  dispose(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  // ----- converting -----

  private edited(): void {
    this.textEdited();
    this.schedule();
  }

  /** Convert again shortly. A panel calls this when something other than a
   *  keystroke changed the answer, such as an include file being pasted in. */
  protected schedule(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      this.timer = null;
      this.convert();
    }, DEBOUNCE_MS);
  }

  /**
   * Convert what is in the box, and change nothing else.
   *
   * On a scratch basis: the document is still read with whatever template it
   * had, so a reader can type through a broken half of a description without
   * the file going out from under them.
   */
  protected convert(): void {
    const text = this.source.value;
    if (text.trim() === "") {
      this.last = null;
      this.converted();
      this.draw();
      return;
    }
    this.last = this.preview(text);
    this.converted();
    this.draw();
  }

  /** Read the open file with the template the text converted to. Nothing to do
   *  while the text does not convert; the button says so by being disabled. */
  apply(): void {
    const last = this.last;
    if (last === null || !last.ok) return;
    this.applyConverted();
  }

  // ----- drawing -----

  protected draw(): void {
    const last = this.last;
    const s = this.strings;
    this.applyBtn.disabled = last === null || !last.ok;
    if (last === null) {
      this.report.replaceChildren(...this.reportPrelude());
      this.out.replaceChildren(el("p", { className: "kp-empty", textContent: s.templateEmpty }));
      return;
    }
    if (!last.ok) {
      this.report.replaceChildren(
        ...this.reportPrelude(),
        el("p", { className: "kp-failed-heading", textContent: s.failedHeading }),
        errorLine(last.message),
      );
      this.out.replaceChildren(el("p", { className: "kp-empty", textContent: s.templateFailed }));
      return;
    }
    const r = last.report;
    this.report.replaceChildren(
      ...this.reportPrelude(),
      this.group(s.gapsHeading(r.gaps.length), s.gapsTitle, r.gaps, "kp-gaps", r.gaps.length > 0),
      this.group(s.notesHeading(r.notes.length), s.notesTitle, r.notes, "kp-notes", r.notes.length > 0),
      this.group(s.fieldsHeading(r.fields.length), null, r.fields, "kp-fields", false),
    );
    this.out.replaceChildren(last.text);
  }

  /** One group of the report: a heading with its count, and the lines under
   *  it. Open when there is news in it; the field list is folded away because
   *  it says something about every field there is. */
  private group(heading: string, title: string | null, lines: readonly PanelLine[], className: string, open: boolean): HTMLElement {
    // A heading with nothing under it is not something to open, so it is not
    // offered as one. Its count is still the news: nothing was left behind.
    if (lines.length === 0) {
      // Both classes land on the one element here, so the stylesheet's
      // `.kp-gaps > .kp-group-heading` does not reach it: an empty group is
      // grey, and the colour is kept for a count of something.
      const empty = el("p", { className: `kp-group kp-group-heading is-empty ${className}`, textContent: heading });
      if (title !== null) empty.title = title;
      return empty;
    }
    const summary = el("summary", { className: "kp-group-heading" }, heading);
    if (title !== null) summary.title = title;
    const group = el("details", { className: `kp-group ${className}`, open });
    group.append(summary);
    for (const line of lines) group.append(this.line(line));
    return group;
  }

  /** One line of the report: where in the description it is, and what became of
   *  it. A button, because clicking it goes somewhere. */
  private line(line: PanelLine): HTMLElement {
    const title = this.strings.lineTitle;
    const button = el(
      "button",
      { type: "button", className: "kp-line", title: line.source === "" ? title : `${line.source}\n\n${title}` },
      el("span", { className: "kp-line-path", textContent: line.path }),
      el("span", { className: "kp-line-message", textContent: line.message }),
    );
    button.addEventListener("click", () => this.goToPath(line.path));
    return button;
  }

  /** Put the caret on the line a report line came from. A path the panel
   *  cannot place leaves the caret where it was. */
  private goToPath(path: string): void {
    const line = this.lineOfPath(this.source.value, path);
    if (line === null) return;
    const lines = this.source.value.split("\n");
    let at = 0;
    for (let i = 0; i < line; i++) at += (lines[i]?.length ?? 0) + 1;
    this.source.focus();
    this.source.setSelectionRange(at, at + (lines[line]?.length ?? 0));
    // Selecting does not scroll on its own in every browser. Putting the line
    // a third of the way down is what a jump to a place in a file does
    // elsewhere in the app: enough above it to see where it sits.
    const rowHeight = this.source.scrollHeight / Math.max(1, lines.length);
    this.source.scrollTop = Math.max(0, (line - 4) * rowHeight);
  }

  /** Tab indents rather than leaving the box, since a description is all
   *  indentation. Escape is how a keyboard gets out, which is also what closes
   *  the panel. */
  private onKeyInText(e: KeyboardEvent): void {
    if (e.key !== "Tab" || e.altKey || e.ctrlKey || e.metaKey) return;
    e.preventDefault();
    if (e.shiftKey) return;
    // `execCommand` rather than `setRangeText`, which empties the browser's own
    // undo stack for the box.
    document.execCommand("insertText", false, this.tab);
  }
}

/** The one line that replaces the report when nothing converted: where in the
 *  description it went wrong, then what was wrong there. The core writes the
 *  two as `where: message`, which is the order they are read in. */
export function errorLine(message: string): HTMLElement {
  const cut = message.indexOf(": ");
  const path = cut < 1 ? "" : message.slice(0, cut);
  const rest = cut < 1 ? message : message.slice(cut + 2);
  const line = el("p", { className: "kp-error" });
  // `/` is the whole file rather than a place in it, and a line holding one
  // slash reads as a stray character. What the message says about where it is
  // stands on its own.
  if (path !== "" && path !== "/") line.append(el("span", { className: "kp-line-path", textContent: path }));
  line.append(el("span", { className: "kp-line-message", textContent: rest }));
  return line;
}

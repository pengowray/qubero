// The `.ksy` converter: Kaitai Struct text on the left, what it could not
// carry across in the middle, the template it produced on the right.
//
// It takes the main pane the way a view does, but it is a tool and not a way of
// reading the file: nothing here is about the bytes on screen. Converting is
// free of consequence until "Use this template" is pressed, so every keystroke
// reconverts on a scratch basis and the document keeps whatever template it had
// until the reader says otherwise.
//
// The middle column is why the panel exists. A template cannot say everything a
// `.ksy` can, and the honest answer to that is a list of what was left behind,
// with the line of the `.ksy` each came from a click away.

import type { Doc, KsyLine, KsyReport } from "./doc.ts";
import { el } from "./dom.ts";
import { KSY } from "./strings.ts";

/** How long after the last keystroke the text is converted. Long enough that
 *  typing a type name does not convert it five times, short enough that a
 *  pause reads as an answer to what was typed. */
const DEBOUNCE_MS = 300;

/** What one tab inserts. The Kaitai formats are written two-space, and a real
 *  tab is invalid YAML indentation. */
const TAB = "  ";

/** What the last conversion produced, or what was wrong with the text. */
type Converted =
  | { readonly ok: true; readonly report: KsyReport; readonly text: string }
  | { readonly ok: false; readonly message: string };

export class KsyPanel {
  readonly el: HTMLElement;
  /** Called when the reader applies the template, with the format's id. The
   *  page closes the panel, goes back to the listing and updates the menu. */
  onApply: ((id: string) => void) | null = null;
  /** Called when the reader closes the panel without applying anything. */
  onClose: (() => void) | null = null;
  /** Where a message about the document itself goes: the status slot the rest
   *  of the page writes to. */
  onMessage: ((text: string, warn?: boolean) => void) | null = null;

  private readonly doc: Doc;
  private readonly source: HTMLTextAreaElement;
  private readonly fileName: HTMLElement;
  private readonly report: HTMLElement;
  private readonly out: HTMLElement;
  private readonly applyBtn: HTMLButtonElement;
  private readonly picker: HTMLInputElement;
  private timer: number | null = null;
  private last: Converted | null = null;

  constructor(doc: Doc) {
    this.doc = doc;

    this.picker = el("input", { type: "file", accept: ".ksy,.yaml,.yml", hidden: true });
    this.picker.addEventListener("change", () => {
      const file = this.picker.files?.[0];
      if (file === undefined) return;
      void file.text().then((text) => this.load(text, file.name));
    });
    const openBtn = el("button", { type: "button", className: "kp-open", textContent: KSY.openButton, title: KSY.openTitle });
    openBtn.addEventListener("click", () => this.picker.click());
    this.fileName = el("span", { className: "kp-name" });

    this.source = el("textarea", { className: "kp-source", spellcheck: false, placeholder: KSY.placeholder });
    this.source.setAttribute("wrap", "off");
    this.source.setAttribute("aria-label", KSY.sourceLabel);
    this.source.addEventListener("input", () => this.schedule());
    this.source.addEventListener("keydown", (e) => this.onKeyInText(e));

    this.report = el("div", { className: "kp-report" });
    this.report.setAttribute("aria-label", KSY.reportLabel);
    this.out = el("pre", { className: "kp-out" });
    this.out.setAttribute("aria-label", KSY.templateLabel);

    this.applyBtn = el("button", { type: "button", className: "primary", textContent: KSY.apply, title: KSY.applyTitle });
    this.applyBtn.disabled = true;
    this.applyBtn.addEventListener("click", () => this.apply());
    const closeBtn = el("button", { type: "button", textContent: KSY.close, title: KSY.closeTitle });
    closeBtn.addEventListener("click", () => this.onClose?.());

    this.el = el(
      "section",
      { className: "kp", tabIndex: -1 },
      el(
        "div",
        { className: "kp-cols" },
        el(
          "div",
          { className: "kp-col kp-col-source" },
          el("div", { className: "kp-bar" }, openBtn, this.fileName, this.picker),
          this.source,
        ),
        el(
          "div",
          { className: "kp-col kp-col-report" },
          el("div", { className: "kp-bar" }, el("span", { className: "kp-heading", textContent: KSY.reportLabel })),
          this.report,
        ),
        el(
          "div",
          { className: "kp-col kp-col-out" },
          el("div", { className: "kp-bar" }, el("span", { className: "kp-heading", textContent: KSY.templateLabel })),
          this.out,
        ),
      ),
      el("div", { className: "kp-foot" }, this.applyBtn, closeBtn),
    );
    this.el.setAttribute("role", "region");
    this.el.setAttribute("aria-label", KSY.regionLabel);
    this.draw();
  }

  /** The text in the box, so the page can keep it while the panel is closed. */
  get text(): string {
    return this.source.value;
  }

  /** Put a `.ksy` in the box and convert it at once: a file picked here, one
   *  dropped on the window, or the text the panel was left with. */
  load(text: string, name: string | null): void {
    this.source.value = text;
    this.fileName.textContent = name ?? "";
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

  private schedule(): void {
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
   * had, so a reader can type through a broken half of a `.ksy` without the
   * file going out from under them.
   */
  private convert(): void {
    const text = this.source.value;
    if (text.trim() === "") {
      this.last = null;
      this.draw();
      return;
    }
    // TODO: pass the bundled formats as imports once `ksy::bundled` exists.
    // Until then a `.ksy` whose `meta/imports` names another is a gap, which
    // the report says in its own words.
    const reply = this.doc.previewKsyTemplate(text, {});
    this.last = reply.status === "ok" ? { ok: true, report: reply.report, text: reply.text } : { ok: false, message: reply.message };
    this.draw();
  }

  /** Read the open file with the template the text converted to. Nothing to do
   *  while the text does not convert; the button says so by being disabled. */
  apply(): void {
    const last = this.last;
    if (last === null || !last.ok) return;
    try {
      const report = this.doc.setKsyTemplate(this.source.value, {});
      this.onMessage?.(KSY.applied(report.name));
      this.onApply?.(report.name);
    } catch (e) {
      this.onMessage?.(e instanceof Error ? e.message : String(e), true);
    }
  }

  // ----- drawing -----

  private draw(): void {
    const last = this.last;
    this.applyBtn.disabled = last === null || !last.ok;
    if (last === null) {
      this.report.replaceChildren();
      this.out.replaceChildren(el("p", { className: "kp-empty", textContent: KSY.templateEmpty }));
      return;
    }
    if (!last.ok) {
      this.report.replaceChildren(
        el("p", { className: "kp-failed-heading", textContent: KSY.failedHeading }),
        errorLine(last.message),
      );
      this.out.replaceChildren(el("p", { className: "kp-empty", textContent: KSY.templateFailed }));
      return;
    }
    const r = last.report;
    this.report.replaceChildren(
      this.group(KSY.gapsHeading(r.gaps.length), KSY.gapsTitle, r.gaps, "kp-gaps", r.gaps.length > 0),
      this.group(KSY.notesHeading(r.notes.length), KSY.notesTitle, r.notes, "kp-notes", r.notes.length > 0),
      this.group(KSY.fieldsHeading(r.fields.length), null, r.fields, "kp-fields", false),
    );
    this.out.replaceChildren(last.text);
  }

  /** One group of the report: a heading with its count, and the lines under
   *  it. Open when there is news in it; the field list is folded away because
   *  it says something about every field there is. */
  private group(heading: string, title: string | null, lines: readonly KsyLine[], className: string, open: boolean): HTMLElement {
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

  /** One line of the report: where in the `.ksy` it is, and what became of it.
   *  A button, because clicking it goes somewhere. */
  private line(line: KsyLine): HTMLElement {
    const button = el(
      "button",
      { type: "button", className: "kp-line", title: line.source === "" ? KSY.lineTitle : `${line.source}\n\n${KSY.lineTitle}` },
      el("span", { className: "kp-line-path", textContent: line.path }),
      el("span", { className: "kp-line-message", textContent: line.message }),
    );
    button.addEventListener("click", () => this.goToPath(line.path));
    return button;
  }

  /** Put the caret on the line of the `.ksy` a report line came from. A path
   *  the scan cannot place leaves the caret where it was. */
  private goToPath(path: string): void {
    const line = lineOfKsyPath(this.source.value, path);
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

  /** Tab indents rather than leaving the box, since a `.ksy` is all
   *  indentation. Escape is how a keyboard gets out, which is also what closes
   *  the panel. */
  private onKeyInText(e: KeyboardEvent): void {
    if (e.key !== "Tab" || e.altKey || e.ctrlKey || e.metaKey) return;
    e.preventDefault();
    if (e.shiftKey) return;
    // `execCommand` rather than `setRangeText`, which empties the browser's own
    // undo stack for the box.
    document.execCommand("insertText", false, TAB);
  }
}

/** The one line that replaces the report when nothing converted: the path in
 *  the `.ksy` first, then what was wrong there. The core writes the two as
 *  `path: message`, which is the order they are read in. */
function errorLine(message: string): HTMLElement {
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

/** A line of the `.ksy`, as far as the scan cares: what it is indented to, and
 *  whether it opens a sequence entry or names a key. */
type Structural = { readonly indent: number; readonly dash: boolean; readonly key: string | null };

const KEY_LINE = /^(\s*)(-\s+)?(-?[A-Za-z0-9_][A-Za-z0-9_\-.]*)\s*:(\s|$)/;
const DASH_LINE = /^(\s*)-(\s|$)/;

function structural(line: string): Structural | null {
  if (line.trim() === "" || line.trimStart().startsWith("#")) return null;
  const dash = DASH_LINE.exec(line);
  const key = KEY_LINE.exec(line);
  if (key !== null) {
    const indent = (key[1] ?? "").length + (key[2] ?? "").length;
    return { indent, dash: dash !== null, key: key[3] ?? null };
  }
  const indent = line.length - line.trimStart().length;
  return { indent, dash: dash !== null, key: null };
}

/**
 * Which line of a `.ksy` a report path points at.
 *
 * The report writes paths the way the Kaitai compiler does:
 * `/types/chunk/seq/2/size` is the `size` of the third entry of the `seq` of
 * the type `chunk`. Walking that needs no YAML parser, only the indentation:
 * a key's value is the lines indented past it, and a sequence entry is a `-`
 * at the key's own depth or deeper. Good enough for a well-formed file, which
 * is the only kind that converts; anything it cannot place returns the deepest
 * line it did place, or null when that is nothing at all.
 */
export function lineOfKsyPath(text: string, path: string): number | null {
  const lines = text.split(/\r?\n/);
  const segments = path.split("/").filter((s) => s !== "");
  let from = 0;
  let to = lines.length;
  let found: number | null = null;
  for (const segment of segments) {
    const step = /^\d+$/.test(segment)
      ? nthEntry(lines, from, to, Number(segment))
      : keyLine(lines, from, to, segment);
    if (step === null) return found;
    found = step.line;
    from = step.from;
    to = step.to;
  }
  return found;
}

/** The column the keys of a range are written in. A key on a `- ` line counts
 *  as being where it is written, two columns in from the dash, so an entry's
 *  first key and the keys under it come out at the same depth. */
function keyIndent(lines: readonly string[], from: number, to: number): number | null {
  let base: number | null = null;
  for (let i = from; i < to; i++) {
    const s = structural(lines[i] ?? "");
    if (s === null || s.key === null) continue;
    if (base === null || s.indent < base) base = s.indent;
  }
  return base;
}

/** The column a range's own lines start in, dashes included. */
function baseIndent(lines: readonly string[], from: number, to: number): number | null {
  let base: number | null = null;
  for (let i = from; i < to; i++) {
    const line = lines[i] ?? "";
    if (structural(line) === null) continue;
    const indent = line.length - line.trimStart().length;
    if (base === null || indent < base) base = indent;
  }
  return base;
}

/** The `n`th `-` entry of a sequence, and the lines that belong to it. */
function nthEntry(lines: readonly string[], from: number, to: number, n: number): { line: number; from: number; to: number } | null {
  const base = baseIndent(lines, from, to);
  if (base === null) return null;
  const starts: number[] = [];
  for (let i = from; i < to; i++) {
    const line = lines[i] ?? "";
    if (structural(line) === null) continue;
    const indent = line.length - line.trimStart().length;
    if (indent === base && DASH_LINE.test(line)) starts.push(i);
  }
  const start = starts[n];
  if (start === undefined) return null;
  return { line: start, from: start, to: starts[n + 1] ?? to };
}

/** Where a key is written, and the lines that are its value. */
function keyLine(lines: readonly string[], from: number, to: number, key: string): { line: number; from: number; to: number } | null {
  const base = keyIndent(lines, from, to);
  if (base === null) return null;
  for (let i = from; i < to; i++) {
    const s = structural(lines[i] ?? "");
    if (s === null || s.key !== key || s.indent !== base) continue;
    const valueFrom = i + 1;
    let valueTo = to;
    for (let j = valueFrom; j < to; j++) {
      const next = structural(lines[j] ?? "");
      if (next === null) continue;
      const indent = (lines[j] ?? "").length - (lines[j] ?? "").trimStart().length;
      // A sibling key ends the value. A `-` at the same depth does not: a
      // sequence may be written under its key without being indented.
      if (indent < s.indent || (indent === s.indent && next.key !== null && !next.dash)) {
        valueTo = j;
        break;
      }
    }
    return { line: i, from: valueFrom, to: valueTo };
  }
  return null;
}

// The `.ksy` converter: Kaitai Struct text on the left, what it could not
// carry across in the middle, the template it produced on the right.
//
// The layout, the debounced conversion, the report and the bottom bar are in
// `convertpanel.ts`, which the ImHex pattern converter shares. What is here is
// what is Kaitai about it: the entries of the core it calls, the list of
// shipped descriptions, and finding the line of a `.ksy` a report path names.

import { ConvertPanel, type PanelResult } from "./convertpanel.ts";
import type { Doc } from "./doc.ts";
import { el } from "./dom.ts";
import { KSY } from "./strings.ts";

/** What a bundled format's template name starts with: `ksy:png` is the format
 *  whose `.ksy` id is `png`. */
const KSY_PREFIX = "ksy:";

/** What one tab inserts. The Kaitai formats are written two-space, and a real
 *  tab is invalid YAML indentation. */
const TAB = "  ";

export class KsyPanel extends ConvertPanel {
  private readonly doc: Doc;
  private readonly bundledList: HTMLSelectElement;
  /** The shipped description in the box and the text it arrived as, or null
   *  for text from anywhere else. Applying it reads the file as that bundled
   *  format rather than as a one-off; an edit to a single character makes it a
   *  one-off again, which is why the text it came as is kept. */
  private bundled: { readonly id: string; readonly text: string } | null = null;

  constructor(doc: Doc) {
    super(KSY, ".ksy,.yaml,.yml", TAB, "kp-ksy");
    this.doc = doc;

    // The descriptions that ship with Qubero, to read or to start from. The
    // ones that are here only to be imported are not in the chooser's list and
    // are not in this one either.
    this.bundledList = el("select", { className: "kp-bundled", title: KSY.bundledTitle });
    this.bundledList.setAttribute("aria-label", KSY.bundledLabel);
    this.bundledList.append(el("option", { value: "", textContent: KSY.bundledPlaceholder }));
    for (const c of doc.templateChoices) {
      if (c.source !== "kaitai") continue;
      const id = c.name.startsWith(KSY_PREFIX) ? c.name.slice(KSY_PREFIX.length) : c.name;
      this.bundledList.append(el("option", { value: id, textContent: KSY.bundledOption(id, c.title) }));
    }
    this.bundledList.addEventListener("change", () => {
      const id = this.bundledList.value;
      if (id === "") return;
      const text = this.doc.bundledKsyText(id);
      if (text === "") return;
      this.loadBundled(id, text);
    });
    this.bar.append(this.bundledList, this.fileName);
    this.draw();
  }

  /** Put a `.ksy` in the box and convert it at once. */
  override load(text: string, name: string | null): void {
    this.bundled = null;
    this.bundledList.value = "";
    this.put(text, name);
  }

  /**
   * Put a shipped description in the box, and remember that it is one.
   *
   * Applying it unedited reads the file as `ksy:<id>`, the same template the
   * chooser offers, rather than as a one-off conversion of this text. Editing a
   * character makes it a one-off again.
   */
  loadBundled(id: string, text: string): void {
    this.bundled = { id, text };
    this.bundledList.value = id;
    // The list is the one place the shipped description is named, so the name
    // beside it stays empty rather than saying it twice in a toolbar that has
    // room for one. A description the list does not offer, which is one that
    // exists only to be imported, is named there instead.
    this.put(text, this.bundledList.value === id ? null : KSY.bundledName(id));
  }

  /** A keystroke in the box. Text that no longer matches the shipped
   *  description is the reader's own, and the list stops claiming otherwise. */
  protected override textEdited(): void {
    if (this.bundled !== null && this.bundled.text !== this.text) {
      this.bundled = null;
      this.bundledList.value = "";
    }
  }

  protected override preview(text: string): PanelResult {
    // No imports of the reader's own: a `meta/imports` name is resolved against
    // the shipped collection by the wasm layer, which is where `common/riff`
    // and the rest of what the Kaitai library leans on already are.
    const reply = this.doc.previewKsyTemplate(text, {});
    return reply.status === "ok" ? { ok: true, report: reply.report, text: reply.text } : { ok: false, message: reply.message };
  }

  protected override applyConverted(): void {
    const shipped = this.bundled;
    // A shipped description nobody has touched is the format the chooser
    // already offers, so it is applied by name: the menu goes on showing the
    // format's title, and the page does not gain a second entry for the same
    // thing. The page says so once it has done it.
    if (shipped !== null && shipped.text === this.text) {
      this.onApply?.(shipped.id, true);
      return;
    }
    try {
      const report = this.doc.setKsyTemplate(this.text, {});
      this.onMessage?.(KSY.applied(report.name));
      this.onApply?.(report.name, false);
    } catch (e) {
      this.onMessage?.(e instanceof Error ? e.message : String(e), true);
    }
  }

  protected override lineOfPath(text: string, path: string): number | null {
    return lineOfKsyPath(text, path);
  }
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

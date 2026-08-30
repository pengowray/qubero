// The file as the text it is.
//
// The hex grid answers "what is at this address" and the listing answers "how
// is this file put together". Neither reads a file the way it was written to
// be read, and plenty of files were: a log, a manifest, a terminal captured to
// disk, a hex dump somebody pasted. This is the view for those.
//
// It scrolls by byte offset, not by line number, for the reason the listing
// already has: nothing can say how many lines a file has without reading all
// of it. So the scrollbar stands for a position in the file, exactly as the
// hex grid's does, and the gutter shows the offset of each line rather than a
// line number that would only be right if the file had been read from the top.
//
// Three things a text file does not say about itself are shown rather than
// smoothed over, because in a hex editor they are the interesting part:
// which line ending each line used, which bytes did not fit the encoding, and
// where the escape sequences are. A capture of a coloured terminal is not
// noise to be hidden; it is what is in the file.

import { formatOffset } from "./doc.js";
import type { Doc, TextLine, TextReading } from "./doc.js";
import { el } from "./dom.js";
import { TEXTVIEW } from "./strings.js";

/** Height of one row, which must match `--tv-row` in the stylesheet: rows are
 *  placed by arithmetic on it. */
const ROW = 20;
/** Rows drawn above and below the window. */
const OVERSCAN = 4;
/** How tall the scrolling canvas is allowed to get. Browsers stop honouring an
 *  element's height past a few tens of millions of pixels. */
const MAX_CANVAS = 20_000_000;
/** Characters drawn on one line before the rest is left off. The core cuts a
 *  line at 4 KiB; this is what fits across a screen with room to spare. */
const MAX_CHARS = 2000;
/** How much of a selection one copy carries. A selection can be the whole
 *  file, and the clipboard is not where a gigabyte belongs. */
const COPY_LIMIT = 1 << 20;

/** Line endings named the way the core names them. */
const ENDINGS = new Set(["LF", "CRLF", "CR", "no ending", "cut"]);

export class TextView {
  readonly el: HTMLElement;
  private readonly gutter: HTMLElement;
  private readonly rows: HTMLElement;
  private readonly canvas: HTMLElement;
  private readonly scroll: HTMLElement;

  /** Byte offset of the first line on screen. */
  private top = 0;
  /** What is drawn now. */
  private lines: readonly TextLine[] = [];
  private reading: TextReading = { encoding: "UTF-8", mark: 0, guessed: true, unit: 1 };
  /** An encoding the reader chose, or "" to let the file decide. */
  private chosen = "";
  /** The byte the cursor is on, so the character holding it can be marked. */
  private cursor = 0;
  /** The ending most lines on screen used, so a line using another can be
   *  marked as the odd one rather than every line being labelled. */
  private usualEnding = "";
  private drawing = false;
  private pending = false;
  /** True while a movement key is extending a selection rather than moving
   *  away from one. */
  private extending = false;

  /** Called when the reader picks a character, with the byte it starts at. */
  onPick: (at: number) => void = () => {};
  /** Called when the reading is settled, so the toolbar can say what it is. */
  onReading: (r: TextReading, usualEnding: string) => void = () => {};
  /** Called when the file changed, so the rest of the page catches up. */
  onEdit: () => void = () => {};
  /** Called when the encoding has no room for a character that was typed. */
  onRefuse: (char: string, encoding: string) => void = () => {};
  /** Called with something to tell the reader, which the toolbar shows. */
  onMessage: (text: string) => void = () => {};
  /** Called when the reader selects a stretch, in bytes. An empty stretch
   *  clears the selection, so one callback covers both. */
  onSelect: (startByte: number, endByte: number, caretByte: number) => void = () => {};

  /** The selected bytes, as the rest of the app has them. Held rather than
   *  owned: the hex view is where a selection lives, and this renders what it
   *  says and writes back through it, so the two can never drift apart. */
  private selection: { start: number; end: number } | null = null;
  /** Where a keyboard or pointer selection is being extended from. */
  private anchor: number | null = null;
  private dragging = false;

  constructor(private doc: Doc) {
    this.gutter = el("div", { className: "tv-gutter" });
    this.rows = el("div", { className: "tv-rows" });
    this.canvas = el("div", { className: "tv-canvas" }, this.gutter, this.rows);
    this.scroll = el("div", { className: "tv-scroll", tabIndex: 0 }, this.canvas);
    this.scroll.setAttribute("role", "region");
    this.scroll.setAttribute("aria-label", TEXTVIEW.regionLabel);
    this.el = el("div", { className: "textview" }, this.scroll);

    this.scroll.addEventListener("scroll", () => this.onScroll());
    this.scroll.addEventListener("keydown", (e) => this.onKey(e));
    this.rows.addEventListener("pointerdown", (e) => this.onPointerDown(e));
    this.rows.addEventListener("pointermove", (e) => this.onPointerMove(e));
    window.addEventListener("pointerup", () => {
      this.dragging = false;
    });
    new ResizeObserver(() => void this.draw()).observe(this.scroll);
  }

  /** How many rows fit. */
  private visible(): number {
    return Math.max(1, Math.floor(this.scroll.clientHeight / ROW));
  }

  /** The encoding in use, or "" when the file decided. */
  get encoding(): string {
    return this.chosen;
  }

  /** Read the file as this encoding instead. "" hands it back to the file. */
  async setEncoding(name: string): Promise<void> {
    this.chosen = name;
    await this.draw(true);
  }

  /** Show the line holding this byte, and mark the character it falls in. */
  async setByte(at: number): Promise<void> {
    this.cursor = at;
    const onScreen = this.lines.some((l) => at >= l.at && at < l.at + l.len);
    if (!onScreen) {
      const b = await this.doc.textBack(this.chosen, at, 0);
      this.top = b.start;
      this.syncScrollbar();
    }
    await this.draw();
  }

  /** Show the selection the rest of the app is holding. */
  setSelection(startByte: number | null, endByte: number): void {
    this.selection = startByte === null || endByte <= startByte ? null : { start: startByte, end: endByte };
    if (this.selection === null) this.anchor = null;
    this.render();
  }

  relayout(): void {
    void this.draw(true);
  }

  /** The scrollbar stands for a position in the file: its canvas is as tall as
   *  the file is long, scaled to something a browser will honour. */
  private canvasHeight(): number {
    const len = Math.max(1, this.doc.lengthBytes);
    return Math.min(MAX_CANVAS, Math.max(this.scroll.clientHeight + 1, len));
  }

  private syncScrollbar(): void {
    const len = Math.max(1, this.doc.lengthBytes);
    const h = this.canvasHeight();
    const y = Math.round((this.top / len) * (h - this.scroll.clientHeight));
    if (Math.abs(this.scroll.scrollTop - y) > 1) {
      this.ignoreScroll = true;
      this.scroll.scrollTop = y;
    }
  }

  private ignoreScroll = false;

  private onScroll(): void {
    if (this.ignoreScroll) {
      this.ignoreScroll = false;
      return;
    }
    const len = Math.max(1, this.doc.lengthBytes);
    const h = this.canvasHeight();
    const span = Math.max(1, h - this.scroll.clientHeight);
    const want = Math.round((this.scroll.scrollTop / span) * len);
    void this.jump(want);
  }

  /** Move to the line holding a byte, without moving the cursor. */
  private async jump(at: number): Promise<void> {
    const b = await this.doc.textBack(this.chosen, Math.max(0, Math.min(at, this.doc.lengthBytes)), 0);
    this.top = b.start;
    await this.draw();
  }

  /** Where a line's text stops, which is where its ending starts. */
  private textEnd(line: TextLine): number {
    const u = this.reading.unit;
    const ending = line.ending === "CRLF" ? 2 * u : line.ending === "LF" || line.ending === "CR" ? u : 0;
    return line.at + line.len - ending;
  }

  /** The line on screen holding the caret, and where it is in that list. */
  private caretLine(): { line: TextLine; index: number } | null {
    const index = this.lines.findIndex((l) => this.cursor >= l.at && this.cursor <= this.textEnd(l));
    const line = this.lines[index];
    return line === undefined ? null : { line, index };
  }

  private onKey(e: KeyboardEvent): void {
    const page = this.visible() - 1;
    const scroll = (n: number): void => {
      e.preventDefault();
      void this.scrollLines(n);
    };
    const move = (f: () => Promise<void>): void => {
      e.preventDefault();
      void f();
    };
    if (e.ctrlKey || e.metaKey) {
      if (e.key === "c" || e.key === "C") {
        e.preventDefault();
        void this.copySelection();
      }
      return;
    }
    if (e.altKey) return;
    // Shift with a movement key extends a selection from wherever the caret
    // was when the first of them was pressed. Anything else drops it, which is
    // what every other text view does.
    const extending = e.shiftKey && SELECTING.has(e.key);
    if (extending && this.anchor === null) this.anchor = this.cursor;
    else if (!extending && !e.shiftKey) this.anchor = null;
    this.extending = extending;
    switch (e.key) {
      case "ArrowRight":
        return move(() => this.moveChar(1));
      case "ArrowLeft":
        return move(() => this.moveChar(-1));
      case "ArrowDown":
        return move(() => this.moveLine(1));
      case "ArrowUp":
        return move(() => this.moveLine(-1));
      case "PageDown":
        return scroll(page);
      case "PageUp":
        return scroll(-page);
      case "Home":
        return move(() => this.moveToLineEdge("start"));
      case "End":
        return move(() => this.moveToLineEdge("end"));
      case "Backspace":
        return move(() => this.erase(-1));
      case "Delete":
        return move(() => this.erase(1));
      case "Enter":
        return move(() => this.insert(endingText(this.usualEnding)));
      default:
    }
    // One printable character. A key name longer than a character is a key and
    // not something to type: "Shift", "F5", "Escape".
    if ([...e.key].length === 1) move(() => this.insert(e.key));
  }

  /** Move the caret a character left or right, following it onto the line
   *  above or below when it runs off the end of this one. */
  private async moveChar(dir: 1 | -1): Promise<void> {
    const here = this.caretLine();
    if (here === null) return;
    const cells = this.charsOf(here.line);
    if (dir === 1) {
      const next = cells.find((c) => c.at >= this.cursor);
      if (next !== undefined && next.at === this.cursor) return this.place(this.cursor + next.width);
      return this.place(here.line.at + here.line.len);
    }
    const prev = [...cells].reverse().find((c) => c.at < this.cursor);
    if (prev !== undefined) return this.place(prev.at);
    if (here.line.at === 0) return;
    const b = await this.doc.textBack(this.chosen, here.line.at - 1, 0);
    await this.place(b.start);
    const above = this.caretLine();
    if (above !== null) await this.place(this.textEnd(above.line));
  }

  /** Move the caret a line up or down, keeping the character it was on. */
  private async moveLine(dir: 1 | -1): Promise<void> {
    const here = this.caretLine();
    if (here === null) return;
    const column = this.charsOf(here.line).filter((c) => c.at < this.cursor).length;
    const target = this.lines[here.index + dir];
    if (target === undefined) {
      await this.scrollLines(dir);
      const after = this.caretLine();
      if (after === null) return;
      const next = this.lines[after.index + dir];
      if (next === undefined) return;
      return this.place(this.columnAt(next, column));
    }
    return this.place(this.columnAt(target, column));
  }

  /** The byte a column lands on in a line, clamped to its end. */
  private columnAt(line: TextLine, column: number): number {
    const cells = this.charsOf(line);
    return cells[column]?.at ?? this.textEnd(line);
  }

  private async moveToLineEdge(edge: "start" | "end"): Promise<void> {
    const here = this.caretLine();
    if (here === null) return;
    await this.place(edge === "start" ? here.line.at : this.textEnd(here.line));
  }

  /** Put the caret on a byte and tell the rest of the app, which is what moves
   *  every other view: the caret is the cursor, not a second position. */
  private async place(at: number): Promise<void> {
    const clamped = Math.max(0, Math.min(at, this.doc.lengthBytes));
    this.cursor = clamped;
    // While a selection is being extended the cursor moves with it, so it is
    // the selection that carries the caret. Announcing the caret on its own as
    // well would clear the selection, which is what putting the cursor
    // somewhere means everywhere else.
    if (this.extending) this.extendTo(clamped);
    else {
      if (this.selection !== null) this.selection = null;
      this.onPick(clamped);
    }
    await this.draw();
  }

  /** Type text in at the caret, over whatever is selected. */
  private async insert(text: string): Promise<void> {
    const got = this.doc.encodeText(this.chosen, this.reading.encoding, text);
    if (got.refused !== "") {
      this.onRefuse(got.refused, this.chosen === "" ? this.reading.encoding : this.chosen);
      return;
    }
    const bytes = Uint8Array.from(got.bytes);
    // What was selected goes and what was typed takes its place, in one write
    // and so in one undo step. The selection is replaced byte for byte, since
    // it may have been made over the bytes elsewhere and half a character is
    // still the bytes somebody picked.
    const sel = this.selRange();
    const at = sel === null ? this.cursor : sel.start;
    this.write(() => this.doc.replaceAt(at, sel === null ? 0 : sel.end - sel.start, bytes));
    if (sel !== null) {
      this.selection = null;
      this.anchor = null;
      this.onSelect(at, at, at);
    }
    await this.after(at + bytes.length);
  }

  /** Take out the character before the caret, or the one after it. */
  private async erase(dir: 1 | -1): Promise<void> {
    const gone = this.removeSelection();
    if (gone !== null) return this.after(gone);
    const here = this.caretLine();
    if (here === null) return;
    const cells = this.charsOf(here.line);
    if (dir === -1) {
      const prev = [...cells].reverse().find((c) => c.at < this.cursor);
      if (prev !== undefined) {
        this.write(() => this.doc.replaceAt(prev.at, prev.width, new Uint8Array()));
        return this.after(prev.at);
      }
      // At the front of a line, what is behind the caret is the line ending
      // above it, however many bytes that turned out to be.
      if (here.line.at === 0) return;
      const b = await this.doc.textBack(this.chosen, here.line.at - 1, 0);
      const above = this.lines.find((l) => l.at === b.start);
      const end = above === undefined ? here.line.at - 1 : this.textEnd(above);
      this.write(() => this.doc.replaceAt(end, here.line.at - end, new Uint8Array()));
      return this.after(end);
    }
    const next = cells.find((c) => c.at === this.cursor);
    if (next !== undefined) {
      this.write(() => this.doc.replaceAt(next.at, next.width, new Uint8Array()));
      return this.after(next.at);
    }
    // At the end of a line, what is in front of the caret is its ending.
    const ending = here.line.at + here.line.len - this.cursor;
    if (ending > 0) this.write(() => this.doc.replaceAt(this.cursor, ending, new Uint8Array()));
    return this.after(this.cursor);
  }

  /** After an edit: the caret lands where the change left it, the encoding is
   *  settled again in case the change was to the front of the file, and every
   *  other view is told. */
  private async after(at: number): Promise<void> {
    this.cursor = Math.max(0, Math.min(at, this.doc.lengthBytes));
    await this.draw(true);
    this.onPick(this.cursor);
    this.onEdit();
  }

  private async scrollLines(n: number): Promise<void> {
    if (n === 0) return;
    if (n > 0) {
      const w = await this.doc.textWindow(this.chosen, this.top, n);
      this.top = w.next;
    } else {
      const b = await this.doc.textBack(this.chosen, this.top, -n);
      this.top = b.back;
    }
    this.syncScrollbar();
    await this.draw();
  }

  /** The byte a pointer is over, or null when it is not over a character. */
  private byteUnder(e: PointerEvent): number | null {
    const target = (e.target as HTMLElement).closest<HTMLElement>("[data-at]");
    if (target === null) return null;
    const at = Number(target.dataset.at);
    if (!Number.isFinite(at)) return null;
    // Past the middle of a character the caret belongs after it, which is what
    // makes selecting the last character of a run possible.
    const box = target.getBoundingClientRect();
    const after = e.clientX > box.left + box.width / 2;
    const width = this.widthAt(at);
    return after ? at + width : at;
  }

  /** How many bytes the character starting at a byte takes. */
  private widthAt(at: number): number {
    for (const line of this.lines) {
      const cell = this.charsOf(line).find((c) => c.at === at);
      if (cell !== undefined) return cell.width;
    }
    return 1;
  }

  private onPointerDown(e: PointerEvent): void {
    const at = this.byteUnder(e);
    if (at === null) return;
    e.preventDefault();
    this.scroll.focus();
    if (e.shiftKey) {
      this.anchor ??= this.cursor;
      this.cursor = at;
      this.extendTo(at);
      this.render();
      return;
    }
    this.dragging = true;
    this.anchor = at;
    this.cursor = at;
    this.selection = null;
    this.onSelect(at, at, at);
    this.onPick(at);
    this.render();
  }

  private onPointerMove(e: PointerEvent): void {
    if (!this.dragging) return;
    const at = this.byteUnder(e);
    if (at === null) return;
    this.cursor = at;
    this.extendTo(at);
    this.render();
  }

  /** Draw what fits, reading only that. */
  async draw(force = false): Promise<void> {
    if (this.drawing) {
      this.pending = true;
      return;
    }
    this.drawing = true;
    try {
      do {
        this.pending = false;
        if (force || this.reading.encoding === "") {
          this.reading = await this.doc.textReading(this.chosen);
        }
        const want = this.visible() + OVERSCAN;
        const w = await this.doc.textWindow(this.chosen, this.top, want);
        this.lines = w.lines;
        this.usualEnding = commonEnding(w.lines);
        this.render();
        this.onReading(this.reading, this.usualEnding);
      } while (this.pending);
    } finally {
      this.drawing = false;
    }
  }

  private render(): void {
    this.canvas.style.height = `${this.canvasHeight()}px`;
    const gutter: HTMLElement[] = [];
    const rows: HTMLElement[] = [];
    for (const line of this.lines) {
      const g = el("div", { className: "tv-off", textContent: formatOffset(line.at * 8) });
      gutter.push(g);
      rows.push(this.row(line));
    }
    this.gutter.replaceChildren(...gutter);
    this.rows.replaceChildren(...rows);
  }

  /** Where each character of a line sits and how many bytes it takes. The
   *  write path asks this as much as the drawing does: what backspace removes
   *  is the width of the character before the caret, not one byte. */
  private charsOf(line: TextLine): { char: string; at: number; width: number }[] {
    const out: { char: string; at: number; width: number }[] = [];
    let at = line.at;
    for (const c of line.text) {
      const width = charWidth(c, this.reading.encoding, this.reading.unit);
      out.push({ char: c, at, width });
      at += width;
    }
    return out;
  }

  private row(line: TextLine): HTMLElement {
    const row = el("div", { className: "tv-row" });
    if (line.lossy) row.classList.add("is-lossy");
    const cells = this.charsOf(line);
    const escapes = escapeMask(cells.length, line.escapes);
    const sel = this.selection;
    const shown = Math.min(cells.length, MAX_CHARS);
    // One span per character rather than one per run of them. A selection and
    // a caret both land between characters, and finding which one a pointer is
    // over is what the spans are for.
    for (let i = 0; i < shown; i++) {
      const cell = cells[i];
      if (cell === undefined) continue;
      const { char: c, at, width } = cell;
      if (this.cursor === at) row.append(el("span", { className: "tv-caret" }));
      const span = el("span", { className: "tv-text" });
      span.dataset.at = String(at);
      if (escapes[i] === true) span.classList.add("tv-esc");
      else if (control(c) !== null) span.classList.add("tv-ctl");
      // The cursor may sit inside a character rather than on its front, which
      // is what a selection made over the bytes elsewhere can do.
      if (this.cursor >= at && this.cursor < at + width) span.classList.add("is-cursor");
      if (sel !== null && at < sel.end && at + width > sel.start) span.classList.add("is-sel");
      span.append(control(c) ?? c);
      row.append(span);
    }
    const end = this.textEnd(line);
    if (this.cursor === end) row.append(el("span", { className: "tv-caret" }));
    // A line ending inside the selection is shown as a selected space at the
    // end of the line, so a selection across lines does not look like it stops
    // at each of them.
    if (sel !== null && end < sel.end && line.at + line.len > sel.start && line.ending !== "no ending" && line.ending !== "cut") {
      row.append(el("span", { className: "tv-text is-sel tv-eol", textContent: " " }));
    }
    if (cells.length > shown) row.append(el("span", { className: "tv-more", textContent: TEXTVIEW.lineClipped }));
    if (line.ending === "cut") row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineCut }));
    else if (ENDINGS.has(line.ending) && line.ending !== this.usualEnding && this.usualEnding !== "") {
      row.append(el("span", { className: "tv-mark", textContent: line.ending }));
    }
    if (line.lossy) row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineLossy(this.reading.encoding) }));
    return row;
  }

  /** The selection as bytes, or null. */
  private selRange(): { start: number; end: number } | null {
    return this.selection;
  }

  /** Put the selection where a move left it, and tell the rest of the app. */
  private extendTo(at: number): void {
    if (this.anchor === null) return;
    const start = Math.min(this.anchor, at);
    const end = Math.max(this.anchor, at);
    this.selection = end > start ? { start, end } : null;
    this.onSelect(start, end, at);
  }

  /** One change to the file, however many writes it takes.
   *
   *  A replacement that changes length is a delete and an insert underneath,
   *  so without this a keystroke over a selection would take two presses of
   *  undo to put back and the file would sit in a state nobody typed. */
  private write(f: () => void): void {
    this.doc.beginBatch();
    try {
      f();
    } finally {
      this.doc.endBatch();
    }
  }

  /** Take out whatever is selected, leaving the caret where it was. */
  private removeSelection(): number | null {
    const sel = this.selRange();
    if (sel === null) return null;
    this.write(() => this.doc.replaceAt(sel.start, sel.end - sel.start, new Uint8Array()));
    this.selection = null;
    this.anchor = null;
    this.onSelect(sel.start, sel.start, sel.start);
    return sel.start;
  }

  /** The selected text, for the clipboard. Read from the document rather than
   *  from the screen, so a selection reaching past what is drawn copies whole
   *  rather than copying what happens to be visible. */
  private async copySelection(): Promise<void> {
    const sel = this.selRange();
    if (sel === null) return;
    const len = Math.min(sel.end - sel.start, COPY_LIMIT);
    const got = this.doc.selectionText(sel.start, len, this.chosen === "" ? this.reading.encoding : this.chosen);
    const text = got?.readings[0]?.text;
    if (text === undefined) return;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      this.onMessage(TEXTVIEW.copyFailed);
    }
  }
}

/** Movement keys that extend a selection when Shift is down. */
const SELECTING = new Set(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"]);

/** The characters a line ending is made of, so Enter writes what the rest of
 *  the file writes. A file that has not settled on one gets a line feed, which
 *  is what a file with no endings at all would be given by anything else. */
function endingText(usual: string): string {
  return usual === "CRLF" ? "\r\n" : usual === "CR" ? "\r" : "\n";
}

/** Which characters of a line are inside an escape sequence. */
function escapeMask(len: number, flat: readonly number[]): boolean[] {
  const mask = new Array<boolean>(len).fill(false);
  for (let i = 0; i + 1 < flat.length; i += 2) {
    const start = flat[i] ?? 0;
    const n = flat[i + 1] ?? 0;
    for (let j = start; j < start + n && j < len; j++) mask[j] = true;
  }
  return mask;
}

/** The picture Unicode gives a control character, so a stray byte is visible
 *  rather than moving the text around. */
function control(c: string): string | null {
  const code = c.codePointAt(0) ?? 0;
  if (code < 0x20) return String.fromCodePoint(0x2400 + code);
  if (code === 0x7f) return "\u{2421}";
  return null;
}

/** How many bytes one character takes in an encoding. This is a write path as
 *  much as a display one: a character above the basic plane is two UTF-16 code
 *  units, so backspacing over one has to take four bytes and not two. */
function charWidth(c: string, encoding: string, unit: number): number {
  const code = c.codePointAt(0) ?? 0;
  if (encoding === "UTF-8") return code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  if (unit === 2) return code < 0x10000 ? 2 : 4;
  return 1;
}

/** The ending most lines used, so the odd one out can be the one that is
 *  marked. Nothing is marked when the file has not settled on one. */
function commonEnding(lines: readonly TextLine[]): string {
  const count = new Map<string, number>();
  for (const l of lines) {
    if (l.ending === "cut" || l.ending === "no ending") continue;
    count.set(l.ending, (count.get(l.ending) ?? 0) + 1);
  }
  let best = "";
  let most = 0;
  for (const [k, n] of count) {
    if (n > most) {
      best = k;
      most = n;
    }
  }
  return best;
}

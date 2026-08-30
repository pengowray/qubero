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

  /** Called when the reader picks a character, with the byte it starts at. */
  onPick: (at: number) => void = () => {};
  /** Called when the reading is settled, so the toolbar can say what it is. */
  onReading: (r: TextReading, usualEnding: string) => void = () => {};
  /** Called when the file changed, so the rest of the page catches up. */
  onEdit: () => void = () => {};
  /** Called when the encoding has no room for a character that was typed. */
  onRefuse: (char: string, encoding: string) => void = () => {};

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
    this.rows.addEventListener("click", (e) => this.onClick(e));
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
    if (e.ctrlKey || e.metaKey || e.altKey) return;
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
    this.onPick(clamped);
    await this.draw();
  }

  /** Type text in at the caret. */
  private async insert(text: string): Promise<void> {
    const got = this.doc.encodeText(this.chosen, this.reading.encoding, text);
    if (got.refused !== "") {
      this.onRefuse(got.refused, this.chosen === "" ? this.reading.encoding : this.chosen);
      return;
    }
    const bytes = Uint8Array.from(got.bytes);
    this.doc.replaceAt(this.cursor, 0, bytes);
    await this.after(this.cursor + bytes.length);
  }

  /** Take out the character before the caret, or the one after it. */
  private async erase(dir: 1 | -1): Promise<void> {
    const here = this.caretLine();
    if (here === null) return;
    const cells = this.charsOf(here.line);
    if (dir === -1) {
      const prev = [...cells].reverse().find((c) => c.at < this.cursor);
      if (prev !== undefined) {
        this.doc.replaceAt(prev.at, prev.width, new Uint8Array());
        return this.after(prev.at);
      }
      // At the front of a line, what is behind the caret is the line ending
      // above it, however many bytes that turned out to be.
      if (here.line.at === 0) return;
      const b = await this.doc.textBack(this.chosen, here.line.at - 1, 0);
      const above = this.lines.find((l) => l.at === b.start);
      const end = above === undefined ? here.line.at - 1 : this.textEnd(above);
      this.doc.replaceAt(end, here.line.at - end, new Uint8Array());
      return this.after(end);
    }
    const next = cells.find((c) => c.at === this.cursor);
    if (next !== undefined) {
      this.doc.replaceAt(next.at, next.width, new Uint8Array());
      return this.after(next.at);
    }
    // At the end of a line, what is in front of the caret is its ending.
    const gone = here.line.at + here.line.len - this.cursor;
    if (gone > 0) this.doc.replaceAt(this.cursor, gone, new Uint8Array());
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

  private onClick(e: MouseEvent): void {
    const target = (e.target as HTMLElement).closest<HTMLElement>("[data-at]");
    if (target === null) return;
    const at = Number(target.dataset.at);
    if (Number.isFinite(at)) this.onPick(at);
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
    let run: HTMLElement | null = null;
    let runKind = "";
    const shown = Math.min(cells.length, MAX_CHARS);
    // The caret sits between bytes, so it is drawn before the character it is
    // in front of rather than on one. A caret at the end of a line has no
    // character to sit before, which is what the last check is for.
    const caretHere = (at: number): boolean => this.cursor === at;
    for (let i = 0; i < shown; i++) {
      const cell = cells[i];
      if (cell === undefined) continue;
      const { char: c, at, width } = cell;
      const kind = escapes[i] === true ? "tv-esc" : control(c) !== null ? "tv-ctl" : "";
      const onCursor = this.cursor >= at && this.cursor < at + width;
      if (caretHere(at)) row.append(el("span", { className: "tv-caret" }));
      if (run === null || runKind !== kind || onCursor) {
        run = el("span", { className: kind === "" ? "tv-text" : `tv-text ${kind}` });
        run.dataset.at = String(at);
        runKind = kind;
        row.append(run);
      }
      if (onCursor) run.classList.add("is-cursor");
      run.append(control(c) ?? c);
      if (onCursor) {
        run = null;
        runKind = "";
      }
    }
    if (caretHere(this.textEnd(line))) row.append(el("span", { className: "tv-caret" }));
    if (cells.length > shown) row.append(el("span", { className: "tv-more", textContent: TEXTVIEW.lineClipped }));
    if (line.ending === "cut") row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineCut }));
    else if (ENDINGS.has(line.ending) && line.ending !== this.usualEnding && this.usualEnding !== "") {
      row.append(el("span", { className: "tv-mark", textContent: line.ending }));
    }
    if (line.lossy) row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineLossy(this.reading.encoding) }));
    return row;
  }
}

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

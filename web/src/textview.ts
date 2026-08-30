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

  private onKey(e: KeyboardEvent): void {
    const page = this.visible() - 1;
    const step = (n: number): void => {
      e.preventDefault();
      void this.scrollLines(n);
    };
    switch (e.key) {
      case "ArrowDown":
        return step(1);
      case "ArrowUp":
        return step(-1);
      case "PageDown":
        return step(page);
      case "PageUp":
        return step(-page);
      case "Home":
        e.preventDefault();
        void this.jump(0);
        return;
      case "End":
        e.preventDefault();
        void this.jump(this.doc.lengthBytes);
        return;
      default:
    }
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

  private row(line: TextLine): HTMLElement {
    const row = el("div", { className: "tv-row" });
    if (line.lossy) row.classList.add("is-lossy");
    const chars = [...line.text];
    const escapes = escapeMask(chars.length, line.escapes);
    const unit = this.reading.unit;
    const utf8 = this.reading.encoding === "UTF-8";
    let at = line.at;
    let run: HTMLElement | null = null;
    let runKind = "";
    const shown = Math.min(chars.length, MAX_CHARS);
    for (let i = 0; i < shown; i++) {
      const c = chars[i] ?? "";
      const width = utf8 ? utf8Width(c) : unit;
      const kind = escapes[i] === true ? "tv-esc" : control(c) !== null ? "tv-ctl" : "";
      const onCursor = this.cursor >= at && this.cursor < at + width;
      if (run === null || runKind !== kind || onCursor) {
        run = el("span", { className: kind === "" ? "tv-text" : `tv-text ${kind}` });
        run.dataset.at = String(at);
        runKind = kind;
        row.append(run);
      }
      if (onCursor) run.classList.add("is-cursor");
      run.append(control(c) ?? c);
      at += width;
      if (onCursor) {
        run = null;
        runKind = "";
      }
    }
    if (chars.length > shown) row.append(el("span", { className: "tv-more", textContent: TEXTVIEW.lineClipped }));
    if (line.ending === "cut") row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineCut }));
    else if (ENDINGS.has(line.ending) && line.ending !== this.usualEnding && this.usualEnding !== "") {
      row.append(el("span", { className: "tv-mark", textContent: line.ending }));
    }
    if (line.lossy) row.append(el("span", { className: "tv-mark", textContent: TEXTVIEW.lineLossy(this.reading.encoding) }));
    return row;
  }
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

function utf8Width(c: string): number {
  const code = c.codePointAt(0) ?? 0;
  if (code < 0x80) return 1;
  if (code < 0x800) return 2;
  if (code < 0x10000) return 3;
  return 4;
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

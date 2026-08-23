// Virtualised address + hex + ascii view.
//
// The browser cannot host a scroll container billions of pixels tall, so this view
// does not use native scrolling for the document. It keeps a `topRow` and renders
// only the rows that fit, with its own scrollbar mapped row <-> file offset.

import type { Doc, Span } from "./doc.js";

export type Pane = "hex" | "ascii";
/** What sits to the right of the bytes: their text, or what the template says
 *  each one is. */
export type RightColumn = "text" | "fields";
/** Hex shows two digits per byte; binary shows the eight bits. */
export type ViewMode = "hex" | "binary";

export type CursorState = {
  readonly offset: number;
  /** Absolute bit position. In hex mode this is always `offset * 8`. */
  readonly bitOffset: number;
  readonly pane: Pane;
  readonly insertMode: boolean;
  readonly mode: ViewMode;
};

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

/** How many entries one screenful of the annotation column may hold. */
const SPAN_LIMIT = 600;
/** Colours the annotation column cycles through, as class suffixes. */
const TINTS = 6;
/** What a stretch of bytes no field covers is called. */
const GAP_LABEL = "not described";
/** Longest value shown on a chip before it is cut short. */
const CHIP_VALUE = 32;
/** Rough width of a character in the chip font, for working out how many
 *  chips fit before any of them are drawn. */
const CHIP_CHAR = 6.7;
/** Padding, border and gap around a chip's text. */
const CHIP_CHROME = 20;

/** What a chip says after the name. A run of numbers says how many; raw bytes
 *  say how many, since the bytes themselves are already on the left. */
function chipDetail(s: Span): string {
  if (s.gap) return "";
  if (s.count > 0) return `${s.count.toLocaleString()} values`;
  if (s.kind === "bytes") {
    return s.size_bits % 8 === 0
      ? `${(s.size_bits / 8).toLocaleString()} bytes`
      : `${s.size_bits.toLocaleString()} bits`;
  }
  return s.value.length > CHIP_VALUE ? `${s.value.slice(0, CHIP_VALUE)}\u2026` : s.value;
}

/** A field's colour, from its place in the tree rather than its place on the
 *  screen, so scrolling never repaints the file in different colours. */
function tintOf(path: readonly number[]): number {
  let n = path.length;
  for (const i of path) n = (n * 31 + i) % (TINTS * 7919);
  return n % TINTS;
}

function asciiGlyph(b: number): string {
  return b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "·";
}

export class HexView {
  readonly el: HTMLElement;
  private readonly header: HTMLElement;
  private readonly rowsEl: HTMLElement;
  private readonly track: HTMLElement;
  private readonly thumb: HTMLElement;
  private rowEls: HTMLElement[] = [];

  private topRow = 0;
  private visibleRows = 1;
  private rowHeight = 20;
  private bytesPerRow = 16;
  private cursor = 0;
  private nibble: 0 | 1 = 0;
  /** Bit within the cursor byte, 0 = most significant. Set by a bit move or by
   * picking a field that starts inside a byte; any byte-level move clears it. */
  private bit = 0;
  private mode: ViewMode = "hex";
  private pane: Pane = "hex";
  private insertMode = false;
  private dragging: { startY: number; startRow: number } | null = null;
  /** Bit range [startBit, endBit) to highlight, e.g. the selected field. */
  private highlight: { startBit: number; endBit: number } | null = null;
  private rightColumn: RightColumn = "text";
  /** Spans for the rows on screen, kept until the view or the file moves. */
  private spanCache: { key: string; spans: Span[]; more: boolean } | null = null;
  /** Width of the annotation column, measured from the last frame. */
  private noteWidth = 0;

  onCursorChange: (c: CursorState) => void = () => {};
  /** A field picked in the annotation column. */
  onPickField: (path: readonly number[]) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("div");
    this.el.className = "hexview";
    this.el.tabIndex = 0;
    this.el.setAttribute("role", "grid");
    this.el.setAttribute("aria-label", "File contents");

    this.header = document.createElement("div");
    this.header.className = "hv-header";
    this.rowsEl = document.createElement("div");
    this.rowsEl.className = "hv-rows";
    const body = document.createElement("div");
    body.className = "hv-body";
    body.append(this.header, this.rowsEl);

    this.track = document.createElement("div");
    this.track.className = "hv-track";
    this.track.setAttribute("aria-hidden", "true");
    this.thumb = document.createElement("div");
    this.thumb.className = "hv-thumb";
    this.track.append(this.thumb);

    this.el.append(body, this.track);

    new ResizeObserver(() => this.relayout()).observe(this.rowsEl);
    this.el.addEventListener("wheel", (e) => this.onWheel(e), { passive: false });
    this.el.addEventListener("keydown", (e) => this.onKey(e));
    this.el.addEventListener("relayout", () => this.relayout());
    this.rowsEl.addEventListener("pointerdown", (e) => this.onPointerDown(e));
    this.rowsEl.addEventListener("pointermove", (e) => this.onPointerMove(e));
    this.rowsEl.addEventListener("pointerup", (e) => this.onPointerUp(e));
    this.rowsEl.addEventListener("pointercancel", (e) => this.onPointerUp(e));
    this.track.addEventListener("pointerdown", (e) => this.onTrackDown(e));
    this.track.addEventListener("pointermove", (e) => this.onTrackMove(e));
    this.track.addEventListener("pointerup", (e) => this.onTrackUp(e));
    this.track.addEventListener("pointercancel", (e) => this.onTrackUp(e));
    doc.onChange(() => {
      this.spanCache = null;
      this.render();
    });
  }

  // ----- geometry -----

  private get totalRows(): number {
    return Math.max(1, Math.ceil((this.doc.lengthBytes + 1) / this.bytesPerRow));
  }
  private get maxTopRow(): number {
    return Math.max(0, this.totalRows - this.visibleRows);
  }

  setBytesPerRow(n: number): void {
    this.bytesPerRow = n;
    this.topRow = Math.min(this.topRow, this.maxTopRow);
    this.relayout();
    this.scrollCursorIntoView();
    this.render();
  }

  setRightColumn(c: RightColumn): void {
    this.rightColumn = c;
    // The text column is where the "ascii" pane lives; without it the cursor
    // has nowhere to be but the bytes.
    if (c !== "text" && this.pane === "ascii") this.pane = "hex";
    this.spanCache = null;
    // Rows are taller while the field column is shown, so the number of rows
    // that fit has to be worked out again.
    this.el.classList.toggle("has-notes", c === "fields");
    this.relayout();
  }

  relayout(): void {
    const probe = this.rowEls[0];
    if (probe) this.rowHeight = Math.max(1, probe.getBoundingClientRect().height || this.rowHeight);
    const h = this.rowsEl.clientHeight;
    this.visibleRows = Math.max(1, Math.floor(h / this.rowHeight));
    this.topRow = Math.min(this.topRow, this.maxTopRow);
    this.ensureRowEls();
    this.render();
  }

  private ensureRowEls(): void {
    while (this.rowEls.length < this.visibleRows) {
      const r = document.createElement("div");
      r.className = "hv-row";
      r.setAttribute("role", "row");
      this.rowsEl.append(r);
      this.rowEls.push(r);
    }
    while (this.rowEls.length > this.visibleRows) {
      this.rowEls.pop()?.remove();
    }
  }

  // ----- cursor & scrolling -----

  get cursorState(): CursorState {
    return {
      offset: this.cursor,
      bitOffset: this.cursor * 8 + this.bit,
      pane: this.pane,
      insertMode: this.insertMode,
      mode: this.mode,
    };
  }

  setMode(mode: ViewMode): void {
    this.mode = mode;
    this.nibble = 0;
    this.scrollCursorIntoView();
    this.render();
    this.onCursorChange(this.cursorState);
  }

  /** Move the cursor to an absolute bit. Bit 0 is the top bit of byte 0. */
  setBitCursor(bitOffset: number, opts: { pane?: Pane } = {}): void {
    if (opts.pane) this.pane = opts.pane;
    const at = Math.max(0, Math.min(this.doc.lengthBits, Math.floor(bitOffset)));
    this.cursor = Math.floor(at / 8);
    this.bit = at % 8;
    this.nibble = 0;
    this.scrollCursorIntoView();
    this.render();
    this.onCursorChange(this.cursorState);
  }

  setCursor(offset: number, opts: { pane?: Pane; nibble?: 0 | 1; bit?: number } = {}): void {
    const max = this.doc.lengthBytes; // one past the end is a valid insert position
    this.cursor = Math.max(0, Math.min(max, Math.floor(offset)));
    this.nibble = opts.nibble ?? 0;
    this.bit = Math.max(0, Math.min(7, opts.bit ?? 0));
    if (opts.pane) this.pane = opts.pane;
    this.scrollCursorIntoView();
    this.render();
    this.onCursorChange(this.cursorState);
  }

  /** Called when the user dismisses the field highlight with Escape. */
  onHighlightClear: () => void = () => {};

  setHighlight(range: { startBit: number; endBit: number } | null): void {
    this.highlight = range;
    this.render();
  }

  scrollTo(row: number): void {
    this.topRow = Math.max(0, Math.min(this.maxTopRow, Math.floor(row)));
    this.render();
  }

  private scrollCursorIntoView(): void {
    const row = Math.floor(this.cursor / this.bytesPerRow);
    if (row < this.topRow) this.topRow = row;
    else if (row >= this.topRow + this.visibleRows) this.topRow = row - this.visibleRows + 1;
    this.topRow = Math.max(0, Math.min(this.maxTopRow, this.topRow));
  }

  private onWheel(e: WheelEvent): void {
    e.preventDefault();
    const rows = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? e.deltaY : e.deltaY / this.rowHeight;
    this.scrollTo(this.topRow + (rows > 0 ? Math.max(1, Math.round(rows)) : Math.min(-1, Math.round(rows))));
  }

  private onPointerDown(e: PointerEvent): void {
    this.el.focus();
    if (e.pointerType === "touch") {
      this.dragging = { startY: e.clientY, startRow: this.topRow };
      this.rowsEl.setPointerCapture(e.pointerId);
      return;
    }
    this.clickCell(e.target);
  }

  private onPointerMove(e: PointerEvent): void {
    if (!this.dragging) return;
    const dy = this.dragging.startY - e.clientY;
    this.scrollTo(this.dragging.startRow + dy / this.rowHeight);
  }

  private onPointerUp(e: PointerEvent): void {
    if (!this.dragging) return;
    const moved = Math.abs(this.dragging.startY - e.clientY) > 6;
    this.dragging = null;
    if (!moved) this.clickCell(e.target);
  }

  private clickCell(target: EventTarget | null): void {
    if (!(target instanceof HTMLElement)) return;
    const off = target.dataset["off"];
    const pane = target.dataset["pane"];
    if (off === undefined || (pane !== "hex" && pane !== "ascii")) return;
    const bit = target.dataset["bit"];
    this.setCursor(Number(off), { pane, bit: bit === undefined ? 0 : Number(bit) });
  }

  private onTrackDown(e: PointerEvent): void {
    this.track.setPointerCapture(e.pointerId);
    this.onTrackMove(e);
  }
  private onTrackMove(e: PointerEvent): void {
    if (!this.track.hasPointerCapture(e.pointerId)) return;
    const r = this.track.getBoundingClientRect();
    const thumbH = this.thumb.offsetHeight;
    const frac = (e.clientY - r.top - thumbH / 2) / Math.max(1, r.height - thumbH);
    this.scrollTo(Math.max(0, Math.min(1, frac)) * this.maxTopRow);
  }
  private onTrackUp(e: PointerEvent): void {
    if (this.track.hasPointerCapture(e.pointerId)) this.track.releasePointerCapture(e.pointerId);
  }

  // ----- editing -----

  private onKey(e: KeyboardEvent): void {
    const bpr = this.bytesPerRow;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "z" && !e.shiftKey) return void (e.preventDefault(), this.doc.undo());
    if (mod && (e.key.toLowerCase() === "y" || (e.key.toLowerCase() === "z" && e.shiftKey)))
      return void (e.preventDefault(), this.doc.redo());
    if (mod) return;

    const bitMode = this.mode === "binary" && this.pane === "hex";
    switch (e.key) {
      case "ArrowLeft":
        e.preventDefault();
        if (bitMode) return this.setBitCursor(this.cursorState.bitOffset - 1);
        if (this.pane === "hex" && this.nibble === 1) return this.setCursor(this.cursor, { nibble: 0 });
        return this.setCursor(this.cursor - 1);
      case "ArrowRight":
        e.preventDefault();
        if (bitMode) return this.setBitCursor(this.cursorState.bitOffset + 1);
        return this.setCursor(this.cursor + 1);
      case "ArrowUp":
        e.preventDefault();
        return this.setCursor(this.cursor - bpr);
      case "ArrowDown":
        e.preventDefault();
        return this.setCursor(this.cursor + bpr);
      case "PageUp":
        e.preventDefault();
        this.topRow = Math.max(0, this.topRow - this.visibleRows);
        return this.setCursor(this.cursor - this.visibleRows * bpr);
      case "PageDown":
        e.preventDefault();
        this.topRow = Math.min(this.maxTopRow, this.topRow + this.visibleRows);
        return this.setCursor(this.cursor + this.visibleRows * bpr);
      case "Home":
        e.preventDefault();
        return this.setCursor(e.shiftKey ? 0 : this.cursor - (this.cursor % bpr));
      case "End":
        e.preventDefault();
        return this.setCursor(e.shiftKey ? this.doc.lengthBytes : this.cursor - (this.cursor % bpr) + bpr - 1);
      case "Tab":
        e.preventDefault();
        return this.setCursor(this.cursor, { pane: this.pane === "hex" ? "ascii" : "hex" });
      case "Insert":
        e.preventDefault();
        this.insertMode = !this.insertMode;
        this.render();
        return this.onCursorChange(this.cursorState);
      case "Delete":
        e.preventDefault();
        if (bitMode) {
          const at = this.cursorState.bitOffset;
          if (at < this.doc.lengthBits) this.doc.deleteBits(at, 1);
          return this.setBitCursor(at);
        }
        if (this.cursor < this.doc.lengthBytes) this.doc.delete(this.cursor, 1);
        return this.setCursor(this.cursor);
      case "Backspace":
        e.preventDefault();
        if (bitMode) {
          const at = this.cursorState.bitOffset;
          if (at > 0) {
            this.doc.deleteBits(at - 1, 1);
            this.setBitCursor(at - 1);
          }
          return;
        }
        if (this.cursor > 0) {
          this.doc.delete(this.cursor - 1, 1);
          this.setCursor(this.cursor - 1);
        }
        return;
      case "Escape":
        // Move first, then drop the highlight: the cursor event can pick a new
        // field, and Escape's job is to leave nothing highlighted.
        this.setCursor(this.cursor, { nibble: 0 });
        if (this.highlight !== null) {
          this.setHighlight(null);
          this.onHighlightClear();
        }
        return;
    }

    if (e.key.length !== 1 || e.altKey) return;
    e.preventDefault();
    if (bitMode) this.typeBit(e.key);
    else if (this.pane === "hex") this.typeHex(e.key);
    else this.typeAscii(e.key);
  }

  private typeBit(ch: string): void {
    if (ch !== "0" && ch !== "1") return;
    const at = this.cursorState.bitOffset;
    const data = Uint8Array.of(ch === "1" ? 0x80 : 0);
    if (this.insertMode || at >= this.doc.lengthBits) this.doc.insertBits(at, data, 1);
    else this.doc.overwriteBits(at, data, 1);
    this.setBitCursor(at + 1);
  }

  private currentByte(): number {
    return this.doc.read(this.cursor, 1).bytes[0] ?? 0;
  }

  private typeHex(ch: string): void {
    const v = parseInt(ch, 16);
    if (Number.isNaN(v)) return;
    const atEnd = this.cursor >= this.doc.lengthBytes;
    if (this.nibble === 0) {
      if (this.insertMode || atEnd) {
        this.doc.insert(this.cursor, Uint8Array.of(v << 4));
      } else {
        this.doc.overwrite(this.cursor, Uint8Array.of((v << 4) | (this.currentByte() & 0x0f)));
      }
      this.setCursor(this.cursor, { nibble: 1 });
    } else {
      this.doc.amendOverwrite(this.cursor, Uint8Array.of((this.currentByte() & 0xf0) | v));
      this.setCursor(this.cursor + 1, { nibble: 0 });
    }
  }

  private typeAscii(ch: string): void {
    const code = ch.charCodeAt(0);
    if (code > 0xff) return;
    const atEnd = this.cursor >= this.doc.lengthBytes;
    if (this.insertMode || atEnd) this.doc.insert(this.cursor, Uint8Array.of(code));
    else this.doc.overwrite(this.cursor, Uint8Array.of(code));
    this.setCursor(this.cursor + 1);
  }

  // ----- rendering -----

  /** Which bits of byte `off` the highlight covers, as [from, to) within 0..8. */
  private highlightBits(off: number): { from: number; to: number } | null {
    const h = this.highlight;
    if (h === null) return null;
    const from = Math.max(h.startBit, off * 8) - off * 8;
    const to = Math.min(h.endBit, off * 8 + 8) - off * 8;
    return to > from ? { from, to } : null;
  }

  /** Mark part of a byte in hex mode: a bar under the bits the field covers. */
  private markBits(el: HTMLElement, from: number, to: number): void {
    // The cell is 3ch wide: half a character of padding, two digits, half again.
    const pad = 100 / 6;
    const step = (100 - 2 * pad) / 8;
    el.classList.add("hv-hlbits");
    el.style.setProperty("--from", `${pad + from * step}%`);
    el.style.setProperty("--to", `${pad + to * step}%`);
  }

  /** The eight bits of one byte, split into spans only where that is needed. */
  private fillBits(cell: HTMLElement, byte: number | null, off: number, hl: { from: number; to: number } | null): void {
    const text = byte === null ? "········" : byte.toString(2).padStart(8, "0");
    if (byte === null) cell.classList.add("hv-pending");
    const onCursor = off === this.cursor;
    const whole = hl !== null && hl.from === 0 && hl.to === 8;
    if (!onCursor && (hl === null || whole)) {
      cell.textContent = text;
      if (whole) cell.classList.add("hv-hl");
      return;
    }
    for (let k = 0; k < 8; k++) {
      const s = document.createElement("span");
      s.textContent = text[k] ?? "0";
      s.dataset["off"] = String(off);
      s.dataset["bit"] = String(k);
      s.dataset["pane"] = "hex";
      if (hl !== null && k >= hl.from && k < hl.to) s.classList.add("hv-hl");
      if (onCursor && k === this.bit) {
        s.classList.add("hv-cur", this.pane === "hex" ? "hv-focus" : "hv-dim");
        if (this.insertMode) s.classList.add("hv-ins");
      }
      cell.append(s);
    }
  }

  /** Spans for the rows on screen. A pending reply leaves the column empty
   *  for one frame; the fetched chunks trigger another render. */
  private spansForView(start: number, count: number): { spans: Span[]; more: boolean } {
    const key = `${start}:${count}:${this.doc.template ?? ""}`;
    if (this.spanCache?.key === key) return this.spanCache;
    const max = Math.min(SPAN_LIMIT, count * 8);
    const r = this.doc.spans(start * 8, (start + count) * 8, max);
    if (r.status !== "ok") return { spans: [], more: false };
    this.spanCache = { key, spans: r.node, more: r.node.length >= max };
    return this.spanCache;
  }

  /** One entry in the annotation column, coloured to match its bytes. */
  private chip(s: Span, carried: boolean): HTMLElement {
    const el = document.createElement("button");
    el.type = "button";
    el.className = "hv-chip";
    if (s.gap) el.classList.add("hv-chip-gap");
    else el.classList.add(`hv-t${tintOf(s.path)}`);
    if (carried) el.classList.add("hv-chip-carried");
    const name = document.createElement("b");
    name.textContent = s.gap ? GAP_LABEL : s.name;
    el.append(name);
    const detail = chipDetail(s);
    if (detail !== "" && !s.gap) {
      const v = document.createElement("span");
      v.className = "hv-chip-val";
      v.textContent = detail;
      el.append(v);
    }
    const where = s.trail.length > 0 ? `${s.trail.join(" ")} ` : "";
    el.title = s.gap
      ? `${GAP_LABEL}: ${(s.size_bits / 8).toLocaleString()} bytes inside ${where}${s.name}`
      : `${where}${s.name} \u00b7 ${s.type}`;
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      if (!s.gap) this.onPickField(s.path);
    });
    if (s.gap) el.disabled = true;
    return el;
  }

  render(): void {
    const bpr = this.bytesPerRow;
    const len = this.doc.lengthBytes;
    const addrWidth = Math.max(8, len.toString(16).length);
    const start = this.topRow * bpr;
    const windowBytes = this.visibleRows * bpr;
    const { bytes, complete } = this.doc.read(start, windowBytes);
    const binary = this.mode === "binary";
    const fields = this.rightColumn === "fields" && this.doc.template !== null;

    // Which span covers each byte on screen, and which start on each row.
    let spans: Span[] = [];
    let more = false;
    const byteSpan = new Int32Array(windowBytes).fill(-1);
    const byRow: { span: Span; carried: boolean }[][] = [];
    if (fields) {
      ({ spans, more } = this.spansForView(start, windowBytes));
      for (let r = 0; r < this.visibleRows; r++) byRow.push([]);
      for (const [i, s] of spans.entries()) {
        const from = Math.floor(s.offset_bits / 8);
        const to = Math.ceil((s.offset_bits + s.size_bits) / 8);
        for (let b = Math.max(from, start); b < Math.min(to, start + windowBytes); b++) {
          byteSpan[b - start] = i;
        }
        // A field is named on the row it starts on. One that started above the
        // view is named on the first row, so nothing on screen is unexplained.
        const row = from < start ? 0 : Math.floor((from - start) / bpr);
        if (row >= 0 && row < this.visibleRows && to > start) {
          byRow[row]?.push({ span: s, carried: from < start });
        }
      }
    }

    const columns = document.createElement("span");
    columns.textContent =
      " ".repeat(addrWidth) +
      "  " +
      Array.from({ length: bpr }, (_, i) => (binary ? (HEX[i] ?? "").padEnd(8) : HEX[i])).join(" ");
    this.header.replaceChildren(columns);
    if (fields) {
      const title = document.createElement("span");
      title.className = "hv-note hv-head-note";
      title.textContent = "Fields";
      this.header.append(title);
    }

    for (let r = 0; r < this.rowEls.length; r++) {
      const row = this.rowEls[r];
      if (!row) continue;
      const rowStart = start + r * bpr;
      if (rowStart > len) {
        row.replaceChildren();
        continue;
      }
      const frag = document.createDocumentFragment();
      const addr = document.createElement("span");
      addr.className = "hv-addr";
      addr.textContent = rowStart.toString(16).padStart(addrWidth, "0");
      frag.append(addr);

      const cells = document.createElement("span");
      cells.className = binary ? "hv-bits" : "hv-hex";
      const asc = document.createElement("span");
      asc.className = "hv-ascii";
      for (let i = 0; i < bpr; i++) {
        const off = rowStart + i;
        const h = document.createElement("span");
        const a = document.createElement("span");
        h.dataset["off"] = a.dataset["off"] = String(off);
        h.dataset["pane"] = "hex";
        a.dataset["pane"] = "ascii";
        const hl = this.highlightBits(off);
        if (off < len) {
          const b = bytes[off - start] ?? 0;
          a.textContent = complete ? asciiGlyph(b) : " ";
          if (complete && !(b >= 0x20 && b < 0x7f)) a.classList.add("hv-np");
          if (binary) {
            this.fillBits(h, complete ? b : null, off, hl);
          } else {
            h.textContent = complete ? HEX[b] ?? "" : "··";
            if (!complete) h.classList.add("hv-pending");
          }
        } else if (off === len) {
          h.textContent = binary ? "        " : "  ";
          a.textContent = " ";
          h.classList.add("hv-end");
        } else {
          h.textContent = binary ? "        " : "  ";
          a.textContent = " ";
        }

        const si = fields && off >= start && off < start + windowBytes ? byteSpan[off - start] ?? -1 : -1;
        if (si >= 0) {
          const s = spans[si];
          if (s !== undefined && !s.gap) h.classList.add("hv-tint", `hv-t${tintOf(s.path)}`);
        }
        if (hl !== null) {
          // The text column cannot show part of a byte, so a partly covered
          // byte is marked more faintly there than a fully covered one.
          a.classList.add(hl.from === 0 && hl.to === 8 ? "hv-hl" : "hv-hl-weak");
          if (!binary && off < len) {
            if (hl.from === 0 && hl.to === 8) h.classList.add("hv-hl");
            else this.markBits(h, hl.from, hl.to);
          }
        }
        if (off === this.cursor) {
          if (!binary) {
            h.classList.add("hv-cur", this.pane === "hex" ? "hv-focus" : "hv-dim");
            if (this.pane === "hex" && this.nibble === 1) h.classList.add("hv-nib1");
            if (this.insertMode) h.classList.add("hv-ins");
          }
          a.classList.add("hv-cur", this.pane === "ascii" ? "hv-focus" : "hv-dim");
        }
        cells.append(h);
        asc.append(a);
      }
      if (fields) {
        const note = document.createElement("span");
        note.className = "hv-note";
        const entries = byRow[r] ?? [];
        // Work out how many fit before drawing any, so what is left over can be
        // counted rather than quietly cut off.
        let room = this.noteWidth || 320;
        let shown = 0;
        for (const { span } of entries) {
          const w = CHIP_CHROME + (span.name.length + chipDetail(span).length + 1) * CHIP_CHAR;
          if (shown > 0 && w > room - (shown < entries.length - 1 ? 44 : 0)) break;
          room -= w;
          shown += 1;
        }
        for (const { span, carried } of entries.slice(0, shown)) note.append(this.chip(span, carried));
        if (shown < entries.length) {
          const rest = document.createElement("span");
          rest.className = "hv-chip hv-chip-gap hv-chip-rest";
          rest.textContent = `+${entries.length - shown}`;
          rest.title = entries
            .slice(shown)
            .map(({ span }) => span.name)
            .join(", ");
          note.append(rest);
        }
        if (more && r === this.rowEls.length - 1) {
          const rest = document.createElement("span");
          rest.className = "hv-chip hv-chip-gap";
          rest.textContent = "more below";
          note.append(rest);
        }
        frag.append(cells, note);
      } else {
        frag.append(cells, asc);
      }
      row.replaceChildren(frag);
    }

    if (fields) {
      const w = this.rowEls[0]?.querySelector(".hv-note")?.clientWidth ?? 0;
      // One redraw when the measured width first disagrees with the guess, so
      // the count of what did not fit is right rather than nearly right.
      const remeasured = w > 0 && Math.abs(w - this.noteWidth) > 4;
      this.noteWidth = w;
      if (remeasured) {
        this.render();
        return;
      }
    }

    // Scrollbar thumb: position is the fraction of rows above the viewport.
    const trackH = this.track.clientHeight;
    const thumbH = Math.max(24, Math.round((this.visibleRows / this.totalRows) * trackH));
    const top = this.maxTopRow === 0 ? 0 : Math.round((this.topRow / this.maxTopRow) * (trackH - thumbH));
    this.thumb.style.height = `${thumbH}px`;
    this.thumb.style.transform = `translateY(${top}px)`;
  }
}

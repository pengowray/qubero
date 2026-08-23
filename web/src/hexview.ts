// Virtualised address + hex + ascii view.
//
// The browser cannot host a scroll container billions of pixels tall, so this view
// does not use native scrolling for the document. It keeps a `topRow` and renders
// only the rows that fit, with its own scrollbar mapped row <-> file offset.

import type { Doc } from "./doc.js";

export type Pane = "hex" | "ascii";

export type CursorState = {
  readonly offset: number;
  readonly pane: Pane;
  readonly insertMode: boolean;
};

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

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
  private pane: Pane = "hex";
  private insertMode = false;
  private dragging: { startY: number; startRow: number } | null = null;
  /** Byte range [start, end) to highlight, e.g. the selected template field. */
  private highlight: { start: number; end: number } | null = null;

  onCursorChange: (c: CursorState) => void = () => {};

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
    doc.onChange(() => this.render());
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
    return { offset: this.cursor, pane: this.pane, insertMode: this.insertMode };
  }

  setCursor(offset: number, opts: { pane?: Pane; nibble?: 0 | 1 } = {}): void {
    const max = this.doc.lengthBytes; // one past the end is a valid insert position
    this.cursor = Math.max(0, Math.min(max, Math.floor(offset)));
    this.nibble = opts.nibble ?? 0;
    if (opts.pane) this.pane = opts.pane;
    this.scrollCursorIntoView();
    this.render();
    this.onCursorChange(this.cursorState);
  }

  /** Called when the user dismisses the field highlight with Escape. */
  onHighlightClear: () => void = () => {};

  setHighlight(range: { start: number; end: number } | null): void {
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
    this.setCursor(Number(off), { pane });
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

    switch (e.key) {
      case "ArrowLeft":
        e.preventDefault();
        if (this.pane === "hex" && this.nibble === 1) return this.setCursor(this.cursor, { nibble: 0 });
        return this.setCursor(this.cursor - 1);
      case "ArrowRight":
        e.preventDefault();
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
        if (this.cursor < this.doc.lengthBytes) this.doc.delete(this.cursor, 1);
        return this.setCursor(this.cursor);
      case "Backspace":
        e.preventDefault();
        if (this.cursor > 0) {
          this.doc.delete(this.cursor - 1, 1);
          this.setCursor(this.cursor - 1);
        }
        return;
      case "Escape":
        if (this.highlight !== null) {
          this.setHighlight(null);
          this.onHighlightClear();
        }
        return this.setCursor(this.cursor, { nibble: 0 });
    }

    if (e.key.length !== 1 || e.altKey) return;
    e.preventDefault();
    if (this.pane === "hex") this.typeHex(e.key);
    else this.typeAscii(e.key);
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

  render(): void {
    const bpr = this.bytesPerRow;
    const len = this.doc.lengthBytes;
    const addrWidth = Math.max(8, len.toString(16).length);
    const start = this.topRow * bpr;
    const { bytes, complete } = this.doc.read(start, this.visibleRows * bpr);

    this.header.textContent =
      " ".repeat(addrWidth) + "  " + Array.from({ length: bpr }, (_, i) => HEX[i]).join(" ");

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

      const hex = document.createElement("span");
      hex.className = "hv-hex";
      const asc = document.createElement("span");
      asc.className = "hv-ascii";
      for (let i = 0; i < bpr; i++) {
        const off = rowStart + i;
        const h = document.createElement("span");
        const a = document.createElement("span");
        h.dataset["off"] = a.dataset["off"] = String(off);
        h.dataset["pane"] = "hex";
        a.dataset["pane"] = "ascii";
        if (off < len) {
          const b = bytes[off - start] ?? 0;
          h.textContent = complete ? HEX[b] ?? "" : "··";
          a.textContent = complete ? asciiGlyph(b) : " ";
          if (!complete) h.classList.add("hv-pending");
          if (complete && !(b >= 0x20 && b < 0x7f)) a.classList.add("hv-np");
        } else if (off === len) {
          h.textContent = "  ";
          a.textContent = " ";
          h.classList.add("hv-end");
        } else {
          h.textContent = "  ";
          a.textContent = " ";
        }
        if (this.highlight && off >= this.highlight.start && off < this.highlight.end) {
          h.classList.add("hv-hl");
          a.classList.add("hv-hl");
        }
        if (off === this.cursor) {
          h.classList.add("hv-cur", this.pane === "hex" ? "hv-focus" : "hv-dim");
          a.classList.add("hv-cur", this.pane === "ascii" ? "hv-focus" : "hv-dim");
          if (this.pane === "hex" && this.nibble === 1) h.classList.add("hv-nib1");
          if (this.insertMode) h.classList.add("hv-ins");
        }
        hex.append(h);
        asc.append(a);
      }
      frag.append(hex, asc);
      row.replaceChildren(frag);
    }

    // Scrollbar thumb: position is the fraction of rows above the viewport.
    const trackH = this.track.clientHeight;
    const thumbH = Math.max(24, Math.round((this.visibleRows / this.totalRows) * trackH));
    const top = this.maxTopRow === 0 ? 0 : Math.round((this.topRow / this.maxTopRow) * (trackH - thumbH));
    this.thumb.style.height = `${thumbH}px`;
    this.thumb.style.transform = `translateY(${top}px)`;
  }
}

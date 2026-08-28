// The file read top to bottom, one row per field: where it starts, the bytes
// it occupies, and what those bytes say. The hex view answers "what is at this
// address"; this answers "what is in this file, in order".
//
// The scroll position is a bit offset rather than a row number. Nothing here
// can know how many rows a file has without walking all of it, and `spans` is
// windowed by bit range, which is the same shape.

import { formatBytes, formatOffset } from "./doc.js";
import { bitSizeText, childWord, countText, GAP_LABEL, NO_TEMPLATE_HINT, NO_TEMPLATE_MATCH } from "./strings.js";
import type { Doc, Span } from "./doc.js";
import { fieldClass } from "./fieldstyle.js";
import { appendAnatomy } from "./anatomy.js";

/** Rows fetched beyond the ones on screen, so a wheel notch has somewhere to
 *  go before the next fetch. */
const OVERSCAN = 4;
/** Bytes of a field shown before the column cuts it short. */
const BYTES_SHOWN = 8;
/** How far back to look for the rows above the top one, in bits, doubling
 *  until enough are found. Most fields are a few bytes, so this starts near a
 *  screenful of them. */
const LOOK_BACK = 1024;
const LOOK_BACK_LIMIT = 1 << 22;
/** Fields asked for in one call, however far back the window reaches. */
const SPAN_LIMIT = 4096;
/** Deep parser paths are still carried in full, but more indentation stops
 * helping after a few levels. The heading continues to name the changed
 * suffix, so no context is thrown away. */
const MAX_INDENT = 4;
/** Preserve the useful tail of a deep parser path. The full path remains in
 * the heading tooltip. */
const BREADCRUMB_PARTS = 5;

export type FieldPick = { readonly path: readonly number[]; readonly startBit: number; readonly endBit: number };

type Row =
  | { readonly kind: "heading"; readonly depth: number; readonly text: string; readonly fullText: string; readonly key: string }
  | { readonly kind: "field"; readonly depth: number; readonly span: Span; readonly key: string };

/** An index belongs to the name before it rather than on a line of its own:
 *  `sections[0] type`, and `matrix[0][1]` for a list of lists.
 *
 *  A labelled index joins a plain name only. Two labelled ones run together
 *  as `[2] MTrk[35] pitch bend ch1`, which reads as one thing and is two.
 */
function trailParts(trail: readonly string[]): string[] {
  const out: string[] = [];
  for (const part of trail) {
    const last = out[out.length - 1];
    const bare = /^\[\d+\]$/.test(part);
    const labelled = !bare && /^\[\d+\] /.test(part);
    if (last !== undefined && (bare || (labelled && !last.includes("[")))) {
      out[out.length - 1] = last + part;
    } else if (last !== part) {
      // Pointer-heavy formats can pass through several IR wrappers all
      // called `object` or `body`. Repeating that label implies a hierarchy
      // the file itself does not have, so it is one logical level here.
      out.push(part);
    }
  }
  return out;
}

/** `0, 1, 4, 9 … 512 values`. The count alone tells a run of zeroes and a run
 *  of samples apart not at all, and the row is where the reader decides
 *  whether to open it. The sample never covers the whole run: a run is not
 *  collapsed until it is longer than the sample. */
function runText(s: Span): string {
  const count = countText(s.count, childWord(s));
  return s.sample.length === 0 ? count : `${s.sample.join(", ")} … ${count}`;
}

function samePrefix(a: readonly string[], b: readonly string[], n: number): boolean {
  for (let i = 0; i < n; i++) if (a[i] !== b[i]) return false;
  return true;
}

function printable(b: number): boolean {
  return b >= 0x20 && b < 0x7f;
}

/** Whether these bytes are worth showing as text as well as hex. One byte of
 *  a count is not: `41` beside an `A` invites reading a number as a letter.
 *  A run that is mostly letters usually is a name or a tag, and reading it
 *  off the row beats decoding it by eye. */
function readableAsText(bytes: Uint8Array): boolean {
  if (bytes.length < 3) return false;
  let n = 0;
  for (const b of bytes) if (printable(b)) n++;
  return n * 2 >= bytes.length;
}

/** A field's own bytes: hex and the same bytes as text, or the bits
 *  themselves when the field does not fill whole bytes. Printing the bytes
 *  around a three-bit field would show its neighbours' bits as its own. */
function fieldBytes(doc: Doc, s: Span): { hex: string; text: string } {
  const none = { hex: "", text: "" };
  if (s.size_bits === 0) return none;
  const whole = s.offset_bits % 8 === 0 && s.size_bits % 8 === 0;
  if (!whole && s.size_bits <= BYTES_SHOWN * 8) {
    const { bytes, complete } = doc.readBits(s.offset_bits, s.size_bits);
    if (!complete) return none;
    let bits = "";
    for (let i = 0; i < s.size_bits; i++) bits += ((bytes[i >> 3] ?? 0) >> (7 - (i % 8))) & 1 ? "1" : "0";
    return { hex: bits, text: "" };
  }
  const at = Math.floor(s.offset_bits / 8);
  const len = Math.ceil(((s.offset_bits % 8) + s.size_bits) / 8);
  const take = Math.min(len, BYTES_SHOWN);
  const { bytes, complete } = doc.read(at, take);
  if (!complete) return none;
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(" ");
  if (!readableAsText(bytes)) return { hex: len > take ? `${hex} …` : hex, text: "" };
  const text = Array.from(bytes, (b) => (printable(b) ? String.fromCharCode(b) : "·")).join("");
  return { hex: len > take ? `${hex} …` : hex, text: len > take ? `${text}…` : text };
}

export class ListingView {
  readonly el: HTMLElement;
  private readonly header: HTMLElement;
  private readonly rowsEl: HTMLElement;
  private readonly track: HTMLElement;
  private readonly thumb: HTMLElement;
  private readonly status: HTMLParagraphElement;

  /** First bit on screen. The listing scrolls the file, not a row list. */
  private topBit = 0;
  private visibleRows = 1;
  private rowHeight = 20;
  private selected: string | null = null;
  private dragging = false;
  /** A vertical touch drag. Horizontal movement remains the body's native
   * column scroll; `touch-action: pan-x` lets the browser choose between them. */
  private touch: { readonly id: number; y: number; remainder: number; moved: boolean } | null = null;
  private suppressClick = false;
  /** Explanation from the latest span request while a large variable-length
   * structure is being mapped. */
  private workStatus = "";
  /** Furthest physical byte reached by the latest unfinished mapping pass. */
  private workReachedBits = 0;
  /** Wheel input received while the evaluator is yielding. It is replayed
   * when the next slice completes instead of being silently discarded. */
  private pendingScrollRows = 0;
  /** Where the cursor went while this view was hidden. */
  private pendingBit: number | null = null;
  /** Whether the file's first bytes matched a template, which decides what an
   *  empty listing has to say for itself. */
  private matched = true;

  onPick: (pick: FieldPick) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("div");
    this.el.className = "listing";
    this.el.tabIndex = 0;
    this.el.setAttribute("role", "grid");
    this.el.setAttribute("aria-label", "Listing");

    this.header = document.createElement("div");
    this.header.className = "lv-header";
    this.rowsEl = document.createElement("div");
    this.rowsEl.className = "lv-rows";
    this.status = document.createElement("p");
    this.status.className = "lv-status";
    const body = document.createElement("div");
    body.className = "lv-body";
    body.append(this.header, this.rowsEl, this.status);

    this.track = document.createElement("div");
    this.track.className = "lv-track";
    this.track.setAttribute("aria-hidden", "true");
    this.thumb = document.createElement("div");
    this.thumb.className = "lv-thumb";
    this.track.append(this.thumb);
    this.el.append(body, this.track);

    new ResizeObserver(() => this.relayout()).observe(this.rowsEl);
    this.el.addEventListener("wheel", (e) => this.onWheel(e), { passive: false });
    this.el.addEventListener("keydown", (e) => this.onKey(e));
    this.rowsEl.addEventListener("click", (e) => this.onClick(e));
    this.el.addEventListener("pointerdown", (e) => this.onTouchDown(e));
    this.el.addEventListener("pointermove", (e) => this.onTouchMove(e));
    this.el.addEventListener("pointerup", (e) => this.onTouchUp(e));
    this.el.addEventListener("pointercancel", (e) => this.onTouchUp(e));
    this.track.addEventListener("pointerdown", (e) => this.onTrackDown(e));
    this.track.addEventListener("pointermove", (e) => this.onTrackMove(e));
    this.track.addEventListener("pointerup", (e) => this.onTrackUp(e));
    this.track.addEventListener("pointercancel", (e) => this.onTrackUp(e));
    doc.onChange(() => {
      if (this.pendingScrollRows !== 0) {
        const rows = this.pendingScrollRows;
        this.pendingScrollRows = 0;
        this.scrollBy(rows);
      } else {
        this.render();
      }
    });
  }

  relayout(): void {
    if (this.pendingBit !== null && !this.el.hidden) {
      const bit = this.pendingBit;
      this.pendingBit = null;
      this.setBit(bit);
    }
    // Headings are deliberately taller than data rows; the scroll unit remains
    // one field row so a touch or wheel step does not change speed at a section.
    const probe = this.rowsEl.querySelector<HTMLElement>(".lv-row:not(.lv-heading)");
    const h = probe?.getBoundingClientRect().height ?? 0;
    if (h > 0) this.rowHeight = h;
    this.visibleRows = Math.max(1, Math.floor(this.rowsEl.clientHeight / this.rowHeight));
    this.render();
  }

  /** Put the field at `path` on screen and select it. */
  reveal(path: readonly number[]): void {
    const key = path.join("/");
    this.selected = key;
    if (!this.rowsFrom(this.topBit, this.visibleRows).some((r) => r.kind === "field" && r.key === key)) {
      const n = this.doc.templateNode(path);
      if (n.status === "ok") this.topBit = n.node.offset_bits;
    }
    this.render();
  }

  /** Whether sniffing the file's first bytes found a template. */
  setMatched(matched: boolean): void {
    this.matched = matched;
    this.render();
  }

  clearSelection(): void {
    this.selected = null;
    this.render();
  }

  /** Bring the field covering `bit` on screen, leaving the selection alone.
   *  The hex view sends one of these per cursor move, so a hidden listing must
   *  not go looking through the file for a row nobody can see. */
  setBit(bit: number): void {
    if (this.el.hidden) {
      this.pendingBit = bit;
      return;
    }
    const rows = this.rowsFrom(this.topBit, this.visibleRows);
    const shown = rows.some((r) => r.kind === "field" && r.span.offset_bits <= bit && bit < r.span.offset_bits + r.span.size_bits);
    if (shown) return;
    this.topBit = bit;
    this.render();
  }

  // ----- rows -----

  /** Fields from `bit` onwards, with a heading wherever the structures they
   *  sit inside change. */
  private rowsFrom(bit: number, want: number): Row[] {
    const r = this.doc.spans(bit, this.doc.lengthBits, want);
    if (r.status !== "ok") {
      if (r.status === "working") {
        const reached = Math.min(this.doc.lengthBytes, r.reachedBytes);
        this.workReachedBits = reached * 8;
        const estimate = this.doc.extentEstimate();
        this.workStatus = estimate === null
          ? `Estimating field lengths… ${formatBytes(reached)} read so far`
          : `Estimating items… ${estimate.measured_items.toLocaleString()} of ${estimate.total_items.toLocaleString()} · ~${bitSizeText(estimate.estimated_bits)}`;
      } else if (r.status === "pending") {
        this.workReachedBits = r.reachedBytes * 8;
        this.workStatus = "Loading bytes needed to map these fields…";
      } else {
        this.workStatus = r.message;
      }
      return [];
    }
    this.workStatus = "";
    this.workReachedBits = 0;
    const rows: Row[] = [];
    let previous: string[] = [];
    for (const s of r.node) {
      const parts = trailParts(s.trail);
      // Entering several structures at once is one heading, not one per level:
      // five rows reading `sections[9]`, `body`, `entries[0]`, `body`, `code`
      // push the fields off the screen to say what one row can say.
      let from = 0;
      while (from < parts.length && samePrefix(parts, previous, from + 1)) from++;
      if (from < parts.length) {
        const changed = parts.slice(from);
        const compact = changed.length > BREADCRUMB_PARTS
          ? ["…", ...changed.slice(-(BREADCRUMB_PARTS - 1))]
          : changed;
        rows.push({
          kind: "heading",
          depth: Math.min(from, MAX_INDENT),
          text: `${from > MAX_INDENT && compact[0] !== "…" ? "… › " : ""}${compact.join(" › ")}`,
          fullText: parts.join(" › "),
          key: `h:${s.offset_bits}:${from}`,
        });
      }
      previous = parts;
      rows.push({ kind: "field", depth: Math.min(parts.length, MAX_INDENT), span: s, key: s.path.join("/") });
    }
    return rows;
  }

  private fieldsFrom(bit: number, want: number): Span[] {
    const r = this.doc.spans(bit, this.doc.lengthBits, want);
    return r.status === "ok" ? r.node : [];
  }

  /** The last `want` fields starting before `bit`, for scrolling back.
   *
   *  The window has to reach `bit`, not merely start before it: asking for a
   *  fixed number of fields from further back returns the first of them, and
   *  the last ones are the ones wanted. A field is at least a byte, so one per
   *  byte of the window is enough to cross it. */
  private fieldsBefore(bit: number, want: number): Span[] | null {
    for (let back = LOOK_BACK; ; back *= 2) {
      const from = Math.max(0, bit - back);
      const max = Math.min(SPAN_LIMIT, Math.ceil((bit - from) / 8) + want);
      const reply = this.doc.spans(from, this.doc.lengthBits, max);
      if (reply.status !== "ok") return null;
      const fields = reply.node.filter((s) => s.offset_bits < bit);
      if (fields.length >= want || from === 0 || back > LOOK_BACK_LIMIT) return fields.slice(-want);
    }
  }

  private scrollBy(rows: number): void {
    if (rows === 0) return;
    if (rows > 0) {
      // Both directions count fields rather than rows on screen, so a notch
      // down and a notch up land back where they started. Counting rows would
      // not: how many headings a screenful carries depends on where it starts,
      // which is not the same going the other way.
      const reply = this.doc.spans(this.topBit, this.doc.lengthBits, rows + 2);
      if (reply.status !== "ok") {
        this.pendingScrollRows = Math.min(SPAN_LIMIT, this.pendingScrollRows + rows);
        this.render();
        return;
      }
      const future = reply.node.filter((span) => span.offset_bits > this.topBit);
      const next = future[rows - 1] ?? future[future.length - 1];
      if (next !== undefined) {
        this.topBit = next.offset_bits;
      } else {
        // A collapsed run can be the only returned row and begin before the
        // current bit. Step past its represented extent so it cannot trap the
        // wheel on the same row forever.
        const covering = reply.node.find((span) =>
          span.offset_bits <= this.topBit && this.topBit < span.offset_bits + span.size_bits,
        );
        if (covering !== undefined) {
          this.topBit = Math.min(this.doc.lengthBits - 1, covering.offset_bits + covering.size_bits);
        }
      }
    } else {
      const back = this.fieldsBefore(this.topBit, -rows);
      if (back === null) {
        this.pendingScrollRows = Math.max(-SPAN_LIMIT, this.pendingScrollRows + rows);
        this.render();
        return;
      }
      this.topBit = back[0]?.offset_bits ?? 0;
    }
    this.render();
  }

  // ----- input -----

  private onWheel(e: WheelEvent): void {
    e.preventDefault();
    const rows = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? e.deltaY : e.deltaY / this.rowHeight;
    this.scrollBy(Math.trunc(rows) || Math.sign(rows));
  }

  private onKey(e: KeyboardEvent): void {
    const page = Math.max(1, this.visibleRows - 1);
    const by: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page };
    const move = by[e.key];
    if (move !== undefined) {
      e.preventDefault();
      this.scrollBy(move);
    } else if (e.key === "Home") {
      e.preventDefault();
      this.topBit = 0;
      this.render();
    }
  }

  private onClick(e: MouseEvent): void {
    if (this.suppressClick) {
      this.suppressClick = false;
      e.preventDefault();
      return;
    }
    const row = (e.target as HTMLElement).closest(".lv-row") as HTMLElement | null;
    const key = row?.dataset["path"];
    if (row === null || key === undefined) return;
    this.selected = key;
    this.render();
    const start = Number(row.dataset["start"] ?? 0);
    this.onPick({
      path: key === "" ? [] : key.split("/").map(Number),
      startBit: start,
      endBit: start + Number(row.dataset["size"] ?? 0),
    });
  }

  private onTouchDown(e: PointerEvent): void {
    if (e.pointerType !== "touch" || (e.target as HTMLElement).closest(".lv-track") !== null) return;
    this.touch = { id: e.pointerId, y: e.clientY, remainder: 0, moved: false };
    this.el.setPointerCapture(e.pointerId);
  }

  private onTouchMove(e: PointerEvent): void {
    const t = this.touch;
    if (t === null || t.id !== e.pointerId) return;
    const dy = e.clientY - t.y;
    t.y = e.clientY;
    t.remainder -= dy;
    if (Math.abs(t.remainder) < this.rowHeight) return;
    const rows = Math.trunc(t.remainder / this.rowHeight);
    t.remainder -= rows * this.rowHeight;
    t.moved = true;
    this.scrollBy(rows);
  }

  private onTouchUp(e: PointerEvent): void {
    const t = this.touch;
    if (t === null || t.id !== e.pointerId) return;
    this.suppressClick = t.moved;
    if (this.el.hasPointerCapture(e.pointerId)) this.el.releasePointerCapture(e.pointerId);
    this.touch = null;
  }

  private onTrackDown(e: PointerEvent): void {
    this.track.setPointerCapture(e.pointerId);
    this.dragging = true;
    this.seekTo(e.clientY);
  }
  private onTrackMove(e: PointerEvent): void {
    if (this.dragging) this.seekTo(e.clientY);
  }
  private onTrackUp(e: PointerEvent): void {
    if (!this.dragging) return;
    this.track.releasePointerCapture(e.pointerId);
    this.dragging = false;
  }
  private seekTo(clientY: number): void {
    const box = this.track.getBoundingClientRect();
    const at = Math.min(1, Math.max(0, (clientY - box.top) / Math.max(1, box.height)));
    this.topBit = Math.floor(at * this.doc.lengthBits);
    this.render();
  }

  // ----- drawing -----

  render(): void {
    if (this.doc.template === null) {
      this.rowsEl.replaceChildren();
      this.header.replaceChildren();
      this.status.textContent = this.matched ? NO_TEMPLATE_HINT : NO_TEMPLATE_MATCH;
      return;
    }
    if (this.header.childElementCount === 0) this.drawHeader();
    const rows = this.rowsFrom(this.topBit, this.visibleRows + OVERSCAN);
    if (rows.length === 0) {
      const r = this.doc.spans(this.topBit, this.doc.lengthBits, 1);
      this.status.textContent = this.workStatus || (r.status === "error" ? r.message : "");
    } else {
      this.status.textContent = "";
    }
    this.rowsEl.replaceChildren(...(rows.length === 0 && this.workStatus ? [this.estimateRow()] : rows.map((r) => this.rowEl(r))));
    this.drawThumb();
  }

  private estimateRow(): HTMLElement {
    const start = Math.min(this.doc.lengthBits, Math.max(this.topBit, this.workReachedBits));
    const el = document.createElement("div");
    el.className = "lv-row lv-estimate";
    const cells = COLUMNS.map(([cls]) => {
      const cell = document.createElement("span");
      cell.className = cls;
      return cell;
    });
    const [addr, length, bytes, name, value, type] = cells;
    if (addr) addr.textContent = formatOffset(start);
    if (length) length.textContent = bitSizeText(Math.max(0, this.doc.lengthBits - start));
    if (bytes) bytes.textContent = "…";
    if (name) name.textContent = "Structure remaining";
    if (value) value.textContent = this.workStatus;
    if (type) type.textContent = "estimating";
    el.append(...cells);
    return el;
  }

  private drawHeader(): void {
    for (const [cls, text] of COLUMNS) {
      const cell = document.createElement("span");
      cell.className = cls;
      cell.textContent = text;
      this.header.append(cell);
    }
  }

  private rowEl(r: Row): HTMLElement {
    const el = document.createElement("div");
    el.className = "lv-row";
    el.style.setProperty("--depth", String(r.depth));
    if (r.kind === "heading") {
      el.classList.add("lv-heading");
      if (r.depth === 0) el.classList.add("lv-major");
      const text = document.createElement("span");
      text.className = "lv-heading-text";
      text.textContent = r.text;
      text.title = r.fullText;
      el.append(text);
      return el;
    }

    const s = r.span;
    el.dataset["path"] = r.key;
    el.dataset["start"] = String(s.offset_bits);
    el.dataset["size"] = String(s.size_bits);
    if (r.key === this.selected) el.classList.add("is-selected");
    if (s.gap) el.classList.add("lv-gap");
    else el.classList.add(fieldClass(s.kind));

    const cells = COLUMNS.map(([cls]) => {
      const c = document.createElement("span");
      c.className = cls;
      return c;
    });
    const [addr, length, bytes, name, value, type] = cells as [
      HTMLElement,
      HTMLElement,
      HTMLElement,
      HTMLElement,
      HTMLElement,
      HTMLElement,
    ];
    addr.textContent = formatOffset(s.offset_bits);
    if (s.parts.length > 1) {
      length.classList.add("has-anatomy");
      const total = document.createElement("span");
      total.className = "length-total";
      total.textContent = bitSizeText(s.size_bits);
      length.append(total);
      appendAnatomy(
        length,
        s.parts.map((part) => ({
          sizeBits: part.size_bits,
          label: part.label,
          rest: part.rest,
        })),
        s.name,
      );
    } else {
      length.textContent = bitSizeText(s.size_bits);
    }
    const raw = fieldBytes(this.doc, s);
    const ascii = document.createElement("i");
    ascii.className = "lv-ascii";
    ascii.textContent = raw.text;
    bytes.append(raw.hex, ascii);
    if (s.line !== null) {
      // A structure that reads on one row has no name worth a column of its
      // own: the line is the whole of it, and takes the name's column too.
      // The name cell is left out rather than emptied, because an empty one
      // still holds column three and would push the line onto a second grid
      // row, where a row one line high shows half of each of them.
      el.classList.add("lv-inline");
      value.textContent = s.line;
      el.append(addr, length, bytes, value, type);
      return el;
    }
    if (s.gap) {
      name.textContent = GAP_LABEL;
      value.textContent = "";
      type.textContent = "undefined";
    } else {
      name.textContent = s.name;
      value.textContent = s.count > 0 ? runText(s) : s.value;
      type.textContent = s.type;
    }
    el.append(...cells);
    return el;
  }

  private drawThumb(): void {
    const trackH = this.track.clientHeight;
    const h = Math.max(24, Math.round(trackH * 0.06));
    const at = this.topBit / Math.max(1, this.doc.lengthBits);
    this.thumb.style.height = `${h}px`;
    this.thumb.style.transform = `translateY(${Math.round(at * Math.max(0, trackH - h))}px)`;
  }
}

const COLUMNS = [
  ["lv-addr", "Offset"],
  ["lv-length", "Length"],
  ["lv-bytes", "Bytes"],
  ["lv-name", "Field"],
  ["lv-value", "Value"],
  ["lv-type", "Type"],
] as const;

// One field's bytes, all of them, in a scrolling area of its own.
//
// A byte strip is a map: a column per field, a dozen bytes each, enough to see
// where things sit. When a field runs longer than that the strip says how much
// it is not drawing, and this is what opens behind that: sixteen bytes a line,
// address on the left and the text reading on the right, scrolled by line
// index the way the hex view scrolls the file.
//
// No cap and no ceiling. A line's top is `i * LINE`, so the bytes on screen
// are a division rather than a search and only those are read: four kilobytes
// of free space and four hundred megabytes of payload both open at once.

import { formatOffset } from "./doc.js";
import type { Doc } from "./doc.js";
import { REPORT } from "./strings.js";

/** Bytes on one line. Sixteen is what every hex dump does, and the reason is
 *  still good: the low digit of the address is the column. */
const PER_LINE = 16;
/** Height of one line, which must match `--bd-line` in the stylesheet: lines
 *  are placed by arithmetic on it. */
const LINE = 18;
/** Lines drawn above and below the window. */
const OVERSCAN = 4;
/** How tall the area is, in lines. Enough to read a sentence out of a string
 *  field without taking the listing's own screen. */
const VIEW_LINES = 10;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** A byte as the one character that stands for it, and a dot for the ones that
 *  stand for nothing. The hex view's own rule, so a run reads the same in
 *  both. */
function text(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".";
  return out;
}

/**
 * The whole of one field, scrolled.
 *
 * `at` and `len` are bytes, since a field opened this way is one whose bytes
 * are worth reading and those start on a boundary. The caller keeps the state
 * of whether this is open; this only draws it.
 */
export function byteDump(doc: Doc, at: number, len: number, name: string, scroll: { get: () => number; set: (top: number) => void }): HTMLElement {
  const host = el("div", "bdump");
  host.append(el("div", "bd-head", REPORT.dumpHead(name, len * 8)));
  const scroller = el("div", "bd-scroll");
  scroller.tabIndex = 0;
  const canvas = el("div", "bd-canvas");
  const lines = Math.ceil(len / PER_LINE);
  canvas.style.height = `${lines * LINE}px`;
  // Ten lines, or the field if it is shorter: a forty-byte string should not
  // sit in an area seven lines taller than itself.
  scroller.style.height = `${Math.min(VIEW_LINES, lines) * LINE}px`;
  scroller.append(canvas);
  host.append(scroller);

  let drawn: { from: number; to: number } | null = null;
  const paint = (): void => {
    // How many lines fit is this file's own constant rather than a measurement:
    // the area's height is set from it, and an element has no height until it
    // is in the document, which is one frame after the first draw.
    const view = Math.min(VIEW_LINES, lines);
    const first = Math.max(0, Math.floor(scroller.scrollTop / LINE) - OVERSCAN);
    const last = Math.min(lines, Math.ceil(scroller.scrollTop / LINE) + view + OVERSCAN);
    if (drawn !== null && drawn.from === first && drawn.to === last) return;
    drawn = { from: first, to: last };
    const out: HTMLElement[] = [];
    for (let i = first; i < last; i++) {
      const start = at + i * PER_LINE;
      const take = Math.min(PER_LINE, at + len - start);
      const line = el("div", "bd-line");
      line.style.top = `${i * LINE}px`;
      line.append(el("span", "bd-at", formatOffset(start * 8)));
      const { bytes, complete } = doc.read(start, take);
      if (!complete) {
        // Not an answer, so nothing is drawn as one. `Doc` has gone for the
        // chunk and the redraw when it lands asks again.
        line.append(el("span", "bd-hex", REPORT.reading));
        out.push(line);
        continue;
      }
      const run = bytes.subarray(0, take);
      line.append(el("span", "bd-hex", Array.from(run, (b) => b.toString(16).padStart(2, "0")).join(" ")));
      line.append(el("span", "bd-txt", text(run)));
      out.push(line);
    }
    canvas.replaceChildren(...out);
  };

  scroller.addEventListener("scroll", () => {
    scroll.set(scroller.scrollTop);
    paint();
  }, { passive: true });
  // The listing draws this again for anything that changes on screen, and a
  // dump that jumped back to its first line every time the selection moved
  // would be unreadable. Where the reader had it is theirs, not the element's.
  scroller.scrollTop = scroll.get();
  paint();
  return host;
}

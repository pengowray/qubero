// What one cell of the grid shows: the characters in it and every mark on it.
//
// A redraw touches six hundred cells and usually changes two of them, so what
// a cell wants is gathered here as strings and the caller writes them back
// only where they differ. A class written back unchanged still costs the
// browser the styling of that cell.
//
// `.ts` rather than the `.js` the rest of `src` writes: the tests run this
// file under `node --test`, which strips the types but does not rewrite a
// `.js` specifier back to the file it came from.
import { fieldClass } from "./fieldstyle.ts";

/** Write a cell's characters, unless they are already the ones it shows.
 *  Scrolling changes every one of them and a cursor key changes none, and the
 *  browser charges for a write either way. */
export function setText(el: HTMLElement, text: string): void {
  if (el.textContent !== text) el.textContent = text;
}

/** The part of one byte a run covers, as bit positions 0 to 8 counting from the
 *  top of the byte. */
export type Run = { from: number; to: number };

/** A run of bits, `[startBit, endBit)`. */
export type Bits = { readonly startBit: number; readonly endBit: number };

/** Whether the runs together cover every bit from `from` to `to`. */
export function covers(runs: readonly Run[], from: number, to: number): boolean {
  let at = from;
  for (const r of runs) {
    if (r.from > at) return false;
    at = Math.max(at, r.to);
  }
  return at >= to;
}

/**
 * Which bits of byte `off` the highlight covers, as [from, to) runs within
 * 0..8, in order and not touching. Empty where the byte is not covered.
 *
 * A run of no bits is kept rather than dropped: a field of no length still
 * has a place, and marking the byte it sits in front of would say it covers
 * that byte, which it does not.
 */
export function highlightBits(highlight: readonly Bits[], off: number): Run[] {
  const out: Run[] = [];
  for (const h of highlight) {
    const from = Math.max(h.startBit, off * 8) - off * 8;
    const to = Math.min(h.endBit, off * 8 + 8) - off * 8;
    if (to < from || from > 8 || to < 0) continue;
    // An empty run belongs to the byte it starts in, and to that byte only,
    // so the one past the end of a previous byte is not counted twice.
    if (to === from && (from === 8 || h.endBit !== h.startBit)) continue;
    out.push({ from, to });
  }
  if (out.length < 2) return out;
  out.sort((a, b) => a.from - b.from || a.to - b.to);
  const merged: Run[] = [];
  for (const r of out) {
    const last = merged[merged.length - 1];
    // Two empty runs at the same place are one mark, not two.
    if (last !== undefined && r.from <= last.to) last.to = Math.max(last.to, r.to);
    else merged.push(r);
  }
  return merged;
}

/**
 * The part of byte `off` the selection covers, as one [from, to) run within
 * 0..8, or null.
 *
 * Asked per byte on screen, so a selection of a whole four gigabyte file
 * costs what a selection of one row costs.
 */
export function selectionBits(sel: Bits, off: number): Run | null {
  const from = Math.max(sel.startBit, off * 8) - off * 8;
  const to = Math.min(sel.endBit, off * 8 + 8) - off * 8;
  return to > from && from < 8 && to > 0 ? { from, to } : null;
}

/** Mark part of a byte in hex mode: a bar under the bits the field covers,
 *  one length of bar per run, or a tick where a run has no bits. The gradient
 *  the caller paints the cell with, or "" for no mark. */
export function markBits(runs: readonly Run[]): string {
  // The cell is 3ch wide: half a character of padding, two digits, half again.
  const pad = 100 / 6;
  const step = (100 - 2 * pad) / 8;
  const stops: string[] = [];
  let at = 0;
  for (const r of runs) {
    // A run of no bits still shows, as a mark a fraction of a bit wide, so
    // that a field of no length is visible where it sits.
    const from = pad + r.from * step;
    const to = pad + Math.max(r.to, r.from + 0.15) * step;
    stops.push(`transparent ${at}%`, `transparent ${from}%`, `var(--accent) ${from}%`, `var(--accent) ${to}%`);
    at = to;
  }
  if (stops.length === 0) return "";
  stops.push(`transparent ${at}%`, "transparent 100%");
  return `linear-gradient(to right, ${stops.join(", ")})`;
}

/** The span covering a byte, as much of it as a cell shows. Null where no
 *  field covers the byte, and for a gap, which is not tinted. */
export type CellSpan = { readonly kind: string; readonly startsHere: boolean };

export type CellInput = {
  /** The byte this cell stands for, and the length of the file. */
  readonly off: number;
  readonly len: number;
  readonly binary: boolean;
  /** False while the bytes are still on their way. */
  readonly complete: boolean;
  /** The byte itself, when it has arrived. */
  readonly byte: number;
  readonly span: CellSpan | null;
  /** The bits of this byte the active field covers. */
  readonly hl: readonly Run[];
  /** The bits of it the user's selection covers. */
  readonly sel: Run | null;
  /** The stretch another tab is looking at, or null. */
  readonly link: Bits | null;
  readonly cursor: number;
  readonly pane: "hex" | "ascii";
  readonly nibble: 0 | 1;
  readonly insertMode: boolean;
};

export type CellDraw = {
  /** The classes the bytes cell wants. */
  readonly hex: string;
  /** The classes the text cell wants. */
  readonly ascii: string;
  /** What the bytes cell says, or null where the bits inside it say it: in
   *  binary the eight bits are written one at a time. */
  readonly hexText: string | null;
  readonly asciiText: string;
  /** The gradient marking part of a byte, or "" for no mark. */
  readonly bits: string;
};

export function asciiGlyph(b: number): string {
  return b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "·";
}

const HEX = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

/** What a cell shows and every mark on it, from the byte it stands for and
 *  what the view is doing. Nothing here touches the document. */
export function cellDraw(c: CellInput): CellDraw {
  const { off, len, binary, complete } = c;
  let hc = "";
  let ac = "";
  let bits = "";
  let text = binary ? "        " : "  ";
  let asciiText: string;
  if (off < len) {
    const b = c.byte;
    asciiText = complete ? asciiGlyph(b) : " ";
    if (complete && !(b >= 0x20 && b < 0x7f)) ac += " hv-np";
    if (!binary) {
      text = complete ? HEX[b] ?? "" : "··";
      if (!complete) hc += " hv-pending";
    }
  } else {
    asciiText = " ";
    if (off === len) hc += " hv-end";
  }
  const hexText = !binary || off >= len ? text : null;

  if (c.span !== null) {
    hc += ` hv-tint ${fieldClass(c.span.kind)}`;
    if (c.span.startsHere) hc += " hv-field-start";
  }
  if (c.hl.length > 0) {
    // The text column cannot show part of a byte, so a partly covered
    // byte is marked more faintly there than a fully covered one, and a
    // run of no bits is not marked there at all: one character standing
    // for a whole byte cannot say "between two of these".
    const whole = covers(c.hl, 0, 8);
    const any = c.hl.some((r) => r.to > r.from);
    if (any) ac += whole ? " hv-hl" : " hv-hl-weak";
    if (!binary && off < len) {
      if (whole) hc += " hv-hl";
      else {
        bits = markBits(c.hl);
        if (bits !== "") hc += " hv-hlbits";
      }
    }
  }
  const sb = c.sel;
  if (sb !== null) {
    // A byte only partly selected is marked weakly in both columns: two
    // hex digits and one text character each stand for the whole byte,
    // and a full mark would say the whole byte is in.
    const whole = sb.from <= 0 && sb.to >= 8;
    if (!binary && off < len) hc += whole && c.pane === "hex" ? " hv-sel" : " hv-sel-weak";
    ac += whole && c.pane === "ascii" ? " hv-sel" : " hv-sel-weak";
  }
  const link = c.link;
  // The linked stretch is marked by the byte, not by the bit: it stands
  // for a place in another document, and half a byte of one is not a
  // finer answer, only a smaller one.
  if (link !== null && off * 8 < link.endBit && (off + 1) * 8 > link.startBit) {
    hc += " hv-linked";
    ac += " hv-linked";
    // The ends of the run get the ends of the outline, so a mark that
    // runs off a row still reads as one stretch rather than as a box per
    // byte.
    if (off * 8 <= link.startBit) {
      hc += " hv-linked-first";
      ac += " hv-linked-first";
    }
    if ((off + 1) * 8 >= link.endBit) {
      hc += " hv-linked-last";
      ac += " hv-linked-last";
    }
  }
  if (off === c.cursor) {
    // In binary the bits carry the cursor, except past the end of the
    // file where there are no bits to carry it.
    if (!binary || off >= len) {
      hc += c.pane === "hex" ? " hv-cur hv-focus" : " hv-cur hv-dim";
      if (!binary && c.pane === "hex" && c.nibble === 1) hc += " hv-nib1";
      if (c.insertMode) hc += " hv-ins";
    }
    ac += c.pane === "ascii" ? " hv-cur hv-focus" : " hv-cur hv-dim";
    if (c.insertMode) ac += " hv-ins";
  }
  return { hex: hc, ascii: ac, hexText, asciiText, bits };
}

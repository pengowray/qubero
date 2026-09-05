// The buttons the annotation column is made of, and how a block of them is
// filled.
//
// What each one says is decided in `chipplan.ts`; this is only the writing of
// it. Every chip here is reused rather than rebuilt: a redraw usually wants
// the very chips that are already there, and on a touch screen a finger may be
// resting on one, which the browser reads as the touch being cancelled if the
// element leaves the document, stopping the drag that is scrolling the view.
//
// `.ts` rather than the `.js` the rest of `src` writes: the tests run these
// files under `node --test`, which strips the types but does not rewrite a
// `.js` specifier back to the file it came from.
import { fieldClass } from "./fieldstyle.ts";
import { chipDetail, type ChipMeasure } from "./chipfit.ts";
import { chipText, continuedDetail, type Chip, type ChipBlock, type ChipText } from "./chipplan.ts";
import { setText } from "./hexcell.ts";

/** How many entries one screenful of the annotation column may hold. */
export const SPAN_LIMIT = 600;

/** The table of values at the end of a block, if the row has one. */
export function valsOf(el: HTMLElement): HTMLElement | null {
  const last = el.lastElementChild;
  return last instanceof HTMLElement && last.classList.contains("hv-vals") ? last : null;
}

/** The chips of a block, which is everything in it but that table. */
export function chipsOf(el: HTMLElement): ChipEl[] {
  return [...el.children].filter((c): c is ChipEl => c.classList.contains("hv-chip"));
}

/** A chip element, with the path of the field it stands for kept on it, so one
 *  click handler serves the button for as long as the button lives. */
export type ChipEl = HTMLButtonElement & { _path?: readonly number[] | undefined };

/**
 * An empty chip, ready to be filled.
 *
 * The click handler is attached once and reads the field off the element, so
 * that filling a chip again does not mean making one again. Keeping the same
 * button matters beyond the cost: on a touch screen a finger may be resting
 * on it, and taking the element out from under a finger is read as the touch
 * being cancelled, which stops the drag that is scrolling the view.
 */
export function newChip(onPick: (path: readonly number[]) => void): ChipEl {
  const el = document.createElement("button") as ChipEl;
  el.type = "button";
  el.className = "hv-chip";
  const nameEl = document.createElement("b");
  const v = document.createElement("span");
  v.className = "hv-chip-val";
  el.append(nameEl, v);
  el.addEventListener("click", (e) => {
    e.stopPropagation();
    const path = el._path;
    if (path !== undefined) onPick(path);
  });
  return el;
}

/** A chip that is not a field: the count of what did not fit, or a note about
 *  the column itself. */
export function fillPlain(el: ChipEl, cls: string, text: string, title: string): void {
  const className = `hv-chip hv-chip-gap ${cls}`.trim();
  if (el.className !== className) el.className = className;
  setText(el.firstElementChild as HTMLElement, text);
  setText(el.lastElementChild as HTMLElement, "");
  if (el.title !== title) el.title = title;
  el._path = undefined;
  el.disabled = true;
  el.removeAttribute("aria-label");
}

/**
 * One entry in the annotation column, coloured to match its bytes.
 *
 * `extra` marks a chip drawn above the bytes it names: it shows that the field
 * runs on through them, and only what the chip shows changes — the title and
 * the aria-label already say it in words.
 */
export function fillChip(el: ChipEl, c: Chip, text: ChipText, extra = false): void {
  const s = c.span;
  const { name, detail } = text;
  let cls = "hv-chip";
  if (s.gap) cls += " hv-chip-gap";
  else cls += ` ${fieldClass(s.kind)}`;
  if (c.carried) cls += " hv-chip-carried";
  if (el.className !== cls) el.className = cls;
  setText(el.firstElementChild as HTMLElement, name);
  const shown = extra ? continuedDetail(detail) : detail;
  setText(el.lastElementChild as HTMLElement, shown);
  const path = [...s.trail, s.name].join(" ");
  let title: string;
  let label: string | null = null;
  if (c.run.length > 0) {
    // The first few, by number and value, so the reader can see what kind
    // of thing the run is without opening it.
    const first = c.run.slice(0, 6).map((e) => `${e.name} ${chipDetail(e)}`);
    if (c.run.length > first.length) first.push("…");
    title = `${s.trail.join(" ")} · ${s.type} · ${detail}: ${first.join(", ")}`;
  } else if (s.gap) {
    title = `No field covers these ${detail}. Inside: ${path}`;
  } else if (c.carried) {
    // The arrow says "this began further up", which a screen reader cannot
    // see and a first-time reader should not have to work out.
    title = `Starts above the visible rows: ${path}, ${detail}`;
    label = `starts above: ${name}, ${detail}`;
  } else {
    title = `${path} · ${s.type}`;
  }
  if (el.title !== title) el.title = title;
  if (label === null) el.removeAttribute("aria-label");
  else el.setAttribute("aria-label", label);
  el._path = s.gap ? undefined : s.path;
  el.disabled = s.gap;
}

/**
 * Put the chips a block wants into it, reusing the elements already there and
 * only adding or dropping one when the count changes.
 *
 * `tail` adds the note that the column has stopped listing fields, which
 * belongs to the last block on the last row rather than to any one field.
 */
export function fillNote(
  el: HTMLElement,
  b: ChipBlock | null,
  continued: boolean,
  tail: boolean,
  onPick: (path: readonly number[]) => void,
): void {
  const n = b === null ? 0 : b.shown;
  const rest = b !== null && b.shown < b.entries.length;
  const want = n + (rest ? 1 : 0) + (tail ? 1 : 0);
  // The table of a folded run's values is the last thing in the block and is
  // not a chip: it is written by `valuecells.ts` and only counted here, so
  // that a chip added or hidden does not land on top of it.
  const vals = valsOf(el);
  const chips = el.childElementCount - (vals === null ? 0 : 1);
  for (let i = chips; i < want; i++) el.insertBefore(newChip(onPick), vals);
  // Spare chips are hidden rather than taken away. A row that scrolls into
  // one with fewer fields would otherwise drop the element a finger is
  // resting on, and a touch whose element leaves the document is one the
  // browser calls off, which stops the drag that is scrolling the view.
  for (let i = 0; i < Math.max(chips, want); i++) {
    const c = el.children[i] as HTMLElement;
    if (c.hidden !== i >= want) c.hidden = i >= want;
  }
  // A block with nothing showing takes no room, the same as one with no
  // children. Only above and below the bytes: the column beside them holds
  // its width whether or not this row has a field.
  el.classList.toggle("hv-empty", want === 0 && (vals === null || vals.classList.contains("hv-empty")));
  for (let i = 0; i < n && b !== null; i++) {
    fillChip(el.children[i] as ChipEl, b.entries[i] as Chip, b.texts[i] as ChipText, continued);
  }
  let at = n;
  if (rest && b !== null) {
    const left = b.entries.slice(b.shown);
    const named = left.slice(0, 8).map((c) => chipText(c).name);
    if (left.length > named.length) named.push("…");
    const what = left.length === 1 ? "field starts" : "fields start";
    fillPlain(
      el.children[at] as ChipEl,
      "hv-chip-rest",
      `+${left.length}`,
      `${left.length} more ${what} on this row: ${named.join(", ")}`,
    );
    at++;
  }
  if (tail) {
    fillPlain(
      el.children[at] as ChipEl,
      "",
      "more fields below",
      `The field column shows up to ${SPAN_LIMIT} fields at a time. Scroll down to see the rest.`,
    );
  }
}

/**
 * How wide the chips' own text is drawn, read off a chip that has been. A
 * chip's name is bold sans and its value mono, so counting characters at
 * one width was wrong for both, and wrong by enough to predict three lines
 * where the browser drew four.
 *
 * Null until a chip exists to read a font from; the caller keeps the
 * character count until then and draws once more when this arrives.
 */
export function readChipFonts(root: HTMLElement): ChipMeasure | null {
  const nameEl = root.querySelector(".hv-chip > b") as HTMLElement | null;
  if (nameEl === null) return null;
  const valEl = root.querySelector(".hv-chip-val") as HTMLElement | null;
  const ctx = document.createElement("canvas").getContext("2d");
  if (ctx === null) return null;
  // Built from the longhands: what `font` computes to is not something every
  // browser will hand back whole.
  const font = (el: HTMLElement): string => {
    const s = getComputedStyle(el);
    return `${s.fontStyle} ${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;
  };
  const nameFont = font(nameEl);
  const valFont = valEl === null ? nameFont : font(valEl);
  const width = (f: string, s: string): number => {
    ctx.font = f;
    return ctx.measureText(s).width;
  };
  return { name: (s) => width(nameFont, s), value: (s) => width(valFont, s) };
}

// A few helpers, shared: build an element, set its properties, append its
// children. Enough of the DOM to write a toolbar with, and no framework.
//
// The three little ones below are the shapes the rail's panels build over and
// over. They live here rather than in one of those panels because two panels
// that each grew their own would drift, and a line saying "nothing here" that
// is grey in one column and not in the next is a difference a reader has to
// account for.

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  Object.assign(e, props);
  e.append(...children);
  return e;
}

/**
 * An address whose leading `+` says what it is counted from, with the answer
 * on the mark itself.
 *
 * The `+` is written in front of two different kinds of address in this app:
 * one counted from the front of an unpacked stream, and one counted from the
 * front of an enclosing structure. Both can be on the inspector at once, and
 * written down they are the same glyph, so a reader who learns one meaning
 * reads the other wrong. What both have in common is that the number is added
 * to something, and the only thing missing is what. So the mark carries it:
 * hover the `+` and it says what the offset is from.
 *
 * The mark is a separate element for that reason alone. A title on the whole
 * address answers a question about the address; this one is about the sign.
 * An address with no `+` is returned as it is, since nothing is being added.
 */
export function address(text: string, from: string): (Node | string)[] {
  return addressParts(text).map((p) => (p.mark ? el("span", { className: "addr-from", title: from }, p.text) : p.text));
}

/**
 * Which pieces of an address are the mark and which are the number.
 *
 * Split out from `address` so the rule can be read and tested without a
 * document, since the rule is the part that can be wrong. A `+` is the mark
 * only where an address begins: at the front of the text, or after a space,
 * which is what the second address of a range comes after. Both ends of
 * `+0x13 to +0x17` count from the same place, so both are marked.
 *
 * The `+` this must not touch is the one inside an address. `formatOffset`
 * writes a sub-byte address as `0x69+7b`, where the plus means seven bits past
 * the byte: a third meaning of the glyph, and one this app cannot give up,
 * since bit addresses are most of what it is for. A mark that promises to say
 * what an offset is added to must not sit on a plus it would answer wrongly.
 */
export function addressParts(text: string): { readonly text: string; readonly mark: boolean }[] {
  const out: { text: string; mark: boolean }[] = [];
  let at = 0;
  for (let i = text.indexOf("+"); i >= 0; i = text.indexOf("+", i + 1)) {
    if (i !== 0 && text[i - 1] !== " ") continue;
    if (i > at) out.push({ text: text.slice(at, i), mark: false });
    out.push({ text: "+", mark: true });
    at = i + 1;
  }
  if (at < text.length) out.push({ text: text.slice(at), mark: false });
  return out;
}

/** One row of a `dl` of facts: the name and the value, as the pair a `dl`
 *  wants, for spreading into a `replaceChildren`. */
export function factRow(key: string, value: string): HTMLElement[] {
  const dt = document.createElement("dt");
  dt.textContent = key;
  const dd = document.createElement("dd");
  dd.textContent = value;
  return [dt, dd];
}

/** A line standing in for a list that has nothing in it, or not yet. */
export function noneLine(text: string): HTMLElement {
  const p = document.createElement("p");
  p.className = "ov-none";
  p.textContent = text;
  return p;
}

/** A standing remark above a list, about the list rather than in it. */
export function noteLine(text: string): HTMLElement {
  const p = document.createElement("p");
  p.className = "ov-note";
  p.textContent = text;
  return p;
}

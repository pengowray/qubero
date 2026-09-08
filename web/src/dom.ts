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

// The pieces every format card is built from: finding a structure's fields
// while their bytes may still be on the way, and drawing the facts, tables and
// controls a card is made of so that each one is a way back to its field.
//
// Shared by the JPEG cards (`jpegcards.ts`) and the PNG ones (`pngcards.ts`),
// and by any card after them, so that a fact on one card behaves as a fact on
// another: the same hover, the same mark when the cursor is in its field, the
// same click to go to it. Which cards a format has is `formatcards.ts`.

import { formatOffset } from "./doc.ts";
import type { Doc, TemplateNode } from "./doc.ts";
import { tagged as el } from "./dom.ts";
import { pathKey } from "./flatten.ts";
import type { DrawContext } from "./listingdraw.ts";
import { bitSizeText, REPORT } from "./strings.ts";

// ----- reading -----

/** One child of a node, or null while its bytes are still on their way. */
export function kid(doc: Doc, node: TemplateNode, at: number): TemplateNode | null {
  const reply = doc.templateChildren(node.path, at, at + 1);
  return reply.status === "ok" ? (reply.node[0] ?? null) : null;
}

/** A run of a node's children, or null while they are being read. */
export function kids(doc: Doc, node: TemplateNode, count = node.child_count): readonly TemplateNode[] | null {
  if (count === 0) return [];
  const reply = doc.templateChildren(node.path, 0, count);
  return reply.status === "ok" ? reply.node : null;
}

/** What an enumerated field is called, without the raw number the listing's
 *  value column carries: `edit_text` is the name on its own. */
export function enumName(node: TemplateNode | null): string {
  return node === null ? "" : node.edit_text;
}

/** A field's value as a number, or NaN where it is not one. */
export function numberOf(node: TemplateNode | null): number {
  const n = node === null ? Number.NaN : Number(node.value);
  return Number.isFinite(n) ? n : Number.NaN;
}

// ----- drawing -----

/** A table's own line inside a card: what it is, and the one fact worth
 *  putting beside the name. */
export function head(name: string, aside: string): HTMLElement {
  const line = el("div", "jc-head");
  line.append(el("b", "jc-title", name));
  if (aside !== "") line.append(el("span", "jc-aside", aside));
  return line;
}

export function table(columns: readonly string[]): HTMLTableElement {
  const grid = document.createElement("table");
  grid.className = "rec jc-table";
  const header = document.createElement("tr");
  for (const name of columns) header.append(el("th", "", name));
  header.append(el("th", "rec-at", REPORT.storedAt));
  grid.append(header);
  return grid;
}

/** One line of a card's table, standing for one structure in the file: the
 *  same shape a record table's line has, so the same code marks it when the
 *  selection lands inside it. */
export function row(c: DrawContext, node: TemplateNode, cells: readonly (TemplateNode | undefined)[], texts: readonly (string | undefined)[] = []): HTMLTableRowElement {
  const tr = document.createElement("tr");
  tr.dataset["at"] = String(node.offset_bits);
  tr.dataset["size"] = String(node.size_bits);
  cells.forEach((field, i) => tr.append(cell(c, field, texts[i])));
  const at = el("td", "rec-at");
  const link = el("button", "rec-link", formatOffset(node.offset_bits));
  link.type = "button";
  link.title = bitSizeText(node.size_bits);
  const key = `r:${pathKey(node.path)}`;
  if (c.bytes.has(key)) link.classList.add("is-on");
  link.addEventListener("click", (e) => {
    e.stopPropagation();
    c.toggleBytes(key);
  });
  at.append(link);
  tr.append(at);
  return tr;
}

/** One cell of a card's table, which is one field of the file. */
function cell(c: DrawContext, node: TemplateNode | undefined, text?: string): HTMLElement {
  const td = el("td", "");
  if (node === undefined) return td;
  td.append(fieldButton(c, node, "jc-value", text ?? node.value));
  return td;
}

/**
 * Anything on a card that stands for one field, as the control that goes to
 * it. `data-reads` is the route every cross-reference in the listing already
 * takes, so a click here selects the field and every other view follows,
 * exactly as clicking a cell of a record table does.
 */
export function fieldButton(c: DrawContext, node: TemplateNode, className: string, text: string): HTMLButtonElement {
  const b = el("button", className, text);
  b.type = "button";
  b.dataset["reads"] = pathKey(node.path);
  b.dataset["at"] = String(node.offset_bits);
  b.dataset["size"] = String(node.size_bits);
  b.title = `${node.name} · ${formatOffset(node.offset_bits)}`;
  if (c.selected !== null && c.selected.offsetBits === node.offset_bits && c.selected.sizeBits === node.size_bits) {
    b.classList.add("is-on");
  }
  return b;
}

/** The control that opens the long half of a card and puts it away again.
 *  The card is measured rather than declared, so opening one has to tell the
 *  listing to lay itself out again, which is what the callback is for. */
export function disclosure(c: DrawContext, key: string, label: string): HTMLElement {
  const on = c.cards.has(key);
  const b = el("button", `jc-more${on ? " is-on" : ""}`, label);
  b.type = "button";
  b.setAttribute("aria-expanded", String(on));
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    c.toggleCard(key);
  });
  return b;
}

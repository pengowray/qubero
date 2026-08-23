// Tree table of the template's fields with live values. Only expanded nodes are
// evaluated; long arrays are paged so a 100k-element array costs nothing until
// the user opens it.

import type { Doc, TemplateNode } from "./doc.js";

const PAGE = 200;

export type FieldPick = { readonly startByte: number; readonly endByte: number };

function key(path: readonly number[]): string {
  return path.join("/");
}

function hexOffset(bits: number): string {
  const byte = Math.floor(bits / 8);
  const rem = bits % 8;
  return `0x${byte.toString(16).toUpperCase()}${rem ? `+${rem}b` : ""}`;
}

function countText(n: number, noun: string): string {
  return `${n.toLocaleString()} ${noun}${n === 1 ? "" : "s"}`;
}

function sizeText(bits: number): string {
  if (bits % 8 === 0) {
    const b = bits / 8;
    return b === 1 ? "1 byte" : `${b.toLocaleString()} bytes`;
  }
  return bits === 1 ? "1 bit" : `${bits.toLocaleString()} bits`;
}

export class TypeTable {
  readonly el: HTMLElement;
  private readonly body: HTMLElement;
  private readonly empty: HTMLElement;
  private readonly expanded = new Set<string>();
  private readonly shown = new Map<string, number>();
  private selected: string | null = null;

  onPick: (pick: FieldPick) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "typetable";
    this.el.setAttribute("aria-label", "Fields");

    const table = document.createElement("table");
    const head = document.createElement("thead");
    head.innerHTML = "<tr><th>Field</th><th>Value</th><th>Type</th><th>Offset</th><th>Size</th></tr>";
    this.body = document.createElement("tbody");
    table.append(head, this.body);
    this.empty = document.createElement("p");
    this.empty.className = "tt-empty";
    this.el.append(this.empty, table);
    this.expanded.add("");
    this.body.addEventListener("click", (e) => this.onClick(e));
    doc.onChange(() => this.render());
  }

  private onClick(e: MouseEvent): void {
    const t = e.target;
    if (!(t instanceof HTMLElement)) return;
    const more = t.closest<HTMLElement>("[data-more]");
    if (more) {
      const k = more.dataset["more"] ?? "";
      this.shown.set(k, (this.shown.get(k) ?? PAGE) + PAGE);
      this.render();
      return;
    }
    const row = t.closest<HTMLElement>("tr[data-path]");
    if (!row) return;
    const k = row.dataset["path"] ?? "";
    if (t.closest(".tt-toggle")) {
      if (this.expanded.has(k)) this.expanded.delete(k);
      else this.expanded.add(k);
      this.render();
      return;
    }
    const start = Number(row.dataset["start"]);
    const end = Number(row.dataset["end"]);
    this.selected = k;
    this.render();
    this.onPick({ startByte: start, endByte: end });
  }

  render(): void {
    if (this.doc.template === null) {
      this.empty.textContent = "No template selected. Pick one above to see the file's fields.";
      this.empty.hidden = false;
      this.body.replaceChildren();
      return;
    }
    this.empty.hidden = true;
    const frag = document.createDocumentFragment();
    const root = this.doc.templateNode([]);
    if (root.status === "ok") this.addRows(frag, root.node, 0);
    else this.addStatusRow(frag, root, 0, "file");
    this.body.replaceChildren(frag);
  }

  private addStatusRow(frag: DocumentFragment, r: { status: "pending" } | { status: "error"; message: string }, depth: number, what: string): void {
    const tr = document.createElement("tr");
    tr.className = r.status === "pending" ? "tt-pending" : "tt-error";
    const td = document.createElement("td");
    td.colSpan = 5;
    td.style.paddingLeft = `${depth * 16 + 8}px`;
    td.textContent = r.status === "pending" ? `Loading ${what}` : `${what}: ${r.message}`;
    tr.append(td);
    frag.append(tr);
  }

  private addRows(frag: DocumentFragment, n: TemplateNode, depth: number): void {
    const k = key(n.path);
    const tr = document.createElement("tr");
    tr.dataset["path"] = k;
    tr.dataset["start"] = String(Math.floor(n.offset_bits / 8));
    tr.dataset["end"] = String(Math.ceil((n.offset_bits + n.size_bits) / 8));
    if (k === this.selected) tr.classList.add("tt-selected");
    if (!n.ok) tr.classList.add("tt-bad");

    const name = document.createElement("td");
    name.style.paddingLeft = `${depth * 16 + 4}px`;
    const open = this.expanded.has(k);
    if (n.composite) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "tt-toggle";
      b.textContent = open ? "▾" : "▸";
      b.setAttribute("aria-label", open ? "Collapse" : "Expand");
      b.setAttribute("aria-expanded", String(open));
      name.append(b);
    } else {
      const spacer = document.createElement("span");
      spacer.className = "tt-toggle tt-leaf";
      name.append(spacer);
    }
    name.append(document.createTextNode(n.name));

    const value = document.createElement("td");
    value.className = `tt-val tt-${n.kind}`;
    value.textContent = n.composite ? countText(n.child_count, n.type.endsWith("[]") ? "item" : "field") : n.value;

    const type = document.createElement("td");
    type.className = "tt-type";
    type.textContent = n.type;
    const off = document.createElement("td");
    off.className = "tt-num";
    off.textContent = hexOffset(n.offset_bits);
    const size = document.createElement("td");
    size.className = "tt-num";
    size.textContent = sizeText(n.size_bits);
    tr.append(name, value, type, off, size);
    frag.append(tr);

    if (n.composite && open) {
      const limit = this.shown.get(k) ?? PAGE;
      const kids = this.doc.templateChildren(n.path, 0, limit);
      if (kids.status !== "ok") {
        this.addStatusRow(frag, kids, depth + 1, n.name);
        return;
      }
      for (const c of kids.node) this.addRows(frag, c, depth + 1);
      if (n.child_count > limit) {
        const tr2 = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 5;
        td.style.paddingLeft = `${(depth + 1) * 16 + 8}px`;
        const b = document.createElement("button");
        b.type = "button";
        b.dataset["more"] = k;
        b.className = "tt-more";
        b.textContent = `Show ${Math.min(PAGE, n.child_count - limit).toLocaleString()} more (${(n.child_count - limit).toLocaleString()} hidden)`;
        td.append(b);
        tr2.append(td);
        frag.append(tr2);
      }
    }
  }
}

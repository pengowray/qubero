// Value inspector: interprets the bytes under the cursor as common primitive
// types and writes edits back. Every row is a small two-way lens: decode bytes
// to text, parse text to bytes.
//
// The cursor is a bit position, so these readings start wherever it is: put the
// cursor three bits into a byte and the rows show what a u16 there would say.

import { formatOffset } from "./doc.js";
import type { Doc, TemplateNode } from "./doc.js";

/** Structure reads the template's field; the other two read raw bytes. */
type Mode = "structure" | "le" | "be";

type Lens = {
  readonly label: string;
  readonly size: number;
  readonly decode: (v: DataView, le: boolean) => string;
  /** Returns null when the text is not a valid value for this type. */
  readonly encode: (text: string, le: boolean) => Uint8Array | null;
};

function f16ToNumber(h: number): number {
  const s = h >> 15 ? -1 : 1;
  const e = (h >> 10) & 0x1f;
  const f = h & 0x3ff;
  if (e === 0) return s * Math.pow(2, -14) * (f / 1024);
  if (e === 0x1f) return f ? NaN : s * Infinity;
  return s * Math.pow(2, e - 15) * (1 + f / 1024);
}

function numberToF16(x: number): number {
  if (Number.isNaN(x)) return 0x7e00;
  const sign = x < 0 || Object.is(x, -0) ? 0x8000 : 0;
  x = Math.abs(x);
  if (x === Infinity) return sign | 0x7c00;
  if (x === 0) return sign;
  let e = Math.floor(Math.log2(x));
  let m = x / Math.pow(2, e) - 1;
  if (e < -14) return sign | Math.round(x / Math.pow(2, -24));
  if (e > 15) return sign | 0x7c00;
  let f = Math.round(m * 1024);
  if (f === 1024) {
    f = 0;
    e += 1;
    if (e > 15) return sign | 0x7c00;
  }
  return sign | ((e + 15) << 10) | f;
}

function intLens(label: string, size: number, signed: boolean): Lens {
  const bits = BigInt(size * 8);
  const min = signed ? -(1n << (bits - 1n)) : 0n;
  const max = signed ? (1n << (bits - 1n)) - 1n : (1n << bits) - 1n;
  return {
    label,
    size,
    decode: (v, le) => {
      let x = 0n;
      for (let i = 0; i < size; i++) {
        const b = BigInt(v.getUint8(le ? size - 1 - i : i));
        x = (x << 8n) | b;
      }
      if (signed && x > max) x -= 1n << bits;
      return x.toString();
    },
    encode: (text, le) => {
      const t = text.trim();
      if (!/^[-+]?(0x[0-9a-f]+|\d+)$/i.test(t)) return null;
      let x: bigint;
      try {
        x = BigInt(t);
      } catch {
        return null;
      }
      if (x < min || x > max) return null;
      if (x < 0n) x += 1n << bits;
      const out = new Uint8Array(size);
      for (let i = 0; i < size; i++) {
        out[le ? i : size - 1 - i] = Number((x >> BigInt(8 * i)) & 0xffn);
      }
      return out;
    },
  };
}

function floatLens(label: string, size: 2 | 4 | 8): Lens {
  return {
    label,
    size,
    decode: (v, le) => {
      const x = size === 2 ? f16ToNumber(v.getUint16(0, le)) : size === 4 ? v.getFloat32(0, le) : v.getFloat64(0, le);
      return Number.isFinite(x) ? String(x) : x > 0 ? "Infinity" : x < 0 ? "-Infinity" : "NaN";
    },
    encode: (text, le) => {
      const t = text.trim().toLowerCase();
      const x = t === "nan" ? NaN : t === "inf" || t === "infinity" ? Infinity : t === "-inf" || t === "-infinity" ? -Infinity : Number(t);
      if (t === "" || (Number.isNaN(x) && t !== "nan")) return null;
      const buf = new ArrayBuffer(size);
      const v = new DataView(buf);
      if (size === 2) v.setUint16(0, numberToF16(x), le);
      else if (size === 4) v.setFloat32(0, x, le);
      else v.setFloat64(0, x, le);
      return new Uint8Array(buf);
    },
  };
}

const LENSES: readonly Lens[] = [
  {
    label: "binary",
    size: 1,
    decode: (v) => v.getUint8(0).toString(2).padStart(8, "0"),
    encode: (t) => (/^[01]{1,8}$/.test(t.trim()) ? Uint8Array.of(parseInt(t.trim(), 2)) : null),
  },
  intLens("uint8", 1, false),
  intLens("int8", 1, true),
  intLens("uint16", 2, false),
  intLens("int16", 2, true),
  intLens("uint32", 4, false),
  intLens("int32", 4, true),
  intLens("uint64", 8, false),
  intLens("int64", 8, true),
  floatLens("float16", 2),
  floatLens("float32", 4),
  floatLens("float64", 8),
  {
    label: "utf-8",
    size: 4,
    decode: (v) => {
      const bytes = new Uint8Array(v.buffer, v.byteOffset, 4);
      try {
        const s = new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, utf8Len(bytes[0] ?? 0)));
        return s;
      } catch {
        return "(not valid UTF-8)";
      }
    },
    encode: (t) => {
      const cp = [...t][0];
      return cp === undefined ? null : new TextEncoder().encode(cp);
    },
  },
];

function utf8Len(b0: number): number {
  return b0 < 0x80 ? 1 : b0 < 0xe0 ? 2 : b0 < 0xf0 ? 3 : 4;
}

export class Inspector {
  readonly el: HTMLElement;
  private mode: Mode = "structure";
  /** Absolute bit position of the cursor. */
  private offset = 0;
  private readonly inputs = new Map<Lens, HTMLInputElement>();
  private readonly status: HTMLElement;
  private readonly table: HTMLElement;
  private readonly struct: HTMLElement;
  private readonly crumbs: HTMLElement;
  private readonly field: HTMLInputElement;
  private readonly detail: HTMLElement;
  private readonly fieldRow: HTMLElement;
  /** Path of the field the structure panel is showing, if any. */
  private at: readonly number[] | null = null;
  /** A field picked by name stays shown until the cursor moves off it. */
  private pinned: readonly number[] | null = null;

  /** Asked for when a breadcrumb is clicked, so the views can follow. */
  onPick: (path: readonly number[]) => void = () => {};

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "inspector";
    this.el.setAttribute("aria-label", "Value at cursor");

    const head = document.createElement("div");
    head.className = "insp-head";
    const title = document.createElement("span");
    title.textContent = "At cursor";
    const seg = document.createElement("div");
    seg.className = "seg";
    seg.setAttribute("role", "radiogroup");
    seg.setAttribute("aria-label", "Read the value as");
    for (const [value, label] of [["structure", "Structure"], ["le", "Little-endian"], ["be", "Big-endian"]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.setAttribute("role", "radio");
      b.setAttribute("aria-checked", String(value === this.mode));
      b.addEventListener("click", () => {
        this.mode = value;
        for (const c of seg.children) c.setAttribute("aria-checked", String(c === b));
        this.render();
      });
      seg.append(b);
    }
    head.append(title, seg);

    const table = document.createElement("table");
    table.className = "insp-table";
    for (const lens of LENSES) {
      const tr = document.createElement("tr");
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = lens.label;
      const td = document.createElement("td");
      const input = document.createElement("input");
      input.type = "text";
      input.spellcheck = false;
      input.autocomplete = "off";
      input.setAttribute("aria-label", lens.label);
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") this.commit(lens, input);
        if (e.key === "Escape") this.render();
      });
      input.addEventListener("blur", () => {
        if (input.dataset["dirty"] === "1") this.commit(lens, input);
      });
      input.addEventListener("input", () => {
        input.dataset["dirty"] = "1";
        input.classList.remove("invalid");
      });
      td.append(input);
      tr.append(th, td);
      table.append(tr);
      this.inputs.set(lens, input);
    }

    this.table = table;

    // Structure panel: where the cursor is in the template, and that field's value.
    this.struct = document.createElement("div");
    this.struct.className = "insp-struct";
    this.crumbs = document.createElement("div");
    this.crumbs.className = "insp-crumbs";
    this.crumbs.addEventListener("click", (e) => {
      const t = e.target;
      if (!(t instanceof HTMLElement)) return;
      const p = t.dataset["path"];
      if (p !== undefined) this.onPick(p === "" ? [] : p.split("/").map(Number));
    });
    this.field = document.createElement("input");
    this.field.type = "text";
    this.field.spellcheck = false;
    this.field.autocomplete = "off";
    this.field.className = "insp-field";
    this.field.addEventListener("keydown", (e) => {
      if (e.key === "Enter") this.commitField();
      if (e.key === "Escape") this.render();
    });
    this.field.addEventListener("blur", () => {
      if (this.field.dataset["dirty"] === "1") this.commitField();
    });
    this.field.addEventListener("input", () => {
      this.field.dataset["dirty"] = "1";
      this.field.classList.remove("invalid");
    });
    this.detail = document.createElement("div");
    this.detail.className = "insp-detail";
    this.fieldRow = document.createElement("div");
    this.fieldRow.className = "insp-fieldrow";
    this.fieldRow.append(this.field, this.detail);
    this.struct.append(this.crumbs, this.fieldRow);

    this.status = document.createElement("div");
    this.status.className = "insp-status";
    this.status.setAttribute("role", "status");

    this.el.append(head, this.struct, table, this.status);
    doc.onChange(() => this.render());
  }

  /** `bitOffset` is absolute, counting from the top bit of byte 0. */
  setOffset(bitOffset: number): void {
    this.offset = bitOffset;
    this.pinned = null;
    this.render();
  }

  /** Show this field rather than the innermost one at the cursor. */
  setPath(path: readonly number[]): void {
    this.pinned = path;
    this.render();
  }

  private commitField(): void {
    this.field.dataset["dirty"] = "0";
    if (this.at === null) return;
    const r = this.doc.writeNode(this.at, this.field.value);
    if (r.status === "error") {
      this.field.classList.add("invalid");
      this.status.textContent = r.message;
      return;
    }
    if (r.status === "pending") {
      this.status.textContent = "Loading this part of the file…";
      return;
    }
    this.status.textContent = "";
  }

  private commit(lens: Lens, input: HTMLInputElement): void {
    input.dataset["dirty"] = "0";
    const bytes = lens.encode(input.value, this.mode === "le");
    if (bytes === null) {
      input.classList.add("invalid");
      this.status.textContent = `Not a valid ${lens.label} value.`;
      return;
    }
    this.doc.overwriteBits(this.offset, bytes, bytes.length * 8);
    this.status.textContent = "";
  }

  render(): void {
    const structure = this.mode === "structure";
    this.struct.hidden = !structure;
    this.table.hidden = structure;
    if (structure) return this.renderStructure();

    const { bytes, complete } = this.doc.readBits(this.offset, 64);
    const avail = Math.max(0, Math.floor((this.doc.lengthBits - this.offset) / 8));
    const view = new DataView(bytes.buffer);
    for (const [lens, input] of this.inputs) {
      if (input.dataset["dirty"] === "1" && document.activeElement === input) continue;
      const fits = lens.size <= avail;
      input.disabled = !fits;
      input.classList.remove("invalid");
      if (!fits) {
        input.value = "";
        input.placeholder = avail === 0 ? "end of file" : `needs ${lens.size} bytes, ${avail} left`;
      } else if (!complete) {
        input.value = "";
        input.placeholder = "loading";
      } else {
        input.placeholder = "";
        input.value = lens.decode(view, this.mode === "le");
      }
    }
  }

  /** Where the cursor is in the template, and what that field holds. */
  private renderStructure(): void {
    if (this.doc.template === null) {
      this.at = null;
      this.crumbs.textContent = "No template selected. Pick one above to read the field here.";
      this.fieldRow.hidden = true;
      this.status.textContent = "";
      return;
    }
    const found = this.pinned === null ? this.doc.locate(this.offset) : ({ status: "ok", node: this.pinned } as const);
    if (found.status !== "ok") {
      this.at = null;
      this.crumbs.textContent = found.status === "pending" ? "Loading this part of the file…" : "Nothing defined at the cursor.";
      this.fieldRow.hidden = true;
      return;
    }
    const path: readonly number[] = found.node;
    const node = this.doc.templateNode(path);
    if (node.status !== "ok") {
      this.at = null;
      this.crumbs.textContent = node.status === "pending" ? "Loading this part of the file…" : node.message;
      this.fieldRow.hidden = true;
      return;
    }
    this.at = path;
    const n = node.node;
    this.crumbs.replaceChildren(...this.trail(path));
    this.fieldRow.hidden = false;
    this.detail.textContent = `${n.type} · ${formatOffset(n.offset_bits)} · ${sizeText(n.size_bits)}`;
    if (this.field.dataset["dirty"] === "1" && document.activeElement === this.field) return;
    this.field.disabled = !n.editable;
    this.field.classList.remove("invalid");
    this.field.value = n.composite ? "" : n.edit_text;
    this.field.placeholder = n.composite ? `${n.child_count.toLocaleString()} inside` : "";
    this.field.setAttribute("aria-label", `${n.name}, ${n.type}`);
  }

  /**
   * The last few steps of the path, each one selectable. The whole chain is in
   * the field table below, which follows the cursor too, so this stays short.
   */
  private trail(path: readonly number[]): HTMLElement[] {
    const out: HTMLElement[] = [];
    const from = Math.max(0, path.length - 2);
    if (from > 0) {
      const more = document.createElement("span");
      more.className = "insp-crumb insp-crumb-more";
      more.textContent = "…";
      out.push(more);
    }
    for (let i = from; i <= path.length; i++) {
      const p = path.slice(0, i);
      const node = this.doc.templateNode(p);
      const b = document.createElement("button");
      b.type = "button";
      b.className = i === path.length ? "insp-crumb insp-crumb-here" : "insp-crumb";
      b.dataset["path"] = p.join("/");
      // A struct field is often called `body`; its type says what it holds.
      b.textContent =
        node.status !== "ok"
          ? "?"
          : node.node.composite && node.node.type !== node.node.name
            ? `${node.node.name} (${node.node.type})`
            : node.node.name;
      out.push(b);
    }
    return out;
  }
}

function sizeText(bits: number): string {
  if (bits % 8 === 0) {
    const b = bits / 8;
    return b === 1 ? "1 byte" : `${b.toLocaleString()} bytes`;
  }
  return bits === 1 ? "1 bit" : `${bits.toLocaleString()} bits`;
}

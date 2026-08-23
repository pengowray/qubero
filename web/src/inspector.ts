// Value inspector: interprets the bytes under the cursor as common primitive
// types and writes edits back. Every row is a small two-way lens: decode bytes
// to text, parse text to bytes.

import type { Doc } from "./doc.js";

type Endian = "le" | "be";

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
  private endian: Endian = "le";
  private offset = 0;
  private readonly inputs = new Map<Lens, HTMLInputElement>();
  private readonly status: HTMLElement;

  constructor(private readonly doc: Doc) {
    this.el = document.createElement("section");
    this.el.className = "inspector";
    this.el.setAttribute("aria-label", "Value at cursor");

    const head = document.createElement("div");
    head.className = "insp-head";
    const title = document.createElement("span");
    title.textContent = "At cursor";
    const endian = document.createElement("div");
    endian.className = "seg";
    endian.setAttribute("role", "radiogroup");
    endian.setAttribute("aria-label", "Byte order");
    for (const [value, label] of [["le", "Little-endian"], ["be", "Big-endian"]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.setAttribute("role", "radio");
      b.setAttribute("aria-checked", String(value === this.endian));
      b.addEventListener("click", () => {
        this.endian = value;
        for (const c of endian.children) c.setAttribute("aria-checked", String(c === b));
        this.render();
      });
      endian.append(b);
    }
    head.append(title, endian);

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

    this.status = document.createElement("div");
    this.status.className = "insp-status";
    this.status.setAttribute("role", "status");

    this.el.append(head, table, this.status);
    doc.onChange(() => this.render());
  }

  setOffset(offset: number): void {
    this.offset = offset;
    this.render();
  }

  private commit(lens: Lens, input: HTMLInputElement): void {
    input.dataset["dirty"] = "0";
    const bytes = lens.encode(input.value, this.endian === "le");
    if (bytes === null) {
      input.classList.add("invalid");
      this.status.textContent = `Not a valid ${lens.label} value.`;
      return;
    }
    this.doc.overwrite(this.offset, bytes);
    this.status.textContent = "";
  }

  render(): void {
    const { bytes, complete } = this.doc.read(this.offset, 8);
    const avail = Math.max(0, this.doc.lengthBytes - this.offset);
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
        input.value = lens.decode(view, this.endian === "le");
      }
    }
  }
}

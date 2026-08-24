// The value inspector's lenses: one reading each of the bytes under the
// cursor, in both directions. Every one is two-way, so what it shows can be
// typed over and written back.

export type Lens = {
  readonly label: string;
  readonly size: number;
  readonly decode: (v: DataView, le: boolean) => string;
  /** Returns null when the text is not a valid value for this type. */
  readonly encode: (text: string, le: boolean) => Uint8Array | null;
};

export function f16ToNumber(h: number): number {
  const s = h >> 15 ? -1 : 1;
  const e = (h >> 10) & 0x1f;
  const f = h & 0x3ff;
  if (e === 0) return s * Math.pow(2, -14) * (f / 1024);
  if (e === 0x1f) return f ? NaN : s * Infinity;
  return s * Math.pow(2, e - 15) * (1 + f / 1024);
}

export function numberToF16(x: number): number {
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

export const LENSES: readonly Lens[] = [
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

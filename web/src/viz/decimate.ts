// The arithmetic behind `valueplot.ts`: what a run's elements are, the numbers
// its bytes come to, and which of them to draw.
//
// Part of candidate `value-plot` in docs/DESIGN-diagrams.md. DOM-free, with
// unit tests in `test/decimate.test.ts`, like every piece's layout arithmetic.
// It registers nothing, so it has no line in `register.ts`.
//
// Why min-max decimation. A WAV of a minute is millions of samples and a plot
// is a few hundred pixels across, so most of the samples have to go. Taking
// every hundredth sample (striding) is fast and wrong: it drops the one spike
// a reader opened the plot to find, and a click on a clipped stretch shows a
// waveform that never touches the rail. Averaging is wrong the same way. What
// survives a pixel's worth of samples is its lowest and its highest, drawn in
// the order they occur, so the line goes down to the bottom of the stretch and
// up to the top of it exactly as the full-resolution line would, and the
// picture at any zoom is the outline of every sample under it. Once a pixel
// holds fewer than two samples nothing is dropped and every sample is drawn.
//
// The numbers are decoded here from the bytes, with a `DataView`, rather than
// asked of the core one element at a time: `runCells` writes each element as
// text, which is right for a row of the hex view and a hundred times too slow
// for a plot of a million of them (section 4.2's note on `value-plot`).

/** One fixed-width number type, as the core names it: `i16 le`, `u8`,
 *  `f32 be`. */
export type NumType = {
  /** The core's own spelling, for the hover card. */
  readonly name: string;
  readonly bytes: number;
  readonly float: boolean;
  readonly signed: boolean;
  /** Little-endian. A one-byte type has no order and says true. */
  readonly le: boolean;
};

/** The element type of a list type: `i16 le[]` holds `i16 le`. */
export function elementType(listType: string): string {
  return listType.replace(/\[\]$/, "");
}

/**
 * The number type `type` names, or null for anything the plot cannot decode
 * from bytes alone: strings, structures, packed weights, bit fields, and the
 * integer sizes no DataView reads (a `u24` would need its own loop and no
 * sample file needs one yet).
 */
export function numType(type: string): NumType | null {
  const m = /^(u|i|f|bf)(8|16|32|64)(?: (le|be))?$/.exec(type.trim());
  if (m === null) return null;
  const [, kind, size] = m;
  const bytes = Number(size) / 8;
  const float = kind === "f" || kind === "bf";
  if (kind === "bf" && bytes !== 2) return null;
  if (kind === "f" && bytes === 1) return null;
  // A two-byte or wider type the core named without an order is not one this
  // can guess at.
  if (bytes > 1 && m[3] === undefined) return null;
  return { name: type.trim(), bytes, float, signed: kind !== "u", le: bytes === 1 || m[3] === "le" };
}

/** IEEE half precision to a number. */
function half(h: number): number {
  const sign = h & 0x8000 ? -1 : 1;
  const exp = (h >> 10) & 0x1f;
  const frac = h & 0x3ff;
  if (exp === 0) return sign * frac * 2 ** -24;
  if (exp === 31) return frac === 0 ? sign * Infinity : NaN;
  return sign * (1 + frac / 1024) * 2 ** (exp - 15);
}

/** How to read one number of each type at a byte offset. Chosen once per run
 *  by `decode`, so the loop over a run's elements does not look at the type's
 *  name for every one of them. */
function reader(v: DataView, name: string, le: boolean): ((at: number) => number) | null {
  switch (name) {
    case "u8": return (at) => v.getUint8(at);
    case "i8": return (at) => v.getInt8(at);
    case "u16": return (at) => v.getUint16(at, le);
    case "i16": return (at) => v.getInt16(at, le);
    case "u32": return (at) => v.getUint32(at, le);
    case "i32": return (at) => v.getInt32(at, le);
    case "u64": return (at) => Number(v.getBigUint64(at, le));
    case "i64": return (at) => Number(v.getBigInt64(at, le));
    case "f16": return (at) => half(v.getUint16(at, le));
    case "bf16": {
      // bfloat16 is the top half of a binary32.
      const word = new Uint32Array(1);
      const float = new Float32Array(word.buffer);
      return (at) => {
        word[0] = v.getUint16(at, le) << 16;
        return float[0] ?? NaN;
      };
    }
    case "f32": return (at) => v.getFloat32(at, le);
    case "f64": return (at) => v.getFloat64(at, le);
  }
  return null;
}

/** Decode `count` numbers of type `t` from the front of `bytes`. A 64-bit
 *  integer past 2^53 comes back rounded, which a plot cannot show anyway. */
export function decode(bytes: Uint8Array, t: NumType, count: number): Float64Array {
  const n = Math.max(0, Math.min(count, Math.floor(bytes.length / t.bytes)));
  const out = new Float64Array(n);
  const read = reader(new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), t.name.split(" ")[0] ?? "", t.le);
  if (read === null) return out;
  for (let i = 0, at = 0; i < n; i++, at += t.bytes) out[i] = read(at);
  return out;
}

/** The values of one channel of an interleaved run: element `frame * stride +
 *  channel`. One channel is stride 1, channel 0. */
export type Channel = { readonly values: ArrayLike<number>; readonly stride: number; readonly channel: number };

/** How many frames (rows of one element per channel) the run holds. A last
 *  frame the run stops partway through is left out. */
export function frameCount(c: Channel): number {
  return Math.floor(c.values.length / c.stride);
}

function at(c: Channel, frame: number): number {
  return c.values[frame * c.stride + c.channel] ?? NaN;
}

/** The lowest and highest finite value of a channel over frames `[from, to)`,
 *  and how many frames that is. Null when none of them is finite. */
export function rangeOf(c: Channel, from: number, to: number): { readonly min: number; readonly max: number; readonly frames: number } | null {
  let min = Infinity;
  let max = -Infinity;
  const end = Math.min(to, frameCount(c));
  for (let f = Math.max(0, from); f < end; f++) {
    const y = at(c, f);
    if (!Number.isFinite(y)) continue;
    if (y < min) min = y;
    if (y > max) max = y;
  }
  return min > max ? null : { min, max, frames: Math.max(0, end - Math.max(0, from)) };
}

/** What to draw: frame numbers in increasing order and the value at each, with
 *  null where a float is NaN or infinite so the line breaks there. */
export type Envelope = {
  readonly frames: number[];
  readonly values: (number | null)[];
  /** True when some frames were left out: each bucket kept only its lowest and
   *  highest. */
  readonly decimated: boolean;
};

/**
 * The frames of `[from, to)` worth drawing across `buckets` pixels: every one
 * of them when there are no more than two a pixel, and otherwise the lowest
 * and the highest of each pixel's share, in the order they occur. One frame
 * either side of the range is kept too, where the run has it, so the line
 * runs off the edge of the plot instead of stopping short of it.
 */
export function envelope(c: Channel, from: number, to: number, buckets: number): Envelope {
  const total = frameCount(c);
  const lo = Math.max(0, Math.floor(from));
  const hi = Math.min(total, Math.ceil(to));
  const frames: number[] = [];
  const values: (number | null)[] = [];
  const push = (f: number): void => {
    const y = at(c, f);
    frames.push(f);
    values.push(Number.isFinite(y) ? y : null);
  };
  if (lo > 0) push(lo - 1);
  const n = hi - lo;
  const b = Math.max(1, Math.floor(buckets));
  const decimated = n > b * 2;
  if (!decimated) {
    for (let f = lo; f < hi; f++) push(f);
  } else {
    const per = n / b;
    for (let i = 0; i < b; i++) {
      const s = lo + Math.floor(i * per);
      const e = Math.min(hi, lo + Math.floor((i + 1) * per));
      let minF = -1;
      let maxF = -1;
      let min = Infinity;
      let max = -Infinity;
      let gap = -1;
      for (let f = s; f < e; f++) {
        const y = at(c, f);
        if (!Number.isFinite(y)) {
          if (gap < 0) gap = f;
          continue;
        }
        if (y < min) {
          min = y;
          minF = f;
        }
        if (y > max) {
          max = y;
          maxF = f;
        }
      }
      if (minF < 0) {
        // A pixel of nothing but NaN still breaks the line.
        if (gap >= 0) push(gap);
        continue;
      }
      const first = Math.min(minF, maxF);
      const second = Math.max(minF, maxF);
      push(first);
      if (second !== first) push(second);
    }
  }
  if (hi < total) push(hi);
  return { frames, values, decimated };
}

/** A finite low and high to scale a set of channels to, padded so a flat run
 *  still has some height. */
export function extentOf(channels: readonly Channel[]): [number, number] {
  return paddedExtent(finiteRange(channels));
}

/** The lowest and highest finite value over every channel, or null when none
 *  is finite: what the plot's note states. */
export function finiteRange(channels: readonly Channel[]): { readonly min: number; readonly max: number } | null {
  let min = Infinity;
  let max = -Infinity;
  for (const c of channels) {
    const r = rangeOf(c, 0, frameCount(c));
    if (r === null) continue;
    min = Math.min(min, r.min);
    max = Math.max(max, r.max);
  }
  return min > max ? null : { min, max };
}

/** The y range for `range`: as it is, or widened round a run of one value so
 *  the line is drawn across the middle of the lane, and -1 to 1 for none. */
export function paddedExtent(range: { readonly min: number; readonly max: number } | null): [number, number] {
  if (range === null) return [-1, 1];
  const { min, max } = range;
  if (min === max) {
    const pad = min === 0 ? 1 : Math.abs(min) * 0.1;
    return [min - pad, max + pad];
  }
  return [min, max];
}

/** The frame nearest a fractional frame position, kept inside the run. */
export function nearestFrame(x: number, frames: number): number {
  if (frames <= 0) return 0;
  return Math.max(0, Math.min(frames - 1, Math.round(x)));
}

/** A value as the hover card writes it: an integer with its digits grouped,
 *  a float to seven significant digits, which is all a binary32 holds. */
export function valueText(v: number, float: boolean): string {
  if (Number.isNaN(v)) return "NaN";
  if (!Number.isFinite(v)) return v > 0 ? "∞" : "-∞";
  if (!float) return v.toLocaleString();
  if (v === 0) return "0";
  return String(Number(v.toPrecision(7)));
}

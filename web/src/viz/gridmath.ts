// The arithmetic behind the grid form of `valueplot.ts`: which values each
// pixel of a zoomed weight matrix stands for, and the colour each value gets.
//
// Part of candidate `value-plot` in docs/DESIGN-diagrams.md. DOM-free, with
// unit tests in `test/decimate.test.ts`.
//
// A grid of 4096 by 4096 weights has sixteen million cells and a pane has a
// few hundred thousand pixels, so zoomed out each pixel stands for a block of
// values. The pixel shows the block's mean, and the plot says how many values
// that is (section 4.2 asks for the sampling to be stated on screen). Zoomed
// in past one value a pixel, each pixel is the one value under it.
//
// Colours: a diverging scale around zero for values of both signs, which is
// what weights are, so a cell's sign and size read together; a sequential
// scale for values of one sign. Mixed in OKLab, where equal steps of the scale
// look like equal steps of lightness, which mixing sRGB hexes does not give.

/** The values each output pixel stands for, over `[c0, c1)` columns by
 *  `[r0, r1)` rows of a grid stored row after row. */
export type GridSample = {
  /** One mean per output pixel, row by row; NaN where the pixel is off the
   *  grid or every value under it is NaN. */
  readonly means: Float32Array;
  /** The most values any one pixel averaged. 1 when zoomed in past one value
   *  a pixel. */
  readonly per: number;
};

/** The first column (or row) of the block under output pixel `p`, and one past
 *  its last. A block is never empty, so a pixel narrower than a cell takes
 *  the cell it is in. */
export function pixelBlock(start: number, end: number, out: number, p: number, limit: number): [number, number] {
  const step = (end - start) / out;
  const a = Math.floor(start + p * step);
  const b = Math.max(a + 1, Math.floor(start + (p + 1) * step));
  return [Math.max(0, a), Math.min(limit, b)];
}

export function sampleGrid(
  values: ArrayLike<number>,
  columns: number,
  rows: number,
  view: { readonly c0: number; readonly c1: number; readonly r0: number; readonly r1: number },
  outW: number,
  outH: number,
): GridSample {
  const w = Math.max(1, Math.floor(outW));
  const h = Math.max(1, Math.floor(outH));
  const means = new Float32Array(w * h).fill(NaN);
  let per = 1;
  const cols: [number, number][] = [];
  for (let px = 0; px < w; px++) cols.push(pixelBlock(view.c0, view.c1, w, px, columns));
  for (let py = 0; py < h; py++) {
    const [ra, rb] = pixelBlock(view.r0, view.r1, h, py, rows);
    if (ra >= rb) continue;
    for (let px = 0; px < w; px++) {
      const [ca, cb] = cols[px] as [number, number];
      if (ca >= cb) continue;
      let sum = 0;
      let n = 0;
      for (let r = ra; r < rb; r++) {
        const base = r * columns;
        for (let c = ca; c < cb; c++) {
          const v = values[base + c] ?? NaN;
          if (Number.isFinite(v)) {
            sum += v;
            n++;
          }
        }
      }
      per = Math.max(per, (rb - ra) * (cb - ca));
      if (n > 0) means[py * w + px] = sum / n;
    }
  }
  return { means, per };
}

/** The range the colours span: every finite value, or the middle 99% of them
 *  when `clip` is set, so a few outliers do not wash the rest out to grey. */
export function colourRange(values: ArrayLike<number>, clip: boolean): { readonly lo: number; readonly hi: number; readonly diverging: boolean } {
  const finite: number[] = [];
  for (let i = 0; i < values.length; i++) {
    const v = values[i] as number;
    if (Number.isFinite(v)) finite.push(v);
  }
  if (finite.length === 0) return { lo: -1, hi: 1, diverging: true };
  finite.sort((a, b) => a - b);
  const q = (p: number): number => finite[Math.min(finite.length - 1, Math.max(0, Math.round(p * (finite.length - 1))))] as number;
  const lo = clip ? q(0.005) : (finite[0] as number);
  const hi = clip ? q(0.995) : (finite[finite.length - 1] as number);
  const diverging = lo < 0 && hi > 0;
  if (diverging) {
    // Symmetric about zero, so grey is zero and equal distances from it are
    // equal colours either side.
    const m = Math.max(-lo, hi);
    return { lo: -m, hi: m, diverging };
  }
  return lo === hi ? { lo: lo - 1, hi: hi + 1, diverging } : { lo, hi, diverging };
}

export type Rgb = readonly [number, number, number];

/** `#rrggbb` to 0..255 channels. */
export function parseHex(hex: string): Rgb | null {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex.trim());
  if (m === null) return null;
  return [parseInt(m[1] as string, 16), parseInt(m[2] as string, 16), parseInt(m[3] as string, 16)];
}

const toLinear = (c: number): number => {
  const s = c / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
};
const fromLinear = (l: number): number => {
  const s = l <= 0.0031308 ? l * 12.92 : 1.055 * l ** (1 / 2.4) - 0.055;
  return Math.max(0, Math.min(255, Math.round(s * 255)));
};

function toOklab([r, g, b]: Rgb): [number, number, number] {
  const lr = toLinear(r);
  const lg = toLinear(g);
  const lb = toLinear(b);
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function fromOklab([L, a, b]: readonly [number, number, number]): Rgb {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    fromLinear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    fromLinear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    fromLinear(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
  ];
}

/** `a` and `b` mixed `t` of the way from `a`, in OKLab. */
export function mix(a: Rgb, b: Rgb, t: number): Rgb {
  const p = toOklab(a);
  const q = toOklab(b);
  const u = Math.max(0, Math.min(1, t));
  return fromOklab([p[0] + (q[0] - p[0]) * u, p[1] + (q[1] - p[1]) * u, p[2] + (q[2] - p[2]) * u]);
}

/**
 * A table of `n` colours from the bottom of the range to the top: through
 * `mid` from `neg` to `pos` when the scale is diverging, and from `mid` to
 * `pos` when it is not. Packed as RGBA bytes, ready for an `ImageData`.
 */
export function ramp(neg: Rgb, mid: Rgb, pos: Rgb, diverging: boolean, n = 256): Uint8ClampedArray {
  const out = new Uint8ClampedArray(n * 4);
  for (let i = 0; i < n; i++) {
    const t = n === 1 ? 0 : i / (n - 1);
    const c = diverging ? (t < 0.5 ? mix(neg, mid, t * 2) : mix(mid, pos, (t - 0.5) * 2)) : mix(mid, pos, t);
    out.set([c[0], c[1], c[2], 255], i * 4);
  }
  return out;
}

/** Which entry of a ramp of `n` a value takes, with values outside the range
 *  taking the end colours. */
export function rampIndex(v: number, lo: number, hi: number, n = 256): number {
  const t = hi === lo ? 0.5 : (v - lo) / (hi - lo);
  return Math.max(0, Math.min(n - 1, Math.round(t * (n - 1))));
}

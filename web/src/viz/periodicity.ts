// How much the bytes look like themselves a given distance on, for every
// distance up to a limit: the autocorrelation of the byte values. The stride
// raster (`strideraster.ts`) draws it as a line beside the picture, and its
// peaks are the row widths worth trying.
//
// Part of candidate `stride-grid` in docs/DESIGN-diagrams.md, built by agent N
// of the first batch. DOM-free, so the tests run it as it is; the worker that
// runs it off the page's thread is `periodicityworker.ts`.
//
// Why autocorrelation, and why through an FFT. A file of fixed-size records
// repeats at the record size: the CD sector's sync bytes every 2,352 bytes, a
// tar header's fields every 512. Correlating the bytes (less their mean)
// against themselves shifted by a lag gives a peak at that lag and at each
// multiple of it. Done directly that is a multiply for every byte at every
// lag, which for 256 KiB at 64 K lags is 17 billion; through an FFT
// (Wiener-Khinchin: the inverse transform of the power spectrum) it is a few
// million, quick enough to do again whenever a file opens.
//
// Why the peak picked is the smallest strong one rather than the highest. At
// a lag of twice the record size the records line up as well as at once the
// record size, and noise decides which of the two comes out higher. A reader
// asking for the row width wants the record, so the shortest lag whose peak
// stands nearly as high as the best one wins (`findPeaks`).

/** In-place radix-2 FFT of `re` + i `im`, whose length is a power of two.
 *  `inverse` runs it the other way, unscaled. */
export function fft(re: Float64Array, im: Float64Array, inverse = false): void {
  const n = re.length;
  // Bit-reversed order first, then the butterflies.
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      const tr = re[i] as number;
      re[i] = re[j] as number;
      re[j] = tr;
      const ti = im[i] as number;
      im[i] = im[j] as number;
      im[j] = ti;
    }
  }
  const sign = inverse ? 1 : -1;
  for (let len = 2; len <= n; len <<= 1) {
    const angle = (sign * 2 * Math.PI) / len;
    const wr = Math.cos(angle);
    const wi = Math.sin(angle);
    const half = len >> 1;
    for (let i = 0; i < n; i += len) {
      let cr = 1;
      let ci = 0;
      for (let k = 0; k < half; k++) {
        const a = i + k;
        const b = a + half;
        const br = re[b] as number;
        const bi = im[b] as number;
        const xr = br * cr - bi * ci;
        const xi = br * ci + bi * cr;
        re[b] = (re[a] as number) - xr;
        im[b] = (im[a] as number) - xi;
        re[a] = (re[a] as number) + xr;
        im[a] = (im[a] as number) + xi;
        const nr = cr * wr - ci * wi;
        ci = cr * wi + ci * wr;
        cr = nr;
      }
    }
  }
}

/**
 * The autocorrelation of `bytes` at every lag from 0 to `maxLag`, where 1 is
 * as alike as at no shift, 0 is no more alike than chance and below 0 is
 * opposite. Each lag is averaged over the pairs it has, so a long lag, with
 * fewer pairs, is not made to look less alike for that alone. `maxLag` is
 * held to half the bytes, past which there are too few pairs to say anything.
 * All zeros for bytes that never change.
 */
export function autocorrelation(bytes: Uint8Array, maxLag: number): Float32Array {
  const n = bytes.length;
  const lags = Math.max(0, Math.min(maxLag, Math.floor(n / 2)));
  const out = new Float32Array(lags + 1);
  if (n < 2) return out;
  let mean = 0;
  for (let i = 0; i < n; i++) mean += bytes[i] as number;
  mean /= n;
  let size = 1;
  while (size < 2 * n) size <<= 1;
  const re = new Float64Array(size);
  const im = new Float64Array(size);
  for (let i = 0; i < n; i++) re[i] = (bytes[i] as number) - mean;
  fft(re, im);
  for (let i = 0; i < size; i++) {
    const r = re[i] as number;
    const m = im[i] as number;
    re[i] = r * r + m * m;
    im[i] = 0;
  }
  fft(re, im, true);
  const zero = (re[0] as number) / size / n;
  if (!(zero > 1e-12)) return out;
  for (let lag = 0; lag <= lags; lag++) out[lag] = (re[lag] as number) / size / (n - lag) / zero;
  return out;
}

/** A lag the bytes repeat at. `prominence` is how far its peak stands above
 *  the lags around it. */
export type Peak = { readonly lag: number; readonly r: number; readonly prominence: number };

/** Below this a peak is noise. Random bytes give peaks of about 1/sqrt(n), so
 *  the floor rises for a short sample. */
export function noiseFloor(sampleBytes: number): number {
  return Math.max(0.03, 4 / Math.sqrt(Math.max(1, sampleBytes)));
}

/** Whether `lag` is a whole multiple, two or more, of `base`. */
function multipleOf(lag: number, base: number): boolean {
  const q = Math.round(lag / base);
  return q >= 2 && Math.abs(lag - q * base) <= Math.max(1, lag * 0.005);
}

/**
 * The lags worth trying as a row width, best first, at most `count`. A peak
 * is a lag higher than every lag within 1% of it, and its prominence is its
 * height above the mean of the lags within 10% either side, outside that 1%.
 * The best is the shortest peak whose prominence is at least `strong` of the
 * highest (see the header); the rest follow by prominence, leaving out the
 * multiples of a lag already listed, which say nothing new. Lags under
 * `minLag` are left out: at lag 1 any smooth run of bytes looks periodic.
 */
export function findPeaks(r: Float32Array, opts: { readonly count?: number; readonly minLag?: number; readonly floor?: number; readonly strong?: number } = {}): Peak[] {
  const count = opts.count ?? 4;
  const minLag = Math.max(1, opts.minLag ?? 2);
  const floor = opts.floor ?? 0.03;
  const strong = opts.strong ?? 0.7;
  const last = r.length - 1;
  const found: Peak[] = [];
  for (let lag = minLag; lag < last; lag++) {
    const v = r[lag] as number;
    const w = Math.max(1, Math.round(lag * 0.01));
    let top = true;
    for (let j = Math.max(1, lag - w); j <= Math.min(last, lag + w) && top; j++) if (j !== lag && (r[j] as number) >= v && !((r[j] as number) === v && j > lag)) top = false;
    if (!top) continue;
    const span = Math.max(4, Math.round(lag * 0.1));
    let sum = 0;
    let n = 0;
    for (let j = Math.max(1, lag - span); j <= Math.min(last, lag + span); j++) {
      if (Math.abs(j - lag) <= w) continue;
      sum += r[j] as number;
      n++;
    }
    const prominence = n > 0 ? v - sum / n : 0;
    if (prominence >= floor) found.push({ lag, r: v, prominence });
  }
  if (found.length === 0) return [];
  const highest = Math.max(...found.map((p) => p.prominence));
  const best = found.filter((p) => p.prominence >= strong * highest).reduce((a, b) => (b.lag < a.lag ? b : a));
  const out: Peak[] = [best];
  for (const p of [...found].sort((a, b) => b.prominence - a.prominence)) {
    if (out.length >= count) break;
    if (out.some((q) => q.lag === p.lag || multipleOf(p.lag, q.lag) || multipleOf(q.lag, p.lag))) continue;
    out.push(p);
  }
  return out;
}

/** What the worker is asked: the sample, and the longest lag wanted. */
export type PeriodicityRequest = { readonly id: number; readonly bytes: Uint8Array; readonly maxLag: number };

/** What it answers. */
export type PeriodicityResult = { readonly id: number; readonly r: Float32Array; readonly peaks: readonly Peak[] };

/** The whole job, as the worker runs it, here so the tests can run it too. */
export function periodicity(req: PeriodicityRequest): PeriodicityResult {
  const r = autocorrelation(req.bytes, req.maxLag);
  return { id: req.id, r, peaks: findPeaks(r, { floor: noiseFloor(req.bytes.length) }) };
}

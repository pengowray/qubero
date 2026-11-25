// Every length of time the app shows, in one place: how far apart two samples
// are, how long a recording is, a fact such as `TBIN`, a time column, the
// ticks of a plot's time axis.
//
// The unit follows the size of the value, so a GUPPI sample interval reads
// `341.3 ns` and not `0.0000003413 s`. Below a minute that is one of ns, µs,
// ms or s; from a minute up a duration is minutes and seconds, then hours.

/** A unit a time below a minute is written in. */
export type TimeUnit = { readonly symbol: "ns" | "µs" | "ms" | "s"; readonly seconds: number };

const NS: TimeUnit = { symbol: "ns", seconds: 1e-9 };
const US: TimeUnit = { symbol: "µs", seconds: 1e-6 };
const MS: TimeUnit = { symbol: "ms", seconds: 1e-3 };
const S: TimeUnit = { symbol: "s", seconds: 1 };

/** Places after the point a time is never given more of: past nine, the digits
 *  are longer than the double they came from is exact. */
const MAX_DECIMALS = 9;

/** Significant digits of a time written on its own: `341.3 ns`, `12.35 s`. */
const SIGNIFICANT = 4;

/** The unit a time of `seconds` reads best in: the largest of ns, µs, ms and
 *  s that leaves at least 1 in front of the point. Zero, and anything that is
 *  not a number, is in seconds. */
export function timeUnit(seconds: number): TimeUnit {
  const a = Math.abs(seconds);
  if (!Number.isFinite(a) || a === 0 || a >= 1) return S;
  if (a >= 1e-3) return MS;
  if (a >= 1e-6) return US;
  return NS;
}

/** Fewest places after the point that still tell apart two times `step`
 *  seconds apart, in `unit`: enough for a round step such as an axis tick's.
 *  `exact` adds places where the step is under one of the unit, so a step
 *  that is not round still shows: 64 µs in ms is `0.064`, not `0.06`. */
export function decimalsFor(step: number, unit: TimeUnit, exact = false): number {
  if (!(step > 0) || !Number.isFinite(step)) return 0;
  const inUnit = step / unit.seconds;
  if (inUnit >= 1) return 0;
  const least = Math.ceil(-Math.log10(inUnit) - 1e-9);
  if (!exact) return Math.min(MAX_DECIMALS, least);
  // A step that ends within three more places is shown to its end, so 125 µs
  // in ms is `0.125` and not `0.13`. One that never ends, as 1/44,100 s does,
  // gets one place more than the least.
  for (let d = least; d <= least + 3 && d <= MAX_DECIMALS; d++) {
    const scaled = inUnit * 10 ** d;
    if (Math.abs(scaled - Math.round(scaled)) < 1e-6) return d;
  }
  return Math.min(MAX_DECIMALS, least + 1);
}

function significant(n: number): string {
  return n.toLocaleString(undefined, { maximumSignificantDigits: SIGNIFICANT });
}

function fixed(n: number, decimals: number): string {
  return n.toLocaleString(undefined, { minimumFractionDigits: decimals, maximumFractionDigits: decimals });
}

/**
 * A length of time on its own: `341.3 ns`, `64 µs`, `12.35 s`, `4 min 12 s`,
 * `10 min`, `1 h 2 min 3 s`. Four significant digits below a minute, whole
 * seconds from a minute up. Empty for something that is not a number.
 */
export function durationText(seconds: number): string {
  if (!Number.isFinite(seconds)) return "";
  const a = Math.abs(seconds);
  // Rounded first, so 59.99 s reads as a minute rather than as `60 s`.
  if (a < 60 && Number(a.toPrecision(SIGNIFICANT)) < 60) {
    // The unit is picked after rounding, so 0.99996 s is `1 s` and not
    // `1,000 ms`.
    const unit = timeUnit(Number(seconds.toPrecision(SIGNIFICANT)));
    return `${significant(seconds / unit.seconds)} ${unit.symbol}`;
  }
  const sign = seconds < 0 ? "-" : "";
  const whole = Math.round(a);
  const h = Math.floor(whole / 3600);
  const m = Math.floor((whole % 3600) / 60);
  const s = whole % 60;
  // Parts that are zero at the end are left off: `10 min`, not `10 min 0 s`.
  const hours = `${h.toLocaleString()} h`;
  if (h > 0) return sign + (s > 0 ? `${hours} ${m} min ${s} s` : m > 0 ? `${hours} ${m} min` : hours);
  return sign + (s > 0 ? `${m} min ${s} s` : `${m} min`);
}

/**
 * How far apart two samples are, at `rate` a second, as a length of time:
 * `125 µs` at 8,000 a second, `292.6 µs` at 3,417.969. The same form as a
 * fact such as `TBIN 125 µs`, so the two can be compared at a glance.
 */
export function spacingText(rate: number): string {
  if (!(rate > 0) || !Number.isFinite(rate)) return "";
  return durationText(1 / rate);
}

/**
 * One time among others `step` seconds apart, such as a sample's time in a
 * hover: in the unit its own size calls for, with places enough to tell it
 * from its neighbours. `27.982 ms` for sample 1,234 at 44,100 a second.
 * Seconds from a minute up, since a point in a recording is read against the
 * ones beside it rather than as a length.
 */
export function timeAtText(seconds: number, step: number): string {
  if (!Number.isFinite(seconds)) return "";
  const unit = timeUnit(seconds === 0 ? step : seconds);
  return `${fixed(seconds / unit.seconds, decimalsFor(step, unit, true))} ${unit.symbol}`;
}

/**
 * The labels of a time axis whose ticks are `step` seconds apart. One unit for
 * every tick, the one the largest tick calls for, so the labels can be read
 * against each other: `0 ms`, `250 ms`, `500 ms`, and not `0 s`, `250 ms`.
 */
export function axisTimes(ticks: readonly number[], step: number): string[] {
  const largest = ticks.reduce((m, t) => Math.max(m, Math.abs(t)), 0);
  const unit = timeUnit(largest === 0 ? step : largest);
  const decimals = decimalsFor(step, unit);
  return ticks.map((t) => `${fixed(t / unit.seconds, decimals)} ${unit.symbol}`);
}

/**
 * How a column of times is written: one unit for the whole column, the one the
 * last row's time calls for, and places enough for two rows in a row to
 * differ. 44,100 rows a second for ten minutes is `s` to five places; 4,096
 * rows 64 µs apart is `ms` to three.
 */
export type TimeColumn = { readonly unit: TimeUnit; readonly decimals: number; readonly step: number };

export function timeColumn(count: number, rate: number): TimeColumn {
  const step = rate > 0 ? 1 / rate : 0;
  const last = Math.max(0, count - 1) * step;
  const unit = timeUnit(last === 0 ? step : last);
  return { unit, decimals: decimalsFor(step, unit, true), step };
}

/** Row `i`'s time in a column, without its unit, which is in the heading. */
export function columnTimeText(i: number, column: TimeColumn): string {
  return fixed((i * column.step) / column.unit.seconds, column.decimals);
}

/** A number written with a unit, said in the unit that suits it when the unit
 *  is seconds: a FITS `TBIN = 6.4E-05` reads `64 µs`. Null for anything else,
 *  which the caller writes as it was. */
export function secondsValueText(value: string, unit: string): string | null {
  if (unit !== "s" || value.trim() === "") return null;
  const n = Number(value.trim());
  return Number.isFinite(n) ? durationText(n) : null;
}

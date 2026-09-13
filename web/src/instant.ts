// The digits of a moment the core has resolved, printed to the precision the
// field has. Kept out of the inspector so the one part of it with arithmetic in
// it, the sixtieth second, can be tested without a panel.

/**
 * An instant as `YYYY-MM-DD HH:MM:SS`, with as many decimal places of a second
 * as the field can actually hold.
 *
 * Read out in UTC throughout, and that is not a choice about zones: the core
 * has already put the answer on the UTC line, so for a field the file records
 * no zone for these are the digits the file wrote, unshifted. `TIME.zone` says
 * which of the three it was. Nothing here may reach for `toLocale*` or
 * `getTimezoneOffset`: a ZIP written in Berlin does not become a different time
 * because it is being read in Auckland.
 *
 * `toISOString` gives a four-digit year over the whole band the core answers
 * for, years 1 to 9999, so slicing is safe; the fraction is built from `nanos`
 * rather than taken from that string, since a hundred-nanosecond FILETIME tick
 * is finer than the milliseconds a `Date` holds.
 *
 * `leap` is a moment inside a leap second, and then `unixSeconds` is second 59
 * of its minute, since a Unix count has no number for the second after it and
 * neither does a `Date`. The date and the minute are that second's, and the
 * seconds are written as 60, which is what the moment was: `23:59:60.5` at the
 * end of 2016, and never `00:00:00.5` on the next day or a second `23:59:59.5`.
 */
export function instantDigits(unixSeconds: number, nanos: number, stepNanos: number, leap = false): string {
  const iso = new Date(unixSeconds * 1000).toISOString();
  const whole = `${iso.slice(0, 10)} ${iso.slice(11, 17)}${leap ? "60" : iso.slice(17, 19)}`;
  const places = decimalPlaces(stepNanos);
  if (places === 0) return whole;
  return `${whole}.${nanos.toString().padStart(9, "0").slice(0, places)}`;
}

/**
 * How many decimal places of a second a field of this precision is worth
 * printing to: nine, less one for every power of ten in the step.
 *
 * A field counting whole seconds gets none, since `.000` after it would be
 * three digits the file never held, and an MS-DOS time, coarser than a second,
 * gets none either. A FILETIME's hundred-nanosecond tick gets seven and not
 * nine: the last two digits of a nine-place fraction are zero in every FILETIME
 * ever written, and printing them says the file is more precise than it is,
 * which is the same lie in miniature as showing a wrong date.
 */
export function decimalPlaces(stepNanos: number): number {
  let places = 9;
  for (let step = stepNanos; step >= 10 && places > 0; step /= 10) places--;
  return places;
}

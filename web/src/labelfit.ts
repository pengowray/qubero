// A name too long for the room it has is written shorter rather than cut off
// with an ellipsis. "PSRFITS search mode (filterbank samples) · FITS" in a
// narrow toolbar reads "PSRFITS search mode", and "Hierarchical Data Format
// (version 5) data" reads "HDF5"; the whole name is in the hover.

/**
 * The ways to write `full`, longest first, each shorter than the one before:
 * the whole name, then up to its first ` · `, then without a bracketed
 * remark at its end, then up to its first comma. `short` goes last: the
 * format's own short name, such as a template's `HDF5`, when it names the
 * same thing `full` does.
 */
export function shorterForms(full: string, short: string | null = null): string[] {
  const forms = [full];
  const add = (s: string): void => {
    const t = s.trim();
    const last = forms[forms.length - 1] ?? "";
    if (t !== "" && t.length < last.length) forms.push(t);
  };
  let s = full;
  const dot = s.indexOf(" · ");
  if (dot > 0) add((s = s.slice(0, dot)));
  // Only a remark at the end: one in the middle, as in "Hierarchical Data
  // Format (version 5) data", leaves words that do not read as a name.
  const remark = /\s*\([^()]*\)$/.exec(s);
  if (remark !== null && remark.index > 0) add((s = s.slice(0, remark.index)));
  const comma = s.indexOf(", ");
  if (comma > 0) add((s = s.slice(0, comma)));
  if (short !== null) add(short);
  return forms;
}

/**
 * Write the longest of `forms` that fits `el` without being cut, or the
 * shortest when none does. `el` has to take its width from the space around
 * it rather than from its text, as a flex item with a basis of 0 does, or
 * every form fits.
 */
export function fitText(el: HTMLElement, forms: readonly string[]): void {
  for (const form of forms) {
    el.textContent = form;
    if (el.scrollWidth <= el.clientWidth) return;
  }
}

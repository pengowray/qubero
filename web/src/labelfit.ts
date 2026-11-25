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
 * Write the longest of `forms` that fits `el` without being cut. `el` has to
 * take its width from the space around it rather than from its text, as a
 * flex item with a basis of 0 does, or every form fits.
 *
 * The shortest form sets `el`'s minimum width, so it is never cut either: a
 * toolbar with a long file name in it gave the file type 14 px, which cut even
 * `HDF5`. The file name, or the next control along, gives way instead.
 */
export function fitText(el: HTMLElement, forms: readonly string[]): void {
  const shortest = forms[forms.length - 1] ?? "";
  el.textContent = shortest;
  // The text's own width, whatever the box's: a range measures the run of
  // text, not the element it is in.
  const range = document.createRange();
  range.selectNodeContents(el);
  // Never wider than the row, so a long name with no shorter form is cut
  // there rather than pushing the page sideways on a phone.
  const min = shortest === "" ? "" : `min(${Math.ceil(range.getBoundingClientRect().width) + 1}px, 100%)`;
  if (el.style.minWidth !== min) el.style.minWidth = min;
  for (const form of forms) {
    el.textContent = form;
    if (el.scrollWidth <= el.clientWidth) return;
  }
}

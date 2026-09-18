// What the browser remembers between visits, and what happens when it will not.
//
// Everything kept here is a convenience for the one reader at this browser: a
// view they had open, a column they chose, a panel they folded. None of it is
// the file, none of it is an edit, and nothing the page does may depend on
// getting it back. A browser can refuse storage outright, which a private
// window and a site with its data blocked both do, and there the very act of
// reaching for `localStorage` throws. An unguarded read at the top of a module
// therefore takes the whole page down before a single row is drawn, over a
// setting. So every access goes through this file, every one of them is inside
// its own `try`, and a refusal reads as "nothing was kept", which is the same
// answer a first visit gives.

/** What was kept under this name, or null where nothing was kept and null
 *  where the browser will not say. The caller cannot tell those apart, and
 *  does not need to: both mean it must fall back to a default. */
export function storedText(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * A choice kept between visits, checked against what is on offer now. A name
 * that is no longer one of them falls back rather than leaving a chooser
 * pointing at nothing.
 */
export function storedChoice(key: string, offered: readonly string[], fallback: string): string {
  const saved = storedText(key);
  return saved !== null && offered.includes(saved) ? saved : fallback;
}

/** A number kept between visits, checked before it is believed. A stored value
 *  that is not a number any more falls back rather than leaving a control
 *  showing nothing. */
export function storedNumber(key: string, fallback: number): number {
  const saved = Number(storedText(key));
  return Number.isFinite(saved) && saved > 0 ? saved : fallback;
}

/** Remember a choice, and carry on if the browser will not keep it. */
export function rememberChoice(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // A browser that refuses storage still gets the choice for this visit.
  }
}

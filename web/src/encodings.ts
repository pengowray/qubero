/**
 * The encodings the app offers by name, in one place so the text view's
 * chooser and the panel's readings cannot drift apart. The names are the ones
 * the core answers with, and are what gets sent back across the boundary.
 */

/** Encodings that need no choosing: the bytes say which one they are, or are
 *  not that one at all. */
export const UNICODE_ENCODINGS = ["UTF-8", "ASCII", "UTF-16 LE", "UTF-16 BE"] as const;

/** Single-byte pages of the ISO, Windows, Mac and KOI8 family. Latin-1 first,
 *  since it is the one a file with high bytes and no other clue is read as. */
export const CODEPAGES_A = [
  "Latin-1",
  "Windows-1252",
  "ISO-8859-15",
  "ISO-8859-2",
  "Windows-1250",
  "Windows-1251",
  "KOI8-R",
  "Mac Roman",
] as const;

/** The DOS pages, which is what a capture of a DOS screen is in. */
export const CODEPAGES_B = ["CP437", "CP850", "CP866"] as const;

/** Languages the selection can be written as a string literal in. */
export const LITERAL_LANGS = ["C", "Rust", "Python", "JavaScript", "JSON", "C#", "Go"] as const;

export const CODEPAGE_A_KEY = "qubero.codepage.a";
export const CODEPAGE_B_KEY = "qubero.codepage.b";
/** What the literal chooser shows for each language: the language alone reads
 *  as a mystery beside a quoted string, so each option says what it is. */
export const LITERAL_LANG_NAMES: Readonly<Record<string, string>> = Object.fromEntries(
  LITERAL_LANGS.map((l) => [l, `${l} string`]),
);

export const LITERAL_LANG_KEY = "qubero.literal.lang";

export const CODEPAGE_A_DEFAULT = "Latin-1";
export const CODEPAGE_B_DEFAULT = "CP437";
export const LITERAL_LANG_DEFAULT = "C";

/**
 * A choice kept between visits, checked against what is on offer now. A name
 * that is no longer one of them falls back rather than leaving a chooser
 * pointing at nothing.
 */
export function storedChoice(key: string, offered: readonly string[], fallback: string): string {
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(key);
  } catch {
    saved = null;
  }
  return saved !== null && offered.includes(saved) ? saved : fallback;
}

/** Remember a choice, and carry on if the browser will not keep it. */
export function rememberChoice(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // A browser that refuses storage still gets the choice for this visit.
  }
}

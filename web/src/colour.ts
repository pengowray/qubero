// Colours written as text, and the swatch that shows one.
//
// A Claude Code theme writes a colour four ways: `rgb(r,g,b)`, `#rrggbb`,
// `ansi256(n)` and `ansi:name`. Three of the four are unreadable as text: an
// index into the 256-colour palette says nothing about what colour it is, and
// neither does a hex triple to most readers. So a field the template typed as
// a colour gets a square of that colour beside its value.
//
// The same square goes beside any structure the template says is a colour: a
// PNG palette entry or pixel, a GIF colour table entry. The core works out what
// colour that is (`TemplateNode.colour`), as `#rrggbb` or `#rrggbbaa`, and a
// colour with an alpha is drawn over a checkerboard so that see-through reads
// as see-through rather than as pale.
//
// Nothing here guesses. A value that is not one of the four syntaxes gets no
// swatch, which is the honest answer for a colour the file wrote wrongly, and
// a structure the template did not call a colour gets none either, whatever
// its fields are called.

/** The sixteen colours an ANSI name refers to, as the palette xterm uses. */
const ANSI16: readonly [number, number, number][] = [
  [0, 0, 0],
  [205, 0, 0],
  [0, 205, 0],
  [205, 205, 0],
  [0, 0, 238],
  [205, 0, 205],
  [0, 205, 205],
  [229, 229, 229],
  [127, 127, 127],
  [255, 0, 0],
  [0, 255, 0],
  [255, 255, 0],
  [92, 92, 255],
  [255, 0, 255],
  [0, 255, 255],
  [255, 255, 255],
];

const ANSI_NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

/** The steps of the 6x6x6 cube that fills the middle of the 256-colour palette. */
const CUBE = [0, 95, 135, 175, 215, 255];

/** One entry of the 256-colour palette, as the terminal draws it. */
function ansi256(n: number): [number, number, number] | null {
  if (!Number.isInteger(n) || n < 0 || n > 255) return null;
  if (n < 16) return ANSI16[n] ?? null;
  if (n < 232) {
    const i = n - 16;
    const r = CUBE[Math.floor(i / 36)];
    const g = CUBE[Math.floor(i / 6) % 6];
    const b = CUBE[i % 6];
    return r === undefined || g === undefined || b === undefined ? null : [r, g, b];
  }
  const grey = 8 + (n - 232) * 10;
  return [grey, grey, grey];
}

/** An ANSI colour by name: the eight names, and the bright half of each. */
function byName(name: string): [number, number, number] | null {
  const lower = name.toLowerCase();
  const bright = lower.startsWith("bright");
  const bare = bright ? lower.slice(6).replace(/^[-_]/, "") : lower;
  const i = ANSI_NAMES.indexOf(bare as (typeof ANSI_NAMES)[number]);
  if (i < 0) return null;
  return ANSI16[i + (bright ? 8 : 0)] ?? null;
}

/**
 * What a colour written one of the four ways comes to, as CSS. Null for text
 * that is not a colour, which is what a swatch is left off for.
 */
export const cssColour = (text: string): string | null => {
  const value = text.trim();
  const hex = /^#([0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{3,4})$/i.exec(value);
  if (hex !== null) return value.toLowerCase();
  const rgb = /^rgb\(\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})\s*\)$/i.exec(value);
  if (rgb !== null) {
    const parts = [rgb[1], rgb[2], rgb[3]].map((n) => Number(n));
    if (parts.every((n) => n >= 0 && n <= 255)) return `rgb(${parts.join(",")})`;
    return null;
  }
  const indexed = /^ansi256\(\s*(\d{1,3})\s*\)$/i.exec(value);
  if (indexed !== null) {
    const rgbValue = ansi256(Number(indexed[1]));
    return rgbValue === null ? null : `rgb(${rgbValue.join(",")})`;
  }
  const named = /^ansi:([a-z_-]+)$/i.exec(value);
  if (named !== null) {
    const rgbValue = byName(named[1] ?? "");
    return rgbValue === null ? null : `rgb(${rgbValue.join(",")})`;
  }
  return null;
};

/** Whether a colour as CSS writes it has an alpha: `#rgba` or `#rrggbbaa`. */
const seeThrough = (css: string): boolean => /^#([0-9a-f]{4}|[0-9a-f]{8})$/i.test(css);

/** The square drawn beside a colour's value, or nothing when the text is not
 *  a colour this knows how to read. */
export const swatch = (text: string): HTMLElement | null => {
  const css = cssColour(text);
  if (css === null) return null;
  const box = document.createElement("span");
  box.className = "rp-swatch";
  paint(box, css);
  // The swatch carries no information the value beside it does not, so it is
  // hidden from anything reading the row out.
  box.setAttribute("aria-hidden", "true");
  return box;
};

/** Fill an element with a colour, over the checkerboard where the colour lets
 *  some of what is behind it through. */
export function paint(box: HTMLElement, css: string): void {
  const clear = seeThrough(css);
  box.classList.toggle("is-see-through", clear);
  // The checkerboard is the class's background image, and the shorthand set
  // inline would wipe it, so a see-through colour goes in by a variable.
  if (clear) {
    box.style.removeProperty("background");
    box.style.setProperty("--see-through", css);
  } else {
    box.style.background = css;
  }
}

/** The type name a template gives a field whose value is a colour. */
export const COLOUR_TYPE = "colour";

/** The swatch for a node: the colour a structure says it is, or a field the
 *  template typed as a colour, read from the text it shows. Null for every
 *  other node. */
export function nodeSwatch(n: { readonly type: string; readonly colour?: string }, shown: string): HTMLElement | null {
  if (n.colour !== undefined) return swatch(n.colour);
  return n.type === COLOUR_TYPE ? swatch(shown) : null;
}

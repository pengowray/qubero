// The section under the value editor, for the types that know more than the
// value in front of them: the bytes a magic field wanted, the values an enum
// names, what each bit of a flags field means, and how a float is put
// together. Nothing here holds state; writing a value chosen here goes back
// through the callback the inspector passes in.

import type { TemplateNode, TypeInfo } from "./doc.js";
import { bf16ToNumber, f16ToNumber, numberToBf16, numberToF16 } from "./lenses.js";

/** Write a value the reader picked here rather than typed. */
export type Apply = (path: readonly number[], text: string) => void;

function span(cls: string, text: string): HTMLElement {
  const e = document.createElement("span");
  e.className = cls;
  e.textContent = text;
  return e;
}

function heading(text: string, suffix = ""): HTMLElement {
  const h = document.createElement("div");
  h.className = "insp-type-head";
  h.append(text);
  // The format's own name, kept out of the way of the heading it qualifies.
  if (suffix !== "") h.append(span("insp-type-head-note", suffix));
  return h;
}

function headingFor(info: TypeInfo): string {
  if (info.kind === "magic") return "Expected bytes";
  if (info.kind === "flags") return "Flags";
  if (info.kind === "float") return "Bit layout";
  return `Defined values (${info.cases.length})`;
}

/**
 * What one binary float is made of. The three fields are fixed by the width,
 * and everything the panel says is worked out from them.
 */
type FloatShape = { exp: number; sig: number; bias: number; name: string };

/** Keyed by the name of the layout rather than by its width: a brain float and
 *  a half float are both sixteen bits and divide them differently. */
const FLOATS: Record<string, FloatShape> = {
  binary16: { exp: 5, sig: 10, bias: 15, name: "binary16" },
  bfloat16: { exp: 8, sig: 7, bias: 127, name: "bfloat16" },
  binary32: { exp: 8, sig: 23, bias: 127, name: "binary32" },
  binary64: { exp: 11, sig: 52, bias: 1023, name: "binary64" },
};

/**
 * The shortest decimal that reads back as the same number in this layout. A f32
 * pi is 3.1415927, not the 3.1415927410125732 its widening to a double prints:
 * the digits past the seventh are an artefact of the reading, not the file.
 */
function shortest(x: number, format: string): string {
  if (!Number.isFinite(x)) return x > 0 ? "Infinity" : x < 0 ? "-Infinity" : "NaN";
  const same = (t: string): boolean => {
    const v = Number(t);
    if (format === "binary64") return v === x;
    if (format === "binary32") return Math.fround(v) === x;
    if (format === "bfloat16") return bf16ToNumber(numberToBf16(v)) === x;
    return f16ToNumber(numberToF16(v)) === x;
  };
  const digits = format === "binary64" ? 17 : format === "binary32" ? 9 : format === "bfloat16" ? 4 : 5;
  for (let p = 1; p < digits; p++) {
    const t = Number(x.toPrecision(p));
    if (same(String(t))) return String(t);
  }
  return String(x);
}

/** `2^-126`, with the power raised rather than written with a caret. */
function power(e: number): DocumentFragment {
  const frag = document.createDocumentFragment();
  const sup = document.createElement("sup");
  sup.textContent = String(e);
  frag.append("2", sup);
  return frag;
}

/** The field's bits, in fours from the low end, the way they are read. */
function clusters(digits: string): HTMLElement[] {
  const out: HTMLElement[] = [];
  // The odd bits go in the first cluster, so the fours line up from the low
  // end: an 11-bit exponent reads 100 0000 0000.
  let at = digits.length % 4 === 0 ? 4 : digits.length % 4;
  let i = 0;
  while (i < digits.length) {
    const cluster = document.createElement("span");
    cluster.className = "insp-fcluster";
    for (const d of digits.slice(i, i + at)) {
      cluster.append(span(d === "1" ? "insp-bit-on" : "insp-bit-off", d));
    }
    out.push(cluster);
    i += at;
    at = 4;
  }
  return out;
}

function bitGroup(label: string, digits: string): HTMLElement {
  const g = document.createElement("div");
  g.className = "insp-fgroup";
  g.append(span("insp-fgroup-label", label));
  const row = document.createElement("div");
  row.className = "insp-fbits";
  row.append(...clusters(digits));
  g.append(row);
  return g;
}

/** One `Exponent  2^1 (stored 128 - bias 127)` line. */
function floatRow(label: string, ...value: (Node | string)[]): HTMLElement {
  const row = document.createElement("div");
  row.className = "insp-frow";
  row.append(span("insp-frow-label", label));
  const v = document.createElement("span");
  v.className = "insp-frow-value";
  v.append(...value);
  row.append(v);
  return row;
}

/**
 * A float taken apart: which bits are the sign, the exponent and the
 * significand, what each of them says, and the number they add up to.
 */
function floatBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  const shape = FLOATS[info.format];
  if (shape === undefined) return frag;
  const width = info.width;
  const raw = BigInt(`0x${info.pattern === "" ? "0" : info.pattern}`);
  const digits = raw.toString(2).padStart(width, "0");
  const negative = digits[0] === "1";
  const expDigits = digits.slice(1, 1 + shape.exp);
  const sigDigits = digits.slice(1 + shape.exp);
  const stored = parseInt(expDigits, 2);
  const frac = BigInt(`0b${sigDigits}`);

  const groups = document.createElement("div");
  groups.className = "insp-fgroups";
  groups.append(bitGroup("sign", digits[0] ?? "0"), bitGroup("exponent", expDigits), bitGroup("significand", sigDigits));
  frag.append(groups);

  const special = (text: string, ...rest: HTMLElement[]): DocumentFragment => {
    const p = document.createElement("p");
    p.className = "insp-fspecial";
    p.textContent = text;
    frag.append(p, ...rest);
    return frag;
  };
  const muted = (text: string): HTMLElement => {
    const e = document.createElement("p");
    e.className = "insp-type-note";
    e.textContent = text;
    return e;
  };

  const top = stored === (1 << shape.exp) - 1;
  if (top && frac === 0n) {
    return special(negative ? "-Infinity (exponent all 1s, significand 0)" : "Infinity (exponent all 1s, significand 0)");
  }
  if (top) {
    const quiet = sigDigits[0] === "1";
    return special(
      quiet ? "Quiet NaN" : "Signaling NaN",
      muted("exponent all 1s, significand not 0"),
      muted(`significand 0x${frac.toString(16)}`),
    );
  }
  if (stored === 0 && frac === 0n) {
    return special(negative ? "Negative zero (only the sign bit is set)" : "Zero (all bits 0)");
  }

  // A subnormal has no leading 1 and does not step its exponent down with the
  // stored zero: it stays at the smallest a normal number reaches.
  const subnormal = stored === 0;
  const e = subnormal ? 1 - shape.bias : stored - shape.bias;
  const fraction = Number(frac) / 2 ** shape.sig;
  const significand = (subnormal ? 0 : 1) + fraction;
  const value = (negative ? -1 : 1) * significand * 2 ** e;

  const rows = document.createElement("div");
  rows.className = "insp-frows";
  rows.append(
    floatRow("Sign", negative ? "-" : "+", span("insp-frow-note", negative ? " negative" : " positive")),
    floatRow(
      "Exponent",
      power(e),
      span("insp-frow-note", subnormal ? " (stored 0, subnormal)" : ` (stored ${stored} - bias ${shape.bias})`),
    ),
    floatRow(
      "Significand",
      shortest(significand, info.format),
      span("insp-frow-note", subnormal ? " (no leading 1)" : " (leading 1 not stored)"),
    ),
  );
  frag.append(rows);

  const line = document.createElement("div");
  line.className = "insp-fvalue";
  line.append(`${negative ? "-" : ""}${shortest(significand, info.format)} × `, power(e), ` = ${shortest(value, info.format)}`);
  frag.append(line);
  return frag;
}

/** An enum's numbers are read the way the format writes them. */
function showNumber(v: number, hex: boolean): string {
  return hex && v >= 0 ? `0x${v.toString(16)}` : String(v);
}

function hexOf(bytes: readonly number[]): string {
  return bytes.map((b) => b.toString(16).padStart(2, "0").toUpperCase()).join(" ");
}

/** The hex view's convention: anything unprintable shows as a dot. */
function textOf(bytes: readonly number[]): string {
  return bytes.map((b) => (b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".")).join("");
}

/**
 * The bytes the format wanted, and when they are not the bytes that are there,
 * both of them lined up so the difference is where the reader is looking.
 */
function magicBody(info: TypeInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  const same =
    info.expected.length === info.actual.length && info.expected.every((b, i) => b === info.actual[i]);
  const row = (label: string, bytes: readonly number[]): HTMLElement => {
    const e = document.createElement("div");
    e.className = "insp-bytes";
    if (label !== "") e.append(span("insp-bytes-label", label));
    e.append(span("insp-bytes-hex", hexOf(bytes)), span("insp-bytes-text", textOf(bytes)));
    return e;
  };
  if (same) {
    frag.append(row("", info.expected));
    return frag;
  }
  frag.append(row("Expected", info.expected), row("In file", info.actual));
  const n = info.expected.filter((b, i) => b !== info.actual[i]).length;
  const total = info.expected.length;
  const note = document.createElement("p");
  note.className = "insp-type-note";
  note.textContent = `${n} of ${total} bytes ${n === 1 ? "differs" : "differ"}.`;
  frag.append(note);
  return frag;
}

/** Every value the enum names, the one in the file marked, click to apply. */
function enumBody(info: TypeInfo, path: readonly number[], apply: Apply): DocumentFragment {
  const frag = document.createDocumentFragment();
  const known = info.cases.some((c) => c.value === info.current);
  if (!known) {
    const line = document.createElement("p");
    line.className = "insp-type-note";
    line.textContent = `${showNumber(info.current, info.hex)} is not a defined value.`;
    frag.append(line);
  }
  const list = document.createElement("div");
  list.className = "insp-cases";
  for (const c of info.cases) {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "insp-case";
    const here = c.value === info.current;
    if (here) row.classList.add("is-current");
    row.setAttribute("aria-current", String(here));
    const mark = document.createElement("span");
    mark.className = "insp-mark";
    mark.textContent = here ? "\u2713" : "";
    const name = document.createElement("span");
    name.className = "insp-case-name";
    name.textContent = c.name;
    const num = document.createElement("span");
    num.className = "insp-case-num";
    num.textContent = showNumber(c.value, info.hex);
    row.append(mark, name, num);
    row.addEventListener("click", () => apply(path, String(c.value)));
    list.append(row);
  }
  frag.append(list);
  return frag;
}

/** Each bit of the field: whether it is set, and what it is called. */
function flagsBody(info: TypeInfo, path: readonly number[], n: TemplateNode, apply: Apply): DocumentFragment {
  const frag = document.createDocumentFragment();
  const width = info.bits.length;
  const readout = document.createElement("div");
  readout.className = "insp-bin";
  const ends = document.createElement("div");
  ends.className = "insp-bin-ends";
  ends.append(span("insp-bin-hi", String(width - 1)), span("insp-bin-lo", "0"));
  // Most significant first, in groups of four, so it lines up with the way
  // the same number is read in hex.
  const digits = document.createElement("div");
  digits.className = "insp-bin-digits";
  for (let i = width - 1; i >= 0; i--) {
    const set = info.bits[i]?.set === true;
    const d = span(set ? "insp-bit-on" : "insp-bit-off", set ? "1" : "0");
    digits.append(d);
    if (i % 4 === 0 && i > 0) digits.append(document.createTextNode(" "));
  }
  readout.append(ends, digits);
  frag.append(readout);

  const list = document.createElement("div");
  list.className = "insp-bits";
  let i = 0;
  while (i < width) {
    const bit = info.bits[i];
    if (bit === undefined) break;
    // A run of unnamed bits that are all off is one line, not five.
    if (bit.name === null && !bit.set) {
      let j = i;
      while (j < width && info.bits[j]?.name === null && info.bits[j]?.set === false) j++;
      if (j - i >= 3) {
        const row = document.createElement("div");
        row.className = "insp-bit-run";
        row.textContent = `bits ${i}-${j - 1} \u00b7 unnamed, none set`;
        list.append(row);
        i = j;
        continue;
      }
    }
    list.append(bitRow(bit, path, n, apply));
    i++;
  }
  frag.append(list);
  return frag;
}

function bitRow(
  bit: { bit: number; name: string | null; set: boolean },
  path: readonly number[],
  n: TemplateNode,
  apply: Apply,
): HTMLElement {
  const row = document.createElement("label");
  row.className = "insp-bit";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = bit.set;
  box.disabled = !n.editable;
  box.addEventListener("change", () => {
    const raw = BigInt(n.edit_text || "0");
    const mask = 1n << BigInt(bit.bit);
    apply(path, String(box.checked ? raw | mask : raw & ~mask));
  });
  const name = document.createElement("span");
  name.className = "insp-bit-name";
  name.textContent = bit.name ?? `bit ${bit.bit} (unnamed)`;
  const num = document.createElement("span");
  num.className = "insp-bit-num";
  num.textContent = `bit ${bit.bit}`;
  row.append(box, name, num);
  return row;
}

/**
 * The whole section, heading and all. Empty for a type whose value already
 * says everything, which the caller checks before asking.
 */
export function typePanel(
  info: TypeInfo,
  path: readonly number[],
  n: TemplateNode,
  apply: Apply,
): DocumentFragment {
  const frag = document.createDocumentFragment();
  frag.append(heading(headingFor(info), info.kind === "float" ? (FLOATS[info.width]?.name ?? "") : ""));
  if (info.kind === "float") frag.append(floatBody(info));
  else if (info.kind === "magic") frag.append(magicBody(info));
  else if (info.kind === "enum") frag.append(enumBody(info, path, apply));
  else frag.append(flagsBody(info, path, n, apply));
  return frag;
}

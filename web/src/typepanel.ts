// The section under the value editor, for the types that know more than the
// value in front of them: the bytes a magic field wanted, the values an enum
// names, what each bit of a flags field means, and how a float is put
// together. Nothing here holds state; writing a value chosen here goes back
// through the callback the inspector passes in.

import type { EnumInfo, FixedInfo, FlagsInfo, FloatInfo, MagicInfo, TemplateNode, TypeInfo } from "./doc.ts";
import { bf16ToNumber, f16ToNumber, numberToBf16, numberToF16 } from "./lenses.ts";
import { quantBody, type GoTo } from "./quantpanel.ts";
import { xrefBody, xrefNote } from "./xrefpanel.ts";
import { chunkBody, chunkNote } from "./chunkpanel.ts";
import { vectorBody, vectorNote } from "./vectorpanel.ts";
import { gribBody, gribNote } from "./gribpanel.ts";
import { pageBody, pageNote } from "./pagepanel.ts";
import { tileBody, tileNote } from "./tilepanel.ts";
import { bufrBody, bufrNote } from "./bufrpanel.ts";
import { objstmBody, objstmNote } from "./objstmpanel.ts";
import { rowBody, rowNote } from "./rowpanel.ts";
import { samplesBody, samplesNote } from "./samplespanel.ts";

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

/** A kind that has a panel. */
type Shown = Exclude<TypeInfo, { kind: "plain" }>;

function headingFor(info: Shown): string {
  switch (info.kind) {
    case "magic":
      return "Expected bytes";
    case "flags":
      return "Flags";
    case "float":
    case "fixed":
      return "Bit layout";
    case "quant":
      return "Weights";
    case "xref":
      return "Cross-reference rows";
    case "objstm":
      return "Objects in this stream";
    case "sqliterow":
      return "Columns in this row";
    case "chunk":
      return "Inside this chunk";
    case "vector":
      return "Inside this vector";
    case "grib":
      return "Inside section 7";
    case "samples":
      return "Samples in this record";
    case "page":
      return "Inside this page";
    case "tile":
      return "Inside this tile";
    case "bufr":
      return "Values in this message";
    case "enum":
      return `Defined values (${info.cases.length})`;
  }
}

/** The format's own name for what the heading is about, where it has one. */
function headingNote(info: Shown): string {
  switch (info.kind) {
    case "float":
      return FLOATS[info.format]?.name ?? "";
    case "fixed":
      return `${info.bits - info.frac}.${info.frac} fixed point, ${info.signed ? "signed" : "unsigned"}`;
    case "quant":
      return info.name;
    case "xref":
      return xrefNote(info);
    case "objstm":
      return objstmNote(info);
    case "sqliterow":
      return rowNote(info);
    case "chunk":
      return chunkNote(info);
    case "vector":
      return vectorNote(info);
    case "grib":
      return gribNote(info);
    case "samples":
      return samplesNote(info);
    case "page":
      return pageNote(info);
    case "tile":
      return tileNote(info);
    case "bufr":
      return bufrNote(info);
    default:
      return "";
  }
}

/**
 * What one binary float is made of. The three fields are fixed by the width,
 * and everything the panel says is worked out from them.
 */
type FloatShape = {
  exp: number;
  sig: number;
  bias: number;
  name: string;
  /** True for a layout with no infinities, where the top exponent still holds
   *  numbers and only an all-ones significand is not one. The eight-bit e4m3
   *  is the only one here that works this way. */
  finite?: boolean;
  /** True when the significand's leading bit is written rather than assumed:
   *  the x87 extended float keeps it as its bit 63. */
  explicit?: boolean;
  /** True for IBM's hexadecimal float, which scales by 16 rather than 2,
   *  keeps no leading 1, and has no infinities or NaNs at all. */
  hex?: boolean;
};

/** Keyed by the name of the layout rather than by its width: a brain float and
 *  a half float are both sixteen bits and divide them differently. */
const FLOATS: Record<string, FloatShape> = {
  e4m3: { exp: 4, sig: 3, bias: 7, name: "e4m3", finite: true },
  e5m2: { exp: 5, sig: 2, bias: 15, name: "e5m2" },
  binary16: { exp: 5, sig: 10, bias: 15, name: "binary16" },
  bfloat16: { exp: 8, sig: 7, bias: 127, name: "bfloat16" },
  binary32: { exp: 8, sig: 23, bias: 127, name: "binary32" },
  binary64: { exp: 11, sig: 52, bias: 1023, name: "binary64" },
  x87: { exp: 15, sig: 64, bias: 16383, name: "x87 extended", explicit: true },
  ibm32: { exp: 7, sig: 24, bias: 64, name: "IBM hexadecimal", hex: true },
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
    // An x87 or IBM value arrives here as the nearest double already.
    if (format === "binary64" || format === "x87" || format === "ibm32") return v === x;
    if (format === "binary32") return Math.fround(v) === x;
    if (format === "bfloat16") return bf16ToNumber(numberToBf16(v)) === x;
    // Every eight-bit float is a double exactly, so the shortest form is the
    // shortest that comes back as the same number.
    if (format === "e4m3" || format === "e5m2") return v === x;
    return f16ToNumber(numberToF16(v)) === x;
  };
  const digits = format === "binary64" || format === "x87" || format === "ibm32" || format === "e4m3" || format === "e5m2" ? 17 : format === "binary32" ? 9 : format === "bfloat16" ? 4 : 5;
  for (let p = 1; p < digits; p++) {
    const t = Number(x.toPrecision(p));
    if (same(String(t))) return String(t);
  }
  return String(x);
}

/** `2^-126`, with the power raised rather than written with a caret. */
function power(e: number, base = 2): DocumentFragment {
  const frag = document.createDocumentFragment();
  const sup = document.createElement("sup");
  sup.textContent = String(e);
  frag.append(String(base), sup);
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

function muted(text: string): HTMLElement {
  const e = document.createElement("p");
  e.className = "insp-type-note";
  e.textContent = text;
  return e;
}

/** A value the bits name outright, in place of the rows that add one up. */
function special(text: string, ...rest: HTMLElement[]): DocumentFragment {
  const frag = document.createDocumentFragment();
  const p = document.createElement("p");
  p.className = "insp-fspecial";
  p.textContent = text;
  frag.append(p, ...rest);
  return frag;
}

/** The bits of a field, from the hex the bridge sends, padded to its width. */
function bitsOf(pattern: string, width: number): string {
  return BigInt(`0x${pattern === "" ? "0" : pattern}`).toString(2).padStart(width, "0");
}

/** `-1.5 × 2^3 = -12`, or `0.5 × 16^2 = 128` for a hexadecimal float. */
function valueLine(negative: boolean, significand: string, e: number, base: number, value: string): HTMLElement {
  const line = document.createElement("div");
  line.className = "insp-fvalue";
  line.append(`${negative ? "-" : ""}${significand} × `, power(e, base), ` = ${value}`);
  return line;
}

/**
 * A float taken apart: which bits are the sign, the exponent and the
 * significand, what each of them says, and the number they add up to.
 */
function floatBody(info: FloatInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  const shape = FLOATS[info.format];
  if (shape === undefined) return frag;
  if (shape.hex === true) return ibmBody(info, shape);
  if (shape.explicit === true) return x87Body(info, shape);
  const digits = bitsOf(info.pattern, info.width);
  const negative = digits[0] === "1";
  const expDigits = digits.slice(1, 1 + shape.exp);
  const sigDigits = digits.slice(1 + shape.exp);
  const stored = parseInt(expDigits, 2);
  const frac = BigInt(`0b${sigDigits}`);

  const groups = document.createElement("div");
  groups.className = "insp-fgroups";
  groups.append(bitGroup("sign", digits[0] ?? "0"), bitGroup("exponent", expDigits), bitGroup("significand", sigDigits));
  frag.append(groups);

  const top = stored === (1 << shape.exp) - 1;
  const allOnes = frac === (1n << BigInt(shape.sig)) - 1n;
  if (top && shape.finite === true) {
    // This layout spends its top exponent on numbers and keeps one pattern
    // back: everything else there is read as an ordinary number below.
    if (allOnes) {
      frag.append(special("NaN", muted("exponent and significand all 1s"), muted("this layout has no infinities")));
      return frag;
    }
  } else if (top && frac === 0n) {
    frag.append(special(negative ? "-Infinity (exponent all 1s, significand 0)" : "Infinity (exponent all 1s, significand 0)"));
    return frag;
  } else if (top) {
    const quiet = sigDigits[0] === "1";
    frag.append(
      special(
        quiet ? "Quiet NaN" : "Signaling NaN",
        muted("exponent all 1s, significand not 0"),
        muted(`significand 0x${frac.toString(16)}`),
      ),
    );
    return frag;
  }
  if (stored === 0 && frac === 0n) {
    frag.append(special(negative ? "Negative zero (only the sign bit is set)" : "Zero (all bits 0)"));
    return frag;
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
  frag.append(rows, valueLine(negative, shortest(significand, info.format), e, 2, shortest(value, info.format)));
  return frag;
}

/**
 * The x87 extended float: a sign, fifteen bits of exponent, and a 64-bit
 * significand whose leading 1 is written out as its top bit rather than
 * assumed. That bit being there lets the bits say things an IEEE float
 * cannot, and those get a note rather than a number: a pseudo-denormal has
 * the bit set under a zero exponent, an unnormal has it clear under a
 * nonzero one.
 */
function x87Body(info: FloatInfo, shape: FloatShape): DocumentFragment {
  const frag = document.createDocumentFragment();
  const digits = bitsOf(info.pattern, info.width);
  const negative = digits[0] === "1";
  const expDigits = digits.slice(1, 1 + shape.exp);
  const intDigit = digits[1 + shape.exp] ?? "0";
  const fracDigits = digits.slice(2 + shape.exp);
  const stored = parseInt(expDigits, 2);
  const frac = BigInt(`0b${fracDigits}`);
  const intBit = intDigit === "1";

  const groups = document.createElement("div");
  groups.className = "insp-fgroups";
  groups.append(bitGroup("sign", digits[0] ?? "0"), bitGroup("exponent", expDigits), bitGroup("integer bit", intDigit), bitGroup("fraction", fracDigits));
  frag.append(groups);

  const top = stored === (1 << shape.exp) - 1;
  if (top && frac === 0n) {
    frag.append(special(negative ? "-Infinity (exponent all 1s, fraction 0)" : "Infinity (exponent all 1s, fraction 0)"));
    if (!intBit) frag.append(muted("integer bit 0: a pseudo-infinity, which modern x86 treats as invalid"));
    return frag;
  }
  if (top) {
    const quiet = fracDigits[0] === "1";
    frag.append(special(quiet ? "Quiet NaN" : "Signaling NaN", muted("exponent all 1s, fraction not 0"), muted(`fraction 0x${frac.toString(16)}`)));
    if (!intBit) frag.append(muted("integer bit 0: a pseudo-NaN, which modern x86 treats as invalid"));
    return frag;
  }
  if (stored === 0 && !intBit && frac === 0n) {
    frag.append(special(negative ? "Negative zero (only the sign bit is set)" : "Zero (all bits 0)"));
    return frag;
  }

  // The exponent of a denormal is the smallest a normal has, and so is a
  // pseudo-denormal's: its integer bit is set, so it is a normal-sized
  // number the hardware would have written with exponent 1.
  const e = stored === 0 ? 1 - shape.bias : stored - shape.bias;
  // 63 bits of fraction into a double: the low ten can round away.
  const fraction = Number(frac) / 2 ** 63;
  const significand = (intBit ? 1 : 0) + fraction;
  const value = (negative ? -1 : 1) * significand * 2 ** e;
  const rounded = (frac & 0x3ffn) !== 0n;

  const rows = document.createElement("div");
  rows.className = "insp-frows";
  rows.append(
    floatRow("Sign", negative ? "-" : "+", span("insp-frow-note", negative ? " negative" : " positive")),
    floatRow("Exponent", power(e), span("insp-frow-note", stored === 0 ? " (stored 0, denormal)" : ` (stored ${stored} - bias ${shape.bias})`)),
    floatRow("Significand", shortest(significand, info.format), span("insp-frow-note", intBit ? " (leading 1 stored)" : " (integer bit 0)")),
  );
  frag.append(rows, valueLine(negative, shortest(significand, info.format), e, 2, shortest(value, info.format)));
  if (rounded) frag.append(muted("shown to double precision: the lowest fraction bits do not fit"));
  if (stored === 0 && intBit) frag.append(muted("pseudo-denormal: the integer bit is set with exponent 0. Modern x86 reads it as a normal number"));
  if (stored !== 0 && !intBit) frag.append(muted("unnormal: the integer bit is 0 with a nonzero exponent. Modern x86 treats it as invalid"));
  return frag;
}

/**
 * IBM's hexadecimal float: a sign, a seven-bit exponent of sixteen, and a
 * fraction with the binary point in front of it and no leading 1 assumed.
 * Nothing is special: every bit pattern is a number.
 */
function ibmBody(info: FloatInfo, shape: FloatShape): DocumentFragment {
  const frag = document.createDocumentFragment();
  const digits = bitsOf(info.pattern, info.width);
  const negative = digits[0] === "1";
  const expDigits = digits.slice(1, 1 + shape.exp);
  const fracDigits = digits.slice(1 + shape.exp);
  const stored = parseInt(expDigits, 2);
  const frac = BigInt(`0b${fracDigits}`);

  const groups = document.createElement("div");
  groups.className = "insp-fgroups";
  groups.append(bitGroup("sign", digits[0] ?? "0"), bitGroup("exponent", expDigits), bitGroup("fraction", fracDigits));
  frag.append(groups);

  if (frac === 0n) {
    frag.append(special(negative ? "Negative zero (fraction 0)" : "Zero (fraction 0)"));
    if (stored !== 0) frag.append(muted(`exponent ${stored} is ignored: a zero fraction is zero at any exponent`));
    return frag;
  }
  const e = stored - shape.bias;
  const fraction = Number(frac) / 2 ** shape.sig;
  const value = (negative ? -1 : 1) * fraction * 16 ** e;
  const rows = document.createElement("div");
  rows.className = "insp-frows";
  rows.append(
    floatRow("Sign", negative ? "-" : "+", span("insp-frow-note", negative ? " negative" : " positive")),
    floatRow("Exponent", power(e, 16), span("insp-frow-note", ` (stored ${stored} - bias ${shape.bias})`)),
    floatRow("Fraction", shortest(fraction, info.format), span("insp-frow-note", " (no leading 1, point in front)")),
  );
  frag.append(rows, valueLine(negative, shortest(fraction, info.format), e, 16, shortest(value, info.format)));
  if (fracDigits.startsWith("0000")) frag.append(muted("not normalised: the top hex digit of the fraction is 0"));
  return frag;
}

/**
 * A fixed-point number taken apart: the bits above the binary point, the
 * bits below it, and the two parts they add up to. A negative two's
 * complement value reads the same way, with a negative integer part and a
 * positive fraction: 0xFFFF8000 in 16.16 is -1 + 0.5.
 */
function fixedBody(info: FixedInfo): DocumentFragment {
  const frag = document.createDocumentFragment();
  const digits = bitsOf(info.pattern, info.bits);
  const intBits = info.bits - info.frac;
  const intDigits = digits.slice(0, intBits);
  const fracDigits = digits.slice(intBits);
  const negative = info.signed && intDigits[0] === "1";
  const rawInt = intBits === 0 ? 0n : BigInt(`0b${intDigits}`);
  const whole = negative ? rawInt - (1n << BigInt(intBits)) : rawInt;
  const frac = info.frac === 0 ? 0n : BigInt(`0b${fracDigits}`);
  const scale = 2 ** info.frac;
  const fraction = Number(frac) / scale;
  const value = Number(whole) + fraction;

  const groups = document.createElement("div");
  groups.className = "insp-fgroups";
  if (info.signed && intBits > 0) {
    groups.append(bitGroup("sign", intDigits[0] ?? "0"));
    if (intBits > 1) groups.append(bitGroup("integer", intDigits.slice(1)));
  } else if (intBits > 0) {
    groups.append(bitGroup("integer", intDigits));
  }
  if (info.frac > 0) groups.append(bitGroup("fraction", fracDigits));
  frag.append(groups);

  const rows = document.createElement("div");
  rows.className = "insp-frows";
  if (info.signed) rows.append(floatRow("Sign", negative ? "-" : "+", span("insp-frow-note", negative ? " negative (two's complement)" : " positive")));
  rows.append(
    floatRow("Integer part", String(whole), span("insp-frow-note", ` (${intBits} bits${negative ? ", two's complement" : ""})`)),
    floatRow("Fraction part", String(fraction), span("insp-frow-note", ` (${frac} / ${scale})`)),
  );
  frag.append(rows);
  const line = document.createElement("div");
  line.className = "insp-fvalue";
  line.append(`${whole} + ${frac}/`, power(info.frac), ` = ${value}`);
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
function magicBody(info: MagicInfo): DocumentFragment {
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
function enumBody(info: EnumInfo, path: readonly number[], apply: Apply): DocumentFragment {
  const frag = document.createDocumentFragment();
  const known = info.cases.some((c) => c.value === info.current);
  if (!known) {
    const line = document.createElement("p");
    line.className = "insp-type-note";
    // A value can be named without being listed: past the values a format
    // names one by one, it starts counting, and the count is the name.
    line.textContent =
      info.named === ""
        ? `${showNumber(info.current, info.hex)} is not a defined value.`
        : `${showNumber(info.current, info.hex)} is ${info.named}.`;
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
function flagsBody(info: FlagsInfo, path: readonly number[], n: TemplateNode, apply: Apply): DocumentFragment {
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
  goTo: GoTo,
  redraw: () => void,
): DocumentFragment {
  const frag = document.createDocumentFragment();
  if (info.kind === "plain") return frag;
  frag.append(heading(headingFor(info), headingNote(info)));
  frag.append(body(info, path, n, apply, goTo, redraw));
  return frag;
}

/** Everything under the heading, from whichever panel the kind belongs to. */
function body(
  info: Shown,
  path: readonly number[],
  n: TemplateNode,
  apply: Apply,
  goTo: GoTo,
  redraw: () => void,
): DocumentFragment {
  switch (info.kind) {
    case "quant":
      return quantBody(info, goTo, redraw);
    case "xref":
      return xrefBody(info, goTo);
    case "objstm":
      return objstmBody(info);
    case "sqliterow":
      return rowBody(info);
    case "chunk":
      return chunkBody(info);
    case "vector":
      return vectorBody(info);
    case "grib":
      return gribBody(info);
    case "samples":
      return samplesBody(info);
    case "page":
      return pageBody(info);
    case "tile":
      return tileBody(info);
    case "bufr":
      return bufrBody(info);
    case "float":
      return floatBody(info);
    case "fixed":
      return fixedBody(info);
    case "magic":
      return magicBody(info);
    case "enum":
      return enumBody(info, path, apply);
    case "flags":
      return flagsBody(info, path, n, apply);
  }
}

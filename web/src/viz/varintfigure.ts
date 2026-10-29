// Varint byte-gathering figure.
//
// Candidate `varint-gather` in docs/DESIGN-diagrams.md, section 4.2; built by
// agent A of the first batch (section 5.3). An inspector section, and a row
// glyph in the listing (`rowglyphs.ts`).
//
// A variable-length number's bytes do not read as bytes. Each byte of a
// LEB128 spends its top bit saying whether another byte follows and its other
// seven on the number; an EBML integer spends its leading zeros saying how
// many bytes it runs to. So `bb 01` is 187, and a reader shown both and no
// split has to take it on trust. This draws the split the way the protobuf
// documentation does: the stored bytes in a row, each written high bit first
// with its hex above it and every bit coloured by what the decoder does with
// it; under them the number the value bits make, written high bit first; and
// a band from each value bit to its place. LEB128 stores the low group first,
// so its bands cross; VLQ, SQLite and EBML store the high group first, and
// theirs run straight. Under the figure is the number in decimal, and the
// signed or zigzag reading where the type uses one.
//
// Which bits are framing and which are value comes from the core
// (`Span.bits`, see `varintbits.rs`). How the value groups join is the type's
// (`orderFor` in `bitlayout.ts`), and the join is checked against the value
// the core read before any band is drawn: if the bits do not make that
// number, only the bytes are drawn and the figure says so.
//
// Explored by pointing: a value bit, its band or its place in the number
// lights its whole group at both ends and says how far the group is shifted;
// a framing bit says what it means. Either marks the bits in the hex view, and
// a click goes to them. No zoom: a varint is at most ten bytes, drawn at a
// fixed pitch no wider than the panel.

import "./bits.css";
import type { BitRoles, Doc, Span, TemplateNode } from "../doc.ts";
import { formatOffset } from "../format.ts";
import { bandPath } from "../report/ribbon.ts";
import { REPORT, VIZ } from "../strings.ts";
import { checkedGather, readAs, type Gather, type GatherOrder, type Reading } from "./bitlayout.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { linkedFor } from "./linked.ts";
import { goToBits } from "./pick.ts";
import { registerRowGlyph } from "./rowglyphs.ts";
import { hideTip, hoverTip, tipCard } from "./tip.ts";

const T = VIZ.varint;

/** The type names the core gives variable-length numbers (`template.rs`,
 *  `Ty::name`). A template's own alias for one is not here, and gets no
 *  figure; the core's split would still be on the span for the bytes strip. */
const VARINT_TYPES: ReadonlySet<string> = new Set(["leb128", "sleb128", "zigzag", "vlq", "varint", "sqlite varint", "EBML ID", "EBML size", "7z number"]);
/** Longest varint the core splits (`MAX_BYTES` in `varintbits.rs`) plus one
 *  for a LEB128 of 64 bits, which can run to ten bytes. */
const MAX_BYTES = 10;

export type VarintData = {
  readonly node: TemplateNode;
  readonly roles: BitRoles;
  /** The join, checked against the value; null when no order gives it. */
  readonly gather: Gather | null;
  readonly order: GatherOrder | null;
  readonly reading: Reading;
};

/** The span that carries the field's bit roles, or null while it is being
 *  read. */
function spanOf(doc: Doc, n: TemplateNode): Span | null | "none" {
  const reply = doc.spans(n.offset_bits, n.offset_bits + n.size_bits, 8);
  if (reply.status !== "ok") return null;
  const s = reply.node.find((x) => x.offset_bits === n.offset_bits && x.size_bits === n.size_bits && x.bits !== null);
  return s ?? "none";
}

export function varintData(doc: Doc, n: TemplateNode): VarintData | null {
  const s = spanOf(doc, n);
  if (s === null || s === "none" || s.bits === null) return null;
  const roles = s.bits;
  const checked = checkedGather(roles.groups, n.type, roles.rule, n.value);
  return { node: n, roles, gather: checked?.gather ?? null, order: checked?.order ?? null, reading: checked?.reading ?? "unsigned" };
}

/** Heights of the figure's parts, in pixels. */
const HEX_H = 13;
const CELL_H = 22;
const GAP_H = 34;
/** The widest and narrowest a bit is drawn, and the narrowest that still
 *  carries its digit. */
const MAX_BIT = 16;
const MIN_BIT = 7;
const DIGIT_MIN = 10;
/** How much of a bit's width its band takes at each end. */
const STRAND = 0.4;

const SVG = "http://www.w3.org/2000/svg";
function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

export type VarintFigure = { readonly el: HTMLElement; destroy(): void };

/** Draw the figure no wider than `width` pixels. */
export function drawVarint(doc: Doc, data: VarintData, opts: { readonly width: number }): VarintFigure {
  const n = data.node;
  const groups = data.roles.groups;
  const g = data.gather;
  const bytes = Math.ceil(groups.reduce((t, x) => t + x.bits.length, 0) / 8);
  const fig = document.createElement("figure");
  fig.className = "vb-fig vg-fig";
  const head = document.createElement("figcaption");
  head.className = "vb-head";
  const title = document.createElement("span");
  title.className = "vb-title";
  title.textContent = T.title(n.type, n.name);
  head.append(title);

  // Units: a bit is one, and half a bit between two bytes (along the top) or
  // two groups (along the bottom), so each reads as one.
  const joined = g?.joined ?? [];
  const bottomUnits = joined.reduce((t, gi) => t + (groups[gi]?.bits.length ?? 0), 0) + Math.max(0, joined.length - 1) * 0.5;
  const topUnits = bytes * 8 + (bytes - 1) * 0.5;
  const units = Math.max(topUnits, bottomUnits);
  const pitch = Math.max(MIN_BIT, Math.min(MAX_BIT, Math.floor(opts.width / units)));
  const width = Math.ceil(units * pitch) + 1;
  const height = HEX_H + CELL_H + (g === null ? 0 : GAP_H + CELL_H) + 2;
  const root = svg("svg", { width, height, class: "vb-svg", role: "img", "aria-label": title.textContent });
  const topX = (byte: number, slot: number): number => (byte * 8.5 + slot) * pitch;
  const yCells = HEX_H;
  const yBottom = HEX_H + CELL_H + GAP_H;

  // Where each value bit lands along the bottom: its group's start plus its
  // place inside the group.
  const groupStart = new Map<number, number>();
  {
    let at = 0;
    for (const gi of g?.joined ?? []) {
      groupStart.set(gi, at);
      at += (groups[gi]?.bits.length ?? 0) + 0.5;
    }
  }
  const placeInGroup = new Map<number, number>();
  {
    const seen = new Map<number, number>();
    g?.stored.forEach((b, i) => {
      const k = seen.get(b.group) ?? 0;
      seen.set(b.group, k + 1);
      placeInGroup.set(i, k);
    });
  }
  const bottomX = (i: number): number | null => {
    const b = g?.stored[i];
    if (b === undefined || b.place === null) return null;
    return ((groupStart.get(b.group) ?? 0) + (placeInGroup.get(i) ?? 0)) * pitch;
  };

  // The hex of each byte, over its bits.
  const stored = g?.stored ?? storedOnly(groups);
  for (let byte = 0; byte < bytes; byte++) {
    let v = 0;
    for (const b of stored) if (b.byte === byte) v = v * 2 + b.digit;
    const t = svg("text", { x: topX(byte, 4), y: HEX_H - 3, class: "vg-hex", "text-anchor": "middle" });
    t.textContent = v.toString(16).padStart(2, "0");
    root.append(t);
  }
  const bandsG = svg("g");
  root.append(bandsG);
  const cells: SVGRectElement[] = [];
  const places: SVGRectElement[] = [];
  const bands: SVGPathElement[] = [];
  stored.forEach((b, i) => {
    const x = topX(b.byte, b.slot);
    const r = svg("rect", { x: x + 0.5, y: yCells + 0.5, width: pitch - 1, height: CELL_H - 1, rx: 2, class: `vg-cell is-${b.role}` });
    r.dataset.i = String(i);
    root.append(r);
    cells.push(r);
    if (pitch >= DIGIT_MIN) {
      const t = svg("text", { x: x + pitch / 2, y: yCells + CELL_H / 2 + 0.5, class: `vg-digit${b.role === "payload" ? "" : " is-frame"}`, "text-anchor": "middle", "dominant-baseline": "central" });
      t.textContent = String(b.digit);
      root.append(t);
    }
    const bx = bottomX(i);
    if (bx === null) return;
    const w = pitch * STRAND;
    const p = svg("path", { d: bandPath(x + (pitch - w) / 2, x + (pitch + w) / 2, yCells + CELL_H, bx + (pitch - w) / 2, bx + (pitch + w) / 2, yBottom), class: "vg-band" });
    p.dataset.i = String(i);
    bandsG.append(p);
    bands.push(p);
    const q = svg("rect", { x: bx + 0.5, y: yBottom + 0.5, width: pitch - 1, height: CELL_H - 1, rx: 2, class: "vg-cell is-payload" });
    q.dataset.i = String(i);
    root.append(q);
    places.push(q);
    if (pitch >= DIGIT_MIN) {
      const t = svg("text", { x: bx + pitch / 2, y: yBottom + CELL_H / 2 + 0.5, class: "vg-digit", "text-anchor": "middle", "dominant-baseline": "central" });
      t.textContent = String(b.digit);
      root.append(t);
    }
  });

  const scroller = document.createElement("div");
  scroller.style.overflowX = "auto";
  scroller.append(root);
  const from = formatOffset(n.offset_bits);
  const to = formatOffset(n.offset_bits + (bytes - 1) * 8);
  const line = (cls: string, text: string): HTMLElement => {
    const d = document.createElement("div");
    d.className = cls;
    d.textContent = text;
    return d;
  };
  fig.append(head, line("vb-row", T.stored(from, to)), scroller);
  if (g !== null) {
    fig.append(line("vb-row", T.number(g.width)));
    const raw = g.value.toString();
    const v = readAs(g.value, g.width, data.reading).toString();
    fig.append(line("vb-value", data.reading === "signed" ? T.valueSigned(v, raw) : data.reading === "zigzag" ? T.valueZigzag(v, raw) : T.value(v)));
    const join = g.joined.length < 2 ? "" : data.order === "low-first" ? T.joinLowFirst : data.order === "first-high-rest-low" ? T.joinSevenZip : T.joinHighFirst;
    const rule = REPORT.bitsRule(data.roles.rule);
    const note = [rule === "" ? "" : `${rule.charAt(0).toUpperCase()}${rule.slice(1)}.`, join].filter((x) => x !== "").join(" ");
    if (note !== "") fig.append(line("vb-note", note));
  } else fig.append(line("vb-note", T.unmatched(n.value)));
  fig.append(legend(groups));

  // ----- hover and click -----
  const bus = linkedFor(doc);
  /** The file bits of group `gi`, in the hex view's numbering. */
  const groupBits = (gi: number): { startBit: number; endBit: number } => {
    let at = 0;
    for (let k = 0; k < gi; k++) at += groups[k]?.bits.length ?? 0;
    return { startBit: n.offset_bits + at, endBit: n.offset_bits + at + (groups[gi]?.bits.length ?? 0) };
  };
  const light = (gi: number | null): void => {
    fig.classList.toggle("is-lit", gi !== null);
    const on = (el: SVGElement): boolean => gi !== null && stored[Number(el.dataset.i)]?.group === gi;
    for (const el of [...cells, ...places, ...bands]) el.classList.toggle("is-on", on(el));
  };
  const stopTip = hoverTip(root, (e) => {
    const i = Number((e.target as SVGElement | null)?.dataset?.i ?? Number.NaN);
    const b = stored[i];
    if (b === undefined) {
      light(null);
      bus.publish({ hover: null });
      return null;
    }
    light(b.group);
    const range = groupBits(b.group);
    bus.publish({ hover: range });
    const where = formatOffset(n.offset_bits + b.byte * 8);
    const grp = groups[b.group];
    if (b.role === "more") return tipCard(T.more, [T.moreLine], where);
    if (b.role === "stop") return tipCard(T.stop, [T.stopLine], where);
    if (b.role === "width") return tipCard(T.marker, [REPORT.bitsRule(data.roles.rule)].filter((x) => x !== ""), where);
    const k = g?.joined.indexOf(b.group) ?? -1;
    const shift = k < 0 ? 0 : (g?.shifts[k] ?? 0);
    const value = BigInt(`0b${grp?.bits ?? "0"}`) << BigInt(shift);
    return tipCard(T.groupTitle(b.byte + 1), [T.groupBits(grp?.bits ?? ""), T.shifted(shift, value.toString())], where);
  });
  const leave = (): void => {
    light(null);
    bus.publish({ hover: null });
  };
  root.addEventListener("pointerleave", leave);
  root.addEventListener("click", (e) => {
    const i = Number((e.target as SVGElement | null)?.dataset?.i ?? Number.NaN);
    const b = stored[i];
    if (b === undefined) return;
    goToBits(doc, [groupBits(b.group)]);
  });

  return {
    el: fig,
    destroy: () => {
      stopTip();
      hideTip();
      root.removeEventListener("pointerleave", leave);
    },
  };
}

/** The stored bits when no join could be checked: the same rows, with no
 *  place in a number. */
function storedOnly(groups: BitRoles["groups"]): Gather["stored"] {
  const out: { byte: number; slot: number; digit: 0 | 1; role: BitRoles["groups"][number]["role"]; group: number; place: null }[] = [];
  let at = 0;
  groups.forEach((g, gi) => {
    for (const ch of g.bits) {
      out.push({ byte: Math.floor(at / 8), slot: at % 8, digit: ch === "1" ? 1 : 0, role: g.role, group: gi, place: null });
      at++;
    }
  });
  return out;
}

/** One entry per role the field has, in the colours of its cells. */
function legend(groups: BitRoles["groups"]): HTMLElement {
  const box = document.createElement("div");
  box.className = "vb-legend";
  const roles = new Set<string>(groups.map((g) => (g.role === "stop" ? "more" : g.role)));
  const entry = (role: string, text: string): void => {
    if (!roles.has(role)) return;
    const sw = document.createElementNS(SVG, "svg");
    sw.setAttribute("width", "9");
    sw.setAttribute("height", "9");
    sw.setAttribute("class", "vb-swatch");
    sw.append(svg("rect", { width: 9, height: 9, rx: 2, class: `vg-cell is-${role}` }));
    const item = document.createElement("span");
    item.append(sw, text);
    box.append(item);
  };
  entry("payload", T.payload);
  entry("more", T.continuation);
  entry("width", T.marker);
  return box;
}

// ----- the row glyph -----

const GLYPHS = new WeakMap<Doc, Map<string, HTMLElement>>();

/** The listing's word-sized form: one cell per byte, its continuation bit a
 *  notch at its left end, dark where another byte follows, so a reader sees
 *  at a glance that a number took three bytes. Only for the types that end on
 *  a clear top bit; EBML and 7z say their width another way. */
function varintGlyph(doc: Doc, n: TemplateNode): HTMLElement | null {
  const key = `${n.path.join("/")}|${n.offset_bits}|${n.size_bits}`;
  let cache = GLYPHS.get(doc);
  if (cache === undefined) {
    cache = new Map();
    GLYPHS.set(doc, cache);
  }
  const hit = cache.get(key);
  if (hit !== undefined) return withTip(hit.cloneNode(true) as HTMLElement, n);
  const count = n.size_bits / 8;
  if (!Number.isInteger(count) || count < 1 || count > MAX_BYTES) return null;
  const read = doc.read(n.offset_bits / 8, count);
  if (!read.complete) return null;
  const glyph = document.createElement("span");
  glyph.className = "vb-glyph is-varint";
  for (let i = 0; i < count; i++) {
    const cell = document.createElement("i");
    if (((read.bytes[i] ?? 0) & 0x80) !== 0 && i < count - 1) cell.className = "is-more";
    glyph.append(cell);
  }
  cache.set(key, glyph);
  return withTip(glyph.cloneNode(true) as HTMLElement, n);
}

/** The glyph's hover card: how many bytes, and each byte's split. */
function withTip(glyph: HTMLElement, n: TemplateNode): HTMLElement {
  const count = n.size_bits / 8;
  // Seven value bits a byte, and all eight of SQLite's ninth.
  const width = count * 7 + (count === 9 ? 1 : 0);
  hoverTip(glyph, () => tipCard(T.glyphTitle(count, width), [n.value], formatOffset(n.offset_bits)));
  return glyph;
}

/** Whether a field is a varint whose continuation bits the glyph can draw. */
function highBitType(n: TemplateNode): boolean {
  return n.type === "leb128" || n.type === "sleb128" || n.type === "zigzag" || n.type === "vlq" || n.type === "varint" || n.type === "sqlite varint";
}

/** Put the figure in the At cursor panel, and its glyph on the listing's
 *  rows, for every variable-length number. */
export function register(): void {
  registerInspectorSection({
    id: "varint-gather",
    applies: (_doc, n) => !n.composite && n.space === 0 && VARINT_TYPES.has(n.type) && n.size_bits > 0 && n.size_bits <= MAX_BYTES * 8,
    draw: ({ doc, node, width }) => {
      const data = varintData(doc, node);
      return data === null ? null : drawVarint(doc, data, { width }).el;
    },
  });
  registerRowGlyph({
    id: "varint-gather",
    // One byte says nothing a glyph could: the row's value is the whole story.
    applies: (_doc, n) => !n.composite && n.space === 0 && highBitType(n) && n.size_bits > 8 && n.size_bits <= MAX_BYTES * 8,
    draw: varintGlyph,
  });
}

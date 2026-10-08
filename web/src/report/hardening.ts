// After section 4, for a program: what it was built to protect itself with,
// the checks `checksec` makes (RELRO, a stack canary, NX, PIE, FORTIFY_SOURCE)
// and a few more, each with the bytes its verdict rests on.
//
// The core reads the verdicts (`Editor.hardening`, `formats/hardening.rs`):
// which segments, dynamic entries, symbols, notes and header flags say what.
// This file lays them out. Two tables per program. The first has the
// protections, one row each, in a fixed order so two reports compare row for
// row, with what is off or unknown named again under the heading for a reader
// who skims. The second has how the program loads: its linking, its dynamic
// linker, the libraries it needs and where it looks for them, its symbols.
//
// Which table a row goes in is decided by its key, never by its verdict, so a
// row stays where it was from one file to the next.

import type { HardeningEvidence, HardeningPart, HardeningRow, HardeningVerdict } from "./coredata.ts";
import { ok } from "./model.ts";
import { formatOffset } from "../format.ts";
import { pointAt } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { HV, type Tally } from "./hardeningtext.ts";

/** The templates the core reads protections for. Asking for any other is a
 *  wasted call into the core. */
const PROGRAM_TEMPLATES: ReadonlySet<string> = new Set(["elf", "bpf", "pe", "macho"]);

/** Rows that say how the program loads, rather than whether a protection is
 *  on, in the order the second table lists them: how it is linked and by
 *  what, what it needs and where it looks for it, then what identifies it.
 *  Everything else is a protection. */
const LOADING_ORDER: readonly string[] = [
  "linking",
  "interpreter",
  "needed",
  "soname",
  "rpath",
  "runpath",
  "symbols",
  "build-id",
  "signature",
  "force-integrity",
  "appcontainer",
  "code-signature",
  "encrypted",
  "arc",
  "restrict",
];
const LOADING_KEYS: ReadonlySet<string> = new Set(LOADING_ORDER);
/** States that say a fact is absent. The core points them at where it
 *  looked, which in a table of facts is a link to nothing in particular. */
const ABSENT: ReadonlySet<string> = new Set(["none", "stripped", "not-found", "off"]);

/** Protections few programs use, shown only when the program does. */
const SHOWN_WHEN_FOUND: ReadonlySet<string> = new Set(["safestack", "cfi"]);

/** Protections the processor enforces, which the note under the table is
 *  about. */
const HARDWARE_KEYS: ReadonlySet<string> = new Set(["ibt", "shstk", "bti", "pac"]);

/** Evidence links given to one row before the rest are counted. */
const EVIDENCE_SHOWN = 4;
/** Names listed inline in a loading fact before the rest open from a count. */
const ITEMS_INLINE = 6;

const GLYPH: Readonly<Record<HardeningVerdict, string>> = { good: "✓", partial: "◐", bad: "✗", unknown: "?", "n/a": "–", info: "" };

export const hardeningSection: Section = {
  id: "hardening",
  render(ctx: ReportCtx): Rendered {
    const template = ctx.doc.template;
    if (template === null || !PROGRAM_TEMPLATES.has(template)) return null;
    const h = ctx.data.memo("hardening", () => ok(ctx.doc.hardening()));
    if (h === WAIT) return WAIT;
    if (h === null || h.parts.length === 0) return null;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-hardening";
    const several = h.parts.length > 1;
    const all = tally(h.parts.flatMap((p) => protections(p)));
    const h2 = document.createElement("h2");
    h2.textContent = several ? HV.headingSlices(h.parts.length, all) : HV.heading(all);
    sec.append(h2);
    if (!several) sec.append(...lede(all, h.parts[0]?.rows ?? []));
    sec.append(Object.assign(document.createElement("p"), { className: "rv-hint", textContent: HV.source }));
    for (const part of h.parts) {
      if (several) {
        const h3 = document.createElement("h3");
        const own = tally(protections(part));
        h3.textContent = HV.sliceHeading(part.name, own);
        sec.append(h3, ...lede(own, part.rows));
      }
      sec.append(protectionTable(part), loadingTable(part));
    }
    const shown = h.parts.flatMap((p) => protections(p));
    if (shown.some((r) => r.key === "fortify" && r.total !== null && r.total > 0)) {
      sec.append(Object.assign(document.createElement("p"), { className: "rv-hint", textContent: HV.fortifyNote }));
    }
    if (h.format === "elf" && shown.some((r) => HARDWARE_KEYS.has(r.key))) {
      sec.append(Object.assign(document.createElement("p"), { className: "rv-hint", textContent: HV.hardwareNote }));
    }
    return sec;
  },
};

function protections(part: HardeningPart): HardeningRow[] {
  return part.rows.filter((r) => !LOADING_KEYS.has(r.key) && !(SHOWN_WHEN_FOUND.has(r.key) && r.state !== "found"));
}

function tally(rows: readonly HardeningRow[]): Tally {
  const t = { good: 0, partial: 0, bad: 0, unknown: 0, total: 0, off: [] as string[], partly: [] as string[], unknownKeys: [] as string[] };
  for (const r of rows) {
    if (r.verdict === "n/a" || r.verdict === "info") continue;
    t.total++;
    if (r.verdict === "good") t.good++;
    else if (r.verdict === "partial") {
      t.partial++;
      t.partly.push(HV.short(r.key));
    } else if (r.verdict === "bad") {
      t.bad++;
      t.off.push(HV.short(r.key));
    } else {
      t.unknown++;
      t.unknownKeys.push(HV.short(r.key));
    }
  }
  return t;
}

/** The line under the heading that names what is off, partly on, or unknown,
 *  for a reader who reads no further. Nothing when every protection is on. */
function lede(t: Tally, rows: readonly HardeningRow[]): HTMLElement[] {
  const text = HV.lede(t, rows.some((r) => r.key === "linking" && r.state !== "dynamic"));
  if (text === "") return [];
  return [Object.assign(document.createElement("p"), { className: "rv-hv-lede", textContent: text })];
}

function protectionTable(part: HardeningPart): HTMLElement {
  const rows = protections(part);
  const wrap = document.createElement("div");
  wrap.className = "rv-tablewrap";
  const table = document.createElement("table");
  table.className = "rv-table rv-stack rv-hv-table";
  const head = table.createTHead().insertRow();
  for (const h of [HV.colProtection, HV.colStatus, HV.colEvidence]) {
    const th = document.createElement("th");
    th.scope = "col";
    th.textContent = h;
    head.append(th);
  }
  const body = table.createTBody();
  const linking = part.rows.find((r) => r.key === "linking")?.state ?? "dynamic";
  for (const r of rows) {
    const tr = body.insertRow();
    tr.className = `rv-hv-row is-${verdictClass(r.verdict)}`;
    const name = document.createElement("th");
    name.scope = "row";
    const label = document.createElement("div");
    label.className = "rv-hv-name";
    label.textContent = HV.name(r.key);
    const desc = document.createElement("div");
    desc.className = "rv-hv-desc";
    desc.textContent = HV.desc(r.key);
    name.append(label, desc);
    const status = tr.insertCell();
    status.className = "rv-hv-status";
    status.append(statusCell(r, linking));
    tr.insertBefore(name, status);
    tr.insertCell().append(...evidence(r.evidence));
  }
  wrap.append(table);
  return wrap;
}

function verdictClass(v: HardeningVerdict): string {
  return v === "n/a" ? "na" : v;
}

function statusCell(r: HardeningRow, linking: string): DocumentFragment {
  const f = document.createDocumentFragment();
  const glyph = GLYPH[r.verdict];
  if (glyph !== "") {
    const g = document.createElement("span");
    g.className = "rv-hv-glyph";
    g.setAttribute("aria-hidden", "true");
    g.textContent = glyph;
    f.append(g);
  }
  const s = HV.status(r, linking);
  const word = document.createElement("span");
  word.className = "rv-hv-word";
  word.textContent = s.word;
  f.append(word);
  if (s.detail !== "") {
    const d = document.createElement("div");
    d.className = "rv-hv-detail";
    d.textContent = s.detail;
    f.append(d);
  }
  if (r.key === "fortify" && (r.items.length > 0 || r.more.length > 0)) {
    f.append(nameList(HV.fortifiedList(r.items.length), r.items), nameList(HV.unfortifiedList(r.more.length), r.more));
  }
  return f;
}

/** A count that opens into the names it counts. Nothing for no names. */
function nameList(summary: string, names: readonly string[]): Node {
  if (names.length === 0) return document.createDocumentFragment();
  const d = document.createElement("details");
  d.className = "rv-hv-names";
  const s = document.createElement("summary");
  s.textContent = summary;
  d.append(s, codeList(names));
  return d;
}

function codeList(names: readonly string[]): HTMLElement {
  const p = document.createElement("span");
  p.className = "rv-hv-list";
  names.forEach((n, i) => {
    if (i > 0) p.append(", ");
    const c = document.createElement("code");
    c.textContent = n;
    p.append(c);
  });
  return p;
}

/** One link per piece of evidence, named as the file names it: `GNU_STACK`
 *  segment, `__stack_chk_fail` symbol. Each points at its bytes, so the
 *  hover shows them and a click goes there. */
function evidence(list: readonly HardeningEvidence[]): Node[] {
  const out: Node[] = [];
  for (const e of list.slice(0, EVIDENCE_SHOWN)) {
    const a = document.createElement("a");
    a.className = "rv-at rv-hv-ev";
    a.href = "#";
    const target = { startBit: e.offset_bits, endBit: e.offset_bits + Math.max(8, e.size_bits) };
    pointAt(a, e.path.length > 0 ? { ...target, path: e.path } : target, HV.evidenceLabel(e));
    if (e.name !== "") {
      const c = document.createElement("code");
      c.textContent = e.name;
      a.append(c, " ");
    }
    a.append(HV.evidenceKind(e.what), " ");
    a.append(Object.assign(document.createElement("span"), { className: "rv-hv-off", textContent: formatOffset(e.offset_bits) }));
    out.push(a);
  }
  if (list.length > EVIDENCE_SHOWN) {
    out.push(Object.assign(document.createElement("span"), { className: "rv-muted", textContent: HV.evidenceMore(list.length - EVIDENCE_SHOWN) }));
  }
  return out;
}

function loadingTable(part: HardeningPart): HTMLElement {
  const table = document.createElement("table");
  table.className = "rv-facts rv-hv-loading";
  const caption = document.createElement("caption");
  caption.textContent = HV.loadingCaption;
  table.append(caption);
  const body = table.createTBody();
  const rows = part.rows.filter((r) => LOADING_KEYS.has(r.key));
  rows.sort((a, b) => LOADING_ORDER.indexOf(a.key) - LOADING_ORDER.indexOf(b.key));
  for (const r of rows) {
    const tr = body.insertRow();
    if (r.verdict === "bad") tr.className = "is-bad";
    const th = document.createElement("th");
    th.scope = "row";
    th.textContent = HV.name(r.key);
    tr.append(th);
    const td = tr.insertCell();
    if (r.verdict === "bad") {
      const g = document.createElement("span");
      g.className = "rv-hv-glyph";
      g.setAttribute("aria-hidden", "true");
      g.textContent = GLYPH.bad;
      td.append(g);
    }
    const s = HV.status(r, "");
    if (r.items.length > 0 && HV.listsItems(r.key)) {
      if (s.word !== "") td.append(s.word, " ");
      if (r.items.length <= ITEMS_INLINE) td.append(codeList(r.items));
      else td.append(codeList(r.items.slice(0, ITEMS_INLINE - 1)), " ", nameList(HV.itemsMore(r.key, r.items.length - (ITEMS_INLINE - 1)), r.items.slice(ITEMS_INLINE - 1)));
    } else td.append(s.word);
    if (s.detail !== "") td.append(Object.assign(document.createElement("div"), { className: "rv-hv-detail", textContent: s.detail }));
    const ev = ABSENT.has(r.state) ? [] : evidence(r.evidence.slice(0, 1));
    if (ev.length > 0) td.append(" ", ...ev);
  }
  return table;
}

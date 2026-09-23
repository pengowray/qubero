// Section 4: what is wrong with the file, most severe first.
//
// Only what the core already reports, in its own words (see
// docs/DESIGN-wrong-values.md): values the format rules out, compressed
// streams that would not unpack, bytes no field describes, and values Qubero
// has no name for. Each finding links to its bytes, and a wrong value shows
// what the file holds beside what the format expects.

import { formatOffset } from "../format.ts";
import { checkGap } from "../gapcheck.ts";
import type { Doc } from "../doc.ts";
import type { ReportData } from "./data.ts";
import { signatureOnly } from "./identity.ts";
import { byteRef } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section, type Target } from "./section.ts";
import { bitsText, RV } from "./text.ts";
import { walkProblems, walkTemplate } from "./walk.ts";

/** Findings listed before the rest are counted. */
const SHOWN = 50;

export type FindingKind = "invalid" | "refused" | "gap" | "undefined" | "zeros";

export type Finding = {
  readonly kind: FindingKind;
  readonly target: Target;
  /** The field's name, or null for bytes no field covers. */
  readonly name: string | null;
  /** What is wrong, in the core's words or the report's. */
  readonly text: string;
  /** What the file holds there, for a value beside what it should be. */
  readonly value: string | null;
  readonly bits: number;
};

export type Findings = {
  readonly items: readonly Finding[];
  readonly counts: Readonly<Record<FindingKind, number>>;
  /** True when there are more wrong values than were collected. */
  readonly more: boolean;
};

const RANK: Readonly<Record<FindingKind, number>> = { invalid: 0, refused: 1, gap: 2, undefined: 3, zeros: 4 };

/** Every finding, sorted, or `WAIT` while the walks still need bytes. */
export function findingsOf(doc: Doc, data: ReportData): Findings | typeof WAIT {
  return data.memo("findings", () => collect(doc, data));
}

function collect(doc: Doc, data: ReportData): Findings | typeof WAIT {
  const counts: Record<FindingKind, number> = { invalid: 0, refused: 0, gap: 0, undefined: 0, zeros: 0 };
  if (doc.template === null) return { items: [], counts, more: false };
  const problems = walkProblems(doc);
  if (problems === WAIT) return WAIT;
  const walk = data.memo("walk", () => walkTemplate(doc));
  if (walk === WAIT) return WAIT;
  const parts = data.parts();
  if (parts === WAIT) return WAIT;
  const items: Finding[] = [];
  for (const n of problems.nodes) {
    const p = n.problem;
    if (p === undefined) continue;
    items.push({
      kind: p.tier,
      target: { path: n.path, startBit: n.offset_bits, endBit: n.offset_bits + n.size_bits },
      name: n.name,
      text: p.text,
      value: n.value === "" ? null : n.value,
      bits: n.size_bits,
    });
  }
  counts.invalid = problems.invalid;
  counts.undefined = problems.undefined;
  for (const s of walk.streams) {
    if (s.refused === null) continue;
    items.push({
      kind: "refused",
      target: { path: s.path, startBit: s.offset_bits, endBit: s.offset_bits + s.size_bits },
      name: s.name,
      text: RV.refused(s.refused),
      value: null,
      bits: s.size_bits,
    });
    counts.refused++;
  }
  // A template built from a file(1) rule describes the signature and claims
  // nothing about the rest, so the rest is not a finding.
  const gapsCount = !signatureOnly(doc);
  for (const g of parts?.groups ?? []) {
    if (!g.gap || g.unexamined || !gapsCount) continue;
    for (const u of g.units) {
      const verdict = checkGap(doc, u.offsetBits, u.sizeBits);
      if (verdict === "unread") return WAIT;
      const bytes = u.sizeBits / 8;
      const kind: FindingKind = verdict === "zeros" ? "zeros" : "gap";
      items.push({
        kind,
        target: { startBit: u.offsetBits, endBit: u.offsetBits + u.sizeBits },
        name: null,
        text: verdict === "zeros" ? RV.gapZeros(bytes) : verdict === "something" ? RV.gapNonzero(bytes) : RV.gapUnchecked(bytes),
        value: null,
        bits: u.sizeBits,
      });
      counts[kind]++;
    }
  }
  items.sort((a, b) => RANK[a.kind] - RANK[b.kind] || b.bits - a.bits || a.target.startBit - b.target.startBit);
  return { items, counts, more: problems.more };
}

/** The heading's parts: each kind of finding with its count. */
export function findingCounts(f: Findings): string[] {
  const out: string[] = [];
  if (f.counts.invalid > 0) out.push(RV.findingInvalid(f.counts.invalid));
  if (f.counts.refused > 0) out.push(RV.findingRefused(f.counts.refused));
  const gaps = f.counts.gap + f.counts.zeros;
  if (gaps > 0) out.push(RV.findingGap(gaps));
  if (f.counts.undefined > 0) out.push(RV.findingUndefined(f.counts.undefined));
  return out;
}

export const findingsSection: Section = {
  id: "findings",
  render(ctx: ReportCtx): Rendered {
    const f = findingsOf(ctx.doc, ctx.data);
    if (f === WAIT) return WAIT;
    const parts = findingCounts(f);
    if (parts.length === 0) return null;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-findings";
    sec.id = "rv-findings";
    const h = document.createElement("h2");
    h.textContent = RV.findingsHeading(parts);
    sec.append(h);
    const list = document.createElement("ol");
    list.className = "rv-findlist";
    const rows = alike(f.items);
    for (const row of rows.slice(0, SHOWN)) list.append(findingRow(row));
    sec.append(list);
    const listed = rows.slice(0, SHOWN).reduce((n, r) => n + r.length, 0);
    const total = f.counts.invalid + f.counts.undefined + f.counts.refused + f.counts.gap + f.counts.zeros;
    if (total > listed) {
      const more = document.createElement("p");
      more.className = "rv-note";
      more.textContent = RV.moreFindings(total - listed);
      sec.append(more);
    }
    return sec;
  },
};

/** Findings that say the same thing about the same field, one row each: a
 *  ZIP's 22 `flags` fields with the same unnamed bits are one finding in 22
 *  places, not 22 findings. */
function alike(items: readonly Finding[]): Finding[][] {
  const rows: Finding[][] = [];
  const byKey = new Map<string, Finding[]>();
  for (const f of items) {
    // Bytes no field covers go together when they are the same size and say
    // the same thing: a PDF's newline between each two objects is one byte
    // nothing describes, ten times over.
    const key = `${f.kind}\u0000${f.name ?? ""}\u0000${f.text}\u0000${f.value ?? ""}`;
    let row = byKey.get(key);
    if (row === undefined) {
      row = [];
      byKey.set(key, row);
      rows.push(row);
    }
    row.push(f);
  }
  return rows;
}

/** Places one finding row names before it counts the rest. */
const PLACES = 8;

function findingRow(same: readonly Finding[]): HTMLElement {
  const f = same[0] as Finding;
  const li = document.createElement("li");
  li.className = `rv-find rv-find-${f.kind}`;
  const mark = document.createElement("span");
  // The glyph every view marks a wrong value with, loud only where the format
  // itself rules the bytes out.
  mark.className = `problem-glyph ${f.kind === "invalid" || f.kind === "refused" ? "is-invalid" : "is-undefined"}`;
  mark.setAttribute("aria-hidden", "true");
  mark.textContent = "●";
  li.append(mark);
  const what = document.createElement("span");
  what.className = "rv-find-what";
  if (f.name !== null) {
    const code = document.createElement("code");
    code.textContent = f.name;
    what.append(code, " ");
  }
  if (same.length === 1) {
    what.append(byteRef(f.target, f.name ?? undefined, formatOffset(f.target.startBit)));
    what.append(`, ${bitsText(f.bits)}`);
  } else {
    what.append(`${RV.inPlaces(same.length)}: `);
    same.slice(0, PLACES).forEach((s, i) => {
      if (i > 0) what.append(", ");
      what.append(byteRef(s.target, s.name ?? undefined, formatOffset(s.target.startBit)));
    });
    if (same.length > PLACES) what.append(`, ${RV.andMore(same.length - PLACES)}`);
  }
  li.append(what);
  const why = document.createElement("span");
  why.className = "rv-find-why";
  why.textContent = f.text;
  li.append(why);
  // What the file holds, beside what the format expects: only where the
  // format rules the value out. An unnamed value already says itself in the
  // reason, `2 unnamed bits set`.
  if (f.value !== null && f.kind === "invalid") {
    const v = document.createElement("span");
    v.className = "rv-find-value";
    const label = document.createElement("span");
    label.className = "rv-muted";
    label.textContent = `${RV.stored}: `;
    const code = document.createElement("code");
    code.textContent = f.value;
    v.append(label, code);
    li.append(v);
  }
  return li;
}

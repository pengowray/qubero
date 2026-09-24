// Section 4: what is wrong with the file, in the order of what it says about
// the file.
//
// Only what the core reports, in its own words (see
// docs/DESIGN-wrong-values.md and "The extent audit" in
// docs/DESIGN-report-view.md): values the format rules out and checksums that
// fail, compressed streams that would not unpack, lengths that do not match
// the parts they size, and bytes no field describes. Values Qubero has no name
// for come last and quietly, as one line that opens into the list: an
// undefined value is a gap in the template, not a fact about the file, and a
// file whose only findings are those gets no heading at all.
//
// Each finding links to its bytes. A wrong value shows what the file holds
// beside what the format expects, and a length that does not match shows the
// stated and the actual end side by side, drawn at one scale.

import { formatOffset } from "../format.ts";
import { checkGap } from "../gapcheck.ts";
import type { Doc } from "../doc.ts";
import type { ExtentAudit, ExtentCheck } from "./coredata.ts";
import type { ReportData } from "./data.ts";
import { extentFigure } from "./extentfigure.ts";
import { signatureOnly } from "./identity.ts";
import { byteRef } from "./refs.ts";
import { WAIT, type ReportCtx, type Rendered, type Section, type Target } from "./section.ts";
import { bitsText, RV } from "./text.ts";
import { walkProblems, walkTemplate } from "./walk.ts";

/** Rows listed before the rest are counted. */
const SHOWN = 50;
/** Places one finding row names before it counts the rest. */
const PLACES = 8;

export type FindingKind = "invalid" | "refused" | "extent" | "gap" | "zeros" | "undefined";

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
  /** For a length that does not match its part: the core's numbers. */
  readonly extent?: ExtentCheck;
  /** Within a kind, lower first. */
  readonly rank: number;
};

export type Findings = {
  readonly items: readonly Finding[];
  readonly counts: Readonly<Record<FindingKind, number>>;
  /** True when there are more wrong values than were collected. */
  readonly more: boolean;
  /** False until the core's extent audit has finished. */
  readonly complete: boolean;
  /** The reader's reason, when the file would not read at all. */
  readonly rootFailed: string | null;
};

const KIND_ORDER: Readonly<Record<FindingKind, number>> = { invalid: 0, refused: 1, extent: 2, gap: 3, zeros: 4, undefined: 5 };

/** Which extent verdicts say the most about the file. */
const VERDICT_RANK: Readonly<Record<string, number>> = { "past-file": 0, "past-parent": 1, unreadable: 2, stretched: 3, short: 4 };

type Base = Omit<Findings, "complete" | "rootFailed">;

/** Every finding so far, sorted, or `WAIT` while the walks still need bytes.
 *  The lengths are added once the core's audit is over, and `complete` says
 *  whether it is. */
export function findingsOf(doc: Doc, data: ReportData): Findings | typeof WAIT {
  const base = data.memo("findings-base", () => collect(doc, data));
  if (base === WAIT) return WAIT;
  const core = data.core();
  const audit = core?.audit ?? null;
  const final = core === null || core.failed !== null || audit?.done === true;
  if (!final) return { ...base, complete: false, rootFailed: null };
  return data.memo("findings", () => withAudit(doc, base, audit));
}

function collect(doc: Doc, data: ReportData): Base | typeof WAIT {
  const counts: Record<FindingKind, number> = { invalid: 0, refused: 0, extent: 0, gap: 0, zeros: 0, undefined: 0 };
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
      rank: 0,
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
      rank: 0,
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
        rank: 0,
      });
      counts[kind]++;
    }
  }
  items.sort(byWeight);
  return { items, counts, more: problems.more };
}

/** Most telling first: by kind, then within a kind, then the larger. */
function byWeight(a: Finding, b: Finding): number {
  return KIND_ORDER[a.kind] - KIND_ORDER[b.kind] || a.rank - b.rank || b.bits - a.bits || a.target.startBit - b.target.startBit;
}

function withAudit(doc: Doc, base: Base, audit: ExtentAudit | null): Findings {
  const items = [...base.items];
  const counts = { ...base.counts };
  for (const c of audit?.checks ?? []) {
    if (c.verdict === "fits") continue;
    items.push(extentFinding(doc, c));
    counts.extent++;
  }
  items.sort(byWeight);
  return { items, counts, more: base.more, complete: true, rootFailed: audit?.root_failed ?? null };
}

/** A length that does not match its part, as a finding. */
function extentFinding(doc: Doc, c: ExtentCheck): Finding {
  const read = c.role === "length" ? c.read : null;
  const size = read ?? (c.role === "length" ? c.stated : null);
  let end = c.offset_bits + Math.max(8, size ?? 0);
  if (c.role === "count") {
    const n = doc.templateNode(c.part_path);
    if (n.status === "ok") end = n.node.offset_bits + Math.max(8, n.node.size_bits);
  }
  return {
    kind: "extent",
    target: { path: c.part_path, startBit: c.offset_bits, endBit: end },
    name: c.part_name,
    text: RV.extentVerdict(c),
    value: null,
    bits: Math.max(0, end - c.offset_bits),
    extent: c,
    rank: VERDICT_RANK[c.verdict] ?? 9,
  };
}

/** The heading's parts: each kind of finding about the file, with its count.
 *  Values without a name are not among them: see the file comment. */
export function findingCounts(f: Findings): string[] {
  const out: string[] = [];
  if (f.counts.invalid > 0) out.push(RV.findingInvalid(f.counts.invalid));
  if (f.counts.refused > 0) out.push(RV.findingRefused(f.counts.refused));
  if (f.counts.extent > 0) out.push(RV.findingExtent(f.counts.extent));
  const gaps = f.counts.gap + f.counts.zeros;
  if (gaps > 0) out.push(RV.findingGap(gaps));
  return out;
}

export const findingsSection: Section = {
  id: "findings",
  render(ctx: ReportCtx): Rendered {
    const first = findingsOf(ctx.doc, ctx.data);
    if (first === WAIT) return WAIT;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-findings";
    sec.id = "rv-findings";
    const draw = (f: Findings): void => sec.replaceChildren(...findingsBody(ctx.doc, f));
    draw(first);
    // The lengths come from the core's walk, which may still be going: the
    // section is drawn again once it is over.
    if (!first.complete) {
      ctx.live(() => {
        const f = findingsOf(ctx.doc, ctx.data);
        if (f === WAIT || !f.complete) return false;
        draw(f);
        return true;
      });
    }
    return sec;
  },
};

function findingsBody(doc: Doc, f: Findings): Node[] {
  const out: Node[] = [];
  const parts = findingCounts(f);
  if (f.rootFailed !== null) {
    const p = document.createElement("p");
    p.className = "rv-find-root";
    p.textContent = RV.rootFailed(f.rootFailed);
    out.push(p);
  }
  if (parts.length > 0) {
    const h = document.createElement("h2");
    h.textContent = RV.findingsHeading(parts);
    out.push(h);
    const list = document.createElement("ol");
    list.className = "rv-findlist";
    const rows = alike(f.items.filter((i) => i.kind !== "undefined"));
    for (const row of rows.slice(0, SHOWN)) list.append(findingRow(doc, row));
    out.push(list);
    const listed = rows.slice(0, SHOWN).reduce((n, r) => n + r.length, 0);
    const total = f.counts.invalid + f.counts.refused + f.counts.extent + f.counts.gap + f.counts.zeros;
    if (total > listed) {
      const more = document.createElement("p");
      more.className = "rv-note";
      more.textContent = RV.moreFindings(total - listed);
      out.push(more);
    }
  }
  // The values the template has no name for: one quiet line that opens into
  // the list.
  if (f.counts.undefined > 0) {
    const d = document.createElement("details");
    d.className = "rv-quiet";
    const s = document.createElement("summary");
    s.textContent = RV.undefinedLine(f.counts.undefined);
    d.append(s);
    const list = document.createElement("ol");
    list.className = "rv-findlist";
    for (const row of alike(f.items.filter((i) => i.kind === "undefined")).slice(0, SHOWN)) list.append(findingRow(doc, row));
    d.append(list);
    out.push(d);
  }
  return out;
}

/** Findings that say the same thing about the same field, one row each: a
 *  ZIP's 22 `flags` fields with the same unnamed bits are one finding in 22
 *  places, not 22 findings. A length that does not match is always a row of
 *  its own, since its numbers are its own. */
function alike(items: readonly Finding[]): Finding[][] {
  const rows: Finding[][] = [];
  const byKey = new Map<string, Finding[]>();
  for (const f of items) {
    const key = f.extent !== undefined ? `extent@${f.target.startBit}:${f.extent.length_path.join("/")}` : `${f.kind}\u0000${f.name ?? ""}\u0000${f.text}\u0000${f.value ?? ""}`;
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

function findingRow(doc: Doc, same: readonly Finding[]): HTMLElement {
  const f = same[0] as Finding;
  const li = document.createElement("li");
  li.className = `rv-find rv-find-${f.kind}`;
  const mark = document.createElement("span");
  // The glyph every view marks a wrong value with, loud only where the file
  // itself breaks the format.
  const loud = f.kind === "invalid" || f.kind === "refused" || f.kind === "extent";
  mark.className = `problem-glyph ${loud ? "is-invalid" : "is-undefined"}`;
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
    if (f.extent === undefined) what.append(`, ${bitsText(f.bits)}`);
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
  if (f.extent !== undefined) li.append(extentDetail(doc, f.extent));
  return li;
}

/** A length that does not match its part: the numbers side by side, and the
 *  two ends drawn at one scale. */
function extentDetail(doc: Doc, c: ExtentCheck): HTMLElement {
  const box = document.createElement("div");
  box.className = "rv-extent";
  const dl = document.createElement("dl");
  dl.className = "rv-extent-facts";
  const fact = (label: string, value: Node | string): void => {
    const dt = document.createElement("dt");
    dt.textContent = label;
    const dd = document.createElement("dd");
    dd.append(value);
    dl.append(dt, dd);
  };
  if (c.length_path.length > 0 || c.length_name !== "") {
    const v = document.createElement("span");
    const code = document.createElement("code");
    code.textContent = c.length_name;
    v.append(code);
    if (c.length_value !== null) v.append(` = ${c.length_value.toLocaleString()}`);
    const n = c.length_path.length > 0 ? doc.templateNode(c.length_path) : null;
    if (n !== null && n.status === "ok") {
      v.append(` ${RV.at} `, byteRef({ path: c.length_path, startBit: n.node.offset_bits, endBit: n.node.offset_bits + n.node.size_bits }, c.length_name));
    }
    fact(RV.extentLengthField, v);
  }
  const amount = (n: number | null): string => (n === null ? RV.extentUnknown : c.role === "count" ? RV.elements(n) : bitsText(n));
  fact(c.role === "count" ? RV.extentStatedCount : RV.extentStated, amount(c.stated));
  fact(c.role === "count" ? RV.extentReadCount : RV.extentRead, amount(c.read));
  if (c.role === "length") {
    if (c.content_bits !== null) fact(RV.extentContent, bitsText(c.content_bits));
    fact(RV.extentRoom, bitsText(c.room_bits));
  }
  box.append(dl);
  if (c.adjusted) {
    const p = document.createElement("p");
    p.className = "rv-note";
    p.textContent = RV.extentAdjusted;
    box.append(p);
  }
  if (c.role === "length") {
    const fig = extentFigure(c);
    if (fig !== null) box.append(fig);
  }
  return box;
}

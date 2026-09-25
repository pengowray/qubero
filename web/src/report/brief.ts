// Section 3: the file in brief, as a table of facts, and under it one line of
// landmarks from the format profile: byte order, number widths, how parts are
// found. Each fact is a number with its comparison where it has one.
//
// The rows fill in as their answers arrive, each in a fixed place, so the
// table does not reorder under the reader: the size at once, the parts once
// the template has placed them, the picture's size once it is decoded, and how
// much of the file the template describes once the core's ledger has counted.

import { cardState } from "../contentcard.ts";
import { tablePlan } from "../tableplan.ts";
import { childWord } from "../strings.ts";
import { factRows } from "./bodies.ts";
import { findingCounts, findingsOf } from "./findings.ts";
import type { ListFact, PartsModel } from "./model.ts";
import { landmarks } from "./profiletext.ts";
import { WAIT, type ReportCtx, type Section } from "./section.ts";
import { counted, fileSizeText, listText, pluralOf, RV, sentenceCase } from "./text.ts";
import { walkTemplate } from "./walk.ts";

/** Lists given a row each before the rest are left to the ledger. */
const LISTS_SHOWN = 4;

export const briefSection: Section = {
  id: "brief",
  render(ctx: ReportCtx): HTMLElement {
    const box = document.createElement("div");
    box.className = "rv-brief";
    const table = document.createElement("table");
    table.className = "rv-facts";
    const body = document.createElement("tbody");
    table.append(body);
    box.append(table);
    const row = (key: string | Node, into: HTMLTableSectionElement = body): { tr: HTMLTableRowElement; td: HTMLTableCellElement } => {
      const tr = document.createElement("tr");
      tr.hidden = true;
      const th = document.createElement("th");
      th.scope = "row";
      th.append(key);
      const td = document.createElement("td");
      tr.append(th, td);
      into.append(tr);
      return { tr, td };
    };
    const size = row(RV.factSize);
    size.td.textContent = fileSizeText(ctx.doc.lengthBytes);
    size.tr.hidden = false;
    if (ctx.doc.template === null) return box;
    const parts = row(RV.factParts);
    // The lists, the table's own facts, and the findings each go in a tbody
    // placed now and filled later, so the rows keep their order.
    const lists = document.createElement("tbody");
    const facts = document.createElement("tbody");
    const tail = document.createElement("tbody");
    table.append(lists, facts, tail);
    const picture = row(RV.factPicture, tail);
    const findings = row(RV.factFindings, tail);
    const described = row(RV.factDescribed, tail);
    const marks = document.createElement("p");
    marks.className = "rv-landmarks";
    marks.hidden = true;
    box.append(marks);
    const done = new Set<string>();
    ctx.live(() => {
      if (!done.has("parts")) {
        const m = ctx.data.parts();
        if (m !== WAIT) {
          done.add("parts");
          const groups = (m?.groups ?? []).filter((g) => !g.gap);
          const total = groups.reduce((n, g) => n + g.units.length, 0) + (m?.unlisted ?? 0);
          if (total > 0) {
            parts.td.textContent = RV.partsCount(total, groups.length);
            parts.tr.hidden = false;
          }
          for (const l of m === null ? [] : listRows(m).slice(0, LISTS_SHOWN)) {
            const name = document.createElement("code");
            name.textContent = l.node.name;
            const r = row(name, lists);
            r.td.textContent = counted(l.count, childWord(l.node));
            r.tr.hidden = false;
          }
        }
      }
      if (!done.has("picture")) {
        const s = cardState(ctx.doc);
        if (s === null || s.status === "failed" || s.status === "too-large") done.add("picture");
        else if (s.status === "ready") {
          done.add("picture");
          picture.td.textContent = RV.pictureSize(s.width, s.height);
          picture.tr.hidden = false;
        }
      }
      if (!done.has("table")) {
        const walk = ctx.data.memo("walk", () => walkTemplate(ctx.doc));
        if (walk !== WAIT) {
          done.add("table");
          const first = walk.tables[0];
          const plan = first === undefined ? null : tablePlan(ctx.doc, first);
          if (plan !== null) {
            for (const f of factRows(plan.facts)) {
              const name = document.createElement("code");
              name.textContent = f.name;
              const r = row(name, facts);
              r.td.textContent = f.value;
              r.tr.hidden = false;
            }
            const r = row(sentenceCase(pluralOf(plan.rowWord)), facts);
            r.td.textContent = plan.count.toLocaleString();
            r.tr.hidden = false;
          }
        }
      }
      if (!done.has("findings")) {
        const f = findingsOf(ctx.doc, ctx.data);
        if (f !== WAIT) {
          if (f.complete) done.add("findings");
          const counts = findingCounts(f);
          findings.td.textContent = counts.length === 0 ? RV.noFindings : listText(counts);
          findings.tr.hidden = false;
        }
      }
      const core = ctx.data.core();
      if (!done.has("described")) {
        const ledger = core?.ledger ?? null;
        if (core === null) done.add("described");
        else if (ledger !== null) {
          const gaps = ledger.rows.filter((r) => r.role === "gap").reduce((n, r) => n + r.bits, 0);
          const covered = Math.max(0, Math.min(ledger.reached_bits, ledger.file_bits) - gaps);
          described.td.textContent = RV.described(Math.floor(covered / 8), Math.ceil(ledger.file_bits / 8), ledger.done);
          described.tr.hidden = false;
          if (ledger.done) done.add("described");
        }
      }
      if (!done.has("landmarks")) {
        const profile = core?.profile ?? null;
        if (core === null) done.add("landmarks");
        else if (profile !== null && profile.done) {
          done.add("landmarks");
          const said = landmarks(profile);
          marks.textContent = said.join(" ");
          marks.hidden = said.length === 0;
        }
      }
      return done.size >= 6;
    });
    return box;
  },
};

/**
 * The lists that get a row of their own under the count of parts. A list
 * whose elements are the parts, and is the only one, would only say again
 * what the parts row says: a ZIP file's 23 parts are its 23 records. Where the
 * parts come from several lists, each list's count says how the parts divide
 * among them: an ELF file's segments and sections. A list that is one part,
 * such as a run of numbers, counts something the parts row does not.
 */
export function listRows(m: PartsModel): ListFact[] {
  const key = (p: readonly number[]): string => p.join("/");
  const asParts = new Set<string>();
  for (const g of m.groups) for (const u of g.units) if (u.list !== null) asParts.add(key(u.list.path));
  return m.lists.filter((l) => l.count > 0 && !(asParts.size === 1 && asParts.has(key(l.node.path))));
}

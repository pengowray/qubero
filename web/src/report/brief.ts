// Section 3: the file in brief, as a table of facts. Each fact is a number
// with its comparison where it has one.
//
// The rows fill in as their answers arrive, each in a fixed place, so the
// table does not reorder under the reader: the size at once, the parts once
// the template has placed them, the picture's size once it is decoded, and how
// much of the file the template describes as the walk over it gets on.

import { cardState } from "../contentcard.ts";
import { tablePlan } from "../tableplan.ts";
import { findingCounts, findingsOf } from "./findings.ts";
import { WAIT, type ReportCtx, type Section } from "./section.ts";
import { fileSizeText, listText, pluralOf, RV, sentenceCase } from "./text.ts";
import { walkTemplate } from "./walk.ts";

/** Groups named in the parts row before the rest are counted. */
const PARTS_NAMED = 8;

export const briefSection: Section = {
  id: "brief",
  render(ctx: ReportCtx): HTMLElement {
    const table = document.createElement("table");
    table.className = "rv-facts";
    const body = document.createElement("tbody");
    table.append(body);
    const row = (key: string, into: HTMLTableSectionElement = body): { tr: HTMLTableRowElement; td: HTMLTableCellElement } => {
      const tr = document.createElement("tr");
      tr.hidden = true;
      const th = document.createElement("th");
      th.scope = "row";
      th.textContent = key;
      const td = document.createElement("td");
      tr.append(th, td);
      into.append(tr);
      return { tr, td };
    };
    const size = row(RV.factSize);
    size.td.textContent = fileSizeText(ctx.doc.lengthBytes);
    size.tr.hidden = false;
    if (ctx.doc.template === null) return table;
    const parts = row(RV.factParts);
    const records = row(RV.factRecords);
    const picture = row(RV.factPicture);
    // The table's own facts, read from the fields its shape names, go between
    // the parts and the findings, so the tbody is placed now and filled later.
    const facts = document.createElement("tbody");
    const tail = document.createElement("tbody");
    table.append(facts, tail);
    const findings = row(RV.factFindings, tail);
    const described = row(RV.factDescribed, tail);
    const done = new Set<string>();
    ctx.live(() => {
      if (!done.has("parts")) {
        const m = ctx.data.parts();
        if (m !== WAIT) {
          done.add("parts");
          const groups = (m?.groups ?? []).filter((g) => !g.gap);
          const total = groups.reduce((n, g) => n + g.units.length, 0) + (m?.unlisted ?? 0);
          if (total > 0) {
            const named = groups.slice(0, PARTS_NAMED).map((g) => RV.groupCount(g.units.length, g.label));
            parts.td.textContent = RV.partsSummary(total, named, groups.length - named.length + (m?.unlisted ?? 0));
            parts.tr.hidden = false;
          }
          const lists = (m?.lists ?? []).filter((l) => l.count > 0);
          if (lists.length > 0) {
            records.td.replaceChildren(
              ...lists.flatMap((l, i) => {
                const code = document.createElement("code");
                code.textContent = l.node.name;
                return [...(i > 0 ? [" · "] : []), code, `: ${l.count.toLocaleString()}`];
              }),
            );
            records.tr.hidden = false;
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
            for (const f of plan.facts) {
              const r = document.createElement("tr");
              const th = document.createElement("th");
              th.scope = "row";
              th.textContent = f.label;
              const td = document.createElement("td");
              td.textContent = f.value;
              r.append(th, td);
              facts.append(r);
            }
            const r = document.createElement("tr");
            const th = document.createElement("th");
            th.scope = "row";
            th.textContent = sentenceCase(pluralOf(plan.rowWord));
            const td = document.createElement("td");
            td.textContent = plan.count.toLocaleString();
            r.append(th, td);
            facts.append(r);
          }
        }
      }
      if (!done.has("findings")) {
        const f = findingsOf(ctx.doc, ctx.data);
        if (f !== WAIT) {
          done.add("findings");
          const counts = findingCounts(f);
          findings.td.textContent = counts.length === 0 ? RV.noFindings : listText(counts);
          findings.tr.hidden = false;
        }
      }
      if (!done.has("described")) {
        const k = ctx.data.kinds();
        if (k !== null) {
          const total = ctx.doc.lengthBytes;
          described.td.textContent = RV.described(Math.floor(k.covered_bits / 8), total, k.done);
          described.tr.hidden = false;
          if (k.done) done.add("described");
        }
      }
      return done.size >= 5;
    });
    return table;
  },
};

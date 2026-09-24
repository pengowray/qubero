// Section 11: what the fields the report names are, in the template's own
// words (`Field.doc`). Collected as the sections above draw: each one that
// names a field hands its description to `ReportData.term`.

import { WAIT, type ReportCtx, type Rendered, type Section } from "./section.ts";
import { RV } from "./text.ts";

export const termsSection: Section = {
  id: "terms",
  render(ctx: ReportCtx): Rendered {
    // Drawn after the sections that name the fields, and only once they have
    // drawn: until then the list would be short of the ones still coming.
    if (!ctx.data.drawn("parts") || !ctx.data.drawn("content")) return WAIT;
    const terms = [...ctx.data.terms];
    if (terms.length === 0) return null;
    const sec = document.createElement("section");
    sec.className = "rv-section rv-terms";
    const h = document.createElement("h2");
    h.textContent = RV.termsHeading(terms.length);
    const dl = document.createElement("dl");
    dl.className = "rv-glossary";
    for (const [name, text] of terms.sort((a, b) => a[0].localeCompare(b[0]))) {
      const dt = document.createElement("dt");
      const code = document.createElement("code");
      code.textContent = name;
      dt.append(code);
      const dd = document.createElement("dd");
      dd.textContent = text;
      dl.append(dt, dd);
    }
    sec.append(h, dl);
    return sec;
  },
};

// Section 10: how the format is built. A STUB, drawing nothing yet.
//
// The format profile in full: byte order and integer widths, how text is
// stored, variable-length numbers, bit-packed fields, enums, flags, magic
// numbers, how parts are found (a length in front, a count, a terminator, a
// pointer), checksums, codecs, and what a reader or writer has to know in
// advance. Counted over the template and over this file, so it can say "the
// template allows 12 kinds of chunk and this file uses 4". See "The generic
// report", item 10, in docs/DESIGN-report-view.md.
//
// Waiting on `format_profile_step` in the wasm module, which another change is
// adding. Wire it here once it lands, stepped the way `ReportData.kinds` steps
// `kind_totals_step`, and draw the landmarks line under the facts table
// (section 3) from the same answer. `report_ledger_step` and `extent_audit`
// are coming with it: the first replaces the ledger `bytes.ts` and
// `everybyte.ts` build from the parts, and the second adds its mismatches to
// `findings.ts`.

import type { Rendered, Section } from "./section.ts";

export const profileSection: Section = {
  id: "profile",
  render(): Rendered {
    return null;
  },
};

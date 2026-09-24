// Section 7: what each directory points to. A STUB, drawing nothing yet.
//
// For every list whose elements place something elsewhere in the file (`Ty::At`
// and pointer lists), the elements in stored order joined to what they place,
// in file order, with the ribbon figure (`ribbon.ts`): ZIP's central directory,
// the TTF table directory, ELF's section headers, TIFF's IFD entries.
//
// Waiting on `placements` in the wasm module, which another change is adding:
// it will say, for each such list, which element placed which range. Wire it
// here once it lands: ask for it with `typeof` the way `identity.ts` asks for
// `format_about`, build `RibbonData` from it (top row the elements in stored
// order, bottom row their targets in file order), and draw it with `ribbon()`.
// Until then the section is left out, which is what the report does with any
// section that has nothing to say.

import type { Rendered, Section } from "./section.ts";

export const directoriesSection: Section = {
  id: "directories",
  render(): Rendered {
    return null;
  },
};

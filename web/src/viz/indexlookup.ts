// Index and string-table lookup: the index, the table, the entry it picks.
//
// Candidate `index-lookup` in docs/DESIGN-diagrams.md, section 4.2; built by
// agent L of the first batch (section 5.3). Drawn in an inspector section
// (`inspectorsections.ts`). Test sample: elf/busybox-x86_64 (sh_name),
// png/png10-palette-trns-text.png (palette).
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

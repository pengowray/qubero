// Pointer arcs over the file's address axis.
//
// Candidate `pointer-arcs-axis` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent C1 of the first batch (section 5.3). Drawn in a Pointers tab
// in the rail (`overviewtabs.ts`) and a heading figure (`headingbuttons.ts`).
// Test sample: pdf/classic-xref-uncompressed.pdf,
// zip/streamed-data-descriptors.zip.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

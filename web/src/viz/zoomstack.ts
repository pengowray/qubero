// Zoom stack: the file, the part, the field, as nested locator maps.
//
// Candidate `locator-family` in docs/DESIGN-diagrams.md, section 4.2; built
// by agent D of the first batch (section 5.3). Drawn in nowhere yet: built
// now, mounted in the inspector heading and `hexheadings.ts` in the later
// batch. Test sample: zip/streamed-data-descriptors.zip, pdf/linearized.pdf.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

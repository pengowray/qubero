// Small multiples: one tile per record under a listing heading.
//
// Candidate `record-small-multiples` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent I of the first batch (section 5.3). Drawn in the listing
// heading row (`listingdraw.ts`, `listingreport.ts`). Test sample:
// png/png10-palette-trns-text.png, zip/streamed-data-descriptors.zip.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

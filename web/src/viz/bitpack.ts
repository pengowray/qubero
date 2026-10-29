// Bit-packing ribbon for any field stored least significant bit first.
//
// Candidate `bit-pack-ribbon` in docs/DESIGN-diagrams.md, section 4.2; built
// by agent A of the first batch (section 5.3). Drawn in an inspector section
// (`inspectorsections.ts`), replacing the deflate-only bit-order figure in
// `inspector.ts`, and a row glyph (`rowglyphs.ts`). Test sample:
// gzip/gnu-gzip-9-with-name.gz.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

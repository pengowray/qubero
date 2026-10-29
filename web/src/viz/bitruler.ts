// Bit ruler, RFC-style header diagram and register map.
//
// Candidate `bit-ruler-register` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent A of the first batch (section 5.3). Drawn in an inspector
// section, and a card for sub-byte structures (`registerKindCard` in
// `formatcards.ts`). Test sample: zip/streamed-data-descriptors.zip (flags),
// pe/busybox-w32-x86_64.exe (Characteristics).
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

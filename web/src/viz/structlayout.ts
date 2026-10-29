// Struct layout with padding holes, in the style of pahole.
//
// Candidate `struct-layout-padding` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent F of the first batch (section 5.3). Drawn in a card for any
// composite (`registerKindCard` in `formatcards.ts`), asked for from a
// heading button (`headingbuttons.ts`, `askedCardKey`). Test sample:
// elf/busybox-x86_64, cdrom/hello-mode1.bin.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

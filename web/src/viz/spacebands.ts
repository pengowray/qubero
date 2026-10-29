// File offsets against memory addresses, as bands between two axes.
//
// Candidate `file-vs-memory-bands` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent C2 of the first batch (section 5.3). Drawn in a figure in
// the rail's Logical tab, through a tab of its own (`overviewtabs.ts`). Test
// sample: elf/busybox-x86_64, pe/busybox-w32-x86_64.exe.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

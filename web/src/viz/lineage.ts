// Lineage breadcrumb: which streams a space was unpacked from.
//
// Candidate `space-nesting-components` in docs/DESIGN-diagrams.md, section
// 4.2; built by agent J of the first batch (section 5.3). Drawn in the tab
// strip in `main.ts` and the inspector heading (`inspectorsections.ts`). Test
// sample: png/png12-itxt-deflated-ja.png, gzip/eight-members-split-tar.tgz.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

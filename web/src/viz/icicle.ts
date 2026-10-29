// Position-true icicle: the field tree as a flame chart in file order.
//
// Candidate `position-icicle` in docs/DESIGN-diagrams.md, section 4.2; built
// by agent B of the first batch (section 5.3). Drawn in a sixth treemap mode,
// In order, in `treemappanel.ts` and `treemapdata.ts`. Test sample:
// mp4/h264-aac-moov-at-end.mp4, png/png10-palette-trns-text.png.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

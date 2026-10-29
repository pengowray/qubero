// Pixel grid linked to the bytes, with block and pass overlays.
//
// Candidate `pixel-grid-linked` in docs/DESIGN-diagrams.md, section 4.2;
// built by agent E of the first batch (section 5.3). Drawn in the content
// card (`contentcard.ts`), with the JPEG MCU map and PNG figures as overlay
// modes. Test sample: png/pngsuite-basi6a16-rgba-16bit-interlaced.png,
// jpeg/pillow-cmyk-adobe-restart.jpg.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

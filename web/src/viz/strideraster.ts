// Stride raster: the raw bytes at a chosen row width, with a periodicity
// line.
//
// Candidate `stride-grid` in docs/DESIGN-diagrams.md, section 4.2; built by
// agent N of the first batch (section 5.3). Drawn in a Stride tab in the rail
// (`overviewtabs.ts`). Test sample: cdrom/hello-mode1.bin (2352),
// tar/pax-long-path.tar (512).
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

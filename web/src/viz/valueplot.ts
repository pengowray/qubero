// A numeric run as a plot: waveform, heatmap, column lines.
//
// Candidate `value-plot` in docs/DESIGN-diagrams.md, section 4.2; built by
// agent K of the first batch (section 5.3). Drawn in a heading button that
// opens the plot in the list pane, and the samples inspector section. Test
// sample: wav/broadcast-pcm16-bext-peak.wav, seismic/blockette300.mseed.
//
// A stub until then: piece zero made this file and its line in `register.ts`
// so the agent building the piece owns both and touches no other shared file.
// How a piece is put together is the header of `register.ts`; `childstrip.ts`
// is a worked example.

/** Add this piece to the host views it is drawn in. Called once, from
 *  `register.ts`. Empty until the piece is built. */
export function register(): void {}

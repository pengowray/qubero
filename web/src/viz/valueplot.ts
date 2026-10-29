// A numeric run as a plot: the samples of a WAV as a waveform, a tensor's
// weights as a line or a grid of colours, a miniSEED record's decoded samples.
//
// Candidate `value-plot` in docs/DESIGN-diagrams.md, section 4.2; built by
// agent K of the first batch (section 5.3). The listing, the hex view and the
// list pane all show a run of numbers one value at a time, as text, and a run
// of twenty-four thousand samples is not something anybody reads as text: the
// silence, the clipping, the spike, the layout of a weight matrix are the
// shape of the whole run, and only a picture of the whole run shows them.
//
// Three forms of one piece, all drawn at a size the caller gives:
// - The line (`drawLinePlot`). One lane per channel, stacked over one x axis,
//   on one y scale so the channels can be compared: interleaved stereo is two
//   lanes, a mono run or a tensor is one. Drawn by uPlot, with the run
//   decimated to the pixels it has (`decimate.ts`: the lowest and highest of
//   each pixel's share, so a spike survives every zoom). The board (`board.ts`,
//   x only) zooms about the pointer and pans; every change of view decimates
//   the visible stretch again, so zooming in is more detail and not bigger
//   pixels, down to one sample a few dozen pixels wide.
// - The grid (`drawGridPlot`), for a run of two dimensions such as a GGUF
//   tensor's weight matrix: one cell per value, coloured on a diverging scale
//   around zero, with a colour bar. Wider than the pixels it has, it shows
//   each pixel's mean and says so above the grid. Zooms both ways.
// - The record's samples (`samplesSection`), for a miniSEED record in the At
//   cursor panel. A Steim sample is the sum of the differences before it, so
//   it is at no one place in the file; hovering one lights the 64-byte frame
//   its difference was read from, and a click selects that frame.
//
// Every element is a route to its bytes: hover publishes the element's bits
// on the shared bus (`linked.ts`) so the hex view marks them, a click selects
// the element as a row of the listing would (`pick.ts`), and the element the
// cursor is on is marked on the plot whichever view moved the cursor.
//
// Where the numbers come from: the run's own bytes, read with `Doc.read` and
// decoded with a DataView by the element type the core names (`i16 le[]`),
// since `runCells` writes each element as text and a plot wants millions of
// them. A run whose bytes are elsewhere (inside a decoded stream) is read with
// `fieldBytes`. Steim samples come decoded from the core (`SamplesInfo`),
// which gives the first 32 of them.
//
// Where the code is: `valuerun.ts` reads the run, `valueline.ts` draws the
// line, `valuegrid.ts` the grid, `valuepane.ts` the list pane's figure and
// `valuesamples.ts` the miniSEED figure. The arithmetic is in `decimate.ts`
// and `gridmath.ts`, with unit tests. This module registers them.

import "../toggleseg.css";
import "./valueplot.css";
import { listPaneFor, setPanePlotter } from "../listpane.ts";
import { childWord, VIZ } from "../strings.ts";
import { headingButton, registerHeadingButton } from "./headingbuttons.ts";
import { registerInspectorSection } from "./inspectorsections.ts";
import { paneFigure } from "./valuepane.ts";
import { plottable } from "./valuerun.ts";
import { dataStart, samplesAt, samplesSection } from "./valuesamples.ts";

/** Add the plot to the listing's headings, the list pane and the At cursor
 *  panel. Called once, from `register.ts`. */
export function register(): void {
  setPanePlotter({ applies: plottable, draw: paneFigure });
  registerHeadingButton({
    id: "value-plot",
    applies: (_c, node) => plottable(node),
    draw: (c, node) => {
      const pane = listPaneFor(c.doc);
      const on = pane !== undefined && pane.showing(node.path) === "plot";
      return headingButton(VIZ.valuePlot.button, VIZ.valuePlot.buttonTitle(node.child_count, childWord(node)), on, () => {
        const p = listPaneFor(c.doc);
        if (p === undefined) return;
        if (p.showing(node.path) === "plot") p.close();
        else p.open(node.path, "plot");
      });
    },
  });
  registerInspectorSection({
    id: "value-plot-samples",
    applies: (doc, node) => samplesAt(doc, node.path) !== null,
    // The record's data, so moving between its frames keeps the figure and
    // its zoom; and the samples, so an edit that changes them draws again.
    key: (doc, node, path) => {
      const info = samplesAt(doc, path);
      return info === null ? path.join(".") : `samples:${dataStart(doc, path, info) ?? path.join(".")}:${info.total}:${info.values.join(",")}`;
    },
    draw: ({ doc, path, width }) => samplesSection(doc, path, width),
  });
}

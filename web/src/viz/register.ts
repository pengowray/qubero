// Every diagram piece, added to the views it is drawn in. Imported once, from
// `main.ts`, before any document opens.
//
// One line per piece below, in the order of the batch table in
// docs/DESIGN-diagrams.md section 5.3. A piece's agent edits its own line and
// its own module and nothing else here, so two agents never touch the same
// line and their merges are clean.
//
// ----- How to add a piece -----
//
// A piece is one module in `src/viz/` (and DOM-free helpers beside it, with
// tests in `test/`). `childstrip.ts` is a worked example of all of this.
//
// 1. Draw at a size you are given. Export
//      draw(data, opts: { width, height, ... }) -> { el, ... }
//    in the style of `treemap.ts`: the caller owns the size and the piece never
//    measures its host, so it can be drawn in a card, at rail width or over the
//    whole window. Take a `Doc` and a path, not a `DrawContext` or a
//    `ReportCtx`; read with `doc.templateNode` and `doc.templateChildren`, and
//    answer null (or draw a "reading" state) while the bytes are on the way.
//    Put the layout arithmetic in a function with no DOM, and test it.
//
// 2. Make it explorable (rule 1 of the design):
//    - Pan and zoom with `board(host, { width, height, xOnly?, onZoom })` from
//      `board.ts`. `onZoom` gets the transform; apply it with `boardmath.ts`
//      (`applyX`, `invertX`, `rescaleDomain` for a d3 scale). Use `xOnly` for
//      anything over the address axis, so file order stays left to right.
//    - Hover with `tip.ts`: `hoverTip(el, e => tipCard(name, lines, where))`.
//      Every value goes in the card, so the drawing can leave labels off.
//    - Click with `pick.ts`: `goToField(doc, {path, startBit, endBit})` or
//      `goToBits(doc, ranges)`. For DOM and SVG, `markPick(el, pick)` and
//      `followPicks(root, doc)`; for canvas or thin marks, hit-test with
//      `nearestOnAxis` or `nearestTarget`, which make every target 24 pixels.
//    - Share hover and selection with `linked.ts`: `linkedFor(doc).publish({
//      hover })` as the pointer moves, and `subscribeWhileConnected(bus, el,
//      fn)` to light the selected field. The hex view marks a published hover.
//
// 3. Never let labels overlap (rule 2): fit names with `fit.ts` (`fitLabel`,
//    `ellipsise`, `freeRect`, `thinLabels`), put brackets and readings over an
//    axis in lanes with `lanes.ts`, use ELK or cytoscape for anything
//    node-link, and d3-axis or uPlot for ticks. Turn on detail at the zoom
//    levels in `lod.ts`. Colour by `fieldstyle.ts`: `fieldClass` for kinds,
//    `roleColor` for dependency edges.
//
// 4. Register it with the host views, in your module's `register()`:
//    - `registerInspectorSection` (`inspectorsections.ts`): a section in the
//      At cursor panel, under the panel's own.
//    - `registerHeadingButton` (`headingbuttons.ts`): a button on a listing
//      heading; `registerKindCard` (`formatcards.ts`) for a card that button
//      asks for in place of the heading's fields.
//    - `registerRowGlyph` (`rowglyphs.ts`): a word-sized glyph beside a row's
//      value in the listing.
//    - `registerOverviewTab` (`overviewtabs.ts`): a tab in the rail beside
//      Contents, Logical and B-trees.
//    Put the piece's words in `strings.ts`, in its own key of `VIZ`.
//
// 5. Check it in the browser on the sample its row of the batch table names:
//    wheel, drag, hover and click, light and dark, and no overlapping labels.

import { register as childStrip } from "./childstrip.ts";
import { register as bitPack } from "./bitpack.ts";
import { register as bitRuler } from "./bitruler.ts";
import { register as varintFigure } from "./varintfigure.ts";
import { register as icicle } from "./icicle.ts";
import { register as pointerArcs } from "./pointerarcs.ts";
import { register as spaceBands } from "./spacebands.ts";
import { register as zoomStack } from "./zoomstack.ts";
import { register as pixelGrid } from "./pixelgrid.ts";
import { register as structLayout } from "./structlayout.ts";
import { register as recordTiles } from "./recordtiles.ts";
import { register as sparkline } from "./sparkline.ts";
import { register as lineage } from "./lineage.ts";
import { register as valuePlot } from "./valueplot.ts";
import { register as indexLookup } from "./indexlookup.ts";
import { register as strideRaster } from "./strideraster.ts";
// No lines for `decimate.ts`, `tableentries.ts` and `periodicity.ts`: they
// are helpers of `valueplot.ts`, `indexlookup.ts` and `strideraster.ts` and
// register nothing of their own.

// The example piece, only with `?vizdemo` in the address: the At cursor panel
// has not been designed to hold it.
if (typeof location !== "undefined" && new URLSearchParams(location.search).has("vizdemo")) childStrip();

// ----- agent A -----
bitPack(); // bit-pack-ribbon
bitRuler(); // bit-ruler-register
varintFigure(); // varint-gather

// ----- agent B -----
icicle(); // position-icicle

// ----- agent C1 -----
pointerArcs(); // pointer-arcs-axis

// ----- agent C2 -----
spaceBands(); // file-vs-memory-bands

// ----- agent D -----
zoomStack(); // locator-family

// ----- agent E -----
pixelGrid(); // pixel-grid-linked

// ----- agent F -----
structLayout(); // struct-layout-padding

// ----- agent I -----
recordTiles(); // record-small-multiples, heading tiles
sparkline(); // record-small-multiples, run sparkline

// ----- agent J -----
lineage(); // space-nesting-components

// ----- agent K -----
valuePlot(); // value-plot

// ----- agent L -----
indexLookup(); // index-lookup

// ----- agent N -----
strideRaster(); // stride-grid

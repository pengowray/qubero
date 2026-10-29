# Diagram pieces: how bits, bytes and structures fit together

Design notes for a family of pictures that show how the parts of a file relate:
the kind of figure found in a format spec, an RFC header diagram, a compiler
textbook, a UML sheet or a good explainer video. Each one is a "piece": a
drawing with a fixed job, drawn at one or more sizes, that can sit on its own
or inside another piece. The aim is many overlapping pieces first (forty-odd
candidates here, twenty or so built) and a cull later once they have been
used on real files.

State of this document: research and design only, 2026-10-29. Nothing here is
built. Section 5 is the batch to build first.

## 1. Purpose and the two hard rules

### Why pictures, and why so many
The listing, the hex view and the inspector say what a field is and where it
is, one field at a time, in text. What they cannot say is how fields relate:
which bytes a length counts, which bits of a word belong to which flag, where
a directory at the end of the file points, how a stream scattered through the
file is one stream, how the bytes under the cursor become the number on the
row. Those are relationships, and relationships are what diagrams are for.

The pieces overlap on purpose. The same fact (a ZIP's central directory
reaching back to each local header) is worth drawing as arcs over the file
axis, as boxes with pointer arrows, as a row of small tiles, and as one lit
segment on a 96px map, because each form answers a different question at a
different size. Building several and keeping the ones readers reach for is
cheaper than arguing about which one is right.

### Vocabulary
Sizes, from smallest to largest:
- `inline`: word-sized, sits in a row, a heading, a tab title or a tooltip. No
  text of its own beyond a hover card.
- `figure`: a block inside a text column: an inspector section, a listing card,
  a report figure. Width follows the host; no zoom unless the host grows.
- `panel`: a rail panel or a card with its own pan and zoom; a grow button
  throws it over the window.
- `view`: a main view tab in `main.ts`.

Scales, by what one drawn unit stands for: `bit`, `byte`, `field`, `structure`
(a record and its members), `stream` (one decoded or stitched space), `file`
(the whole address space) and `format` (the template, independent of this file).

### Rule 1: explorable with a mouse and a wheel
Every piece above inline size answers the wheel (zoom about the pointer, or
scroll when zoom means nothing), drags to pan, shows values on hover in one
shared hover card, and goes to the bytes on click. Nothing is only a picture:
a bar, an arc, a band or a cell is a route to the field it stands for, using
the same `FieldPick` and bit-range contract the listing and the hex view use.
Wheel zoom inside a scrolling page uses the rule in `report/wheel.ts`: the
figure takes the wheel only when the page is still, so a reader scrolling past
a figure is not stopped by it.

### Rule 2: labels never overlap, so layout libraries
A label that collides with another is a label the reader cannot read, which is
worse than no label, because it also hides the one it covers. Hand-placed text
with nudging is the known defect in `diagramview.ts` (role words on arrows) and
`hexlinks.ts`, and no new piece repeats it. Instead:
- Anything node-link goes through a layout engine that sizes nodes from
  measured text and reserves room for edge and port labels: ELK (`elkjs`, in
  the repo) or cytoscape with an ELK adapter.
- Labels inside boxes and bars follow the rule `treemap.ts` already has:
  measure the text, draw it when it fits, otherwise drop it and keep the name
  in the hover card. Never shrink text below a readable size to make it fit.
- Braces, brackets and readings over a byte axis take separate lanes assigned
  by interval colouring, so two that overlap in x never share a y.
- Margin callouts with leader lines (poster mode) go through a 1-D label
  placer rather than by hand.
- Axis ticks come from the charting library (d3-axis, uPlot), which thins them.

The one exception, stated once: a fixed-pitch grid (one digit per bit cell, an
8x8 block, a 256-bar histogram, a byte raster) has no layout problem. Every
cell is the same size and the text is one or two characters, so there is
nothing to collide. The rule bites wherever a label's position depends on
measured text or on where other labels landed. Hand-rolling a graph, a chart
with axes, a calendar or a tree layout is still out.

A corollary for scale: a piece that can be handed 50,000 records (PNG with
4,096 IDATs, a tar with 50k entries, a ZIP with 5,000 members) folds repeats
before layout (a run of identical neighbours becomes one tile marked xN, a
sub-pixel sibling pool becomes one "N small fields" bar) or virtualises its
rows. A piece that only works on small files is not finished.

## 2. What already exists

The inventory of drawn things in `web/src`, merged where two passes listed the
same piece twice. Text-only panels (step lists, GRIB and BUFR derivation lines,
xref rows, fact tables) are left out; they are hosts for new pieces, not
pictures. "Data" names the Doc call the piece reads.

| Piece | Module | Size | Data |
|---|---|---|---|
| Type boxes and arrows (Diagram view, Arrows mode) | `diagramview.ts`, `diagramcounts.ts` | view | `templateDiagram`, `diagramCensus` |
| Spec-style strips with funnels (Diagram view, Strips mode) | `strips.ts`, `diagramview.ts` | view | `templateDiagram` |
| Census count badges and muted boxes | `diagramcounts.ts` | inline | `diagramCensus` |
| Field graph, force-directed (Graph view, behind `?graph`) | `graphview.ts` | view | `graph(path, limit)` |
| Kind hulls over the graph | `graphview.ts` | view | layout positions + `GraphNode.kind` |
| Treemap widget (generic, caller owns size) | `treemap.ts` | panel | `TreeNode` supplied by caller |
| Structure, byte-class, kind, byte-value and set/clear-bit treemaps | `treemapdata.ts`, `treemappanel.ts` | panel | `templateChildren`, `overviewStep`, `kindTotalsStep` |
| Minimap byte-class cell grid and block close-up | `minimappanel.ts` | panel | `overviewStep`, `overviewFocusStep`, `spans` |
| File map strip (whole file, one part lit) | `filemap.ts` | inline | `MapSegment[]` from the listing's headings |
| Position bar (many stretches marked) | `report/posbar.ts` | inline | `Extent[]` from `PartsModel` |
| Byte strip (fields as columns under a heading) | `bytestrip.ts` | figure | `spans`, `read`, `readBits` |
| Bit-role chip (varint framing against value bits) | `bytestrip.ts` (private) | inline | `Span.bits` |
| Bit extraction formula | `bitextract.ts` | inline | bit offset, width, lsbFirst |
| Deflate bit-order ribbon | `bitorder.ts`, `report/ribbon.ts` | figure | `read`, `DecodedCode` parts |
| Ribbon (two orderings joined by bands) | `report/ribbon.ts` | figure | `RibbonData` supplied by caller |
| Directory ribbon | `report/directories.ts` | figure | `reportStep('directories_step')` |
| Deflate trace ribbon | `report/streams.ts` | figure | `Space.mapOutAny`, `Space.read` |
| Stream stages bars | `report/streams.ts` | inline | packed and unpacked counts |
| Zoom map (parts, fields, byte classes, semantic zoom) | `report/zoommap.ts`, `report/wheel.ts` | figure | `spans`, `read`, `overviewStep` |
| Ledger stacked bar and share table | `report/bytes.ts`, `report/ledger.ts` | figure | `reportStep('byte_ledger_step')` |
| Extent figure (stated end against actual end) | `report/extentfigure.ts` | figure | `reportStep('extent_audit_step')` |
| B-tree icicle and address strip (HDF5) | `btreedraw.ts`, `btreepanel.ts` | panel | `btree(path, 4096)` |
| JPEG quantisation square, Huffman length bars and code table, frame and scan cards | `jpegcards.ts`, `jpegtables.ts`, `cardparts.ts` | figure | `templateChildren`, `read` |
| JPEG MCU cost map, block code strip, zigzag row, coefficient grids | `report/jpeg.ts` | figure | `jpegScan`, `jpegBlock` |
| PNG palette swatch grid | `pngcards.ts` | figure | `templateChildren` |
| PNG Adam7 tile, pass table and scanline filter strip | `report/pngscanlines.ts`, `report/pngpasses.ts` | figure | template scanline trace |
| Decoded image content card | `contentcard.ts`, `report/content.ts` | figure | browser decode of `read(0, len)` |
| Quantised weight block grid | `quantpanel.ts` | panel | `typeInfo` -> `QuantInfo` |
| Hex value cells (values drawn over their bytes) | `valuetable.ts`, `valuelayout.ts`, `valuecells.ts` | inline | `runCells` |
| Hex origin arrows and sidebar hover box | `hexlinks.ts` | view | `origins`, `templateNode` |
| Hex heading lines | `hexheadings.ts`, `headingdensity.ts` | inline | `OutlineHeading[]` |
| Listing heading, field, gap and record rows | `listingdraw.ts`, `listingreport.ts`, `records.ts` | view | `flatten.ts` items |
| Logical outline tree (Logical tab) | `logicaloutline.ts`, `pngoutline.ts`, `overviewpanel.ts` | panel | per-format adapters |
| Table view | `tableview.ts` and `table*.ts` | view | `tableShape`, `templateChildren`, `pickleCells` |

What the inventory says as a whole: most of the pictures are hand-rolled DOM,
most hover text is a native `title`, only the Diagram view and the zoom map
have wheel zoom, only the treemap widget lets its caller own its size, and
the file map is the one piece that nests (four hosts). The bit scale has one
figure (the deflate ribbon), the file scale has three strips and a treemap,
and nothing draws two address spaces against each other.

## 3. Shared infrastructure to build first

The candidates in section 4 assume the same handful of services. Building
them once, as piece zero, is what lets the first batch be built by agents in
parallel without each rolling its own pan and zoom or its own tooltip. New
modules go in `web/src/viz/`.

### 3.1 Pan and zoom board: `viz/board.ts`
A thin wrapper over `d3-zoom` that gives any SVG or canvas host wheel zoom
about the pointer, drag with a 4px slop (so a click is still a click), Fit and
Home, an optional x-only mode (file order must stay left to right in anything
over an address axis), and the page-scroll rule from `report/wheel.ts`. The
Diagram view's hand-written stage, the Graph view's cytoscape viewport and the
treemap full-window mode keep what they have for now; new pieces use the board
at figure, panel and view size alike, since the board never measures its host.
Library: `d3-zoom`, `d3-scale`, `d3-axis`, `d3-shape` added once (about 30KB
gzipped together; only `d3-hierarchy` is installed today). Lazy-imported like
cytoscape and elkjs.

### 3.2 One hover card: `viz/tip.ts`
`report/tip.ts` promoted out of the report: one element per page, shown beside
the pointer and kept on screen, taking a node or a string. Every piece puts its
values here, which is what lets the drawings themselves stay unlabelled. It
replaces native `title` attributes in the treemap, the file map and the
minimap's readout line as those are touched.

### 3.3 Click-to-bytes contract and hit testing: `viz/pick.ts`
`cardparts.ts` `fieldButton` already marks DOM elements with `data-reads`,
`data-at` and `data-size`, and the listing follows them. The same contract for
SVG elements and for canvas (a hit-test helper that maps a pointer position to
the nearest bar, arc end, cell or band) so that every piece selects a path or
a bit range the same way: `FieldPick {path, startBit, endBit}` for a field and
`BitRange[]` runs for split bits, as the quant grid already passes. Hit targets
are at least 24px, so a 2px map segment resolves to the nearest segment and a
1px arc is picked by its endpoint.

### 3.4 Linked highlight bus: `viz/linked.ts`
The judges named this as a requirement rather than a piece: one hover and one
selection, published once, so hovering a bit, a field or a record in any
diagram lights it in the hex view, the locator and every other open piece.
Today each view syncs the cursor through `main.ts` by hand. The bus carries
`{hover: BitRange | null, selection: FieldPick | null}` and pieces subscribe;
`main.ts` publishes what it already tracks. Without it the overlapping pieces
cannot be explored as one instrument.

### 3.5 Label helpers: `viz/fit.ts`, `viz/lanes.ts`
`fit.ts`: measure text once per font (canvas `measureText`), draw a label when
it fits its box with padding, otherwise ellipsis down to a minimum, otherwise
drop. Extracted from `treemap.ts`. `lanes.ts`: interval colouring (sort by
start, greedy first free lane) for braces, brackets and readings over an axis.
Margin callouts with leaders are a later helper; the only piece needing them
(the poster mode of the annotated byte strip) is in the later batch.

### 3.6 Level-of-detail conventions: `viz/lod.ts`
The pixel widths at which the file-to-bit view, the position icicle, the pixel
grid and the raster turn on child fetching, labels, hex digits and bit cells,
in one table, so the four feel like one instrument: fetch children when a bar
is wider than 24px, label when the measured text fits, hex digits when a byte
cell is at least 14px (the zoom map's threshold today), bit cells at 24px per
byte cell and over.

### 3.7 Colours: `fieldstyle.ts`
`fieldClass`, `byteClassColors` and `fieldHue` stay the single source for
kinds, byte classes and hues. Add one role-to-colour table for dependency
edges (length, count, type, position, value, name, width, condition, case,
points) shared by the pointer arcs, the DAG, the matrix, the hex links and the
Diagram view's arrows, so a reader learns the colours once.

### 3.8 Size-independent widget API
Every piece exports `draw(data, opts {width, height, ...}) -> {el, ...}` in the
`treemap.ts` style: the caller owns the size and the widget never measures its
host. That is what lets a piece be drawn tiny inside a card, at rail width, or
over the whole window, and nested inside another piece. Pieces that read the
Doc take a `Doc` and a path, not a `DrawContext` or a `ReportCtx`; where a
listing card is wanted, `report/cards.ts` already shows how to build a
`DrawContext` stand-in.

### 3.9 Registries, so batch agents do not share host files
Four host files are touched by nearly every candidate: `inspector.ts` (3,559
lines), `listingdraw.ts`, `overviewpanel.ts` and `main.ts`. Piece zero gives
each a registry the way `formatcards.ts` already has a map of card readers:
- `viz/inspectorsections.ts`: a list of `(node, doc) => HTMLElement | null`
  sections the inspector appends after its own, dispatched like `typepanel.ts`.
- `viz/headingbuttons.ts`: a list of buttons the listing heading row offers
  (today: bytes, stream, list, table), each with a predicate on the node.
- `viz/rowglyphs.ts`: inline glyphs the listing field row draws beside a
  value when a predicate matches (sub-byte field, varint, time field).
- `viz/overviewtabs.ts`: tabs the rail offers beside Contents, Logical and
  B-trees, each with a predicate on the Doc.
- `formatcards.ts`: opened to cards registered by node kind, not only by
  template, so a struct layout card can offer itself for any composite.
Piece zero can pre-register one stub module per batch piece, so a batch agent
owns its stub and its new files and touches nothing else. Each registry file
is append-only, so two agents adding a line merge cleanly.

### 3.10 Generalised ribbon and read-ahead
`report/ribbon.ts` opened to any two orderings joined by bands (it already
serves three callers), with a `d3-shape` area path as the canvas fallback past
a few thousand bands; `bitorder.ts` `bitOrderLayout` opened to any field with
`lsb_first` or any sub-byte field. A read-ahead helper that walks
`Doc.templateChildren` in pages while bytes may still be arriving (what
`kid`/`kids` in `cardparts.ts` do) as an async iterator, used by every piece
that enumerates records, time fields, checksums or flags.

### 3.11 Charting dependencies
One charting library for sparklines, histograms, strips and decimated lines:
`uPlot` (about 50KB, fast canvas, cursor sync, brush zoom). The d3 grid-and-
band family (`d3-scale`, `d3-axis`, `d3-shape`, `d3-zoom`, `d3-hierarchy`,
`d3-sankey` only if the file-wide Sankey is built). `elkjs` for anything
node-link. `railroad-diagrams` for the grammar piece. Rejected after the
judges' review: `bit-field` and `bytefield-svg` (static SVG, no hover hooks; a
fixed-pitch grid is less work), `labella.js` (unmaintained; deferred with the
poster mode), `Observable Plot` (pulls all of d3 for static figures uPlot or
canvas cover), `KaTeX` (280KB for one formula line), `vis-timeline` (its own
DOM model and CSS), `reorder.js` (unmaintained, pulls science.js),
`OpenSeadragon` (only if images outgrow a canvas).

### 3.12 Core DTOs the marked candidates wait on
Not needed for the first batch. Listed so they can be scheduled:
- an `Expr` AST per Relation (`{op, args[], value, path?}`) for the expression tree;
- the anchor kind and base bit of each `At` field for the offset-frames ruler;
- the parts list (the Extent vec) of a stitched space for the order-against-address ribbon;
- a per-bucket entropy array in `OverviewState` (bridgeable web-side from `read`);
- byte-pair counts for a range (bridgeable in a worker up to a few MiB);
- record reference edges (PDF `n 0 R`, HDF5 object headers) for the reference graph;
- a walk-order seek trace for the numbered reading path;
- bulk lists: every check field, every time field, a stream's decode steps, a numeric run as a typed array.

## 4. The catalog

### 4.0 What the first pass missed (the critic's additions)
Checked against the `###` headings of `docs/DESIGN.md` from line 987 on and the
exported types of `web/src/doc.ts`:

- Three of the judges' fifteen "missing" items are one picture (a numeric run as
  a waveform, heatmap or decimated line) and two more are one picture (a
  field-aligned diff). Two are infrastructure (the viewport kit, the linked
  highlight bus) and are in section 3. One (in-grid annotations in the hex
  view) is the hex-view form of the annotated byte strip and is folded into
  that candidate. One (the byte-ledger bar) is the inline form of the entropy
  and ledger pieces and is noted under `entropy-histogram-strip`. One (the
  pointer scatter) is noted as a mode of `pointer-arcs-axis`, since it draws
  the same edges on the same axis. One (bit-position usage) is noted as a
  variant of the byte histogram.
- `DumpScan` (`doc.ts` ~720) and the DESIGN heading "A file that is a dump of
  another file" have no picture anywhere: nothing shows which dump lines made
  which bytes, where the holes and autoskip runs are, or where two lines
  disagreed. `SigVerdict`, `Identification`, `ToolMatch` and
  `TemplateChoice.magic` ("Which answer names the file", "The signature
  database") have no picture either; a figure lighting each candidate rule's
  magic bytes on a byte strip is noted under `parse-failure-explainer` rather
  than added, since it is the same expected-against-found form.
- "A signature that is wrong, read beside the one it should have been" and "A
  run that stops short says why" (DESIGN 2966, 2985) are two headings about
  deviations, which are what a reader is looking for, and no candidate drew
  them.
- "A structure that says which field names it" and "Names through an
  encoding's own steps" (DESIGN 1096, 1125) are index-into-table lookups, the
  commonest indirection in binaries; the pointer arcs show them as a line.
- Thin scales: `field` had two candidates, both needing core; `byte` had one;
  `stream` had nothing above 11. Thin sizes: nothing at `view` size reads a
  long list, which is where the listing's own virtualised rows could carry a
  picture.

Six candidates are added, marked "(critic)" in the table and in their
headings: `value-plot`, `tree-table-gantt`, `index-lookup`,
`parse-failure-explainer`, `record-diff`, `dump-reconstruction`. They are
scored on the same three lenses (reader value, legibility and explorability,
feasibility, each out of 5).

### 4.1 Summary table
Library is the choice after the judges' review (what they overruled is in each
subsection). "Home" names the host module; new modules are named in section 5.
Scores: R reader value, L legibility and explorability, F feasibility.

| id | name | visual language | scale | sizes | home | data | core? | library | R L F | total |
|---|---|---|---|---|---|---|---|---|---|---|
| `bit-pack-ribbon` | Bit-packing ribbon for any LSB-first field | two bit rows joined by bands | bit | figure | inspector (bitorder host) | `Span.lsb_first`, `read`, `decodedCode` | no | `ribbon.ts`, `bitorder.ts` | 4 5 5 | 14 |
| `bit-ruler-register` | Bit ruler (RFC header, register map) | spec bit ruler, boxes over bits | bit | inline figure panel | inspector, formatcards, row glyph | `spans`, `typeInfo` (Flags, Enum) | no | fixed-pitch SVG grid, board at panel size | 5 4 4 | 13 |
| `varint-gather` | Varint byte-gathering figure | protobuf-docs bytes split and stacked | bit | inline figure | inspector, row glyph | `Span.bits`, `read` | no | `ribbon.ts`, SVG bit grid | 4 4 5 | 13 |
| `position-icicle` | Position-true icicle | partition, offset across, depth down | file | panel view | `treemappanel.ts` sixth mode | `templateChildren`, `spans` | no | d3-hierarchy partition, board | 4 5 4 | 13 |
| `file-vs-memory-bands` | File offsets against memory addresses | ELF linking-view against execution-view bands | file | figure panel | overview tab (Logical figure), `report/hardening.ts` | `elfContents`, `templateChildren` (PE) | no | d3-scale piecewise, d3-shape, board | 5 4 4 | 13 |
| `locator-family` | Locator glyphs: live file map, zoom stack, crumb bars, scrollbar track | inset locator maps | file | inline panel | `filemap.ts`, `trail.ts`, new `overviewtrack.ts` | `MapSegment[]`, `trail`, `partOf`, problems and hits | no | existing DOM, canvas, d3-array bin | 5 4 4 | 13 |
| `pixel-grid-linked` | Pixel grid linked to bytes, with overlays | zoomable image, per-pixel hex, heat overlay | structure | figure panel view | `contentcard.ts`, `report/jpeg.ts`, `report/pngscanlines.ts` | `jpegScan`, `jpegBlock`, `mapOut`, scanlines | no | canvas (no smoothing), board | 5 5 3 | 13 |
| `value-plot` (critic) | Numeric run as a plot: waveform, heatmap, column lines | decimated line chart, 2-D colour grid | structure | figure panel view | heading button, `samplespanel.ts` | `read` + type decode, `runCells`, `tableShape`, `SamplesInfo` | no | uPlot, canvas | 5 4 4 | 13 |
| `tree-table-gantt` (critic) | Outline with position bars | Gantt-style tree table | file | view | listing (`listingreport.ts`) optional column | items' `offset_bits`, `size_bits` | no | d3-scale, board (x only) | 4 5 4 | 13 |
| `index-lookup` (critic) | Index and string-table lookup | lookup figure: index, table row, resolved value | field | inline figure | inspector section, `-> reads` hover card | `origins`, `relations`, `templateNode`, `TemplateNode.colour` | no | DOM, `ribbon.ts` for the arrow | 4 5 4 | 13 |
| `parse-failure-explainer` (critic) | Expected against found where the template stopped | two-row byte comparison with a chain of causes | field | figure | inspector problem section, `report/findings.ts` | `Problem`, `MagicInfo`, `EnumInfo`, `extent_audit_step`, `origins` | no | DOM, `extentfigure.ts` | 5 5 3 | 13 |
| `struct-layout-padding` | Struct layout with padding holes | pahole-style word rows | structure | figure panel | formatcards card, heading button | `templateChildren`, `spans` (gaps) | no | CSS grid, d3-scale margins | 4 4 4 | 12 |
| `pointer-arcs-axis` | Pointer arcs over the file axis | arc diagram (Shape of Song) | file | inline figure panel | overview tab, heading figure, `report/directories.ts` | `graph`, `origins`, `relations` | no | d3-shape arcs, board, canvas past 2k | 5 3 4 | 12 |
| `reads-dag` | Layered dependency DAG | compiler data-dependence graph | structure | panel view | `graphview.ts` layout mode | `graph`, `relations` | no | cytoscape + cytoscape-elk | 4 3 5 | 12 |
| `entropy-histogram-strip` | Entropy strip with byte-value histogram | line over class band and file map; 256 bars | file | inline figure panel view | `minimappanel.ts`, `report/bytes.ts` | `histogram`, `FocusState`; per-bucket entropy bridged from `read` | optional | uPlot | 4 5 3 | 12 |
| `huffman-tree` | Canonical Huffman tree with table | binary code tree beside canonical table | bit | figure panel | `jpegcards.ts` DHT card, inspector | `templateChildren`, `huffmanCodes`, `DecodedCode` | no | d3-hierarchy tree (not cluster), board | 4 4 4 | 12 |
| `space-nesting-components` | Address-space nesting with lineage breadcrumb | UML component boxes; breadcrumb | format | inline figure panel | overview tab, tab strip in `main.ts`, inspector heading | `TemplateNode.space`, `partOf`, `unpacking`, `space_origin` (wasm export, no Doc method yet) | no | elkjs compound, DOM | 5 4 3 | 12 |
| `record-small-multiples` | Small multiples per record, sparklines, column sparks | unit chart, Tufte sparklines | structure | inline figure panel | heading row, `tablecolumns.ts`, `valuetable.ts` | `templateChildren`, `runCells`, `tableShape` | no | uPlot, canvas tiles | 4 4 4 | 12 |
| `record-diff` (critic) | Field-aligned diff of two records | side-by-side diff with inserted rows | structure | figure panel | inspector section, Table view compare | `templateChildren` of both, `TemplateNode.absent` | no | DOM table, `lanes.ts` | 4 4 4 | 12 |
| `value-pipeline` | Byte-to-value pipeline | stages: endianness crossing, IEEE bands, place values | bit | inline figure panel | inspector via `typepanel.ts` | `typeInfo` (Float, Fixed, Enum, Quant), `read` | no | flex row of DOM stages, d3-scale | 4 4 3 | 11 |
| `readings-lanes` | Same bytes read several ways | genome-browser lanes; agreement matrix | byte | figure | inspector selection, `stringsview.ts`, formatcards (overlap) | `SelectionText`, `StringPrefix`, `shape`, `read` | no | SVG lanes, d3-scale | 3 4 4 | 11 |
| `annotated-byte-strip` | Annotated byte strip: braces, seals, conditions | spec byte boxes with brace lanes; poster | structure | inline figure view | `bytestrip.ts`, `report/parts.ts`, hex rows (in-grid form) | `spans`, `relations`, `checkOf`, `runCheck`, `absent` | no | `bytestrip.ts` + d3-scale, `lanes.ts` | 5 3 3 | 11 |
| `record-pointer-boxes` | Box-and-pointer records, chains | Python Tutor heap boxes; linked list | structure | panel view | Diagram view mode, inspector (chain) | `graph` (points, position), `shape`, `SqliteRowInfo` | partly | elkjs with ports | 4 4 3 | 11 |
| `file-to-bit-zoom` | Semantic zoom from file to bit | powers-of-ten continuous zoom | file | view | new main view from `report/zoommap.ts` | `templateChildren`, `spans`, `read`, `readBits`, `locate` | no | board, d3 partition, canvas LOD | 4 5 2 | 11 |
| `stride-grid` | Stride raster and page allocation grid | byte raster at a stride; page cell grid | stream | panel view | overview tab | `read`, `tableShape`, `SqliteRowInfo`, `isoDirectory` | no | canvas, board, uPlot | 4 4 3 | 11 |
| `lz77-arcs` | LZ77 back-reference arcs | arc diagram over text | stream | figure panel view | stream tab, `report/streams.ts` | `mapOut`, `mapOutAny`, `decodedCode` | no | d3-shape arcs, board, uPlot | 4 3 4 | 11 |
| `railroad-grammar` | Railroad diagram with this file's counts | syntax diagram; state chart | format | figure view | Diagram view third mode | `templateDiagram`, `diagramCensus` | partly | railroad-diagrams, elkjs | 4 4 3 | 11 |
| `schema-class-er` | UML class and ER diagrams of declared types | UML class, crow's-foot ER | format | panel view | Logical tab figure, Diagram view | `rootContents`, `contents`, `sqlitesql.ts`, `templateDiagram` | no | elkjs with port labels | 4 4 3 | 11 |
| `dump-reconstruction` (critic) | Dump lines against the bytes they wrote | two-row ribbon, holes and conflicts marked | file | figure panel | formatcards card for the hexdump template, `report/bytes.ts` | `DumpScan` | no | `ribbon.ts`, d3-scale | 3 4 4 | 11 |
| `tlv-nesting-arcs` | Token stream with nesting brackets; tag tiles | Dyck-path brackets beside tidy tree | structure | inline figure panel | formatcards, heading glyph, Logical figure | `templateChildren`, `relations`, `Span.bits` | no | d3-hierarchy tree, d3-shape, board | 3 4 3 | 10 |
| `decides-matrix` | Adjacency matrix of what decides what | reorderable matrix | structure | panel view | `graphview.ts` mode | `graph`, `templateDiagram` | no | canvas, board | 2 4 4 | 10 |
| `expression-tree` | Expression tree and worked equation | annotated parse tree | field | inline figure | inspector origins, hover cards | `relations`, `origins`; tree needs an Expr DTO | yes (tree) | d3-hierarchy tree; card is DOM | 3 5 2 | 10 |
| `offset-frames-ruler` | Nested rulers for offsets from another zero | stacked number lines | structure | figure | inspector section, `filemap.ts` marker | `Origin.target_bits`, `read_at`; anchor base needs core | yes | d3-scale, d3-axis | 4 4 2 | 10 |
| `digraph-plot` | Byte-pair dot plot | 256x256 density matrix | file | figure panel | `minimappanel.ts` block close-up, `report/bytes.ts` | pair counts (worker over `read`; core past a few MiB) | partly | canvas ImageData, board | 2 4 4 | 10 |
| `pipeline-sankey` | Decode pipeline funnel with in/out ratio glyph | funnel of bars; trapezoid glyph | stream | inline figure panel | `steplist.ts` hosts, heading glyph | `ChunkInfo.steps`, `PageInfo`, `unpacking`, `JoinedPart` | no | bars on one log scale, uPlot | 3 3 4 | 10 |
| `step-decoder-timeline` | Step-through decoder with timeline | explainer stepping, lane timeline | stream | panel view | inspector (deflate, JPEG scan), rail panel | `decodedCode`, `mapIn`, `jpegScan`, `jpegBlock` | partly | d3-brush, uPlot, `ribbon.ts` | 3 4 3 | 10 |
| `timestamp-timeline` | Timeline of the file's timestamps | forensic time axis with lanes | field | figure panel | `report/brief.ts`, overview tab, row glyph | `timeOf` per node | partly | d3-scale time + lane stacker | 3 4 3 | 10 |
| `hilbert-map` | Hilbert-curve byte map | space-filling curve | file | panel view | `minimappanel.ts` layout toggle | `overviewStep`, `overviewFocusStep` | no | canvas, 20-line d2xy, board | 2 3 4 | 9 |
| `order-vs-address-ribbon` | Stream order against file order | two rows joined by crossing bands | stream | figure panel | Logical row figure, inspector | `partOf`, `partAt`; parts list needs core | yes | d3-scale, d3-shape, `ribbon.ts` | 4 3 2 | 9 |
| `shape-parallel-sets` | Parallel sets of how fields were placed and sized | alluvial; glyph vocabulary | format | inline figure panel | `report/profile.ts`, row glyph | `templateProfile`, `shape`, ledger roles | no | d3-sankey | 2 3 3 | 8 |

### 4.2 Candidates, one by one
Ordered by scale, smallest first: bit, byte, field, structure, stream, file,
format. Each entry says what it looks like, what it shows, which formats it
pays off on, how it is explored, how its labels are kept apart, what it nests
in or holds, what existing piece it extends, and what the judges found.

#### Bit scale

##### `bit-pack-ribbon`: Bit-packing ribbon for any LSB-first field (14)
- Looks like: two rows of single-bit boxes. The top row is every byte the field
  touches, written high bit first as the hex view shows them, with other
  fields' bits faint. The bottom row is the field (or each part of a code) as
  the number it is read as. Coloured bands run from each stored bit to its
  place in the value, crossing where LSB-first packing mirrors the order and
  visibly splitting a field that straddles a byte boundary. An MSB/LSB toggle
  shows the wrong reading for comparison.
- Shows: why a value packed from the bottom of the byte looks mirrored against
  the hex, and which byte each bit came from.
- Formats: deflate codes in gzip, zip, png and `compressed`; GIF LZW codes;
  FLAC subframes; MP3 side info; JXL headers; JPEG scan codes with FF 00 stuffing.
- Interaction: hover a band lights the bit at both ends and fades the rest;
  click a bit box goes to its byte in hex (new); arrow keys step code by code
  through a stream. No zoom; the figure widens with its host.
- Labels: bit cells have a fixed pitch and one digit each; part labels sit in
  the bottom row one per part, and a part narrower than its label is hover-only.
- Nesting: one part of the step-through decoder; can be embedded in the bit
  ruler's hover card.
- Overlaps: a direct extension of `bitorder.ts`, which today fires only for a
  deflate code in the inspector; the host condition becomes any `Span` with
  `lsb_first` or any sub-byte field.
- Judges: R4 (answers a real confusion; shares a home with the bit ruler's
  MSB/LSB toggle), L5 (already proven in `bitorder.ts`; bounded at 64 bits),
  F5 (`bitOrderLayout` and `DecodedCode.code_bits`/`extra_bits` exist; about a
  day). Library stays `ribbon.ts`.

##### `bit-ruler-register`: Bit ruler, RFC header and register map (13)
- Looks like: rows of 8, 16 or 32 bit columns with a bit-number ruler on top
  and byte offsets down the left, as an RFC draws a header. Each sub-byte field
  is a box spanning its bits with its name inside and its value under it; a
  field crossing a row wraps. Set flag bits are filled, unnamed set bits
  hatched and marked undefined. A toggle switches the numbering between
  MSB-first (7..0) and LSB-first (0..7) so `lsb_first` fields read as their
  spec draws them. A small-multiples variant stacks the same register for every
  element of a list (one row per ZIP entry's general-purpose flags), so a
  column of identical bits reads as a stripe and the odd one out shows.
- Shows: how several sub-byte fields and flags share one word, which bits are
  named, which are set, and which direction the format numbers them.
- Formats: PE Characteristics and DllCharacteristics; ELF p_flags and sh_flags;
  ZIP general-purpose flags; MP3 frame header; FLAC STREAMINFO; gzip FLG;
  pcap and pcapng headers; deflate BFINAL/BTYPE/HLIT/HDIST; JPEG SOF sampling
  nibbles; BUFR descriptors (F, X, Y).
- Interaction: hover a box: name, bit range, raw value, meaning and the
  shift-and-mask formula (`bitextract.ts`), with those bits lit in hex. Click
  goes to the field. Wheel zooms at panel size; an 8/16/32 width switch
  re-wraps the rows; MSB/LSB toggle.
- Labels: every label sits inside its own box; a name wider than its box is
  dropped from the box, listed in a numbered legend under the lane and shown
  on hover. Rotated text is not used.
- Nesting: one lane per word; stacks as small multiples per record; sits inside
  the annotated byte strip or a value-pipeline stage; the inline form is one
  8-cell byte glyph beside a sub-byte row.
- Overlaps: draws the bits `hexcell.ts` `fieldBits` and `bitextract.ts`
  describe in text; takes over the bit-role chip's colours.
- Judges: R5 (the hex view cannot show which bits belong to which field at
  all; the per-record stripe makes the odd entry stand out), L4 (bounded to
  one word; the per-record stack needs virtualised rows past a few hundred
  entries), F4 (data is all there: `Span.offset_bits/size_bits/lsb_first`,
  `FlagsInfo.bits`). Overruled: drop the `bit-field` npm dependency, which
  outputs static SVG with no hover hooks; a fixed-pitch SVG grid is less work.
  Lives in its own module, not in `inspector.ts`.

##### `varint-gather`: Varint byte-gathering figure (13)
- Looks like: the stored bytes in a row, each split into its continuation bit
  (or EBML width marker, or SQLite ninth-byte rule) and its payload bits,
  coloured by role, as the protobuf documentation draws it. Bands carry each
  payload group down into the reassembled binary number, crossing where groups
  are stored low-first (LEB128) and straight where high-first (EBML, SQLite).
  Below: the binary, the decimal, and the zigzag unfold where the format uses
  it. The inline form is a tiny bar of byte cells with the "more" bits as
  notches, so a reader sees it is 3 bytes at a glance.
- Shows: which bits of a variable-length number are the number and which are
  bookkeeping, and in what order the groups join.
- Formats: wasm (LEB128 sizes); SQLite cell varints; Matroska EBML IDs and
  sizes; MIDI delta times; Parquet's Thrift compact footer; pickle and torch
  frames.
- Interaction: hover a payload group lights its bits in both rows and shows the
  shift amount; click a byte goes to it in hex; zigzag toggle where the rule
  says so. No zoom.
- Labels: role names live in a fixed legend, not per group; the one free label
  is the decimal result under the bottom row; byte cells carry only their hex.
  Bit pitch is fixed and the rows wrap at rail width, so a 10-byte LEB128 (70
  payload bits) does not tangle its bands.
- Nesting: sits inside the TLV figure's varint inset and the value pipeline as
  a stage.
- Overlaps: extends `bytestrip.ts` `bitsChip` (private today, needs exporting)
  with the gathering bands; reuses `ribbon.ts`.
- Judges: R4 (shows the one thing a varint hides; a handful of formats), L4
  (tiny and bounded; fix the pitch and wrap), F5 (`Span.bits` is exactly
  `BitRoles {rule, groups[{bits, role}]}`; `bandPath`/`scaleOf` exist in
  `ribbon.ts`).

##### `huffman-tree`: Canonical Huffman tree with code table and length histogram (12)
- Looks like: a binary tree with 0 on left edges and 1 on right, symbols at
  the leaves, drawn so each leaf's depth is its code length and all leaves of
  one length line up, as RFC 1951 and CLRS draw it. Beside it the canonical
  table: length, count of codes at that length (the 16 BITS values from the
  file), the codes in order and the bits for each symbol. The existing 16-bar
  length histogram sits above the tree and maps bars to depths. The path of
  the code under the cursor is lit.
- Shows: the shape of the code set the file wrote only as counts and symbols,
  what each code means, and why short codes are the common symbols.
- Formats: JPEG DHT; deflate dynamic blocks in gzip, zip, png and
  `compressed`; MP3 tables; FLAC Rice parameters as a variant.
- Interaction: hover a table row lights its path in the tree and the reverse;
  hover a leaf: symbol, code, width; click a symbol jumps to the next place in
  the scan or stream where that code occurs; click a table row goes to the
  byte that set its length; wheel pans and zooms the tree; branches collapse
  below a depth for 288-symbol deflate trees.
- Labels: the table always carries every label, which is the fallback that
  guarantees legibility. Leaves show a dot below a zoom threshold and their
  symbol on hover.
- Nesting: sits inside the DHT card; the step-through decoder lights paths in
  it; the histogram is the existing bars.
- Overlaps: extends the JPEG Huffman card (`jpegcards.ts`: hand-rolled bars, a
  flat list, no tree) and generalises to deflate tables.
- Judges: R4, L4, F4. Overruled: use `d3.tree`, not `d3.cluster`, because
  cluster puts every leaf on one baseline, which contradicts "depth equals
  code length". Skip Observable Plot; the histogram bars exist. JPEG DHT is
  about a day with `jpegtables.ts` `huffmanCodes`; deflate dynamic tables
  (code lengths from `templateChildren` and `DecodedCode.entry`) are more.

##### `value-pipeline`: Byte-to-value pipeline (11)
- Looks like: a chain of stage boxes for one field, left to right: the stored
  bytes in file order; the same bytes in significance order joined by straight
  (big-endian) or crossing (little-endian) lines; the bits with their place
  values under them (sign bit as -2^n, binary point for fixed-point); the
  split the type uses (sign | exponent | mantissa; scale and packed nibble for
  a quantised weight; the enum lookup); the arithmetic line with numbers
  filled in; the final value with unit. For floats, a small log number line
  shows the neighbouring representable values and the ULP gap.
- Shows: how the bytes under the cursor become the number the listing prints,
  one transformation per stage.
- Formats: GGUF (f16, bf16, q4_0 blocks); safetensors; npy; FITS (BITPIX -32
  and -64, BSCALE); WAV float samples; TIFF (II against MM); pcap magic; font
  F2Dot14 and Fixed 16.16; GRIB (R + X x 2^E) / 10^D.
- Interaction: hover a bit or byte lights it in every stage it passes through
  and names its contribution; click a bit flips it in a local preview only (no
  edit) and the value updates; click the strip goes to the bytes; stages
  collapse; wraps to rail width.
- Labels: bit labels sit in fixed-width cells; the formula is one line of
  plain text that wraps under the strip.
- Nesting: contains the bit ruler and the varint figure as stages; opens from
  a value in any table or card.
- Overlaps: replaces the flat text of `quantpanel.ts` recipe/packing/sources
  and the GRIB and BUFR cursor lines; the bit extraction formula becomes one
  stage. The inline form is a two-row endianness glyph for any 2/4/8-byte
  selection in the selection readout.
- Judges: R4 (pays off for floats, fixed-point and quantised blocks; overkill
  for plain integers, so lead with the inline endianness glyph), L4 (flipping
  a bit in a preview is a strong verb), F3 (several type-specific stage
  renderers). Overruled: no ELK for a straight chain (use a flex row, which
  wraps at rail width) and no KaTeX for one formula line.

#### Byte scale

##### `readings-lanes`: Same bytes read several ways (11)
- Looks like: one byte strip on top. Under it one lane per reading: u8 = 5,
  u16le = 5, u32be = 83886080, LEB128 = 5, UTF-8 text, UTF-16LE text starting
  one byte later. Each reading is a bracket over exactly the bytes it
  consumed; a length reading that lands on the end of a run gets a tick;
  believed readings solid, weak ones dashed. A second form is a matrix of
  encodings (columns) against distinct texts (rows), so "ASCII, Latin-1, CP437
  and UTF-8 agree" is one wide row. Union fields draw as stacked lanes, one
  per variant, at the same x-scale.
- Shows: the competing readings of one stretch of bytes, which ones agree, and
  where a reading starts a byte late or ends early.
- Formats: PE UTF-16 resources and .NET strings; pickle and Parquet counted
  strings; font name table UTF-16BE; WAV samples holding text; ADIOS BP5
  entries read twice; ELF string tables.
- Interaction: hover a bracket: type, value and the bytes it consumed, lit in
  the strip; click a lane goes to that reading's path or selects those bytes;
  no zoom (selections are short; cap at a few hundred bytes).
- Labels: each lane is a fixed-height row with one label at its left edge, so
  lanes cannot collide; bracket captions are drawn only when the bracket is
  wider than the text, else on hover.
- Nesting: sits under a byte strip; the matrix form fits a card; a lane can
  hold a varint figure.
- Overlaps: draws what the selection readout and the strings view say in text;
  extends `bytestrip.ts` with lanes.
- Judges: R3 (mostly redraws text the readout already gives; the union layer
  cake is the stronger part), L4, F4. Notes: `SelectionText.readings` is only
  `{encodings[], text}`, so numeric lanes are computed web-side from `read`
  (trivial); `StringPrefix` has a `weak` flag, not believed/possible;
  `Shape.placed === 'overlap'` exists.

#### Field scale

##### `index-lookup` (critic): Index and string-table lookup (13)
- Looks like: three boxes in a row joined by one arrow and one bracket. Left:
  the field holding the index or offset (`sh_name = 0x1B`), drawn as its bytes
  with the value under them. Middle: the table it indexes, drawn as a strip of
  entries with the one it lands on lit and its neighbours faint (for a string
  table, the NUL-separated names; for a palette, the swatches; for a constant
  pool or a TIFF IFD, the entry rows). Right: the resolved value (`.text`, a
  colour swatch, the tag's name). The inline form is the existing `-> reads`
  link in the listing with a hover card holding this figure.
- Shows: the commonest indirection in a binary: a small number somewhere that
  means an entry somewhere else, and what that entry says.
- Formats: ELF `sh_name` into `.shstrtab` and `st_name` into `.strtab`; PNG
  palette indexes (pixels into PLTE); TIFF tag value offsets; GGUF and HDF5
  string offsets; Java class constant-pool indexes; wasm name sections;
  SQLite `sqlite_master` root pages.
- Interaction: hover the index lights the entry; hover the entry lights the
  bytes in hex; click any box goes to it; left and right arrows step to the
  previous or next entry of the table and show which fields index that one.
- Labels: each box has one label in a fixed slot; entry text inside the strip
  is drawn when it fits, else on hover.
- Nesting: the figure is the hover card of `-> reads` links, DAG edges with
  role `name`, and pointer arcs; the table strip can hold a file-map locator.
- Overlaps: the pointer arcs show the same edge as a line; the listing's
  `readsLink` names the target in text. Draws `Relation` role `name` and
  `Origin` edges that exist today.
- Judges (critic's scores): R4 (the lookup is where most "what does this
  number mean" questions end), L5 (three boxes in fixed slots cannot
  collide), F4 (`origins`, `relations`, `templateNode` and `TemplateNode.colour`
  give the data; the table-entry strip needs a per-kind reader for string
  tables, palettes and entry lists).

##### `parse-failure-explainer` (critic): Expected against found where the template stopped (13)
- Looks like: two rows of byte boxes at the point a problem was raised: what
  the template expected (the magic bytes, the enum's cases, the claimed
  length as a bracket) over what the file held, with the differing bytes red.
  To the left, the chain of fields that led there (`chunk[47].length = 0x7FFF
  -> data ends at 0x8120 -> file ends at 0x8000`) as a short column of chips,
  each lighting its bytes. For a run that stopped short, the `extentfigure.ts`
  bars (stated end, read end, parent end, file end) sit under the rows. For an
  enum with no case, the cases as a list with the found value parked outside
  it. For a wrong signature, the candidate rules from `SigVerdict` whose magic
  bytes do match, each as a faint row below.
- Shows: what the format wanted, what the file had, and which earlier value
  made the difference, which is the deviation a reader opened the file to find.
- Formats: every templated format; strongest where files are often hand-edited
  or truncated: png, zip, wav (RIFF sizes), tar checksums, pcap, elf, pdf.
- Interaction: hover a chip or byte: the field, its value, where it was read;
  click goes there; click a candidate rule offers that template, as the
  identification dialog does.
- Labels: byte boxes carry two hex digits at fixed pitch; the chain chips are
  one per line; the extent figure's own labels follow the lane rule.
- Nesting: the figure is the hover card of every problem glyph (listing, hex
  headings, treemap red edge, scrollbar track); it holds the extent figure
  and a file-map locator.
- Overlaps: `Problem.text` and the `insp` type panel's two rows of magic bytes
  say this in text; `report/findings.ts` lists stated against expected values
  in a table.
- Judges (critic's scores): R5 (DESIGN headings 2966 and 2985 describe the
  need and nothing draws it), L5 (short, fixed slots), F3 (`Problem` is text;
  `MagicInfo` carries the expected bytes and `extent_audit_step` the ends, but
  "expected" for an enum or a count must be read from `EnumInfo` and
  `relations` per kind; a structured expected-value DTO is a later core help).

##### `expression-tree`: Expression tree and worked equation (10)
- Looks like: the expression behind a field (`header_size -
  sizeof(header_size)`, `Product of dims`, `SumOf counts`) as a small tree:
  operators in circles, fields and literals as leaves, each node annotated
  with the value it came to. Under it the three-line card the `Relation`
  already gives: written, substituted, result, each field name a chip coloured
  by role that lights its bytes on hover. For a fallback chain (ZIP64: size is
  0xFFFFFFFF? then the extra field tag 1, then the data descriptor) the tree
  becomes a small flowchart with the branch taken bold.
- Shows: how the number that sizes or places a field was computed, from which
  fields, and which branch of a fallback was taken.
- Formats: zip (ZIP64 fallbacks); png; wav (`header_size - sizeof`); gguf
  (`Product of dims`); jpeg DHT `SumOf counts`; fits `PadTo`; dos
  `header_paragraphs * 16`; pe `pe_header_offset - 64`.
- Interaction: hover a node: its value and where it was read, bytes lit in
  hex; click a leaf goes to its bytes; no zoom.
- Labels: operators are one or two characters inside circles; leaves are
  spaced by the tidy layout; values are a second line inside the node. A
  SumOf or Product over a long array folds its arguments into one "sum of N"
  leaf.
- Nesting: the hover card of pointer arcs, DAG edges and matrix cells; a leaf
  opens the value pipeline.
- Overlaps: draws the origins panel's `Relation` lines.
- Judges: R3 (the worked-equation card and the ZIP64 branch are valuable; a
  tree for `a - 4` is overkill), L5 (the most legible piece here), F2 for the
  tree (`Relation` is strings only; the tree needs an Expr DTO through
  `crates/wasm`). The worked-equation card with role chips alone is a 5 today
  and needs no core change.

##### `timestamp-timeline`: Timeline of the timestamps the file records (10)
- Looks like: a horizontal time axis with one item per time field the file
  writes: ZIP entry times, tar mtimes, PE TimeDateStamp, PDF creation and
  modification dates, journal entries, miniSEED record start times as a band.
  Items grouped in lanes by the field they came from; each a dot with an
  error bar of its precision (`step_nanos`); local or unknown zones hollow;
  unset (1980 zero dates) and impossible values drawn distinctly and parked
  at the axis ends. The inline form is a small clock glyph beside a time
  field with unset and impossible as distinct marks.
- Shows: when the file and its members were made, which timestamps disagree,
  which are placeholders, and how precise each is.
- Formats: zip (MS-DOS times), tar, gzip mtime, pe, pdf, journal, cdf
  (TT2000), font head dates, seismic record times, iso.
- Interaction: wheel zooms and drag pans along time; hover an item: date,
  field, raw count, epoch and step; click goes to the field; lane toggles.
- Labels: overlapping items stack into separate rows by the lane helper; item
  labels only when the item is wide enough; lane labels in a fixed left
  gutter; time ticks from d3-axis.
- Nesting: an item's hover card holds the epoch figure; the glyph nests in
  rows and record tiles.
- Overlaps: draws the `FieldTime` the inspector shows one line at a time.
- Judges: R3 (about the file's history more than how bytes fit; the inline
  unset/impossible glyph is the cheap win), L4 (dense lanes past a few
  thousand items become a density band), F3 (`FieldTime` is complete, but
  finding every time field means `timeOf` on every node, so it wants a core
  list for big files). Overruled: no `vis-timeline`; d3-scale time with the
  lane stacker is lighter.

#### Structure scale

##### `pixel-grid-linked`: Pixel grid linked to bytes, with block and pass overlays (13)
- Looks like: the decoded image (or a raw byte range rendered at a chosen
  width) in a pan-and-zoom canvas with nearest-neighbour scaling; at high zoom
  each pixel shows its hex value. Overlays: for JPEG, the 8x8 block grid and
  MCU outlines with bits-per-block as a heat layer per channel (Y, Cb, Cr tabs)
  at adjustable opacity, restart intervals as stripes and the MCU order path;
  for PNG, the Adam7 pass of each pixel as a tint and the filter type per
  scanline as a thin coloured column beside the image; for FITS tiled images,
  the tile grid with the tile under the cursor lit.
- Shows: which bytes make which pixels, where the scan spent its bits, and
  how passes, filters and tiles divide the picture.
- Formats: jpeg; png and apng (Adam7, filters); bmp, tga, pcx, gif, qoi;
  fits tiles; pico8 and picotron sub-byte pixels.
- Interaction: wheel zooms around the pointer, drag pans; hover a pixel:
  coordinates, value, byte offset, scanline filter, pass, MCU cost; click goes
  to the bytes (and for JPEG picks the block for the code strip and
  coefficient grids); selecting bytes in hex lights the pixels they make;
  opacity and channel controls; a width slider with autocorrelation-suggested
  widths for raw ranges.
- Labels: hex values per pixel only above a zoom where a cell is at least
  24px; MCU costs only in the hover card and a step legend; the filter column
  has a legend, not per-line text.
- Nesting: hosts the JPEG block code strip and coefficient grids as its block
  detail; the stride raster is its templateless form; the scanline filter
  strip nests as a side column.
- Overlaps: extends the content card (no zoom, no pixel-to-byte link), the
  MCU cost map (no zoom or pan) and the PNG pass and filter figures (not
  linked to each other), merging the three.
- Judges: R5 (the most direct "these bytes make this" link possible for
  images), L5 (nearest-neighbour canvas with the board; very large images
  need tiling past the canvas size limit), F3 (`JpegScan`, `MapStep` kind
  `pixel`/`filtered`, `pngpasses.ts` and `cardState` exist; merging three
  figures with linking both ways is several days).

##### `value-plot` (critic): Numeric run as a plot: waveform, heatmap, column lines (13)
- Looks like: three forms of one piece. A run of numbers (WAV samples, miniSEED
  samples, a GGUF or npy tensor, a FITS or Parquet column) as a full-width line
  with min-max decimation, a crosshair giving sample index, value and byte
  offset, and the cursor's element marked. A two-dimensional run (a tensor
  with two dims, a FITS image plane, an HDF5 dataset) as a colour grid with a
  colour bar. A table as one line per numeric column on a shared x. The inline
  form is the one-line sparkline of `record-small-multiples`; this piece is
  the figure, panel and view sizes the judges said sparklines cannot cover.
- Shows: the shape of a long run of plain numbers: silence and clipping in a
  waveform, a spike or a step in a sensor record, the layout of a weight
  matrix, a column's distribution before scrolling it.
- Formats: wav, aiff, au; seismic (miniSEED, Steim); gguf, safetensors, npy,
  torch; fits images and tables; hdf5, netcdf, zarr; parquet, arrow; segy.
- Interaction: wheel zooms along x around the pointer, drag pans, brush zooms
  and refetches the range at finer decimation; hover a point or cell lights
  its bytes in hex and shows index, value, offset and type; click goes to the
  element (`[...path, index]` as the hex value cells do); the hex cursor moves
  the marker; log/linear y toggle; for the heatmap, a colour-range clamp.
- Labels: axis ticks from uPlot; values only in the cursor legend; a colour
  bar with ticks for the heatmap; column names in a legend, never on the lines.
- Nesting: nests in the Table view under a column header at figure size, in
  the inspector for a `SamplesInfo` record, and in the list pane; holds a
  file-map locator for the visible range.
- Overlaps: the hex value cells draw the same numbers one row at a time;
  `samplespanel.ts` lists the first values as text; the quant grid draws one
  block of weights as buttons.
- Judges (critic's scores): R5 (named three times in the missing list; turns
  "a long run of plain numbers" into something readable), L4 (standard chart
  forms; the heatmap of a 4096x4096 tensor needs tiles or a sampled grid with
  the sampling stated on screen), F4 (`runCells` gives text, so for speed the
  fixed-width int and float runs are decoded web-side from `read` with a
  `DataView` using `TemplateNode.type`; Steim and other decoded runs read
  `runCells` or `SamplesInfo.values` and are capped; a typed-array core call is
  the later speed-up).

##### `struct-layout-padding`: Struct layout with padding holes (12)
- Looks like: a vertical stack of word-wide rows (4, 8 or 16 bytes,
  switchable) with offsets in the left gutter, as pahole prints a struct. Each
  member is a coloured box at its exact offset in the kind colour the hex view
  uses; padding, alignment slack and unread gaps are hatched and labelled with
  their byte count ("3 bytes padding", "unread"). A right margin gives each
  member's size and inferred alignment; a summary line gives total size,
  holes and wasted bytes. Nested structs indent with a bracket; arrays show
  one element with a repeat brace; a union stacks its alternatives.
- Shows: where each member sits inside its record, how much of the record is
  padding or unread slack, and where the writer left holes.
- Formats: elf (header, program headers); pe (optional header 32 and 64 side
  by side); gguf (tensor info, aligned data); cdrom 2352-byte sector anatomy;
  dtb 4-byte alignment; cpio; journal 8-byte objects; adios FFS records; rpm
  header store.
- Interaction: hover a member or hole: type, offset, alignment, value; click
  goes to the field; the row width toggle (1/2/4/8/16) re-wraps; wheel
  scrolls long structs; a hole click selects the slack bytes (24px minimum
  hit area).
- Labels: names inside boxes only when the measured text fits, otherwise a
  numbered key beside the row; holes always carry only a byte count.
- Nesting: repeats per record as small multiples (virtualised past 50k); a
  member box opens the value pipeline; the record's file-map locator sits in
  the summary line.
- Overlaps: instance-level and to scale, unlike `strips.ts` (template,
  text-width boxes) and the byte strip (no proportion); shares `TemplateNode`
  reading with `cardparts.ts`.
- Judges: R4 (file formats rarely have compiler padding; unread gaps and
  alignment slack are what the reader learns from), L4 (CSS grid places the
  boxes; fit-or-key needs measured text), F4 (`templateChildren` and
  `Span.gap` give the data; alignment is inferred from offsets; unions and
  nesting add a day).

##### `record-small-multiples`: Small multiples per record, sparklines per field, column sparks (12)
- Looks like: a wrapping grid of identical small tiles, one per record (PNG
  chunk, ZIP entry, JPEG segment, SQLite page, ELF section, MIDI event), each
  coloured by type and filled by its length, in file order; a run of IDAT
  tiles, a lone tEXt tile and an odd one out stand out, and problem tiles get
  a red edge. Beside a field that repeats, a one-line sparkline (min-max
  envelope across the run, the current record marked): chunk lengths, entry
  sizes, timestamps, sample values; a repeat glyph collapses identical
  neighbours to one cell with a count. Under each Table view column header,
  an 80x24 histogram, top-values bar or null-fraction bar with min and max
  at the ends.
- Shows: the shape of a list at a glance: which records differ, where a value
  spikes or goes backwards, and what each column holds before scrolling.
- Formats: png, zip, jpeg, sqlite, elf, midi, wav samples, seismic record
  times, gguf tensor sizes, parquet, fits tables, pickle frames.
- Interaction: hover a tile or point: record index, type, size, value,
  offset; click goes to that record; a type legend filters tiles; wheel zooms
  the tile grid so tiles grow to show their own file-map strip; the
  sparkline's marker follows the cursor's record; click a spark bin
  highlights the rows in it.
- Labels: tiles and sparklines carry no labels at small sizes; type names in
  the legend; only min and max as text at the sparkline's two ends in fixed
  slots; column sparks have no axis.
- Nesting: tiles hold a file-map strip or a bit-ruler row when zoomed;
  sparklines nest in headings, hex value cells and Logical rows.
- Overlaps: fills the listed gaps of the heading row (no count sparkline),
  the gap row (no mini picture), the hex value cells (no sparkline of the
  run) and the table view (no per-column distribution).
- Judges: R4, L4 (draw the tile grid on canvas or virtualise it; collapse
  repeated neighbours before laying out), F4 (`templateChildren` and
  `runCells` with `Cell.repeat` give tiles and sparklines directly; three
  placements, each about a day). Overruled: no Observable Plot cell mark
  (SVG chokes at 50k tiles).

##### `record-diff` (critic): Field-aligned diff of two records (12)
- Looks like: two records of the same type side by side as field rows aligned
  by name, as a two-column diff: equal fields quiet, differing values lit with
  both values shown, a field present on one side only (an optional part, a
  chunk the other lacks) as an inserted row with a gap opposite, a differing
  size as a bracket. Above the rows, two byte strips to the same scale with
  the differing bytes lit. The two records can be two elements of one run
  (this ZIP entry against the previous, or against a pinned one), two
  instances of a type anywhere in the file, or, later, the same path in two
  open files.
- Shows: what is different about this record, which is how a reader explains
  the odd one out the small multiples found.
- Formats: zip (entries), png (chunks), elf (section headers), pe (sections),
  sqlite (pages), midi (events), tar (headers), gguf (tensor infos), any list.
- Interaction: shift-click two records in the listing or the Table view, or
  "compare with previous" from a run element in the inspector; hover a row:
  both values and both offsets; click either side goes to that field; pin one
  side and step the other through the run with arrow keys so the diff plays
  through the list.
- Labels: one row per field, so names cannot collide; values truncate with a
  hover card; the byte strips follow the byte-strip rules.
- Nesting: a row opens the bit ruler for a flags field or the value pipeline;
  each side carries a file-map locator.
- Overlaps: the Table view shows both as rows but not aligned by difference;
  nothing marks a difference today.
- Judges (critic's scores): R4 (named twice in the missing list), L4 (a table
  with lanes for inserted rows), F4 (`templateChildren` for both records and
  alignment by name; `TemplateNode.absent` for optional parts; two files needs
  a second `Doc`, which is a later step).

##### `annotated-byte-strip`: Annotated byte strip: braces, seals, conditions, poster mode (11)
- Looks like: the existing byte strip redrawn to scale: one box per field,
  width proportional to bytes with a minimum, a long payload elided in the
  middle with "... N bytes ...". Above and below, braces in lanes: "length =
  13 counts these bytes", "CRC-32 covers these", tied by a thin line to the
  field that states them, as the PNG and zlib specs draw them; a brace that
  disagrees with the real span is red with both extents. Conditional parts
  ("if FLG.FEXTRA") get their own row and are greyed when absent. At view
  size the same data becomes a poster: a real hex dump with each field tinted
  and leader lines to margin callouts grouped by structure. The hex-view form
  (the judges' in-grid annotations): underlines and brackets in `hexrows.ts`
  for the selected field's relations, so the dependency is drawn on the bytes.
- Shows: which bytes a length counts, which bytes a checksum seals and
  whether it holds, which parts are present in this file and why.
- Formats: png (chunk length and CRC); zip (CRC32 against compressed size);
  gzip (FLG-dependent FEXTRA and FNAME, ISIZE); tar (header checksum with the
  blanked field); wav and aiff RIFF sizes; elf e_ehsize; pcap IPv4 IHL and
  checksum; hdf5 fletcher32.
- Interaction: hover a brace: stated against actual and the `Verdict`; hover
  a condition caption: the flag bit that decided it, lit in hex; click a
  brace selects the covered range; click a box or callout goes to the field;
  wheel zooms the poster and long payloads.
- Labels: overlapping braces take separate lanes by interval colouring; a
  brace label sits in its lane only when the brace is wider than the text,
  else a numbered key; poster callouts are placed by a 1-D placer with
  leaders and overflow into a second column.
- Nesting: carries the file-map locator in its caption; a box holds a bit
  ruler; repeats per record in a card.
- Overlaps: extends `bytestrip.ts` with proportion, braces and conditions;
  the extent figure becomes the red brace case; the stream stages bars fold
  into the seal lane for `unpacked_from`.
- Judges: R5 (the core of how bytes fit together; the listing cannot show
  coverage at a glance), L3 (three pieces in one; busy headers need many
  lanes), F3. Overruled: reject `bytefield-svg` (a ClojureScript DSL bundle
  with no interaction) and extend `bytestrip.ts` with d3-scale; the poster
  mode with callouts is a separate multi-day piece; the file-wide seal Gantt
  wants a core list of check fields.

##### `record-pointer-boxes`: Box-and-pointer records, chains and reference graph (11)
- Looks like: each record a rounded box with its fields as stacked slots; a
  slot holding an offset or index shows a dot with a curved arrow to the box
  it points at, as Python Tutor draws a heap. Boxes are laid out by
  reachability, not by address: a header on the left, what it points to
  fanning out to the right, each box with a thin address tag and a tiny
  file-map locator. A chain (SQLite overflow pages, CFB sector runs, TIFF
  next-IFD) lays out left to right in chain order with the null drawn as a
  ground symbol and a broken or looping hop in red; under it a file strip
  lights the same pages in file order so the jumps show. A later mode draws a
  record reference graph (PDF objects from the trailer, HDF5 object headers)
  and its dominator tree with retained bytes; that mode needs reference edges
  from the core.
- Shows: which records point at which, what a chain looks like followed hop
  by hop, and how scattered the hops are in the file.
- Formats: zip; pdf (xref rows, /Prev chain); elf sh_link and sh_info; tiff
  IFD chain and out-of-line values; sqlite overflow pages; doc/xls/ppt CFB
  sector chains; pe data directories; hdf5 object headers; minidump directory.
- Interaction: wheel zooms, drag pans; hover a slot: value and target address;
  click a box goes to its bytes; click a slot dot expands a collapsed target
  (lazy); "collapse runs" folds 20 contiguous sectors into one box marked x20.
  Existing node positions are pinned (ELK interactive mode) on expand so the
  layout does not jump.
- Labels: field names inside boxes ELK sizes to their text; no edge labels
  (the arrow starts at the named slot); long names truncate with a hover card.
- Nesting: each box carries a file-map locator; a box opens as a struct
  layout; the chain form sits under a heading.
- Overlaps: reachability layout of the edges the pointer arcs draw on an axis;
  reuses the Diagram view shell; the chain form replaces the xref panel's
  text rows for /Prev.
- Judges: R4, L4, F3 (the Diagram view's boxes are type-level; instance boxes
  need a new adapter from `graph()` and `templateNode`; `SqliteRowInfo.pages`
  exists; PDF refs and dominators need core).

##### `tlv-nesting-arcs`: Serialised tree: token stream with nesting brackets and tidy tree (10)
- Looks like: the records of a serialised tree in file order along a line:
  tag, length, value boxes with a brace from each length to the value it
  measures; nesting shown as bracket arcs above, each open joined to its
  close, stacked like a mountain range. Varint lengths get a small inset
  splitting marker from payload. Beside the stream, the same nesting as a
  tidy tree; hovering links a token to its node. A chunk-sequence glyph form
  folds runs of identical tags into one tile with a count (IHDR PLTE IDAT
  x4096 IEND).
- Shows: what comes after what, what is inside what, and how each length
  steps a reader over a record it does not understand.
- Formats: matroska (EBML), mp4 boxes, png chunks, wav and avi RIFF, dtb,
  msgpack, bson, torrent (bencode), zip extra fields, tiff IFD entries.
- Interaction: wheel zooms along the stream, drag pans; hover an arc lights
  its subtree in both drawings and shows tag, length, value; click a token
  goes to its bytes; click a tile in the glyph goes to the first such record.
- Labels: each nesting depth is its own lane row so brackets at one depth
  never overlap; token labels appear only when they fit at the current zoom;
  the tree side places labels by the tidy layout and collapses runs.
- Nesting: a token opens the struct layout; a value opens the varint figure;
  the tag-tile glyph nests in headings and treemap boxes.
- Overlaps: instance-level companion to the template-only `strips.ts`; the
  tag-tile glyph gives the heading row a picture of its children.
- Judges: R3 (the listing tree already shows nesting; the folded tag-tile
  glyph is the high-value part and could stand alone), L4, F3 (detecting "a
  list of tagged records" generically is fuzzy without a template hint; the
  glyph alone is a 5).

##### `reads-dag`: Layered dependency DAG, what must be read before what (12)
- Looks like: fields as small rounded boxes coloured by kind, in
  left-to-right layers: a field sits one layer right of every field that
  decided something about it, as a compiler's data-dependence graph. Edges
  coloured and labelled by role (length, count, type, position, condition,
  points). A PNG chunk's length is in layer 0, data in layer 1, the CRC that
  covers both in layer 2; a GGUF header count, tensor infos, offsets and
  tensor data read as one straight run. Hovering a node dims everything not
  upstream or downstream of it.
- Shows: the reading order the template implies and the chains of decisions,
  without the address noise of the force layout.
- Formats: png, gguf, parquet footer, elf headers, zip central directory,
  midi, sqlite.
- Interaction: wheel zooms and drag pans via cytoscape; hover a node: name,
  type, offset, size (the Graph view lacks this today); hover an edge: the
  `Relation` written, substituted, result; click goes to `GraphNode.path`
  and dims non-ancestors and non-descendants.
- Labels: ELK sizes nodes from measured labels and places edge labels itself;
  role words shown only for the hovered or selected node's edges past about
  60 edges. Repeated records fold into one node marked xN before layout.
- Nesting: a node's hover card holds the expression tree; the fold-node rule
  from `graphview.ts` collapses plain fields of a structure.
- Overlaps: extends the Graph view with a deterministic layout, role-coloured
  edges and a hover readout, three of its listed gaps.
- Judges: R4, L3 (layer 0 of a PNG with 4096 chunks is a column 4096 nodes
  tall unless folded; layered layout on 500+ nodes takes seconds), F5
  (`FieldGraph` is exactly what is needed; add `cytoscape-elk` or topo-sort
  layers). Still behind `?graph`.

##### `decides-matrix`: Adjacency matrix of what decides what, in file order (10)
- Looks like: a square grid with fields in file order on both axes; a filled
  cell at (row A, column B) means A decided something about B, coloured by
  role. Fields decided by their predecessor hug the diagonal; a header count
  that sizes a table far away is a lone cell far off it; an offset table is a
  vertical stripe. Blocks along the diagonal show self-contained records. A
  type-level version uses `DiagramBox` x `DiagramBox`.
- Shows: the density and reach of dependencies across the file, and which
  structures are self-contained, at a scale where node-link is a hairball.
- Formats: midi, pe, zip, sqlite, parquet, hdf5.
- Interaction: wheel zooms into a corner, drag pans; a crosshair hover
  highlights the row and column and names both fields and the role; click a
  cell goes to the decided field with the deciding field marked; ordering
  toggle: address, tree. Zoomed out, cells aggregate into binned density blocks.
- Labels: row and column labels drawn only when a row is at least one text
  line tall at the current zoom; otherwise the crosshair names them; nothing
  inside the grid.
- Nesting: a cell's hover card holds the expression tree; diagonal blocks
  expand into the struct layout.
- Overlaps: same `FieldGraph` as the Graph view; the scalable alternative to
  it and to the reads DAG.
- Judges: R2 (most programmers cannot read an adjacency matrix without
  training), L4, F4. Overruled: skip `reorder.js` (unmaintained, pulls
  science.js); address and tree orders need no library.

#### Stream scale

##### `stride-grid`: Stride raster and page allocation grid (11)
- Looks like: the bytes of a range drawn as pixels, a chosen number of bytes
  per row: grey by value or coloured by class, as a raw-image import tool
  shows them. At the right stride a record table snaps into vertical stripes
  (CD sector sync bytes line up as a column; a misaligned record breaks it)
  and a raw bitmap snaps into a picture. Beside it a small line of
  self-similarity against lag whose peaks suggest the stride. When the stride
  is a format's page or sector size the same grid becomes an allocation map:
  each cell a page coloured by owner (interior, leaf, overflow, free,
  directory, stream A), with a chain overlay drawing thin arrows through one
  stream's cells in chain order.
- Shows: the record size of unknown data, whether a template's record size is
  right, and which pages belong to what and how a chain threads through them.
- Formats: cdrom (2352-byte sectors), sqlite pages by kind, doc/xls/ppt CFB
  sectors, iso sectors and extents, tar 512-byte blocks, utmp fixed records,
  segy traces, w4v 392-byte blocks, hdf5 chunks, parquet pages.
- Interaction: wheel zooms and drag pans the raster; a stride control and the
  up and down arrow keys change bytes per row; clicking a peak in the
  periodicity line sets the stride; hover a pixel or page: offset, value,
  owner and the next page in its chain; click goes to the byte or page; click
  an owner in the legend lights its cells and draws its chain (one owner at a
  time, or 10k hops become spaghetti); page numbers appear once cells are big
  enough.
- Labels: no text on the raster; axis ticks from d3-axis; cell numbers only
  above a minimum cell size; owners in a legend list.
- Nesting: a page cell opens the struct layout of that page; the grid sits
  inside the Logical tab's sqlite or iso row.
- Overlaps: complements the B-tree address strip with the page-level picture;
  the minimap grid is this at one fixed stride.
- Judges: R4, L4 (draw a 100 MB file at stride 2352 in viewport tiles), F3
  overall: the raw raster from `read` at a stride is a 5 (canvas, no
  library); the allocation-map half needs per-format page-owner adapters.

##### `lz77-arcs`: LZ77 back-reference arcs over the output (11)
- Looks like: the decompressed output as a line of byte cells or characters.
  Each match is an arc from where its copy lands back to where it was copied
  from, height by distance, matched runs shaded and their source outlined;
  literals plain. Under the line, a compression-ratio sparkline (bits in per
  byte out). A second row above shows the compressed run with ribbons from
  each step's input bits to its output bytes: literals thin and straight,
  matches fanning wide. Arcs whose source is off screen end in an edge marker
  naming the distance.
- Shows: the repetition structure of the data the way the compressor saw it,
  and where in the compressed bits each output byte came from.
- Formats: deflate in gzip, zip, png IDAT, pdf FlateDecode streams, docx and
  xlsx members; lz4 where traced.
- Interaction: wheel zooms along the output around the pointer, drag pans;
  hover an output byte (not the arc strokes): the step, its length, distance
  and the codes that carried it with their bit cost, lit in the hex view
  (`unpackedlink.ts` maps both ways); click goes to the compressed bits of
  that step; brush either bar to see the matching range on the other.
- Labels: arcs and ribbons carry no labels; characters show only when cells
  are wide enough, else a density strip; axis offsets from d3-axis.
- Nesting: the step-through decoder is its zoomed-in form; a matched run's
  hover card holds the bit-pack ribbon for its code.
- Overlaps: extends the deflate trace ribbon in `report/streams.ts` (a fixed
  window near the first copy, no arcs) with scrolling, zoom and arcs.
- Judges: R4, L3 (at a zoom where characters show, most matches reach up to
  32 KiB back off screen, so the edge marker and hover-from-the-byte rule are
  required), F4 (`MapStep` has kind, len, dist, in and out; `mapOutAny` steps
  by `out_end` per window as the trace ribbon already does).

##### `pipeline-sankey`: Decode pipeline funnel with inline in/out ratio glyph (10)
- Looks like: left to right, the stored bytes of a chunk, page, tile or entry
  flow through each filter or codec step (shuffle, deflate, Fletcher32,
  dictionary, RLE and bit-pack, differencing) as bars whose heights are bytes
  in and out on one shared log scale, widening where a stream inflates;
  skipped steps drawn as dashed pass-throughs; at the right the unpacked
  bytes split into what the template reads them as (pixels, rows, tensors)
  with a sparkline of the first values. The inline glyph is a two-bar
  trapezoid (compressed to unpacked) the height of one text line, on the same
  scale down a list so ratios compare by eye; several thin bars converging
  into one for a stream joined from runs. A file-wide Sankey from the whole
  file to field kinds is the only part that needs d3-sankey.
- Shows: what was done to get from the stored bytes to the numbers, how much
  each stage grew or shrank the data, and which stages were skipped.
- Formats: hdf5 chunk filters, parquet pages, fits tiled compression, gwf
  vectors, grib section 7, seismic Steim frames, zip entries, png IDAT to
  scanlines to pixels, bufr.
- Interaction: hover a bar: bytes in, bytes out, ratio; hover a step: the
  note and the reason it stopped; click the left bar goes to the stored run;
  click a right bar opens that unpacked space as a tab; hover the glyph: both
  sizes and ratio; click the glyph opens the pipeline.
- Labels: step names under their bars in fixed slots; bars shorter than the
  text height are hover-only; the glyph carries no labels.
- Nesting: the ratio glyph nests in headings, Logical rows, treemap tiles and
  the space-nesting diagram; a stage opens the step-through decoder.
- Overlaps: draws the data `steplist.ts` writes as text; the stream stages
  bars become the two-stage case.
- Judges: R3 (two to five stages carry little more than the step list's
  numbers; the inline ratio glyph, comparable down a list, is the valuable
  part), L3 (log-scaled widths break the reading that widths add up, so draw
  bars with the scale stated rather than a Sankey), F4 (`ChunkStep`,
  `PageStep`, `Unpacking` exist; the glyph is a 5). Overruled: no d3-sankey
  for a linear chain.

##### `step-decoder-timeline`: Step-through decoder with scrubbable timeline (10)
- Looks like: three linked strips: the compressed bits with a cursor; the
  current code being read (its bits, the table row it matched, the symbol, as
  the bit-pack ribbon); the output buffer growing byte by byte, a match step
  drawing a copy arrow back into earlier output. Play, step back, step
  forward and speed controls. Above them a timeline: one lane per block or
  channel, each step a bar whose length is bits spent, restart markers as
  ticks, a cumulative output line; a brush window picks the stretch the
  strips show. For JPEG the lanes are Y, Cb, Cr with MCU bars.
- Shows: how a decoder consumes bits one code at a time, where each output
  byte came from, and where in the stream the bits were spent.
- Formats: deflate in gzip, zip, png and `compressed`; jpeg scan MCUs; pdf
  FlateDecode; mp3 frames; seismic miniSEED frames; grib; bufr.
- Interaction: space plays and pauses, arrow keys step (scoped to the panel's
  focus, or they collide with hex navigation), wheel over the timeline scrubs
  and zooms it, brush selects the window; hover any output byte shows the
  step that produced it and lights its bits in hex; hover a timeline bar: the
  step and its cost; click a step jumps there in hex.
- Labels: only the current step is labelled in full; neighbouring steps are
  unlabelled ticks with hover detail; timeline bars show no text; lanes have
  one label in a fixed left gutter.
- Nesting: contains the bit-pack ribbon and the LZ77 arcs' local window; the
  Huffman tree lights the matched path alongside.
- Overlaps: animates what the trace ribbon, bit-order ribbon and JPEG block
  code strip draw statically; the timeline is the scrollable form of the MCU
  cost map.
- Judges: R3 (explainer-video value for learners; the bits-spent timeline
  lane is the more reusable insight), L4, F3 (stepping works now one step at
  a time; the JPEG timeline is ready from `JpegScan.mcu_bits`; a whole deflate
  stream timeline is millions of wasm calls and wants a bulk step-list core
  call).

##### `order-vs-address-ribbon`: Stream order against file order (9)
- Looks like: top row, the parts of a joined stream end to end in stream
  order; bottom row, the file to scale with each run where it sits. Bands
  join each block's two copies, so a permuted or fragmented layout reads as a
  tangle and a sequential one as parallel lines; widening bands show packed
  BGZF members inflating; a record cut across member boundaries is one span
  on top and several pieces below. Seam marks on the hex view where a field
  crosses a part boundary. A BGZF virtual offset draws as a 64-bit field
  diagram (block offset << 16 | byte within block) for the hovered byte.
- Shows: how one logical stream is scattered through the file, and which part
  and virtual offset a given byte belongs to.
- Formats: pdb (MSF block lists), bam (BGZF), png (IDAT joined), godot RSCC,
  hdf4 linked blocks, cab folders, adios BP5 in a zip, parquet pages per
  column chunk, fits binary table heap.
- Interaction: wheel zooms the address row only, drag pans; hover a band:
  part index, both ranges, packed and unpacked sizes; click either copy goes
  to its bytes (file hex or the stream's tab); a toggle sorts the top row by
  address to show what is in order. Contiguous runs merge into one band
  before drawing; hover is on the stream row's blocks, not the bands.
- Labels: block labels only when a block is wider than its label; axis ticks
  from d3-axis; the virtual offset diagram is a fixed two-cell figure.
- Nesting: the pipeline ratio glyph's converging form is its thumbnail; a
  band opens the struct layout of that block.
- Overlaps: generalises the directory ribbon from pointer lists to stitched
  spaces; the B-tree address strip is the one-row form.
- Judges: R4 (for PDB, BAM and joined IDAT this is the key fact), L3, F2
  (`partOf`/`partAt` answer one field or byte at a time with no parts list;
  needs the core DTO in 3.12; the FITS and Parquet gather form could use
  directory targets today).

#### File scale

##### `position-icicle`: Position-true icicle, a flame chart of the field tree (13)
- Looks like: stacked horizontal bands, one per tree depth. The x axis is file
  offset: each field is a bar exactly as wide as its bytes, under its parent.
  Depth 0 the file, depth 1 the headings (PE sections, SQLite pages, PNG
  chunks), deeper rows their fields, gaps and the unread tail hatched,
  problems as a red edge. Unlike the treemap it keeps order and position, so
  it doubles as a map. An ordered-nesting variant draws boxes as
  [size | type] header strips with children inside, the way the MP4 and RIFF
  specs draw containers.
- Shows: what the file is made of, in file order, with containment visible as
  depth and size visible as width.
- Formats: mp4, matroska, png, zip, elf, pe, sqlite, hdf5, wav.
- Interaction: wheel zooms around the pointer on the offset axis, bars widen
  and deeper levels are fetched lazily once wide enough (the LOD table);
  drag pans; click a bar makes it full width with a crumb trail out;
  double-click goes to the bytes; hover: name, type, value, offset, size.
- Labels: a label is drawn only inside a bar whose pixel width exceeds the
  measured text width, else ellipsis down to a minimum, then omitted; one
  label per bar, clipped to the bar. Sub-pixel siblings fold into an "N small
  fields" bar.
- Nesting: draws the file map's segments at depth 1, so it is the file map
  zoomed; a bar carries the tag-tile glyph; it is the middle levels of the
  file-to-bit view.
- Overlaps: extends `treemap.ts`/`treemapdata.ts` structure mode with position
  kept; the minimap's layout strip is its depth-1 row.
- Judges: R4, L5 (d3 partition plus fit-or-drop per bar, zoom on one axis and
  lazy deeper levels scale to 100 MB), F4 (partition is installed and
  `treemappanel.ts` has modes, a trail and pick/jump callbacks; lazy deeper
  fetch on zoom is the hard part). Overruled: drop the sunburst variant,
  since radial labels and lost order defeat the point.

##### `file-vs-memory-bands`: File offsets against memory addresses, two-space bands (13)
- Looks like: two tall columns, file offsets on the left and virtual addresses
  on the right, as the ELF spec draws linking view against execution view.
  Each section or segment is a box in each column joined by a band; bands
  that fan out show alignment slack; .bss widens on the memory side out of
  nothing; overlays and debug sections have no partner. Permissions (r, w, x)
  mark each band; a broken axis skips huge empty address ranges. Symbols as
  ticks inside .text. The same two-axis form serves file against unpacked
  space for a compressed run, and the Logical outline against the file for an
  archive (where each entry sits physically), which the inventory lists as a
  missing figure.
- Shows: how the loader maps what is stored to where it runs, which sections
  exist only in memory or only in the file, and where alignment leaves slack.
- Formats: elf (program headers against sections), pe (PointerToRawData
  against VirtualAddress), macho segments, ne and le segments, coff, wasm data
  segments.
- Interaction: linked zoom by default (wheel zooms both axes together);
  unlinked zoom as an option so a 16-byte section opens up on one side while
  the other stays whole; hover a band: name, both ranges, sizes, flags; click
  the file side goes to the bytes; click the memory side goes to the section
  in the Logical tab.
- Labels: section names sit outside the columns in fixed-height rows assigned
  by the lane helper, with leader lines; names that still do not fit are
  hover-only.
- Nesting: each band's hover card holds a file-map locator; a band opens the
  struct layout of its header entry.
- Overlaps: draws the `ElfContents` the Logical outline lists; no existing
  piece draws two address spaces.
- Judges: R5 (central to understanding executables and invisible in a
  listing), L4 (unlinked zoom shears the bands into crossings, so linked is
  the default), F4 (`ElfContents.sections` has offset, address and size;
  program headers are not in `ElfContents`, so the true segment view reads
  `templateChildren`; PE needs field-name lookups). Overruled: `labella.js`
  replaced by the lane helper.

##### `locator-family`: Locator glyphs: live file map, zoom stack, breadcrumb bars, scrollbar track (13)
- Looks like: the existing 96px file map made live: hover names the segment
  under the pointer, click goes to it, a second ring under it shows the
  unpacked space's strip for a field inside a stream, and a 32px Hilbert
  thumbnail is an alternative shape. A zoom stack: the strip, then under it
  the lit part expanded to full width with its own children, joined by lens
  lines, one level per step of the current path down to the field, as an
  ideogram with a lens. Breadcrumb bars: each crumb in `trail.ts` carries a
  24x4 bar showing where that part sits in its parent. A scrollbar annotation
  track beside the hex, listing and text scrollbars, like an IDE's overview
  ruler: ticks for problems (red), wrong values (amber), search and string
  hits, the selection, heading boundaries; dense clusters merge into a taller
  block.
- Shows: where in the whole file (and in the stream, and in the parent) the
  thing under the reader's eye is, and where the marks worth visiting are.
- Formats: all; strongest where the interesting part is far from the start:
  zip, pdf, sqlite, elf, tar.
- Interaction: hover a segment at any level: name, offset, percent; click goes
  there (the stack rebuilds); wheel over a stack level steps to the previous
  or next sibling; hover a track cluster lists what is there ("3 problems, 1
  search hit at 0x4A00"), click scrolls there; category toggles from a small
  legend. Click on a 2px segment resolves to the nearest segment.
- Labels: segments are unlabelled inline; in the zoom stack one label per
  segment only when the measured text fits, otherwise the lit segment's name
  is a caption beside the strip; the crumb's own text is its label; the track
  has no text and merges ticks closer than 2px.
- Nesting: the universal nested piece: inside cards, tooltips, diagram boxes,
  tiles and the component diagram; the zoom stack is the inline form of the
  file-to-bit view.
- Overlaps: extends `filemap.ts` (inert, top-level only) and merges
  `posbar.ts` into it; the track extends the minimap's viewport outline to
  every scrollable view.
- Judges: R5 (spares the reader holding state; nests everywhere; the track
  flags deviations in every view), L4, F4 (clickable `filemap.ts` and crumb
  bars are 5s; the scrollbar track across three views via `rowheights.ts`,
  and a file-wide problem and hit list, are the expensive parts).

##### `tree-table-gantt` (critic): Outline with position bars (13)
- Looks like: the listing as it is, with one more column: a bar on a shared
  file-offset axis per row, at the row's offset and width, in the row's kind
  colour, as a Gantt chart draws tasks against time. The axis header above
  the column zooms with the wheel and pans with a drag, and a faint lens on
  the axis shows which stretch of the file the visible rows cover. A child
  row's bar sits inside its parent's; a gap row's bar is hatched; a field read
  by address draws a thin line from where it was named to where it sits.
- Shows: the icicle and the listing together: every field's position and size
  beside its name, one row each, so labels cannot collide and the list
  scrolls to 50k records.
- Formats: all templated formats; most useful where fields are placed out of
  order: zip, pdf, elf, tiff, hdf5, wav with data before fmt.
- Interaction: wheel over the axis zooms it around the pointer, drag pans it;
  double-click a bar fits the axis to that field; hover a bar: offset, size,
  share; click selects the row as the row itself does; the hex cursor's
  position is a vertical line down the column.
- Labels: none in the column; names are the row's own; axis ticks from
  d3-axis.
- Nesting: the column is a one-row-per-field form of the position icicle; the
  bars carry the problem red edge.
- Overlaps: the listing heading's share text and file map say this per
  heading; the position icicle says it without names.
- Judges (critic's scores): R4, L5 (the most legible form of position
  available: each label already has its own row), F4 (items already carry
  `offset_bits` and `size_bits`; the work is a column in `listingdraw.ts`, a
  scale in `listingreport.ts` and the axis header; rows that are virtualised
  already stay virtualised).

##### `pointer-arcs-axis`: Pointer arcs over the file's address axis (12)
- Looks like: a horizontal bar of the whole file with the same segments as
  the file map, zoomable. Semicircular arcs from each pointer, length or count
  field to the bytes it governs: forward references above the axis, backward
  below, height by distance, colour by role, as the Shape of Song draws
  repeats. A PDF shows a fan of arcs from the xref at the end back to every
  object; a ZIP's central directory reaches back to each local header; a
  PNG's lengths are short brackets. Gaps no arc reaches are hatched. The
  selected field's arcs are bright, the rest faint. A scatter mode (the
  judges' pointer scatter) draws each edge as a dot at (source offset, target
  offset): forward above the diagonal, backward below, a directory as a
  column of dots; dots on canvas scale to a million edges.
- Shows: where each directory, offset table and length points, how far, in
  which direction, and which bytes nothing claims.
- Formats: pdf, zip, elf (e_shoff, sh_offset), pe (e_lfanew, data
  directories), tiff IFD offsets, gguf tensor offsets, font table directory,
  wad, sqlite page pointers.
- Interaction: wheel zooms the address axis around the pointer, drag pans,
  arcs re-project; hover picks by endpoint or nearest arc, not by stroke, and
  shows the from-field, its value, `Relation.written` and `substituted`, the
  target offset; hover a field shows only its arcs; click either end goes
  there; role filter chips.
- Labels: arcs carry no text; the axis shows only file-map segment names
  thinned by d3 ticks; all values in the hover card.
- Nesting: nests the file map as its axis; an arc's hover card holds the
  expression tree; repeats as a thin strip under headings showing only that
  field's arcs.
- Overlaps: whole-file companion to `hexlinks.ts` arrows (one field, on screen
  only) and the directory ribbon (report, capped entries); draws the
  `FieldGraph` edges the Graph view hides.
- Judges: R5 (directory structure is the least visible thing in a hex dump),
  L3 (thousands of 1px arcs give no hit target; `graph(path, limit)` reports
  `omitted`, so a whole-file view bins arcs by bucket pair), F4 (`GraphEdge`
  roles and `GraphNode.offset_bits` place arcs directly). A numbered
  reading-order variant needs a core walk trace.

##### `entropy-histogram-strip`: Entropy strip with byte-value histogram (12)
- Looks like: a wide line chart: x is file offset, y is Shannon entropy per
  bucket (0 to 8 bits per byte), a dashed line at the block's `entropy_max`,
  a thin byte-class band under the line and the file-map ribbon of parts
  under that, so a jump in entropy lines up with its section. Under it, a
  256-bar histogram of byte values for the visible range, log y, printable
  ASCII shaded, 00 and FF picked out. A bit-position variant (the judges' bit
  usage) draws one bar per bit position of a numeric run or a flags field
  across records: how often each bit is set, so constant high zeros and a
  flag never used show; it replaces the set/clear-bits treemap. The inline
  forms are a 120px entropy sparkline beside a heading, a 256x16 grey
  sparkbar beside a block's facts, and the judges' byte-ledger bar (payload,
  machinery, unread, gap as one stacked bar beside headings and in the brief).
- Shows: where the file is compressed, encrypted, textual or empty, and the
  shape of the byte distribution in the range under the brush.
- Formats: pe packed sections, elf, zip, gzip, png IDAT, jpeg scan, firmware,
  ogg, `compressed`.
- Interaction: brush on the x axis zooms and refetches buckets for that
  range, double-click resets; hover: offset, entropy, class, covering field,
  with a vertical cursor shared with the ribbon; click jumps the hex view;
  hover a histogram bar: value, glyph, count, percent; click a bar hands that
  byte to the search bar; log/linear toggle.
- Labels: axis ticks from uPlot; section names in the ribbon only where a
  segment is wide enough; per-point values in the cursor legend; the sparkbar
  has no labels.
- Nesting: carries the file map as its bottom ribbon; the sparkline and
  sparkbar nest in headings, block facts and treemap tiles.
- Overlaps: replaces the byte-value treemap (a weak form for a histogram) and
  the set/clear bits treemap; extends the minimap's facts table.
- Judges: R4, L5 (textbook dataviz with uPlot), F3 (the histogram half is
  ready from `OverviewState.histogram` and `FocusState`; the per-bucket
  entropy line lacks an array but can be bridged web-side from `read` chunks
  or per-range `overviewFocusStep`, so core is optional).

##### `file-to-bit-zoom`: Semantic zoom from file to bit, one canvas (11)
- Looks like: one canvas. Fully out: the file as a bar of top-level parts with
  the byte-class strip under it. Zoom in: parts split into child fields as
  nested rectangles in file order; further: each field shows its hex bytes;
  further still: each byte splits into 8 bit cells with 0/1 and the bits a
  sub-byte field owns outlined. Labels and detail appear at the zoom where
  they fit. A breadcrumb shows the path of the field under the pointer; a
  fixed scale stack beside it (file > part > structure > field > byte > bits)
  shows where the current zoom sits.
- Shows: one continuous route from the whole file down to the bit under the
  cursor, with structure visible at every level.
- Formats: all templated formats; the bit level pays off for deflate headers,
  mp3, flac, pcap, bufr.
- Interaction: wheel zooms around the pointer on x only so file order stays
  left to right, drag pans; hover: path, value, bit range; click goes to the
  field; double-click zooms to fit a field; the scale stack is clickable.
- Labels: drawn only when the measured width fits the rectangle at the
  current scale, else ellipsis then dropped; the name is always in the hover
  card and breadcrumb; axis ticks from d3.
- Nesting: contains the position icicle (its middle levels), the byte-class
  strip and the bit ruler (its deepest level); the scale stack is the locator
  family's zoom-stack glyph.
- Overlaps: extends `report/zoommap.ts` (three rows, one field level, "zoom
  in" past 600 spans) to full depth and to the bit level, mounted as a view.
- Judges: R4 (a good mental model of scale; risks being a navigation toy that
  repeats the icicle and bit ruler), L5 (proven by `zoommap.ts`), F2 (a new
  main view, a multi-level LOD canvas renderer, and `zoommap.ts` unhooked from
  `ReportCtx`: a week, not a day).

##### `dump-reconstruction` (critic): Dump lines against the bytes they wrote (11)
- Looks like: two rows joined by bands, as the ribbon draws. Top: the lines of
  the hex dump text in file order, each a box as wide as its line, the
  address column tinted, autoskip lines (`*`) marked. Bottom: the
  reconstructed binary to scale. A band runs from each line to the bytes it
  wrote; an autoskip line's band widens to the run it stands for; a hole (an
  address the dump never wrote) is hatched on the bottom row; two lines that
  wrote different digits to the same address are a red band pair with both
  values in the hover card; skipped lines (a prompt, a heading, a shell
  command) are grey boxes with no band. A caption gives the tool and tier the
  core recognised (`xxd`, `od`, `certutil`, PowerShell) and what was assumed.
- Shows: which dump lines made which bytes, where the dump is silent or
  contradicts itself, and what the reader of the dump assumed about its
  layout.
- Formats: the `hexdump` template, for text captures from xxd, od, certutil,
  PowerShell Format-Hex, XTree Gold and shell sessions (the
  `../qubero-samples/hexdump` folder).
- Interaction: hover a line or a band: the line's text, the address it
  claimed, the bytes it wrote; click the top row goes to the line in the text
  view; click the bottom row goes to the byte in the reconstructed space's
  hex tab; wheel zooms the bottom row for long dumps.
- Labels: no text on the rows beyond the address tint; holes and conflicts
  are hatched and red, named in the hover card; the caption is one line.
- Nesting: carries the file-map locator of the reconstructed space; a band
  opens the text line beside its bytes.
- Overlaps: nothing draws `DumpScan` today; the text view shows the dump and
  the hex tab the result, with no line between them.
- Judges (critic's scores): R3 (a niche template, but the one time a reader
  opens a dump they need exactly this), L4 (two rows and bands; past a few
  thousand lines, merge contiguous lines into one band), F4 (`DumpScan`
  carries `extents`, `holes`, `conflicts`, `skipped_lines`, `bytes_per_line`
  and `names`; the ribbon exists; a `Doc` method wrapping the core scan is
  the only wiring).

##### `digraph-plot`: Byte-pair dot plot (10)
- Looks like: a square, as cantor.dust and Veles draw it: a dot at (x, y) for
  each pair of consecutive bytes x then y, brightness by log count. Text forms
  a bright block in the 20-7E square, x86 code a characteristic scatter with
  diagonals, compressed data a uniform haze, UTF-16 a line along one edge,
  float arrays a lattice. Axis ticks every 0x20 with the control, ASCII and
  high ranges captioned outside the square. A range picker (the file-map
  strip) scrubs which part of the file feeds the plot.
- Shows: what kind of data a block is (code, text, integers, compressed)
  before a template exists, and whether an embedded stream differs from its
  host.
- Formats: pe, elf, firmware, unknown blobs, gguf tensor data, fits, hdf5,
  zip members.
- Interaction: wheel zooms into a sub-square, drag pans; hover a cell: "after
  0x48 H comes 0x65 e, 412 times"; click a cell hands the pair to search;
  brush a rectangle to mark every matching pair on the file map and in hex;
  the range picker scrubs the source range.
- Labels: axis ticks at 00/40/80/C0/FF filtered by d3-axis per zoom level;
  quadrant captions outside the square; cell values only in the hover card.
- Nesting: sits beside the class grid and histogram in the block close-up;
  the range picker is the file-map locator.
- Overlaps: a new channel in the minimap's block close-up.
- Judges: R2 (niche; takes training to read), L4, F4 (65,536 pair counts over
  `read` in a worker is trivial for MB ranges; a 100 MB file needs the core
  scan or a sampled range stated on screen). Overruled: no Observable Plot.

##### `hilbert-map`: Hilbert-curve byte map (9)
- Looks like: a square where the file runs along a Hilbert curve, so bytes
  near each other in the file stay near each other on the square and a
  section is a compact blob rather than a stripe. Cells coloured by byte
  class or by entropy; template parts as thin outlines; the part under the
  cursor and the main view's range glow. A toggle swaps Hilbert for Z-order
  and plain scan lines so the reader sees why the curve helps. A 32px version
  is a thumbnail locator.
- Shows: where the zeros, text, data and compressed runs are, as regions
  rather than stripes, with the template's parts outlined over them.
- Formats: firmware, elf, pe, zip, iso, model, gguf, pdb.
- Interaction: wheel zooms around the pointer (re-scanning the visible range
  at finer buckets through `overviewFocusStep`), drag pans; hover: offset
  range, class and the covering field, with a trace of the curve path through
  the hovered region so order can be followed; click goes to the bytes;
  second click opens the block close-up; layout toggle.
- Labels: cells carry no text; one part name per outlined region placed at
  its largest inscribed cell and dropped when the region is under about 40px
  across; everything else in the hover card.
- Nesting: the 32px thumbnail is a member of the locator family; the square
  sits in the report's whole-file section.
- Overlaps: a second ordering of the minimap's cell grid, adding zoom,
  outlines and a pointer tooltip.
- Judges: R2 (striking, but the reader loses position and order; the linear
  minimap and entropy strip answer the same question more plainly), L3, F4
  (`OverviewState.classes` reorders by a 20-line d2xy; no dependency).

#### Format scale

##### `space-nesting-components`: Address-space nesting diagram with lineage breadcrumb (12)
- Looks like: each byte space the file opens is a component box, as a UML
  component diagram or a set of matryoshka boxes to scale: the file (space 0);
  inside it, each compressed run as a box that expands into its own space box
  with its own file-map strip; recursively (a zip of a pickle of a tensor; PNG
  IDAT joined from 12 runs then inflated then unfiltered). Lines between
  boxes are labelled with how one became the other ("joined from 12 IDAT",
  "inflate", "format inside"); each box carries a tiny strip showing which
  part of its parent it came from and the pipeline ratio glyph. The inline
  form is a lineage breadcrumb (file > IDAT joined > zlib > scanlines) for
  tab titles and the inspector heading.
- Shows: how many layers of container, codec and format sit between the file
  and the bytes a tab shows, and where each layer came from.
- Formats: png, zip, docx/xlsx/pptx, deb (ar then tar.gz), rpm (cpio
  payload), torch (zip with pickles), zarr ZipStore, pdf (FlateDecode, object
  streams), jpeg (EXIF, ICC, thumbnail).
- Interaction: hover a box: format, size and the parent range; click a box
  opens that space's tab; click a line marks the source runs in the parent's
  hex view; wheel zoom and drag pan at panel size; the breadcrumb's crumbs
  switch tabs. A zip with 5,000 members groups siblings into one box marked
  xN that opens a list.
- Labels: box titles are ELK node labels placed inside at the top; edge
  labels are ELK edge labels with reserved space; nesting is shallow (2 to 5
  boxes).
- Nesting: contains file-map locators and ratio glyphs; a box opens the
  pipeline funnel; the breadcrumb is a member of the locator family.
- Overlaps: new picture; today only the tab titles and the Unpacking numbers
  say anything about spaces.
- Judges: R5 (shown nowhere today; the breadcrumb is cheap and useful
  everywhere), L4, F3 (`space_origin` is a wasm export at
  `crates/wasm/src/lib.rs:812` with no `Doc` method, a web-side addition;
  enumerating every space a file can open needs a walk for `Span.opens` and
  `decoded`; the breadcrumb alone is a 5).

##### `railroad-grammar`: Railroad diagram of the format's grammar with this file's counts (11)
- Looks like: a track left to right through rounded terminals (markers, chunk
  types, fields) and rectangular non-terminals (types), as a syntax diagram.
  Optional parts are bypass loops, repeats are return loops marked with this
  file's count ("IDAT x 47"), alternatives branch in parallel; branches the
  file never took are greyed. Clicking a non-terminal opens its own railroad
  beneath. A state-chart variant draws record types as states with
  transitions thickened by how often this file took them ("IDAT -> IDAT x
  41") and allowed-but-unseen transitions thin and grey.
- Shows: what the format allows and what this file did with that freedom:
  which loops, how many times, which branches.
- Formats: png, jpeg markers, midi events, wav and avi RIFF, mp4, ogg, sqlite
  page kinds, deflate block kinds.
- Interaction: wheel zoom and drag pan on the existing board; hover a box:
  count and bytes; click a counter lists this file's instances and goes to
  the first (`goTarget` from `diagramcounts.ts`); click a non-terminal opens
  its railroad; a loop expands to show each instance in order.
- Labels: `railroad-diagrams` sizes each box from its text and lays the track
  out by construction; counters are part of the box text; state-chart
  transition labels are ELK edge labels. The library measures by a fixed
  per-character width, so boxes use a monospace font or a patched measure.
- Nesting: a terminal shows the tag-tile glyph; a non-terminal's railroad
  nests under its parent's track.
- Overlaps: a third reading of the `TemplateDiagram` the Arrows and Strips
  modes draw, with the census badges as loop counters.
- Judges: R4 (format freedom against what this file did is symbolic
  understanding of the format), L4, F3 (`TemplateDiagram` gives seq,
  instances and switch boxes and condition and case edges; optional and
  until semantics must be inferred; loop-exit labels need core; observed
  transitions need a full child walk).

##### `schema-class-er`: UML class and ER diagrams of the types the file describes, with multiplicities (11)
- Looks like: one class box per type the file declares about itself: a ROOT
  file's StreamerInfo classes with members as typed attributes, base classes
  as hollow-triangle arrows, counted arrays with an edge to the member that
  counts them; HDF5 compound datatypes; Parquet and Avro schemas with nested
  groups. For SQLite, an ER diagram: each table from `sqlite_master` as an
  entity with its columns and declared types, indexes as small boxes joined
  with crow's feet, foreign keys as relationship lines, a root-page badge on
  each. As an extension of the Diagram view, each composition line gains a
  UML multiplicity at its far end ("1", "0..*") plus this file's count ("x 37
  here"), and switch cases become generalisation triangles.
- Shows: the data model the file carries about itself, separately from the
  template's parsing structure, and how many of each the file holds.
- Formats: root (StreamerInfo), hdf5 compound types, parquet schema, avro,
  sqlite schema, adios FFS formats, pickle NumPy dtypes, arrow.
- Interaction: pan and zoom on the Diagram board; hover a member: type and
  on-disk size; click a class goes to its StreamerInfo record or CREATE row;
  click a root-page badge goes to that page; click a table opens its rows in
  the Table view; click a multiplicity goes to the first instance, click again
  steps to the next. Scoped to classes reachable from the selected one, with a
  search box, so hundreds of ROOT classes do not become a hairball.
- Labels: rows are HTML table rows ELK sizes; multiplicities are ELK port
  labels placed outside so ELK reserves room; crow's-foot ends are
  port-attached markers; long C++ template names elide with a hover card.
- Nesting: a member row opens the struct layout; the ER's root-page badge is a
  file-map locator.
- Overlaps: extends the Diagram view's renderer with a second data source and
  UML multiplicities.
- Judges: R4, L4, F3 (`RootMember` has `base` but no `counted_by`, which is on
  `RootLeaf`, so counted-array edges need a lookup; `sqlitesql.ts` and
  `records.ts` exist for the ER form; three data adapters plus the Arrows-mode
  multiplicity extension).

##### `shape-parallel-sets`: Parallel sets of how fields were placed, sized and typed (8)
- Looks like: three or four vertical columns of stacked categories: how a
  field was placed (follows, element, pointer, chain, gathered, stream,
  stitched), how it was sized (type, count, expression, terminator,
  remaining), its kind (u32, bytes, struct, str), and payload against
  machinery. Bands flow between columns with width proportional to bits
  covered (or field count, by toggle). One glance says "most of this file is
  bytes placed by a pointer and sized by an expression". A fixed glyph
  vocabulary (one small icon per placed and sized word) on listing rows and
  diagram boxes; a radial fingerprint of the Profile categories for comparing
  two formats.
- Shows: the format's habits: where it writes sizes, whether it points
  forward or back, how much is machinery, how two formats differ.
- Formats: zip (from_end, pointers), png (everything follows), gguf (forward
  pointers), elf (backward and forward), parquet, hdf5, pdf.
- Interaction: drag a column to reorder; hover a band: bits and field count;
  click a band lists those fields; click a category filters the listing to
  it; hover a glyph names the placement word.
- Labels: category labels beside their column's stacked block with
  d3-sankey's node padding; categories thinner than one text line group under
  "other" with a hover list; glyphs have no text and need a hover card each.
- Nesting: the glyph nests in rows and diagram boxes; the fingerprint in the
  report brief and the template picker.
- Overlaps: visual form of the report's profile table and ledger grouping.
- Judges: R2 (a meta-view of template habits interests format authors more
  than a programmer reading a file; a learn-once icon vocabulary works against
  plain reading), L3 (the radial fingerprint is a radar chart, a weak
  comparison form), F3 (`shape` per field is one wasm call each, so bucket via
  the report ledger instead).

##### `offset-frames-ruler`: Nested rulers for offsets counted from somewhere else (10)
- Looks like: the file ruler on top; beneath it a second ruler zeroed at the
  frame the offset counts from (the TIFF header inside a JPEG APP1, the HDF5
  user block, the PE RVA base, the unpacked space's zero). The stored offset
  is a measured arrow from its frame's zero to the target, labelled with both
  numbers ("+0x8 in the TIFF = 0x1E in the file"). A pointer that would land
  on an open ancestor (a ring) is drawn crossed out with the reason on hover.
  A tiny origin marker on the file-map locator says where this field's
  offsets count from.
- Shows: which zero an offset counts from and where it lands in file
  coordinates.
- Formats: jpeg EXIF TIFF, tiff sub-IFDs, hdf5 user block, mat (MATLAB 7.3 is
  hdf5 with a header), pe RVA against file offset, wad, root RNTuple anchor,
  icc inside png and jpeg.
- Interaction: hover the arrow: stored value, frame base and file address;
  click the target end goes there; click the frame tick goes to the frame's
  start; wheel zooms the ruler pair together around the pointer; opens
  zoomed to fit frame and target, with the file ruler as context.
- Labels: tick labels from d3-axis thin themselves; the one free label sits
  on the arrow's own lane; frame names sit at their zero tick in a fixed slot.
- Nesting: the frame ticks repeat on `filemap.ts` strips; the figure sits
  inside the pointer-arcs hover card.
- Overlaps: adds a coordinate frame to the hex origin arrows and pointer arcs,
  which only know file bits.
- Judges: R4 (a classic source of confusion; nothing shows it today), L4, F2
  (`TemplateNode.read_at` and `Origin.target_bits` exist, but the `At` anchor
  kind and base are not exported; small figure once the DTO exists). Its
  scale is structure (one field against the frame it counts from); it is
  placed last here because it waits on core.

## 5. Recommended first batch

Fourteen candidates, none needing a core change, chosen for high totals and
for spread: three visual languages at the bit scale, four at the file scale,
four at the structure scale, one each at field, stream and format scale;
sizes from an inline glyph to a full-window panel. Several are a named slice
of a larger candidate; the rest of that candidate is in section 6.

### 5.1 Piece zero first
Section 3 is built and merged before any batch agent starts: `viz/board.ts`,
`viz/tip.ts`, `viz/pick.ts`, `viz/linked.ts`, `viz/fit.ts`, `viz/lanes.ts`,
`viz/lod.ts`, the role colours in `fieldstyle.ts`, the four registries
(`viz/inspectorsections.ts`, `viz/headingbuttons.ts`, `viz/rowglyphs.ts`,
`viz/overviewtabs.ts`) with their hooks in `inspector.ts`, `listingdraw.ts`
and `overviewpanel.ts`, `formatcards.ts` opened to cards by node kind, and
the dependencies (`d3-zoom`, `d3-scale`, `d3-axis`, `d3-shape`, `uPlot`)
added to `web/package.json`. Piece zero also pre-registers one empty stub
module per batch piece, so every agent below owns its stub and its new files
and touches no shared file at all. Where a batch agent must still touch a
shared file, that file is named and no other agent touches it.

### 5.2 The rules for the agents
- One agent per host file. The host files in this batch: `inspector.ts` (A),
  `treemappanel.ts` and `treemapdata.ts` (B), `filemap.ts`, `trail.ts` and
  `report/posbar.ts` (D), `contentcard.ts`, `report/jpeg.ts` and
  `report/pngscanlines.ts` (E), `jpegcards.ts` (reserved, not in this batch),
  `listingdraw.ts` and `listingreport.ts` (I), `main.ts` and `doc.ts` (J),
  `listpane.ts` and `samplespanel.ts` (K). Everything else is a new module
  under `web/src/viz/` or `web/src/`.
- Never edit a file another agent has in flight; agents merge `main` before
  starting and before opening a merge; merges are gated on `tsc` and the
  tests from a separate worktree (see the memory notes on shared checkouts).
- Every piece ships with its layout arithmetic in a DOM-free module with unit
  tests, as `bitorder.ts`, `strips.ts` and `treemap.ts` do, and is checked in
  the browser on the named sample before it is called done.
- Every piece uses `viz/tip.ts` for hover, `viz/pick.ts` for click, and the
  size-independent `draw(data, opts)` shape.

### 5.3 The batch

| Agent | Candidates | Home | New files | Shared files touched | Test sample |
|---|---|---|---|---|---|
| A | `bit-pack-ribbon`, `varint-gather`, `bit-ruler-register` (inspector and card forms; the row glyph registers through `viz/rowglyphs.ts`) | inspector sections, formatcards card for sub-byte structs | `viz/bitruler.ts`, `viz/varintfigure.ts`, `viz/bitpack.ts` (the generalised host), tests | `inspector.ts` (replace the deflate-only `fillBitOrder` host), `bitorder.ts`, `report/ribbon.ts`, `bytestrip.ts` (export `bitsChip`), the registries | `gzip/gnu-gzip-9-with-name.gz` (deflate codes), `wasm/wabt-imports-data-names.wasm` (LEB128), `zip/streamed-data-descriptors.zip` (flags), `pe/busybox-w32-x86_64.exe` (Characteristics) |
| B | `position-icicle` | sixth treemap mode "In order", grow button for view size | `viz/icicle.ts`, tests | `treemappanel.ts`, `treemapdata.ts` | `mp4/h264-aac-moov-at-end.mp4`, `png/png10-palette-trns-text.png` |
| C1 | `pointer-arcs-axis` (tab and heading strip; the report hook later) | new Pointers overview tab via `viz/overviewtabs.ts`; heading figure via `viz/headingbuttons.ts` | `viz/pointerarcs.ts`, tests | registries only | `pdf/classic-xref-uncompressed.pdf`, `zip/streamed-data-descriptors.zip` |
| C2 | `file-vs-memory-bands` (ELF and PE; the report hook later) | Logical tab figure via `viz/overviewtabs.ts` | `viz/spacebands.ts`, tests | registries only | `elf/busybox-x86_64`, `pe/busybox-w32-x86_64.exe` |
| D | `locator-family` slice: live file map (hover, click, width parameter, second ring), crumb bars, `posbar.ts` merged in | `filemap.ts`, `trail.ts` | `viz/zoomstack.ts` (the stack, mounted later), tests | `filemap.ts`, `trail.ts`, `report/posbar.ts`, `report/parts.ts` (posbar call site) | `zip/streamed-data-descriptors.zip`, `pdf/linearized.pdf` |
| E | `pixel-grid-linked` | content card gains zoom, pan and overlays; the JPEG MCU map and PNG figures become overlay modes | `viz/pixelgrid.ts`, tests | `contentcard.ts`, `report/jpeg.ts`, `report/pngscanlines.ts` | `png/pngsuite-basi6a16-rgba-16bit-interlaced.png` (Adam7), `jpeg/pillow-cmyk-adobe-restart.jpg` (restart intervals), `jpeg/libjpeg-turbo-testorig-baseline.jpg` |
| F | `struct-layout-padding` | formatcards card for any composite, offered from the heading via `viz/headingbuttons.ts` | `viz/structlayout.ts`, tests | registries only | `elf/busybox-x86_64` (ELF header, program headers), `cdrom/hello-mode1.bin` (sector anatomy) |
| I | `record-small-multiples` slice: heading tile grid and run sparkline (table column sparks later) | listing heading row and the run's first element | `viz/recordtiles.ts`, `viz/sparkline.ts`, tests | `listingdraw.ts`, `listingreport.ts` | `png/png10-palette-trns-text.png` (chunk tiles), `midi/format1-smpte-three-tracks.mid` (event sparkline), `zip/streamed-data-descriptors.zip` |
| J | `space-nesting-components` slice: lineage breadcrumb in tab titles and the inspector heading, `Doc.spaceOrigin` wired to the existing wasm export (the component diagram later) | tab strip in `main.ts`, inspector heading through `viz/inspectorsections.ts` | `viz/lineage.ts`, tests | `main.ts`, `doc.ts` (append one method) | `png/png12-itxt-deflated-ja.png` (IDAT joined, zlib, zTXt), `torch` folder (zip with pickles), `gzip/eight-members-split-tar.tgz` |
| K | `value-plot` (critic) | heading button opening the plot in the list pane; `SamplesInfo` inspector section | `viz/valueplot.ts`, `viz/decimate.ts` (min-max decimation, DOM-free), tests | `listpane.ts`, `samplespanel.ts`, registries | `wav/broadcast-pcm16-bext-peak.wav`, `seismic/blockette300.mseed`, `gguf/llama2c-stories260k-f32.gguf` |
| L | `index-lookup` (critic), inspector section only (the `-> reads` hover card later, since it lives in `listingdraw.ts`) | inspector section via `viz/inspectorsections.ts` | `viz/indexlookup.ts`, `viz/tableentries.ts` (string table, palette and entry-list readers), tests | registries only | `elf/busybox-x86_64` (`sh_name`), `png/png10-palette-trns-text.png` (palette), `tiff/libtiff-float64-predictor2-lzw-mm.tif` (IFD offsets) |
| N | `stride-grid` slice: the raw raster with stride control and periodicity line (page-owner adapters later) | new Stride overview tab via `viz/overviewtabs.ts` | `viz/strideraster.ts`, `viz/periodicity.ts` (autocorrelation in a worker), tests | registries only | `cdrom/hello-mode1.bin` (2352), `tar/pax-long-path.tar` (512), `wav/g711-alaw-mono-8000.wav` |

Agents C1, C2, F, L and N touch only registry files and can run in any order;
A, B, D, E, I, J and K each own their host files. All sample folders named
above exist under `../qubero-samples` (checked 2026-10-29).

### 5.4 What makes the batch diverse
Visual languages: ribbon bands, RFC bit ruler, protobuf byte gathering,
icicle partition, arc diagram, two-axis bands, locator maps and crumb bars,
zoomable raster with overlays, pahole struct layout, unit chart and
sparklines, lineage breadcrumb, decimated line and heatmap,
lookup figure, stride raster. Scales: bit (3), field (1), structure (4),
stream (1), file (4), format (1). Sizes: inline (varint chip, byte glyph,
file map, crumb bars, breadcrumb, sparkline), figure (most), panel (icicle,
arcs, bands, pixel grid, raster, plot) and view through the grow buttons.

## 6. Later batch, and candidates needing core changes

### 6.1 Later batch (no core change)
In the order they become cheap once the first batch is in:
- `huffman-tree` (JPEG DHT first; `jpegcards.ts` is free once E is merged;
  deflate tables after A).
- `tree-table-gantt` (critic): the position column goes in once agent I has
  merged, since it shares `listingdraw.ts` and `listingreport.ts`; test on
  `wav/data-before-fmt-pcm16.wav`, whose fields are out of file order.
- `reads-dag` as a layout mode in `graphview.ts`; still behind `?graph`.
- `annotated-byte-strip` in slices: proportion and braces on `bytestrip.ts`
  (after A has exported `bitsChip`); the hex-view in-grid underlines in
  `hexrows.ts`; the poster mode and seal Gantt last.
- `record-diff` (critic): two records in one file; two files when a second
  `Doc` can be opened.
- `parse-failure-explainer` (critic): the figure on the problem glyphs; the
  candidate-rules row needs the identification dialog's `SigVerdict`.
- `locator-family` remainder: the zoom stack in the inspector heading and
  `hexheadings.ts`, the scrollbar track (`viz/overviewtrack.ts`) over
  `rowheights.ts` in the hex, listing and text views.
- `record-small-multiples` remainder: Table view column sparks in
  `tablecolumns.ts`, the run sparkline in `valuetable.ts`.
- `space-nesting-components` remainder: the component diagram as a Spaces tab.
- `stride-grid` remainder: page-owner adapters for sqlite, CFB and ISO with
  the chain overlay.
- `pixel-grid-linked` remainder: FITS tiles, raw-range width slider with
  suggested widths from `viz/periodicity.ts`.
- `value-pipeline`, leading with the inline endianness glyph.
- `readings-lanes`, `tlv-nesting-arcs` (the tag-tile glyph first, which is a 5
  on its own), `pipeline-sankey` (the ratio glyph first), `lz77-arcs`,
  `railroad-grammar`, `schema-class-er`, `record-pointer-boxes` (chains
  first), `decides-matrix`, `timestamp-timeline` (the inline glyph first),
  `hilbert-map`, `dump-reconstruction` (critic), `digraph-plot` (worker form),
  `shape-parallel-sets`, `step-decoder-timeline` (JPEG lanes first).
- `file-to-bit-zoom` last: it is mostly the icicle, the byte-class strip and
  the bit ruler on one canvas, so it is cheaper once those exist.
- `entropy-histogram-strip`: the histogram half and the sparkbar now; the
  entropy line bridged web-side until the core array exists.

### 6.2 Candidates waiting on core or IR changes
Each needs one DTO from section 3.12 before its full form can be drawn:
- `expression-tree`: an `Expr` AST per `Relation`. The worked-equation card
  with role chips needs nothing and can go in the later batch.
- `offset-frames-ruler`: the anchor kind and base bit of each `At` field.
- `order-vs-address-ribbon`: the parts list of a stitched space.
- `record-pointer-boxes` reference-graph and dominator mode: record reference
  edges (PDF `n 0 R`, HDF5 object headers).
- `pointer-arcs-axis` numbered reading path: a walk-order seek trace.
- `step-decoder-timeline` whole-stream form: a bulk decode step list.
- `timestamp-timeline`, the seal Gantt in `annotated-byte-strip`, and the
  file-wide `record-small-multiples`: bulk lists of time fields and check
  fields, so big files do not need one wasm call per node.
- `entropy-histogram-strip` and `digraph-plot` at 100 MB: per-bucket entropy
  and byte-pair counts from the core scan.
- `value-plot` at speed: a numeric run as a typed array.
- `railroad-grammar` loop-exit labels: `Until` conditions exported in
  `TemplateDiagram`.
- `parse-failure-explainer` beyond magic and extents: a structured expected
  value on `Problem`.

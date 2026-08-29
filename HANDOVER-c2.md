# Handover: implement the unified listing view (mockup C v2)

Written 2026-08-29 for the implementing agent. The spec is `c2-listing.html`, which
lives with the other mockups outside this repository, in
`../qubero2-extras/mockups/`. Open it in a browser directly; the dev server no longer
serves a copy. This document says what to build, where the pieces go, and which parts
of the mockup are binding vs. illustrative.

## Goal

Merge today's three overlapping structure views — `web/src/listingview.ts`,
`web/src/logicaloutline.ts`, and the bottom Structure panel driven by
`web/src/typetable.ts` — into one scrollable view that is both the overview and the
listing:

- Collapsed to headings, it reads like the current structure/overview panel.
- Expanded, it is the listing: one row per field, in physical file order.
- Any heading or row can unfold its bytes in place as a field-shaded hex strip.

The old views should end up as thin wrappers or be deleted; do not leave three parallel
implementations alive.

## The binding design rules

These were the point of the mockup rounds; keep them even where convenient shortcuts
exist:

1. **Physical order, every byte accounted for.** Unused/zero/reserved runs are rows
   like any field, styled as gaps (striped), with a size and a verification verdict
   ("3,746 B, verified zeros"). Nothing silently vanishes. Unknown/undefined regions get
   the same treatment as known ones (same columns, dim styling).

2. **Headings scale with structural depth.** Top-level format divisions (pages,
   segments, sections) are h0 headings with: color swatch, title, address range,
   "bytes" toggle, size + share, and a mini file map. Sub-divisions are h1. Depth
   beyond that is rows, not more indentation ladders.

3. **Machinery folds behind its owner.** Length prefixes, cell-pointer arrays,
   serial-type headers, "23 more fields, all default" — one dim collapsible row per
   owner (`▸`), never interleaved with the payload rows at full loudness. The payload
   keeps its own name and value at full strength.

4. **The repeated map.** One small fixed-geometry strip of the whole file (the same
   segments every time; a minimum lit width so tiny ranges stay visible) appears in
   every heading and every open byte strip, with only the lit part changing. It shows
   the *physical* location of that item's bytes. Sub-ranges light a sliver inside their
   parent segment. This replaces per-row percentage bars.

5. **Two palettes, one per scale, never mixed roles.**
   - *Section palette*: colors the rail map, heading swatches, and lit mini-map
     segments. Assigned per top-level division.
   - *Field palette*: five hues (`#5b8dd6 #62c48b #c9a45c #b48ce0 #d98a9e` in the
     mockup) cycled per field *within one open byte strip*. A field's hue appears in
     exactly three places: its byte tint, its bracket label, its chip. Nowhere else.
   - The existing `web/src/fieldstyle.ts` may already own field coloring; reconcile
     rather than adding a third scheme.

6. **The byte strip.** Column-per-field layout: dimmed hex digits on top, a colored
   bracket label directly beneath, clipped to exactly the field's byte width (CSS in
   the mockup: label `width: 0; min-width: 100%; overflow: hidden`). The digits are
   deliberately dim — in this view their job is byte count and position; values live in
   the chips below, same hues, same order. Sub-byte detail uses the bits chip pattern
   (see the varint chip: stop bit + payload bits).

7. **Records render as records.** Where the format stores a table (SQLite rows, GGUF
   metadata), show an actual table with the format's own column names, plus a
   "stored at" column and a link back to the field-by-field view. Cross-references
   (root page N, cell pointers) are links with a direction arrow.

8. **Shared selection.** Selecting a row highlights it in the record table, the open
   byte strip, and the mini map. Same state object as the hex view cursor
   (`Evaluator::locate` already maps bit → field path; the listing needs path → DOM
   row, which the current listing partially has).

## What is illustrative, not binding

- Exact pixel values, the 210px rail width, specific chip wording layout.
- The mockup's static "open" states — really everything toggles.
- The mode buttons ("Logical tree", "Expand all", "Headings only") are real features
  but "Logical tree" can land later; ship physical order first.
- Search/filter and a side-by-side logical-tree pane: explicitly out of scope.

## Where the data comes from

- Tree: `doc.templateNode(path)` / `doc.templateChildren(path, from, to)` in
  `web/src/doc.ts` (wasm surface over `crates/core/src/eval.rs`). Note
  `templateChildren` returns `type` and `value`; `templateNode` does not carry
  `type_name` — extend the wasm surface if the view needs it (bindings only in
  `crates/wasm`; logic in core).
- What counts as a "major section" / where headings go: this is a *view* decision, not
  the IR's. Heuristic that matched the mockups: children of the root struct that are
  containers (pages, segments, chunks) become h0; their named sub-composites h1; leaf
  runs become rows. Expect per-format tuning; put the heuristic in one place.
- Gap verification ("verified zeros") needs a byte-scan over the run — the overview
  panel (`web/src/overviewpanel.ts`) already computes zero/text/data classes per cell;
  reuse that machinery rather than rescanning.
- Machinery-vs-payload classification (rule 3) is **done**, as both. `crates/core/src/
  machinery.rs` reads the template's own expressions and answers, per field, which
  sibling reads it for a length, a count, a type or a position; that reaches the app as
  `TemplateNode.consumed_by`. `StructDef::machinery` and `StructDef::payload` name
  fields either way and win, arriving as the tri-state `TemplateNode.machinery`.
  Whether a field is *folded* is the view's call, not core's: see `isMachinery` in
  `web/src/flatten.ts`. SQLite's root structure is flat, so `page_size` is a sibling of
  the pages it sizes and would fold away on the shapes alone; it stays a header row
  because the pages are a different part of the file.

## Virtual scrolling constraint

The current listing renders a window, not the whole tree, and large files (GGUF: 389
tensors, 250k-string token arrays; h5ad) must stay usable. The report layout must
therefore be virtualized like `hexview.ts` is: headings and rows are flat render items
with computed heights. Sticky breadcrumb = the deepest heading scrolled past, which
falls out of the flat item list. Do not build this as one giant DOM.

## Suggested order of work

1. ~~Flatten the template tree into render items (heading/row/gap/fold/record) with the
   section heuristic.~~ **Done**: `web/src/flatten.ts`, tested by `npm test` in `web`
   (Node's own runner over fixture trees, no framework and no wasm). The section rule
   lives in `sectionBreaks` and `elementsAreSections`, and expects per-format tuning.
2. ~~Render items virtualized; headings, rows, gaps, folds.~~ **Done**:
   `web/src/listingreport.ts`, mounted dev-only behind `?view=report`. Includes the
   sticky trail, which the virtualization note asks for.
3. ~~Mini map component.~~ **Done**: `web/src/filemap.ts`, used by the headings. The
   rail and the strip captions take it as they land.
4. ~~Byte strip on demand per item.~~ **Done**: `web/src/bytestrip.ts`, over
   `Doc.spans` and `chipfit`'s chip vocabulary, plus gap verdicts in
   `web/src/gapcheck.ts`. **One part of rule 6 is not built**: see the open questions.
5. ~~Record rendering for formats that declare it (SQLite cells first).~~ **Done for
   table leaves**: `web/src/records.ts`, a registry keyed by template name. Left for
   its next entries: index pages and b-tree interior pages, which are not tables of
   rows; the schema page itself, whose columns SQLite fixes rather than declares;
   and GGUF metadata. A row's "stored at" opens its bytes under the table, which is
   rule 7's link back to the fields. The other link rule 7 asks for, a schema row's
   root page pointing at that page, waits for the shared selection in step 6.
6. ~~Shared selection with the hex view cursor.~~ **Done**: the selection is a bit
   range rather than a row, so the same state lights the row, the record table's
   line, the byte strip's column and every file map. `main.ts` drives whichever of
   the two views is showing through one name, `structure`, which is what step 7
   collapses back to one thing.
7. ~~Swap it in as the Listing mode; delete/absorb `listingview.ts`,
   `logicaloutline.ts`, and the Structure panel's tree duplication.~~ **Done for the
   listing**: the dev flag is gone, `listingview.ts` is deleted with the CSS only it
   used, and the report gained the keyboard the old view had, as a row cursor rather
   than as a scroll. Two of the three deletions were not made, and the reasons are
   not "later":
   - `logicaloutline.ts` is not another copy of this tree. It is per-format adapters
     reading a file as its own domain objects, and the logical-tree mode that would
     absorb them is deferred. It stays until that mode exists.
   - The Structure panel (`typetable.ts`) edits values in place, which the report
     cannot do. That is an absorb, not a delete, and the editing has to move first.

## Test recipe

- Dev server: `.claude/launch.json` name `web` (vite, port 5173). No file-open URL
  param: fetch a sample from `web/public/samples/`, wrap in `File`, dispatch a
  synthetic `drop` on `document`. `window.__qubero` exposes `{doc, view, inspector,
  table, structure, overview}` for headless assertions.
- Files to exercise, in order: `notes.sqlite` (the mockup's file, to compare against
  the mockup side by side), `tiny.png`, `D:/koboldcpp/bge-m3-q8_0.gguf` (scale + deep
  length-prefixed metadata), an `.h5ad` from
  `D:/datasets/zebrahub/figshare-2026-08-26/20510367` (depth), and
  `C:/Windows/Web/Wallpaper/Theme1/img1.jpg` (huge opaque run).
- **Correction: the vite `fs.allow` trick does not work.** Adding the directory to
  `server.fs.allow` and fetching `/@fs/D:/koboldcpp/...` returns `index.html`, restart
  or no; vite will not hand out an arbitrary binary that way. Two routes that do work:
  a dev-only middleware plugin in `web/vite.config.ts` serving a configured directory
  with `Range` support (about fifteen lines, and it covers the `.h5ad` too), or a
  range-fetching `ByteSource` passed straight to `Doc.open`, which takes that shape
  already (`web/src/doc.ts`), so nothing has to be held in memory. Step 1 got by on
  `dump_tree` for the GGUF; steps 4 to 6 will not.
- Native-side checks without the browser:
  `cargo run --release -p qubero-core --example dump_tree -- <file> [template] [depth]`.
- Core changes need `npm run wasm` (~35 s) before they reach the app.
- The in-app browser pane paints and screenshots fine at the top of a page, but
  screenshots of *scrolled* positions come back garbled (pane capture bug). Verify
  scroll behavior via `window.scrollTo` + DOM queries, or trust a real browser.

## Repo conventions that will bite

- **No `cargo fmt`** — the repo is hand-formatted wide; formatting churns 44 files.
- Python scripted edits on Windows flip line endings: open with `newline=''` or check
  `git diff --stat` before committing.
- Commit to `main` as you go, plain subjects, no co-author byline.
- **User-facing strings: reuse the mockup's wording verbatim.** The mockup copy has
  been through review. If a state needs a string the mockup doesn't have, do not
  invent one — collect the needed strings and ask, or hand the drafting to a
  Fable-class model, then list all strings at the end of the turn (see the user's
  global CLAUDE.md rules).

## Known open questions (fine to defer, don't silently decide)

- Where "Logical tree" mode gets its alternate ordering from (likely a second
  flattening of the same tree, cells in pointer order instead of physical order).
- Whether the record-table rendering is declared per format in the template IR or in a
  TS registry keyed by template name. **Answered for now: the registry**, in
  `web/src/records.ts`, on the grounds that one format is not evidence for changing
  the IR and a second or third would be. Revisit when GGUF metadata joins it. (Mockup E in `../qubero2-extras/mockups/`
  sketches the wider address-space question; it is context, not part of this task.)
- Rule 6's **bits chip is not built**. The rule says sub-byte detail uses the bits
  pattern, and the mockup's row 3 cell shows it: `payload_size 18 [0|0010010]
  varint, 1 byte: high bit 0 ends it`, with the stop bit and the payload bits drawn
  apart. The strip today writes `payload_size 18 1 byte` and stops. It was left out
  because it is not a rendering job: which bits are marker and which are payload is
  decode knowledge, held by `SqliteVarint`, `Leb128`, `EbmlVint` and `Vlq` in core.
  Drawing it in TS means either writing those decoders a second time in the view or
  exposing bit roles from core the way `consumed_by` was exposed, which is the
  better answer. The chip's tail ("high bit 0 ends it") is per-type copy and needs
  a drafting pass of its own. A commit series, not a patch.
- A gap's label and its verdict can disagree, and that is deliberate. `board.dtb`
  reads `unused space` beside `not all zeros`, because the label is what the shape
  of the template implies and the verdict is what the bytes actually say. A template
  that does not cover everything it should is exactly the case the verdict exists to
  catch, so the row is meant to look odd there.
- Rule 3's *second* fold kind, "23 more fields, all default", is on the back burner
  rather than deferred to a date. A long header is not a problem in itself, and it is
  not clear the idea of a default even carries across formats: a SQLite header's
  zeros are defaults, a PNG IHDR's are values. The IR has no notion of one either.
  Explore it on its own terms before building anything: what would have to be true
  of a format for "all default" to be an honest thing to say about a run of its
  fields, and whether the answer is a per-template list or something a reader can
  check for themselves.

## Long lists (a task of its own, after the steps above)

A list of two hundred is a page. A list of two hundred thousand is a different
problem, and the app already handles it in a way that feels clunky. Two things are
wanted, and neither is in the seven steps:

1. ~~**Reach one item without unfolding the ones before it.**~~ Done, in
   "Draw a window on a long list, not its first page". `shown` holds a
   `{from, to}` per list rather than a count, both ends of it are rows, and
   `reveal` moves the windows on the way to a field a page at a time, aligned to
   pages. The ends own the bytes of the elements they stand for, which is what
   keeps them from reading as gaps. Verified against a git pack index: clicking
   byte 0xee68 selects `names[3000]`, draws 3,000 to 3,200, and the listing stays
   at 816 items.

2. ~~**Give a long list somewhere of its own to live.**~~ Done, in "Read one long
   list in a pane of its own": `web/src/listpane.ts`, opened from the control on a
   long list's heading and on both ends of a drawn window. It scrolls by index with
   no paging, since every element is one row of the same height, and it shares the
   selection with everything else. Verified on a git pack index: 3,306 names, scroll
   to the far end, click through to the hex view and back.

Both are about the same thing: a list long enough to be its own subject stops being a
row that opens. Both are built.

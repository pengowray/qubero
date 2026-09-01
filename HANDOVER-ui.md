# Handover: the UI as three surfaces

Written 2026-09-02. Replaces `HANDOVER-c2.md` as the working spec for the UI; that
document's binding rules for the listing still hold and are not repeated here.

## What was wrong

Four places said "where am I in the file's structure", each in its own vocabulary:
the Overview sidebar's Regions list, the bottom Structure panel's tree, the listing's
headings, and the inspector's trail. Two of them (Regions, Structure) were built from
the template's root children directly, so they disagreed with the listing about what
the parts of the file are. The bottom panel took 38% of the height from whichever main
view was showing, to repeat what the listing shows better. The hex view's field column
cut annotations off with `+N` on rows with five short fields while the rows below had
room to spare.

## The model

One file, one position (the hex cursor, a bit), three surfaces with one job each:

| Surface | Job | Answers |
|---|---|---|
| Contents rail (left) | navigate | what is in this file, how big each part is, where I am |
| Main view (centre) | read | the bytes, as a listing, as a hex grid, or as text |
| Inspector (right) | detail and edit | what is at the cursor, what it depends on, change it |

The bottom Structure panel is gone. Its tree is the rail's Contents; its Logical mode
is the rail's Logical tab; its editing was already the inspector's (both call
`doc.writeNode` with the same path).

### One source of truth for the parts of the file

`ListingReport.outline()` is the list of headings the listing draws (level 0 and 1),
each with its section index, path, name, extent and colour. The rail lists them, the
hex view draws heading rows from them, and every file map's segments are the level-0
ones. Nothing else works out what the parts of the file are. `sectionBreaks` in
`flatten.ts` stays the one heuristic.

### Colour: three palettes, three scales, never mixed

Already in `fieldstyle.ts`, now the rule for every surface:

- **Section colour** (`sectionColor`): one per top-level part. Rail swatches and layout
  strip, listing h0 headings, hex heading rows, lit file-map segments.
- **Field family** (`fieldClass`): what bytes mean (number, text, marker, category,
  structure, binary). Hex grid tints and chips, record cells, inspector.
- **Strip hue** (`fieldHue`): which field is which inside one open byte strip. Its
  bytes, its bracket, its chip. Nowhere else.
- The byte-class map (zeros, repeated, text, data, entropy) is a fourth vocabulary and
  appears only on the class map and its legend.

## The rail

Top to bottom, ordered by what a reader wants first:

1. **Facts**: size, the identification sentence, scale of the map. As now.
2. **Map**: the byte-class map as now, and under it a one-row **layout strip** coloured
   by section (same `fileMap` the headings use, wider). The two say different things
   and sit together so the reader can see "the zeros are the unused half of page 1".
3. **Contents**: the outline. Level-0 headings as rows: swatch, name, size, share.
   Level-1 headings nested under them, folded by default, unfolded for the section the
   main view is in. Scroll-spy: the heading whose bytes are at the top of the main
   view (hex or listing) is marked, and the rail scrolls to keep it visible. Clicking
   a row moves the cursor to the part's first bit and brings the main view there; the
   listing scrolls to the heading, the hex view to the row. Hover lights the part on
   the map, as Regions did.
   A file with hundreds of level-0 parts (a SQLite file of 100k pages) lists them
   virtualised or windowed, not as 100k buttons: show the first N with "and 99,900
   more pages", and always the one the view is in.
4. **Logical** tab beside Contents, present only where `logicaloutline.ts` has an
   adapter (HDF5, GGUF, zip, SQLite, ELF, ISO 9660, WAV). The tree the bottom panel's
   Logical mode drew, moved here. Same click behaviour.
5. **Notes**: the sentences ("7.56 KiB (63%) at 0x70 is zeros"). As now.
6. **Block detail**: the picked cell's stats and its unmapped stretches. As now, and
   still only after a cell is picked.

The rail keeps its collapsed state and stays 17rem. It is `overviewpanel.ts`
restructured, not a new file beside it; `typetable.ts` is deleted once its Logical
rendering has moved.

## The hex view

1. **Annotations wrap.** A row whose chips do not fit in the field column continues
   on further lines of the same row, up to three lines; only past that does `+N`
   appear. Rows are therefore not all one height. Row elements are still created for
   `height / rowHeight` rows and extra ones are clipped at the bottom; the cursor is
   kept in view by geometry (`getBoundingClientRect` of its cell against the
   viewport), not by row index. The scrollbar keeps mapping rows.
2. **Section headings inside the scroll.** Before the row that holds the first byte of
   a level-0 part, a heading line: swatch, name, address range, size and share, from
   `outline()` (passed in by `setSections`). Level-1 parts get a smaller line. A part
   that starts mid-row gets its heading above that row; the byte itself keeps its
   field-start hairline. Headings are not rows: they do not take a cursor and the
   row-to-offset mapping is unchanged.
3. **Regular runs stay one chip.** `spans` already folds a run of plain numbers into
   one span with a count. Check what still arrives as a jumble on the samples
   (`tune.mid`, `bat.wav`, `board.dtb`, `tagged.mp3`, `hello.exe`) and fold what
   should fold: a run of same-typed elements of a list is one chip
   (`codes 512 values`), not 512.
4. The view reports what it shows: `onViewport(startBit, endBit)` after each render.

## The listing

1. **Hierarchy you can see from across the room.** Level-0 headings get more space
   above than below (space groups; lines do not), a larger title, the swatch, range,
   size and share on one line. Level-1 headings are visibly smaller. Rows are rows.
   No indentation ladders: depth beyond a level-1 heading is a row with a dimmer
   name, as the c2 rules say.
2. **Content card.** For a file whose template is an image the browser can decode
   (PNG, JPEG, GIF, BMP, WebP), the listing opens with a level-0 section named for
   the content ("Image") showing the decoded picture, scaled to fit, with its pixel
   size beside it. This is the first honest step of the content-first goal; there is
   no pixel-to-byte mapping yet, and the card does not pretend there is. Decode via
   `createImageBitmap` on a blob of the document's bytes (whole-file read is fine for
   images; refuse past 64 MiB with a line saying so).
3. The listing exposes `outline()` and fires `onOutline` after every rebuild, and
   `onViewport(startBit, endBit)` after every paint, from the first and last mounted
   items.

## The toolbar

Left: Open, Save as, file name and size, identification. Centre: the view switch.
Right: the showing view's controls, then Template, Undo, Redo. The view switch is
already there; the only change is that the bottom panel's buttons ("Open visible
sections", "Collapse to overview") go with the panel.

## Not in this round

Logical-order listing (the rail's Logical tab covers the need), address-space
mapping for compressed and paged content (mockup E), pixel-to-byte mapping, search
inside the listing, the audio and video content cards.

## Work packages

Three worktree agents, in parallel, after the shared hooks are in `main`:

- **A, the rail**: `overviewpanel.ts`, `typetable.ts` (delete), `main.ts` wiring,
  `style.css` (overview and typetable sections), DESIGN.md "The overview".
- **B, the hex view**: `hexview.ts`, `chipfit.ts`, `style.css` (hexview section),
  DESIGN.md "The field column".
- **C, the listing**: `listingreport.ts`, `listingdraw.ts`, `flatten.ts` if the card
  needs an item kind, `strings.ts`, `style.css` (report section), DESIGN.md "The
  listing".

Each: `npx tsc --noEmit` and `npm test` in `web` before every commit; plain commit
subjects; no byline. Test files, in order: `notes.sqlite`, `tiny.png`, `img1.jpg`
(untracked copy in `web/public/samples`, not to be committed), then
`/local/koboldcpp/Kokoro_no_espeak_Q4.gguf` and an `.h5ad` under
`/local/datasets/zebrahub/...` on the `web-local` server (`.claude/launch.json`, port
17273, serves `D:/`). Verify scrolled states by DOM queries; the pane's screenshots of
scrolled positions are unreliable.

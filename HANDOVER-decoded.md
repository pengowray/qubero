# Handover: unpacked streams as a second address space

Written 2026-09-03. Extends `Ty::Decoded` (see DESIGN.md, "A stream read as the
fields inside it") from "fields nested under the stream in the listing" into a
general feature: any compressed run can be opened as its own document, in its own
tab, and that document stays connected to the bytes it came from.

## The model

A **space** is an address space. Space 0 is the file. A `Decoded` node opens a
space over the bytes its codec produces. A space is a document like any other:
it has bytes, a template (the `Decoded` node's `inner`, or, when that is only
`bytes`, whatever `recognise` says the unpacked bytes are: a tar inside a gzip,
a PNG inside a zip entry), a cursor, a selection, and all three main views.

A space is **connected** to the space it was unpacked from by a **map**: for any
byte of the output, which bits of the input produced it and by which step; for
any bit of the input, which output bytes it produced. How fine the map is
depends on the codec:

| Codec | Map granularity | The steps a reader can see |
|---|---|---|
| deflate, zlib, gzip | per symbol | block header, Huffman code lengths and tables, each literal or `match(len, dist)` |
| LZ4 block | per sequence | token, literals, offset, match length |
| zstd | per block | frame header, block headers; inside a block only as bytes this round |
| xz / lzma | per block | stream and block headers; inside a block only as bytes |

The steps are **fields in the compressed space**. The deflate template gains real
structure: blocks, their headers, the code-length alphabet, the literal/length
and distance tables, and the symbol run (folded as one chip per run, opened on
demand). Nothing is shown that the decoder did not actually read; the trace the
decoder emits is the only source of the fields and of the map.

## What the reader can do

- **Open unpacked**: from a decoded stream in the listing or the inspector, a
  button opens the space as a tab. The tab strip sits above the toolbar: the file
  first, then one tab per opened space, named `<stream name> unpacked from
  <file name>`; closing a tab forgets nothing (the space is cached in the core).
- **Follow the cursor both ways**: the status bar of an unpacked tab shows where
  the byte under the cursor came from (`from bits 0x1a3.5 to 0x1a4.2 of
  hello.txt.zst: match, 5 bytes back 12`); the compressed tab's hex view marks
  that input range when the unpacked tab is showing, and the unpacked tab's hex
  view marks the output range of the field under the compressed tab's cursor.
  Moving the cursor in either tab updates the other's mark. A click on the mark
  switches tabs to it.
- **Inspector**: a field in an unpacked space shows its origin step in the
  connections section, like any other origin: `unpacked by deflate block 2,
  literal at bit 0x1a3.5`.
- **Editing** an unpacked space is refused this round with a line saying so.
- The rail's map and Contents belong to the tab that is showing.

## Contract between core and web

Core (`crates/core`):

- `Evaluator::open_space(doc, path) -> SpaceId` for a `Decoded` node; the space
  holds the decoded bytes, the trace, and an evaluator over `inner` (or the
  recognised template when `inner` is bytes).
- `Space::map_out(byte) -> Option<Step>` and `Space::map_in(bit) -> Option<OutRange>`
  where `Step { in_bits: Range<u64>, out_bytes: Range<u64>, kind: StepKind }` and
  `StepKind::{Literal, Match { len, dist }, Stored, Block, Header, Table, Opaque}`.
- The deflate decoder is our own (`crates/core/src/codec/inflate.rs`), emitting
  the trace; it is checked byte-for-byte against `miniz_oxide` in tests over the
  sample collection. LZ4 block likewise (`codec/lz4.rs`). zstd and xz keep the
  crates and emit block-level steps from the frame headers they parse.

Wasm/web (`crates/wasm`, `web/src`):

- `Editor.openSpace(path) -> spaceId`; every Doc method takes a space (default
  0), so `Doc` is constructed per tab over `(editor, spaceId)` and the views are
  unchanged.
- `Editor.mapOut(space, byte)` and `Editor.mapIn(space, bit)` for the cursor link.
- `web/src/tabs.ts` owns the tab strip and which tab is showing; `main.ts` wires
  the cursor link.

## Work packages

- **A, core codecs and trace** (crates only): inflate with trace, LZ4 with trace,
  deflate template structure from the trace, `open_space`, `map_in`/`map_out`,
  tests against miniz_oxide on every zlib/gzip/zip sample.
- **B, wasm and tabs** (crates/wasm, web): spaces as documents, tab strip, Open
  unpacked, cursor link, status bar line, inspector origin line. Builds against
  A's contract; until A lands, `map_*` return `None` and the link shows nothing.

Strings the reader sees are written by the coordinator; agents use the wording
in this document and list what they used.

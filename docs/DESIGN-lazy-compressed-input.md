# Design: reading the compressed input of a large run a piece at a time

Written 2026-11-26, for developers. This is a proposal, and nothing in it is
built yet. It follows `docs/DESIGN-lazy-unpacking.md`, which describes how a
compressed run whose output is past `codec::CAP_BYTES` (64 MiB) is indexed in
steps and read a span at a time. That design reads the output a piece at a
time. This one does the same for the compressed input, so that memory no
longer grows with the compressed size and the browser can open a run whose
compressed bytes do not fit in its chunk store.

"Still to do" and "Open questions" at the end list what this leaves open.
"What was checked" lists the code read to write it.

## The problem

The output side of a run read a piece at a time is bounded by the table in
DESIGN-lazy-unpacking's "Limits": checkpoints, span cache, traces. The
compressed side is not:

- `Evaluator::index_pass` (`eval/space.rs`) reads the whole run with one
  `read_in` before its first step, and hands it to `lazy::Indexing::new` as an
  `Arc<Vec<u8>>`.
- The `Seekable` the pass makes keeps that vector (`Seekable::packed`). Every
  span decode, span trace, block trace, block start found again
  (`Seekable::group`) and `Seekable::block` reads from it.
- `Indexes` keeps up to 64 MiB of these vectors between all its indexes
  (`INDEXED_RUN_BYTES`), weighed by `Seekable::packed().len()`.
- `Joined` keeps a copy of every stored part and of every packed part's run
  (`JoinedBytes::Held`, `JoinedBytes::Packed`), up to `whole_cap` (64 MiB)
  between them (`Evaluator::join_lazily`).
- So a run read a piece at a time may be at most 64 MiB compressed.
  `open_space_at` (`size / 8 > CAP_BYTES`), `index_pass`
  (`key.2 / 8 > CAP_BYTES`), `decode_or_index_with`, `Indexing::new` and
  `DeflatePass::new` (`packed.len() > CAP_BYTES`) all refuse past that.

In the browser the limit is lower. A `read_in` of the file is answered only
when every chunk it touches is in the file's `ChunkStore` at the same time.
The store keeps 512 chunks of 64 KiB (`CHUNK_CAPACITY` in `web/src/doc.ts`),
so a read of more than 32 MiB is never answered:

1. `read_in` answers `EvalError::Pending` with every chunk it is missing.
2. `Doc.settle` fetches them, and after a run of contiguous misses it also
   starts reading the 48 chunks that follow (`READ_AHEAD`).
3. Each chunk fed in pushes out the chunk read longest ago
   (`ChunkStore::insert`), which is one this read needs.
4. The next ask answers `Pending` again, for the chunks pushed out.

Two files have shown it:

- **A zip with one deflated entry of 34 MiB compressed** (seen on
  2026-11-26). The entry's `data` is a `Decoded` node whose size the local
  header gives. When the listing asks what is inside it, `open_space_at` calls
  `index_run`, and `index_pass` reads all 34 MiB with one `read_in`. The pass
  is never made, so `Unfinished` is empty, `Evaluator::unpacking` answers
  nothing, and `pendingText` (`listingdraw.ts`) shows `REPORT.reading`,
  "Loading bytes needed to map these fields…", for as long as the file is
  open. Open unpacked stops after a round or two with `UNPACKED.tooMuchAtOnce`,
  because `waitToOpen` sees a chunk asked for again.
- **A .gz of more than 32 MiB compressed** fails earlier. The gzip template
  sizes `members[i].compressed` with `E::stream_len(Codec::Deflate)`, and
  `Evaluator::stream_len_at` (`eval/expr.rs`) reads everything from the start
  of the run to the end of the file, up to 64 MiB, with one `read_in`, to hand
  to `inflate::inflate_len`. The member never gets a size, so nothing under
  it reads, `index_run` included.

Every other read that takes a run whole sticks the same way past 32 MiB:

- a run of a codec that cannot be indexed (`open_space_at` reads it with
  `self.read` before `decode_traced_with`);
- a packed part of a stitched stream (`Evaluator::unpack_run`);
- `join_whole` and `join_lazily`, which read every part in one call;
- a check over a run of the file (`Evaluator::covered_bytes`);
- the code panel: the wasm crate's `Editor::decode_step` reads the run from
  its front to the end of the step (`field_bytes` with `want_bytes`), so a
  step near the end of a 34 MiB run waits on 34 MiB.

### The rule

Every one of these is a breach of the same rule, and each part of this design
applies it to one reader:

> One synchronous call may need only a bounded amount of the file at once,
> well under what the chunk store keeps. Work that needs more is done in
> steps, and each step keeps its progress across a `Pending` as the index pass
> already keeps it across a `Busy`.

## What changes for the reader

- A zip entry, a gzip, zlib or raw deflate run, a zstd run of several frames
  or an xz stream of several blocks opens in the browser whatever its
  compressed size, up to 512 MiB compressed and 1 GiB unpacked.
- While such a run is read, the listing says how far the unpacking has got,
  as it already does for a large run whose compressed bytes are in memory.
- A tab over such a run reads the file as it goes. A part of the tab whose
  compressed bytes have been pushed out of the store shows as loading, as a
  part of the file's own tab does, and is filled in when the chunks arrive.
- Memory for a run read a piece at a time is what DESIGN-lazy-unpacking's
  table bounds, plus a few small additions (see "Memory"), whatever the run's
  compressed size. The compressed bytes stay in the file's chunk store, which
  the file's tab uses anyway.
- A read that still needs more of the file at once than the store keeps says
  so, instead of waiting for ever.

## Recommendation

1. **Say why, now.** A read that needs more chunks at once than the store
   keeps fails with a reason instead of answering `Pending` (part 1). Small,
   and it stays as the guard for any read later parts do not convert.
2. **Gather a run a window at a time.** A run that is read whole is read a
   window at a time across goes, into a copy kept between goes, with memory as
   today (part 2). This alone fixes both files above, up to 64 MiB
   compressed.
3. **Decoders read a window.** Every decoder reads from a window of the run,
   with positions still counted from the run's front, and stops at a place it
   can carry on from before the window runs out (part 4).
4. **A run reads its input through a handle.** A run's compressed bytes are
   read through a `RunInput`: a shared handle onto the source the run sits in,
   with where the run starts in it. The index pass reads its windows through
   it, and the `Seekable` keeps it to read spans later. A read that finds
   chunks missing reports them and waits for nothing (parts 5 to 7).
5. **A missing chunk is never a refusal.** It travels as `Missing` up to
   `read_in`, which answers `EvalError::Pending`, and up to
   `SpaceSource::read_bytes`, which reports it as a chunk of the file. The web
   side already fetches every chunk a reply names as a chunk of the file.
6. **Every read of a `Seekable` is bounded in compressed input.** Deflate
   checkpoints are taken by compressed distance as well as by output, a block
   trace stops decoding once its trace has turned coarse, and a zstd frame or
   xz block over 16 MiB compressed is refused as too large (part 6).

"Order of work" splits this into parts that build and merge on their own.

## Reading a span later, with no evaluator

The pass runs in the evaluator, which has the document. A span is read later
by whatever holds the `Seekable`: the evaluator's `read_in`, and also a tab,
whose `SpaceSource::Lazy` is read through `ChunkStore::behind` with no
evaluator and no document (`Sheet::from_space` in the wasm crate). A
recognised tab has an `Evaluator` of its own, but over the tab's bytes, not
over the file the run is in. So the question is where a `Seekable` gets its
compressed bytes from at that point.

**A. A shared handle on the source.** The `Seekable` keeps a `RunInput`: an
`Arc` onto the source the run sits in, which for a run of the file is the
file's chunk store, shared, and where the run starts there. A span decode
reads its input through it. A chunk not loaded is reported as `Missing`, which
is already a chunk of the file, and flows to the web as every other missing
chunk does.

**B. Per-span compressed slices.** The pass copies each span's compressed
bytes into the index. Spans tile the run, so this keeps the whole run, cut
into pieces: the same memory as today, and the 32 MiB browser limit goes only
because no single read needs all of it. Keeping only some slices in a bounded
cache needs a way to read a dropped one again, which is A or C.

**C. The caller fills the input.** The `Seekable` keeps no handle. A read says
which compressed bytes it needs, and whoever holds a document reads them and
hands them in. The evaluator can. A tab cannot: every entry point of the wasm
crate that reads a lazy tab would have to catch the request, read the bytes
in the home sheet's reading and ask again. A recognised tab's own evaluator
reads its bytes deep inside an evaluation, so nothing can fill them before the
read, and its `Pending` would name chunks of the tab, not of the file.

**D. One whole copy, assembled a window at a time.** The pass reads its
windows from the source and appends each to a copy the `Seekable` keeps. This
is part 2. It fixes the browser case for runs up to 64 MiB compressed, with
the same memory as today, but keeps the 64 MiB limit and the memory.

| | A. shared handle | B. per-span slices | C. caller fills | D. whole copy |
|---|---|---|---|---|
| Compressed bytes held | none outside the chunk store | the whole run | a bounded cache | the whole run |
| A tab's reads, with no evaluator | read the source, report file chunks | work | need a catch in every wasm entry point | work |
| A recognised tab's own evaluator | `Pending` as for any read | works | cannot be filled in advance | works |
| Largest compressed run | 512 MiB (trace positions, see "Limits") | memory | 512 MiB | 64 MiB |
| What changes | `Source`, `ChunkStore`, `Document`, `Seekable` | the pass, `Seekable` | `Seekable`, the wasm crate | the evaluator |

**Recommended: A, with D as the fallback** for a run no handle can reach (see
"Where a RunInput comes from"). D is also the first step on the way (part 2),
since it fixes the files seen with no change to the decoders.

### A chunk pushed out while a synchronous read is under way

`Source::read_bytes` cannot wait, and does not need to. With A:

1. A span decode first reads all of the span's input through the `RunInput`,
   in one call, which reports every missing chunk of that input at once. If
   any is missing it decodes nothing.
2. It returns `ReadError::Missing(chunks)` and caches nothing for the span.
   Spans are cached only whole, so no span can look decoded when it is not.
3. `SpaceSource::read_bytes` fills `out` with zeroes and returns the chunks.
   The wasm crate's `read_bytes` passes them to the web, where `Doc.read`
   answers `complete: false` and `fetchChunk` loads each one as a chunk of the
   file. The hex view already draws an incomplete read as loading, and
   `notify` redraws it when the chunks land.
4. An evaluator reading the space through `read_in` gets
   `EvalError::Pending(chunks)` and asks again, as for any read of the file.
5. xz's `read_span` puts its part-decoded span (`lazy::xz::Prefix`) back in
   the cache when the read is missing chunks. Today it drops the prefix on any
   error, which is right for damage and wasteful here.

A read of many spans decodes them one after another. Spans decoded before a
miss stay in the span cache (16 MiB), so asking again does not decode them
again. One read of more than 16 MiB of output, whose spans together need more
compressed input than the store keeps, could still go round: the second ask
finds the first spans dropped from the span cache and their chunks pushed out
of the store. Nothing reads that much in one call except a check over one xz
block (`run_check`, up to 64 MiB); see "Open questions".

The input bounds in part 6 keep step 1 to at most 8 MiB and 64 KiB for a
deflate span, and 20 MiB for a zstd or xz span, under the store's 32 MiB.

## The decoders read a window

Part 4. No behaviour changes; every caller passes the whole run as one window
until part 7.

### Deflate

- **`Bits` gains a base.** `Bits` (`bits.rs`) reads `buf[at >> 3]`, with `at`
  counted from the first bit of `buf`. A new field gives how many bits of the
  run come before `buf[0]`. `at` stays relative to `buf`, so `bit` and `take`,
  the hot path, are unchanged. Only `pos()` adds the base, and the places that
  set `at` from a position of the run (`run`, `skip`, `stored`'s
  `bits.buf()[start..stop]`) take it off.
- **`inflate::run` takes an `Input`** in place of `data: &[u8]`: the window
  the codes are read from, and for a decode that starts inside a
  Huffman-coded block, a second small window holding that block's header and
  tables. `run` reads the header and tables from the second, and moves to the
  first where it moves to `Resume.in_bit` today (`skip`).
- **Why the header gets a window of its own.** A decode that starts inside a
  block reads the block's header and tables again from `Resume.block`, and a
  block may be as long as the run (zlib ends a block every 16,384 codes, but
  an encoder may write one block for a whole file). The header can be
  anywhere earlier in the run. So `Resume` and `Start::Deflate` keep a copy of
  the header: the bytes from the header's first byte, at most 287. A dynamic
  header with its tables is at most 2,286 bits (3 for BFINAL and BTYPE, 14 for
  HLIT, HDIST and HCLEN, 57 for 19 code-length code lengths, and at most 7 for
  each of 286 + 30 code lengths, since `dynamic_tables` refuses more), plus up
  to 7 bits before it in its first byte. 256 checkpoints with a copy each are
  under 72 KiB.
- **Where a decode stops before its window ends.** The decoder may stop at
  any place it calls `Sink::boundary`: the start of a block, and before every
  code of a Huffman-coded block, including the first (`symbols`). The longest
  stretch between two such places is a stored block: 3 header bits, up to 7
  padding bits, 32 bits of LEN and NLEN and up to 65,535 bytes, under 65,541
  bytes in all. A dynamic header with its tables is under 300 bytes, and one
  code is at most 48 bits. `Code::decode` reads a bit at a time, so the
  decoder never reads past the code it is decoding. So a decode given
  `stop_at` at least 65,541 bytes before the window's end stops at a boundary
  before it runs out, unless the window reaches the end of the deflate data.
  `DEFLATE_MARGIN` is that 65,541.
- **The wrapper.** `inflate::framing` takes the head of the run and the run's
  length instead of the whole run, and says how many more bytes it needs when
  a gzip name, comment or extra field runs past the head it was given: a gzip
  header has no length limit. `trailer_agrees` takes the trailer's 4 or 8
  bytes instead of the run.

### zstd

`ZstdWalk::run` (`codec/frames.rs`) slices `data[f.at + f.read..self.end]` and
hands it to `ruzstd`'s `decode_blocks` one block at a time. With a window,
before each block it reads the 3-byte block header from the window and checks
that the block, and the frame's 4-byte checksum after the last block, lie
inside the window. If they do not, and the window ends before the run does,
the walk stops there and says it needs input from that byte on. It does the
same before a frame header (at most 18 bytes) and before a skippable frame's
8-byte header; a skippable frame's contents are skipped without being read.
So a zstd step needs at most one block past where it stops: a margin of
128 KiB and 25 bytes. `ruzstd` keeps the frame's window itself between
blocks, so nothing else changes.

### xz

`codec::xz::layout` reads the stream header (12 bytes), the footer (12 bytes),
the index whose length the footer gives, and the header of every block (to
learn the header's length and whether `plain_lzma2` holds). It takes these
from a slice of the whole stream. Part 7 makes it a pass in steps; see "xz".

The LZMA2 decoder (`codec::lzma::Lzma2`) already decodes one chunk per call
and keeps its state between calls. A chunk is at most 64 KiB packed. It reads
its chunk from a slice of the block's LZMA2 data, which becomes a window with
a base. `lzma::lzma2_len`, which reads every chunk header of a block before
the block's first chunk is decoded, reads through the same window.

### `decode_step`

`inflate::decode_step` reads only the step's own bits; it rebuilds the block's
tables from the trace's head steps (`rebuild`), not from the run. It takes a
window and the byte the window starts at. The wasm crate's `decode_step` then
reads the bytes from `in_bits.start / 8` to `in_bits.end.div_ceil(8)` of the
run, at most 7 bytes, instead of `field_bytes` from the run's front. That is
part 3.

## The index pass reads through a `RunInput`

Part 7 for deflate and zstd; xz follows.

### `RunInput`

```rust
/// Where a run's compressed bytes are read from, for as long as anything
/// reads the run: a shared handle onto the source the run sits in, and where
/// the run starts there.
#[derive(Clone)]
pub struct RunInput {
    source: Arc<dyn Source + Send + Sync>,
    at: u64,
    len: u64,
}

impl RunInput {
    /// Bytes `at..at + out.len()` of the run, or every chunk of the file the
    /// read is missing, all at once.
    pub fn read(&self, at: u64, out: &mut [u8]) -> Result<(), Vec<Missing>>;
}
```

`Send + Sync` keeps `Seekable` `Send + Sync` as it is today. It lives in
`codec::lazy` and knows nothing of the evaluator: a space is wrapped as a
`Source`, which `SpaceSource` already is.

**Every `Missing` names a chunk of the file.** Every source a run can sit in
ends in the file's `ChunkStore`, and a space's own store never misses
(`ChunkStore::behind`). The web side already assumes this: `fetchChunk`
fetches any chunk a reply names as a chunk of the file, "whichever document
asks".

### Where a `RunInput` comes from

`Evaluator::run_input(doc, key)` makes one for the run at `key`:

| The run is in | The handle | Copy |
|---|---|---|
| The reading's own document (space 0), in a stretch no edit touched | `Source::share()` of the document's source, at `Document::source_offset_of(at, len)` | none |
| A space held whole | an `ArcSource` over the space's `Arc<Vec<u8>>` | none |
| A space read a piece at a time | the space's `SpaceSource::Lazy` | none |
| A stitched space, an edited stretch, or a source that cannot be shared | the run gathered whole (part 2), as an `ArcSource`, at most `CAP_BYTES` | the run |

- **`Source::share(&self) -> Option<Arc<dyn Source + Send + Sync>>`**, a new
  trait method that is `None` by default: a handle that reads the same bytes
  and outlives the borrow. `ChunkStore` gives a handle onto its chunks, which
  move into an `Arc<Mutex<…>>` with the LRU clock that is now in `Cell`s (the
  wasm build is single-threaded, so the lock is never contended). A store
  with a source behind it gives that source. `ArcSource` and `SpaceSource`
  give a clone of themselves. `MemSource` gives none: its field is public and
  there are 1,042 `MemSource(...)` constructions, and it holds the file in
  memory anyway, so the copy costs what it costs today.
- **`Document::source_offset_of(at_bits, n_bits) -> Option<u64>`**, new: the
  byte of the source a stretch of the document starts at, when the stretch
  lies in one piece of the original (`Piece` with `Src::Orig`) and starts on a
  byte there. An edit inside the run, or a bit-level insert before it, gives
  `None` and the fallback.
- **Edits.** Any edit drops every index (`Spaces::forget`), and the wasm
  crate's `ensure_open` opens a stale tab's stream again before answering
  anything about it. A handle made before an edit still reads the original
  file at the place it was given, never the edited bytes, and nothing reads
  through it once the edit is made.

### A step

A deflate step:

1. Works out the window it needs: from the byte holding the resume bit to
   `STEP_INPUT` (256 KiB) past it, plus `DEFLATE_MARGIN`, or to the end of the
   deflate data, whichever comes first.
2. Reads the part of the window it does not already hold through the
   `RunInput`. If chunks are missing it returns them and changes nothing.
3. Decodes with `stop_at` set `DEFLATE_MARGIN` before the window's end (none
   when the window reaches the end of the data), and with its output share as
   today, stopping at whichever comes first.
4. Keeps the bytes from where it stopped onward, and the copy of the current
   block's header, and drops the rest.

So a step reads at most about 320 KiB of new input, and the pass holds at
most that much. A step bounded by input as well as output also bounds a run
of empty blocks, which reads input and makes no output.

The first step also reads the run's last 8 bytes, once, and keeps them: gzip's
`ISIZE` decides whether `DeflatePass::new` starts as `known_large`, and the
trailer is checked at the end. And it grows its read of the head until
`framing` can parse the wrapper.

A zstd step does the same with the zstd margin. A frame whose compressed size
passes `Limits::unit_input` (16 MiB) is refused as `TooLarge`, the way a frame
whose output passes `Limits::span` is.

### How `Pending` fits the go machinery

`Indexing::step` gets a third answer beside `Working` and `Done`:
`NeedsInput(chunks)`. `index_pass` becomes:

```text
pass = the run's pass from Unfinished, or Indexing::new(codec, run_input, limits)
loop:
    charge INDEX_STEP_ELEMENTS to the go; if it ran out, put the pass back, Busy
    step INDEX_STEP_BYTES:
        Working        -> carry on
        NeedsInput(c)  -> put the pass back, return Pending(c)
        Done(decoded)  -> Ok(Ok(decoded))
        Refused(why)   -> Ok(Err(why)), kept in refused_runs
```

- `Indexing::new` reads nothing now, so the pass exists, and is in
  `Unfinished`, from the first ask on. `Evaluator::unpacking` answers
  throughout, waits for chunks included, and the listing shows
  `REPORT.unpacking` with a percentage instead of `REPORT.reading`.
- A missing chunk travels as `EvalError::Pending`, not as a `Refusal`, so it
  never reaches `refused_runs`.
- In the browser each wait loads one window's chunks, which are contiguous,
  so `Doc.settle` also starts reading the 48 after them. That is about one
  round trip for every 3 MiB of compressed run (an estimate from `READ_AHEAD`,
  not measured).
- Charging is unchanged. A step that finds its input missing has been charged
  for nothing, which a go can afford.
- Making the `RunInput` may itself wait, for the fallback copy. That read
  keeps its progress as in part 2.

### Checkpoints by compressed distance

`Checkpoints::offer` takes a checkpoint when the output since the last one
reaches `spacing`, or when the compressed input since the last one reaches
`in_spacing` (`IN_SPACING_START`, 1 MiB). Thinning doubles both. Without the
second rule a span's input has no bound: empty blocks are 10 bits each and
make no output, and a hostile run can be nothing else.

- A deflate checkpoint can go between any two codes, so a span's input is at
  most twice `in_spacing` plus one stored block. At the 512 MiB input limit,
  256 checkpoints are 2 MiB apart and `in_spacing` can double once more, to
  4 MiB, so a span needs at most 8 MiB and 64 KiB of input.
- A zstd or xz checkpoint is only ever at the start of a frame or block, so a
  span's input is at most `in_spacing` plus one frame or block: 4 MiB plus
  16 MiB.
- A span may now hold no output at all. `span_of_byte` already answers the
  last span starting at or before a byte, so such a span is never read for
  bytes; `span_of_bit` still finds it for the bits it covers.

When the output passes the cap, the pass goes back to the last checkpoint as
today. Its window is read again from that checkpoint's bit, at most
`in_spacing` back, whose chunks the step before had loaded.

### xz

Part 8. xz's layout becomes a pass in steps, so it joins `lazy::can_step` and
is charged and reports progress like the others:

1. The stream header and footer.
2. The index, whose length the footer gives. An index longer than
   `Limits::unit_input` is treated as one `layout` cannot read, which sends
   the stream to the whole decoder as today.
3. The block headers, up to 64 per step. Each is read where the index puts
   it, and the missing chunks of all 64 are reported together, so a stream of
   a thousand blocks spread over 300 MiB loads its headers in about 16 waits
   rather than a thousand.

A block whose compressed size passes `Limits::unit_input` is refused as
`TooLarge`. Until this part lands, an xz run is gathered whole (part 2) and
laid out from the copy as today.

## A `Seekable` reading through its `RunInput`

`Seekable::packed` goes; the `Seekable` keeps a `RunInput`. Each read gathers
its input first (see "A chunk pushed out while a synchronous read is under
way"):

| Read | Compressed input it needs at once |
|---|---|
| `decode_span(k)`, `deflate_span_trace(k)` | from checkpoint k's bit to checkpoint k+1's, and the header copy: at most 8 MiB and 64 KiB |
| zstd `decode_span`, `span_trace` | the span's frames: at most 20 MiB |
| xz `read_span` (`Prefix::reach`) | the LZMA2 data of the block it is in, read again on each later read of the span: at most 16 MiB |
| xz `span_trace` | the span's blocks: at most 20 MiB |
| `block(i)` | none: the pass records BFINAL and BTYPE with each kept block start, in the high bits of the start's bit, which is under 2^32 |
| `group(g)` | from kept block start g to g+1. Refused as `TooLarge` past `unit_input`, which only a hostile run reaches: more than 65,536 blocks with a long one among them |
| `block_trace(i)` | from the block's start until its trace turns coarse (`span_steps`, 1,000,000 codes, so at most 6 MB). The decode stops there and closes the block with one opaque step to the block's end, which the index knows. Today it decodes on to the end without recording, which is the same trace |

### Everything that has to treat `Missing` as "not yet"

Today these turn a failed read into a refusal, an empty answer or zeroes. Each
must pass `Missing` on instead:

| Where | Today | With `Missing` |
|---|---|---|
| `Seekable::read`, `span_trace`, `trace_at_byte`, `trace_at_bit`, `map_out`, `map_in`, `block`, `block_of_bit`, `block_trace` | `Refusal`, or `None` | `ReadError::Missing(chunks)` beside `ReadError::Refused(Refusal)` |
| `read_in`'s arm for a space read a piece at a time | `fail("unpacking failed")` | `EvalError::Pending` |
| `read_stitched` and `part_index` through `index.read` | `part_refusal` | `EvalError::Pending` |
| `SpaceSource::read`, `read_bytes` | zeroes, no chunks reported | zeroes, the chunks reported |
| `Evaluator::template_for`, which sniffs the first 36 KiB | sniffs whatever came back | answers `Pending` before sniffing, or it sniffs zeroes |
| xz `read_span` | drops the prefix | keeps it |
| `Joined::read`, `map_out`, `map_in`, `part_trace` | `Refusal::Failed`, or `None` | `Missing` |
| `run_check` reading one xz block through the index | `fail(refused(why))` | `EvalError::Pending` |
| `traced::Blocks::get`, `at_bit`, `trace_of` | `Option` | `R<Option<_>>` |
| `Space::map_out`, `map_in`, `trace_at_bit` | `Option` | a `Result` whose error is the missing chunks; the wasm crate replies "pending" |
| `index_run`'s `refused_runs` | keeps refusals | never sees a missing chunk, which is an `EvalError` |

## Measuring a gzip member

`stream_len_at` reads up to 64 MiB in one `read_in`. Part 2 gathers that read
a window at a time, which fixes .gz files up to 64 MiB compressed. Part 9
replaces the gathered copy with a pass of its own, `lazy::Measuring`: the
`CountSink` decode `inflate_len` makes, done a window at a time through a
`RunInput` over the stretch from the run's start to the end of its window. Its
state is the resume point and the count. It is kept between goes in a small
table in `Spaces` beside `Unfinished` (two entries, keyed by space, offset and
end), charged per step like the index pass, and reported by
`Evaluator::unpacking`. Its output cap stays 64 MiB, for the reason
DESIGN-lazy-unpacking gives: a larger one would let a hostile file cost the
full 1 GiB walk twice. This also does the "Still to do" item there about
`inflate_len` being resumable.

A .gz then makes two passes in the browser, one to size the member and one
to index `decoded`'s part, and the listing shows the unpacking percentage go
to 100 twice. That is what happens; the first pass is simply not shown today.

## Joined streams

Part 10, after the handle.

- `JoinedBytes::Held(Arc<Vec<u8>>)` becomes `JoinedBytes::Stored(RunInput)`.
- `JoinedBytes::Packed { run, codec }` becomes `Packed { input, codec }`. The
  run is read through the handle when the part is decoded; a BGZF part is
  under 64 KiB.
- `JoinedBytes::Indexed` keeps its `Seekable`, which reads its own input.
- `join_lazily`'s 64 MiB limit on what it reads from goes.
- `join_lazily` checks that every small packed part unpacks to the length its
  run claimed, by decoding every part in one call, uncharged: a gigabyte BAM
  is a gigabyte of inflate in one go. It becomes a walk kept across goes in
  `Spaces`, keyed by the stream's path and holding the parts made so far,
  charged per part like an index step and reported by
  `Evaluator::unpacking`. The alternative is to check each part the first
  time it is read, which `Joined::read` already does. That is cheaper, but the
  tab would open over a part that is wrong and show zeroes there.

Two changes need no handle and come earlier, in part 2:

- `join_whole`, for a stream of at most 64 MiB, reads every part in one call.
  It becomes a walk kept across goes too, holding the bytes and the trace made
  so far, as it holds them now.
- `unpacked_part` reads a packed part whole through `unpack_run`. For a codec
  that can be indexed it calls `index_run`, which steps; for any other, the
  read is gathered.

## Limits

| Limit | Value | Status |
|---|---|---|
| `LAZY_CAP_BYTES` | 1 GiB of output | Stays. The zip-bomb guard. |
| `CAP_BYTES`, as the output of a run held whole | 64 MiB | Stays. |
| `CAP_BYTES`, as the compressed size of a run that can be indexed | 64 MiB | Replaced by `Limits::input`, 512 MiB. |
| `CAP_BYTES`, as the input and output of a codec that cannot be indexed | 64 MiB | Stays. The input is gathered a window at a time (part 2). |
| `inflate_len`'s output cap | 64 MiB | Stays. |
| `Limits::span` | 64 MiB of output per zstd frame or xz block | Stays. |
| `INDEXED_RUN_BYTES` | 64 MiB of compressed runs | Weighs each index by what it keeps for as long as it lives (windows, header copies, block starts: at most about 9.1 MiB), with the same 64 MiB total. The caches are bounded per index as today. |
| `whole_cap`, as the input of a joined document read a part at a time | 64 MiB | Goes. |
| `Limits::input` (new) | 512 MiB of compressed input | A trace keeps each step's bit in 32 bits (`RawStep.in_start`; `pack` truncates silently), and 2^32 bits is 512 MiB. Refused as `TooLarge` before anything is read, since the field's size is known. |
| `IN_SPACING_START` (new) | 1 MiB of compressed input between checkpoints | Doubled with `SPACING_START` when checkpoints are thinned. |
| `Limits::unit_input` (new) | 16 MiB of compressed input per zstd frame or xz block | Keeps a zstd or xz span's reads under 20 MiB. A frame or block over it is refused as `TooLarge`. |
| `STEP_INPUT` (new) | 256 KiB of new compressed input per step | Beside `INDEX_STEP_BYTES`, 256 KiB of output. |
| `DEFLATE_MARGIN`, the zstd margin (new) | 65,541 bytes; 128 KiB and 25 bytes | How far past where a step stops its window must reach. |
| `MAX_HEAD_BYTES` (new) | 287 | A deflate block header and its tables, from its first byte. |
| `GATHER_WINDOW` (new) | 1 MiB, or a quarter of what the store keeps when that is less, and at least one chunk | Part 2. |

**The zip-bomb guard.** Memory stays bounded by the table above and
DESIGN-lazy-unpacking's, so a hostile run can only cost time. That is now one
pass over at most 1 GiB of output and at most 512 MiB of input, each read
once, a window at a time.

**`unit_input` is a choice.** It refuses a file that opens natively today: a
stream of at most 64 MiB compressed and more than 64 MiB unpacked, with one
frame or block of more than 16 MiB compressed. `xz -T0 -6` writes 24 MiB
blocks, so data that packs at less than 1.5 to 1 with it is refused. A larger
value leaves less of the 32 MiB store for everything else a span read
competes with. Lifting it needs a span decode that keeps its decoder between
reads; see "Still to do".

## The browser

### What the listing and the status bar say

| While | The listing's waiting row | The status bar, after Open unpacked |
|---|---|---|
| a run is gathered (part 2) | `REPORT.reading`, "Loading bytes needed to map these fields…", which is now true and ends | `UNPACKED.loading` |
| the index pass runs, or waits for chunks (part 7) | `REPORT.unpacking`, "Unpacking the compressed data to read these fields… 37%" | `UNPACKED.unpacking`, on "pending" replies as well as "working" ones |
| a read can never fit (part 1) | the node's refusal, in its value column | the refusal, as `JOINED.refused` and `DECODED_REFUSED` say it |

No new string is needed for the first two rows. A percentage while a run is
gathered would help for a 64 MiB run; that string should be drafted with the
`ui-text` and `no-riddlespeak` skills when part 2 is built.

### `waitToOpen`

`waitToOpen` (`openwait.ts`) gives up when a chunk it loaded is asked for
again with no "working" reply in between. Once the pass reads its input in
windows, a wait can see many "pending" replies in a row with unpacking done
between them, and a chunk asked for again then (for example the last
checkpoint's window after the pass goes back at the cap, when something else
pushed it out) is not the store being too small. So:

- `Doc.openSpace` returns the unpacking progress with a "pending" reply as
  well, and `waitToOpen` clears `loaded` whenever the progress has moved
  since the last reply, not only on "working".
- The status bar shows `UNPACKED.unpacking` on a "pending" reply while a pass
  is under way, so the line does not switch between loading and unpacking on
  every round.
- `MAX_ROUNDS` counts rounds with no progress. A 512 MiB run takes about 170
  rounds at one per 3 MiB, under the 512 there are now, but a count of rounds
  without progress is the backstop that was meant.

These land with part 7 or before it.

### `ensureRange` on a tab

`Doc.ensureRange` loads every chunk `has_chunk(this.space, c)` says is absent,
and for a tab that is never: a space's store answers true for every chunk. So
on a tab over a run read a piece at a time it resolves at once and the read
after it comes back incomplete. For a space other than 0, `ensureRange` should
read the range and wait on `fileChunks` of the chunks the read names, until
the read is complete, giving up by the same rule as `waitToOpen`. A read's
missing chunks are exactly the file chunks it waits on, so no new query of the
core is needed.

### The smaller fix first (part 1)

**Worth doing first: yes.** It is about half a day, it ends the endless wait
in every case at once (the zip entry, the .gz, a bzip2 or LZMA run, a check
over a long run, a joined stream), and its check in `read_in` stays as the
guard for any read that later parts do not convert. After part 2 no
compressed run reaches it, so its refusal becomes rare, and that is fine.

- `Source::resident_limit(&self) -> Option<u64>`, new: the most bytes the
  source holds at once. `ChunkStore` without a source behind it answers its
  capacity times its chunk size; every other source answers `None`, and
  `Document` passes on its source's answer.
- `read_in`, for space 0, fails a read that touches more chunks than the
  store keeps, with the message below, instead of answering `Pending`.
- Before reading a run whole, `index_pass`, `open_space_at`'s arm for codecs
  that cannot be indexed, `unpack_run`, `join_whole` and `join_lazily` refuse
  it with a new `Refusal::TooMuchAtOnce` (tag `too-much-at-once`), kept like
  any refusal. A refusal shows on the node, where a failed read would leave
  the node with nothing under it (`flatten.ts` draws an error reply for
  children as no children).
- `stream_len_at` measures nothing for such a run, as for a run past the cap,
  so the gzip template's fallback places the member as everything before its
  trailer.
- On the web side, `DECODED_REFUSED` and `JOINED.refused` get the new tag.
- What it does not catch: a read that fits alone but not beside the other
  chunks a go needs can still go round. `waitToOpen`'s rule stays for that.

**Proposed strings.** Drafted by a fresh agent from a brief, with no code in
its context, and checked against the code since. They are proposals for
sign-off.

| Where | Proposed string | Note |
|---|---|---|
| Value column of a compressed field, the inspector after ` · `, and `{field} is …` for a joined part (`Refusal::message` and `DECODED_REFUSED["too-much-at-once"]`) | `too large for the editor to keep in memory at once` | The 32 MiB goes in the row's tooltip, for example "The editor keeps at most 32 MiB of the file in memory." If no tooltip is possible: `too large for the editor to keep in memory at once (limit 32 MiB)`. |
| The failure of any other read that cannot fit (`read_in`) | `Can't load {needed} of the file at once: the editor keeps at most 32 MiB in memory` | `{needed}` is the read's size, like `40 MiB`. Avoids the verb "read", which would be wrong for a checksum field whose own bytes are readable. |

The drafting agent also pointed out that the existing `too large to unpack
(over 64 MiB)` does not say what is over 64 MiB, and that beside a 32 MiB
phrase it can read as one limit with two values. `Refusal::TooLarge` is given
today both for a compressed run over 64 MiB and for an output over 64 MiB, and
with this design also for a run over 512 MiB compressed. See "Open
questions".

## Memory

For a run read a piece at a time, besides DESIGN-lazy-unpacking's table:

- the pass's window: at most about 320 KiB for deflate and 384 KiB for zstd,
  while the pass runs;
- the header copies: under 72 KiB per run;
- the trailer: 8 bytes;
- what a span read gathers: at most 8 MiB and 64 KiB for deflate and 20 MiB
  for zstd or xz, for the length of the read;
- what a block trace gathers: at most 6 MB, for the length of the trace.

The compressed bytes otherwise stay in the file's chunk store, 32 MiB in the
browser, which the file's tab keeps anyway. A run that falls back to a copy
(an edited run, a run in a stitched space, a `MemSource`) holds that copy, at
most 64 MiB, as every run does today. Gathering (part 2) holds at most 64 MiB
of partial copies between all runs, dropping the one read longest ago first.

## Order of work

Each part builds, passes its tests and can be merged on its own, in this
order.

1. **Refuse a read that can never fit.** `Source::resident_limit`, the check
   in `read_in`, `Refusal::TooMuchAtOnce` before the whole-run reads,
   `stream_len_at` measuring nothing, the web strings. Fixes: the endless
   wait, everywhere.
2. **Gather a run a window at a time.** `Evaluator::gather_run`: a run read
   whole is read `GATHER_WINDOW` at a time through `read_in`, into a copy kept
   in `Spaces` between goes (at most 64 MiB of partial copies), charged per
   window. Used by `index_pass`, `open_space_at`, `unpack_run`,
   `stream_len_at` and `covered_bytes`. `join_whole` becomes a walk kept
   across goes, and `unpacked_part` steps through `index_run` for codecs that
   can be indexed. Fixes: the 34 MiB zip entry and a .gz up to 64 MiB
   compressed in the browser, with memory as today.
3. **`decode_step` reads only its step.** Needs only the window for
   `decode_step` from part 4, so it can come with part 4 or right after it.
   Fixes: the code panel on a step deep in a long run.
4. **Decoders read a window.** `Bits` with a base, `inflate::run` taking an
   `Input`, `framing` and `trailer_agrees` taking pieces, `ZstdWalk` and
   `Lzma2` on windows. Every caller passes the whole run. No behaviour change.
5. **`Missing` as "not yet".** `RunInput`; the `Seekable` keeps one, made
   over its copy (an `ArcSource`), so it never misses yet; `ReadError`; every
   row of the table in "Everything that has to treat `Missing` as 'not yet'".
   No behaviour change.
6. **Bound what a `Seekable` read needs.** Checkpoints by compressed
   distance, header copies, BFINAL and BTYPE with the block starts, block
   traces that stop when coarse, `group`'s bound, `Limits::unit_input`.
   Changes the spans of runs that pack badly; their bytes and traces stay the
   same.
7. **Share the source.** `Source::share`, `ChunkStore`'s chunks behind an
   `Arc<Mutex<…>>`, `Document::source_offset_of`, `Evaluator::run_input`;
   the deflate and zstd passes read their windows through the `RunInput` and
   answer `NeedsInput`; `index_pass` keeps the pass across `Pending`;
   `Limits::input` replaces the 64 MiB compressed limit in the five places;
   `Indexes` weighs fixed parts. On the web side, `waitToOpen`, the status
   line and `ensureRange` as in "The browser". Fixes: runs over 64 MiB
   compressed, and the memory of every run with a handle.
8. **xz's layout in steps.** xz joins `can_step`.
9. **Measure a gzip member in steps.** `lazy::Measuring` replaces the
   gathered copy in `stream_len_at`.
10. **Joined streams through handles.** `JoinedBytes` over `RunInput`,
    `join_lazily` without its input limit and walked across goes.
11. **Bring DESIGN-lazy-unpacking up to date.** Its "State", the "Limits"
    row for `CAP_BYTES`, the paragraph at the end of "The browser" and
    "Still to do".

Parts 1 and 2 fix both files seen. Parts 4 to 7 are the substance of the
design and make the most change; part 7 is the one that removes the 64 MiB
limit and needs parts 4 to 6.

## Tests

### The core (`codec::lazy`, small limits)

- **Windows.** For each run in the existing pass-in-steps tests, a pass whose
  window is every size from the margin plus one byte up to the whole run
  comes to the same checkpoints, block starts, sums, bytes and, under the
  cap, the same trace step for step as the pass over the whole run.
- **A source that loses chunks.** A test `Source` over a vector, with a set
  of chunks it holds: a read reports the rest missing and fills them with
  zeroes. The pass, fed only the chunks it reports, comes to the same index.
  It never reports more chunks in one answer than its window covers. With a
  source that drops every chunk once it has been read, it still finishes,
  since a step needs only its own window.
- **Span reads over that source.** A read reports exactly the chunks of its
  span's input, all at once; after they are fed it returns the right bytes;
  a span is never cached from a read that was missing chunks; an xz prefix
  survives a read that was missing chunks.
- **Compressed distance.** Runs of empty fixed blocks, of stored blocks and
  of incompressible data: no span's input is over the bound, thinning keeps
  both spacings, and a span of no output is never read for bytes.
- **Block traces.** A block of more than `span_steps` codes: the trace
  stops decoding at the coarse point, reads no further input, and equals the
  trace made by decoding to the block's end.
- **Wrapper.** A gzip with a 1 MiB comment and a gzip with a 100 KiB extra
  field index with 64 KiB windows; a truncated gzip header is still refused.
- **Limits.** With `Limits::input` small, a longer run is refused as
  `TooLarge` before anything is read. A zstd frame and an xz block over
  `unit_input` are refused; a `group` over it is refused.
- **zstd and xz windows.** Windows that end inside a block, a frame header, a
  checksum and a skippable frame's header. xz's layout in steps reads block
  headers 64 at a time, still refuses a delta or branch filter, and sends an
  index over `unit_input` to the whole decoder.

### The evaluator (`eval::space`, `eval::stitch`)

- A zlib run past the small cap, in a `Document<ChunkStore>` with a store
  smaller than the run and fed only the chunks each `Pending` names, opens.
  `unpacking` moves forward on every answer, waits included, and the index is
  the one a `MemSource` gives.
- `Document::source_offset_of`: no edits; an insert before the run; an
  overwrite inside it (the fallback copy); a bit-level insert before it (the
  fallback copy). An edit drops the index and its handle.
- Gathering (part 2): a run read through a store of four chunks comes to the
  same bytes as one `read_in`; a gather interrupted by `Busy` and by
  `Pending` carries on from where it stopped.
- Joined: a BGZF stream joined through handles reads the same bytes as with
  copies; the walk that checks part lengths carries on across goes and
  refuses a part of the wrong length as now.

### The wasm crate (`crates/wasm/tests`)

These follow `open_space_waits.rs`: an `Editor` with a small store, fed only
the chunks each "pending" reply names, asked again on "working".

- **The stuck listing, reproduced.** A zip of one deflated entry whose
  compressed size is several times the store (for example a store of 16
  chunks and an entry of 4 MiB compressed, made from enough lines of hex, as
  `lines()` makes them, to pack to that). The test asks `template_children`
  for the entry's `data` and its contents, the way the listing does.
  - Today: a chunk is asked for again after it was fed, within a few rounds,
    and the contents never come. The test asserts this first, so the case is
    known to be reproduced.
  - After part 1: the `data` node is refused with `too-much-at-once` within a
    few rounds.
  - After part 2 (with the store big enough for one `GATHER_WINDOW`) and
    after part 7: the contents read within a bounded number of rounds, and no
    reply names more chunks than one window covers.
- **The .gz.** A gzip of one member larger than the store, read with the
  gzip template. Today `members[0].compressed` never gets a size. After part
  2, and again after part 9, the member is sized, `decoded` reads, and Open
  unpacked gives a tab.
- **A chunk pushed out and fetched again.** On that tab, `read_bytes` at the
  start, then at the end, which pushes the start's chunks out, then at the
  start again: the third read reports the start's chunks as missing, and
  after they are fed returns the same bytes as the first.
- **A recognised tab.** A gzip of a tar larger than the store: the tab's own
  reading reads the tar's first member's name, with "pending" replies that
  name chunks of the file.
- **`decode_step`.** A bit near the end of a 4 MiB run: the "pending" reply
  names one chunk, not the run's.
- **The existing test that asserts the old behaviour.**
  `a_run_longer_than_the_store_asks_again_for_chunks_it_was_fed` keeps a
  store of 3 chunks and a run of 7, and asserts that a chunk is asked for
  again. After part 1 it asserts the refusal instead, and after part 2, with
  a gather window of one chunk, that the run opens. Its name changes with
  it. After part 7 a deflate step needs at least 3 chunks at once (a window
  must reach `DEFLATE_MARGIN` past where it stops), so tests of the pass use
  stores of 16 chunks or more.

### The browser

A new `web/test/lazyinput.browser.mjs`, run by hand like the other
`*.browser.mjs` scripts (global Playwright through `PLAYWRIGHT_MODULE`, a
`TEST_URL` that says `localhost`). Like `hexload.browser.mjs` it makes its
files in Node: a zip with one entry of 40 MB of lines packed with
`zlib.deflateRawSync`, and a gzip of the same lines over 32 MiB compressed.
It drops each on the page as a `File` with a synthesised `drop` event and
checks that:

- the listing shows `REPORT.unpacking` with a growing percentage, never
  `REPORT.reading` for more than a few seconds, and then the entry's fields;
- Open unpacked opens a tab, and the hex view at the tab's end and back at
  its start shows the right bytes;
- the wasm memory (`WebAssembly.Memory.buffer.byteLength`) stays under a
  bound well below the compressed size plus 64 MiB;
- the code panel answers for a byte near the tab's end.

Before part 1 the same script shows the stuck row, which is the reproduction
in the browser.

## Still to do

- **Lift `unit_input`.** A zstd span decode could keep its frame decoder
  between reads, as xz's `Prefix` keeps its LZMA2 decoder, and xz's
  `lzma2_len` could read a block's chunk headers a window at a time with its
  progress kept in the `Prefix`. The traces of zstd and xz spans would still
  need the whole span's input at once.
- **One read of more than the span cache.** See the end of "A chunk pushed
  out while a synchronous read is under way". `run_check` over one xz block
  could sum the block a span at a time instead of reading it whole.
- **Runs past 512 MiB compressed.** Trace positions counted from the trace's
  own first bit (`Trace::start`) instead of from the run's front would lift
  the limit to 512 MiB per span.
- **`UNFINISHED_PASSES`.** Two passes are kept, because one in its traced
  phase holds up to 64 MiB. A pass past the cap holds only its windows and
  checkpoints, so more of those could be kept; a listing showing three large
  zip entries at once starts the third pass again each time today.
- **`CHUNK_CAPACITY`.** A larger store would let `unit_input` grow. That is a
  choice about the browser's memory, not part of this design.

## Open questions

- Should `TooLarge` be split, so that the listing can say whether the
  compressed run or the output is too large? With this design the two limits
  are 512 MiB and 64 MiB (or 1 GiB), and one phrase cannot say both.
- Is the length check in `join_lazily` worth a walk across goes, or should a
  part of the wrong length show as a failed read when it is read?
- Should gathering report a percentage, and with what string?

## What was checked

Read in full or in the parts named, on the branch as merged with main on
2026-11-26:

- `docs/DESIGN-lazy-unpacking.md`, in full.
- `codec/lazy/mod.rs`, `deflate.rs`, `seekable.rs`, `zstd.rs` and `xz.rs`, in
  full: every use of `packed`, the checkpoint and block-mark thinning, and
  the size checks.
- `codec/inflate.rs`: `framing`, `trailer_agrees`, `Sink`, `CountSink`,
  `run`, `skip`, `stored`, `dynamic_tables`' field limits, `symbols`' calls to
  `boundary` and its coarsening, `Code::decode`, `decode_step` and `rebuild`,
  `inflate_len`.
- `bits.rs`: `Bits` and `gather`. `codec/frames.rs`: `ZstdWalk`.
  `codec/xz.rs`: `layout`. `codec.rs`: `RawStep`, `pack`, `Refusal`,
  `stream_len`, `decode`.
- `eval/space.rs`, in full: `Space`, `SpaceSource`, `Joined`, `JoinedBytes`,
  `Backing`, `Indexes`, `Unfinished`, `index_run`, `index_pass`.
- `eval/stitch.rs`: `measure_run`, `index_part`, `unpack_run`,
  `read_stitched`, `unpacked_part`, `join_whole`, `join_lazily`.
- `eval/mod.rs`: `EvalError`, `open_space_at`, `template_for`.
  `eval/read.rs`: `read_in`. `eval/expr.rs`: `stream_len_at`.
  `eval/check.rs`: `run_check`, `covered_bytes`. `eval/traced.rs`: `Blocks`.
  `eval/go.rs`: `Go::charge`.
- `source.rs`, in full. `document.rs`: `read_bits`, `read_bytes`, the
  `Source` impl. `piece.rs`: `Piece`, `PieceTable`. The count of
  `MemSource(` constructions.
- `formats/gzip.rs`: how `members[i].compressed` is sized. `formats/zip.rs`:
  `data_len`.
- `crates/wasm/src/lib.rs`: `Sheet::from_space`, `ensure_open`, `open_in`,
  `open_space`, `unpacking`, `decode_step`, `read_bytes`, `feed_chunk`,
  `has_chunk`, `WORK_SLICE`, `SPACE_CHUNK`.
- `crates/wasm/tests/open_space_waits.rs` in full, and the helpers of
  `cursor_link.rs`.
- `web/src/doc.ts`: `CHUNK_CAPACITY`, `READ_AHEAD`, `settle`, `openSpace`,
  `read`, `fetchRun`, `fetchChunk`, `fileChunks`, `ensureRange`.
  `web/src/openwait.ts` in full. `listingdraw.ts`: `pendingText` and the
  refusal in the value column. `flatten.ts`: how a pending or failed reply is
  drawn. `strings.ts`: `DECODED_REFUSED`, `UNPACKED.loading`, `unpacking`,
  `tooMuchAtOnce`, `REPORT.reading`, `REPORT.unpacking`. `hexview.ts`: how an
  incomplete read is drawn.

Not measured: no timing or memory figure in this document comes from a run.
The round-trip estimate is arithmetic from `READ_AHEAD` and the chunk size.

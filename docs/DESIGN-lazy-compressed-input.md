# Design: reading the compressed input of large runs a piece at a time

Written 2026-11-26, for developers. This is a proposal. Parts 1 and 2 are
built (2026-11-26; "The smaller fix first (part 1)" and "Gathering a run a
window at a time (part 2)" say where the code differs from this text), and
parts 3 to 11 are not. It follows
`docs/DESIGN-lazy-unpacking.md`, which describes how a compressed run whose
output is past `codec::CAP_BYTES` (64 MiB) is indexed in steps and its output
read a span at a time. This design does the same for the compressed input, so
that memory no longer grows with the compressed size and the browser can open
a run whose compressed bytes do not fit in its chunk store.

"Still to do" and "Open questions" at the end list what this leaves open.
"What was checked" lists the code read to write it.

## Terms

Most of these come from DESIGN-lazy-unpacking or the code.

- **Run**: the compressed bytes of one `Decoded` field, or of one part of a
  joined stream.
- **Indexed run**: a run of deflate, zlib, gzip, zstd or xz whose output is
  past 64 MiB, read through a `Seekable` instead of being held whole. The
  output of an indexed run is already read a piece at a time; this design is
  about its compressed input.
- **Index pass**: the pass that makes a `Seekable` (`lazy::Indexing`). It runs
  in **steps** of about 256 KiB of output.
- **Checkpoint**: a place in a run where a decoder can start again. A
  **span** is the output from one checkpoint to the next. When a run has more
  than `MAX_CHECKPOINTS` (256), every second one is dropped and the spacing
  doubles; that is **thinning**.
- **The cap**: `CAP_BYTES`, 64 MiB.
- **Go**: one call into the core with an allowance of work (5,000 elements in
  the web app, `WORK_SLICE`). A go that runs out answers `Busy`, and the next
  call carries on.
- **Round**: in the browser, one "pending" reply, the chunks it names being
  loaded, and the call made again.
- **Chunk**: 64 KiB of the file in the `ChunkStore`. An LZMA2 chunk, which is
  something else, is always called an LZMA2 chunk here.
- **Window**: the stretch of a run's compressed bytes that a decoder is given
  at once. zstd's own history window is called the zstd history window.
- **Reading**: an `Evaluator` over one document. The file has one. A
  **recognised tab** (a stream whose bytes were sniffed as a format, such as
  a tar inside a gzip) has one of its own, over the tab's bytes; its **home
  sheet** is the sheet whose reading opened it.
- **Joined stream**: a `Stitched` field opened as a document, read a part at a
  time through a `Joined`. Its pieces are called a stream's parts here; the
  parts of this plan are called "part 1", "part 2" and so on.
- **Coarse**: a trace past its step budget, which names a block's codes as one
  opaque step instead of one step per code.

## The problem

The memory an indexed run takes for its output is bounded by the table in
DESIGN-lazy-unpacking's "Limits": checkpoints, span cache, traces. The memory
it takes for its compressed input is not:

- `Evaluator::index_pass` (`eval/space.rs`) reads the whole run with one
  `read_in` before its first step, and hands it to `lazy::Indexing::new` as an
  `Arc<Vec<u8>>`.
- The `Seekable` the pass makes keeps that vector (`Seekable::packed`). Every
  span decode, span trace, block trace, search for dropped block starts
  (`Seekable::group`) and `Seekable::block` reads from it.
- `Indexes` keeps up to 64 MiB of these vectors in total
  (`INDEXED_RUN_BYTES`), weighed by `Seekable::packed().len()`.
- `Joined` keeps a copy of every stored part and of every packed part's run
  (`JoinedBytes::Held`, `JoinedBytes::Packed`), up to `whole_cap` (64 MiB) in
  total (`Evaluator::join_lazily`).
- So an indexed run may be at most 64 MiB compressed. Five places refuse
  past that: `open_space_at` (`size / 8 > CAP_BYTES`), `index_pass`
  (`key.2 / 8 > CAP_BYTES`), `decode_or_index_with`, `Indexing::new` and
  `DeflatePass::new` (`packed.len() > CAP_BYTES`).

In the browser the limit is lower. A `read_in` of the file is answered only
when every chunk it touches is in the file's `ChunkStore` at the same time.
The store keeps 512 chunks (`CHUNK_CAPACITY` in `web/src/doc.ts`), 32 MiB, so
a read of more than 32 MiB is never answered:

1. `read_in` answers `EvalError::Pending` with every chunk it is missing.
2. `Doc.settle` fetches them, and after a run of contiguous misses it also
   starts reading the 48 chunks that follow (`READ_AHEAD`).
3. Each chunk fed in pushes out the chunk read longest ago
   (`ChunkStore::insert`), which is one this read needs.
4. The next call of `read_in` answers `Pending` again, for the chunks pushed
   out.

Two files have shown this endless wait:

- **A zip with one deflated entry of 34 MiB compressed** (seen on
  2026-11-26). The entry's `data` is a `Decoded` field whose size the local
  header gives. When the listing asks what is inside it, `open_space_at` calls
  `index_run`, and `index_pass` reads all 34 MiB with one `read_in`. The pass
  is never made, so `Unfinished` is empty, `Evaluator::unpacking` answers
  nothing, and `pendingText` (`listingdraw.ts`) shows `REPORT.reading`,
  "Loading bytes needed to map these fields…", for as long as the file is
  open. The Open unpacked button stops after a round or two with
  `UNPACKED.tooMuchAtOnce`, because `waitToOpen` sees a chunk asked for again.
- **A .gz of more than 32 MiB compressed** waits earlier. The gzip template
  gives `members[i].compressed` its size with `E::stream_len(Codec::Deflate)`,
  and `Evaluator::stream_len_at` (`eval/expr.rs`) reads everything from the
  start of the run to the end of the file, up to 64 MiB, with one `read_in`,
  to hand to `inflate::inflate_len`. The member never gets a size, so none of
  the fields under it can be read, and `index_run` is never reached.

Every other read that takes a run in one call waits for ever the same way past
32 MiB:

- a run of a codec that cannot be indexed (`open_space_at` reads it with
  `self.read` before `decode_traced_with`);
- a packed part of a stitched stream (`Evaluator::unpack_run`);
- `join_whole` and `join_lazily`, which read every part in one call;
- a checksum check over a run of the file (`Evaluator::covered_bytes`);
- the code panel: the wasm crate's `Editor::decode_step` reads the run from
  its front to the end of the trace step (`field_bytes` with `want_bytes`),
  so a trace step near the end of a 34 MiB run waits on 34 MiB.

### The rule

Every one of these breaks the same rule, and each part of this design applies
the rule to one piece of code that reads:

> One synchronous call must not need more than a bounded amount of the file
> at once, well under what the chunk store keeps. Work that needs more is done
> in steps, and each step keeps its progress across a `Pending` as the index
> pass already keeps it across a `Busy`.

## What changes for the reader

- In the browser, a run of a codec that can be indexed opens up to 512 MiB
  compressed and 1 GiB unpacked: a deflated zip entry, a gzip, zlib or raw
  deflate run, a zstd run of several frames or an xz stream of several
  blocks. One zstd frame or xz
  block may be at most 16 MiB compressed (see "Limits"). A run of a codec
  that cannot be indexed (bzip2, LZMA and the rest) opens up to 64 MiB
  compressed, as it does natively today.
- While such a run is read, the listing says how far the unpacking has got,
  as it already does for an indexed run whose compressed bytes are in memory.
- A tab over an indexed run reads the file as it goes. A part of the tab
  whose compressed bytes have been pushed out of the store shows as loading,
  as a part of the file's own tab does, and is filled in when the chunks
  arrive.
- Memory for an indexed run is what DESIGN-lazy-unpacking's table bounds,
  plus the bounded additions listed in "Memory", whatever the run's
  compressed size. The compressed bytes stay in the file's chunk store, which
  the file's tab uses anyway.
- A read that still needs more of the file at once than the store keeps is
  refused with the reason, instead of waiting for ever.

## Recommendation

- **Refuse a read that can never fit (part 1).** A read that needs more
  chunks at once than the store keeps fails with a reason instead of
  answering `Pending`. It is small, and the check stays as the guard for any
  read the later parts leave as it is.
- **Gather a run a window at a time (part 2).** A run that is read in one
  call today is read a window at a time across goes, into a copy kept from
  one go to the next, with memory as today. This alone fixes both files
  above, up to 64 MiB compressed.
- **Decoders read a window (part 3).** Every decoder reads from a window of
  the run, with positions still counted from the run's front. It stops at a
  place it can carry on from, before the window runs out.
- **A run reads its input through a handle (parts 5 to 7).** A run's
  compressed bytes are read through a `RunInput`: a shared handle onto the
  source the run is in, with where the run starts there. The index pass reads
  its windows through it, and the `Seekable` keeps it to read spans later. A
  read that finds chunks missing reports them instead of waiting.
- **A missing chunk is never a refusal.** It travels as `Missing` up to
  `read_in`, which answers `EvalError::Pending`, and up to
  `SpaceSource::read_bytes`, which reports it as a chunk of the file. The web
  side already fetches every chunk a reply names as a chunk of the file.
- **Every read of a `Seekable` has a bounded compressed input (part 6).**
  Checkpoints are taken by compressed distance as well as by output, a block
  trace stops decoding once its trace has turned coarse, a zstd frame or xz
  block over 16 MiB compressed is refused as too large, and a zstd or xz span
  read that would need more than 16 MiB at once is refused when read.

"Order of work" splits this into parts that build and merge on their own.

## Reading a span later, with no evaluator

The index pass runs in the evaluator, which has the document. A span is read
later by whatever holds the `Seekable`: the evaluator's `read_in`, and also a
tab, whose `SpaceSource::Lazy` is read through `ChunkStore::behind` with no
evaluator and no document (`Sheet::from_space` in the wasm crate). A
recognised tab has an `Evaluator` of its own, but over the tab's bytes, not
over the file the run is in. So the question is where a `Seekable` gets its
compressed bytes from at that point.

**A. A shared handle on the source.** The `Seekable` keeps a `RunInput`: an
`Arc` onto the source the run is in (for a run of the file, the file's chunk
store, shared) and where the run starts there. A span decode reads its input
through it. A chunk not loaded is reported as a `Missing`, which already names
a chunk of the file, and reaches the web as every other missing chunk does.

**B. Per-span compressed slices.** The pass copies each span's compressed
bytes into the index. Spans cover the whole run, so this keeps the whole run,
cut into pieces: the same memory as today. The 32 MiB browser limit is gone
only because no single read needs all of it. Keeping only some slices, in a
bounded cache, needs a way to read a dropped slice again, which is A or C.

**C. The caller fills in the input.** The `Seekable` keeps no handle and no
input. A read says which compressed bytes it needs, and whoever holds a
document reads them and hands them in. The evaluator can. A tab cannot: every
entry point of the wasm crate that reads a tab over an indexed run would have
to catch the request, read the bytes in the home sheet's reading and call
again. A recognised tab's own evaluator reads its bytes deep inside an
evaluation, so nothing can fill them in before the read, and its `Pending`
would name chunks of the tab, not of the file.

**D. One whole copy, assembled a window at a time.** The pass reads its
windows from the source and appends each to a copy the `Seekable` keeps. This
is part 2. It fixes the browser case for runs up to 64 MiB compressed, but
keeps both the 64 MiB limit and the memory.

| | A. shared handle | B. per-span slices | C. caller fills in | D. whole copy |
|---|---|---|---|---|
| Compressed bytes held | none outside the chunk store | the whole run | none | the whole run |
| A tab's reads, with no evaluator | read the source, report file chunks | work | need a catch in every wasm entry point | work |
| A recognised tab's own evaluator | `Pending`, as for any read | works | cannot be filled in advance | works |
| Largest compressed run | 512 MiB (trace positions, see "Limits") | memory | 512 MiB | 64 MiB |
| What changes | `Source`, `ChunkStore`, `Document`, `Seekable` | the index pass, `Seekable` | `Seekable`, the wasm crate | the evaluator |

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
   `EvalError::Pending(chunks)` and calls again, as for any read of the file.
5. xz's `read_span` puts its part-decoded span (`lazy::xz::Prefix`) back in
   the cache when the read is missing chunks. Today it drops the prefix on any
   error, which is right for damage and wasteful here.

A read of many spans decodes them one after another. Spans decoded before a
miss stay in the span cache (16 MiB), so calling again does not decode them
again. One read of more than 16 MiB of output, whose spans together need more
compressed input than the store keeps, could still ask for the same chunks for
ever: the second call finds the first spans dropped from the span cache and
their chunks pushed out of the store. Nothing reads that much in one call
except a check over one xz block (`run_check`, up to 64 MiB of output); see
"Still to do".

The bounds in part 6 keep the single read in step 1 to at most 8.5 MiB for a
deflate span, and to at most `Limits::span_input` (16 MiB) for a zstd or xz
span, under the store's 32 MiB. See "Checkpoints by compressed distance".

## The decoders read a window

Part 3. Nothing a reader sees changes; until part 7, every caller passes the
whole run as one window, except `decode_step` (part 4).

### Deflate

- **`Bits` gains a base.** `Bits` (`bits.rs`) reads `buf[at >> 3]`, with `at`
  counted from the first bit of `buf`. A new field gives how many bits of the
  run come before `buf[0]`. `at` stays relative to `buf`, so `bit` and `take`,
  the hot path, are unchanged. Only `pos()` adds the base, and the places that
  set `at` from a position in the run (`run`, `skip`, and `stored`'s
  `bits.buf()[start..stop]`) subtract it.
- **`inflate::run` takes an `Input`** in place of `data: &[u8]`. The `Input`
  holds the window the codes are read from and, for a decode that starts
  inside a Huffman-coded block, a second small window that holds that block's
  header and tables. `run` reads the header and tables from the small window.
  Where it sets the reader's position to `Resume.in_bit` today (`skip`), it
  changes to the code window instead.
- **Why the header gets a window of its own.** A decode that starts inside a
  block reads the block's header and tables again from `Resume.block`. A
  block may be as long as the run: zlib ends a block every 16,384 codes, but
  an encoder may write one block for a whole file. So the header can be
  anywhere earlier in the run. `Resume` and `Start::Deflate` therefore keep a
  copy of the header: the bytes from the header's first byte, at most 287. A
  dynamic header with its tables is at most 2,286 bits: 3 for BFINAL and
  BTYPE, 14 for HLIT, HDIST and HCLEN, 57 for 19 code-length code lengths, and
  at most 7 for each of the 286 + 30 code lengths that `dynamic_tables`
  accepts. Up to 7 more bits come before it in its first byte. 256
  checkpoints with a copy each come to under 72 KiB.
- **Where a decode stops before its window ends.** The decoder may stop at
  any place where it calls `Sink::boundary`: the start of a block, and before
  every code of a Huffman-coded block, the first code included (`symbols`).
  The longest stretch between two such places is a stored block: 3 header
  bits, up to 7 padding bits, 32 bits of LEN and NLEN and up to 65,535 bytes,
  under 65,541 bytes in all. A dynamic header with its tables is under 300
  bytes, and one code is at most 48 bits. `Code::decode` reads one bit at a
  time, so the decoder never reads past the code it is decoding. So a decode
  whose `stop_at` is at least 65,541 bytes before the window's end stops at a
  boundary before it runs out of window, unless the window reaches the end of
  the deflate data. `DEFLATE_MARGIN` is that 65,541.
- **The wrapper.** `inflate::framing` takes the head of the run and the run's
  length instead of the whole run. It says how many more bytes it needs when
  a gzip name or comment runs past the head it was given: those two fields
  end at a zero byte and have no length limit. `trailer_agrees` takes the
  trailer's 4 or 8 bytes instead of the run.

### zstd

`ZstdWalk::run` (`codec/frames.rs`) slices `data[f.at + f.read..self.end]` and
hands it to `ruzstd`'s `decode_blocks` one zstd block at a time. With a
window, before each zstd block the walk reads the 3-byte block header from
the window. It checks that the block, and the frame's 4-byte checksum after
the last block, lie inside the window. If they do not, and the window ends
before the run does, the walk stops there and says it needs input from that
byte on. It does the same before a frame header (at most 18 bytes) and before
a skippable frame's 8-byte header; the contents of a skippable frame are
skipped without being read. So a zstd step needs at most one zstd block past
where it stops: a margin of 128 KiB + 25 bytes. `ruzstd` keeps the zstd
history window itself from one zstd block to the next, so nothing else
changes.

### xz

`codec::xz::layout` reads the stream header (12 bytes), the footer (12 bytes),
the stream's index, whose length the footer gives, and the header of every
block, to learn the header's length and whether `plain_lzma2` holds. It takes
all of these from a slice of the whole stream. Part 8 makes it a pass in
steps; see "xz" below.

The LZMA2 decoder (`codec::lzma::Lzma2`) already decodes one LZMA2 chunk per
call and keeps its state between calls. An LZMA2 chunk is at most 64 KiB
packed. The decoder reads its LZMA2 chunk from a slice of the block's LZMA2
data, which becomes a window with a base. `lzma::lzma2_len`, which reads every
LZMA2 chunk header of a block before the block's first LZMA2 chunk is
decoded, reads through the same kind of window.

### `decode_step`

`inflate::decode_step` reads only the trace step's own bits. It rebuilds the
block's tables from the trace's head steps (`rebuild`), not from the run. It
takes a window and the byte the window starts at. Then, in part 4, the wasm
crate's `decode_step` reads the run from byte `in_bits.start / 8` to byte
`in_bits.end.div_ceil(8)`, at most 7 bytes, instead of reading with
`field_bytes` from the run's front.

### Built (part 3)

**Built on 2026-11-26.** Every caller still passes the whole run, so the
bytes, traces and refusals are the same as before. Where the code differs
from the text above:

- **One window type.** `codec::Window` serves all three decoders: the bytes,
  the byte of the run they start at, and the run's length, so that
  `reaches_end` can tell a window that ends early from a run that does.
  `within` gives the window over part of the run, such as an xz block's LZMA2
  data, with positions counted from that part's front.
- **Deflate.** `Bits::with_base` sets the base; `pos`, `seek`, `end_at` and
  `end_pos` count from the run's front, and `slice` takes a stored block's
  payload. `inflate::Input` holds the code window and the optional head
  window (`Input::whole`, `Input::new`). `DEFLATE_MARGIN` and
  `MAX_HEAD_BYTES` are public constants in `inflate`. A decode that runs out
  of a window that ends before the deflate data does is refused as `Failed`,
  like a stream cut short: keeping the margin is the caller's job.
- **The wrapper.** `framing(wrapper, head, run_len)` answers `Framed::At`, or
  `Framed::NeedsHead(n)` when the header goes on past the head. A head that
  is the whole run never gets `NeedsHead`, and `Framed::found` turns the
  answer into a `Result` for callers that pass the whole run.
  `trailer_agrees` takes the bytes `Framing::trailer` names.
- **`Seekable`'s deflate reads take their input** as a parameter, so that
  part 7 can pass windows: `deflate_span`, `deflate_span_trace`, `group_from`
  and `block_trace_from`. Today each is passed `Input::whole`.
- **`decode_step` keeps its signature**, the whole run from the front, so the
  wasm crate and the evaluator's test did not change. The windowed form is
  `decode_step_at(data, at, trace, step)`, for part 4 to call.
- **zstd.** `ZstdWalk::run` takes a `Window` and answers `Walked::Ended`,
  `Walked::Stopped` or `Walked::Needs(range)`. The range starts where the
  walk stopped and ends where the thing it reads next ends. Before a frame
  or skippable frame the walk needs 18 bytes, or up to the span's end if that
  comes first. Before a block it needs the 3-byte header, and then the block
  with the frame's checksum when the block is the last. A window that holds
  one byte less gets the same `Needs` again, and the walk has not changed.
  `ZSTD_MARGIN` (128 KiB + 25 bytes) is a public constant in `frames`.
  `lazy::zstd::decode_span` and `span_trace` take a window.
- **LZMA2 and xz.** The range coder has a base, which `bit_pos` and
  `byte_pos` add; reading a byte costs the same. `Lzma2::chunk_in` and
  `lzma2_len_in` read from a window. A window that does not hold the next
  chunk is refused as `Failed`, before the decoder changes, rather than
  answering that it needs more input as the zstd walk does: this design gives
  that answer only to zstd, and a later part can call again with a larger
  window. `Lzma2::chunk` still takes the whole stream, and `lzma2_len` is
  now used only by tests. `xz::blocks`, `lazy::xz::read_span` and
  `lazy::xz::span_trace` take a window of the stream, which has to hold the
  LZMA2 data of every block they decode.
- **Tests**, each from copies of only the bytes the decode needs: every
  deflate span and its trace from its checkpoint's byte to the next
  checkpoint's, with the block header in a head window of `MAX_HEAD_BYTES`
  (stored, fixed and dynamic blocks, every wrapper, the last span with the
  trailer); every block trace and every group of dropped block starts;
  `decode_step_at` from the step's own bytes; a gzip header read from heads
  of every length; a zstd walk given windows from 1 byte to the whole run,
  with and without skippable frames; every zstd span; an LZMA2 stream read a
  chunk at a time from windows that hold exactly the next chunk; every xz
  span's trace and reads.
- **Speed: no change that the measurements can show.** Measured on
  2026-11-26 in release builds, native, with harnesses outside `cargo test`,
  pinned to one core. "Before" is built from the commit this part started
  from. Before and after runs alternated, and the table gives the fastest run
  of each. The machine was loaded by other sessions (a load average of 6 to
  14 on 12 cores), and single runs varied by 10% or more, so differences
  under about 3% are noise. The files were generated for the measurement:
  rows of CSV-like text packed by Python's `zlib` at level 6, and bytes that
  are nine tenths one letter packed with Huffman coding only.

  | What | File | Before | After | Change |
  |---|---|---|---|---|
  | `decode_or_index` | 190 MB of CSV, 61 MB packed (6 runs each) | 1,952 ms | 1,909 ms | -2.2% |
  | Reading all of it through the index, 4 MiB at a time | the same | 1,562 ms | 1,527 ms | -2.2% |
  | Every span's trace | the same | 1,277 ms | 1,275 ms | -0.2% |
  | `decode_or_index` | 300 MB of literals, 56 MB packed (6 runs each) | 3,957 ms | 3,886 ms | -1.8% |
  | Reading all of it through the index | the same | 3,221 ms | 3,162 ms | -1.8% |
  | Every span's trace | the same | 2,413 ms | 2,434 ms | +0.9% |
  | User CPU time of the whole run | the same | 10.45 s | 10.39 s | -0.6% |
  | `inflate::gzip`, traced | 60 MB of CSV, 19 MB packed (8 runs each, fastest of 5) | 619.7 ms | 619.1 ms | -0.1% |
  | `inflate_len` | the same | 350.2 ms | 353.0 ms | +0.8% |

## The index pass reads through a `RunInput`

Part 7 for deflate and zstd. xz follows in part 8.

### `RunInput`

```rust
/// Where a run's compressed bytes are read from, for as long as anything
/// reads the run: a shared handle onto the source the run is in, and where
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

The `Send + Sync` bound keeps `Seekable` `Send + Sync`, as it is today.
`RunInput` lives in `codec::lazy` and knows nothing of the evaluator: a space
is passed to it as a `Source`, which `SpaceSource` already is.

**Every `Missing` names a chunk of the file.** Every source a run can be in
reads, in the end, from the file's `ChunkStore`, and a space's own store never
reports a chunk missing (`ChunkStore::behind`). The web side already assumes
this: `fetchChunk` fetches any chunk a reply names as a chunk of the file,
"whichever document asks".

### Where a `RunInput` comes from

`Evaluator::run_input(doc, key)` makes one for the run at `key`:

| The run is in | The handle | Copy |
|---|---|---|
| The reading's own document (space 0), in a stretch no edit touched | `Source::share()` of the document's source, at `Document::source_offset_of(at_bits, n_bits)` | none |
| A space held whole | an `ArcSource` over the space's `Arc<Vec<u8>>` | none |
| A space read through an index | the space's `SpaceSource::Lazy` | none |
| A stitched space, an edited stretch, or a source that cannot be shared | the run gathered whole (part 2), as an `ArcSource`, at most `CAP_BYTES` | the run |

- **`Source::share(&self) -> Option<Arc<dyn Source + Send + Sync>>`** is a new
  trait method, `None` by default: a handle that reads the same bytes and
  outlives the borrow. `ChunkStore` gives a handle onto its chunks. To share
  them, it moves its chunk map into an `Arc<Mutex<…>>`, together with its LRU
  clock, which today is in `Cell`s; the wasm build is single-threaded, so the
  lock is never contended. A store with a source behind it gives that source.
  `ArcSource` and `SpaceSource` give a clone of themselves. `MemSource` gives
  none: its field is public, there are 1,042 `MemSource(...)` constructions
  to change, and it holds the file in memory anyway, so a copy costs what it
  costs today.
- **`Document::source_offset_of(at_bits, n_bits) -> Option<u64>`** is new: the
  byte of the source where a stretch of the document starts, when the stretch
  lies inside one piece of the original (a `Piece` with `Src::Orig`) and
  starts on a byte there. An edit inside the run, or a bit-level insert before
  it, gives `None`, and the fallback copy is made.
- **Edits.** Any edit drops every index (`Spaces::forget`), and the wasm
  crate's `ensure_open` opens a stale tab's stream again before answering
  anything about it. A handle made before an edit still reads the original
  file at the place it was given, never the edited bytes, and nothing reads
  through it once the edit is made.

### A step

A deflate step:

1. Works out its window: from the byte holding the bit where the step starts,
   to `STEP_INPUT` (256 KiB) + `DEFLATE_MARGIN` past it, or to the end of the
   deflate data if that comes first.
2. Reads the part of the window it does not already hold, through the
   `RunInput`. If chunks are missing, it returns them and changes nothing.
3. Decodes with `stop_at` set `DEFLATE_MARGIN` before the window's end (or no
   `stop_at` when the window reaches the end of the data), and with its output
   budget of `INDEX_STEP_BYTES` as today, and stops at whichever comes first.
4. Keeps the window's bytes from where it stopped onward, and the copy of the
   current block's header, and drops the rest.

So a step reads at most about 320 KiB of new input, and the pass holds at
most that much. Because a step is bounded by input as well as by output, a
run of empty blocks, which reads input and makes no output, is bounded too.

The first step also reads the run's last 8 bytes, once, and keeps them. For
gzip, they hold `ISIZE`, which today `DeflatePass::new` reads to decide
whether the pass starts as `known_large`; that decision moves to the first
step, since `Indexing::new` no longer has any bytes. They also hold the
trailer the last step checks. And the first step grows its read of the run's
head until `framing` can parse the wrapper.

A zstd step does the same with the zstd margin. A frame whose compressed size
passes `Limits::unit_input` (16 MiB) is refused as `TooLarge`, the way a frame
whose output passes `Limits::span` is; that limit and its refusals come in
part 6. The size counted is the frame's own, without the skippable frames in
front of it.

### How `Pending` fits the go machinery

`Indexing::step` gets a third answer beside `Working` and `Done`:
`NeedsInput(chunks)`. A refusal stays an error, as now. `index_pass`
becomes:

```text
pass = the run's pass from Unfinished, or Indexing::new(codec, run_input, limits)
loop:
    charge INDEX_STEP_ELEMENTS to the go; if it ran out, put the pass back, Busy
    step INDEX_STEP_BYTES:
        Working        -> carry on
        NeedsInput(c)  -> put the pass back, return Pending(c)
        Done(decoded)  -> Ok(Ok(decoded))
        error why      -> Ok(Err(why)), kept in refused_runs
```

- `Indexing::new` reads nothing now, so the pass exists, and is in
  `Unfinished`, from the first call on. `Evaluator::unpacking` answers
  throughout, including while the pass waits for chunks, and the listing
  shows `REPORT.unpacking` with a percentage instead of `REPORT.reading`.
- A missing chunk travels as `EvalError::Pending`, not as a `Refusal`, so it
  never reaches `refused_runs`.
- In the browser each wait loads one window's chunks. They are contiguous,
  so `Doc.settle` also starts reading the 48 chunks after them. That is about
  one round for every 3 MiB of compressed run (an estimate from `READ_AHEAD`,
  not measured).
- Charging is unchanged. A step that finds its input missing has been
  charged for work it did not do.
- Making the `RunInput` may itself wait, when it is the fallback copy. That
  read keeps its progress as in part 2.

### Checkpoints by compressed distance

Part 6. `Checkpoints::offer` takes a checkpoint when the output since the last
checkpoint reaches `spacing`, or when the compressed input since the last one
reaches `in_spacing` (`IN_SPACING_START`, 1 MiB). Thinning doubles both
spacings. Without the second rule a span's input has no bound: an empty fixed
block is 10 bits and makes no output, and a hostile run can be made of
nothing but empty blocks.

- **How far the spacings grow.** Output and input checkpoints share one list
  of at most `MAX_CHECKPOINTS` (256), so a run has about `out / s + in / s`
  checkpoints, where `s` is the spacing both rules use (they start equal and
  double together). At the limits, 1 GiB of output and 512 MiB of input, that
  is 1.5 GiB / `s`, which is at most 256 once `s` is 8 MiB. So both spacings
  reach at most 8 MiB, three doublings from 1 MiB. Today the output spacing
  reaches at most 4 MiB; a run that is large on both sides now gets spans of
  up to 8 MiB of output, two of which fill the 16 MiB span cache.
- **Deflate.** A deflate checkpoint can go between any two codes. A new
  checkpoint is taken at the first boundary at or past `in_spacing`, so a
  span's input is at most `in_spacing` plus the longest stretch between two
  boundaries (a stored block, 64 KiB). Each thinning joins two spans and so
  adds one more such stretch to the most a span can be, and there are at most
  three thinnings: at most 8 MiB + 8 × 64 KiB, which is 8.5 MiB.
- **zstd and xz.** A checkpoint is only ever at the start of a frame or block.
  So the pass also takes one at the start of any frame or block whose own
  compressed size is `in_spacing` or more, so that such a frame or block is a
  span of its own: xz knows each block's size from its index, and the zstd
  pass learns a frame's size when the frame ends and puts the checkpoint in
  at the frame's start then. Before thinning, a span is then either one frame
  or block of at most `unit_input` (16 MiB), or smaller ones that come to at
  most twice `in_spacing`. Thinning can join two spans into one larger than
  that. So a zstd or xz span read checks the span's compressed size against
  `Limits::span_input` (16 MiB) before it reads anything, and a span over it
  is refused when read, as `TooLarge`, instead of asking for more chunks than
  the store keeps. Thinning needs more than 256 frames or blocks, and the
  tools do not write runs that reach the limit that way: `pzstd -1` writes
  frames of 2 MiB of output, and `xz -T0 -0` blocks of 1 MiB, so their spans
  stay a few MiB of input even after thinning.
- **Skippable frames** count towards `in_spacing`, since they are bytes of the
  run, but not towards a frame's `unit_input`. A zstd span read reads the
  8-byte header of each skippable frame at the span's start and gathers its
  input from the first frame on, so a skippable frame's contents are never
  read.
- **Spans of no output.** A span may now hold no output at all.
  `span_of_byte` already answers the last span that starts at or before a
  byte, so such a span is never read for bytes. `span_of_bit` still finds it
  for the bits it covers.

When the output passes the cap, the pass goes back to the last checkpoint, as
today. It reads its window again from that checkpoint's bit, which is at most
`in_spacing` back; the steps just before had loaded those chunks.

### xz

Part 8. xz's layout becomes a pass in steps, so it joins `lazy::can_step` and
is charged and reports progress like the others:

1. The stream header and the footer.
2. The stream's index, whose length the footer gives. An index longer than
   `Limits::unit_input` is treated as one `layout` cannot read, which sends
   the stream to the decoder of whole streams, as today.
3. The block headers, up to 64 per step. Each is read where the index puts
   it, and the missing chunks of all 64 are reported together. So a stream of
   a thousand blocks spread over 300 MiB loads its headers in about 16 rounds,
   not a thousand.

A block whose compressed size passes `Limits::unit_input` is refused as
`TooLarge` (part 6 adds this, checked from the gathered copy until part 8).
Until part 8 lands, an xz run is gathered whole (part 2) and laid out from the
copy as today.

## A `Seekable` reading through its `RunInput`

`Seekable::packed` is removed; the `Seekable` keeps a `RunInput`. Each read
gathers its input first (see "A chunk pushed out while a synchronous read is
under way"):

| Read | Compressed input it needs at once |
|---|---|
| `decode_span(k)`, `deflate_span_trace(k)` | from checkpoint k's bit to checkpoint k+1's, and the header copy: at most 8.5 MiB |
| zstd `decode_span`, `span_trace` | the span's frames, without the contents of skippable frames: at most `span_input`, 16 MiB, or the read is refused |
| xz `read_span` (`Prefix::reach`) | the LZMA2 data of the block the read is in, read again on each later read of the span: at most `unit_input`, 16 MiB |
| xz `span_trace` | the span's blocks: at most `span_input`, 16 MiB, or the read is refused |
| `block(i)` | none. The index pass records BFINAL and BTYPE with each kept block start, in the top bits of the integer that holds the start's bit position, which is always under 2^32 |
| `group(g)` | from kept block start g to kept block start g+1. Block starts are thinned once a run has more than `MAX_BLOCK_MARKS` (65,536) blocks, so this only arises then, and only a hostile run then has a long stretch between two kept starts. A stretch longer than `Limits::unit_input` is refused as `TooLarge` |
| `block_trace(i)` | from the block's start until its trace turns coarse: `span_steps` is 1,000,000 trace steps, one per code, so at most 6,000,000 bytes. The decode stops there and closes the block with one opaque step to the block's end, which the index knows. Today it decodes to the block's end without recording the codes, and the trace comes out the same |

### Everything that has to treat `Missing` as "not yet"

Part 5. Today these turn a failed read into a refusal, an empty answer or
zeroes. Each must pass `Missing` on instead:

| Where | Today | With `Missing` |
|---|---|---|
| `Seekable::read`, `span_trace`, `trace_at_byte`, `trace_at_bit`, `map_out`, `map_in`, `block`, `block_of_bit`, `block_trace` | a `Refusal`, or `None` | `ReadError::Missing(chunks)`, beside `ReadError::Refused(Refusal)` |
| `read_in`'s arm for a space read through an index | `fail("unpacking failed")` | `EvalError::Pending` |
| `read_stitched` and `part_index`, through `index.read` | `part_refusal` | `EvalError::Pending` |
| `SpaceSource::read`, `read_bytes` | zeroes, no chunks reported | zeroes, the chunks reported |
| `Evaluator::template_for`, which sniffs the first 36 KiB | sniffs whatever came back | answers `Pending` before sniffing; otherwise it would sniff zeroes |
| xz `read_span` | drops the prefix | keeps it |
| `Joined::read`, `map_out`, `map_in`, `part_trace` | `Refusal::Failed`, or `None` | `Missing` |
| `run_check` reading one xz block through the index | `fail(refused(why))` | `EvalError::Pending` |
| `traced::Blocks::get`, `at_bit`, `trace_of` | `Option` | `R<Option<_>>` |
| `Space::map_out`, `map_in`, `trace_at_bit` | `Option` | a `Result` whose error is the missing chunks; the wasm crate replies "pending" |
| `index_run`'s `refused_runs` | keeps refusals | never sees a missing chunk, which is an `EvalError` |

## Measuring a gzip member

`stream_len_at` reads up to 64 MiB in one `read_in`: the rest of the file
from the run's start, not the member, whose length is what is being found.
Part 2 gathers that read in stretches that double, from 1 MiB up to 64 MiB,
and runs `inflate_len` after each stretch; a stream that ends inside a
stretch is measured there. So a small member costs a small read, and a large
one is decoded at most about twice over, in total, before it is measured.
This fixes a .gz of up to 64 MiB compressed. As built, part 2 grows the
walk's bytes a window at a time instead of in doubling stretches; see
"Gathering a run a window at a time (part 2)". Part 9
replaces the gathered copy with a pass of its own, `lazy::Measuring`: the
`CountSink` decode that `inflate_len` makes, done a window at a time through
a `RunInput` over the stretch `stream_len_at` measures in, from the run's
start to the end of the field around it. Its state is where the next step
starts and how much output it has counted. It is kept from one go to the next
in a small table in `Spaces` beside `Unfinished`, which keeps at most two
measuring passes, keyed by space, offset and end. It is charged per step like
the index pass, and reported by `Evaluator::unpacking`. Its output cap stays
64 MiB, for the reason DESIGN-lazy-unpacking gives: a larger cap would let a
hostile file cost a full 1 GiB decode once to size the member and again to
index it. This also does the item in DESIGN-lazy-unpacking's "Still to do"
about making `inflate_len` resumable.

A .gz then makes two passes in the browser, one to size the member and one to
index the part of `decoded` over it, and the listing shows the unpacking
percentage go to 100 twice. Two passes are what the design means to do. Today
the first pass also happens, but in one go, uncharged and with no progress
shown.

## Joined streams

Part 10, after part 7.

- `JoinedBytes::Held(Arc<Vec<u8>>)` becomes `JoinedBytes::Stored(RunInput)`.
- `JoinedBytes::Packed { run, codec }` becomes `Packed { input, codec }`. The
  run is read through the handle when the stream's part is decoded; a BGZF
  part is under 64 KiB compressed.
- `JoinedBytes::Indexed` keeps its `Seekable`, which reads its own input.
- The 64 MiB limit on what `join_lazily` reads is removed.

Three changes need no handle and come earlier, in part 2 (built; see
"Gathering a run a window at a time (part 2)"). Without them a
joined stream whose parts come to more than the store keeps asks for chunks
for ever, since both joins start again from the first part on every
`Pending`, and each part's own read fits.

- `join_whole`, for a stream of at most 64 MiB, reads every part in one call.
  It becomes a walk kept from one go to the next, in `Spaces` and keyed by
  the stream's path, holding the bytes and the trace made so far, as it holds
  them now.
- `join_lazily` checks that every small packed part unpacks to the length its
  run claimed, by decoding every part in one call, uncharged: a 1 GiB BAM is
  1 GiB of inflate in one go. It becomes the same kind of walk, holding the
  parts made so far, charged per part like an index step and reported by
  `Evaluator::unpacking`. The alternative is to drop this check and rely on
  the check `Joined::read` already makes when it first decodes a part. That
  is cheaper, but the tab would open over a part of the wrong length and show
  zeroes there.
- `unpacked_part` reads a packed part whole through `unpack_run`. For a codec
  that can be indexed, `unpacked_part` calls `index_run` instead, which steps;
  for any other codec, `unpack_run`'s read is gathered.

## Limits

| Limit | Value | Status |
|---|---|---|
| `LAZY_CAP_BYTES` | 1 GiB of output | Stays. The zip-bomb guard. |
| `CAP_BYTES`, as the output of a run held whole | 64 MiB | Stays. |
| `CAP_BYTES`, as the compressed size of a run that can be indexed | 64 MiB | Replaced by `Limits::input`, 512 MiB, in the five places listed in "The problem". |
| `CAP_BYTES`, as the input and output of a codec that cannot be indexed | 64 MiB | Stays. The input is gathered a window at a time (part 2). |
| `inflate_len`'s output cap | 64 MiB | Stays. |
| `Limits::span` | 64 MiB of output per zstd frame or xz block | Stays. |
| `INDEXED_RUN_BYTES` | 64 MiB of compressed runs | Stays at 64 MiB, but weighs each index by what it holds for as long as it is kept: its checkpoints' 32 KiB windows (at most 8 MiB), its header copies (under 72 KiB) and its block starts (at most 1 MiB), about 9.1 MiB at most. The caches inside each index are bounded as today. |
| `whole_cap`, as the input of a joined document read a part at a time | 64 MiB | Removed. |
| `Limits::input` (new) | 512 MiB of compressed input | A trace keeps each step's bit in 32 bits (`RawStep.in_start`, and `pack` truncates without a word), and 2^32 bits is 512 MiB. A longer run is refused as `TooLarge` before anything is read, since the size of the field holding the run is known. |
| `IN_SPACING_START` (new) | 1 MiB of compressed input between checkpoints | Doubled with `SPACING_START` when checkpoints are thinned, to at most 8 MiB. |
| `Limits::unit_input` (new) | 16 MiB of compressed input | The most one zstd frame (without the skippable frames in front of it) or one xz block may be, and the most one stretch between two kept deflate block starts may be (`group`). A frame, block or stretch over it is refused as `TooLarge`. |
| `Limits::span_input` (new) | 16 MiB of compressed input | The most one read of a zstd or xz span may gather at once. A span over it is refused when read, as `TooLarge`. Deflate spans never reach it (8.5 MiB at most). |
| `STEP_INPUT` (new) | 256 KiB of compressed input | How far past its start a step may stop; its window reaches `DEFLATE_MARGIN` further. Beside `INDEX_STEP_BYTES`, 256 KiB of output. |
| `DEFLATE_MARGIN`, and the zstd margin (new) | 65,541 bytes; 128 KiB + 25 bytes | How far past where a step stops its window has to reach. |
| `MAX_HEAD_BYTES` (new) | 287 bytes | A deflate block header and its tables, from the header's first byte. |
| `GATHER_WINDOW` (new) | 1 MiB, or a quarter of what the store keeps when that is less | Part 2, built without the floor of one chunk the proposal had. |
| `GATHER_BYTES` (new) | 64 MiB | Part 2: the most the copies in `Gathers` may set aside between them. |

**The zip-bomb guard.** Memory stays bounded by the table above and
DESIGN-lazy-unpacking's, so a hostile run can only cost time. That time is
the index pass over at most 1 GiB of output and 512 MiB of input, plus, for a
gzip member, the measuring pass over at most 64 MiB of output.

**`unit_input` is a choice.** It refuses some files that open natively today:
a stream of at most 64 MiB compressed and more than 64 MiB unpacked, with one
frame or block of more than 16 MiB compressed. `xz -T0 -6` writes blocks of
24 MiB of output, so data that packs at less than 1.5 to 1 with it is refused.
A larger value leaves less of the 32 MiB store for everything else a span
read competes with. Lifting it needs a span decode that keeps its decoder
between reads; see "Still to do".

## The browser

### What the listing and the status bar say

| While | The listing's waiting row | The status bar, after Open unpacked |
|---|---|---|
| a run is gathered (part 2) | `REPORT.reading`, "Loading bytes needed to map these fields…", which is accurate now that the gather finishes | `UNPACKED.loading` |
| the index pass runs, or waits for chunks (part 7) | `REPORT.unpacking`, "Unpacking the compressed data to read these fields… 37%" | `UNPACKED.unpacking`, on "pending" replies as well as on "working" ones |
| a read can never fit (part 1) | the refusal, in the value column of the field | the refusal, as `JOINED.refused` and `DECODED_REFUSED` word it |

No new string is needed for the first two rows. A percentage while a run is
gathered would help for a 64 MiB run; that string should be drafted with the
`ui-text` and `no-riddlespeak` skills when part 2 is built.

### `waitToOpen`

`waitToOpen` (`openwait.ts`) gives up when a chunk it loaded is asked for
again with no "working" reply in between. Once the pass reads its input in
windows, a wait can see many "pending" replies in a row with unpacking done
between them. A chunk asked for again then (for example the last checkpoint's
window after the pass goes back at the cap, when something else had pushed
it out) does not mean the store is too small. So:

- `Doc.openSpace` returns the unpacking progress with a "pending" reply as
  well, and `waitToOpen` clears `loaded` whenever the progress has moved
  since the last reply, not only on "working".
- The status bar shows `UNPACKED.unpacking` on a "pending" reply while a pass
  is under way, so that the line does not switch between loading and
  unpacking on every round.
- `MAX_ROUNDS`, which is 512, should count only rounds with no progress. A
  512 MiB run takes about 170 rounds at one per 3 MiB, which is under 512 as
  things are, but a backstop is meant to stop a wait that has stopped moving.

These changes land with part 7 or before it.

### `ensureRange` on a tab

`Doc.ensureRange` loads every chunk that `has_chunk(this.space, c)` says is
absent, and on a tab `has_chunk` never says a chunk is absent: a space's store
answers true for every chunk. So on a tab over an indexed run, `ensureRange`
resolves at once and the read after it comes back incomplete. For a space
other than 0, `ensureRange` should read the range, wait on `fileChunks` of the
chunks the read names, and repeat until the read is complete, giving up by the
same rule as `waitToOpen`. A read's missing chunks are exactly the file chunks
it waits on, so no new query of the core is needed.

### The smaller fix first (part 1)

**Worth doing first: yes.** It is about half a day. It ends the endless wait
in every case where one call needs more chunks than the store keeps: the zip
entry, the .gz, a bzip2 or LZMA run, a checksum check over a long run, a
joined stream whose parts add up to more than the store. Its check in `read_in` stays as the guard for any read that
later parts leave as it is. After part 2 no compressed run reaches the check,
so its refusal becomes rare, and that is fine.

- `Source::resident_limit(&self) -> Option<u64>` is new: the most bytes the
  source holds at once. A `ChunkStore` with no source behind it answers its
  capacity times its chunk size, every other source answers `None`, and
  `Document` passes on its source's answer.
- `read_in`, for space 0, fails a read that touches more chunks than the
  store keeps, with the message below, instead of answering `Pending`.
- Before reading a run in one call, `index_pass`, `open_space_at`'s arm for
  codecs that cannot be indexed and `unpack_run` refuse it with a new
  `Refusal::TooMuchAtOnce` (tag `too-much-at-once`), kept like any refusal. A
  refusal shows on the field, whereas a failed read would leave the field
  with nothing under it (`flatten.ts` draws an error reply for children as no
  children).
- `join_whole` and `join_lazily` read many parts in one call, and each part's
  read fits on its own. So they add up the sizes of the parts' runs that are
  in the file before reading any, and refuse the stream with the same
  refusal when the total is more than the store keeps.
- `stream_len_at` caps its read at half of what the store keeps, as well as
  at 64 MiB. A member whose stream ends inside that is measured as now. A
  longer one measures as nothing, as a run past the cap does, and the gzip
  template's fallback places it as everything before its trailer, which is
  right for a file of one member.
- On the web side, `DECODED_REFUSED` and `JOINED.refused` get the new tag.
- What part 1 does not catch: a read that fits on its own, but not beside the
  other chunks a go needs, can still ask for the same chunks for ever.
  `waitToOpen`'s rule stays for that.

**Built on 2026-11-26.** Where the code differs from the list above:

- **Runs in a stitched space are counted too.** A run inside a joined stream,
  such as a PNG's zlib stream over its IDAT chunks, is read with
  `read_in` on the stitched space, which `read_stitched` turns into one
  space-0 read per stored part. Each of those fits, so the guard in `read_in`
  never fires, and the run asked for the same chunks for ever. The three
  refusals ask `Evaluator::cannot_fit`, which adds up what
  `file_bytes_at_once` says the read needs of the file: the bytes for space
  0, the stored parts in the file for a stitched space, nothing for a space in
  memory. The joins use the same sum (`parts_cannot_fit`). The guard in
  `read_in` stays space 0 only.
- **A join past the cap is refused as too large, before reading.** Without
  that, `join_lazily` would read parts up to the 64 MiB cap before refusing,
  and in the browser that read waits for ever. So when the parts cannot fit,
  a stream whose runs pass the cap (or, in `join_whole`, one with a packed run
  past 64 MiB or runs past what a trace can count) is refused with
  `TooLarge`, as it is natively, and any other with `TooMuchAtOnce`.
- **`stream_len_at` still reads in one call**, through `stream_end`, which now
  measures a deflate stream a step at a time with `inflate::Measuring` and
  keeps the walk in `Measures`. The walk's bytes are still one `read_in` of
  the whole stretch, so the cap is applied where the stretch is chosen, in
  `stream_len_at`, for space 0 only.
- **Bytes, not chunks.** The checks compare bytes against
  `resident_limit()`. A read within one chunk of the limit that does not start
  on a chunk can still touch one chunk more than the store keeps, and waits
  as before. `waitToOpen`'s rule covers it.
- **What is still wrong, and expected.** A .gz of several members, one of
  them over 16 MiB compressed, now reads in the browser as one member running
  to the last trailer, even when the file fits in the store; part 2's
  doubling stretches fix it. And listing a joined stream whose part is
  refused, such as a large .gz's `decoded`, fails the read with the part's
  name (`members[0].compressed is too large for the editor to keep in memory
  at once`), which `flatten.ts` draws as nothing under the stream, as for any
  part that fails. Part 2 fixed both: the member is sized as itself, and a
  part of 64 MiB or less is read a window at a time instead of refused.
- **Strings.** Beyond the two below: the joined stream's status-bar message
  reuses `UNPACKED.tooMuchAtOnce`, which now takes the limit; the report's
  finding (`RV.refused`) and its PNG paragraph (`RV.pngNotInflated`) have
  words of their own; a check over a refused stream says "the data it covers
  is too large for the editor to keep in memory at once". The 32 MiB comes
  from `RESIDENT_BYTES` in `doc.ts`, made from the numbers the store is made
  with, and in the core from `resident_limit()`. The listing and the
  inspector carry it in the hover.
- **Tests.** `crates/wasm/tests/open_space_waits.rs` reproduced the zip entry
  and the .gz asking for chunk 0 again before the fix, and asserts the
  refusal after it; the test that asserted the old behaviour is now
  `a_run_longer_than_the_store_is_refused_as_too_much_at_once`. The core's
  are in `eval/tests/resident.rs`.

**Proposed strings.** Drafted by a fresh agent from a brief, with no code in
its context, and checked against the code since. They are proposals for
sign-off.

| Where | Proposed string | Note |
|---|---|---|
| Value column of a compressed field, the inspector after ` · `, and `{field} is …` for a joined stream's part (`Refusal::message` and `DECODED_REFUSED["too-much-at-once"]`) | `too large for the editor to keep in memory at once` | The 32 MiB goes in the row's tooltip, for example "The editor keeps at most 32 MiB of the file in memory." If no tooltip is possible: `too large for the editor to keep in memory at once (limit 32 MiB)`. |
| The failure of any other read that cannot fit (`read_in`) | `Can't load {needed} of the file at once: the editor keeps at most 32 MiB in memory` | `{needed}` is the read's size, like `40 MiB`. It avoids the verb "read", which would be wrong for a checksum field whose own bytes can be read. |

The drafting agent also pointed out that the existing `too large to unpack
(over 64 MiB)` does not say what is over 64 MiB, and that beside a 32 MiB
phrase it can read as one limit with two values. Today `Refusal::TooLarge` is
given both for a compressed run over 64 MiB and for an output over 64 MiB,
and with this design also for a run over 512 MiB compressed. See "Open
questions".

### Gathering a run a window at a time (part 2)

**Built on 2026-11-26.** Where the code differs from "Recommendation",
"Measuring a gzip member", "Joined streams", "Memory" and the part 2 item in
"Order of work". A **place** below is one of the two slots that
`Unfinished` keeps for index passes, as in DESIGN-lazy-unpacking's "Waiting
for a place"; `Measures` and `Spaces::joins` have two places each as well.

- **Which reads are gathered.** A read of a whole run is made a window at a
  time only when the file's source keeps part of the file
  (`resident_limit()` answers a number) and the read needs more of the file
  at once than one window (`Evaluator::gathers`, from `file_bytes_at_once`).
  Every other read of a whole run, including every read natively and every
  read of a space held in memory, is still one `read_in`. The window size is
  `GATHER_WINDOW`, 1 MiB, or a quarter of `resident_limit()` when that is
  less. The proposal's floor of one chunk is not built, since the core does
  not know the chunk size. Each window ends on a multiple of the window
  size, counted from the start of the space the run is in: the file, or a
  stitched stream for a run joined from parts. In the browser the window is
  1 MiB, so each window of the file is 16 whole chunks of 64 KiB. Each window
  is charged to the go before it is read, one element for each 16 KiB
  (`GATHER_BYTES_PER_ELEMENT`), so a go of 5,000 elements (5,000 × 16 KiB)
  can copy up to 78 MiB of chunks that are already loaded.
- **An index pass reads its run into its own place.** A pass that is
  stepped across goes first takes a place (`wait_for_a_place`, as before),
  and then reads its run into that place (`Held::Gathering`). The pass
  starts in the same place once the run is complete. So no run is read to
  the end with no place for its pass. The partial copy is in the place, and
  the finished copy stays there as the pass's own copy of the run. A call
  waiting for a place steps a run being read in the same way as it steps a
  pass. An xz run, and any run in a go with no allowance, is read through
  `gather_run` rather than into a place.
- **`gather_run` and `Gathers`.** The other reads of a whole run use
  `Evaluator::gather_run`, which keeps its copies in `Spaces::gathers`:
  `open_space_at` for a codec that cannot be indexed, `unpack_run`,
  `run_check` over a run of the file, and a joined stream's parts. A gather
  sets the run's whole length aside against `GATHER_BYTES` (64 MiB) when it
  starts. A gather that has started and not finished is never let go of; the
  proposal's "drops the one read longest ago first" is not built. A gather
  that would take the total past 64 MiB waits, as a pass waits for a place:
  it reads on into the unfinished copy with the fewest bytes left. A copy
  finished by a waiting call stays in `Gathers` until the call that started
  it takes it. When every copy in `Gathers` is finished and none has been
  taken, the copy put back in `Gathers` longest ago is let go of.
  `gather_run` hands a finished copy to its caller and keeps nothing, so a
  caller reads anything else that may wait for bytes before it gathers the
  run: `open_space_at` reads a JPEG scan's settings first, and `run_check`
  the sum written in the file. Otherwise a wait after the gather would lose
  the copy.
- **Where `TooMuchAtOnce` is still given.** `gather_run` refuses a run as
  `Refusal::TooMuchAtOnce` when it cannot set the run's length aside and
  nothing in `Gathers` can be read on or let go of. That happens only for a
  run longer than 64 MiB, which every caller refuses as too large before it
  calls `gather_run`, or for a run gathered while another gather in the same
  call is under way and has set aside the rest of the 64 MiB. No run of
  64 MiB or less is refused this way on its own. The check in `read_in`
  stays, for the reads still made in one call: the code panel's read of a
  run from its front to a trace step (`decode_step`, which part 4 changes), a
  JPEG scan's settings, and a template field read whole. The tests are
  `copies_past_what_they_may_come_to_wait_for_room_and_a_run_past_it_is_refused`
  (with the 64 MiB lowered to 2 KiB) and
  `a_read_longer_than_the_store_fails_with_the_reason_and_a_shorter_one_waits`
  in `eval/tests/resident.rs`, and the `decode_step` call at the end of
  `a_zip_entry_longer_than_the_store_lists_a_window_at_a_time` in
  `crates/wasm/tests/open_space_waits.rs`.
- **A gzip member's bytes are read as far as its stream goes.** Since this
  document was written, `inflate::Measuring` keeps where it stopped between
  steps, so the walk that measures a member does not read stretches of
  doubling length and run `inflate_len` again over each. Before each
  measuring step, `measure_steps` reads more of the walk's bytes, a window at
  a time and each window charged, until they reach `MEASURE_AHEAD_BYTES` past
  where the step starts. That is 128 KiB for the step's 1 Mibit, 65,541
  bytes for a stored block the step may not stop in, and 1 KiB for one more
  code: about 193 KiB. Those bytes are always enough for the step. As a
  guard, a step refused as failed while there are bytes left to read is
  taken once more with one window more: a step that ran out of bytes would
  then succeed, and a step over a damaged stream fails again. Any other
  refusal is final at once, such as output past `Limits::measure` (64 MiB),
  which more bytes would not change. So each member is decoded once, and its
  walk reads about one step past the end of its stream, or past where its
  output passed 64 MiB. The bytes stay in `Measures` with the walk, so each of its two
  places holds at most 64 MiB, as before. This applies natively too:
  measuring a short member now reads about 193 KiB, where it read everything
  to the end of the container, up to 64 MiB. Part 1's cap of half the store
  is removed, and the walk's key again covers everything from the member's
  start to the end of the container or 64 MiB on. This undoes the regression part 1 recorded: a member
  longer than half the store is sized as itself.
- **Joins are walks.** `join_whole` and `join_lazily` are now one walk,
  `JoinWalk` in `stitch.rs`, which either holds the joined stream whole or
  makes it ready to read a part at a time, as the two functions did. The
  walk is kept in `Spaces::joins` by the path of the stitched node, and waits
  for one of its two places as a pass does. Before a part is read, the walk
  checks that the go has allowance left. After the part is read, the go is
  charged one element, plus one for each 512 bytes the part unpacks to (the
  rate an index step is charged at) or for each 16 KiB copied. A part that
  waits for bytes is charged nothing, so it cannot spend each go's allowance
  before it is read. When a part short enough to be read in one call waits
  for bytes, the walk also asks for the chunks of the parts after it, up to a
  window of the file, so a BAM loads about a window of BGZF blocks in each
  round rather than one block. A go that runs out in a walk reports how far
  the walk has got through `Evaluator::unpacking`: the bits of the parts'
  runs read so far, out of the bits of all of them, with the bit where the
  first part's run starts as the run's place in the file. A stream too large
  to join is refused before any part is read (`too_large_to_join`), since
  the sizes it is refused for are all in the part table. Part 1's
  `parts_cannot_fit` is removed. `join_lazily`'s check that each packed part
  unpacks to the length its run claimed is kept, and charged per part.
- **`unpacked_part` goes through `index_run`** only for a packed part, of a
  codec that can be indexed, whose run needs more of the file than one
  window. With no `resident_limit()`, as natively, that is a run longer than
  1 MiB. A shorter run is read and decoded in one call, as before. A BGZF
  block is under 64 KiB, and an index pass for it would wait for a place
  behind the passes over other runs.
- **The web.** The inspector's checksum check asks `runCheck` again until it
  answers (`Doc.runCheckLoaded`, through `waitToOpen`). A check over a run
  longer than a window can now answer "working", when the go ran out, or
  "pending", when chunks are not loaded. A run that fits in the store is
  loaded before the first ask, as before. `waitToOpen` is unchanged. No
  string was added or changed, and no percentage is shown while a run is
  gathered (see "Open questions").
- **What may still go wrong in the browser.** A .gz member longer than the
  store has its run read twice, once to measure it and once to unpack it, so
  the core asks again for chunks it read earlier. `waitToOpen` gives up when
  the core asks again for a chunk that `waitToOpen` loaded with no "working"
  reply in between. If Open unpacked is pressed while such a member is still
  being measured, its wait sees both reads. Measuring answers "working"
  between its rounds, but reading a run a window at a time answers only
  "pending" while its chunks load. So the second read, reaching chunks the
  end of the first read loaded and the store has since pushed out, can make
  `waitToOpen` give up with `UNPACKED.tooMuchAtOnce`. Pressing Open unpacked
  again then carries on from where the core stopped. The tests do not meet
  this, and it was not tried in a browser. Part 7's change to `waitToOpen`
  removes the risk.
- **Tests.** In `eval/tests/resident.rs`, over a store of four chunks of
  1 KiB, the six tests that asserted part 1's refusals, and the one that
  asserted its regression for gzip members, now assert that the same runs
  open or are sized as themselves. Four tests are new. The first checks
  that a run gathered across goes and waits comes to the same bytes as one
  read. The second asks for three runs longer than the store by turns, and
  checks that all three open and no pass starts twice. The third checks that
  copies past what `Gathers` may set aside wait for room, and that a run
  longer than that on its own is refused. The fourth checks that a gzip
  member whose output passes a lowered `Limits::measure` (64 KiB) is
  measured from under 300 KiB of the file, not all 1 MiB of it. In
  `crates/wasm/tests/open_space_waits.rs`, the store keeps 3 chunks of
  64 KiB in `a_run_longer_than_the_store_opens_a_window_at_a_time`, where a
  gzip file of 7 chunks opens, and 16 chunks in the other new tests. A zip
  entry and a gzip member of about 3 MiB compressed each list and open, with
  no reply naming more than the 4 chunks of one window and no chunk asked
  for more than 3 times. The gzip member is followed by a short second
  member, and is sized as itself. A BGZF file of 140 blocks, over 3 MiB,
  opens as a tab.

## Memory

For an indexed run, besides DESIGN-lazy-unpacking's table:

- the index pass's window: at most about 320 KiB for deflate and 384 KiB for
  zstd, while the pass runs;
- the header copies: under 72 KiB per run, for as long as the index is kept;
- the trailer: 8 bytes, while the pass runs;
- what a span read gathers: at most 8.5 MiB for deflate and 16 MiB for zstd
  or xz, for the length of the read;
- what a block trace gathers: at most 6,000,000 bytes, for the length of the
  trace.

The compressed bytes otherwise stay in the file's chunk store, 32 MiB in the
browser, which the file's tab keeps anyway. A run that falls back to a copy
(an edited run, a run in a stitched space, a run in a `MemSource`) holds that
copy, at most 64 MiB, as every run does today. Gathering (part 2) sets aside
at most 64 MiB for the copies in `Gathers`, and never lets go of a copy it
has started. An index pass reads its run into its own place in `Unfinished`,
which holds the same copy once the pass starts.

## Order of work

Each part builds, passes its tests and can be merged on its own, in this
order. Each part needs only the parts before it.

1. **Refuse a read that can never fit.** `Source::resident_limit`, the check
   in `read_in`, `Refusal::TooMuchAtOnce` before the reads of a whole run and
   for joins whose parts add up to more than the store keeps,
   `stream_len_at`'s read capped at half the store, the web strings. Fixes:
   the endless wait, wherever one call needs more chunks than the store
   keeps. **Done 2026-11-26**; see the end of "The smaller fix first
   (part 1)".
2. **Gather a run a window at a time.** `Evaluator::gather_run`: a run read in
   one call today is read `GATHER_WINDOW` at a time through `read_in`, into a
   copy kept in `Spaces` from one go to the next (at most 64 MiB of partial
   copies), charged per window. Used by `index_pass`, `open_space_at`,
   `unpack_run` and `covered_bytes`, and by `stream_len_at` in doubling
   stretches. `join_whole` and `join_lazily` become walks kept from one go to
   the next, and `unpacked_part` steps through `index_run` for codecs that can
   be indexed. Fixes, in the browser and with memory as today: the 34 MiB zip
   entry, a .gz of up to 64 MiB compressed, and a joined stream such as a BAM
   whose parts come to up to 64 MiB. **Done 2026-11-26**; see "Gathering a
   run a window at a time (part 2)".
3. **Decoders read a window.** `Bits` with a base, `inflate::run` taking an
   `Input`, `framing` and `trailer_agrees` taking pieces of the run,
   `ZstdWalk`, `Lzma2` and `decode_step` on windows. Every caller passes the
   whole run. Nothing a reader sees changes. **Done 2026-11-26**; see "Built
   (part 3)" under "The decoders read a window".
4. **`decode_step` reads only its trace step.** The wasm crate reads at most
   7 bytes of the run. Fixes: the code panel on a trace step deep in a long
   run.
5. **`Missing` as "not yet".** `RunInput`; the `Seekable` keeps one, made over
   its copy (an `ArcSource`), so that until part 7 no read of a `Seekable`
   finds a chunk missing; `ReadError`; every row of the table in "Everything
   that has to treat `Missing` as 'not yet'". Nothing a reader sees changes.
6. **Bound what a `Seekable` read needs.** Checkpoints by compressed
   distance, and at the start of large frames and blocks; header copies;
   BFINAL and BTYPE with the block starts; block traces that stop when coarse;
   `group`'s bound; `Limits::unit_input` and `Limits::span_input` and their
   refusals. The spans of runs that pack badly change; their bytes and traces
   stay the same.
7. **Share the source.** `Source::share`, `ChunkStore`'s chunks behind an
   `Arc<Mutex<…>>`, `Document::source_offset_of`, `Evaluator::run_input`. The
   deflate and zstd passes read their windows through the `RunInput` and
   answer `NeedsInput`, and `index_pass` keeps the pass across `Pending`.
   `Limits::input` replaces the 64 MiB compressed limit in the five places
   listed in "The problem", and `Indexes` weighs what each index holds for as
   long as it is kept. On the web side, `waitToOpen`, the status line and
   `ensureRange`, as in "The browser". Fixes: runs over 64 MiB compressed,
   and the memory of every run that has a handle.
8. **xz's layout in steps.** xz joins `can_step`.
9. **Measure a gzip member in steps.** `lazy::Measuring` replaces the
   gathered copy in `stream_len_at`.
10. **Joined streams through handles.** `JoinedBytes` over `RunInput`, and
    `join_lazily` without its 64 MiB input limit.
11. **Bring DESIGN-lazy-unpacking up to date.** Its "State", the "Limits" row
    for `CAP_BYTES`, the paragraph at the end of "The browser", and "Still to
    do".

Parts 1 and 2 fix both files seen. Parts 3 to 7 are the substance of the
design and make the most change; part 7 is the one that removes the 64 MiB
limit.

## Tests

### The core (`codec::lazy`, small limits)

- **Windows.** For each run in the existing tests of the pass in steps, a
  pass whose window is every size from the margin plus one byte up to the
  whole run comes to the same checkpoints, block starts, checksums, bytes
  and, for a run under the cap, the same trace step for step, as the pass
  over the whole run.
- **A source that loses chunks.** A test `Source` over a vector, with a set of
  chunks it holds: a read reports the other chunks missing and fills them with
  zeroes. The pass, fed only the chunks it reports, comes to the same index.
  The pass never reports more chunks in one answer than its window covers.
  With a source that drops every chunk once it has been read, the pass still
  finishes, since a step needs only its own window.
- **Span reads over that source.** A read reports exactly the chunks of its
  span's input, all at once; after they are fed, the read returns the right
  bytes. A span is never cached from a read that was missing chunks, and an
  xz prefix is kept after a read that was missing chunks.
- **Compressed distance.** Runs of empty fixed blocks, of stored blocks and of
  incompressible data, with both limits reached: no deflate span's input is
  over `in_spacing` plus one stored block for each thinning, thinning doubles
  both spacings, and a span of no output is never read for bytes. zstd and
  xz runs with one large frame or block among small ones: the large one is a
  span of its own. A span joined by thinning past `span_input` is refused
  when read and reads nothing. A zstd span read with a large skippable frame
  at its start does not read the skippable frame's contents.
- **Block traces.** A block of more than `span_steps` codes: the trace stops
  decoding at the coarse point, reads no further input, and equals the trace
  made by decoding to the block's end.
- **Wrapper.** A gzip with a 1 MiB comment, and one with an extra field of
  60 KiB (its length field is 16 bits), are indexed with windows of 64 KiB. A
  gzip whose header is cut short is still refused.
- **Limits.** With `Limits::input` small, a longer run is refused as
  `TooLarge` before anything is read. A zstd frame, an xz block and a `group`
  stretch over `unit_input` are refused.
- **zstd and xz windows.** Windows that end inside a zstd block, a frame
  header, a checksum and a skippable frame's header. xz's layout in steps
  reads block headers 64 at a time, still refuses a delta or branch filter,
  and sends an index over `unit_input` to the decoder of whole streams.

### The evaluator (`eval::space`, `eval::stitch`)

- A zlib run past the small cap, in a `Document<ChunkStore>` with a store
  smaller than the run and fed only the chunks each `Pending` names, opens.
  `unpacking` moves forward on every answer, waits included, and the index is
  the one a `MemSource` gives.
- `Document::source_offset_of`: no edits; an insert before the run; an
  overwrite inside it (the fallback copy); a bit-level insert before it (the
  fallback copy). An edit drops the index and its handle.
- Gathering (part 2): a run read through a store of four chunks comes to the
  same bytes as one `read_in`, and a gather interrupted by `Busy` and by
  `Pending` carries on from where it stopped. `stream_len_at` over a gzip of
  several small members reads about each member's length, not 64 MiB, and
  still measures a member that ends past the first stretch.
- Joins (parts 1 and 2): a BGZF stream whose parts add up to more than a
  small store is refused with `too-much-at-once` after part 1, and opens as a
  document after part 2, with no part decoded twice for the length check.
- Joined streams: a BGZF stream joined through handles reads the same bytes
  as with copies, and the walk that checks the parts' lengths carries on from
  one go to the next and refuses a part of the wrong length as now.

### The wasm crate (`crates/wasm/tests`)

These follow `open_space_waits.rs`: an `Editor` with a small store, fed only
the chunks each "pending" reply names, and called again on "working".

- **The stuck listing, reproduced.** A zip of one deflated entry whose
  compressed size is several times the store: for example a store of 16
  chunks and an entry of 4 MiB compressed, made from enough lines of hex, as
  `lines()` makes them, to pack to that. The test asks `template_children` for
  the entry's `data` and its contents, the way the listing does.
  - Today: a chunk is asked for again after it was fed, within a few rounds,
    and the contents never come. The test asserts this first, so the case is
    known to be reproduced.
  - After part 1: the `data` field is refused with `too-much-at-once` within
    a few rounds.
  - After part 2 (a store of 16 chunks gives a gather window of 4 chunks) and
    again after part 7: the contents read within a bounded number of rounds,
    and no reply names more chunks than one window covers.
- **The .gz.** A gzip of one member larger than the store, read with the
  gzip template. Today `members[0].compressed` never gets a size. After part
  2, and again after part 9, the member gets its size, `decoded` reads, and
  Open unpacked gives a tab.
- **A chunk pushed out and fetched again.** On that tab, with a store small
  enough that reading the tab's end pushes out the chunks its start needed:
  `read_bytes` at the start, then at the end, then at the start again. The
  third read reports the start's chunks as missing, and after they are fed it
  returns the same bytes as the first read.
- **A recognised tab.** A gzip of a tar larger than the store: the tab's own
  reading reads the name of the tar's first member, with "pending" replies
  that name chunks of the file.
- **`decode_step`.** A bit near the end of a 4 MiB run: the "pending" reply
  names one chunk, not the run's chunks.
- **The existing test that asserts the old behaviour.**
  `a_run_longer_than_the_store_asks_again_for_chunks_it_was_fed` keeps a
  store of 3 chunks and a run of 7 chunks, and asserts that a chunk is asked
  for again. After part 1 it asserts the refusal instead, and after part 2,
  with a gather window of one chunk, that the run opens; its name changes to
  match. After part 7 a deflate step's window is up to 256 KiB + 65,541 bytes,
  6 chunks of 64 KiB, so tests of the pass use stores of 16 chunks or more.

### The browser

A new `web/test/lazyinput.browser.mjs`, run by hand like the other
`*.browser.mjs` scripts (global Playwright through `PLAYWRIGHT_MODULE`, and a
`TEST_URL` that says `localhost`). Like `hexload.browser.mjs`, it makes its
files in Node: a zip with one entry of lines packed with
`zlib.deflateRawSync` to more than 32 MiB, and a gzip of lines packed to more
than 32 MiB. It drops each on the page as a `File`, with a synthesised `drop`
event, and checks that:

- the listing shows `REPORT.unpacking` with a growing percentage, and
  `REPORT.reading` for no more than a few seconds, and then the entry's
  fields;
- Open unpacked opens a tab, and the hex view at the tab's end, and back at
  its start, shows the right bytes;
- the wasm memory (`WebAssembly.Memory.buffer.byteLength`) stays below the
  compressed size plus 64 MiB by a clear margin;
- the code panel answers for a byte near the tab's end.

Before part 1 the same script shows the stuck row, which is the reproduction
in the browser.

## Still to do

- **Lift `unit_input`.** A zstd span decode could keep its frame decoder
  between reads, as xz's `Prefix` keeps its LZMA2 decoder, and xz's
  `lzma2_len` could read a block's LZMA2 chunk headers a window at a time,
  with its progress kept in the `Prefix`. The traces of zstd and xz spans
  would still need the whole span's input at once.
- **One read of more than the span cache.** See the end of "A chunk pushed
  out while a synchronous read is under way". `run_check` over one xz block
  could sum the block a span at a time instead of reading it whole.
- **Runs past 512 MiB compressed.** Trace positions counted from the trace's
  own first bit (`Trace::start`), instead of from the run's front, would lift
  the limit to 512 MiB per span.
- **`UNFINISHED_PASSES`.** Two passes are kept, because one in its traced
  phase (the part of the pass that may still come to a run held whole) holds
  up to 64 MiB. A pass past the cap holds only its window and checkpoints, so
  more of those could be kept. Today a listing that shows three large zip
  entries at once starts one of the three passes again from the beginning
  each time the listing carries it on.
- **`CHUNK_CAPACITY`.** A larger store would let `unit_input` grow. That is a
  choice about the browser's memory, not part of this design.

## Open questions

- Should `TooLarge` be split, so that the listing can say whether the
  compressed run or the output is too large? With this design the compressed
  limit is 512 MiB, and the output limits are 64 MiB for a run held whole and
  1 GiB for an indexed run. One phrase cannot say all three.
- Is the length check in `join_lazily` worth a walk across goes, or should a
  part of the wrong length show as a failed read when it is read?
- Should gathering report a percentage, and with what string?

## What was checked

Read in full or in the parts named, on the branch as merged with main on
2026-11-26:

- `docs/DESIGN-lazy-unpacking.md`, in full.
- `codec/lazy/mod.rs`, `deflate.rs`, `seekable.rs`, `zstd.rs` and `xz.rs`, in
  full: every use of `packed`, the thinning of checkpoints and block starts,
  and the size checks.
- `codec/inflate.rs`: `framing`, `trailer_agrees`, `Sink`, `CountSink`,
  `run`, `skip`, `stored`, the field limits in `dynamic_tables`, the calls to
  `boundary` and the coarsening in `symbols`, `Code::decode`, `decode_step`
  and `rebuild`, `inflate_len`.
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
  `MemSource(` constructions in `crates`.
- `formats/gzip.rs`: how `members[i].compressed` gets its size.
  `formats/zip.rs`: `data_len`.
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

Not measured: no timing or memory figure in this document comes from running
anything. The estimate of rounds is arithmetic from `READ_AHEAD` and the chunk
size.

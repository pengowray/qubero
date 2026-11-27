# Design: reading the compressed input of large runs a piece at a time

Written 2026-11-26, for developers. This is a proposal. Parts 1 to 3 are
built (2026-11-26; "The smaller fix first (part 1)", "Gathering a run a
window at a time (part 2)" and "Built (part 3)" say where the code differs
from this text). Parts 4 to 6 are built too (2026-11-28; see part 4 in
"Order of work", "Built (part 5)" and "Built (part 6)"). Part 7 is split into
7a, 7b and 7c (see "Review before part 7"), and 7a and 7b are built
(2026-11-28; see "Built (part 7a)" and "Built (part 7b)"). Parts 7c and 8 to
11 are not. It follows
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
`field_bytes` from the run's front. Part 4 was built on 2026-11-28; see item 4
of "Order of work".

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
- **Speed.** The span traces of a stream of short codes are about 2% slower;
  everything else measured is the same within the noise. Measured on
  2026-11-26 in release builds, native, with harnesses outside `cargo test`,
  pinned to one core. "Before" is built from the commit this part started
  from. Before and after runs alternated, and the table gives the fastest of
  each. The machine was loaded by other sessions (a load average of 4 to 14
  on 12 cores), and single runs varied by 10% or more, so the tables compare
  the fastest runs, and differences under about 2% are noise. The files were
  generated for the measurement: rows of CSV-like text packed by Python's
  `zlib` at level 6, and bytes that are nine tenths one letter packed with
  Huffman coding only, whose codes are mostly one bit long.

  | What | File | Runs | Before | After | Change |
  |---|---|---|---|---|---|
  | `decode_or_index` | 190 MB of CSV, 61 MB packed | 6 | 1,952 ms | 1,909 ms | -2.2% |
  | Reading all of it through the index, 4 MiB at a time | the same | 6 | 1,562 ms | 1,527 ms | -2.2% |
  | Every span's trace | the same | 6 | 1,277 ms | 1,275 ms | -0.2% |
  | `decode_or_index` | 100 MB of literals, 18.5 MB packed | 6 of 4 | 1,227 ms | 1,204 ms | -1.9% |
  | Reading all of it through the index | the same | 6 of 4 | 950 ms | 956 ms | +0.7% |
  | Every span's trace | the same | 6 of 4 | 858 ms | 878 ms | +2.3% |
  | `inflate::gzip`, traced | 60 MB of CSV, 19 MB packed | 8 of 5 | 620 ms | 619 ms | -0.1% |
  | `inflate_len` | the same | 8 of 5 | 350 ms | 353 ms | +0.8% |

  "6 of 4" is six runs of a harness that takes the fastest of four tries.
  The span traces of the literals were slower after in each of the six runs,
  by 0.2% to 2.6%; that is the one difference the measurements show. The
  likely cause: each code there makes a step, and each step's bit is now the
  reader's base plus its place in the window, one addition more. Earlier comparisons on 300 MB of literals, one
  try a run under a load of 12, ranged from -2% to +4% for the same three
  rows.

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
  file at the place it was given, never the edited bytes. A few calls still
  read through it after the edit: `open_again` answers from the stale tab
  while the stream is opened again (`selection_text`, for one). That is
  harmless, since the source does not change and the old index matches it.

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

### Built (part 5)

**Built on 2026-11-28.** The index passes still read a copy of the run
gathered whole (part 2), and each hands its `Seekable` a `RunInput` over
that copy (`RunInput::held`, an `ArcSource`). So outside the tests no read
finds a chunk missing yet, and nothing a reader sees changes. Where the
code differs from the text above:

- **`RunInput`** is in `codec/lazy/input.rs`, as sketched, with `held` for a
  copy. Two crate-private methods sit on top of `read`: `copy` cuts a range
  short at the run's end and reads it, and `copies` reads several ranges and
  names every chunk that any of them is missing, sorted and once each.
  `read` fills any bytes past the run's end with zeroes. No caller asks for
  bytes there.
- **Which reads answer `ReadError`.** `read`, `span_trace`, `block`,
  `block_trace` and `block_trace_from` answer `ReadError`, as the table's
  first row says. The five reads that answered an `Option` before
  (`trace_at_byte`, `trace_at_bit`, `map_out`, `map_in` and `block_of_bit`)
  answer `Result<Option<_>, Vec<Missing>>` instead. For them a refusal
  still gives `None`, as before, and the error holds only missing chunks.
  `lazy::refused_as_none` converts a `Result<T, ReadError>` into that form.
  `Space::map_out`, `map_in` and `trace_at_bit`, and `Joined::map_out` and
  `map_in`, answer the same kind of `Result`. `Evaluator::map_out` and
  `map_in` answer `R<Option<Step>>`, with `EvalError::Pending` for missing
  chunks, and the wasm crate turns that into a "pending" reply.
- **Each read copies only the compressed bytes it needs** through the
  `RunInput`. The ranges are the ones part 3's tests decode from:
  - a deflate span: from the byte holding its checkpoint's bit to the byte
    holding the next checkpoint's bit, and `MAX_HEAD_BYTES` from the header
    of the block the checkpoint is inside;
  - a zstd span, and the trace of a zstd or xz span: the span's frames or
    blocks, the bytes `span_in` gives;
  - `block(i)`: the one or two bytes holding BFINAL and BTYPE;
  - `group(g)`: from kept block start `g` to the next kept block start;
  - `block_trace(i)`: from the block's first byte to the byte holding its
    last bit, or to the run's end for the last block;
  - an xz read: it asks `Prefix::needs` which blocks it will decode, and
    reads their LZMA2 data whole, or nothing when the bytes it reads are
    already decoded.

  Until part 6 bounds what a read needs, one copy can be as long as the
  whole run. Each read makes its copy again, and the time the copies take
  was not measured.
- **A read of several spans** stops at the first span missing chunks and
  names that span's, as "A chunk pushed out while a synchronous read is
  under way" says.
- **In the evaluator**, `read_in`'s arm for a space read through an index is
  `read_indexed`, and `read_stitched`'s read of a part's index is
  `read_part_index`, both `#[cold]` and out of line for the stack. `Blocks`
  gains `first_start`, out of line for the same reason, for placing a
  stream's `blocks` row. `traced_count`, `traced_at` and
  `Evaluator::block_trace` answer `R`. `traced_at` now reads the block or
  block trace it needs first, and passes it to `traced_step_at`, which
  finds the row. `unit_of` takes the first block's kind from a whole trace
  only, and never reads an indexed run for it: an indexed run's blocks are
  deflate blocks, and every kind of deflate block is counted as "block".
- **Rows that needed no change.** `part_index` reads nothing through an
  index; only `read_stitched`'s read does. `Joined::part_trace` decodes the
  copy of the compressed run that a `JoinedBytes::Packed` holds; part 10
  replaces that copy with a `RunInput`. No code that reads a `Seekable`
  adds an entry to `refused_runs` or `Stitch::troubles`, and the tests check
  that both stay empty.
- **The web** needed no change. `Doc.settle` already fetches the chunks
  that any "pending" reply names, and `mapOut`, `mapIn` and `decodedCode`
  return `null` for such a reply.
- **A problem left for later.** `template_for` can now answer `Pending`
  after a joined stream's walk has finished. `join_whole` gives the finished
  walk to its caller and keeps nothing, so when the call is made again, the
  evaluator walks the joined stream again from its first part. In part 5
  `template_for` never answers `Pending` there: it does so only when an
  index reads through a source that can lose chunks, and part 7 is the
  first part to make such an index. See "Still to do".
- **Tests.** In `codec/lazy/tests.rs`, `Lossy` is a source over a file that
  holds only the chunks it was fed. A `Seekable` built as usual is given a
  `RunInput` onto it (`Seekable::set_input`, for tests), starting past
  other bytes of the file. Over deflate (with checkpoints inside blocks),
  zstd (with skippable frames) and xz runs, the read of each span names
  exactly the chunks of the bytes it needs (for xz, chunks inside the span)
  in one answer, leaves the span out of the span cache, and reads the right
  bytes once those chunks are fed. An xz span
  read part of the way keeps its prefix through a read that finds chunks
  missing, and carries on from it. Span traces, blocks, block traces, the
  block of a bit with thinned block starts, and `map_out` and `map_in` wait
  the same way, and then give what an index over the whole run gives. In
  `eval/space.rs`, a zlib stream whose index reads through such a source is
  opened, read by a field and by a tab, mapped both ways, and listed down to
  a block and its codes. Each of these first answers `Pending`, naming only
  chunks inside the run, and `refused_runs` stays empty. In
  `eval/stitch.rs`, a BGZF part read through such an index answers
  `Pending` in the stitched space and in the joined stream opened as a
  document, and is never added to `Stitch::troubles`.

### Built (part 6)

**Built on 2026-11-28.** `Limits::unit_input` and `Limits::span_input` are
in the code, and apply only when the source under the `RunInput` has a
resident limit (`Source::resident_limit` answers a number), as "Review before
part 7" asks. Until part 7, every index reads through `RunInput::held`, an
`ArcSource` over the pass's copy of the run, which has no resident limit. So
nothing that opens today is refused, and only the tests reach the
refusals. The output bytes and traces of every run are the
same as before. Where the code differs from "Checkpoints by compressed
distance" and the table above:

- **Which runs the limits apply to.** `RunInput::resident_limit` gives the
  source's resident limit. The passes are given the `RunInput` the index will
  read through, by `Indexing::through` and `decode_or_index_through`. These
  are private for now, and `Indexing::new` and `decode_or_index_with` call
  them with `RunInput::held`. The test source `Lossy` has a resident limit
  only when it is made with `Lossy::keeping`, so the part 5 tests still read
  with no limit.
- **Checkpoints.** `Checkpoints::offer` takes a checkpoint when the output or
  the input since the last checkpoint reaches its spacing, and then resets
  both. Thinning doubles both spacings. No code caps the spacings: under the
  default limits they reach at most 8 MiB, as worked out above. A place that
  is offered twice (by a step that carries on from it, or by the pass going
  back to its last checkpoint at the cap) gets one checkpoint. `Checkpoints::take_if_long` takes one
  at the start of a zstd frame or xz block whose input is the input spacing
  or more.
- **zstd frames.** For a frame with skippable frames in front of it, the
  input since the last checkpoint is counted up to the frame's own magic,
  past those skippable frames, while the checkpoint is taken where the first
  of them starts. So a long skippable frame always starts a span, and a read
  of the span can skip it. The pass learns a frame's size when the next frame starts
  or the walk ends, and `ZstdWalk` now reports both where a frame's skippable
  frames start and where the frame itself starts (`FrameAt`), and where the
  last frame ended (`frames_end`). Skippable frames after the last frame
  become a span of their own, with no output, once they reach the input
  spacing.
- **zstd span reads.** Each zstd checkpoint keeps the byte where its span's
  first frame starts (`Start::Frame { own }`). A read of the span's bytes
  copies the run from there to the span's end, so it reads none of the
  skippable frames in front of that frame. A span's trace has a step for each
  of those skippable frames, so it reads their 8-byte headers. It reads them
  in pieces of up to 64 KiB (`FRONT_PIECE`), and reads the first piece and
  the span's frames together, in one read through the `RunInput`. So the
  trace of a span that starts with one long skippable frame reads the first
  64 KiB of that frame and none of the rest. A skippable frame in the middle
  of a span is read with the span. It is shorter than the input spacing,
  except in a span that thinning made from two.
- **`unit_input` for a zstd frame** is checked when the frame ends. The size
  counted is from the frame's own magic to the start of the next frame's
  skippable frames. As for `Limits::span`, a frame that ended while the run
  might still be held whole is checked only once the run passes the cap, so
  a run that is held whole is never refused for it.
- **xz blocks.** `unit_input` is checked against a block's whole size in the
  stream (`Layout::block_len`: header, data, padding and check). An xz read
  copies one block's LZMA2 data and the data of the blocks after it that the
  read needs, up to `span_input` bytes of the stream, and decodes them before
  it copies more. `Prefix::reach` returns the number of the next block it
  needs when its window does not hold that block. A read of a span larger
  than `span_input` therefore takes more than one round, and works. With no
  resident limit, a read copies every block it needs at once, as before. The
  table above says an xz read needs at most `unit_input` at once. As built it
  needs at most `span_input` at once, or one whole block when that block is
  larger; a block larger than `unit_input` is refused when the stream is
  indexed. Both limits are 16 MiB.
- **Where `span_input` is checked.** By the read and the trace of a zstd span,
  against the bytes from its first frame's own start, and by the trace of an
  xz span, against the whole span. Deflate span reads are not checked, since
  their input is bounded by the input spacing.
- **`group`'s bound** is checked when `group` reads a stretch, not by the
  pass.
- **Header copies** are made when the pass finishes, from its copy of the
  run, for the checkpoints it keeps. Checkpoints inside the same block share
  one copy, and `held_bytes` counts the copy once for each of them, so it
  overstates the memory a little. In part
  7 the pass holds only a window of the run, so it has to copy the current
  block's header when the block starts.
- **BFINAL and BTYPE.** `inflate::run` passes a block's BFINAL and BTYPE to
  the sink once it has read them (`Sink::head`). `inflate::BlockStart` keeps
  them in the top three bits of the word that holds the bit where the block
  starts, so a block start is still 16 bytes. The lists that `group` decodes again record
  them too. `block(i)` reads the run only when the start of block `i` or
  block `i + 1` was dropped by thinning.
- **Block traces.** With `TraceBuilder::stopping_when_coarse`, `inflate::run`
  stops where the trace turns coarse and returns `Ended::Coarse`, with the
  block still open in the trace. `block_trace_from` then closes the block at
  the end the index gives. For the last block it also adds the padding step
  and any step after the stream, with `inflate::stream_tail`, which
  `inflate::run` now uses as well. For a Huffman-coded block, `block_trace`
  copies at most `MAX_HEAD_BYTES` plus 6 bytes for each of `span_steps`
  codes. A stored block, which never turns coarse, is copied whole. The last
  block is now copied to its own end, not to the end of the run.
- **Refusals.** Every refusal added is `Refusal::TooLarge`, as the design
  says. Its message, "too large to unpack (over 64 MiB)", names 64 MiB of
  output, while these refusals are for 16 MiB of compressed input; see "Open
  questions". No reader sees it before
  part 7, which is the first part to make an index over a source with a
  resident limit. No string was added or changed. Part 7a changed all of
  these refusals to `Refusal::TooMuchAtOnce`; see "Built (part 7a)".
- **Spans that change.** Measured on 2026-11-28 in a debug build, by indexing
  each run with the default limits, and again with the input spacing set so
  large that it never takes a checkpoint, which gives the old spans:

  | Run | Packed | Unpacked | Spans, old rule | Spans, new rule | Largest span input, old rule | Largest span input, new rule |
  |---|---|---|---|---|---|---|
  | The Rubin raw, gzip | 94,940 B | 75,677,760 B | 73 | 73 | 3,021 B | 3,021 B |
  | 1 MiB of text and 1 MiB of noise by turns, 40 times, gzip level 6 | 53,794,726 B | 83,886,080 B | 80 | 80 | 1,049,179 B | 1,049,318 B |
  | 70 MiB of zeros with 8,000,000 empty fixed blocks after the first MiB, raw deflate | 10,071,321 B | 73,400,320 B | 70 | 79 (9 of them with no output) | 10,001,033 B | 1,048,576 B |

  Text and noise packs to about as many bytes as it unpacks to, so both rules
  take a checkpoint at about the same places. In the run of empty blocks, the
  9 extra checkpoints keep 288 KiB more of output windows, 32 KiB each. The header copies
  of the three runs come to 20,664, 22,099 and 19,803 bytes.
- **Tests,** in `codec/lazy/tests.rs`:
  - Checkpoints by input: runs of 160,000 empty fixed blocks, of stored
    blocks and of noise, where no span's input passes the input spacing plus
    a stored block, a byte is never read from a span of no output, and the
    trace of such a span names its bits as the whole trace does. Thinning
    doubles both spacings.
  - zstd and xz runs with one large frame or block, where it is a span of its
    own and the spans of small ones stay under twice the input spacing. The
    zstd run also has a 100 KB skippable frame that starts a span, and a
    40 KB skippable frame after the last frame. A read of that span names only
    the chunks of its frames, and its trace names only the first 64 KiB of
    the skippable frame, the next header and the frames.
  - A zstd frame and an xz block over `unit_input` are refused from a source
    with a resident limit, in one pass, in steps and told the run is large.
    The same runs index and read back from a copy in memory, and are held
    whole when they fit under the cap.
  - zstd and xz spans joined by thinning past `span_input` are refused when
    read or traced from such a source, before anything is read. An xz span
    is still read, a few blocks at a time, and no answer names more chunks
    than `span_input` covers. A long stretch between kept block starts is
    refused the same way.
  - `block(i)` through a source that holds none of the run gives every block
    when no start was thinned, and with thinned starts reads only for a
    block whose start or end was dropped.
  - Block traces with `span_steps` of 200: each is the trace that decoding
    to the block's end gives, and a coarse block reads only the chunks
    before the coarse point.
- **Part 5 tests changed on purpose.** A deflate span read no longer names
  the chunks of the block header, and a zstd span read starts at its first
  frame's own magic. `block(i)` no longer waits for chunks, so the part 5
  test of reads that wait counts block traces instead.

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

These changes landed with part 7b; see "Built (part 7b)".

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

### Built (part 7c, web)

Built on 2026-11-28. The waiting loops are in `web/src/chunkwait.ts`, which
the node tests load without the wasm.

- **`Doc.readLoaded(at, len)`** reads a range 4 MiB at a time
  (`PIECE_BYTES`). It loads the file chunks each piece's read names, and
  reads that piece again until it is complete. A deflate span holds at most
  8 MiB of output, so a piece touches at most 16 MiB of spans when they are
  at least as long as the piece, and under 12 MiB when they are shorter.
  Either way, the spans its earlier rounds decoded are still in the span
  cache at its last round. It is used on the file as well as on tabs, by the
  hex view's copy, the image card, the stride view's sample and `openplan`'s
  `loadBytes`. So a stored run over 32 MiB now opens as a tab, and a picture
  of 32 to 64 MiB decodes. The inspector loads a checked run before
  `runCheckLoaded` only on the file, because on a tab the run's length counts
  bytes of the stream, not of the file.
- **One read per stretch.** `fetchChunks` groups the chunks a reply or a
  read names into stretches of neighbours, at most 256 chunks each, and reads
  each stretch with one blob read and one `notify`. `settle`, `read`,
  `readBits` and `fileChunks` use it, and `fetchRun`'s read-ahead is
  unchanged. A failed read is a `ReadFailure` for the whole stretch.
- **When a wait gives up** (`ChunkWait`). A round that asks for a chunk
  loaded in the same wait since the core last got further ends the wait. Any
  other round goes on. The backstop is `roundLimit`: as many rounds as the
  file has chunks, and at least `MAX_ROUNDS`. One round loads at most half
  the store, the first chunks named. `ensureTabRange`, `textWindow`,
  `textIndex`, `textBack` (which used to stop after 3 rounds) and
  `stringsScan` use this rule. `runCheckLoaded` and Open unpacked use
  `waitToOpen` with `roundLimit`, and `runCheckLoaded` passes
  `Doc.unpacking()` with each reply.
- **Left as they were:** `dumpScan` and `dumpBytes` still read a tab in one
  call, and `ensureRange` on the file still fetches one chunk at a time.
  `stringsScan` on the file still loads one chunk per round, since the core
  stops at the first chunk it has not got; a read-ahead there would speed up
  a scan of a large file.
- **Strings.** None was added. `REPORT.imageFailed`, "Couldn't decode this
  image", now also shows when the picture's bytes did not load, where the
  card used to say "Loading the image's bytes…" for ever. The words are not
  quite right for that case.
- **Checked in the browser** (2026-11-28, before part 7c's core half), with a
  gzip of 43 MB that unpacks to 74 MB, a zip with a stored entry of 40 MB,
  and a BMP of 42 MB: the listing filled in; `readLoaded` gave the right
  bytes for 256 KiB, 20 MiB and the whole of the unpacked tab and for the
  whole 43 MB file; the text window, text back and a strings scan to the end
  worked on the tab and on the file; the gzip CRC and the zip entry's CRC
  over 40 MB checked; the stored entry opened as a tab with the right bytes;
  and the BMP's picture decoded, 4,000 by 3,500 pixels.

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
order. Each part needs only the parts before it. "Review before part 7"
changes parts 6 to 10: read it first.

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
   run. **Done 2026-11-28.** `inflate::step_bytes` gives the bytes of the run
   a step's bits are in, and `Evaluator::field_bytes_at` reads them, so the
   wasm crate's `decode_step` calls `decode_step_at` with those bytes. A step
   that is not a code, such as a stored block's payload or a coarse block's
   one step, now answers null without reading any of the run; before, the
   run up to the step was read first, and could be refused as too much at
   once. Tested by
   `a_code_deep_in_a_run_longer_than_the_store_is_read_from_its_own_bytes`
   in `crates/wasm/tests/open_space_waits.rs`: a store of 4 chunks, a run of
   20, and a code three quarters of the way along it. And for a run read
   through an index, whose trace is its block's, by
   `a_code_deep_in_an_indexed_run_is_read_from_its_own_bytes`: a store of 2
   chunks and the zlib of 70 MiB of tar, about 7 chunks.
5. **`Missing` as "not yet".** `RunInput`; the `Seekable` keeps one, made over
   its copy (an `ArcSource`), so that until part 7 no read of a `Seekable`
   finds a chunk missing; `ReadError`; every row of the table in "Everything
   that has to treat `Missing` as 'not yet'". Nothing a reader sees changes.
   **Done 2026-11-28**; see "Built (part 5)" under "A `Seekable` reading
   through its `RunInput`".
6. **Bound what a `Seekable` read needs.** Checkpoints by compressed
   distance, and at the start of large frames and blocks; header copies;
   BFINAL and BTYPE with the block starts; block traces that stop when coarse;
   `group`'s bound; `Limits::unit_input` and `Limits::span_input` and their
   refusals. The spans of runs that pack badly change; their bytes and traces
   stay the same. **Done 2026-11-28**, with the limits held to only when the
   source has a resident limit, as "Review before part 7" asks; see "Built
   (part 6)" under "A `Seekable` reading through its `RunInput`".
7. **Share the source.** `Source::share`, `ChunkStore`'s chunks behind an
   `Arc<Mutex<…>>`, `Document::source_offset_of`, `Evaluator::run_input`. The
   deflate and zstd passes read their windows through the `RunInput` and
   answer `NeedsInput`, and `index_pass` keeps the pass across `Pending`.
   `Limits::input` replaces the 64 MiB compressed limit in the five places
   listed in "The problem", and `Indexes` weighs what each index holds for as
   long as it is kept. On the web side, `waitToOpen`, the status line and
   `ensureRange`, as in "The browser". Fixes: runs over 64 MiB compressed,
   and the memory of every run that has a handle. "Review before part 7"
   splits this into 7a, 7b and 7c. **7a done 2026-11-28**: `Source::share`,
   the shared store, `source_offset_of`, `run_input`, every index reading
   its run through the `RunInput` that `run_input` gives once the pass is
   over, and a tab passing the store's limit on. The passes still read a
   copy of the run, and the 64 MiB limit is unchanged. See "Built (part 7a)"
   under "Review before part 7". **7b done 2026-11-28**: the deflate, zlib,
   gzip and zstd passes read their run through its handle a window at a
   time, answer `NeedsInput`, and wait in `Unfinished` across `Pending`,
   when the source has room for two windows; any other pass still reads a
   copy. xz, the 64 MiB limit and the web are unchanged. See "Built (part
   7b)" under "Review before part 7".
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

## Review before part 7 (2026-11-28)

A review of this design against the code, before part 7 was built, found the
shared store sound. No lock is held across a read of input, so the `Mutex` is
never entered twice on the one thread, and nothing in the store points back at
a tab or an evaluator. It asked for the changes below, which are now part of
the plan.

**Before part 7 lifts the 64 MiB compressed limit:**

- **A tab passes the store's limit on.** `SpaceSource::Lazy` and
  `SpaceSource::Joined` answer `resident_limit()` with the span cache's size
  when the source under their `RunInput` has a limit, and `ChunkStore::behind`
  and `RunInput` pass that limit on. `file_bytes_at_once` counts a lazy
  space's bytes. Today `ChunkStore::behind` answers `None` and `SpaceSource`
  does not override it, so a reading over a tab skips part 1's check and part
  2's gathering. Once a tab's missing chunks are the file's, a whole-run read
  in a tab (a 40 MiB bzip2 member in a .tar.gz, a stored zip entry's checksum)
  would wait for ever.
- **The fallback copy keeps the 64 MiB limit.** `run_input` refuses a copy
  over `CAP_BYTES` as `TooLarge`, a `RunInput` reports the bytes it owns, and
  `Indexes` weighs them. Native tests read through an `ArcSource`, which
  shares, so the handle path runs outside the browser too. `MemSource` gives
  no handle.

**Before the web side of part 7 ships:**

- **The text loops fetch the file's chunks.** `textWindow`, the line index,
  `textBack` and the strings scan in `web/src/doc.ts` pass each missing chunk
  to `ensureRange(c * CHUNK_SIZE, CHUNK_SIZE)`, which is a range of the tab.
  After part 7 the chunks are the file's, so these call `fileChunks(missing)`.
- **One fetch per stretch of missing chunks.** `RunInput::read` asks which
  chunks are missing before it copies anything (a new `Source::missing`), and
  `Doc.read` fetches a stretch of neighbouring chunks with `fetchRun` and
  redraws once. A 16 MiB span fetched chunk by chunk is 256 fetches and 256
  redraws, and each redraw copies the chunks present so far.
- **Tabs read large ranges in pieces.** `hexview`, `inspector`, `contentcard`,
  `strideraster` and `openplan` await `ensureRange` and then read in one call.
  On a tab, a range whose input is larger than the store is never present all
  at once. They call a new `readLoaded(at, len)` instead, which reads a few
  MiB at a time, waits on `fileChunks` between pieces, and returns the bytes
  joined. `ensureRange` stays for the file's own tab.

**Part 6:** `unit_input` and `span_input` apply only when the source under the
`RunInput` has a resident limit. A fallback copy, an `ArcSource` and native
reads keep today's behaviour, so part 6 refuses nothing that opens today.

**Order:**

- Part 7 is merged in three steps. 7a: `Source::share`, the shared store,
  `source_offset_of`, `run_input`, and the `Seekable` reading through its
  handle, with the 64 MiB limit unchanged, plus the first two changes above.
  7b: the passes read windows, answer `NeedsInput`, and are kept across
  `Pending`. 7c: `Limits::input`, the `Indexes` weights, and the web changes
  above.
- Part 10 (joined streams) comes before part 8 (xz's layout). A BAM past
  64 MiB is common, and xz already works from a gathered copy.
- Part 9 is put off. A gzip member past 64 MiB of output is never measured
  (`expr.rs`), so part 9 saves only the walk's copy. Dropping the walk's bytes
  before its current step saves most of that for less work.

**Later, not needed for part 7:**

- Any edit drops every index, and with runs up to 512 MiB that is many rounds
  for each open tab. `Indexes` could be keyed by source offset, length and
  codec for runs with a handle, since a handle reads the original source.
- `impl Source for &S` and `Gathered` (`gather.rs`) must forward `share`, or
  the copy is made with no sign of it.
- Runs in a stitched space (a PNG's zlib stream across IDAT chunks) and stored
  parts stay on the copy, so they stay at 64 MiB. Part 10 covers `Joined`,
  not `Stitch`.
- A .gz of several members where one passes 64 MiB of output is not measured.
- xz reads a block's whole LZMA2 data again on each later read of a span.
  `Lzma2::chunk_in` takes a window, so the prefix could read on from where it
  stopped.
- Every lazy space opened keeps its `Seekable`, with its 16 MiB span cache and
  its prefix cache, until `forget`. The new weighing does not count them.
- Every field read in the evaluator takes the store's lock. Measure listing
  speed before and after, as part 3 did.

### Built (part 7a)

**Built on 2026-11-28.** Every index the evaluator makes reads its run
through the `RunInput` that `Evaluator::run_input` gives, from the end of its
pass on. In the browser that input is a handle onto the file's chunk store,
for every run of the file that no edit touched, so part 6's limits now apply
there. The passes still read a copy of the run, gathered a window at a time
as in part 2, and let go of the copy when they end. The 64 MiB limit on a run's
compressed size is unchanged. Where the code differs from the text above:

- **The store.** `ChunkStore` keeps its chunks, and the clock that says
  which chunk was read longest ago, in one `Arc<Mutex<…>>` that every handle
  shares (`Kept` in `source.rs`). A read through a handle marks a chunk as
  read, as a read of the store does. The lock is held only while bytes are
  copied out of the chunks, and no source is read while it is held.
  `impl Source for Arc<S>` is new, so that `Gathered::share` can gather the
  same parts from a handle onto its source. `Document`'s own `Source` impl
  gives no handle, since its bytes are the edited ones; `run_input` asks the
  document's source for one.
- **`source_offset_of`** is built as designed, on `PieceTable::orig_bit_of`.
- **`run_input` and the pass's copy.** Until 7b a pass needs a copy of the
  run, and when no handle reaches the run, the index reads that same copy,
  so that the run is gathered once. A pass that reads its run through
  `gather_run` calls `run_input` first, and when `run_input` gives a copy,
  the pass reads that copy (`RunInput::held_copy`). A pass that reads its run
  into its place in `Unfinished` (`Held::Gathering`) asks only for a handle
  once the run is read (`Evaluator::shared_input`), and without one the
  index reads the pass's copy. The evaluator makes every pass with
  `Indexing::through` or `decode_or_index_through`, which "Built (part 6)"
  calls private and which are public now. `run_input` refuses a copy longer
  than `CAP_BYTES` as `TooLarge` before it reads anything, and never refuses
  a handle. A handle is made only for a run of whole bytes, and in a space
  other than the file, only for a run that starts on a byte.
- **Weighing.** `RunInput::owned_bytes` is the length of a copy, and 0 for a
  handle. `Indexes` weighs each index by that. An index over a handle
  therefore weighs nothing, and `Indexes` keeps any number of them, until 7c
  weighs what each index holds.
- **The limit a tab passes on.** `Seekable::resident_limit` is the size of
  the span cache when the index's input has a resident limit, and
  `SpaceSource::Joined` answers the least of its indexed parts' limits.
  `file_bytes_at_once` counts the bytes of a space read through an index.
  Counting those bytes changes one native path too: a packed part of a
  stitched stream,
  whose run is in such a space and longer than one gather window (1 MiB
  natively), is now unpacked through `index_run`, as a part whose run is in
  the file already was.
- **Part 6's refusals are `TooMuchAtOnce`.** A zstd frame or xz block whose
  compressed bytes pass `Limits::unit_input`, a zstd or xz span read past
  `Limits::span_input`, and a stretch between two kept deflate block starts
  past `unit_input` are refused as `Refusal::TooMuchAtOnce` (tag
  `too-much-at-once`), not `TooLarge`. Each applies only when the source
  under the `RunInput` has a resident limit, so its reason is that the
  editor cannot read that much of the file at once; the run is not too large
  to unpack. A joined stream with such a part is refused with the same tag,
  where a part refused as `TooLarge` was refused as `Failed` before.
- **A read in one call longer than an index keeps fails.** `read_indexed`,
  for a space read through an index, and `read_part_index`, for a part of a
  stitched space read through its index, fail a read that finds chunks
  missing and is longer than `Seekable::resident_limit`, with the failure
  `read_in` gives a read of the file that can never fit
  (`pending_unless_too_much`). Such a read, made in one call where a tab is
  read in the file's reading, used to ask for the same chunks for ever. A
  whole run is still read a window at a time, so it never gets there.
- **Strings.** Two strings changed, in `web/src`:
  - `JOINED.refused` for `too-much-at-once`, in the status bar after Open
    joined stream, now gives `JOINED.partTooMuchAtOnce`: "Couldn't open
    {tab}: one of its parts is too large for the editor to keep in memory
    at once." It gave `UNPACKED.tooMuchAtOnce`, which says the stream needs
    more than 32 MiB of the file at once, and is false for a frame of
    17 MiB.
  - `RV.refused` for `too-much-at-once`, a finding in the report, is now
    "Too large for the editor to keep in memory at once, so it was not
    unpacked". It said the stream takes up over 32 MiB of the file, which
    is false for the same frame. Neither takes the limit as an argument any
    more.

  The listing and the inspector already showed `DECODED_REFUSED`'s "too
  large for the editor to keep in memory at once" for the tag, with the
  hover "The editor keeps at most 32 MiB of the file in memory." Two
  existing strings show in new places. The failure "Can't load {needed} of
  the file at once: the editor keeps at most {limit} in memory" can show for
  a read of a stream read through an index, with the span cache's 16 MiB as
  the limit and "of the file" meaning the stream's bytes. And a zstd frame or
  an xz block over 16 MiB compressed now shows the words for
  `too-much-at-once`, where the hover's 32 MiB is the store and the limit
  that applied is 16 MiB.
- **Tests.** In `source.rs` and `gather.rs`: a handle onto a store reads
  chunks fed in after the handle was made, and finds missing the chunks the
  store pushed out; a chunk read through a handle counts as read when the
  store picks the chunk read longest ago to push out; which sources give a
  handle; a handle onto gathered parts reads the parts. In
  `document.rs`: `source_offset_of` with no edit, after an insert and a
  delete before the stretch, with an overwrite inside it, and after a 3-bit
  insert before it. In `eval/tests/run_input.rs`: a run of the file, a run
  moved by edits, a run in a space held whole and a run in a space read
  through an index are each read through a handle with no copy, and each
  reads back byte for byte after the pass, at places the span cache no
  longer holds; a run with an overwrite inside it, and a run in a stitched
  space, are read through a copy; a copy over `CAP_BYTES` is refused, and a
  handle that long is not. In `eval/tests/resident.rs`: a run longer than a
  store of 48 chunks of 1 KiB is indexed and then read from the store, and a
  read of a span whose chunks were pushed out waits for them. Over a tab
  whose index reads such a store, a whole run is gathered a window at a
  time, in the file's reading and in the tab's own, and a read longer than
  the span cache in one call is refused. With `file_bytes_at_once` counting
  nothing for a space read through an index, as before, that test fails its
  check of `Evaluator::gathers`, and without that check it asks for the same
  chunk a fourth time and fails there. A gzip past the cap, over a store of
  48 chunks, opens as a joined stream read a part at a time, whose one part
  is read through an index over the store; the joined stream's limit is the
  span cache, and reads of it wait for pushed-out chunks and then give the
  member's bytes. A zstd frame over a lowered `unit_input` is refused as
  `too-much-at-once` over the store, and opens from memory. A read in one
  call of all of a stream read through an index, and of all of a joined
  stream's indexed part, fails with the reason; without the new check each
  asks for the same chunk a fourth time and fails there. A read of 48 KiB
  of each still waits and reads. The space tests of
  streams past the cap, `lazy_real`, `decoded_real` and `compressed_real`
  now read their files through an `ArcSource`, so their indexes read
  through handles. In `crates/wasm/tests/open_space_waits.rs`,
  `an_indexed_stream_reads_its_run_through_the_store` opens a 70 MiB zlib
  stream of a byte pattern over a store of 4 chunks. A read of the tab's
  last bytes names no chunks, since the pass read the chunks under them
  last. A read a fifth of the way along the tab names chunks the store has
  pushed out, and gives the pattern once those chunks are fed.
- **A test changed on purpose.**
  `a_code_deep_in_an_indexed_run_is_read_from_its_own_bytes` keeps 2 chunks,
  and its run was one deflate block of about 7 chunks. A block's trace now
  reads the block through the store, up to where the trace turns coarse, and
  a block of 7 chunks never fits in a store of 2. The run is now written in
  blocks of 20,000 copies of 258 bytes each (length 258, distance 1), about
  32 KiB of the run per block. The test makes the trace of the code's block
  once, before it feeds other chunks to push the code's chunk out.
- **Speed.** Listing through a chunk store takes as long as before, within
  the noise. Measured on 2026-11-28 with `examples/listing_timing.rs`, in
  release builds with LTO off (`CARGO_PROFILE_RELEASE_LTO=false` and 16
  codegen units), pinned to one core, before and after by turns, four runs
  of each. Each run gives the fastest of 7 tries (9 for the last two files),
  and the table gives the fastest run. The load average was 3.5 to 5.5 on
  12 cores. The "in memory" columns read the file from a `MemSource`, which
  gives no handle, so every index there reads a copy, before and after.
  Through the store, the Rubin raw's index reads a copy before, and the
  store through a handle after.

  | File | Rows listed | Through the store, before | After | In memory, before | After |
  |---|---|---|---|---|---|
  | The Rubin raw, gzip, and its FITS tab | 11,722 | 526.8 ms | 525.4 ms | 527.5 ms | 524.4 ms |
  | `wad/freedm.wad`, 22 MB | 20,003 | 66.0 ms | 64.3 ms | 64.9 ms | 62.6 ms |
  | `epub/the-bird-book-pg30000.epub`, 50 MB | 1,347 | 98.7 ms | 98.8 ms | 98.1 ms | 98.5 ms |

- **Left for 7b and 7c**, as planned: the passes reading windows,
  `NeedsInput`, `Limits::input`, the weights in `Indexes`, and the rest of
  the web changes. One web change came forward into 7a, since without it a
  view on a tab could show an incomplete read: `ensureRange` on a tab now
  reads the range, loads the file chunks the read names, and reads again
  until the read is complete, stopping as `waitToOpen` does when a chunk it
  loaded is asked for again (`Doc.ensureTabRange`, as "`ensureRange` on a
  tab" describes). Before, it asked `has_chunk` of the tab, whose store always
  answers yes, and loaded nothing. A range whose input is more than the store
  keeps still comes back incomplete; `readLoaded` in 7c reads such a range in
  pieces. "A joined stream walked twice" in
  "Still to do" can happen from 7a on: an index over a part of a joined
  stream now reads the store, so `template_for` can answer `Pending` after
  the walk has finished. `Doc.read`, `textWindow`, `textIndex`, `textBack` and
  `stringsScan` already fetch the chunks a read names as chunks of the file.

### Built (part 7b)

**Built on 2026-11-28.** A deflate, zlib, gzip or zstd pass now reads its run
through the run's handle a window at a time, and holds no copy of the run,
when a handle reaches the run and the source has room for a window. In the
browser that is every such run of the file that no edit touched, read
through the file's chunk store. When a step's window is missing chunks of
the file, the step answers `Stepped::NeedsInput` with those chunks, and
`index_pass` answers `EvalError::Pending` with them, as the pseudocode in
"How `Pending` fits the go machinery" shows. xz runs are still gathered into
one copy, and the stream is laid out from that copy. The 64 MiB limit on a
run's compressed size and `stream_len_at` are unchanged. Where the code
differs from the text above:

- **The step's window.** A step reads from the byte that holds its first
  bit up to `Limits::step_input` bytes past that byte, plus the decoder's
  margin: `DEFLATE_MARGIN` (65,541 bytes) or `ZSTD_MARGIN` (128 KiB + 25
  bytes). `step_input` is 256 KiB (`STEP_INPUT`) by default. `step_input` is
  a field of `Limits` so that the tests can read runs of a few hundred
  kilobytes in dozens of windows. A step stops at the first place the
  decoder can stop at or past `step_input` bytes from its start, whatever
  `in_bits` value the caller passes, even when its window reaches the end
  of the deflate data. When the window reaches the end of the deflate
  data, the step reads on to the end of the run, so that a copy of a block
  header near the end has the same bytes as one made from the whole run.
  Between steps the pass keeps the bytes from where the last step stopped
  (`PassWindow` in `input.rs`), and the next step reads only the bytes of
  its window that the pass does not hold already.
- **The first step** reads the run's last 8 bytes for zlib and gzip, and
  nothing for raw deflate. It reads the start of the run in the same read
  as its first window, so that the wrapper's header and the first window
  arrive in one round, and it names the missing chunks of both in one
  answer. When `inflate::framing` says the gzip header goes on past the
  bytes read, the step reads twice as many bytes, but no more than half of
  the source's resident limit. A gzip header longer than half of the
  source's resident limit is refused as `Refusal::TooMuchAtOnce`. The design
  set no limit on the header's length. Only a gzip name or comment can make
  a header that long, and without the limit the pass would ask for the same
  chunks for ever. The first step also decides, from the gzip trailer's
  ISIZE, whether the pass starts as `known_large`. A call of
  `Indexing::known_large` made before the first step is kept until then
  (`Phase::Start`).
- **Header copies** are made during the pass. When a step ends, each
  checkpoint it took inside a Huffman-coded block gets a copy of that
  block's header. If the block started before the step, the copy is the one
  the pass already holds for that block. If the block started in the step,
  the copy is made from the step's window, which holds the block's header.
  The pass then keeps a copy of the header of the block the step stopped
  in, and the next step reads that header and its tables from the copy.
  When the output passes the cap, the pass goes back to the last checkpoint
  and takes that checkpoint's copy. Each copy has the same bytes as in part
  7a: up to `MAX_HEAD_BYTES` bytes, from the byte where the header starts,
  cut short at the end of the run. The checkpoints inside one block still
  share one copy.
- **zstd.** `ZstdWalk` skips the contents of a skippable frame without
  reading them. When a skipped frame ends past the window's end, the step
  reads a new window from the byte the walk has reached, and goes on while
  its `step_input` bytes last. So the pass reads the contents of a
  skippable frame only as far as the step's window happens to reach, at
  most one window. A block whose header claims more than `ZSTD_MARGIN`
  bytes is refused as failed before it is read. `ruzstd` refuses a block
  over 128 KiB as failed too.
- **Which passes read a window at a time.** `lazy::reads_in_windows`
  answers true when the source under the handle answers any read whole, or
  when its resident limit is at least twice what one step reads at once:
  655,370 bytes for deflate and 786,482 bytes for zstd, under the default
  limits. The browser's chunk store keeps 32 MiB, and a tab whose stream is
  read through an index passes on the index's span cache, 16 MiB, as its
  limit. A pass whose source has less room than that reads a copy of the
  run, as in part 7a (`Indexing::from_copy`, over a copy made by
  `gather_run` or `Held::Gathering`), and the index it makes still reads
  the handle once the pass is over. So does a pass that no handle reaches:
  a run in a stitched space, in an edited stretch, or in a `MemSource`. The
  design meant every pass over a handle to read windows. The tests use
  stores of 2 to 48 chunks, which cannot hold one window, and a step that
  needs more of the file than the store keeps would ask for the same chunks
  for ever.
- **The constructors.** `Indexing::through(codec, input, limits)` reads
  nothing when it is called. The pass and the index it makes both read
  `input`. `Indexing::from_copy(codec, packed, input, limits)` is what part
  7a called `through`: the pass reads `packed`, and the index reads
  `input`. `Indexing::new` and `decode_or_index_with` read a run held in
  memory, and `decode_or_index_through` keeps its signature and its pass
  reads `packed`.
- **In the evaluator,** `index_pass` starts a pass over a handle at once,
  with no gather. The pass takes one of the two places in `Unfinished` when
  the go has an allowance. It also takes one in a go with no allowance when
  the source has a resident limit, since a step can then wait for chunks.
  When a step answers `NeedsInput`, `index_steps` puts the pass back in its
  place and answers `Pending`, which never reaches `refused_runs`. When the
  pass is the call's own, `index_steps` also records the pass's progress,
  as it does when a go runs out, so `Evaluator::unpacking` answers while
  the pass waits for chunks.
- **What the listing and the status bar say while a pass waits.** Before
  7b the gather's "pending" replies all came before the pass's "working"
  replies. Now a "pending" reply comes each time a step's window reaches
  chunks that are not loaded, about once for every 3 MiB of the run in the
  browser (an estimate from `READ_AHEAD`). So that the listing and the
  status bar do not switch words from one reply to the next, a "pending"
  reply from a pass is answered the way a "working" one is, as "The
  browser" asks:
  - `slice_reply` gives a "pending" reply `ready` (the children before the
    first that waits) and `unpacking` when `Evaluator::unpacking` names a
    pass, and `Doc.handleReply` and `Flattener.kids` pass them on. The rows
    above the stream stay drawn, and its waiting row says
    `REPORT.unpacking` with its percentage. Before, the rows above it went
    and came back with each window.
  - `Doc.openSpace` returns the progress with a "pending" reply. The status
    bar says `UNPACKED.unpacking` on it once a pass has said how far it has
    got, and `UNPACKED.loading` only before that.
  - `waitToOpen` takes progress that has moved since the last reply as the
    core getting further, whether the reply is "pending" or "working": it
    clears the chunks loaded for the wait and starts `MAX_ROUNDS` again.
  Tested by
  `a_gzip_members_header_lists_while_its_stream_waits_for_the_next_window`
  in `crates/wasm/tests/open_space_waits.rs` and four tests at the end of
  `web/test/openwait.test.ts`. `runCheckLoaded` and the text loops still
  count rounds with no progress to go by; see "Still to do".
- **`UNFINISHED_PASSES` stays at 2.** A pass past the cap now holds less
  than 10 MiB: its checkpoints' windows (at most 8 MiB), its block starts
  (at most 1 MiB), its window of output, its window of the run and its
  header copies. A pass that might still come to a run held whole holds up
  to 64 MiB of output and its trace. A pass cannot be told apart from the
  other kind before it starts: a gzip pass learns whether its run is large
  from the trailer at its first step. Keeping more passes would need two
  kinds of place, and `wait_for_a_place` would have to carry on a pass that
  frees the kind of place the waiting call needs. That change is not
  simple, so it was not made. See "Still to do".
- **Speed and memory.** Measured on 2026-11-28 with
  `examples/pass_memory.rs`, in release builds with LTO off
  (`CARGO_PROFILE_RELEASE_LTO=false` and 16 codegen units), pinned to one
  core. The example makes the
  pass the evaluator makes for a run of the file, over an `ArcSource`
  handle, and steps it with the evaluator's step sizes until it ends.
  Before this part, the pass read a copy of the run, so the example made
  that copy as the evaluator's gather did. "Peak heap" is the most heap the
  example had allocated during the pass, less what it had allocated before
  the pass: the copy, the pass, and the index the pass makes. Before and
  after were run by turns, three to five times each, with 7 tries a run for
  the Rubin raw and 5 for the other file, and the table gives the fastest
  try. The peak was the same in every try. The load average was 4 to 9 on
  12 cores, and runs of one build varied by up to 30%, so the times are the
  same within the noise.

  | File | Packed | Unpacked | Steps, before | Steps, after | Time, before | Time, after | Peak heap, before | Peak heap, after |
  |---|---|---|---|---|---|---|---|---|
  | The Rubin raw, gzip | 94,940 B | 75,677,760 B | 289 | 289 | 310.6 ms | 314.1 ms | 2,992,244 B | 2,992,781 B |
  | Text and noise, gzip level 6 | 43,149,999 B | 74,211,974 B | 283 | 283 | 578.8 ms | 574.3 ms | 46,057,351 B | 3,497,327 B |

  The index each pass makes holds the same number of bytes before and
  after: 2,380,120 for the Rubin raw and 2,342,596 for the text and noise.
  The Rubin raw's run, 94,940 bytes, is shorter than one window, so its pass
  holds as much of the run as before. For the text and noise, the 43 MB copy
  is gone. The rest of its peak, about 1.15 MB, is the pass's window of the
  run, its window of output and the buffers the decoder works in.
  The second file is about 1.7 MB of CSV-like rows and then 1 MiB of noise,
  by turns, packed by Python's `gzip` at level 6.
- **Tests.** In `codec/lazy/tests.rs`, each pass reads a store of 1 KiB
  chunks with room for two windows of 8 KiB plus the margin, and is fed only
  the chunks each step names. Over deflate, zlib and gzip runs of more than
  400 KB, made of one block, of blocks every 3,000 bytes and of flushed
  blocks, and over zstd runs of many frames with skippable frames, each
  pass makes the same index as one window over the whole run in memory:
  the same checkpoints with their windows and header copies, the same block
  starts, sums and span traces. Under a larger cap each comes back whole,
  with the same bytes and trace. No answer names more than one window's
  chunks, no chunk is asked for more than twice, and the pass never holds
  more of the run than one window, the trailer and one header copy. A pass
  over a skippable frame of 900,000 bytes reads no more than one window of
  it. Over 400,000 empty blocks, steps given no limit on the bits they read
  still stop after their `step_input` bytes and one empty block. A wrong
  CRC, Adler-32 or ISIZE, and a run that is not a gzip, are refused as
  failed when read from a store. A gzip with a 60 KiB extra field and a
  100 KiB comment is indexed from a store; one with a 300 KiB comment is
  refused as too much at once there, and is indexed from memory. In
  `eval/tests/resident.rs`, a zlib run three times longer than a store of
  160 chunks opens in goes. Each `Pending` names at most one window's
  chunks, `Evaluator::unpacking` answers after each one with progress that
  never goes back, no refusal is kept, the pass starts once, and the index
  matches the one made in memory. Three such runs, each longer than a store
  of 240 chunks, asked for by turns, all open: the third waits for a place
  and carries the other two passes on while they wait for chunks, no
  refusal is kept, and no pass starts twice. In `crates/wasm/tests/open_space_waits.rs`,
  `open_unpacked_on_a_stream_longer_than_the_store_unpacks_it_a_window_at_a_time`
  opens a zlib stream of 52 chunks of 64 KiB over a store of 16.
  `Editor::unpacking` answers after every "pending" reply but the first,
  which waits for the zlib header before the pass starts. No reply names
  more than 6 chunks, and every chunk of the file is asked for once.
- **Tests changed on purpose.**
  `a_zip_entry_longer_than_the_store_lists_a_window_at_a_time` and
  `a_gzip_member_longer_than_the_store_is_measured_and_its_stream_reads`
  keep 16 chunks, so their passes now read through the store, a step's
  window of up to 6 chunks at a time. They allowed a reply of at most 4
  chunks, the gather window, and now allow 6.
- **Strings.** None was added or changed. A gzip header longer than half
  of the store is refused with the existing words for `too-much-at-once`:
  "too large for the editor to keep in memory at once".

## Still to do

- **A joined stream walked twice.** `join_whole` gives a finished walk to
  its caller and keeps nothing. If `open_space` then answers `Pending`
  because chunks under the stream's first bytes are missing
  (`template_for`), the next call walks the joined stream again from its
  first part. The second walk gives the same stream, so only time is lost.
  This cannot happen before part 7. A fix: `open_space` keeps the joined
  stream that `unpack` made until the next call for the same stream takes
  it.
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
  up to 64 MiB. Since part 7b a pass past the cap holds less than 10 MiB, so
  more of those could be kept, in places of a second kind, with
  `wait_for_a_place` carrying on a pass that frees the kind of place the
  waiting call needs. Part 7b left it at 2; see "Built (part 7b)". A listing
  that shows three large zip entries at once no longer starts a pass again
  from the beginning, as this item said before part 2: the third entry's call
  carries on one of the other two passes until a place is free, so the third
  entry shows no percentage until its own pass starts.
- **Rounds with no progress to go by.** `runCheckLoaded`, `ensureTabRange`
  and the text loops in `web/src/doc.ts` give up after `MAX_ROUNDS` loads
  however far the core gets. A check of a long run read a window at a time
  takes a round each time it waits for chunks, so once 7c lifts the 64 MiB
  limit a long enough run could reach the backstop. They could pass on
  progress as `waitToOpen`'s callers do.
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

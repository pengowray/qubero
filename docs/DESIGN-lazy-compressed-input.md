# Design: reading the compressed input of large runs a piece at a time

Written 2026-11-26 as a proposal, for developers, and built since. It
follows `docs/DESIGN-lazy-unpacking.md`, which describes the output side:
how a compressed run whose output is past `codec::CAP_BYTES` (64 MiB) is
indexed in steps and its output read a span at a time. This design does the
same for the compressed input, so that memory no longer grows with the
compressed size and the browser can open a run whose compressed bytes do
not fit in its chunk store.

**State, 2026-11-28.** Every part in "Order of work" is built except part 9,
which is deferred (see "Review before part 7"). The sections that describe a
part as proposed are kept as the design; a "Built" section after each says
where the code differs, and wins where the two disagree:

- Part 1: "The smaller fix first (part 1)". Part 2: "Gathering a run a
  window at a time (part 2)". Part 3: "Built (part 3)". Part 4: item 4 of
  "Order of work".
- Parts 5 and 6: "Built (part 5)" and "Built (part 6)".
- Part 7, split into 7a, 7b and 7c (see "Review before part 7"): "Built
  (part 7a)", "Built (part 7b)", "Built (part 7c, core)" and, under "The
  browser", "Built (part 7c, web)".
- Part 10: "Built (part 10)", and after it "Faster opening of joined
  streams".
- Part 8: "Built (part 8)".
- Part 11: DESIGN-lazy-unpacking.md was brought up to date with all of these
  on 2026-11-28.

"Still to do" and "Open questions" at the end list what this leaves open.
"What was checked" lists the code read to write the proposal.

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

This section describes the code before part 1, which the design changes.

The memory an indexed run takes for its output is bounded by the table in
DESIGN-lazy-unpacking's "Limits": checkpoints, span cache, traces. Before
part 1, the memory it took for its compressed input was not:

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
  compressed, as it does natively.
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
- **Gather a run a window at a time (part 2).** A run that is read in one call
  before part 2 is read a window at a time across goes, into a copy kept from
  one go to the next, with memory as before. This alone fixes both files
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
cut into pieces: the same memory as before part 7. The 32 MiB browser limit is
gone only because no single read needs all of it. Keeping only some slices, in
a bounded cache, needs a way to read a dropped slice again, which is A or C.

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
"Where a `RunInput` comes from"). D is also the first step on the way (part 2),
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
   answers `complete: false` and `fetchChunks` loads them as chunks of the
   file, one read for each stretch of neighbours (see "Built (part 7c,
   web)"). The hex view already draws an incomplete read as loading, and
   `notify` redraws it when the chunks land.
4. An evaluator reading the space through `read_in` gets
   `EvalError::Pending(chunks)` and calls again, as for any read of the file.
5. xz's `read_span` puts its part-decoded span (`lazy::xz::Prefix`) back in
   the cache when the read is missing chunks. Before part 5 it dropped the
   prefix on any error, which is right for damage and wasteful here.

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

Part 3. Nothing a reader sees changes. Part 3 made every caller pass the
whole run as one window; part 4 changed `decode_step`, and part 7 the index
passes and span reads, to pass windows.

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
  Where it sets the reader's position to `Resume.in_bit` before part 3
  (`skip`), it changes to the code window instead.
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
steps; see "xz" below and "Built (part 8)".

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
- **`Seekable`'s deflate reads take their input** as a parameter, so that part
  7 can pass windows: `deflate_span`, `deflate_span_trace`, `group_from` and
  `block_trace_from`. At part 3 each was passed `Input::whole`; since part 7
  they are passed windows.
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

The `Send + Sync` bound keeps `Seekable` `Send + Sync`, as it was before part
5. `RunInput` lives in `codec::lazy` and knows nothing of the evaluator: a
space is passed to it as a `Source`, which `SpaceSource` already is.

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
  clock, which before part 7a was in `Cell`s (built as `Kept` in
  `source.rs`); the wasm build is single-threaded, so the lock is never
  contended. A store with a source behind it gives that source. `ArcSource`
  and `SpaceSource` give a clone of themselves. `MemSource` gives none: its
  field is public, there were 1,042 `MemSource(...)` constructions to change,
  and it holds the file in memory anyway, so a copy costs what it did before.
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

As proposed for part 7b. "Built (part 7b)" says where the code differs: the
first step reads the head and the first window in one read, a long gzip
header is refused as too much at once, and header copies are made during
the pass.

A deflate step:

1. Works out its window: from the byte holding the bit where the step starts,
   to `STEP_INPUT` (256 KiB) + `DEFLATE_MARGIN` past it, or to the end of the
   deflate data if that comes first.
2. Reads the part of the window it does not already hold, through the
   `RunInput`. If chunks are missing, it returns them and changes nothing.
3. Decodes with `stop_at` set `DEFLATE_MARGIN` before the window's end (or no
   `stop_at` when the window reaches the end of the data), and with its output
   budget of `INDEX_STEP_BYTES` as before, and stops at whichever comes first.
4. Keeps the window's bytes from where it stopped onward, and the copy of the
   current block's header, and drops the rest.

So a step reads at most about 320 KiB of new input, and the pass holds at
most that much. Because a step is bounded by input as well as by output, a
run of empty blocks, which reads input and makes no output, is bounded too.

The first step also reads the run's last 8 bytes, once, and keeps them. For
gzip, they hold `ISIZE`, which before part 7b `DeflatePass::new` read to
decide whether the pass starts as `known_large`; that decision moves to the
first step, since `Indexing::new` no longer has any bytes. They also hold the
trailer the last step checks. And the first step grows its read of the run's
head until `framing` can parse the wrapper.

A zstd step does the same with the zstd margin. A frame whose compressed size
passes `Limits::unit_input` (16 MiB) is refused, the way a frame whose output
passes `Limits::span` is; that limit and its refusals come in part 6, and
since part 7a the refusal is `TooMuchAtOnce`. The size counted is the
frame's own, without the skippable frames in front of it.

### How `Pending` fits the go machinery

`Indexing::step` gets a third answer beside `Working` and `Done`:
`NeedsInput(chunks)`. A refusal stays an error, as before. `index_pass`
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
  reach at most 8 MiB, three doublings from 1 MiB. Before part 6 the output
  spacing reached at most 4 MiB; a run that is large on both sides now gets
  spans of up to 8 MiB of output, two of which fill the 16 MiB span cache.
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
  pass learns a frame's size when the frame ends and puts the checkpoint in at
  the frame's start then. Before thinning, a span is then either one frame or
  block of at most `unit_input` (16 MiB), or smaller ones that come to at most
  twice `in_spacing`. Thinning can join two spans into one larger than that.
  So a zstd or xz span read checks the span's compressed size against
  `Limits::span_input` (16 MiB) before it reads anything, and a span over it
  is refused when read, as `TooMuchAtOnce` since part 7a, instead of asking
  for more chunks than the store keeps. Thinning needs more than 256 frames or
  blocks, and the tools do not write runs that reach the limit that way:
  `pzstd -1` writes frames of 2 MiB of output, and `xz -T0 -0` blocks of 1
  MiB, so their spans stay a few MiB of input even after thinning.
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
it did before part 6. It reads its window again from that checkpoint's bit,
which is at most `in_spacing` back; the steps just before had loaded those
chunks.

### xz

Part 8, as proposed. It was built on 2026-11-28, and "Built (part 8)" says
where the code differs from this list. xz's layout becomes a pass in steps,
so it joins `lazy::can_step` and is charged and reports progress like the
others:

1. The stream header and the footer.
2. The stream's index, whose length the footer gives. An index longer than
   `Limits::unit_input` is treated as one `layout` cannot read, which sends
   the stream to the decoder of whole streams. As built, a stream whose index
   lists more than `Limits::block_marks` blocks is refused as
   `TooManyXzBlocks` instead.
3. The block headers, up to 64 per step. Each is read where the index puts
   it, and the missing chunks of all 64 are reported together. So a stream of
   a thousand blocks spread over 300 MiB loads its headers in about 16
   rounds, not a thousand. As built, one step's headers also lie within half
   of what the store keeps of each other, 16 MiB in the browser, which for
   that stream is about 53 headers a step, so about 20 rounds.

A block whose compressed size passes `Limits::unit_input` is refused (part 6
adds this, as `TooMuchAtOnce` since part 7a). Before part 8, an xz run was
gathered whole (part 2) and laid out from the copy.

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
| `group(g)` | from kept block start g to kept block start g+1. Block starts are thinned once a run has more than `MAX_BLOCK_MARKS` (65,536) blocks, so this only arises then, and only a hostile run then has a long stretch between two kept starts. A stretch longer than `Limits::unit_input` is refused, as `TooMuchAtOnce` since part 7a |
| `block_trace(i)` | from the block's start until its trace turns coarse: `span_steps` is 1,000,000 trace steps, one per code, so at most 6,000,000 bytes. The decode stops there and closes the block with one opaque step to the block's end, which the index knows. Before part 6 it decoded to the block's end without recording the codes, and the trace came out the same |

### Everything that has to treat `Missing` as "not yet"

Part 5. Before part 5 these turned a failed read into a refusal, an empty
answer or zeroes. Each must pass `Missing` on instead:

| Where | Before part 5 | With `Missing` |
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
  first part to make such an index. Part 10 fixed it: see the item "A joined
  stream walked twice is fixed" in "Built (part 10)".
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
nothing that opened then was refused, and only the tests reach the
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

`stream_len_at` measures a member within the rest of the file from the
run's start, up to 64 MiB, since the member's length is what is being found.
Before part 1 it read all of that in one `read_in`. The proposal for part 2
gathered that read in stretches that double, and ran `inflate_len` after
each. As built, the walk that measures the member (`inflate::Measuring`,
made a step at a time as DESIGN-lazy-unpacking's "Measuring a member's
stream in steps" describes) reads its bytes a window at a time as it
reaches them, about 193 KiB past where each step starts. So a small member
costs a small read, and each member is decoded once. See "Gathering a run a
window at a time (part 2)".

Part 9, deferred, would replace the walk's own copy of those bytes, up to
64 MiB, with reads through a `RunInput` over the stretch `stream_len_at`
measures in. The walk is already kept from one go to the next in `Measures`,
beside `Unfinished`, with places for two, charged per step and reported by
`Evaluator::unpacking`. Its output cap stays 64 MiB (`Limits::measure`), for
the reason DESIGN-lazy-unpacking gives: a larger cap would let a hostile
file cost a full 1 GiB decode once to size the member and again to index
it. See "Review before part 7" for why part 9 is deferred, and "The
measuring walk of a large gzip member" in "Still to do".

A .gz of one member past 64 MiB of output makes two passes, one to size the
member, which the walk gives up on at 64 MiB of output, and one to index the
part of `decoded` over it. The listing's percentage goes up once for each.

## Joined streams

Part 10, after part 7, as proposed. "Built (part 10)" and, after it,
"Faster opening of joined streams" say where the code differs. `join_whole`
and `join_lazily` are now one walk, `JoinWalk` in `stitch.rs`.

- `JoinedBytes::Held(Arc<Vec<u8>>)` becomes `JoinedBytes::Stored(RunInput)`.
- `JoinedBytes::Packed { run, codec }` becomes `Packed { input, codec }`. The
  run is read through the handle when the stream's part is decoded; a BGZF
  part is under 64 KiB compressed.
- `JoinedBytes::Indexed` keeps its `Seekable`, which reads its own input.
- The 64 MiB limit on what `join_lazily` reads is removed. As built,
  `whole_cap` still holds the copies the stream makes of parts that no
  handle reaches.

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
  zeroes there. Since "Faster opening of joined streams" the walk does not
  make the check, and a read of such a part fails and says why.
- `unpacked_part` reads a packed part whole through `unpack_run`. For a codec
  that can be indexed, `unpacked_part` calls `index_run` instead, which steps;
  for any other codec, `unpack_run`'s read is gathered. As built, only a run
  that needs more of the file at once than one gather window goes through
  `index_run`; a BGZF block is read and decoded in one call.

## Limits

| Limit | Value | Status |
|---|---|---|
| `LAZY_CAP_BYTES` | 1 GiB of output | Stays. The zip-bomb guard. Refused past it as `TooLargeLazy` (`too-large-lazy`). |
| `CAP_BYTES`, as the output of a run held whole | 64 MiB | Stays. Refused past it as `TooLargeOutput` (`too-large-output`). |
| `CAP_BYTES`, as the compressed size of a run that can be indexed | 64 MiB | Since part 7c, replaced by `Limits::input`, 512 MiB, for a pass that reads its run through a handle a window at a time. A pass that reads a copy of its run keeps 64 MiB, as `Limits::copy_input`: a run no handle reaches, and a run in a source with too little room for two windows. Since part 8 an xz stream read through a handle is laid out a step at a time and takes `Limits::input` too; one decoded whole (64 MiB or less unpacked, or with an index or footer that does not hold together) is copied and keeps 64 MiB. |
| `CAP_BYTES`, as the input and output of a codec that cannot be indexed | 64 MiB | Stays. The input is gathered a window at a time (part 2). Refused past it as `TooLargeCopy` (`too-large-copy`) for the input and `TooLargeOutput` for the output. |
| `inflate_len`'s output cap | 64 MiB | Stays. |
| `Limits::span` | 64 MiB of output per zstd frame or xz block | Stays. Refused past it as `TooLargeUnit` (`too-large-zstd-frame`, `too-large-xz-block`). |
| `MAX_WINDOW` in `codec/frames.rs` | a zstd window of 128 MiB | Stays. A frame asking for more is refused as `TooLargeWindow` (`too-large-zstd-window`) before any of it is decoded, so nothing says how large it is. |
| `INDEXED_RUN_BYTES` | 64 MiB of what indexes hold | Stays at 64 MiB. Since part 7c it weighs each index by what it holds for as long as it is kept (`Seekable::kept_bytes`): its checkpoints' 32 KiB windows (at most 8 MiB), its header copies (under 72 KiB), its block starts (at most 1 MiB), and the copy of its run when it holds one: about 9.1 MiB at most for deflate and zstd. An xz stream's list of blocks is weighed too, at 32 bytes a block natively and 24 in wasm. Since part 8 a stream may list at most `Limits::block_marks` (65,536) blocks, 2 MiB of list natively, or is refused as `too-many-xz-blocks`. `xz -T0 -6` writes a block for every 24 MiB of output, 43 blocks for 1 GiB. The caches inside each index are bounded as before, and not weighed. |
| `whole_cap`, as the input of a joined document read a part at a time | 64 MiB | Since part 10 it counts only what the joined stream copies: a stored part or a packed part's run that no handle reaches (in a `MemSource`, a stitched space or an edited stretch), a packed run decoded whole that is too long to read from the store in one read (next row), and the run of a packed part read through an index that holds a copy. A part read through a handle counts nothing. Since part 7c a packed part read through an index over a handle counted nothing. |
| A packed part's run read through a handle, for a part decoded whole (new) | the gather window (1 MiB in the browser), and an eighth of what the source keeps | Part 10 (`Evaluator::joined_input`). Each decode reads the whole run in one read. A longer run is copied, and counts against `whole_cap`. No limit when the source answers any read whole. A BGZF block is under 64 KiB. |
| One read of a joined stream read a part at a time (new) | Half of what the source keeps, less two of those runs, over `expand` | Part 10 (`Joined::resident_limit`), when a part is read through a handle onto a source with a resident limit. `expand` is the most bytes of run a packed part read through a handle takes per byte it unpacks to, and at least 1. For a BAM in the browser it is 16 MiB less two blocks. The least of that and the span cache of each part read through an index. |
| `Limits::input` (new) | 512 MiB of compressed input | Built in part 7c, for deflate, zlib, gzip and zstd, and in part 8 for xz. A trace keeps each step's bit in 32 bits (`RawStep.in_start`, and `pack` truncates without a word), and 2^32 bits is 512 MiB; a run of exactly 512 MiB is allowed. A longer run read through a handle is refused as `Refusal::TooLargeInput` (`too-large-input`) before anything is read, since the size of the field holding the run is known. |
| `Limits::copy_input` (new) | 64 MiB (`CAP_BYTES`) of compressed input | Part 7c. The most a run may take up when its pass reads a copy of it. A longer one is refused as `TooLargeCopy` (`too-large-copy`) before anything is read. A field of `Limits` so that a test can show a run past it opening through a handle. |
| `IN_SPACING_START` (new) | 1 MiB of compressed input between checkpoints | Doubled with `SPACING_START` when checkpoints are thinned, to at most 8 MiB. |
| `Limits::unit_input` (new) | 16 MiB of compressed input | The most one zstd frame (without the skippable frames in front of it) or one xz block may be, and the most one stretch between two kept deflate block starts may be (`group`). Since part 6, a frame, block or stretch over it is refused as `TooMuchAtOnce` (`too-much-at-once`), and only when the source has a resident limit. Part 8 does not hold an xz stream's index to it: `Limits::block_marks` limits the number of blocks, and so how much of the index is read. |
| `Limits::span_input` (new) | 16 MiB of compressed input | The most one read of a zstd or xz span may gather at once. Since part 6, a span over it is refused when read, as `TooMuchAtOnce`, and only when the source has a resident limit. Deflate spans never reach it (8.5 MiB at most). |
| `STEP_INPUT` (new) | 256 KiB of compressed input | How far past its start a step may stop; its window reaches `DEFLATE_MARGIN` further. Beside `INDEX_STEP_BYTES`, 256 KiB of output. An xz step reads this much of the stream's index, with no margin. |
| `HEADERS_PER_STEP` (new) | 64 xz block headers | Part 8. The most block headers one xz step reads, and names the missing chunks of in one answer. The headers of one step also lie within half of what the source keeps of the first, so that their chunks fit in the store together. |
| `Limits::block_marks`, as the most blocks of an xz stream (new) | 65,536 blocks | Part 8. An xz stream whose index lists more is refused as `TooManyXzBlocks` (`too-many-xz-blocks`), "too many xz blocks to unpack (over 65,536 blocks)", from the index's count, whatever it unpacks to, read through a handle or from a copy. It bounds the list of blocks an index keeps to 2 MiB natively. For deflate the same limit is still the most block starts kept before they are thinned. |
| `COPY_STEPS` (new) | 4 times `STEP_INPUT`, 1 MiB | Part 8. The most of an xz stream one step copies when the stream is decoded whole, cut to a quarter of what the source keeps when that is less, though never to less than `STEP_INPUT`. The copy decodes nothing until it is complete, so a step can copy more than a step of a decode reads. |
| `DEFLATE_MARGIN`, and the zstd margin (new) | 65,541 bytes; 128 KiB + 25 bytes | How far past where a step stops its window has to reach. |
| `MAX_HEAD_BYTES` (new) | 287 bytes | A deflate block header and its tables, from the header's first byte. |
| `GATHER_WINDOW` (new) | 1 MiB, or a quarter of what the store keeps when that is less | Part 2, built without the floor of one chunk the proposal had. |
| `GATHER_BYTES` (new) | 64 MiB | Part 2: the most the copies in `Gathers` may set aside between them. |

**The zip-bomb guard.** Memory stays bounded by the table above and
DESIGN-lazy-unpacking's, so a hostile run can only cost time. That time is
the index pass over at most 1 GiB of output and 512 MiB of input, plus, for a
gzip member, the measuring pass over at most 64 MiB of output.

**`unit_input` is a choice.** In the browser it refuses some files that open
natively: a stream of at most 64 MiB compressed and more than 64 MiB unpacked,
with one frame or block of more than 16 MiB compressed. `xz -T0 -6` writes
blocks of 24 MiB of output, so data that packs at less than 1.5 to 1 with it
is refused. A larger value leaves less of the 32 MiB store for everything else
a span read competes with. Lifting it needs a span decode that keeps its
decoder between reads; see "Still to do".

### Refusal tags

Each limit above that refuses a run has a `Refusal` of its own, and the web
looks its words up by the refusal's tag (`Refusal::as_str`). The tags are listed once, in
`codec.rs`: a `refusal_tags!` list makes both `Refusal::as_str` and
`Refusal::ALL`, so a refusal added to the enum without a tag does not
compile. `web/src/refusaltags.ts` is generated from `Refusal::ALL`, and the
core test `refusal_tags_ts_matches_the_enum` fails while the checked-in file
differs, naming the command that writes it:

    UPDATE=1 cargo test -p qubero-core --lib refusal_tags

On the web side `DECODED_REFUSED` is a `Record<RefusalTag, string>`, and
`JOINED.refused`, `RV.refused`, `RV.pngNotInflated` and
`RV.pngNotUnfiltered` are a `switch` over every tag that ends in
`unknownRefusal`, so `tsc` fails on a tag with no words. The wasm still sends
a plain string, since a `src/pkg` built from another core may send a tag
this build does not know: `isRefusalTag` narrows it, and `refusedText` and
`refusedTitle` give the words and the hover.

## The browser

### What the listing and the status bar say

| While | The listing's waiting row | The status bar, after Open unpacked |
|---|---|---|
| a run is gathered (part 2) | `REPORT.reading`, "Loading bytes needed to map these fields…", which is accurate now that the gather finishes | `UNPACKED.loading` |
| the index pass runs, or waits for chunks (part 7) | `REPORT.unpacking`, "Unpacking the compressed data to read these fields… 37%" | `UNPACKED.unpacking`, on "pending" replies as well as on "working" ones |
| a read can never fit (part 1) | the refusal, in the value column of the field | the refusal, as `JOINED.refused` and `DECODED_REFUSED` word it |

No new string was needed for the first two rows. A percentage while a run is
gathered would help for a 64 MiB run; part 2 added none, and whether to add
one is in "Open questions".

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

These changes landed with part 7b; see "Built (part 7b)". The rule now
lives in `ChunkWait` (`chunkwait.ts`), and the backstop is `roundLimit`, as
many rounds as the file has chunks and never fewer than `MAX_ROUNDS`: see
"Built (part 7c, web)".

### `ensureRange` on a tab

`Doc.ensureRange` loads every chunk that `has_chunk(this.space, c)` says is
absent, and on a tab `has_chunk` never says a chunk is absent: a space's store
answers true for every chunk. So on a tab over an indexed run, `ensureRange`
resolves at once and the read after it comes back incomplete. For a space
other than 0, `ensureRange` should read the range, wait on `fileChunks` of the
chunks the read names, and repeat until the read is complete, giving up by the
same rule as `waitToOpen`. A read's missing chunks are exactly the file chunks
it waits on, so no new query of the core is needed. Built in part 7a as
`Doc.ensureTabRange` (see "Built (part 7a)"); `readLoaded` reads a long range
in pieces (see "Built (part 7c, web)").

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
  at 64 MiB. A member whose stream ends inside that is measured as before. A
  longer one measures as nothing, as a run past the cap does, and the gzip
  template's fallback places it as everything before its trailer, which is
  right for a file of one member.
- On the web side, `DECODED_REFUSED` and `JOINED.refused` get the new tag.
- What part 1 does not catch: a read that fits on its own, but not beside the
  other chunks a go needs, can still ask for the same chunks for ever.
  `waitToOpen`'s rule stays for that.

**Built on 2026-11-26.** Where the code differed from the list above when
part 1 was built. Part 2 changed several of these since: the joins refuse
through `too_large_to_join`, and `parts_cannot_fit` is gone; `stream_len_at`
reads a window at a time; and the test named at the end is now
`a_run_longer_than_the_store_opens_a_window_at_a_time`. See "Gathering a run
a window at a time (part 2)".

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
  `TooLarge` (now `TooLargeCopy`), as it is natively, and any other with
  `TooMuchAtOnce`.
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
its context, and checked against the code since. Both are in the code as
proposed: `Refusal::message` for `TooMuchAtOnce`, and the failure in
`eval/read.rs`.

| Where | Proposed string | Note |
|---|---|---|
| Value column of a compressed field, the inspector after ` · `, and `{field} is …` for a joined stream's part (`Refusal::message` and `DECODED_REFUSED["too-much-at-once"]`) | `too large for the editor to keep in memory at once` | The 32 MiB goes in the row's tooltip, for example "The editor keeps at most 32 MiB of the file in memory." If no tooltip is possible: `too large for the editor to keep in memory at once (limit 32 MiB)`. |
| The failure of any other read that cannot fit (`read_in`) | `Can't load {needed} of the file at once: the editor keeps at most 32 MiB in memory` | `{needed}` is the read's size, like `40 MiB`. It avoids the verb "read", which would be wrong for a checksum field whose own bytes can be read. |

The drafting agent also pointed out that the existing `too large to unpack
(over 64 MiB)` does not say what is over 64 MiB, and that beside a 32 MiB
phrase it can read as one limit with two values. At the time
`Refusal::TooLarge` was given both for a compressed run over 64 MiB and for
an output over 64 MiB, and this design would also give it for a run over
512 MiB compressed. It has since been split: see the first of "Open
questions", and "Refusal tags" under "Limits".

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
  pass. A run in a go with no allowance is read through `gather_run`
  rather than into a place. So was an xz run until part 8, which steps xz's
  pass and gives it a place like the others; since part 7b, a pass that
  reads its run through a handle gathers nothing.
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
  run from its front to a trace step (`decode_step`, until part 4 made it
  read only the step's own bytes), a JPEG scan's settings, and a template
  field read whole. The tests are
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
- **What could go wrong in the browser, until part 7b.** A .gz member longer
  than the store has its run read twice, once to measure it and once to unpack
  it, so the core asks again for chunks it read earlier. `waitToOpen` gives up
  when the core asks again for a chunk that `waitToOpen` loaded with no
  "working" reply in between. If Open unpacked is pressed while such a member
  is still being measured, its wait sees both reads. Measuring answers
  "working" between its rounds, but reading a run a window at a time answers
  only "pending" while its chunks load. So the second read, reaching chunks
  the end of the first read loaded and the store has since pushed out, can
  make `waitToOpen` give up with `UNPACKED.tooMuchAtOnce`. Pressing Open
  unpacked again then carries on from where the core stopped. The tests do not
  meet this, and it was not tried in a browser. Part 7b's change to
  `waitToOpen` removed the risk: a "pending" reply whose progress has moved
  counts as the core getting further.
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
  zstd, while the pass runs. An xz pass holds the stream's index while it
  reads the block headers, at most `Limits::unit_input` and a few hundred
  bytes for most streams, or its copy of a stream decoded whole;
- the header copies: under 72 KiB per run, for as long as the index is kept;
- the trailer: 8 bytes, while the pass runs;
- what a span read gathers: at most 8.5 MiB for deflate and 16 MiB for zstd
  or xz, for the length of the read;
- what a block trace gathers: at most 6,000,000 bytes, for the length of the
  trace.

The compressed bytes otherwise stay in the file's chunk store, 32 MiB in the
browser, which the file's tab keeps anyway. A run that falls back to a copy
(an edited run, a run in a stitched space, a run in a `MemSource`) holds that
copy, at most 64 MiB, as every run did before part 7. Gathering (part 2) sets
aside at most 64 MiB for the copies in `Gathers`, and never lets go of a copy
it has started. An index pass reads its run into its own place in
`Unfinished`, which holds the same copy once the pass starts.

Since part 10, a joined stream read a part at a time holds no copy of a part
that a handle reaches. Over handles it holds a `RunInput` for each part,
about 60 bytes with its handle, its cache of decoded parts (16 MiB,
`STITCH_CACHE_BYTES`), and one part's trace. A read of it holds, for the
length of the read, the run of the packed part it decodes, under 64 KiB for
a BGZF block, and that part's output until the cache takes it. Since
"Faster opening of joined streams" the walk that joins it decodes only the
parts `checked_when_joined` names, and holds one such run and its output at
a time. A part no handle reaches is copied, as every part was before part
10, and the copies are held to `whole_cap`.

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
   one call before part 2 is read `GATHER_WINDOW` at a time through `read_in`,
   into a copy kept in `Spaces` from one go to the next (at most 64 MiB of
   partial copies), charged per window. Used by `index_pass`, `open_space_at`,
   `unpack_run` and `covered_bytes`, and by `stream_len_at` in doubling
   stretches. `join_whole` and `join_lazily` become walks kept from one go to
   the next, and `unpacked_part` steps through `index_run` for codecs that can
   be indexed. Fixes, in the browser and with memory as before: the 34 MiB zip
   entry, a .gz of up to 64 MiB compressed, and a joined stream such as a BAM
   whose parts come to up to 64 MiB. **Done 2026-11-26**; see "Gathering a run
   a window at a time (part 2)".
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
   7b)" under "Review before part 7". **7c's core half done 2026-11-28**: a
   pass that reads its run through a handle a window at a time is held to
   `Limits::input`, 512 MiB, and refused past it as `too-large-input`; any
   other pass reads a copy and keeps the 64 MiB limit, now
   `Limits::copy_input`. A joined stream's part read through an index over
   a handle no longer counts against `whole_cap`. `Indexes` weighs what each
   index holds. `Source::missing` says which chunks a read would find
   missing, and `RunInput::read` asks it before it copies. So a .gz of one
   member and a zip entry past 64 MiB compressed open, natively and through
   the wasm `Editor`. See "Built (part 7c, core)" under "Review before part
   7". The web half (the text loops, `fetchRun`, `readLoaded`) is built
   too; see "Built (part 7c, web)" under "The browser".
8. **xz's layout in steps.** xz joins `can_step`. **Done 2026-11-28**: an
   xz stream read through a handle is laid out a step at a time from its
   footer, its index and its block headers, holds no copy of the stream,
   and may be up to 512 MiB packed. A stream decoded whole is copied a
   piece at a time and keeps the 64 MiB limit. See "Built (part 8)" under
   "Review before part 7".
9. **Measure a gzip member through a handle.** A pass of its own replaces
   the walk's copy of the member's bytes in `stream_len_at`. **Deferred**
   (see "Review before part 7"). The walk already steps, with
   `inflate::Measuring`, and reads its bytes a window at a time as it reaches
   them; see "Measuring a gzip member".
10. **Joined streams through handles.** `JoinedBytes` over `RunInput`, and
    `join_lazily` without its 64 MiB input limit. **Done 2026-11-28**: a
    joined stream read a part at a time reads each stored part, and the
    run of each packed part it decodes whole, through a handle onto the
    file, and copies only what no handle reaches, so only those copies
    count against `whole_cap`. "A joined stream walked twice" is fixed with
    it. See "Built (part 10)" under "Review before part 7".
11. **Bring DESIGN-lazy-unpacking up to date.** Its "State", the "Limits" row
    for `CAP_BYTES`, the paragraph at the end of "The browser", and "Still to
    do". **Done 2026-11-28**, for every part above and "Faster opening of
    joined streams", with the input side linked here rather than repeated.

Parts 1 and 2 fix both files seen. Parts 3 to 7 are the substance of the
design and make the most change; part 7 is the one that removes the 64 MiB
limit.

## Tests

This is the test plan as proposed. The tests as built, with their names, are
listed in each "Built" section, and those lists win where the two differ. A
few differ in kind: the size refusals are the ones in "Refusal tags", an xz
index is held to `Limits::block_marks`, the test that asserted the old
behaviour is now `a_run_longer_than_the_store_opens_a_window_at_a_time`, and
`web/test/lazyinput.browser.mjs` was not written: the browser checks were
made by hand (see "Built (part 7c, web)", "Built (part 8)" and "Faster
opening of joined streams").

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
  `TooLargeInput` (as built) before anything is read. A zstd frame, an xz
  block and a `group` stretch over `unit_input` are refused.
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
  one go to the next and refuses a part of the wrong length as before.

### The wasm crate (`crates/wasm/tests`)

These follow `open_space_waits.rs`: an `Editor` with a small store, fed only
the chunks each "pending" reply names, and called again on "working".

- **The stuck listing, reproduced.** A zip of one deflated entry whose
  compressed size is several times the store: for example a store of 16
  chunks and an entry of 4 MiB compressed, made from enough lines of hex, as
  `lines()` makes them, to pack to that. The test asks `template_children` for
  the entry's `data` and its contents, the way the listing does.
  - Before part 1: a chunk is asked for again after it was fed, within a few
    rounds, and the contents never come. The test asserts this first, so the
    case is known to be reproduced.
  - After part 1: the `data` field is refused with `too-much-at-once` within
    a few rounds.
  - After part 2 (a store of 16 chunks gives a gather window of 4 chunks) and
    again after part 7: the contents read within a bounded number of rounds,
    and no reply names more chunks than one window covers.
- **The .gz.** A gzip of one member larger than the store, read with the gzip
  template. Before part 2 `members[0].compressed` never got a size. After part
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
a tab or an evaluator. It asked for the changes below, which became part of
the plan and are built: each in the part the "Order" list below names.

**Before part 7 lifts the 64 MiB compressed limit:**

- **A tab passes the store's limit on.** `SpaceSource::Lazy` and
  `SpaceSource::Joined` answer `resident_limit()` with the span cache's size
  when the source under their `RunInput` has a limit, and `ChunkStore::behind`
  and `RunInput` pass that limit on. `file_bytes_at_once` counts a lazy
  space's bytes. Before part 7a, `ChunkStore::behind` answered `None` and
  `SpaceSource` did not override it, so a reading over a tab skipped part 1's
  check and part 2's gathering. Once a tab's missing chunks are the file's, a
  whole-run read in a tab (a 40 MiB bzip2 member in a .tar.gz, a stored zip
  entry's checksum) would wait for ever.
- **The fallback copy keeps the 64 MiB limit.** `run_input` refuses a copy
  over `CAP_BYTES` as `TooLarge` (now `TooLargeCopy`), a `RunInput` reports
  the bytes it owns, and
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
reads keep the behaviour from before part 6, so part 6 refused nothing that
opened before it.

**Order:**

- Part 7 is merged in three steps. 7a: `Source::share`, the shared store,
  `source_offset_of`, `run_input`, and the `Seekable` reading through its
  handle, with the 64 MiB limit unchanged, plus the first two changes above.
  7b: the passes read windows, answer `NeedsInput`, and are kept across
  `Pending`. 7c: `Limits::input`, the `Indexes` weights, and the web changes
  above.
- Part 10 (joined streams) comes before part 8 (xz's layout). A BAM past
  64 MiB is common, and xz then worked from a gathered copy.
- Part 9 is put off. A gzip member past 64 MiB of output is never measured
  (`expr.rs`), so part 9 saves only the walk's copy. Dropping the walk's bytes
  before its current step saves most of that for less work.

**Later, not needed for part 7:**

- Any edit drops every index, and with runs up to 512 MiB that is many rounds
  for each open tab. `Indexes` could be keyed by source offset, length and
  codec for runs with a handle, since a handle reads the original source.
- `impl Source for &S` and `Gathered` (`gather.rs`) must forward `share`, or
  the copy is made with no sign of it. Done in 7a.
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
  speed before and after, as part 3 did. Measured in 7a: the same within the
  noise (see "Built (part 7a)").

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

### Built (part 7c, core)

**Built on 2026-11-28.** A run read through a handle may now be up to
512 MiB compressed, so a .gz of one member and a deflated zip entry past
64 MiB compressed open, through the core and through the wasm `Editor`.
The web half of 7c (the text loops, `fetchRun`, `readLoaded`) is not part
of this; its notes go in "The browser". Where the code differs from the
text above:

- **Two limits, decided by the way the pass reads.** `Limits::input`
  (`INPUT_CAP_BYTES`, 512 MiB) is the limit for a pass that reads its run
  through a handle a window at a time, and `Limits::copy_input`
  (`CAP_BYTES`, 64 MiB) is the limit for every pass that reads a copy of
  its run: an xz run, a run no handle
  reaches, and a run in a source with too little room for two windows
  (`lazy::reads_in_windows`). The design named one limit. `copy_input` is
  a field so that a test can show a run past it opening through a handle,
  and its default is the 64 MiB of before. `lazy::too_long_to_index(len,
  through_handle, limits)` is the one comparison, used by `index_pass`
  (`through_handle` is whether the pass is windowed), by
  `Indexing::reading` (whether the pass reads a copy), by
  `decode_or_index_through` and by `run_input` (a copy). A copy is never
  held to `Limits::input`, so a copy path never shows the new words. A run
  of exactly 512 MiB is allowed: its last step starts below bit 2^32.
- **Where each run is refused.** `index_pass` works out whether the pass
  is windowed before anything else, and refuses a run too long for that
  way of reading before it waits for a place or gathers anything. So a run
  past 64 MiB with a handle that would be read through a copy is refused
  as `too-large` (now `too-large-copy`), and not as `too-much-at-once` by
  `GATHER_BYTES`.
  `open_space_at` now refuses past 64 MiB only a run of a codec that
  cannot be indexed, and leaves the rest to `index_pass`. `DeflatePass::new`
  had lost its check in 7b. `Indexing::new` reads a run held in memory, so
  it is held to `copy_input`.
- **The refusal.** `Refusal::TooLargeInput`, tag `too-large-input`, for a
  run past `Limits::input`. Its words, in `Refusal::message` and
  `DECODED_REFUSED`, were "too large to unpack (over 512 MiB compressed)",
  and are now "(over 512 MiB packed)" (see "Open questions").
  A joined stream with such a part is refused with the same tag
  (`join_part_lazily`), which `JOINED.refused` turns into
  `JOINED.partTooLargeInput`: "Couldn't open {tab}: one of its parts is
  over 512 MiB compressed." The check of a checksum over such a stream
  says "Can't check this: the data it covers is too large to unpack (over
  512 MiB packed)." The limit is on one run, not on a joined stream's parts
  together. `RV.refused` in `web/src/report/text.ts` had no words for the
  tag at first; it now says the listing's words with a capital, as for the
  other size refusals.
- **What else stopped at 64 MiB on the way to a tab.** Read on the path
  from a .gz or a zip entry past 64 MiB compressed to an open tab:
  - `too_large_to_join` summed every part's run against `whole_cap`, so
    Open joined stream on a .gz of one member past 64 MiB compressed was
    refused as `too-large` (now `too-large-copy`). It now counts only what
    the joined stream
    copies: stored parts, packed parts decoded whole, and packed parts read
    through an index that no handle reaches, whose index holds a copy. A
    packed part read through an index over a handle counts nothing. Which
    parts are read through an index is decided as `index_part` decides it.
  - `stream_len_at` reads at most 64 MiB of a member and counts at most
    `Limits::measure` (64 MiB) of output. A member past 64 MiB compressed
    comes to more than 64 MiB of output, so the walk is refused at the
    measuring cap and the gzip template takes the member to run to its
    trailer (`Remaining - 8`). A .gz of one member opens that way. The walk
    still reads and holds up to 64 MiB of the member first; see "Still to
    do".
  - `gather_run` and `GATHER_BYTES` are reached only by copy paths, which
    are held to `copy_input`, and are unchanged.
  - `run_check` over bytes of the file (`Coverage::over`), as for a zip
    entry stored as it is, still refuses past 64 MiB. A deflated entry's
    CRC-32 is the index's sum, and is checked at any length.
  - A tab's read needs one span's compressed bytes at once. A deflate span
    takes up at most 8.5 MiB and a zstd or xz span at most
    `Limits::span_input`, 16 MiB, within the browser's 32 MiB store. A
    tab's read from a store smaller than one span's bytes waits for ever;
    only tests make stores that small.
  The end-to-end check at full size is
  `a_gzip_member_and_a_zip_entry_past_64_mib_open_end_to_end` in
  `crates/wasm/tests/open_space_waits.rs`, marked `#[ignore]`: 65 MiB of
  runs in stored blocks, under the default limits, from a store of 4 MiB.
  It took 6 seconds and 290 MB in a debug build.
- **`Indexes` weighs what each index holds.** `Seekable::kept_bytes` is
  its checkpoints' windows, its header copies counted once each however
  many checkpoints share one, its block starts and an xz stream's list of
  blocks. It does not change once the index is made, so the weight is
  worked out once, in `Indexes::put`, with `RunInput::owned_bytes`. The
  caches are not weighed, as planned. `INDEXED_RUN_BYTES` stays 64 MiB; the
  cap is a field of `Indexes`, made small by tests (`set_index_cap`).
- **`Source::missing(offset, len)`** gives the chunks a `read_bytes` of
  the same bytes would report, sorted, without copying them. It may name
  fewer when finding them needs a decode, so a caller told nothing is
  missing still checks what the read reports; it never names a chunk the
  read would not. The default reads the bytes into a buffer of its own,
  which is right for any source. `MemSource`, `ArcSource`, `DumpSource`
  and `textnum`'s window answer none. The store's chunks (`Kept`) are
  looked up one by one, and a chunk found is marked as read, as a read
  marks it, so that a read waiting for the rest keeps the chunks it has.
  `ChunkStore`, `&S`, `Arc<S>` and `Gathered` forward it. `Document`
  forwards it through its edits (`PieceTable::missing_bits`).
  `SpaceSource` asks its index, which answers what its read would name:
  the chunks of the first span it reaches that is not held decoded and is
  missing some, worked out for xz from how far the span is decoded
  (`xz::span_missing`). `Joined` asks its first indexed part that is
  missing some. `RunInput::read` asks first and copies nothing when chunks
  are missing; it still honours what the read reports after an empty
  answer. `&S` and `Gathered` already forwarded `share`, as the review
  asked, from 7a on.
- **The contract of a tab's `read_bytes` is unchanged.** A read names
  every missing chunk of the first span it needs that is not held decoded,
  up to 8.5 MiB of the file for deflate and 16 MiB for zstd and xz, and no
  chunk of any span after it.
- **The wasm crate.** No exported function and no reply's JSON changed,
  apart from `too-large-input` as a new value of `refused`.
  `Editor::set_stream_limits`, in an `impl Editor` block outside
  `#[wasm_bindgen]`, sets the file's reading's limits for this crate's
  tests, through `Evaluator::set_stream_limits`; both are
  `#[doc(hidden)]`.
- **Tests.** In `codec/lazy/tests.rs`: a zlib and a zstd pass over a store
  are refused as `TooLargeInput` one byte past `Limits::input` and indexed
  at it, while `from_copy`, `Indexing::new` and `decode_or_index_with`
  are refused as `TooLarge` (now `TooLargeCopy`) past `copy_input` and not
  held to `input`, nor
  is xz; an index's `missing` names what a read of each span, and of each
  span's second half into the next, names, for deflate, zstd and xz, and
  nothing for a span held decoded; `RunInput::read` copies nothing from a
  source missing a chunk. In `source.rs`: a store's and a handle's
  `missing` against the read's list; a chunk found by `missing` is kept
  when a fed chunk pushes one out; every wrapper asks its source and reads
  nothing. In `eval/tests/resident.rs`: a zlib run past `copy_input` and
  three times longer than the store opens through its handle at exactly
  `Limits::input`, and one byte past it is refused as `too-large-input`
  with nothing read; from a store too small for two windows, and in
  memory, it is refused as `too-large` (now `too-large-copy`); a .gz of one
  member past
  `copy_input` opens as a joined stream read a part at a time, says
  `too-large-input` past `Limits::input` on the joined stream and the
  member's stream, and is refused as too large in memory, where its index
  holds a copy; and that joined stream's `missing`, and a lazy space's in
  `space.rs`, match what their reads then report. In `space.rs` and
  `eval/tests/run_input.rs`: weights, with the copy counted, and the index
  used longest ago let go first. In
  `crates/wasm/tests/open_space_waits.rs`, with small limits set through
  `Editor::set_stream_limits`: a .gz of one member and a zip entry of
  3 MiB of run, past `copy_input`, open end to end from a store of 1 MiB,
  their tabs reading the right bytes; one byte past `Limits::input`, both
  rows and Open unpacked say `too-large-input`, and so does Open joined
  stream.

### Built (part 10)

**Built on 2026-11-28.** A joined stream read a part at a time now reads
each stored part's bytes, and the run of each packed part that it decodes
whole, through a handle onto the source those bytes are in. It copies a part
only when no handle reaches the part. In the browser, a handle reaches every
part of the file that no edit touched, through the file's chunk store.
`whole_cap` (64 MiB) is the most a joined stream may come to and still be
held whole, and also the most a joined stream read a part at a time may
copy. So it no longer limits a BAM, a stream of stored pages, or a .gz of
many members each at most 1 MiB compressed, read from the store. Where the
code differs from "Joined streams":

- **`JoinedBytes`.** `Held(Arc<Vec<u8>>)` is now `Stored(RunInput)`, and
  `Packed { run, codec }` is now `Packed { input, codec }`. `Indexed`, a
  packed part too large to decode whole, keeps its `Seekable`. A part that
  no handle reaches gets a `RunInput::held` over the copy that `gather_run`
  makes, the same copy as before part 10. So `Joined::read` reads a part
  through a `RunInput` whether it has a handle or a copy.
- **Which parts get a handle.** `Evaluator::joined_input` decides, and
  `too_large_to_join` calls the same function, so the check made before the
  walk counts the same copies that the walk then makes. A stored part gets a
  handle whenever `shared_input` gives one. A packed part decoded whole gets
  a handle only when its run is short, because each decode reads the whole
  run in one read. Short is at most a gather window (1 MiB in the browser),
  and at most an eighth of what the source keeps (4 MiB of the browser's
  32 MiB store), whichever is less. A BGZF block, under 64 KiB, is always
  short. Over a source with no resident limit, such as an `ArcSource`
  natively, a run of any length gets a handle. The parts that are copied
  are a part in a stitched space, a part in an edited stretch, a part of a
  source that gives no handle (a `MemSource`), and a packed run that is not
  short. The design set no limit on the length of a packed run read through
  a handle. A run longer than the store keeps would never be in the store
  all at once, so each read of it would ask for the same chunks for ever.
- **The walk.** The walk does not read a stored part that has a handle. It
  still reads and decodes each packed part decoded whole, to check its
  length (see "Open questions"; since "Faster opening of joined streams" it
  no longer decodes them). After the decode it drops the copy of the
  part's run when a handle reaches the run, and keeps the copy otherwise.
  The walk now refuses as `too-large` (now `too-large-copy`) a packed run
  decoded whole that is
  longer than `CAP_BYTES` (64 MiB), as `unpack_run` refuses one. Before part
  10, `too_large_to_join` refused such a run before the walk started; now it
  counts no copy for a run with a handle onto a source with no resident
  limit, and lets the run through. `probe_parts` skips stored parts that
  have a handle. Each part's charge to the go is now worked out where the
  part is read (`copy_cost`, `unpack_cost`), because a stored part with a
  handle copies nothing and is charged one element. A walk that waits for
  bytes now records how far it has got, as it does when a go runs out, so
  `Evaluator::unpacking` answers after a "pending" reply from a walk, as it
  does after one from an index pass (7b). Before, a walk recorded it only
  when the part it waited on was read a window at a time.
- **A read names every chunk it is missing at once.** When one part of a
  `Joined::read` is missing chunks, the read only asks each later part which
  chunks it would miss (`Source::missing` through the part's handle), and
  decodes nothing more. So one "pending" reply names every chunk those parts
  need. A packed part that is in the cache needs no chunks. A later part
  read through its index is not asked, since one span of an index can need
  16 MiB of the file: a read names the chunks of at most one indexed part's
  span, as 7c's read of a stream read through an index does. Two such
  spans could need 32 MiB at once, which the browser's store never holds.
  `Joined::missing` names the chunks of every stored and packed part the
  read covers, and those of the first indexed part when no part before it
  is missing any. Before, it named only the chunks of the first indexed part
  that was missing some. `map_out` and `map_in` on a packed part read the
  part's run through its handle to make the part's trace, so they too can
  answer with missing chunks.
- **The limit on one read.** `Joined::resident_limit` was the smallest span
  cache of the stream's indexed parts, and no limit for a stream with no
  indexed part. It now also counts the parts read through a handle onto a
  source with a resident limit (`handle_limit` in `space.rs`). A read of `n`
  bytes of the stream needs at most `n × expand + 2 × longest` bytes of the
  file at once. `longest` is the longest packed run read through a handle.
  `expand` is the most bytes of run that such a packed part takes for each
  byte it unpacks to, and is at least 1, since a stored part takes one byte
  of the file for each byte. The limit is the `n` for which that comes to
  half of what the source keeps. For a BAM in the browser, `expand` is 1
  and the limit is 16 MiB less two blocks; `Doc.readLoaded` reads 4 MiB at
  a time. A tab's own reading takes its gather window from this limit, and
  so does the check in `read_in` that refuses a read longer than the limit,
  as over a stream read through an index. Without the limit, a tab's
  reading over a stream of stored pages read through handles would read a
  long run in one call, and never refuse a read that cannot fit, as "Review
  before part 7" says of `ChunkStore::behind` before 7a.
- **The bytes read to recognise a stream.** `template_for` reads a stream's
  first 36 KiB in one read, to see what format they are. It now reads no
  more than the stream's resident limit, which only a test's small store
  makes less than 36 KiB. Before, opening a stream of stored pages over a
  store of 4 KiB asked for the same chunks for ever, since the walk had read
  none of the pages and 36 KiB never fits in that store.
- **"A joined stream walked twice" is fixed**, as "Still to do" suggested.
  When `template_for` answers `Pending`, `Evaluator::open_space` keeps the
  stream that `unpack` made in a new field of the `Evaluator`, `unopened`,
  with the space and path that were asked for. The next call for the same
  stream opens the kept stream instead of joining it again. An edit or a
  new template drops it. With part 10 the fix is needed for the stream to
  open at all, and not only to save time. Take a .gz of many members whose
  runs come to more than the store keeps. The walk reads every member's run,
  and by its end has pushed the stream's first chunks out of the store.
  `template_for` then waits for those chunks, and a second walk, from the
  first member, pushed them out again, so the stream asked for the same
  chunks for ever. A stream of stored pages was only joined twice, since its
  walk reads none of the pages. The stream is kept by the `Evaluator` that
  `open_space` is called on, and not put back in `Spaces::joins` as a
  finished join. That way it does not take one of the two places in
  `Spaces::joins` that walks wait for, and a stream that a recognised tab's
  own reading unpacked is kept the same way as one the file's reading
  unpacked. One stream is kept at a time: when a call for another stream
  also waits, its stream replaces the kept one.
- **The cache of decoded parts** of a joined stream takes its cap from
  `Spaces::cache_cap`, which is `STITCH_CACHE_BYTES` (16 MiB) unless a test
  lowers it. A stitched space's cache takes its cap from there too.
- **What a joined stream holds.** Before part 10, a joined stream read a
  part at a time held a copy of every stored part's bytes and of the run of
  every packed part that it decodes whole, up to `whole_cap` (64 MiB) in
  total, besides its cache of decoded parts (16 MiB) and one part's trace.
  Over handles it now holds a `RunInput` for each part, about 60 bytes with
  its handle, the cache and the trace. While a read decodes a packed part,
  it also holds that part's run, and the part's output until the cache
  takes it.
- **What is still held to 64 MiB.** The copies of parts that no handle
  reaches are still held to `whole_cap` in total: every part of a
  `MemSource` natively, a part in a stitched space, a part in an edited
  stretch, and a packed run decoded whole that is not short. So a .gz of
  members over 1 MiB compressed and at most 64 MiB unpacked each, which
  are decoded whole, is still copied, and refused past 64 MiB of runs. A
  joined stream held whole, at most `whole_cap` of output, is unchanged.
- **Strings and JSON.** No string, refusal tag or reply shape changed. A
  "pending" reply to Open unpacked on a joined stream can now come with
  progress from `unpacking`, as one on a stream read through an index can
  since 7b. So the status bar shows `UNPACKED.unpacking` ("Unpacking
  {field}… {percent} through the compressed data, …") while a join waits
  for chunks, where it showed `UNPACKED.loading` before; it already showed
  `UNPACKED.unpacking` between those waits.
- **Tests.** In `eval/tests/resident.rs`, with the cap on a joined stream
  held whole lowered through `Evaluator::set_stream_limits`, and stores of
  chunks of 1 KiB:
  - A BGZF file of 100 blocks, each of about 2 KiB of output, opens as a
    joined stream read a part at a time. The blocks' runs come to about
    60 KiB, more than three times a store of 16 chunks and more than the cap
    of 16 KiB. The file is fed only the chunks each `Pending` names, once in
    a single call with no limit on work and once in goes of 100 elements.
    Each time, the stream is joined once, no answer names more than 6
    chunks, and the stream holds no copy. Reads of 2,000 bytes at four
    places, and of the whole stream 6 KiB at a time, give the blocks' bytes
    and name the chunks that `Joined::missing` names; a read of 2,000 bytes
    names at most 3 chunks, and one of 6 KiB at most 8. The cache holds at
    most its lowered cap of 8 KiB. Four bytes map to the same decoder steps
    as in the stream held whole. When one block's trailer claims one byte
    more than the block unpacks to, the stream was refused as failed; since
    "Faster opening of joined streams" it opens and reads of that block fail,
    and that case moved to another test. In
    memory, the copies come to more than the cap, and the stream is refused
    as too large.
  - 48 stored pages of 1 KiB, past a cap of 8 KiB, over a store of 16
    chunks, open after one `Pending`, which names the 8 chunks of the first
    8 KiB, and the stream is joined once. It holds no copy, its limit on one
    read is 8 KiB, and reads at three places and of the whole stream in
    8 KiB pieces give the pages. After an overwrite inside page 3, the
    stream opens again with a copy of that one page, and reads the edited
    bytes. In memory, the stream is refused as too large.
  - A reading over that stream of pages, as a tab's own reading is, refuses
    a read of 9 KiB in one call with "Can't load 9 KiB of the file at once:
    the editor keeps at most 8 KiB in memory", and a read of 6 KiB waits
    for its chunks and reads.
  - The same kind of BGZF blocks read with the gzip template, 40 members of
    64 KiB of output whose runs come to more than a store of 512 chunks,
    open once their first 36 KiB are loaded, and are joined once. The
    stream holds no copy and reads back whole.
  - With `open_space` changed back to keep nothing, the gzip test asks for
    chunk 0 a fourth time, which is the wait that never ends, and the
    pages' test and
    `a_gzip_past_the_cap_opens_as_a_joined_stream_read_from_the_store` join
    their streams twice. All three fail.

  In `crates/wasm/tests/open_space_waits.rs`,
  `a_joined_stream_past_the_cap_reads_its_blocks_through_the_store` opens a
  BGZF file of 140 blocks, 3.5 MB of runs, over a store of 16 chunks of
  64 KiB, with the cap lowered to 256 KiB through
  `Editor::set_stream_limits`. Its tab reads the right bytes at four places
  and for 300 KiB across several blocks. No reply names more than 6 chunks,
  no chunk is asked for more than 3 times, and once a "pending" reply comes
  with the walk's progress, every later one does.
  `a_joined_stream_past_64_mib_compressed_opens_end_to_end`, marked
  `#[ignore]`, opens a BGZF file of 66 MiB of noise under the default
  limits, from a store of 4 MiB, and its tab reads the right bytes. Each
  block's deflate is one stored block (deflate's block type that holds its
  bytes as they are), so the file is past 64 MiB compressed.
- **A test changed on purpose.**
  `a_joined_stream_whose_parts_add_up_to_more_than_the_store_opens`
  asserted that six pages past a cap of 5 KiB were refused as too large
  over a store of 4 chunks of 1 KiB. The pages now open through handles,
  the whole stream reads back, and the test asserts the refusal in memory.
- **Speed and memory.** Measured on 2026-11-28 with
  `examples/joined_timing.rs`, new in this part, and
  `examples/listing_timing.rs`, in release builds with LTO off
  (`CARGO_PROFILE_RELEASE_LTO=false` and 16 codegen units), pinned to one
  core, before and after by turns, three runs of each. Each file is read
  through a chunk store with room for the whole file, so that nothing is
  pushed out, and from a `MemSource`. `joined_timing` opens a file's joined
  stream as a document, and then reads the stream end to end in reads of
  64 KiB; its heap row is the heap allocated by opening the stream, which
  was the same in every try. `listing_timing` lists a file's tree depth
  first: all 13,029 rows of `mpileup.1.bam`, and 3,000 rows of
  `big100.bam` (3,024 with the last node's children) after opening its
  joined stream. The real BAMs in the sample collection are under 72 KB, so
  their joined streams are held whole and never read a part at a time.
  `big100.bam` is made from `mpileup.1.bam` by repeating its records to
  100 MiB and writing them in BGZF blocks of 65,280 bytes, as samtools does;
  it packs to 32 MB in 1,610 blocks. The load average was 11 to 15 on 12
  cores, and runs of one build varied by up to 30%. The table gives the
  fastest run.

  | File and what was timed | Rows | Chunk store, before | Chunk store, after | `MemSource`, before | `MemSource`, after |
  |---|---|---|---|---|---|
  | `big100.bam`: opening the joined stream | | 972 ms | 965 ms | 974 ms | 982 ms |
  | `big100.bam`: reading the stream end to end | | 961 ms | 960 ms | 957 ms | 954 ms |
  | `big100.bam`: heap held after opening | | 46,998,099 B | 15,114,808 B | 46,997,459 B | 47,087,635 B |
  | `mpileup.1.bam`: the listing | 13,029 | 50.4 ms | 52.0 ms | 50.7 ms | 50.8 ms |
  | `big100.bam`: opening the stream and listing | 3,024 | 1,003 ms | 1,014 ms | 1,007 ms | 1,030 ms |

  Through the chunk store, the 31.9 MB of runs that the stream copied are
  gone. A `MemSource` gives no handle, so the runs are copied before and
  after, and the `RunInput`s of the 1,610 parts add about 90 KB. The times
  are the same within the noise. A BGZF file's template declares what its
  joined stream holds, so the listing reads the stream's records through
  the file's own reading, from its stitched space, which this part did not
  change. A tab's bytes, as the hex view shows them, are read from the
  joined stream.

  `examples/big_bam.rs`, new in this part, makes such a BAM of any size.
  One of 300 MiB of BAM, 96 MB of file and 4,820 blocks, past 64 MiB
  compressed, is refused as too large through the chunk store before this
  part, since its runs come to more than `whole_cap`. After it, it opens in
  3.3 s and reads end to end in 3.1 s, holding 57.5 MB after opening, none
  of it copies of runs. Where the rest goes was not measured. From a
  `MemSource` it is
  refused as too large before and after.

### Faster opening of joined streams

**Built on 2026-11-28, after part 10.** A joined stream read a part at a
time now opens without unpacking any of its parts. Each packed part is
checked when a read first unpacks it, and a part that fails the check makes
every read of its bytes fail, with the reason shown to the reader.

**Where the time went.** Measured with `examples/joined_timing.rs`, which now
times the walk to the stream's parts apart from the rest of the open, on a
BAM made with `examples/big_bam.rs` from `mpileup.1.bam`: 314,588,858 bytes
of BAM in 95,790,072 bytes of file and 4,821 blocks. The build was release
with LTO off and 16 codegen units, pinned to one core, and the file was read
through a chunk store with room for all of it. The load average was 17 to 25
on 12 cores. Before this change the open took 3.4 to 4.7 s:

- The walk to the parts (`open_stitched_at`, which reads every block's
  header and trailer): about 0.3 s, 8 to 10% of the open.
- The rest, 3.3 to 3.6 s, was `join_lazily`'s length check: each block's run
  read and decoded with `codec::decode`, which runs the traced inflate and
  throws the trace away. Decoding every block's run on its own, outside the
  evaluator, took 3.8 s. Copying the runs took 12 ms. `template_for` reads
  nothing for a BGZF stream, whose template declares what the stream holds.
  So the check was about 90% of the open.
- For comparison, `inflate_len`, which counts the output and keeps none,
  took 2.8 s over the same runs, and `miniz_oxide` took 0.8 s.

**The rule now.** A packed part small enough to decode whole is not decoded
before the stream opens. Its length is the one the file gives for it, which
`reach_part` already used to lay the stream out: for a BGZF block or a gzip
member, `original_size`, the trailer's ISIZE. The first read that unpacks the
part checks it, in `Joined::unpacked`. A part that will not unpack, or
unpacks to another length than the file says, is kept in `Joined::troubles`
with the reason, as `Stitch::troubles` keeps one for the listing, and is not
decoded again. Its bytes read as zeroes and every read that covers them is
refused. Nothing after it moves, since the stream was laid out from the
lengths the file gives: a part of the wrong length never shifts the bytes
after it, and its own bytes are never shown as bytes.

Two kinds of part are still decoded before the stream opens
(`checked_when_joined` in `stitch.rs`), and refuse the stream as failed when
they unpack to another length, as every bad part did before:

- One the file says unpacks to nothing. It gives the stream no bytes, so no
  read lands on it, and a block whose trailer says 0 while it holds data
  would otherwise lose that data without a word. A BGZF file's end block is
  such a part, 28 bytes, and normally the only one.
- One the file says unpacks to more than it could. No part decoded whole
  unpacks to more than 64 MiB (`CAP_BYTES`), whatever its codec, and a
  deflate, zlib or gzip run to more than 1,032 bytes for each of its bytes,
  a copy of 258 bytes in two bits. A claim past either is a lie the walk
  catches for one decode.

**How large a trailer's claim can make a stream.** A part of a codec with an
index (deflate, zlib, gzip, zstd, xz) claiming more than 64 MiB
(`Limits::whole`) is read through an index pass in the walk, as before this
change, which finds its real length; so is one claiming the most a 32-bit
ISIZE can say, past the 1 GiB (`LAZY_CAP_BYTES`) a part read through an
index may be, and the stream is refused for it. A part of any other codec
claiming more than 64 MiB, and a deflate part claiming more than 1,032
times its run, is decoded by the walk and refused. So each part decoded
whole adds at most 64 MiB to the stream, and a deflate part at most what its
run can fill. A zstd or xz part claiming up to 64 MiB is not checked until
it is read, so a stream of such parts can be laid out as long as 64 MiB a
part. No limit on a joined stream's total length was added or removed. No
buffer is sized by a claim: a part is decoded into what it unpacks to, at
most 64 MiB, and compared with the claim after. The tests are
`a_bgzf_block_claiming_a_huge_size_refuses_the_stream_or_its_reads` and
`a_part_of_a_codec_with_no_index_claiming_more_than_a_part_can_be_refuses_the_stream`,
which failed before the 64 MiB check: the stream opened 100 MiB long.

How the reader is told, by each path that reads a tab's bytes:

- `Joined::read` reads past a part in trouble. It fills that part's bytes
  with zeroes, reads the parts after it, and fails as refused. Missing chunks
  still come first. `SpaceSource::read` keeps the other parts' bytes rather
  than zeroing the whole read, as it still does for a stream read through an
  index.
- `Source` has two new methods with defaults, `may_be_unreadable` and
  `unreadable`, which `SpaceSource`, `ChunkStore` (for the source behind it),
  `Document`, `&S` and `Arc<S>` pass on. `unreadable(offset, len)` names the
  first part in trouble among the bytes asked about: where its bytes are, and
  why. It decodes nothing, since the read before it found the part out.
  `Document::unreadable_bits` maps it through the piece table.
- `read_in` on a document whose source may be unreadable (`#[cold]`
  `read_or_unreadable`) fails with the reason instead of handing back the
  zeroes. That is the reading of a tab whose stream was recognised, such as
  a .gz of many members holding a gzip or a tar, over its own bytes. Every
  other document says `false` and pays one call per read.
- `template_for` looks only at the bytes before the first part in trouble
  when it recognises a stream from its first 36 KiB. A stream whose first
  part fails opens as the bytes it declares.
- `Evaluator::unreadable(space, at, len)` says the same with the part named
  as the listing names it, through the stitched space at the stream's path.
  The words are the ones a read of the part in the listing has, built by the
  new `trouble_said`, which `part_trouble` now uses too:
  `blocks[50].compressed unpacks to 2098 bytes, not the 2099 the file says`
  and `blocks[50].compressed could not be unpacked`. Where the stitched
  space cannot say, the part is named `part index 50`, the fallback
  `part_label` already had.
- `Joined::map_out` and `map_in` give no step for a byte of a part in
  trouble, and the trace they make of a part checks its length too.
- `Joined` counts the reads that reached a part in trouble and keeps the
  latest such part. `Source::unreadable_reads` and `last_unreadable` give
  them, passed on by the same wrappers, and `Evaluator::last_unreadable`
  names the part. Code that reads a tab through other code, a scan or a
  format's own reader, takes the count before and compares it after.
- The wasm has `Editor::unreadable(space, at, len)`, which answers
  `{status:"ok",node:{at,len,why}}` or a null node. `read_bytes` and
  `read_bits` are unchanged; the web asks `unreadable` after them.
- Every other wasm call that reads a tab's bytes outside the template's own
  reading marks them or refuses with the reason (`refusing_unreadable`,
  which compares the count). These refuse: `overview_step` (and every later
  step, kept in `scan_refused`, rather than scanning again from the front to
  the same part), `overview_focus_step`, `holds_hdf5`, `contents`, `btree`,
  `elf_contents`, `root_contents`, `iso_volume`, `iso_directory`,
  `hardening`, `jpeg_scan` and `jpeg_block`. `spans` drops the instruction
  names it read from such bytes and gives the rows as the template reads
  them. `text_window` gives each line the unreadable stretches it holds,
  with the reason. `strings_scan` drops every string that touches them,
  length prefix and terminator included, and names them. `selection_text`
  gives the reason instead of readings, and `selection_literal` nothing.
  `search_step` steps past a match on them. `text_index` and `text_back` are
  unchanged: they scan the same zeroes the text window does, so line starts
  agree, and none moves, since the part is as long as its trailer says.
  `head`, which the text readings sniff the encoding from, reads the file
  and never a tab. Calls that read through the template's own reading
  (`template_*`, `spans`, `run_cells`, the walks behind the report and the
  census) fail field by field through `read_in`, as a listing does.
- The web's `Doc.read` and `readBits` ask `unreadable` after a read of a
  joined tab that found no chunk missing, and give `unreadable` in the
  `ReadResult`, with `complete` false. `readLoaded` stops at the first piece
  with unreadable bytes and names them. Every caller that would read a value
  from the bytes refuses, and every one that draws them marks them:
  - the hex view draws them as `??`, muted, with the reason on hover, and a
    copy that reaches them says `Couldn't copy: the selection includes
    unreadable bytes ({reason}).`;
  - the text view draws each character over them as `?`, muted, with the
    reason on hover and an `unpacking failed` mark at the line's end, and a
    copy says the same as the hex view's;
  - the inspector writes `unreadable bytes` in a value's box, with the
    reason on hover, and `Selection includes unreadable bytes: {reason}` in
    place of the
    selection's text readings and its number;
  - the strings view adds `{size} unreadable, not scanned: {reason}` to its
    status line;
  - the report's byte dump and the listing's byte strips write the bytes as
    `??` (`hexMarked`), and so do the report's first bytes of a stretch, its
    zoomable byte map and its hover dump (`byteHex`), which no longer wait
    for those bytes as if they were loading;
  - the document's own sniff of what template fits looks only at the bytes
    before them, as the core's `template_for` does;
  - the minimap says the reason where its progress would be, and keeps the
    classes it found before the part; the report's ledger keeps those and
    counts the rest as unread;
  - an image card says `Image includes unreadable bytes: {reason}`, opening
    a run as a file fails with `Couldn't open these bytes: some are
    unreadable ({reason}).`, and the
    report's plain-text sniff, the JPEG cards and the outline's metadata
    read nothing from them.

  The other callers of `read`, `readBits` and `readLoaded` (the listing's
  value cells, the diagram pieces in `viz/`, the gap check, the stride
  raster, the identify and signature sniffs, the dump scan) already draw or
  decide nothing from a read that is not `complete`, which a read with
  unreadable bytes is not. Saving reads the file, and hashing reads the
  file's own blob, so neither ever reads a tab. The arithmetic is in
  `web/src/unreadable.ts`.
- The core's own readers that read a document directly, outside `read_in`,
  were traced: the overview scan, the strings scan, the text window and
  index, search, the HDF5, ROOT, ELF hardening, WebAssembly and NE readers
  are reached only through the calls above. `sqlite_overflow::read`, which a
  SQLite row spread over overflow pages is gathered by, now gives the row's
  problem as `Row not assembled: one of its pages is unreadable
  ({reason}).` instead of a row made of zeroes;
  `Gathered` passes the new `Source` methods on, mapped through its extents.
  `uf2::image` has no caller outside its tests. `textnum`'s `Window` and the
  hex dump reader's `DumpSource` hold bytes of their own and wrap no source.
  `save.rs` reads the file.

What did not change:

- A joined stream held whole, up to `whole_cap` (64 MiB of output), still
  decodes every part with its trace before it opens, and a part that will
  not unpack, or unpacks to another length, refuses it, as before. So there
  are two rules by size: held whole, a bad part refuses the stream when it
  opens; read a part at a time, it fails the reads that reach it.
- A packed part too large to decode whole is still read through an index
  whose pass runs in the walk and checks the part's length.
- A packed part that no handle reaches is still copied by the walk, and the
  copy is not decoded. A part read through a handle is charged one element,
  as a stored part is, and `probe_parts` skips it, so the walk asks for no
  chunk of it, unless it is a part that unpacks to nothing, which the walk
  reads. In the browser the open waits only for the chunks that hold the
  blocks' headers and trailers, and the end block's run, which is in the
  last chunk those reads loaded; no "pending" reply said how far unpacking
  had got in the test that checks this.

**Why this, and not the alternatives.**

- Check after the tab opens, in goes in the background. A reader can read a
  part before the check gets to it, so every read would still need the check
  and the failure this change adds. On top of that it would cost the whole
  inflate again, about a minute in the browser for this file, competing with
  the reader's own reads, to find parts that no read has reached.
- A sampled check, of the first, the last and some parts between. It
  guarantees nothing for the parts it skips, so the read would still need
  the check and the failure.
- Check only the parts under a read. This is the one built: a read unpacks
  the part anyway, so the check costs one length comparison per part, and
  nothing when the stream opens but the decode of the parts that unpack to
  nothing.
- A faster inflater for the check, such as `miniz_oxide`, at about a fifth of
  the time. The bytes a reader sees would then come from one decoder and the
  check from another, which `codec::decode` exists to avoid, and the open
  would still be one inflate of the whole stream.

**CRC-32 is not checked.** A part that inflates to its stated length with
the wrong bytes reads as those bytes, as it did before this change: the old
check compared lengths only. The file's own listing has the CRC-32 check on
each block's `crc32` field. Checking it here would need `Ty::Stitched` to
name each part's checksum field, beside `part_len`, and would add a CRC-32 of
each part to its first read: the table-driven `crc32` reads about 0.5 GB/s
here, about a fifth more on top of the inflate. See "Still to do".

**Time before and after.** `joined_timing` on the same BAM, before and
after by turns, three tries a run, three runs each, with the load average
at 21 to 24. The fastest try of each run; the binary from before this change
does not split the open.

| Run | Open, before | Open, after | Walk to the parts, after | Rest of the open, after | Read end to end, before | Read end to end, after |
|---|---|---|---|---|---|---|
| 1 | 4,707 ms | 215 ms | 213 ms | 1.8 ms | 4,465 ms | 3,509 ms |
| 2 | 3,770 ms | 251 ms | 250 ms | 1.3 ms | 3,644 ms | 3,782 ms |
| 3 | 3,586 ms | 205 ms | 204 ms | 1.2 ms | 3,465 ms | 3,528 ms |

An earlier set of three runs, with the load at 17 to 25, gave 3,401 to
3,854 ms before and 221 to 391 ms after. Timed apart by a throwaway example,
before the change the walk took 0.35 to 0.7 s and the rest of `open_space`
4.7 to 4.8 s. Reading the whole stream after opening is the same work before
and after, since each block is decoded by the first read that reaches it in
both. The heap the opened stream holds is the same, 57.5 MB.

**In the browser.** The same BAM with block 2's trailer one byte long and
block 3000's deflate damaged, served by the dev server from the worktree's
own wasm build: from loading the page, the file was open at 1.3 s, Open
joined stream was offered at 4.7 s, once the walk to the parts had loaded
every chunk of the file, and the tab opened 0.19 s after the press. Pressed
again on a page where the walk had already run, the tab opened in 0.75 s.
Part 10 reported 61 s for the press before this change; that was not
measured again here.

**What a reader sees for a damaged part.** In a BAM's tab, the listing's run
of records stops where the bad block begins, with
`blocks[2].compressed unpacks to 65280 bytes, not the 65281 the file says`,
as it did in the file's own tab before. The hex view shows the block's bytes
as `??`, or `????????` in binary, with the same words on hover, and the
bytes either side as they are: in the browser, the last `??` is at
`0x2fd00` and the next block's bytes start at `0x2fd01`, where its trailer
says. A copy of a selection that reaches the block says `Couldn't copy: the
selection includes unreadable bytes (blocks[3000].compressed could not be
unpacked).` A search steps past it. Before, the tab would not open, and Open unpacked said it
was refused as failed.

**What still reads the zeroes.** A part read through its index that later
refuses a span: the read is refused and counted, but no trouble is kept, so
`unreadable` names nothing and the hex view shows zeroes there, as it did
for a stream read through an index before this change. The calls guarded by
`refusing_unreadable` refuse it all the same, since they go by the count.
`filetype.ts`'s search for PyBasic's module name reads the front of any
document and finds nothing in zeroes. The hex view's byte under the cursor,
read to type over, is read on a tab that cannot be typed in.

**Tests.**

- `eval/tests/resident.rs`:
  `opening_a_joined_stream_of_many_blocks_unpacks_only_the_end_block` counts
  decodes, through a counter in `codec::decode_traced` that every deflate
  decode passes, while a stream of 100 blocks opens from the store and from
  copies: one, the end block's, where the old walk made 101. A read of one
  block then makes one, and a read of it again none.
  `a_bgzf_block_whose_trailer_says_it_unpacks_to_nothing_refuses_the_stream`
  sets block 50's trailer to 0: the stream is refused as failed, and the
  same file unspoiled opens.
  `a_bgzf_block_that_lies_about_its_length_or_is_damaged_fails_where_it_is_read`
  spoils block 50 of 100 with a trailer one byte long and, separately, with a
  deflate block type that does not exist. The stream opens; a read across
  blocks 49 to 51 is refused, gives the bytes of 49 and 51 and zeroes for 50,
  and decodes block 50 once in three reads; `unreadable` names block 50's
  bytes in the listing's words, and `part index 50` without the stitched
  space; reads elsewhere are not touched; a byte of block 50 maps to no step.
  `a_tab_reading_its_own_fields_fails_where_a_member_will_not_unpack` reads a
  recognised gzip inside a .gz of members, through the tab's own reading,
  and a read across the bad member fails with the reason; with the first
  member bad, the stream opens unrecognised. The spoiled-trailer case of
  `a_bgzf_stream_past_the_store_and_the_cap_opens_through_handles`, which
  expected the stream to be refused, moved into the second test.
- `eval/stitch.rs`:
  `a_joined_stream_past_the_cap_with_a_part_that_will_not_unpack_refuses_reads_of_that_part`
  does the same over handles with no limit, for a BAM of three blocks.
  `a_joined_stream_with_a_part_that_will_not_unpack_is_refused_as_a_document`
  still holds for a stream held whole.
- `crates/wasm/tests/open_space_waits.rs`:
  `a_joined_stream_past_the_cap_reads_its_blocks_through_the_store` now
  expects no "pending" reply to say how far unpacking has got, where it
  expected some.
  `a_joined_stream_with_a_block_that_lies_about_its_length_says_which_bytes_will_not_read`
  opens 140 blocks with block 70 one byte long, reads around it through the
  store, checks what `unreadable` says, and searches for zeroes forward and
  backward and finds none.
- `web/test/hexcell.test.ts` draws an unreadable byte as `??` in hex, and
  `web/test/chunkwait.test.ts` has `readInPieces` stop at a piece with
  unreadable bytes and name them.
- `crates/wasm/tests/unreadable_parts.rs`, on a BGZF file of three blocks of
  text whose middle block's trailer is one byte long: the hex view's reads
  and `unreadable`; the text view's marks, every byte of the block marked
  once, and `text_index` and `text_back` agreeing with the lines through
  the block; the strings scan finding no string that touches the block and
  naming it; the selection's text and literal; the overview refused with
  the reason, at once on every later step, and the focus refused over the
  block and read over the first; and the format readers giving nothing for
  a stream that is none of their formats. An ISO 9660 image in gzip members
  of two sectors, whose tenth member is one byte long, with `iso_volume` and
  `iso_directory` refused where they reach it and a directory in the whole
  members read.
- `eval/tests/resident.rs`:
  `a_bgzf_block_claiming_a_huge_size_refuses_the_stream_or_its_reads`, and
  the spoiled-block test now checks the count of refused reads and
  `Evaluator::last_unreadable`.
- `web/test/unreadable.test.ts`: `reaches`, `stretchAt`, `masked`,
  `unreadableError`, `hexMarked`, `byteHex`, `settled`, and the new words.

### Built (part 8)

**Built on 2026-11-28.** An xz stream read through a handle is now laid out
by a pass in steps, `XzPass` in `codec/lazy/xz.rs`, which reads the stream
through its `RunInput` a piece at a time and holds no copy of it. It is
stepped, charged and kept across `Pending` in `Unfinished` as the deflate
and zstd passes are, and `Evaluator::unpacking` says how far it has got.
So `lazy::can_step` now says yes to every codec `can_index` does, an xz
stream read through a handle takes `Limits::input` (512 MiB packed) and is
refused past it as `too-large-input`, and an xz stream read from a copy
keeps `Limits::copy_input` (64 MiB) as before. Each block's limits
(`Limits::span`, `unit_input`, `span_input`) and their refusals are
unchanged. Where the code differs from "xz" under "The index pass reads
through a `RunInput`":

- **One layout, in pieces.** `codec::xz::layout` is split into the pieces a
  stepped pass needs: `xz::ends` reads the stream's header and footer,
  `xz::Records` reads the index's records one at a time, and
  `xz::block_at` reads one block's header. `layout` puts them together over
  a whole stream, and the stepped pass over the bytes it reads, so the two
  come to the same `Layout`. The checks once every block is placed
  (`laid_out`: a filter, a block past `Limits::span`, a stream past
  `Limits::lazy`, a block too long to read at once) are the same function
  and in the same order for both.
- **The steps.** The first step reads the stream's first and last 12
  bytes, the header and the footer. The next reads the index, whose length
  the footer gives, `Limits::step_input` bytes a step and none of the
  stream before it; in the browser the index is nearly always in the
  footer's chunk of the file, which the first step loaded. Then come the
  block headers. The design did not say how much of a header to read: each
  is read in two reads, its first byte, which gives its length, and then
  the header, so that no read names a chunk of the file that holds only a
  block's data. A step reads at most 64 headers (`HEADERS_PER_STEP`), and,
  which the design did not have, the last of them ends no more than
  `RunInput::most_at_once` bytes (half of what the source keeps) past the
  start of the first, so that their chunks fit in the store together: 16 MiB
  of the stream in the browser. A header that has been read is kept while
  its step waits for the others, so a store that cannot hold all of a
  step's headers, or a tab's source that decodes as it reads, still gets
  further each round.
- **Which streams are decoded whole.** As over a copy, a stream whose index
  says it unpacks to the cap or less, or whose header, footer, index or a
  block header does not hold together, goes to the decoder of whole
  streams. The pass decides from the index alone whether the stream fits
  under the cap, and reads no block header then. The design did not say
  how such a stream is read: the pass copies it, a piece of up to four
  times `step_input` (1 MiB) a step and at most a quarter of what the
  source keeps (`COPY_STEPS`), and decodes it once the copy is complete.
  The copy is held to `Limits::copy_input`. A longer stream is refused for
  what sent it to the decoder of whole streams (`Whole` in
  `codec/lazy/xz.rs`): as `too-large-copy` when its size is the reason,
  which is a stream that fits under the cap, one with stream padding after
  its footer, or one joined end to end from several streams; and as
  `failed`, "unpacking failed", when its header, footer, index or a block
  header does not hold together, which is what the decoder of whole
  streams says of a stream it cannot read. Before this, a damaged stream
  past 64 MiB packed was refused as `too-large-copy`, whose hover says an
  xz stream with no edited bytes may be 512 MiB packed. A stream whose
  blocks end before its index is checked for a stream header where the
  last of several joined streams would start, one read of 12 bytes, to
  tell the joined stream from a damaged index. Up to 64 MiB packed,
  either kind is decoded whole as before. The refusal does not say which
  part is damaged: `Refusal::Failed` carries no detail, and the decoder of
  whole streams gives none either.
- **The number of blocks.** The design sends an index longer than
  `Limits::unit_input` (16 MiB) to the decoder of whole streams. Instead,
  the pass refuses a stream whose index lists more than
  `Limits::block_marks` blocks, 65,536, as `TooManyXzBlocks`
  (`too-many-xz-blocks`), "too many xz blocks to unpack (over 65,536
  blocks)", once it has read the index's count and before it reads any
  block header, and whatever the stream unpacks to. The index keeps an
  entry for every block, 32 bytes natively and 24 in wasm, so the list is
  at most 2 MiB; without the limit a hostile index of 16 MiB could list
  8 Mi blocks, 256 MiB. Once the count is read, the pass reads only as much
  of the index as that many records can take, 18 bytes a record, so it
  holds at most 1.2 MB of index. A copy in memory is held to the same
  limit, from the same count (`count_of`), so the two still agree; before
  this, a copy of any number of blocks was laid out or decoded whole.
  `xz -T0 -6` writes a block for every 24 MiB of output, and `xz -T0
  --block-size=16KiB` on 1 GiB writes 65,536.
- **A copy in memory, and a short stream.** A pass over a copy of the
  stream, and one over a stream no longer than `step_input`, lays the
  stream out in one step from the whole stream with the code it had before
  (`index_xz`), so what `decode_or_index_with` and `Indexing::new` give for
  xz is unchanged. A short stream is read in one read.
- **A refusal said later.** A stream past 64 MiB packed was refused as
  `too-large-copy` before any of it was read. Read through a handle, a
  stream that lays out and has a BCJ or delta filter in a block past the
  cap is now refused as `too-large-output` once its block headers are read,
  as it is under 64 MiB packed; one that is to be decoded whole is refused
  as `too-large-copy` once its index is read, or as `failed` once the pass
  finds it damaged.
- **Progress.** `Progress::read_bits` is where the furthest block header
  read starts, those of a step that waits for the rest of its headers
  included, or how much of the stream is copied, and never goes back. So
  each round that reads more headers is progress to `waitToOpen`, which
  takes a chunk asked for again with no progress as a wait that will not
  end. A stream whose headers were read and that is then copied whole
  keeps the furthest bit reached until the copy passes it. Nothing is unpacked, so
  `unpacked` is 0 until the pass ends. An xz step decodes nothing, so it
  takes no share of output or input from `Indexing::step`'s arguments, and
  `known_large` changes nothing for xz.
- **Which passes read in windows.** `Limits::step_window` has no margin for
  xz, so `reads_in_windows` says yes for xz from a source that keeps at
  least twice `step_input`, 512 KiB under the default limits.
- **In the evaluator** the branch that called `decode_or_index_through` for
  xz, with nothing charged and no place taken, is gone. An xz pass takes a
  place in `Unfinished` and is charged `INDEX_STEP_ELEMENTS` a step, as the
  other passes are. A stream no handle reaches is gathered into its place
  first (`Held::Gathering`), as other runs are.
- **What a pass holds.** Before part 8 the pass read a copy of the whole
  stream, gathered first. Now it holds the stream's index while it reads
  the block headers, the blocks placed so far and up to 64 being read, and,
  for a stream decoded whole, its copy. See "Speed and memory" below.
- **Speed and memory.** Measured on 2026-11-28 with `examples/pass_memory.rs`,
  which now takes the codec to read and, with `copy`, makes the pass read a
  copy of the run made first, as the evaluator's gather made one before
  this part: for xz that is the copy and one call of
  `decode_or_index_through`, which is what `index_pass` did. Release builds
  with LTO off (`CARGO_PROFILE_RELEASE_LTO=false` and 16 codegen units),
  pinned to one core, the before build from 84589d49 with the new example.
  Five tries before and seven after; the table gives the fastest try. The peak was the
  same in every try. The load average was 18 to 38 on 12 cores. "Peak heap"
  is as in "Built (part 7b)": the most heap allocated during the pass over
  what was allocated before it, the file not counted. The before times
  include copying the file once, as the gather did, and not reading it
  through a chunk store. The files were made with `xz` 5.4.5: `seq 1
  30000000` through `xz -T1 -6` and `xz -T4 -6`, and base64 lines of random
  bytes through `xz -T1 -6` and `xz -T4 --block-size=8MiB -6`.

  | File | Packed | Unpacked | Blocks | Came to | Steps after | Time, before | Time, after | Peak heap, before | Peak heap, after |
  |---|---|---|---|---|---|---|---|---|---|
  | `seq`, `-T1` | 6,450,604 B | 258,888,897 B | 1 | `too-large-xz-block` | 4 | 1.2 ms | 0.001 ms | 6,450,772 B | 560 B |
  | `seq`, `-T4` | 3,541,316 B | 258,888,897 B | 11 | indexed | 4 | 0.2 ms | 0.003 ms | 3,543,020 B | 2,932 B |
  | base64, one block | 31,143,724 B | 40,526,316 B | 1 | held whole | 32 | 3,074 ms | 2,236 ms | 382,594,152 B | 382,594,112 B |
  | base64, 8 MiB blocks | 53,994,828 B | 70,245,617 B | 9 | indexed | 4 | 55.8 ms | 0.003 ms | 53,996,532 B | 2,928 B |
  | base64, 8 MiB blocks | 93,460,168 B | 121,578,948 B | 15 | before `too-large-copy`, after indexed | 4 | refused | 0.004 ms | refused | 2,976 B |

  An index keeps the same before and after: 32 bytes a block of
  `Seekable::kept_bytes`, 352 B for 11 blocks, and no checkpoint windows. A
  stream past the cap now costs the pass its index's bytes and a few
  hundred bytes besides, where it cost a copy of the stream. Read through a
  chunk store, the first steps can each wait for chunks: the one that reads
  the ends, the one that reads the index, and one for each 64 block
  headers. The last step reads nothing and checks the layout. A stream
  held whole costs what it did, since it is copied and decoded whole as
  before; its time is the decode, the same within the noise. Its peak is the copy, the output and the trace. Read through a
  handle, the copy is now made by the pass and held in its place in
  `Unfinished`; before this part it came from `gather_run` and counted
  against `GATHER_BYTES`.
- **Strings and JSON.** No reply shape changed. One refusal tag is new,
  `too-many-xz-blocks` ("The number of blocks" above), with its words in
  `Refusal::message` and `DECODED_REFUSED`, "too many xz blocks to unpack
  (over 65,536 blocks)"; after a field's name, "has too many xz blocks to
  unpack (over 65,536 blocks)"; in a checksum's line, "Can't check this:
  the data it covers is in an xz stream that has too many blocks to unpack
  (over 65,536 blocks)."; and after Open joined stream,
  `JOINED.partXzTooManyBlocks`, "Couldn't open {tab}: one of its parts has
  over 65,536 xz blocks." A report line says the listing's words with a
  capital, and the PNG views say what they say of a damaged stream. The hover on
  the two packed-size reasons, `PACKED_LIMITS` in `web/src/strings.ts`, now
  names xz: "The limit is 512 MiB for a deflate, zlib, gzip, zstd or xz
  stream that has no edited bytes and is not inside a joined stream." The
  status bar's `UNPACKED.unpacking` leaves out "{size} unpacked so far"
  while nothing has been unpacked, which for xz is the whole wait, where it
  said `UNPACKED.loading` while a stream was gathered; see "Still to do".
- **Tests.** In `codec/lazy/tests.rs`, each pass reads a store of 1 KiB
  chunks, fed only the chunks each step names, with steps of 8 KiB, from a
  store with room for two steps (20 chunks) and from one of 200. Over xz
  streams of one block, of 300 blocks, of blocks of many LZMA2 chunks, and
  of blocks with CRC64, SHA-256 and no check, each pass comes to the index
  the copy in memory comes to: the same `Layout` field by field, which is
  also `xz::layout`'s over the whole stream, the same checkpoints, members,
  weight, spans and span traces, and the bytes read back. No step names a
  chunk that lies wholly inside a block's LZMA2 data, none names more than
  half the store and three chunks, no chunk is asked for more than twice,
  the pass holds no more than the index and one piece, and progress never
  goes back (now checked for every pass `fed_pass` steps). The store of 200
  takes no more rounds than one for each 64 blocks, one for each half store
  of the stream, and one for the ends. Under a larger cap the stream is
  copied a piece at a time and comes back whole with the same bytes and
  trace. A block with a delta or an x86 filter among plain ones is refused
  as too large past the cap and as failed under it, from the store as from
  memory, and so are the streams `liblzma` wrote with those filters, read a
  few bytes a piece. One block past the span limit, a stream past
  `Limits::lazy` and, from the store only, a block too long to read at once
  are refused as from memory. A stream under the cap past `copy_input` is
  refused as too large from the store, and one past the cap opens. With
  each byte of the stream's header, index and footer changed in turn, the
  pass from the store comes to what the pass from memory does, and past
  the limit on a copy it is refused as failed where nothing lays it out,
  as too large to copy where its index says it fits under the cap, and as
  in memory otherwise. A stream past the limit on a copy with a damaged
  index, and one with a block header that says it is longer than its
  block, are refused as failed from a store; one with stream padding, and
  two streams joined end to end, as too large to copy. A stream of one
  block more than `block_marks` is refused as having too many blocks, from
  a store and from memory, under the cap and past it, having read no chunk
  inside a block's data; at the limit it is indexed. In
  `eval/tests/resident.rs`, an xz stream of 38 blocks of noise, more than
  three times a store of 160 chunks, opens in goes past the cap: each wait
  names at most half the store, `Evaluator::unpacking` answers after each
  with progress that never goes back, no refusal is kept, the pass starts
  once, the index holds no copy and is the one made in memory, and fewer
  than a quarter of the file's chunks are read to lay it out; under a cap
  it fits under, it opens held whole. In
  `crates/wasm/tests/open_space_waits.rs`,
  `open_unpacked_on_an_xz_stream_longer_than_the_store_lays_it_out_a_few_headers_at_a_time`
  opens an xz stream of 25 blocks of 256 KiB, written by hand in stored
  LZMA2 chunks, about 100 chunks of 64 KiB, over a store of 16. Laid out
  past a lowered cap, opening it asks for 29 of the 100 chunks; decoded
  whole under the default cap, it asks for every chunk. Either way no chunk
  is asked for more than twice, no reply names more than 10 chunks, the
  status says how far unpacking has got and that never goes back, and the
  tab reads the stream's bytes.
  `an_xz_stream_past_64_mib_packed_opens_end_to_end`, marked `#[ignore]`,
  opens 66 MiB of noise in xz blocks of 1 MiB under the default limits,
  from a store of 4 MiB, and its tab reads the right bytes. In
  `codec/lazy/tests.rs`, a step that waits for one block header's chunk
  counts the headers it has read in its progress. The real
  `xz-T0-blocks.xz` in the sample collection reads as before, and laid out
  through an `ArcSource` in steps of 64 KiB it comes to the index the copy
  comes to (`lazy_frames_real`).
- **Checked by hand.** `base64-120M-blocks.xz`, 93,460,168 bytes of `xz -T4
  --block-size=8MiB -6` over base64 lines of random bytes, 15 blocks and
  121,578,948 bytes unpacked, opens through the wasm `Editor` with the
  browser's store of 512 chunks of 64 KiB, and five reads of its tab agree
  with `xz -dc`. Opening it took 8 "pending" replies, which asked for 113
  of the file's 1,427 chunks, each once: the stream's header, its footer
  and index, its 15 block headers, and the first block's data, about
  6 MB, which the reading decodes to see what the stream holds. The
  widest reply, that block's data, named 97 chunks.
- **Tests changed on purpose.**
  `a_pass_reads_in_windows_only_from_a_store_with_room_for_two` asserted
  that an xz pass never reads in windows; it now checks xz's window of
  `STEP_INPUT` with the others. `a_pass_is_held_to_the_limit_of_the_way_it_reads_its_run`
  said an xz stream's layout always reads a copy; xz now goes through the
  same checks as zlib and zstd.

## Still to do

- **The time a joined stream takes to open.** Since "Faster opening of
  joined streams" no part is unpacked to open a stream read a part at a
  time. What is left is the walk to the parts, which reads every block's
  header and trailer: about 0.3 s natively for a BAM of 4,821 blocks, and in
  the browser it loads every chunk of the file, 96 MB for that BAM, before
  the tab opens. A BAI, CSI or GZI index beside the file names where the
  blocks are, and could stand in for reading them. Not measured in the
  browser after this change.
- **The format readers over a joined tab, tested for real.** The HDF5, ELF,
  ROOT, hardening and JPEG calls refuse through the shared count, and the
  test checks them only on a stream of text, which none of them reads far.
  An HDF5 file or an ELF inside a .gz of many members, with a bad member
  where the reader reaches, would test each one's own reads.
- **A part read through its index that refuses a span** keeps no trouble,
  so `unreadable` cannot name it, and the hex view shows zeroes there. It
  could keep the refused span as a trouble of its own, by range rather than
  by part.
- **The browser tests of the text view** (`web/test/textview.browser.mjs`)
  were not extended. In the browser pane, on the BAM of 300 MiB with block
  2's trailer one byte long, the text view's muted `?` with the reason on
  hover and its `unpacking failed` marks, the inspector's `unreadable bytes` boxes
  for the 8-byte values that reach the block, the selection's reason, and
  the strings view's `63.8 KiB not searched: …` were seen, before the
  strings were reworded after a cold read. The minimap's
  reason was not: the pane was hidden, and the minimap draws on a frame.
- **The reason is on hover**, over the `??` cells, the `?` characters and the
  `unreadable bytes` boxes, which a touch screen never shows. The text
  view's `unpacking failed` mark and the inspector's selection line say that the
  bytes cannot be read without a hover; only the reason itself needs one.
- **The CRC-32 of a part when it is first read.** A part that inflates to
  the length its trailer says with the wrong bytes reads as those bytes.
  `Ty::Stitched` could name each part's checksum field beside `part_len`, a
  BGZF block's `crc32`, and `Joined::unpacked` and `unpacked_part` compare it
  when they first decode the part, as they compare its length. It costs a
  CRC-32 of each part on its first read, about a fifth on top of the inflate
  with the table-driven `crc32`.
- **The listing and the tab decode the same parts twice.** A BAM's tab reads
  its records through the stitched space's cache (`Stitch::cache`) and its
  bytes through the joined stream's (`Joined::cache`), so a part on screen
  in both is decoded once for each, and a part in trouble is found once by
  each. One cache, and one record of the parts in trouble, shared by the two
  would halve that.
- **A joined stream's limit on one read is set by its worst part.**
  `handle_limit` takes the largest ratio of run to output over every packed
  part read through a handle, so one short part that packs badly lowers the
  limit for reads anywhere in the stream, to about a quarter of what the
  store keeps for a part whose run is twice its output. A limit worked out
  for each read, from the parts it covers, would avoid that, but
  `Source::resident_limit` is one number for the whole source.
- **Which part of a damaged xz stream is damaged.** A damaged stream past
  64 MiB packed is refused as `failed`, which says nothing of where. The
  pass knows whether it was the header, the footer, the index or a block
  header, but a refusal carries only its tag. A detail beside the tag in a
  node's `refused` would need a new field in the reply.
- **Stream padding and joined xz streams** are read only by the decoder
  of whole streams, so past 64 MiB packed they are refused as
  `too-large-copy`, although the hover says an xz stream may be 512 MiB.
  `xz::layout` could skip stream padding, and lay out each stream of a
  joined file from its own index.
- **An xz stream inside a stream read through an index.** A tab's reading
  reads an xz stream in the tab through `SpaceSource::Lazy`, whose reads
  decode the outer stream's spans. One step's block headers lie within
  half of the tab's resident limit (the span cache, 16 MiB) of each other,
  so they can fall in several of the outer stream's spans, and one answer
  can name the chunks of more than one span. Headers read are kept and
  count as progress, so the pass gets further each round, but no test
  covers it.
- **The status bar while an xz pass runs.** An xz pass unpacks nothing
  until its end, so `UNPACKED.unpacking` gives only the percentage (it
  leaves out "{size} unpacked so far" while that is 0, rather than saying
  0 B). A stream past the cap is laid out in a few rounds. A
  stream that fits is copied a piece at a time, so the percentage reaches
  100% and the stream is then unpacked in one go; before part 8 the status
  bar said `UNPACKED.loading` while such a stream was gathered. `Unpacking`
  names no codec, so the web cannot word it differently yet.
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
- **`CHUNK_CAPACITY`.** A larger store would let `unit_input` grow. That is a
  choice about the browser's memory, not part of this design.
- **The measuring walk of a large gzip member.** `stream_len_at` reads a
  member's bytes into the walk's own copy, up to 64 MiB, until the walk
  finds the stream's end or passes `Limits::measure`. A member past 64 MiB
  compressed always passes it, so a .gz of one such member reads and holds
  64 MiB of itself before the template falls back to the trailer, and two
  walks can hold 128 MiB at once. A .gz of several members, one of them
  past 64 MiB of output, still reads as one member. This is part 9, which
  is deferred; letting go of the walk's bytes before its current step would
  save most of the memory.
- **A checksum over more than 64 MiB of the file.** `run_check` refuses a
  sum over bytes of the file (`Coverage::over`) past `CAP_BYTES`, so the
  CRC-32 of a zip entry stored as it is, past 64 MiB, is not checked. It
  could be summed a window at a time, as `gather_run` reads.
- **Runs that keep the 64 MiB limit.** A run read through a copy:
  a run in a stitched space or in an edited stretch, a run in a source with
  too little room for two windows, and an xz stream decoded whole, which is
  one of 64 MiB or less unpacked, or one whose index or footer does not hold
  together. Since part 8 an xz stream read through a handle and laid out
  from its index takes `Limits::input`. Since part 10, a joined stream read a part at a time counts
  against `whole_cap` only the parts it copies: those no handle reaches, and
  packed runs decoded whole that are longer than one read from the store may
  take (see "Built (part 10)").

## Open questions

- Should `TooLarge` be split, so that the listing can say whether the
  compressed run or the output is too large? With this design the compressed
  limit is 512 MiB, and the output limits are 64 MiB for a run held whole and
  1 GiB for an indexed run. One phrase cannot say all three. Part 7c gave
  the 512 MiB limit a refusal of its own, `too-large-input`; `too-large`
  still said both a copy past 64 MiB compressed and a run held whole past
  64 MiB of output.
  **Answered: split, in three.** `TooLargeCopy` (`too-large-copy`) is a
  compressed run past 64 MiB read from a copy held whole, which is every
  run that is not read through a handle, and a joined stream whose copies
  of its parts pass `whole_cap`. `TooLargeOutput` (`too-large-output`) is a
  run decoded whole that comes to more than 64 MiB, or that its stream or
  container says will. `TooLargeWindow` (`too-large-zstd-window`) is a zstd
  frame asking for a window over 128 MiB, which `TooLarge` said before
  although nothing says how large such a frame is. Every size in the words
  now says which size it is: "too large to unpack (over 64 MiB packed)",
  "(over 512 MiB packed)", "(over 64 MiB unpacked)" and "(over 1 GiB
  unpacked)". "Packed", the size before unpacking, rather than
  "compressed": a stored ZIP entry, a base64 run and a PNG's inflated
  scanlines are refused past 64 MiB too, and one word for both packed
  limits reads as one size with two limits. The hover on the two packed
  reasons is the same text, saying which streams may be 512 MiB packed, and
  the hover on the two unpacked reasons does the same for 1 GiB, so a
  reader who meets both limits in one file reads the same rule on either
  row. See "Refusal tags" under "Limits".
- Is the length check in `join_lazily` worth a walk across goes, or should a
  part of the wrong length show as a failed read when it is read?
  **Answered in part 10: the check stays.** Over handles it costs time and
  no memory: the walk holds one packed run and its output at a time. The
  tests in "Tests" expect a part of the wrong length to refuse the stream
  as before. Without the check, the tab would open over that part and show
  zeroes there, since `Joined::read` refuses the part and a refused read of
  a space reads as zeroes with no chunks named.
  **Answered again, after part 10: a failed read, when the part is read.**
  The check was about 90% of the time a BAM of 300 MiB took to open, so the
  walk no longer makes it. The reason it stayed, zeroes shown for the part,
  is answered by making the zeroes say what they are: the read is refused,
  `Source::unreadable` and `Evaluator::unreadable` name the part's bytes and
  why, a tab's own reading fails with those words, and the hex view shows
  the bytes as `??` with the words on hover. A part of the wrong length never
  shifts the bytes after it, since the stream is laid out from the lengths
  the file gives. A part the file says unpacks to nothing, which no read
  reaches, is still checked when the stream opens, and a stream held whole
  still refuses a bad part when it opens. See "Faster opening of joined
  streams".
- Should gathering report a percentage, and with what string?

## What was checked

When the proposal was written, on the branch as merged with main on
2026-11-26, these were read in full or in the parts named. The code has
changed since, part by part: `Seekable::packed`, `join_lazily` and
`fetchChunk`, for example, are gone. Each "Built" section says what its part
changed, and DESIGN-lazy-unpacking.md was checked against the code on
2026-11-28 (part 11), including the joined streams of "Faster opening of
joined streams".

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

The proposal's figures were not measured: they are estimates, such as the
rounds worked out from `READ_AHEAD` and the chunk size. The timing and
memory figures in the "Built" sections and in "Faster opening of joined
streams" were measured, each with the build, machine load and date it gives.

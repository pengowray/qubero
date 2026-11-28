# Design: unpacking large compressed runs a piece at a time

Written 2026-10-21, for developers. Brought up to date with the code on
2026-11-28.

This document covers the output side of a compressed run whose output is
larger than `codec::CAP_BYTES` (64 MiB): the index pass that finds places to
restart the decoder, done a step at a time; reading the output a piece at a
time through that index; the spans, block starts and traces the index gives;
and how the browser waits for a pass and shows its progress. Deflate, zlib
and gzip come first; zstd and xz are in "zstd frames and xz blocks"; the
other codecs are covered at the end. DESIGN-lazy-compressed-input.md covers
the input side: how the index pass, and a read of a span, get the run's
compressed bytes, either a window at a time through a handle onto the file's
chunk store, or from a copy of the run.

A go, here, is one call into the core with an allowance of work, counted in
elements (5,000 in the web app, `WORK_SLICE`). A call whose go runs out
answers `Busy`, and the next call carries on where it stopped.

**State:** built and in main, in `codec/lazy/`, `eval/space.rs`,
`eval/stitch.rs`, the wasm crate and `web/src`:

- The index pass, the spans and reads through them, for deflate, zlib and
  gzip runs, zstd runs of several frames and xz streams of several blocks.
- The `blocks` listing for a deflate run read a piece at a time. See
  "Blocks".
- The index pass, and the walk that finds where a gzip member's stream ends,
  run a step at a time, each step charged to the go, so the browser keeps
  drawing and says how far each has got. See "The pass in steps" and
  "Measuring a member's stream in steps".
- `Unfinished` keeps at most two passes that have not finished, and a pass
  that has started is never pushed out to make room for another. See
  "Waiting for a place".
- One press of Open unpacked is enough: the browser loads the chunks the
  stream needs, waits while the stream is unpacked, and then opens its tab.
  See "The browser".

"Still to do" at the end lists what is left.

## The problem

A codec in `codec.rs` decodes a run whole into one `Vec<u8>` and refuses
with `Refusal::TooLargeOutput` once the output passes 64 MiB. The cap guards
against zip bombs, and it also refuses ordinary large files. The test case is a
gzipped Rubin LATISS raw, `qubero2-extras/rubin/3018092000065-det000.fits`:
94,940 bytes of gzip that inflate to 75,677,760 bytes (a ratio of 797 to 1),
which hold a FITS file: a primary HDU, 16 image HDUs named `Segment00` to
`Segment07` and `Segment10` to `Segment17` (576 × 2048 32-bit integers each, not tile-compressed) and two
binary tables.

Before runs past the cap were read a piece at a time, the gzip template read
that file through three paths, and all three hit the cap. They are the paths
the design changes:

1. `members[0].compressed` is a `Decoded` run sized by
   `E::stream_len(Codec::Deflate)`. `inflate_len` inflates it to the cap,
   refuses, and the template falls back to "everything before the trailer".
   That cost a 64 MiB inflate before anything was shown (about 170 ms native
   at the time, when the inflate still kept its output). `inflate_len` still
   inflates up to 64 MiB before it refuses, but it counts the output without
   keeping it, and the evaluator runs it a step at a time: see "Measuring a
   member's stream in steps".
2. `decoded` is a `Stitched` stream over every member's `compressed` run, with
   each part as long as the member's `original_size`. A read of it reached
   `Evaluator::unpacked_part`, which inflated the whole part through
   `codec::decode` and failed. A read of `decoded` now reads such a part
   through the part's index: see "Stitched streams" under "Where it plugs
   in".
3. Open unpacked on `decoded` called `join_whole`, which refused a joined
   stream past `whole_cap` (64 MiB) before reading anything. Open unpacked
   on `decoded` now opens a joined document that is read a part at a time.

A `Decoded` node opened directly (`open_space_at`) refused the same way, and
now opens through its index.

## What changes for the reader

- A gzip, zlib or raw deflate run that inflates to more than 64 MiB and at most
  1 GiB opens: its fields read in the listing, and Open unpacked gives a tab
  over the whole output. The run itself may be up to 512 MiB compressed when
  its pass reads it through a handle onto the source it is in, as for a run
  of the file that no edit touched, and up to 64 MiB when the pass reads a
  copy of it (see "Limits").
- The tab, the listing and the hex view read the output a piece at a time.
  Memory stays bounded whatever the output size (see "Limits").
- Selecting a byte of the unpacked tab shows the deflate code that made it,
  and selecting a bit of the compressed run shows the bytes that the bit's
  code made, as for a run held whole.
- The compressed run has its `blocks` child: one row per deflate block,
  and inside each block its header, tables and codes. The rows inside a block
  are made when the block is opened. See "Blocks".
- A zstd run of several frames and an xz stream of several blocks open the
  same way, as long as no one frame or block comes to more than 64 MiB. A
  run with a zstd frame or an xz block whose output passes 64 MiB is
  refused, and its row says that a zstd frame or an xz block is too large to
  unpack; a file of one frame or one block, which plain `zstd` and `xz`
  write, is refused this way once its output passes 64 MiB. A run of any
  other codec is refused when its compressed size or its output passes
  64 MiB.
- Runs at or under 64 MiB of output are decoded and traced whole.

## Limits

The constants are in `codec.rs`, `codec/lazy/mod.rs` and `eval/space.rs`.
`lazy::Limits` carries the ones a pass is held to, so that tests can make
them small.

| Limit | Value | What it bounds |
|---|---|---|
| `CAP_BYTES` (`Limits::whole`) | 64 MiB | Output of a run held whole. Past it a run decoded whole is refused as `TooLargeOutput`, and a run that can be indexed is read a piece at a time instead. |
| `LAZY_CAP_BYTES` (`Limits::lazy`) | 1 GiB | Output of a run read a piece at a time. The zip-bomb guard. Past it the run is refused as `TooLargeLazy`. |
| `Limits::input` and `Limits::copy_input` | 512 MiB and 64 MiB | Compressed size of a run that can be indexed: 512 MiB when its pass reads it through a handle onto its source, 64 MiB when the pass reads a copy of it. Refused past them, before any of the run is read, as `TooLargeInput` and `TooLargeCopy`. A run of a codec that cannot be indexed is read whole, and refused past 64 MiB as `TooLargeCopy`. See DESIGN-lazy-compressed-input.md, "Limits". |
| `MAX_CHECKPOINTS` | 256 | Restart points kept per run. 256 × 32 KiB = 8 MiB of windows. |
| `SPACING_START` | 1 MiB | Output between restart points, doubled whenever the count passes 256. |
| `IN_SPACING_START` | 1 MiB | Compressed input between restart points, doubled with `SPACING_START`. Both reach at most 8 MiB. |
| `SPAN_CACHE_BYTES` | 16 MiB | Decoded output kept per run. |
| `SPAN_TRACE_STEPS` | 1,000,000 | Steps in one deflate span's trace before it names blocks instead of codes, as `MAX_STEPS` does for a whole run. Also the steps in one deflate block's trace, and in all the block traces a run keeps. A zstd or xz span's trace may hold `MAX_STEPS` (4,000,000), since such a span can be as large as a run decoded whole: a 24 MiB `xz -T0` block of text takes 2.3 million steps, about 47 MB of trace. |
| `MAX_BLOCK_MARKS` (`Limits::block_marks`) | 65,536 | Deflate block starts kept per run. 65,536 × 16 bytes = 1 MiB. Also the most blocks an xz stream's own index (the list at the end of the stream) may list: past it the stream is refused as `TooManyXzBlocks`, since the `Seekable` keeps an entry for every xz block. |
| `Limits::span` | 64 MiB (`CAP_BYTES`) | Output of one zstd frame or one xz block. These are decoded from their start, so a larger one is refused, as `Refusal::TooLargeUnit`. A field of its own, not `whole`, so that tests can index real `pzstd` and `xz -T0` files with `whole` set small: those tools choose their own frame and block sizes, 2 MiB and 1 MiB at the smallest. |
| `MAX_WINDOW` (`codec/frames.rs`) | 128 MiB | The zstd window a frame may ask for. A frame asking for more is refused as `TooLargeWindow` before any of it is decoded. |
| `Limits::measure` | 64 MiB (`CAP_BYTES`) | Output a walk measuring a gzip member counts before it refuses. See "Measuring a member's stream in steps". |
| `INDEXED_RUN_BYTES` | 64 MiB | What the indexes kept in `Indexes` may weigh between them. See "Where it plugs in". |
| `UNFINISHED_PASSES` | 2 | Passes kept in `Unfinished` between goes. See "Waiting for a place". |
| `GATHER_WINDOW` | 1 MiB, or a quarter of what the file's source keeps when that is less | The most of the file one read takes when a run is read a window at a time across goes, called a gather window below. See DESIGN-lazy-compressed-input.md, "Gathering a run a window at a time (part 2)". |

When the run is read from a source that keeps only part of the file, as the
browser's chunk store does, `Limits::unit_input` and `Limits::span_input`
(16 MiB each) bound the compressed input that one zstd frame, one xz block or
one span read may need at once. A run with a frame, block or span read that
needs more than that is refused as `TooMuchAtOnce`. They are input-side
limits: see
DESIGN-lazy-compressed-input.md, "Checkpoints by compressed distance".

**Why an output limit and not a time limit.** Time depends on the machine, so
a time limit makes the same file open on one computer and not on another, and
tests cannot check it. A ratio limit would refuse the test file, which is 797
to 1; deflate's own maximum is about 1032 to 1, so any ratio limit low enough
to matter refuses real files. An output limit is the same everywhere. 1 GiB of
output costs at most about 8 MiB of checkpoints, and output positions stay
inside the `u32` that `RawStep.out_start` stores.

**What a bomb costs.** Memory is bounded by the table above, so a bomb can
only cost time: one pass over up to 1 GiB of output and 512 MiB of input,
and for a gzip member, a measuring walk over up to 64 MiB of output first.
The measured speed of the decoder is about 70 MB/s native with tracing on
(60 MB of text-like data in 832 ms), so the worst case for the output is
around 15 s native, more in wasm. A run past 1 GiB is refused as soon as the
pass reaches 1 GiB.

### Refusals

Each refusal below names the limits that cause it, and the interface looks up
the words for a refusal by its tag. The words below are
`Refusal::message`, which `DECODED_REFUSED` in `web/src/strings.ts` repeats.

| Refusal | Tag | Words | When |
|---|---|---|---|
| `TooLargeCopy` | `too-large-copy` | too large to unpack (over 64 MiB packed) | A run past 64 MiB read from a copy held whole, and a joined stream whose copies of its parts pass `whole_cap`. |
| `TooLargeOutput` | `too-large-output` | too large to unpack (over 64 MiB unpacked) | A run decoded whole whose output passes 64 MiB, and an xz stream past the cap with a delta or branch filter. |
| `TooLargeLazy` | `too-large-lazy` | too large to unpack (over 1 GiB unpacked) | A run read a piece at a time whose output passes 1 GiB. |
| `TooLargeInput` | `too-large-input` | too large to unpack (over 512 MiB packed) | A run past 512 MiB whose pass reads it through a handle. |
| `TooLargeUnit(ZstdFrame)` | `too-large-zstd-frame` | a zstd frame is too large to unpack (over 64 MiB unpacked) | One zstd frame past `Limits::span`. |
| `TooLargeUnit(XzBlock)` | `too-large-xz-block` | an xz block is too large to unpack (over 64 MiB unpacked) | One xz block past `Limits::span`. |
| `TooManyXzBlocks` | `too-many-xz-blocks` | too many xz blocks to unpack (over 65,536 blocks) | An xz index that lists more than `Limits::block_marks` blocks. |
| `TooLargeWindow` | `too-large-zstd-window` | zstd with a window over 128 MiB, not unpacked | A zstd frame asking for a window past `MAX_WINDOW`. |
| `TooMuchAtOnce` | `too-much-at-once` | too large for the editor to keep in memory at once | A read that needs more of the file at once than the source keeps, such as a zstd frame or an xz block over 16 MiB compressed in the browser. |

`Refusal::after_name` gives the same words after a field's name, as a read of
a joined stream's part says them: `is too large to unpack (over 1 GiB
unpacked)`, or `has a zstd frame that is too large to unpack (over 64 MiB
unpacked)`. The tags are listed once, in `codec.rs`, and
`web/src/refusaltags.ts` is generated from that list: see
DESIGN-lazy-compressed-input.md, "Refusal tags".

## Deflate: restarting in the middle of a stream

A deflate decoder at any point in a stream needs three things to carry on:

1. the bit position of the next code,
2. the last 32 KiB of output, because a match can copy from up to 32,768 bytes
   back,
3. the Huffman tables of the block it is in.

zlib's `zran.c` example keeps (1) and (2) at block boundaries only, because
zlib's inflater can only be entered at the start of a block. Qubero's inflater
is its own (`codec/inflate.rs`), so it can be entered anywhere, and item (3)
is cheap: a checkpoint inside a block keeps the bit position of that block's
header and a copy of the header and its tables, at most 287 bytes
(`MAX_HEAD_BYTES`), and restarting reads them again from the copy. A block
can be as long as the run, so its header can be anywhere earlier in it, and
the copy means a restart reads the run only from the checkpoint on. A
checkpoint is (`codec/lazy/mod.rs`):

```rust
struct Checkpoint {
    in_bit: u64,   // where the next code starts, in bits of the run
    out: u64,      // how many bytes of output come before it
    start: Start,
}

enum Start {
    Deflate {
        block: Option<u64>,       // bit of the header of the block it is inside, if any
        head: Option<Arc<[u8]>>,  // a copy of that header and its tables
        blocks: u64,              // how many deflate blocks start before it
        window: Arc<[u8]>,        // the last min(32 KiB, out) bytes of output
    },
    Frame { own: usize },         // zstd: byte where the span's first frame starts
    Unit(usize),                  // xz: the block's number in the stream's index
}
```

Checkpoints inside a block keep the work per read bounded however the encoder
chose its blocks. zlib ends a block every 16,384 codes by default, which at
258 bytes per match is up to 4.2 MB of output, and an encoder may write one
block for a whole file. Checkpoints are never taken inside a stored block; a
stored block holds at most 65,535 bytes, so a span overshoots its target by at
most that much.

### The index pass: one walk

Nothing says how large the output is before decoding it: raw deflate and zlib
record no size, and gzip's `ISIZE` is the size modulo 4 GiB of one member. So
the decision is made during the decode, in one walk:

1. Decode as a run held whole is decoded, into a growing `Vec<u8>`, with the
   trace on, and record a checkpoint every `SPACING_START` of output
   (copying 32 KiB out of the `Vec` is about 3% of the work).
2. If the stream ends at or under 64 MiB: drop the checkpoints and return the
   bytes and the trace, exactly what `decode_traced` returns.
3. If the output passes 64 MiB, the decoder stops with `TooLargeOutput`, as
   it does for a run held whole, and the pass goes on from there. Keep the
   checkpoints, take the CRC-32 and
   Adler-32 of the output up to the last checkpoint, drop the `Vec` and the
   trace, and decode again from the last checkpoint with the trace off,
   keeping only the last 32 KiB of output and recording checkpoints. The
   checksums are brought up to date as output leaves the window. Starting
   again costs at most one spacing of output decoded twice.
4. At the end, check the gzip or zlib trailer, and return an index: the
   `RunInput` it reads the run through, the checkpoints, the block starts
   (see "Blocks"), the output length, both checksums, and where the deflate
   data starts and ends inside the run.

The peak memory of the pass is the first decode's: up to 64 MiB of output
and its trace, which is what decoding a run at the cap costs.

Past `LAZY_CAP_BYTES` the walk stops and the run is refused as
`Refusal::TooLargeLazy` (tag `too-large-lazy`), which the interface words as
"too large to unpack (over 1 GiB unpacked)".

A checkpoint is also taken when the compressed input since the last one
reaches `IN_SPACING_START` (1 MiB), so that a span's compressed input is
bounded as well as its output: a run of empty blocks makes no output. When
the checkpoint count passes `MAX_CHECKPOINTS`, every second checkpoint is
dropped and both spacings double, to at most 8 MiB. A 75 MB run keeps about
72 checkpoints at 1 MiB (2.3 MiB of windows); a 1 GiB run whose input is
much smaller than its output keeps about 256 at 4 MiB. See
DESIGN-lazy-compressed-input.md, "Checkpoints by compressed distance".

Measured in a release build, native:

| Run | Packed | Unpacked | Index pass | Spans | Windows | Notes |
|---|---|---|---|---|---|---|
| The Rubin raw | 94,940 B | 75,677,760 B | 0.38 s | 73 | 2.3 MiB | |
| Generated CSV-like text | 18.3 MB | 500 MB | 2.1 s | 239 | 7.8 MiB | 50 random 64 KiB reads: 0.2 s. Reading all 500 MB in 1 MiB reads: 0.97 s. Peak RSS 117 MB. |

`inflate_len`, which sizes a gzip member's deflate run in the listing, stops
at 64 MiB of output (`Limits::measure`). Past that the template's fallback
(everything before the trailer) is right for a file of one member, which the
test file is. Raising it would make a hostile file cost the full 1 GiB walk
once to size the member and again to index it. It counts the output rather
than keeping it, and keeps no trace, since only the end of the stream is
wanted. The evaluator makes the same walk a step at a time: see "Measuring a
member's stream in steps".

### The pass in steps

`lazy::Indexing` is the same pass made resumable. `Indexing::step(out_bytes,
in_bits)` decodes about that much more output, or reads about that many bits
of the run, stops, and the next step carries on from exactly where it stopped,
not from the last checkpoint: the sink (with its 32 KiB window of output, or
its whole output), the trace builder, the checkpoints, the block starts, the
running sums and the pass's window of the run (the part of the compressed run
it has loaded) are all kept in the `Indexing` between steps. `decode_or_index`
steps it with no limit until it is done. Deflate's, zstd's and xz's passes are
all done in steps (`lazy::can_step` says yes to every codec `lazy::can_index`
does). This section is about deflate's; zstd's and xz's are in "zstd frames
and xz blocks".

- **Where a step stops.** At the first code past its share of output, or past
  its share of bits read, whichever comes first, and never inside a stored
  block (so at most 64 KiB over). See "A step bounded by the bits it reads".
  The pass keeps a copy of the header and tables of the block it stopped in.
  In the traced phase a step that stops inside a block leaves the block open
  in the trace (`TraceBuilder::carried_on`, `codec::Paused`), and the next
  step reads the block's header and tables again from that copy, with the
  trace muted. So a pass cut into steps gives the same trace, entry for
  entry, as one pass run without stopping. That includes a block in which
  the trace passes `MAX_STEPS`, whose symbols are then named as one trace
  entry.
- **Block starts.** A step that stops at the start of a block does not record
  that start. The next step starts there, so the decoder tells the sink about
  the same start again, and the start is recorded then. A step that stops
  inside a block has recorded its start already, and the next step reads the
  block's header again without telling the sink about it, since a decode
  starting inside a block only calls `Sink::boundary` from the first code on.
  So each block is recorded once however the pass is cut into steps. The
  checkpoint the pass goes back to at the cap keeps how many blocks start
  before it, as in "Blocks".
- **The sums.** In the traced phase they are brought up to the latest
  checkpoint at the end of each step, because past the cap the pass goes back
  to that checkpoint. A pass in one go never stops, so it sums as it goes.
- **Progress.** `Indexing::progress` gives the bits of the run read and the
  bytes unpacked, each at the furthest it has been, since the pass goes back a
  little at the cap.
- **Skipping the traced phase.** `Indexing::known_large` starts with the trace
  off, for a run the caller knows is past the cap: a stitched stream's part
  whose length the file gives (`index_part`). A gzip run whose trailer says
  more than the cap starts that way too. It comes to the same index; a run
  that turns out to fit under the cap comes back indexed, and its caller finds
  the length is not the one it was told.

In the evaluator, `Evaluator::index_run` is the one way in, used by
`open_space_at`, `measure_run`, `index_part` and `unpacked_part`. Each step,
of 256 KiB of output or 2 Mibit of the run, whichever comes first, is
charged to the go's allowance first (`Go::charge`, which takes the evaluator
shared, since reads of a stitched stream reach here and reads do), and a go
that runs out hands back `Busy` with the pass put back in `Spaces`'
`Unfinished` table, keyed by the run like `Indexes`. A step whose window of
the run is missing chunks of the file answers `Stepped::NeedsInput`, having
done nothing; the pass is put back the same way, and the call hands back
`Pending` with those chunks, which is never a refusal (see
DESIGN-lazy-compressed-input.md, "How `Pending` fits the go machinery").
`Unfinished` has places for only two unfinished passes, since one in its
traced phase holds up to 64 MiB, and a pass that would start while both are
taken waits for one (see "Waiting for a place"). A finished index goes to
`Indexes`; a half-made one never leaves `Unfinished`, because a tab's source
reads an index with no evaluator behind it. `Spaces::forget` drops both.
`Evaluator::unpacking` reports the pass the current go ran out of allowance
in, or waited for chunks in, which is the pass the call that answered `Busy`
or `Pending` waits on (see "Which pass a call waits on"). zstd and xz passes
are stepped there the same way, each step charged alike.

### A step bounded by the bits it reads

If a step ended only once it had made its share of output (`out_bytes`, 256
KiB in the evaluator), a run that makes no output would be indexed in a single
step, however long the run is. A deflate run can be all empty blocks. The
smallest is an empty fixed-Huffman block of 10 bits: a 3-bit block header and
the 7-bit end-of-block code. A run read through a handle may be up to 512 MiB
of compressed bytes (`Limits::input`), which can hold about 430 million of
them, and a run read from a copy up to 64 MiB, about 54 million. zstd has the
same kind of run: frames with no content, and skippable frames, which are at
least 8 bytes each.

So `Indexing::step(out_bytes, in_bits)` also stops a step once it has read
`in_bits` bits of the run since the step began, at the next place the
following step can resume from:

- **Deflate.** The next code or block start, the same places a step stops at
  for its share of output. The bits are counted from the code the step
  resumes at, not from the header of the block it is inside, whose tables it
  reads again first. So every step decodes at least one code, and the pass
  always moves forward, however long the block's tables are.
- **zstd.** The next boundary between two zstd blocks of a frame, between two
  frames, or between two skippable frames, once the step has read
  `in_bits / 8` bytes. The skippable frames in front of a frame count as part
  of that frame, so a checkpoint taken at the frame starts at the first of
  them (see "zstd: one pass"). A step can stop partway through a sequence of
  skippable frames, so the zstd walk keeps the offset of the first one, and
  the frame after them, and any checkpoint taken at it, start at that offset,
  as they do when the pass runs to the end in one call.

The index that steps come to is the index the pass makes when it runs to the
end in one call. Tests check this for both codecs (see "Tests").

The evaluator gives each step 2 Mibit (`INDEX_STEP_BITS`), the number of
bits in the step's 256 KiB share of output. So where the run unpacks to at
least its own size, a step still ends at its share of output. A step ends by
its bits only over a stretch of the run that unpacks to less than it takes
up. A pass that reads its run a window at a time also stops each step at
the first place it can stop that is at least `Limits::step_input` (256 KiB
of the run) past the step's start, whatever `in_bits` it is given, so that
the step stays inside its window. Under the default limits the two bounds
are the same.

Building deflate's two fixed Huffman codes, for literals and lengths and for
distances, once (`fixed_codes`) instead of for every fixed block made
measuring an empty fixed block about 50 times faster (`inflate_len`), and
decoding one with its trace about 16 times faster (`decode_traced`).

Timed on 2026-11-26, native, with a bench outside `cargo test` built at
`opt-level` 3 without LTO, on a raw deflate stream of 838,861 empty fixed
blocks (1 MiB of compressed bytes). The machine was loaded (a load average
of about 15 on 12 cores), so each cell gives two timings, taken one after
the other.

| | Before | Fixed codes built once |
|---|---|---|
| Decoding with the trace (`decode_traced`), each block | 1,840 and 1,630 ns | 116 and 95 ns |
| Measuring in one call (`inflate_len`), each block | 1,740 and 1,520 ns | 35 and 29 ns |
| Longest measuring step (1 Mibit of the run) | 188 and 180 ms | 3.7 and 3.2 ms |
| The index pass, with the trace, all 1 MiB in one step | 1,357 and 1,423 ms | 95 and 98 ms |
| The index pass with the trace off (`known_large`), all 1 MiB in one step | 1,298 and 1,307 ms | 35 and 36 ms |

The trace is on while the run may still be held whole, that is while its
output is under the cap (64 MiB), and a run of empty blocks never passes it.
Most of an empty block's cost then is the trace: each empty block adds one
block and three trace steps to it.

With the fixed codes built once and steps of 2 Mibit, on a stream of
6,710,886 empty fixed blocks (8 MiB), each figure the median of three
timings:

| | Steps | Longest step | Longest go |
|---|---|---|---|
| With the trace | 32 | 31 ms | 248 ms |
| With the trace off (`known_large`) | 32 | 9.5 ms | 89 ms |

A go of 5,000 elements, which is what the web app asks for, runs at most ten
steps: each step is charged 512 elements before it starts, and a go starts
another step while any of its allowance is left. The longest go in these
tables is the longest ten steps in a row, added up, from the bench; no go was
timed through the evaluator.

Steps of 1 Mibit were tried as well. They made the longest go 127 ms with the
trace and 56 ms with it off, but a gzip whose output is 100 MB, three fifths
random bytes and two fifths zeros in pieces of 1 MB, took 601 steps
instead of 381, about 0.5 ms each, and so 61 goes instead of 39. 2 Mibit was
kept so that a run like that one, or a run of stored blocks, steps by its
output as before. With 2 Mibit, the two files timed in "Measuring a member's
stream in steps" took the same number of steps as before: 458 for the
CSV-like text (120 MB of output) and 306 for the literals of one or two bits
(80 MB), with the longest go 32 to 44 ms.

### Measuring a member's stream in steps

A gzip member's deflate run has no length in the file, so the gzip template
sizes it with `E::stream_len(Codec::Deflate)`. To find where the stream ends,
the evaluator decodes it with the output counted and not kept (`inflate_len`).
`stream_len_at` measures within the bytes from the start of the run to the
end of the field around it, which for gzip is the end of the file, and at
most 64 MiB of those bytes. A stream that does not end within them, or whose
output comes to more than 64 MiB (`Limits::measure`), is refused,
`StreamLen` comes to 0, and the template falls back to sizing the run as
everything up to the 8-byte trailer at the end of the file.

Made in one go, with nothing charged to the go's allowance, this walk takes
5 ms native for the Rubin raw, because its 64 MiB of output are long
matches. For 64 MiB of CSV-like text it takes about 500 ms native, and the
page would stop for all of it before the listing showed anything. So the
walk is made a step at a time and charged to the go, like the index pass.
Its result is the same as in one go: the same end, the same refusals, and
the same fallback past 64 MiB.

- **What a walk keeps.** `inflate::Measuring` keeps the bit where the last
  step stopped, the bit where the block it stopped in starts, and how many
  bytes of output the stream has come to. Nothing more is needed, because the
  output is counted and not kept. A step that stopped inside a Huffman-coded
  block reads that block's header and tables again before it goes on.
  `inflate_len` is the same walk made in one step.
- **Where a step stops.** At the first code or block start that is 1 Mibit
  (128 KiB) or more past where the step started (`MEASURE_STEP_BITS`), and
  never inside a stored block. Steps are counted in bits read, not in output,
  because a decoder that only counts the output does about the same work for
  each code it reads, whether the code is a literal or a match that copies
  258 bytes. A code is at least one bit long, so a step of 1 Mibit reads at
  most about a million codes. Measured native: a step takes 3.5 ms on CSV-like
  text, and 8 ms on a stream of literals whose codes are one or two bits long,
  the slowest stream measured. A step over a hostile stream of empty fixed
  blocks took about 180 ms until their codes were built once, and takes 3 to
  4 ms since (see "A step bounded by the bits it reads"). The Rubin raw's run
  is 95 KB of compressed bytes, so it is measured in one step.
- **Charging.** `Evaluator::measure_steps`, which `stream_end` calls, checks
  that the go has allowance left before each step (`Go::charge(0)`, which
  hands back `Busy` when it has none). After the step it takes one element off
  the allowance for each 1,024 bits the step read (`Go::spent`). So a go of
  5,000 elements is about five steps, 17 ms native on CSV-like text. Because a
  step is charged after it is done, for the bits it read, a short member costs
  a go almost nothing. The index pass charges each step in full before it
  starts it; charged that way, each member of a file of a thousand short
  members would cost a full step, 1,024 elements.
- **The bytes a walk reads.** Before each step, `measure_steps` reads more of
  the bytes measured, a window at a time and each window charged to the go,
  until they reach `MEASURE_AHEAD_BYTES` past where the step starts: 128 KiB
  for the step's 1 Mibit, 65,541 bytes for a stored block the step may not
  stop in, and 1 KiB for one more code, about 193 KiB. So a walk reads about
  one step past where its stream ends, or past where it was refused, and a
  short member costs a short read however far the file goes on after it. A
  window whose chunks of the file are not loaded hands back `Pending` with
  them.
- **Between goes.** A go that runs out hands back `Busy`, and the walk goes
  into the `Measures` table in `Spaces`, together with the bytes it has read
  so far. The table is keyed by the space, the bit where the stream starts and
  the number of bits in the stretch the walk measures within. The next go that
  measures the same stream carries on from where the walk stopped, without
  reading those bytes again. There are places for two walks, and each holds
  the bytes it has read, up to 64 MiB. A walk that would start while both
  places are taken waits for one, as an index pass does (see "Waiting for a
  place"). `Measures` is separate from `Unfinished`, so a member's walk and
  another member's index pass never wait for each other.
- **The ends found.** `Measures` also keeps the last 64 results: where each
  stream ended, or why it was refused. This is a guard. If a go ran out after
  a walk had finished but before the field that the walk sizes was placed, the
  next go would ask for the same stream again, and without the result kept it
  would start the walk again from the beginning. For a stream that takes more
  than one go to measure, every go could then run out before the walk
  finished. The gzip template places the field as soon as the walk finishes,
  and its tests pass with the results not kept; a template that puts
  `StreamLen` inside a larger expression might not.
- **Edits.** An edit clears the spaces only when `Spaces::any` says there are
  some. A member is measured before anything opens it, so `Spaces::any`
  counts more than the streams that have been opened: measuring walks and
  the results they found, indexes finished or not, refusals kept, runs being
  gathered and joins under way. Otherwise an edit could leave a result
  measured from the old bytes in place.
- **Progress.** `Evaluator::unpacking` reports a walk in the same way as an
  index pass: the bits of the measured bytes read so far, and the bytes of
  output counted. If a walk and an index pass are both under way, it reports
  the one the current go ran out in. The percentage is of
  the bytes the walk measures, which run from the start of the member to the
  end of the file, at most 64 MiB of them. So in a file of several members,
  each walk ends short of 100% and the next one starts at 0. For a member
  whose output is past 64 MiB, the walk stops at 64 MiB of output, and then
  the index pass starts again from the beginning of the run.
- **The cap.** `Limits::measure` is how much output a walk counts before it
  refuses: 64 MiB. It is a separate limit from `Limits::whole`, so that a test
  can make the runs held whole small and still measure members larger than
  that.

**Why the walk is not also the index pass.** The walk could have been the
index pass itself, so that a run past 64 MiB is decoded once and its index
kept for the open that follows. It is not, for three reasons:

1. The walk does not know where the run ends. It measures within the bytes
   from the start of the run to the end of the file or to 64 MiB of them,
   but an
   index holds exactly the run's bytes and is keyed by where the run starts
   and how long it is. Past
   the cap, the run's length comes from the template's fallback (`.or(...)`),
   which the walk never sees.
2. An index made over the longer bytes would be wrong at its end.
   `inflate::run`, given bytes after the stream, adds an opaque step over
   them, so the trace of the last span would cover the trailer and any members
   after it.
3. Members under the cap would cost more. The index pass keeps a window,
   checkpoints and two checksums, or the whole output and its trace. Every
   member of an ordinary gzip of several members would pay that, for an index
   that is then thrown away.

So the walk decodes the same bits it would decode in one go, in steps, and
the work for the whole open is the same. What the steps change is how long
the longest go takes.

Measured on 2026-11-26 in a release build, native, with a harness outside
`cargo test` that opens a gzip file in goes of 5,000 elements, as the web app
asks, until the first bytes of `decoded` are read. Each file is one member
past 64 MiB, so the measuring walk refuses and the index pass follows. The
CSV-like text and the literals were generated for the measurement: rows of a
date, a name, two numbers and a word, packed by Python's `gzip` at level 6;
and bytes that are nine tenths one letter, packed by `zlib` with Huffman
coding only. The machine was loaded (a load average of 12 to 16 on 12 cores),
so single goes vary by a fifth or more.

| File | Packed | Unpacked | Measuring in one go | Longest go before | Longest go after | Goes before | Goes after |
|---|---|---|---|---|---|---|---|
| The Rubin raw | 94,940 B | 75,677,760 B | 4.6 ms | 21 to 28 ms | 19 to 20 ms | 30 | 30 |
| CSV-like text | 40,927,252 B | 120,062,834 B | 560 to 590 ms | 680 to 820 ms | 107 to 117 ms | 47 | 82 |
| Literals of one or two bits | 11,434,544 B | 80,000,000 B | 535 to 620 ms | 710 to 750 ms | 89 to 111 ms | 31 | 46 |

The whole open took about as long after the change as before: 0.4 s for the
Rubin raw, 2.4 to 2.8 s for the CSV-like text, and 1.9 to 2.4 s for the
literals. After the change, a go that only measures takes 16 to 18 ms on the
CSV-like text and 32 to 40 ms on the literals. At the time of this
measurement, the first go also copied out of the document all the bytes the
walk would measure, before measuring any of them: 41 MB in 56 ms for the
CSV-like text. The walk now reads those bytes a window at a time as it
reaches them (see "The bytes a walk reads" above). The other longest goes
are in the index pass.

### One decoder

`codec.rs` says the traced path is the only path, so the bytes a reader sees
are the bytes the trace describes. The index pass, the span decodes and the
span traces all run `inflate::run`, which differs from a plain inflater in
three ways:

- **Output goes to a sink**, a trait with `literal`, `copy(dist, len)`,
  `stored(bytes)` and `len()`. One sink grows a `Vec`, as for a run held
  whole. The other keeps a sliding window and can capture an output range,
  record checkpoints and stop at a target. The index pass starts with the
  first and switches mode when the output passes 64 MiB.
- **It can start from a checkpoint**: read the block header and tables at
  `block` if there is one, from the checkpoint's copy of them, then move to
  `in_bit` and carry on decoding codes.
- **It reads its codes from a window of the run** (`inflate::Input`), with
  positions still counted from the run's front, and the header copy as a
  second small window. See DESIGN-lazy-compressed-input.md, "The decoders
  read a window".

The trace builder has an "off" switch, so a decode that only wants bytes
records nothing.

### Reading bytes

The output is divided into spans, one per checkpoint, from that checkpoint to
the next. A read of bytes `[a, b)` finds the spans it covers by binary search,
and for each span either takes it from the cache or decodes it from its own
checkpoint and caches it. One decode per span: there is no decoding from an
earlier checkpoint and throwing output away. Spans are at most twice the
spacing plus 64 KiB, so a read costs at most one span decode per span it
touches (about 15 ms native for a 1 MiB span of text). The cache drops the
span read longest ago once it holds more than `SPAN_CACHE_BYTES`. An xz span
is decoded only as far as the read reaches instead: see "Reading an xz span"
under "zstd frames and xz blocks".

Before it decodes a span, the read copies that span's compressed bytes
through the index's `RunInput`, in one read. When chunks of them are not
loaded, the read stops at that span, names all of its missing chunks
(`ReadError::Missing`), and decodes and caches nothing of it. Spans decoded
before it stay in the cache, so the read made again once the chunks are
loaded does not decode them again. A deflate span's compressed bytes are at
most about 8.5 MiB. See DESIGN-lazy-compressed-input.md, "A chunk pushed out
while a synchronous read is under way".

The reads come through `&self` (`Source::read_bytes` and `Evaluator::read_in`
both take `&self`), so the cache sits behind a `Mutex`.

### Traces

The trace of a deflate stream needs no window. A literal step names its byte
and a match step names its length and distance, and neither depends on what
the earlier output was. Only the output bytes do. So the steps of any span can
be found by decoding from that span's checkpoint with the trace on, without
knowing any output before it.

A run opened this way keeps no whole trace. Instead it builds the trace of one
span when asked, and keeps the most recent one:

- The span trace starts at the header of the block the checkpoint is inside
  (or at the checkpoint itself, if it is at a block boundary), so the block's
  header and table steps are in the trace and `inflate::decode_step` can
  rebuild the block's Huffman tables from them. The header and tables are
  read from the checkpoint's copy of them, so the trace reads the run only
  from the checkpoint on.
- If the checkpoint is inside a block, one `Opaque` step covers the codes
  between the end of the tables and the checkpoint. It produces no output in
  this span; those bytes belong to the span before. The steps still tile the
  span's own bits and bytes, so `check_tiles` holds for it.
- Span 0 of a gzip or zlib run starts with the wrapper header step, and the
  last span ends with the padding and the wrapper trailer step, as
  `inflate::zlib` and `inflate::gzip` push them for a run held whole.
- Positions in a span trace are the same as in a whole trace: bits from the
  start of the run and bytes from the start of the output. Both fit the `u32`
  a `RawStep` keeps, since the run is at most 512 MiB (2^32 bits,
  `Limits::input`) and the output at most 1 GiB. So `map_out`, `map_in` and
  `decode_step` need no shifting. The 512 MiB limit on a run read through a
  handle is there so that its bit positions fit that `u32`. `Trace` keeps the
  bit and byte its first step starts at (zero for a whole trace), and
  `check_tiles` counts from there.

A bit of the compressed run belongs to the span whose checkpoint bit range
holds it, `[checkpoint k, checkpoint k+1)`, and never to a span whose trace
only reaches back over it to read a block header. So a bit inside the header
or the gap step of span k's first block is answered by span k-1, whose trace
names it as a code.

Checkpoint 0 is the start of the deflate data, with an empty window, and is
never dropped when the checkpoints are thinned. Checkpoints are taken only at
the start of a block and between two codes of a Huffman-coded block, never
inside a block header, its tables or a stored block.

### Blocks

The `blocks` child that `traced.rs` lays over a compressed run is built from
a whole trace for a run held whole. A run read a piece at a time has none, so
its blocks come from the index pass instead.

- **Where each block starts.** The decoder calls `Sink::boundary` with
  `block` set to `None` at the start of every block, and the index pass
  records two numbers there: the bit position in the compressed run, and how
  many bytes of output come before the block. Where the last block ends
  comes back from `inflate::run` as `Ended::Stream(bit)`. A block ends where
  the next one starts. The decoder also passes each block's BFINAL and BTYPE
  to the sink (`Sink::head`), and the pass keeps them in the top three bits
  of the word that holds the block's bit position (`inflate::BlockStart`), so
  a block start is 16 bytes. `Seekable::block(i)` reads the run only when the
  start of block `i` or of block `i + 1` was dropped by thinning.
- **Bounded like checkpoints.** A hostile run can start a block every 10 bits,
  which is tens of millions of blocks in 64 MiB of compressed run and
  hundreds of millions in 512 MiB. Past `MAX_BLOCK_MARKS` the index keeps
  the start of every second block, then of every fourth, every eighth and so
  on, each time the list fills. A dropped start is found again by decoding
  from the kept start before it with the output counted and not kept, and the
  lists found this way are cached (16 of them). The total count is kept
  exactly, so the listing shows every block either way. From a source that
  keeps only part of the file, a stretch between two kept starts longer than
  `Limits::unit_input` (16 MiB) is refused as `TooMuchAtOnce` when it is
  decoded, since it is read whole.
- **Where the second pass starts.** When the first pass's output passes
  64 MiB (see "The index pass: one walk"), the second pass starts from the
  last checkpoint and records again every block that starts at or after it.
  Each checkpoint keeps how many blocks start before it, so the record is cut
  back to that count first. A pass done in steps records each block once too;
  see "The pass in steps".
- **A block's rows.** Opening a block traces that block alone: the decoder
  starts at the block's header (as a span starting at a block boundary does),
  with the output counted and not kept, and stops at the next block's start.
  `SPAN_TRACE_STEPS` sets two limits here: a block with more codes than that
  has its codes as one opaque step, and the cached block traces of one run
  hold at most that many steps in total. When a block's codes pass the first
  limit, the trace turns coarse and the decoder stops there, and the opaque
  step is closed at the block's end, which the index knows
  (`TraceBuilder::stopping_when_coarse`, `Ended::Coarse`). So a block trace
  reads the run only up to that point. The trace has the block as its only
  entry, so `BlockView` and the rest of `traced.rs` read it as they read
  block `i` of a whole trace.
- **One view in the evaluator.** `traced::Blocks` is either a whole trace or
  an index. The `blocks` row, each block's row, and which block holds a bit all
  come from the block starts, with no block traced (a dropped start is still
  found by decoding, as above). The rows inside a block come from
  `Blocks::trace_of(i)`, which returns the trace holding block `i`'s steps and
  the block's number in that trace: the whole trace and `i`, or the block's
  own trace and 0.
- **The inspector's code panel.** `Space::trace_at_bit` gives the trace of the
  block holding the bit, and the span's trace only for a bit outside every
  block (the wrapper, the padding). `Seekable::map_in`, which says which bytes
  a code wrote, asks the same trace. The listing lays a block's codes out from
  the same trace, so the panel can decode every code the listing shows. A
  span's trace could disagree with the listing: a span of 1 MiB of literals
  has more than `SPAN_TRACE_STEPS` steps, so its trace has one opaque step
  where the listing shows codes.

The gzipped Rubin raw, the test file above, has 10 blocks in its one member,
about 7.5 MB of output each.

## Where it plugs in

Nothing in the IR (`Ty`, `Expr`, templates) changes. The output side is in
the decoded-space layer, plus `ChunkStore::behind` in `source.rs`. How a pass
and a span read get their compressed input changed `Source`, `ChunkStore`
and `Document` as well: see DESIGN-lazy-compressed-input.md, "Where a
`RunInput` comes from".

**`codec::lazy`.** `Seekable`: the index, the span cache, the span trace
cache, `read(at, out)`, `missing(at, len)`, `map_out(byte)`, `map_in(bit)`,
`span_trace(k)`, `trace_at_bit(bit)`, `crc32()`, `adler32()`, and for deflate
`block(i)` and `block_trace(i)`. A read answers `ReadError::Missing` with the
chunks of the file it needs, or `ReadError::Refused`. Each `Seekable` reads
its run through a `RunInput` (`Seekable::input`): a handle onto the source the
run is in, such as the file's chunk store, when that source can give one, and
otherwise a copy of the run. A run in a stitched space, in an edited stretch
or in a `MemSource` is copied. `decode_traced` keeps its signature.
`decode_or_index(codec, packed)` returns either the whole bytes and trace or a
`Seekable`, for the three deflate codecs, zstd and xz, and calls
`decode_traced` for the rest. `Indexing` is the pass in steps, for deflate,
zstd and xz.

**`Spaces` (`eval/space.rs`).** A third backing, `Backing::Lazy(Arc<Seekable>)`.
`Spaces::set_lazy_limits` (tests only) makes the limits small, as
`set_whole_cap` does for joined streams, and `Evaluator::set_stream_limits`
sets both, for the wasm crate's tests. `len_bits` answers from the index;
`buf` and `trace` answer `None`; `lazy(space)` hands out the `Seekable`.
`Evaluator::read_in` has an arm for it beside the stitched one
(`read_indexed`). Indexes are also kept in `Indexes`, keyed by where the run
is (space, bit offset, bits, codec), so the member's `Decoded` node, the
stitched part over the same run and the tab all share one index. `Indexes`
weighs each index by what it holds for as long as it is kept: its
checkpoints' windows, its header copies, its block starts and an xz
stream's list of blocks (`Seekable::kept_bytes`), and the copy of its run
when no handle reaches the run (`RunInput::owned_bytes`). The weights come
to at most `INDEXED_RUN_BYTES` (64 MiB), and the index used longest ago goes
first. The caches inside each index are not weighed. A run whose pass
refused it is kept beside the indexes, by the same key, with its refusal
(`refused_runs`): `Evaluator::index_run` hands that refusal back to the next
node or read over the run without another pass, and `Spaces::forget` drops
it with the indexes.

**`open_space_at`.** For a codec that can be indexed, calls
`Evaluator::index_run`, which makes the pass of `decode_or_index` a step at
a time (see "The pass in steps"); a `Seekable` becomes `Backing::Lazy`
instead of a refusal. The pass refuses a run too long for the way it reads
it, before reading any of it: past `Limits::input` (512 MiB) for a pass that
reads through a handle, and past `Limits::copy_input` (64 MiB) for one that
reads a copy. `open_space_at` itself refuses past 64 MiB only a run of a
codec that cannot be indexed, as `TooLargeCopy`.

**Stitched streams (`eval/stitch.rs`).** `read_stitched`, for a packed part
whose unpacked length is past 64 MiB, as the file gives it or as the walk that
finds the parts (`open_stitched_at`) measured it, reads through the `Seekable`
in `Indexes` for that part's run (`index_part`, which starts the pass with
`known_large`). Any other packed part goes to `unpacked_part`, which decodes
it whole. A part of a codec that can be indexed whose run needs more of the
file at once than one gather window is unpacked through `index_run` too, so
that it is read a window at a time and decoded a step at a time, each charged
to the go; it comes back held whole.

`join_whole`, for a stream past `whole_cap` (64 MiB), makes a joined document
(`Joined`) that is read a part at a time. A walk over the parts, kept from one
go to the next, decodes once each packed part that will later be decoded
whole, to check that it unpacks to the length its run claimed. Once the stream
is open, a packed part that unpacks to 64 MiB or less is decoded whole on
first read and kept in a cache of decoded parts, which holds 16 MiB
(`STITCH_CACHE_BYTES`) and at most one part larger than that; a part that
unpacks to more is read through its `Seekable`. Each stored part's bytes, and
the run of each packed part decoded whole, are read through a handle onto the
source they are in, when one reaches them (`Evaluator::joined_input`). What no
handle reaches is copied and kept: a part in a stitched space, in an edited
stretch or in a `MemSource`, and a packed run decoded whole that is longer
than a gather window or an eighth of what the store keeps. Those copies are
held to `whole_cap` between them. Past that the joined stream is refused as a
document, as `TooLargeCopy`, and still reads a part at a time in the listing.
A BAM read from the file's chunk store, or a single-member gzip of any output
size up to 1 GiB, is not refused that way. See
DESIGN-lazy-compressed-input.md, "Built (part 10)".

When the index pass of a packed part is refused as `TooLargeLazy` (the part
unpacks to more than 1 GiB), `TooLargeInput` (its run is past 512 MiB),
`TooLargeUnit` (one zstd frame or xz block of it unpacks to more than
64 MiB) or `TooMuchAtOnce`, the joined stream is refused with that same
refusal. Any other refusal from a part's index pass, `TooManyXzBlocks`
included, refuses the stream as `Failed` (see "Still to do"). A read of the
part in the listing says why after the part's name, in the words of
`Refusal::after_name`: `blocks[0].compressed is too large to unpack (over
1 GiB unpacked)`, or `member.run has a zstd frame that is too large to
unpack (over 64 MiB unpacked)`. A read of a part that will not decode says
`could not be unpacked`. A part of a codec with no index (bzip2, LZMA and
the rest) is decoded whole: a read of one whose run is past 64 MiB says `is
too large to unpack (over 64 MiB packed)`, and of one whose output is past
64 MiB, `is too large to unpack (over 64 MiB unpacked)`. FITS tiles and
Parquet pages use the words of `Refusal::message` too.

A packed part that will not decode, or that decodes to another length than
the file says, is kept with the reason in its stitched space
(`Stitch::troubles`), whether it was decoded whole or through an index pass.
A read that reaches it again fails with the same message and decodes
nothing, so a listing scrolling over a damaged BGZF block decodes the block
once. That includes a zstd or xz part the file says is past 64 MiB which
decodes whole: the run itself decodes, so it is not a refusal in
`refused_runs`, and the part's own `Decoded` node still opens it.
`Spaces::forget` drops the stitched space, and the kept reasons with it.

**`Space` (the document of its own).** Its document is
`Document<SpaceSource>`, where `SpaceSource` is an enum of
`Whole(Arc<Vec<u8>>)`, `Lazy(Arc<Seekable>)` and `Joined(Arc<Joined>)`, all
implementing `Source`. A read of `Lazy` or `Joined` can find chunks of the
file missing under the stream; it reads them as zeroes and reports them,
and they are always chunks of the file, whichever document asks.
`Space::source()` gives it, and `Space::held()` gives the buffer of a space
held whole. `Space::bytes()` is for tests and callers that know the space is
held whole, and panics otherwise; no code outside tests calls it.
`template_for` sniffs the first 36 KiB through `read`, or no more than the
stream's `resident_limit`, and answers `Pending` when the read finds chunks
missing rather than sniff zeroes. `map_out` and `map_in` go to the
`Seekable` or the joined parts when there is no whole trace, and
`Space::trace_at_bit(bit)` gives the wasm crate's `decode_step` the trace
that holds a bit: the whole trace, or the trace of the block or the span
that holds it (see "Blocks"). All three answer the missing chunks as an
error, and the wasm crate replies "pending". A joined document's codec is
`Stored`, so `decode_step` answers nothing for one.

**`ChunkStore` (`source.rs`).** `ChunkStore::behind` makes a store with a
source behind it. It keeps no chunks of its own and reads every byte from
that source, and `has` answers true. The wasm crate's `Sheet::from_space`
builds every space's store this way, so a space is never copied into the
store whole. A read of
such a store reports the chunks of the file that its source is missing, and
the store passes on its source's `resident_limit`. On a tab, `doc.ts`'s
`ensureRange` therefore asks the read which chunks of the file it is missing
(`ensureTabRange`), since `has_chunk` of the tab always answers true.

**Checks over unpacked bytes (`eval/check.rs`).** `run_check` reads the
whole output of a `Decoded` node. For a lazy one it uses the CRC-32 or
Adler-32 the index pass kept when the check is one of those (gzip's `crc32`
and zlib's `adler32` are), and otherwise refuses as too large. A check over
one member of a lazy run reads that member's bytes through the index when
the member is an xz block (see "zstd frames and xz blocks"), and is refused
as unmapped otherwise.

**Other `spaces.buf` callers.** `buf` answers only for a space held whole.
`picklecells.rs` reads a cell from the buffer when there is one and through
`read_in` otherwise, and `pickletorch.rs` reads a PyTorch storage through
`read_in`, so both read a space read through an index too. `jpegscan.rs`
takes JPEG coefficients from the buffer only; a JPEG scan has no index, so
its space is always held whole. `stitch.rs`'s `unpacked_part` uses the
buffer of a part's own `Decoded` node when that node is open and held
whole, and otherwise unpacks the part.

**The web interface.** Whether a joined stream opens as a document depends
on what its parts are and where they are, which only the core knows, so
every joined stream is offered Open joined stream, and a refusal is said in
the status bar when the button is pressed. `JOINED.refused` turns the tag
into words: `tooLarge` for `too-large-copy`, `partTooLarge` for
`too-large-lazy`, `partTooLargeInput` for `too-large-input`,
`partZstdFrameTooLarge` and `partXzBlockTooLarge` for
`too-large-zstd-frame` and `too-large-xz-block`, `partXzTooManyBlocks` for
`too-many-xz-blocks` (which this path does not reach yet: see "Still to
do"), `partTooMuchAtOnce` for `too-much-at-once`, and
`failed` for the rest. `DECODED_REFUSED` has words for every tag, and the
report's finding for a stream left as bytes (`RV.refused` in
`report/text.ts`) says the listing's words with a capital for each size
refusal. The tags come from one list in `codec.rs`, and
`web/src/refusaltags.ts` is generated from it: see
DESIGN-lazy-compressed-input.md, "Refusal tags".

## The browser

The pass is charged to the same allowance as the rest of a reading, so
whatever reaches the stream first carries it on, a go at a time: the listing
reading the stream's first bytes, or Open unpacked. `open_in` in the wasm
crate runs with the usual `WORK_SLICE`. `open_space` answers "working" when
a go runs out before the pass is done, and "pending" when the call waits for
chunks of the file, as a step of the pass does when its window of the run is
not loaded. `Editor::unpacking(space)` reports the progress of the pass
that the last call ran out of allowance in, or waited for chunks in. It
looks in the reading that the tab's next call will use, which
`Editor::working_reading` chooses. That is the tab's own reading, or for a
stream read where it was declared, the reading of the tab it was opened
from (its home). While the tab's stream, or a stream it was opened from, is
being opened again after an edit, `ensure_open` opens those first, from the
file down, each in its home's reading, so the pass is in the home of the
first one still to open, and that is the reading used.

The wasm crate never runs a pass in the reading the core keeps inside a
recognised `Space` (`Space::reading()`): each tab over a recognised stream
has an evaluator of its own in its `Sheet`, and `open_in` always asks a
sheet's evaluator for its space 0. The core's own way into those readings,
`Evaluator::open_space` given a recognised space and `tab_node`, gives the
inner evaluator no allowance (`set_slice` is never called on it), so a pass
there runs in one go, apart from waits for chunks of the file, and nothing
reports its progress.

On the web side, `Doc.openSpace` comes back as `{working, unpacking}` after
a "working" reply, and as `{pending, unpacking}` after a "pending" one, with
the chunks of the file it waits on. Open unpacked (`openUnpacked` in
`main.ts`) hands both to `waitToOpen` (`openwait.ts`), which asks again
after the page has drawn or once `Doc.fileChunks` has the chunks, and opens
the tab at the end, all from one press. Meanwhile the status bar says
`UNPACKED.loading` ("Loading {field} from the file…") until a pass has said
how far it has got, and `UNPACKED.unpacking` with the percentage after that,
on "pending" replies as well as "working" ones. A press while a wait is
under way puts a new wait in its place.

`waitToOpen` stops by the rule in `ChunkWait` (`chunkwait.ts`). It gives
up, and says `UNPACKED.tooMuchAtOnce`, when the core asks again for a chunk
this wait loaded since the core last got further: the chunk was pushed out
of the store before the core could use it, and each load would push out
different ones. A "pending" reply whose progress has moved counts as the
core getting further, as a pass reading its run a window at a time waits
for each next window with no "working" reply in between. A "working" reply
lets the chunks loaded so far be asked for again. The backstop is
`roundLimit` of the file: as many rounds since the core last got further as
the file has chunks, and never fewer than `MAX_ROUNDS` (512). It also gives
up when a chunk cannot be read.

`fileChunks` waits on the reads already started (`Doc.inflight` keeps each
chunk's read) and tests the file's store (`has_chunk(0, c)`), since a
pending reply's chunks are the file's whichever tab asked. `fetchChunks`
reads each stretch of neighbouring chunks a reply names with one read of
the file (`fetchStretch`) and one redraw, at most 256 chunks a stretch.

Code that needs a long range of bytes at once, of a tab or of the file, reads
it with `Doc.readLoaded`: the hex view's copy, the image card
(`contentcard.ts`), the stride view's sample and `openplan.ts`. `readLoaded`
reads the range 4 MiB at a time (`readInPieces`, `PIECE_BYTES`), loads the
chunks of the file each piece names, and reads that piece again until it is
complete. A piece of 4 MiB of output touches at most 16 MiB of deflate spans
(two spans of at most 8 MiB each, or shorter spans that add up to less than
three pieces), which the span cache holds at once. So a range whose compressed
input is longer than the chunk store (32 MiB in the browser) can still be
read. `ensureRange` on a tab (`ensureTabRange`) reads the range and loads the
chunks it names in the same way, in one piece. See
DESIGN-lazy-compressed-input.md, "Built (part 7c, web)".

The listing's placeholder row for fields it cannot show yet says how far the
unpacking of the stream they wait on has got (`pendingText` in
`pendingtext.ts`), and is updated on each redraw, while a pass waits for
chunks as well as between goes. While the core works on them with no
progress to give, it says "Reading these fields…" (see "Which pass a call
waits on"), and while bytes of the file load with no pass to report,
"Loading bytes needed to map these fields…". The report's streams section
waits for a pass the same way it waits for bytes of the file.

### Which pass a call waits on

The listing and Open unpacked work in the same reading, so while the listing
carries on one stream and Open unpacked another, a report of the pass stepped
most recently would show each the other's progress. The listing asks for
progress when it draws, after all the calls its walk made and any calls Open
unpacked made between frames. And a go that runs out of allowance before it
reaches its own stream's pass may have spent it on another stream's pass.

So the go keeps the pass it ran out in (`Go::ran_out_in`). `index_steps`,
`measure_steps` and the walk that joins a stitched stream (`join_steps`)
note it, with how far the pass has got, as they put the pass back and hand
back `Busy`. `index_steps` and `join_steps` also note it when a step waits
for chunks of the file and hands back `Pending`; `measure_steps` notes it
only on `Busy`. `begin_slice` clears it, so `Evaluator::unpacking` says
nothing until the go runs out or waits in a pass, and a go that runs out
anywhere else waits on no pass. Only the first pass a go runs out in is
kept. A pass reached after the allowance has gone is put back without having
moved, and it is not what the go spent its allowance on.

So `unpacking` asked right after a call is that call's stream. `Doc.openSpace`
asks right after `open_space`, after a "working" reply and after a "pending"
one. The listing cannot, since it draws after its walk, so
`template_children` puts it on its reply as `unpacking`: on a "working"
reply, and on a "pending" reply whose wait is a pass's (see "Children that
are ready"). The walk keeps it on the pending item it puts where those
children go, and each placeholder then shows the stream its own fields wait
on. After a "working" reply with no `unpacking`, a placeholder keeps the
percentage it already shows (`keepsProgress` in `listingdraw.ts`), and Open
unpacked keeps the last progress it was told, rather than going back to 0% for
one go. A placeholder that shows no percentage yet says "Reading these
fields…" after such a reply (`pendingText` in `pendingtext.ts`). It cannot
say more, because a reply with no progress can come from a go that ran out
before it reached the stream's pass, from a stream waiting for a place (see
"Waiting for a place"), or from any other reading that takes more than one
go. "Loading bytes needed to map these fields…" is said for a "pending"
reply that names no pass: bytes of the file loading before a pass has
started, or for anything other than a pass.

Each entry in `Unfinished` has the stamp it was put there with. It is used
only to choose which finished run to drop when both places hold one (see
"Waiting for a place"). The walks in `Measures` have none.

### Waiting for a place

Passes that have not finished are kept in `Unfinished`, which has places for
two (`UNFINISHED_PASSES`), because a pass in its traced phase holds up to
64 MiB of output and its trace. A pass past the cap that reads its run
through a handle holds less than 10 MiB, but the two kinds cannot be told
apart before a pass starts: a gzip pass learns from its trailer at its first
step whether the run is large. See DESIGN-lazy-compressed-input.md, "Built
(part 7b)".

A pass that has started is never pushed out to make room for another. A
table that pushed out the pass stepped least recently would never finish
three large streams stepped in turn, as in a listing of three ZIP entries
each past 64 MiB unpacked: each go would start or carry on one pass and push
out another, which would start again from the front on its next go. A test
showed it when the table did that: three ZIP entries of about 2.2 MB
unpacked each, with the cap lowered to 64 KiB for the test, stayed at one
go's progress for 300 rounds. Three walks measuring deflate streams in
`Measures` did the same.

A call whose pass would start while both places are taken waits for a place,
and does work meanwhile (`Evaluator::wait_for_a_place`):

- With its own go's allowance, the call carries on the pass in `Unfinished`
  with the fewest compressed bits left to read, since that pass frees a place
  soonest. A place can also hold a run being read into it a window at a time
  for a pass that reads a copy (`Held::Gathering`), which is carried on the
  same way. A go that runs out in that pass hands back `Busy` and notes that
  its call waits with no progress of its own to give (`Go::ran_out_waiting`),
  so `unpacking` says nothing, rather than the other stream's progress. A
  step of that pass that waits for chunks of the file hands back `Pending`
  with them, and the pass goes back in its place.
- A pass finished this way leaves its result where the call over its own run
  will find it. An index goes to `Indexes` and a refusal to `refused_runs`,
  both kept by the run, and the pass's place comes free. Output of no more
  than the cap, held whole, stays in the pass's place (`Held::Whole`) until a
  call over the same run takes it. The call that finished the pass cannot hand
  it to the `Decoded` node, which keeps a run held whole by the node's path,
  because it does not know that path. Output held whole counts as one of the
  two places, since it is as large as a pass in its traced phase.
- When both places hold output finished this way and not yet taken, the one
  put there first is dropped, since nothing would free a place otherwise. Its
  stream is unpacked again if it is asked for.
- A go with an unlimited allowance (`set_slice(None)`) finishes its pass
  before it hands back, so it never puts one back and needs no place. Its pass
  runs beside the two in `Unfinished`. The exception is a pass that reads its
  run through a handle onto a source that keeps only part of the file: a step
  of it can wait for chunks, so it takes a place in any go. A go that has
  spent all of its allowance starts no pass, since the pass would take a
  place having done nothing.
- `put` never pushes a pass out. It checks that a place is free in a debug
  build (`debug_assert!`), so that a test fails if that ever breaks, without
  stopping the page in a release build.

Measuring walks wait the same way in `Measures`, and the end a waiting call
finds for another call's stream is kept in `Measures` with the other ends
found. No template has two walks under way at once. Only the gzip template uses
`StreamLen` (the text form of a template prints it but cannot read it back),
and a gzip member's start is known only once the member before it has been
measured, which the gzip tests check. The waiting is there so that a template
measuring streams read independently of each other would not bring the
problem back.

**Why waiting, and not more places.** Two other ways were considered.

1. Keep a pass that is past its traced phase outside the two places. Such a
   pass holds its window of output, its checkpoints (up to 8 MiB of windows),
   its block starts (up to 1 MiB) and its window of the run, under 10 MiB in
   all, or its copy of the run (up to 64 MiB) when it reads one. So it is
   smaller than a traced pass. On its own this does not help. A ZIP entry's
   run is opened with `known_large` false, so its pass starts traced, and raw
   deflate has no trailer to say the run is large. So all three passes start
   in their traced phase at once, and push each other out before any of them
   passes the cap. Waiting is needed for the traced phase anyway, and with it,
   more places would not finish the streams any sooner, since the work is the
   same and each go's allowance is the limit. With waiting, the first two
   streams finish first and the third after them, at about the time all three
   would have finished together.
2. Answer a waiting call with `Busy` at once, without carrying anything on,
   and once it has been turned away some number of times, let it push out a
   pass that no call stepped in that time. A call cannot tell a pass nobody
   wants from one asked for less often than the waiting call is. Open unpacked
   asks again on `setTimeout(0)`, several times a frame when each go comes back
   at once, and the listing asks once a frame, so a turned-away Open unpacked
   would push out passes the listing is still carrying on. Carrying a pass on
   needs no such count: every ask moves a held pass forward, however often the
   asks come.

The cost: a pass nobody asks for any more, such as one whose Open unpacked was
replaced by a press on another stream, keeps its place until calls waiting
behind it have finished it, and finishing it can mean unpacking up to 1 GiB.
Meanwhile those calls show no progress of their own. See "Still to do".

### Children that are ready

A slice of children answered whole, `Busy` or `Pending` as soon as one child
is, would draw one placeholder for the whole structure. While a gzip member's
stream is unpacked, the member's ten header fields would wait with it.

`Evaluator::children_ready` gives the children before the first one that
waits, and what the rest wait on. A child that answers `Busy` ends the slice,
since the go has no allowance left to read the others. A child that answers
`Pending` does not: the children after it are still read for the chunks they
need, so that children placed by offsets are one wait and not one each.
`Evaluator::children` is the same answered whole.

`template_children` puts the ready children on its "working" reply as
`ready`, with the chunks their previews want as `wanted`. They are never sent
under "ok", because a short array there reads as a page of a list: `drawn`
would add an edge counting the rest, and the other callers of
`templateChildren` would take it as the whole answer.

A "pending" reply, which waits on bytes of the file, carries `ready` and
`unpacking` only when the wait is a pass's: a pass that reads its run a
window at a time and waits for the next window's chunks
(`Evaluator::unpacking` names a pass, see `slice_reply` in the wasm crate's
`dto/reply.rs`). That wait comes between goes as running out of allowance
does, so it is answered the same way, and the rows above the stream and the
progress stay drawn while the chunks load. Any other "pending" reply carries
neither, and the listing draws one placeholder for the structure: its walk
over a file still loading was tuned to stop at such a structure, and reading
on into the children it has would ask for more chunks at once (see
`CHUNK_BUDGET`). The web's `TemplateReply` carries `ready` as children
whatever its type, so that a reply that is not "ok" can still be handed on
as the reply to another question.

Only the listing's walk uses it (`Walk.kids` in `flatten.ts`, which marks the
slice with `waitsTo`, for a "working" reply and for a "pending" one that
carries `ready`). `drawn` puts the pending item after the ready rows, over the
rest of the structure, and draws no edge counting the children still to come.
`sections` does the same for the root. Neither draws a gap anywhere in a slice
that waits: children placed by offsets need not be in the file in the order
they are read, so a child still to come may land between two that are drawn,
and calling that stretch bytes nothing describes would be false. A format card
or a record table is drawn from the whole structure, so one whose children are
still to come is a placeholder until they are all in. A slice with none ready
is a placeholder for the whole structure.

While the member's stream is measured, this does not help: the listing has
no rows at all, and showing any needs more than the children that are ready
(see "Still to do").

Measured on 2026-10-26 in Chromium, with a generated gzip of 500,007,092
bytes of CSV-like text packed to 20,306,027 (`make_gz.py` in that session's
scratchpad: 16 KB blocks of rows, a sixth of them new):

| | Before | After |
|---|---|---|
| Longest task while opening and unpacking | 2,928 ms | 122 ms |
| Tasks over 50 ms | 4, one of them the 2,928 ms | 11, each 50 to 122 ms |
| Whole pass | 2.9 s, in that one task | 3.0 s, from the drop to the tab |
| One go of 5,000 elements | | 2.5 MiB, about 18 ms (1.25 MiB took 9 ms, trace on or off) |

The tasks over 50 ms are frames in which the listing and the Open unpacked
loop both carried the pass on, with the drawing around them.

In the browser the file's chunk store holds 512 chunks of 64 KiB, 32 MiB
(`CHUNK_CAPACITY` in `doc.ts`). A pass reads its compressed run through a
handle onto that store a window at a time, and each span read copies only
its own span's compressed bytes, so a run of the file of up to 512 MiB
compressed opens there when no edit touches it. How that works is in
DESIGN-lazy-compressed-input.md. Open unpacked says `UNPACKED.tooMuchAtOnce`
only when its wait asks again for a chunk it loaded with no progress in
between (see "The browser").

## zstd frames and xz blocks

A zstd frame never copies from an earlier frame, and an xz block never copies
from an earlier block. So the start of a frame or of an xz block is a
checkpoint with no window: a decoder starting there needs only that position
in the compressed run. `Seekable` keeps one list of checkpoints for all three
codecs. Each checkpoint has the bit offset in the compressed run, the byte
offset in the output, and what a decoder needs to start there: for deflate,
where the header of the block it is inside starts and a copy of that header,
the 32 KiB window, and how many deflate blocks start before it; for zstd,
the byte where the span's first frame starts, after any skippable frames in
front of it; for xz, the number of the xz block.
`Seekable` also keeps the same span cache (decoded spans) and span trace cache
(the trace of the span asked about most recently) for all three, except that
an xz run keeps its spans part decoded in a cache of their own: see "Reading
an xz span". Each codec has its own decoder for spans.

Spans are built from these checkpoints with the same rules as deflate's. A
frame start or an xz block start becomes a checkpoint when the output since
the previous checkpoint is at least `SPACING_START` bytes (1 MiB), or the
compressed input since it is at least `IN_SPACING_START` (1 MiB). A frame or
block whose own compressed size is the input spacing or more is a span of
its own (`Checkpoints::take_if_long`). When the list grows past
`MAX_CHECKPOINTS` (256), every second checkpoint is dropped and both
spacings double. So one span can hold several small frames or blocks, and
the number of spans stays at most 256 however small the frames are.

A span is decoded from its start, so a frame or an xz block whose output is
larger than `Limits::span` is refused with `Refusal::TooLargeUnit`. This rule
is what keeps refusing a file whose only frame or only xz block unpacks to
more than 64 MiB. The refusal says which of the two it is,
`SpanUnit::ZstdFrame` or `SpanUnit::XzBlock`, with the tags
`too-large-zstd-frame` and `too-large-xz-block`, and the listing says "a zstd
frame is too large to unpack (over 64 MiB unpacked)" or "an xz block is too
large to unpack (over 64 MiB unpacked)". Only the index pass gives this
refusal: a frame decoded as part of a run held whole is refused as
`TooLargeOutput`, since there the whole run is past the cap too. A zstd
frame whose header asks for a window over 128 MiB is refused as
`TooLargeWindow` (see "The zstd decoder"). A decoder can start inside a
frame only if the checkpoint keeps the frame's whole window (up to 128 MiB
with `zstd --long`), and inside an xz block only if it keeps the LZMA
dictionary (up to 64 MiB).

When the run is read from a source that keeps only part of the file, as the
browser's chunk store does, one frame or block of more than
`Limits::unit_input` (16 MiB) compressed is refused as `TooMuchAtOnce`,
since a read of its span copies its compressed bytes whole. See
DESIGN-lazy-compressed-input.md, "Checkpoints by compressed distance".

**zstd: one pass.** A zstd frame records its output size only in the
optional content size field, and `pzstd` does not write that field, so the
index pass decodes every frame. The pass uses `frames::ZstdWalk`, the same
walk that decodes a run whole, in the same way that deflate's index pass
uses the same inflater. Until the output passes `Limits::whole`, the pass
keeps the output and the trace, and a run that ends before then is returned
whole. Once the output passes `Limits::whole`, the pass starts again at the
start of the frame it had reached, with the trace off, and counts the output
without keeping it. At most one frame is decoded twice. If a frame's output
passes `Limits::span`, the pass stops there and the run is refused. Each
skippable frame that `pzstd` writes before a frame is in the same span as the
frame that follows it.

**zstd: the pass in steps.** `lazy::Indexing` makes zstd's pass a step at a
time too, and `Evaluator::index_run` charges each step to the go as it does
deflate's. So the browser keeps drawing while the pass runs, and the listing
and the status bar say how far it has got, with the strings deflate's pass
uses.

A step stops between two zstd blocks, inside a frame as well as between
frames, and between two skippable frames (see "A step bounded by the bits it
reads"). `ruzstd`'s frame decoder holds everything a frame needs between two
blocks, its window included, and is handed the input again for each block. So
the walk keeps the frame's decoder from one step to the next, with how far it
has read and how much of the frame's output the decoder has let go of, and the
next step decodes the next block, as the pass in one go would. The pass comes
to the same checkpoints and spans as the pass in one go, and a run under the
cap comes back with the same bytes and the same trace, step for step. A step
that takes the output past `Limits::whole` ends the traced part, and the next
step starts again at the start of that frame, as the pass in one go does.
`Indexing::known_large` starts a zstd pass past the cap, with the trace off,
as it does deflate's.

**How long a step is.** A step stops at the first block boundary after it has
decoded its share, 256 KiB in the evaluator, so it decodes at most one block
(128 KiB) more than that. It also stops once it has read 256 KiB of the run
(2 Mibit), at most one block past that. Once a frame's decoder has let go of any
output, it holds exactly one window, so the output so far is known. Before
that it is not, since `ruzstd` does not say how much it holds, and each block
counts as a full 128 KiB. If the blocks before a frame's window fills are
smaller than that, a step stops early, and the next step decodes the
difference as well as its own share. That is at most one window more, and
never more than the frame's own output: 512 KiB for `pzstd -1`, 2 MiB for
`pzstd -3`, and at most 64 MiB, since a frame larger than `Limits::span` is
refused. On the generated `pzstd -1` file measured below, no step took longer
than 14 ms native.

An unfinished pass holds what the pass in one go holds at the same point: in
the traced part up to `Limits::whole` of output and its trace, and the decoder
of the frame it is in, with up to one window of that frame's output. A pass
that reads its run through a handle also holds its window of the run, at most
`Limits::step_input` (256 KiB) plus `ZSTD_MARGIN` (128 KiB + 25 bytes), and
one that reads a copy holds the copy. `Unfinished` keeps two passes, as it
does for deflate.

xz's pass decodes nothing: see "xz: laid out from its index". Neither zstd
nor xz gets the `blocks` child that "Blocks" describes, which is for
deflate: `Seekable::blocks` is 0 for both.

**xz: laid out from its index.** The index at the end of an xz stream lists
each block's packed and unpacked size, so the pass finds every block without
decoding any of them. The xz pass is an `Indexing` like the others (`XzPass`
in `codec/lazy/xz.rs`), stepped and charged by `index_run` and kept in
`Unfinished` between goes. Its first step reads the stream's header and
footer, the next ones its index, and then up to 64 block headers a step,
where the index puts them. Its progress is where the furthest block header
it has read starts, or how much of the stream it has copied, and it unpacks
nothing until it ends. How the pass reads these through the run's handle is
in DESIGN-lazy-compressed-input.md, "Built (part 8)".

- A stream whose index gives a total unpacked size of at most
  `Limits::whole` is decoded whole. When the pass reads such a stream
  through a handle, it first copies the stream a piece at a time.
- A stream whose own index lists more than `Limits::block_marks` (65,536)
  blocks is refused as `TooManyXzBlocks`, whatever it unpacks to, since the
  `Seekable` keeps an entry for every xz block.
- A stream whose blocks use a filter that our own LZMA2 decoder does not
  run (delta, or a branch filter) is refused as `TooLargeOutput` once it is
  past the cap, because `lzma-rs`, the decoder that runs those filters, reads
  only whole streams.
- A stream that `layout` cannot read (several streams joined end to end,
  stream padding, or an index that does not match the blocks) goes to the
  whole decoder, which decodes it or refuses it. A copy is held to 64 MiB
  (`Limits::copy_input`), so past that such a stream is refused instead of
  copied: as `TooLargeCopy` when it is joined or padded, and as `Failed`
  when it is damaged.

Because nothing is decoded when the stream is indexed, a block whose
unpacked size differs from the index is found only when a read first
reaches the block, and that read is refused.

**Reading an xz span.** A read of an xz span decodes only as far as the
read reaches, and a later read carries on from where it stopped:

- The LZMA2 decoder (`codec::lzma::Lzma2`) decodes one LZMA2 chunk per
  call and keeps its state between calls. A read stops after the chunk that
  holds its last byte. A chunk comes to at most 2 MiB, and `xz` also ends a
  chunk at 64 KiB of packed data, so a read of text decodes at most about
  2 MiB past its end.
- `Seekable` keeps each xz span as far as reads have decoded it, with the
  decoder of the block that the last read stopped in (`lazy::xz::Prefix`).
  A read further into the span carries on from there, and a read of bytes
  already decoded decodes nothing.
- These spans are kept in a cache of their own (`Seekable::prefixes`) under
  the same rule as the cache of whole spans: 16 MiB, plus the span read
  most recently, whatever its size. A span counts as the memory it holds:
  its decoded bytes, the room made for the next chunk (never more than the
  block's size), and the decoder's state while a block is part way through:
  about 16 KiB for the settings `xz` writes, and at most 28 KiB.
- Each block decodes into its own vector, as when a whole stream is
  decoded, because the LZMA2 decoder reads its dictionary from that vector
  and counts its 64 MiB cap from the vector's start.
- Before the first chunk of a block is decoded, the block's chunk headers
  are read up to the byte that ends its LZMA2. Each header gives its chunk's
  packed and unpacked sizes, so this decodes nothing. The unpacked sizes
  have to add up to what the index says the block comes to, or the read is
  refused, so a block the index is wrong about is still refused before any
  of its bytes are returned.
- Damage inside a chunk's packed data is found only by a read that reaches
  that chunk. A read of the chunks in front of it returns their bytes. A
  read that is refused drops what was decoded of its span, and the next read
  of the span starts again from its first block. A read that finds chunks of
  the file missing keeps what was decoded.
- The read copies the LZMA2 data of the blocks it decodes through the
  index's `RunInput`. From a source that keeps only part of the file, it
  copies at most `Limits::span_input` (16 MiB) of the stream at once, and
  decodes what it copied before it copies more. From such a source, a block
  longer than `Limits::unit_input`, also 16 MiB, is refused when the stream
  is indexed.
  See DESIGN-lazy-compressed-input.md, "Built (part 6)".

**Traces.** The trace of a zstd span is the same as the part of a whole
run's trace that covers the same frames, starting at the span's first bit
and byte. The trace of an xz span is the per-symbol LZMA2 trace that a whole
stream has for the same blocks (`codec::xz::blocks`), with the stream header
in span 0 and the index and footer in the last span. It is made by decoding
every block of the span, however far reads have decoded it. In both, every
step is the same step that the whole trace has at that position.

**The zstd decoder.** `codec::zstd` reads a run with one walk over every
frame (`frames::ZstdWalk`), for the bytes and for the trace, with a new
`ruzstd` decoder for each frame. A run may start with skippable frames, as
every `pzstd` file does. Bytes after the last frame that are neither a zstd
frame nor a skippable frame make the run fail to decode. Until a frame ends,
`ruzstd` holds back the last window's worth of output and does not report
where the outputs of the zstd blocks in it divide. So the zstd blocks that
decode while `ruzstd` is still filling its first window are one step of the
trace, and every zstd block after that is a step of its own, with the output
that block produced. A frame that asks for a window larger than 128 MiB
(`MAX_WINDOW` in `codec/frames.rs`) is refused as `TooLargeWindow` before it
is decoded, because `ruzstd` hands over nothing until the window is full, so
the output of such a frame could grow far past every limit before any check
sees it. 128 MiB is the largest window the `zstd` command line decodes by
default. The sniff in `formats/recognise.rs` takes a file that starts with a
skippable frame as zstd when the zstd magic is where that frame's length
says the next frame starts, so a `pzstd` file is recognised. (LZ4 uses the
same skippable frame magics.)

**Checks.** For a whole deflate run past the cap, the index pass computes
gzip's CRC-32 and zlib's Adler-32. A zstd or xz run has no sum over its whole
output. An xz block's own check (declared in the template with
`Covers::UnpackedMember`) is computed over that block's output, read through
the index.

**zstd checksums.** A zstd frame may end with a checksum of its own output
(the low 32 bits of its XXH64), and `zstd` and `pzstd` write one by default.
Every walk over a frame checks it when the frame ends: the
whole decode of a run under the cap, both parts of the index pass, and each
span's decode and trace. A frame whose checksum does not match is refused as
`Refusal::Failed`, as a gzip trailer that does not match is. `ruzstd` sums
the output as it lets go of it when built with its `hash` feature, which adds
only the `twox-hash` crate with that one hash. On the generated `pzstd -1`
file measured below, the sums over its 500 MB of output took 52 to 88 ms
native, about 6 to 8% of the index pass, and made decoding 2 to 4% slower
in wasm.

**What the tools write.** Checked with zstd 1.5.5 and xz 5.4.5:

- `pzstd -<level>` cuts its input every four window sizes (2 MiB at level 1,
  8 MiB at level 3) and writes each piece as a frame, with a skippable frame
  before it that gives its length, and with no content size. The last frame
  is empty.
- `zstd -T0 -B<size>` writes one frame. In compression, `-B` sets the size of
  each thread's job, and a job may copy from the job before it. (The help
  text's "cut file into independent chunks" describes `-B` in benchmark mode
  only.) So a file from it whose output is over 64 MiB is still refused.
- `xz -T0` cuts its input into blocks of three times the dictionary size, or
  1 MiB if that is larger: 24 MiB at the default `-6`, 1 MiB at `-0`. Plain
  `xz` writes one block. `xz -T0 -9` writes 192 MiB blocks, which are over
  the 64 MiB span limit, so a file from it is refused once its output passes
  64 MiB.

Measured in a release build, native, on 500,000,000 bytes of generated
CSV-like text. Every read was compared with the original text. Packed sizes
and peak RSS are in MB (10^6 bytes); span sizes are output in MiB.

| File | Packed | Spans | Index pass | 50 random 64 KiB reads | Reading all of it, 1 MiB at a time | Peak RSS |
|---|---|---|---|---|---|---|
| `pzstd -1` | 37.5 MB | 239 spans of 2 MiB | 1.59 s | 0.36 s (7 ms each) | 1.76 s | 106 MB |
| `pzstd -3` | 43.4 MB | 60 spans of 8 MiB | 1.63 s | 1.55 s (31 ms each) | 2.01 s | 117 MB |
| `xz -6 -T0` | 21.3 MB | 20 spans of 24 MiB | 35 µs (laid out from its index, from a copy) | 4.07 s (81 ms each) | 3.55 s | 103 MB |
| `zstd -3 -T6` (one frame) | 43.1 MB | refused as `too-large-zstd-frame`, 0.31 s after the pass started | | | | 116 MB |

The peak RSS of the zstd files comes from the first part of the index pass,
which holds up to 64 MiB of output (`Limits::whole`) and its trace, as
deflate's pass does.

The `xz -6 -T0` row was measured again on 2026-11-26, after a read of an xz
span stopped at the end of the read (see "Reading an xz span"). It is from a
different 500,000,000 bytes of CSV-like text, which packs to 21.3 MB rather
than 17.4 MB, so its packed size and peak RSS are not comparable with the
earlier rows. Each figure is the median of three runs, with the bench built
without LTO, on a machine running other builds at the same time. The same
bench on the same file before the change took 8.77 s for the 50 random reads
(175 ms each) and 3.94 s to read all of it, with a peak RSS of 102 MB.

A random read of the xz file decodes from the start of the 24 MiB block
it falls in to the end of the LZMA2 chunk holding its last byte, which is
half the block on average, and nothing when the cache already holds that far.
The cache keeps 16 MiB of spans and always the most recent one, so it keeps
one or two of these blocks part decoded. Reading the whole file 1 MiB at a
time decodes each block once, a chunk or two per read.

Measured on 2026-11-26, before and after zstd's pass was made a step at a
time and its checksums checked, in a release build, native. The file is
another 500,000,000 bytes of generated CSV-like text (16 KB blocks of rows, a
sixth of them new and the rest repeats of one of the 8 new blocks before it),
packed by `pzstd -1` to 33,360,350 bytes: 239 frames of 2 MiB, each with its
checksum. Other work was running on the machine, so the before and after
runs were made back to back, twice, and each cell gives the first pair's
time, then the second pair's. A range is the two opens of one run.

| | Before | After |
|---|---|---|
| Index pass in one go (`decode_or_index`), best of 3 | 817 ms; 1,041 ms | 863 ms; 1,087 ms |
| Opening `decoded` through the zstd template in goes of 5,000 elements | 1 go: 695 to 727 ms; 819 to 909 ms | 192 goes: 729 to 792 ms; 946 to 963 ms |
| Longest go | the one go | the first: 98 to 108 ms; 130 to 139 ms |
| Every other go | | under 21 ms |
| The pass in steps of 256 KiB (`Indexing::step`) | | 1,911 steps, the longest 8 to 11 ms; 10 to 14 ms |
| XXH64 over all 500 MB of output | | 52 ms; 88 ms |

The first go spends its time before the pass starts, and the pass reads only
the first 8,709 bytes of the run in it. Of the first go's time, 40 to 55 ms
was `read_in` copying the 33 MB packed run out of the document; at the time
of this measurement `index_run` made that copy for every codec. The rest was
spent in `open_space` before it called `index_run`. Before the change, both
were part of the one go that opened the stream. A pass that reads its run
through a handle onto the file copies none of it: see
DESIGN-lazy-compressed-input.md, "Where a `RunInput` comes from".

In wasm, under Node 24, a loop that decodes the same file a block at a time
with `ruzstd`, as the walk does, took 668 to 681 ms without the `hash`
feature and 693 to 696 ms with it and the checksums compared: 2 to 4% more.
The `hash` feature makes that module 2.3 KB larger.

## The other codecs

| Codec | Restart points | Plan |
|---|---|---|
| zstd | A frame is independent; inside a frame, blocks share a window of up to 8 MiB at the default levels and up to 128 MiB at `--long`. A `.zst` file is usually one frame. | Built for several frames: see "zstd frames and xz blocks". One frame stays whole. The seekable format (a skippable frame with a jump table) would give its frames' places without a pass; `ruzstd` does not read it. |
| xz | Each xz block is independent and the index lists every block's two sizes. `xz -T` writes several blocks; plain `xz` writes one. | Built for several blocks: see "zstd frames and xz blocks". One block stays whole. |
| bzip2 | Blocks are independent and at most 900 kB of output, but start at any bit, found by the 48-bit block magic. | Restartable in principle. `bzip2-rs` decodes a whole stream only, so it needs a block decoder of our own or a way to hand the crate one block. |
| LZ4 frame | Independent blocks when the descriptor says so; linked blocks need 64 KiB of history. | Same shape as deflate with a 64 KiB window, and our own decoder. Later. |
| LZMA, lzip, RAR 5, LHA, Brotli, the rest | The dictionary or the context model is the state, and it is large or not exposed. | Stay whole. |

## Still to do

- While a gzip member's stream is measured, the listing has no rows at all,
  not even a placeholder. Only once the index pass starts does the listing
  show the member's header above a placeholder. For the CSV-like text in
  "Measuring a member's stream in steps", measuring was about 35 of the 82
  goes it took to open. The children that are ready (see "Children that are
  ready") cannot show anything sooner, because everything above the member's
  header waits on the walk:
  1. `flatten` draws nothing until the root's `node` answers, and the root's
     `node` needs `size_of`, which needs where every member ends.
  2. `children_ready` of the root gives nothing while `members` waits:
     `members` is a list read to the end of the file, and its `node` needs
     its count, so `members` waits and ends the slice at the first child.
  3. The member being measured has no size until its walk ends, so its own
     `node` waits too, though its ten header fields are ready.

  In a file of one member, which is most gzip files, no member has been
  measured yet while the walk runs, so showing the members measured so far
  would show nothing: only the member's own header could show. Doing it would
  take a `NodeInfo` that can say its size and its count are not known yet; a
  root, a list and a structure answered that way while a walk runs under
  them; a `children_ready` that gives a list's elements before its count is
  known; and a listing that draws a list whose count grows and a structure
  whose end is not known: no edge counting the rest, no range on the heading
  and no share of the file until the walk ends. A finished listing would look
  as it does now. Neither the wire format nor the listing has any of these
  yet, so this was left.
- A pass nobody asks for any more keeps its place in `Unfinished` until calls
  waiting for a place have finished it, up to 1 GiB of output: for example a
  stream whose Open unpacked was replaced by a press on another stream, with
  two other streams being unpacked. Letting it go instead would need a way to
  tell a pass nobody wants from one asked for less often than the call
  waiting; see "Waiting for a place" for why counting asks cannot. Meanwhile
  the waiting call has no progress to show: the listing says "Reading these
  fields…", and Open unpacked's status bar says `UNPACKED.unpacking` at 0%,
  since the call's replies are "working" ones with no progress, until its
  own pass starts. Saying that the stream waits for others to finish would
  need `Unpacking` to carry that.
- A slice whose children wait on bytes of the file is answered whole on the
  wire, though `Evaluator::children_ready` has the children before the first
  that waits. The exception is a pass waiting for the next window of its run
  (see "Children that are ready"). Sending them for every wait would let a ZIP
  entry's header show while its data loads, but the listing's walk would then
  read on into those children while their siblings' chunks load, which is what
  `CHUNK_BUDGET` was tuned against. It would want a test of the chunk cache
  whose source gives them, as `cachedSource` in `flatten.test.ts` does not.
- A stream opened through the core's own reading of a recognised space
  (`Evaluator::open_space` given that space, or `tab_node`) is unpacked with
  no allowance: in one go, apart from the waits for chunks of the file when
  the stream's run is read through the file's chunk store. The wasm crate
  does not go that way (see "The browser"), but a caller of the core that
  does gets no progress and a long call.
- While gzip members are measured, the percentage in the listing and the
  status bar is of the bytes from the member being measured to the end of the
  file. So in a file of several members it goes back to 0 for each member,
  and for a member past 64 MiB it goes up to where the walk reached 64 MiB of
  output and goes back to 0 when the index pass starts. Saying which pass is
  running, or reporting them as one, would need the interface to know there
  are two.
- A go over a run of empty deflate blocks takes up to about 250 ms native
  while the run may still be held whole, most of it the trace each block adds
  (see "A step bounded by the bits it reads"). Charging a step for the bits it
  read, as measuring does, or a smaller trace for an empty block, would
  shorten it.
- The other codecs, in the order of the table above: bzip2, then LZ4 frames.
- A read of an xz span decodes to the end of the LZMA2 chunk that holds its
  last byte, up to 2 MiB past the read. Stopping at the read's last byte
  would need the range coder's state kept between reads, and a stop between
  two symbols in `decoder::run`.
- A joined stream with a packed part whose xz index lists more than 65,536
  blocks is refused as `failed` when Open joined stream is pressed, because
  `join_part_lazily` passes on only `TooLargeLazy`, `TooLargeInput`,
  `TooLargeUnit` and `TooMuchAtOnce` from a part's index pass. A read of the
  part in the listing says `has too many xz blocks to unpack (over 65,536
  blocks)`, and `JOINED.refused` has words for `too-many-xz-blocks`
  (`JOINED.partXzTooManyBlocks`) that this path never reaches. Passing
  `TooManyXzBlocks` on as well would make the two agree.
- How a pass reads its compressed input has its own list: see
  DESIGN-lazy-compressed-input.md, "Still to do".

## Tests

- `codec::lazy` unit tests, with the cap made small (a test knob, like
  `Spaces::set_whole_cap`), on a few megabytes of generated data: bytes match
  `miniz_oxide` at every span boundary and across them, checkpoint thinning,
  spans read in random order, cache eviction, span traces tile, `map_out` and
  `map_in` agree with the whole trace of the same stream decoded under the
  normal cap, and `decode_step` gives the same answer through both.
- The test file: the gzip template reads 16 HDUs out of `decoded` in the
  listing, and Open unpacked on `decoded` gives a FITS tab with 16 image HDUs
  named `Segment..`. Skipped when `qubero2-extras` is absent.
- Blocks: under small limits, the blocks the index lists are the whole
  trace's, one for one (where each starts and ends, its kind, its steps),
  with stored blocks, flushed streams and thinned block starts; the listing
  of a zlib stream read a piece at a time matches the listing of the same
  stream held whole, row for row, down to the codes, the value table's cells,
  the column beside the bytes and the code panel's answer; and the Rubin raw
  lists its blocks under `members[0].compressed`.
- Joined streams (`eval::stitch`), with the limits made small: a BGZF stream
  past the cap read a part at a time; a stream of stored and packed parts
  past the cap, from a template of the test's own, whose bytes, `map_out` for
  every byte and `map_in` for every bit of the file match the same stream held
  whole; and a part past `Limits::lazy` refusing the stream as `TooLargeLazy`,
  with a read of it saying so, beside a damaged part of the same size refusing
  it as `Failed`.
- A generated gzip of several hundred MB, timed with the release-mode bench in
  the session scratchpad rather than in `cargo test`, which runs in debug.
- The pass in steps (`codec::lazy`): steps from 100 bytes to the whole run,
  over runs of one block and of a block every 3,000 bytes, come to the same
  checkpoints, sums and bytes as one pass, and under the cap to the same
  trace step for step; a trace that names blocks partway through a block
  that steps stopped in; refusals; `known_large` and a gzip trailer that
  says the run is large, truthfully or not. Steps of 100 bytes, stopping at
  block starts (stored blocks) and inside blocks, before the cap and after
  it, with `known_large` and with thinned block starts, record the same block
  starts as one pass, the same count before each checkpoint, and the blocks
  of the whole trace.
- The evaluator (`eval::space`): a zlib stream past the (small) cap opened a
  go at a time, with `unpacking` moving forward between goes and the index
  the same as one go's, and a gzip's joined stream read a go at a time
  through the listing's path, a read of its first bytes.
- zstd's pass in steps (`codec::lazy`): steps from 100 bytes to the whole run,
  over frames shorter than their window and frames three windows long, with
  and without skippable frames, come to the same checkpoints and bytes as one
  pass, and under the cap to the same trace step for step; progress never
  goes back, across the cap as well; a step of 16 KiB over 400 KB frames lets
  go of at most its share, a block and a window; `known_large`; the same
  refusals in steps as in one go. A damaged checksum, and a damaged byte in a
  frame of raw blocks, are refused whole, past the cap, and in steps. In
  `eval::space`, a zstd run past the cap opened through its template a go at
  a time, with `unpacking` moving forward, comes to the index of one go.
- Measuring in steps. In `codec::inflate`: steps of 1 to 2^20 bits, over runs
  packed at levels 0, 1, 6 and 9, with bytes after the stream, cut short,
  damaged, and under a cap of a third of the output, end where one go ends or
  are refused in the same way. In `eval::space`, with `whole` made small: a
  gzip of two members, each run several steps long, reads to the same fields
  in goes of 600 elements as in one go, with each walk moving forward from go
  to go, and the joined stream reads across the join between the members. A
  member past `Limits::measure`, set to 1 MiB, falls back in goes as in one
  go, in a file of one member and in a file of two. An edit to a member that
  has been measured, with no stream opened, gives the same fields as a fresh
  reading of the edited file.
- A step bounded by the bits it reads (`codec::lazy`): two runs of 40,000
  empty fixed blocks, one before the cap and one after it, indexed in steps of
  10,000 bits, are dozens of steps with no output and come to the index of one
  pass, raw, zlib and gzip, with block starts thinned or not and told the run
  is large or not; under the cap, steps of 1, 64 and 999 bits come back whole
  with the trace of one pass, step for step, and steps of 200 bits over runs
  of dynamic blocks stop at the first code of blocks whose tables are longer
  than that. Runs of 2,000 skippable frames and 2,000 empty zstd frames,
  before the cap and after it, indexed in steps of 1,000 bytes, come to the
  same checkpoints, one of them at the first skippable frame of a run, and
  under the cap to the same trace.
- zstd and xz, in `codec::lazy` with the limits made small: runs of several
  frames packed by `ruzstd`, with and without `pzstd`'s skippable frames, and
  streams of several blocks packed by `lzma-rust2`, read back from anywhere;
  span traces that match the whole trace; one frame or block past
  `Limits::span` refused as `TooLargeUnit`, in one go, in steps, told the run
  is large, and when the frame was finished before the cap; a frame asking for
  a 1 TiB window refused as `TooLargeWindow`; a frame's raw blocks traced to
  exactly their own bytes once the frame is past its first window; an xz block
  that does not match the index refused when read. In `eval::space`, both open
  through their templates, and an xz block's CRC-32 is checked through the
  index.
- xz spans read part at a time (`codec::lazy`, small limits), over blocks of
  many chunks that start over, blocks whose chunks carry the state on,
  stored chunks, and spans of two blocks: after each read, the span is
  decoded exactly to the end of the chunk holding the read's last byte; bytes
  read front to back in pieces, in random order, and part early and the rest
  later all match the stream's bytes; the cache stays under its limit plus
  the largest span; the span traces match the whole trace. A damaged chunk
  is refused by a read that reaches it, while the chunks in front of it and
  the other blocks still read. In `codec::lzma`, two LZMA2 decoders taking
  turns a chunk at a time give each stream's bytes, and the chunk headers
  alone give each stream's length.
- `crates/core/tests/lazy_frames_real.rs`: files from `pzstd -1`,
  `zstd -1 -T0 -B1MiB` and `xz -0 -T0` in the sample collection's
  `compressed` folder, made by its `tools/make_frame_samples.py`, read back
  byte for byte with `whole` at 256 KiB. The `zstd -B` file is refused as a
  frame too large. The `xz -0 -T0` file, laid out through an `ArcSource` in
  steps of 64 KiB, comes to the index that a copy of it comes to.
- `crates/wasm/tests/open_space_waits.rs`, on files made in the test: a gzip
  whose chunks are fed only as `open_space` names them opens in a few rounds,
  with no chunk asked for twice; a gzip run of 7 chunks opens from a store of
  3, read a window at a time, with no chunk asked for more than twice
  (`a_run_longer_than_the_store_opens_a_window_at_a_time`); and a tar of
  70 MiB of zeros packed by hand as
  zlib, opened as a recognised tab and then opened again after an edit, says
  how far that unpacking has got on every "working" reply about the tab. The
  same tar packed as a gzip inside a gzip, opened from the tab of the inner
  gzip, says how far it has got in that tab's own reading. The tar packed as
  a gzip of one member: the member's children come back "working" with the
  ten header fields as `ready` and the member's run as `unpacking`, and then
  whole. Two such tars as the entries of a ZIP: a listing of the first
  entry's fields and Open unpacked on the second, by turns, each say how far
  their own entry's pass has got, on every "working" reply. Three tars of
  12 MiB of zeros as the entries of a ZIP, each entry's fields asked for once
  a round: all three list within 100 rounds, at least one "working" reply
  names no pass, since its entry waited for a place, and every other one
  names its own entry's pass, which never goes back. A gzip of one member,
  more than three times as long as a store of 16 chunks: after the pass
  starts, every "pending" reply to the listing's ask for the member's fields
  carries the ten header fields as `ready` and the pass's progress as
  `unpacking`, which never goes back
  (`a_gzip_members_header_lists_while_its_stream_waits_for_the_next_window`).
- Which pass a call waits on, and the children that are ready, in
  `eval::space` with the limits made small. A gzip member's children, read in
  goes of 600 elements, come back with the header ready and the rest waiting
  in at least three goes of the index pass, and then whole. Two members'
  streams carried on by turns, a listing read of one and an open of the
  other, each report their own run; a go reports nothing until it runs out
  in a pass; and a go that runs out in one pass and then reaches the other
  reports the first.
- Waiting for a place, in `eval::space` with the limits made small, on ZIPs
  made in the test. Three entries of about 2 MB each, opened by turns a go at
  a time, all open, with three passes made and no more; also with the third
  entry asked four times a round, before the others and after them. Every
  `Busy` names its own entry's run or none, no pass goes back, no more than
  two passes are held at any go, and each entry reads back its own text. Two
  entries started and never asked for again, then a third: the third opens,
  past the cap with three passes made, and under it, where both places come
  to hold runs held whole, with four, the first of those having been let go
  of; then the first two open and read back their text. A call with an
  unlimited allowance opens a third entry in one call beside two passes held,
  which it leaves as they were, and those then finish with no pass made
  twice. Three deflate streams
  measured by turns each come to their own end, and the gzip template's
  members are measured one walk at a time. Before the change, the three
  entries and the three streams stayed at one go's progress for 300 rounds.
- `web/test/openwait.test.ts`: `waitToOpen` loads and asks again through
  "pending" and "working" to the tab, and stops on the same chunks twice, on
  chunks pushed out by their own load, on a read that fails, after
  `MAX_ROUNDS`, and with nothing more said once the wait is no longer wanted.
  A pass that waits on each next window of its run says how far it has got
  as the chunks load, and goes past `MAX_ROUNDS` while its progress moves; a
  pass that waits again on the same window with no progress ends the wait.
- `web/test/chunkwait.test.ts`: `readInPieces` reads a range longer than the
  store in pieces, each read again until it is complete, and gives up at a
  piece that needs more of the file at once than the store keeps; a round
  that asks again for a chunk loaded in the same wait ends it; the backstop
  is the file's chunk count and never less than `MAX_ROUNDS`; and `stretches`
  groups missing chunks into one read for each run of neighbours.
- The tests of how a pass reads its compressed input, through a store that
  holds only some chunks of the file, are in DESIGN-lazy-compressed-input.md,
  under "Tests" and in each of its "Built" sections.
- `web/test/flatten.test.ts`: a member whose children come back "working"
  with six ready is six rows and one pending item over the rest of the
  member, with no gap and no edge; with none ready, one pending item over the
  member; two members waiting on two passes each keep their own progress; a
  root whose second part waits has no gap after the first; a structure whose
  ready children have a child still to come between them in the file has no
  gap, at the root and further in; a record table whose rows wait is one
  pending item; a page of a list that stops at a child that waits has no edge
  counting the rest.

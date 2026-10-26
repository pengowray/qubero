# Design: unpacking large compressed runs a piece at a time

Written 2026-10-21. How a compressed run whose output is larger than
`codec::CAP_BYTES` (64 MiB) opens, with only the part being read decoded.
Deflate, zlib and gzip first; the other codecs are covered at the end.

**State, 2026-10-21:** built as described below, on branch
`nonhuman/sweet-solomon-104c5d`, in `codec/lazy.rs`, `eval/space.rs`,
`eval/stitch.rs` and the wasm crate. "Still to do" at the end lists what is
left. **2026-10-26:** the `blocks` listing for these runs (see "Blocks") is on
branch `nonhuman/great-kapitsa-d7a052`, built on that one.

## The problem

Every codec in `codec.rs` decodes a run whole into one `Vec<u8>` and refuses
with `Refusal::TooLarge` once the output passes 64 MiB. The cap guards against
zip bombs, and it also refuses ordinary large files. The test case is a
gzipped Rubin LATISS raw, `qubero2-extras/rubin/3018092000065-det000.fits`:
94,940 bytes of gzip that inflate to 75,677,760 bytes (a ratio of 797 to 1),
which hold a FITS file: a primary HDU, 16 image HDUs named `Segment00` to
`Segment07` and `Segment10` to `Segment17` (576 × 2048 32-bit integers each, not tile-compressed) and two
binary tables.

The gzip template reads that file through three paths, and all three hit the
cap:

1. `members[0].compressed` is a `Decoded` run sized by
   `E::stream_len(Codec::Deflate)`. `inflate_len` inflates it to the cap,
   refuses, and the template falls back to "everything before the trailer".
   That costs a 64 MiB inflate (about 170 ms native) before anything is shown.
2. `decoded` is a `Stitched` stream over every member's `compressed` run, with
   each part as long as the member's `original_size`. A read of it reaches
   `Evaluator::unpacked_part`, which inflates the whole part through
   `codec::decode` and fails.
3. Open unpacked on `decoded` calls `join_whole`, which refuses a joined stream
   past `whole_cap` (64 MiB) before reading anything.

A `Decoded` node opened directly (`open_space_at`) refuses the same way.

## What changes for the reader

- A gzip, zlib or raw deflate run that inflates to more than 64 MiB and at most
  1 GiB opens: its fields read in the listing, and Open unpacked gives a tab
  over the whole output.
- The tab, the listing and the hex view read the output a piece at a time.
  Memory stays bounded whatever the output size (see "Memory").
- Selecting a byte of the unpacked tab still shows the deflate code that made
  it, and selecting a bit of the compressed run still shows the bytes it made.
- The compressed run still has its `blocks` child: one row per deflate block,
  and inside each block its header, tables and codes. The rows inside a block
  are made when the block is opened. See "Blocks".
- Runs of the other codecs past 64 MiB are refused as they are today.
- Runs at or under 64 MiB are decoded and traced exactly as they are today.

## Limits

| Limit | Value | What it bounds |
|---|---|---|
| `CAP_BYTES` | 64 MiB | Output of a run held whole, as today. Also the compressed input, which is still read into memory whole. |
| `LAZY_CAP_BYTES` | 1 GiB | Output of a run read a piece at a time. The zip-bomb guard. |
| `MAX_CHECKPOINTS` | 256 | Restart points kept per run. 256 × 32 KiB = 8 MiB of windows. |
| `SPACING_START` | 1 MiB | Output between restart points, doubled whenever the count passes 256. |
| `SPAN_CACHE_BYTES` | 16 MiB | Decoded output kept per run. |
| `SPAN_TRACE_STEPS` | 1,000,000 | Steps in one span's trace before it names blocks instead of codes, as `MAX_STEPS` does for a whole run today. Also the steps in one block's trace, and in all the block traces a run keeps. |
| `MAX_BLOCK_MARKS` | 65,536 | Block starts kept per run. 65,536 × 16 bytes = 1 MiB. |

**Why an output limit and not a time limit.** Time depends on the machine, so
a time limit makes the same file open on one computer and not on another, and
tests cannot check it. A ratio limit would refuse the test file, which is 797
to 1; deflate's own maximum is about 1032 to 1, so any ratio limit low enough
to matter refuses real files. An output limit is the same everywhere. 1 GiB of
output costs at most about 8 MiB of checkpoints, and output positions stay
inside the `u32` that `RawStep.out_start` stores.

**What a bomb costs now.** Memory is bounded by the table above, so a bomb can
only cost time: one pass over up to 1 GiB of output. The measured speed of the
existing decoder is about 70 MB/s native with tracing on (60 MB of text-like
data in 832 ms), so the worst case is around 15 s native, more in wasm. A run
past 1 GiB is refused as soon as the pass reaches 1 GiB.

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
header, and restarting reads the header and tables again from the compressed
run, which is in memory. A checkpoint is:

```rust
struct Checkpoint {
    in_bit: u64,          // where the next code starts, in bits of the run
    out: u64,             // how many bytes of output come before it
    block: Option<u64>,   // the header of the block it is inside, if any
    window: Box<[u8]>,    // the last min(32 KiB, out) bytes of output
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

1. Decode as today, into a growing `Vec<u8>`, with the trace on, and record a
   checkpoint every `SPACING_START` of output (copying 32 KiB out of the
   `Vec` is about 3% of the work).
2. If the stream ends at or under 64 MiB: drop the checkpoints and return the
   bytes and the trace, exactly what `decode_traced` returns today.
3. If the output passes 64 MiB, the decoder refuses as it does today. Keep the
   checkpoints, take the CRC-32 and Adler-32 of the output up to the last
   checkpoint, drop the `Vec` and the trace, and decode again from the last
   checkpoint with the trace off, keeping only the last 32 KiB of output and
   recording checkpoints. The checksums are brought up to date as output
   leaves the window. Starting again costs at most one spacing of output
   decoded twice.
4. At the end, check the gzip or zlib trailer as today, and return an index:
   the compressed run, the checkpoints, the output length, both checksums, and
   where the deflate data starts and ends inside the run.

The peak memory of the pass is the first decode's: up to 64 MiB of output
and its trace, which is what decoding a run at the cap costs today.

Past `LAZY_CAP_BYTES` the walk stops and the run is refused with a new
refusal, `Refusal::TooLargeLazy` (tag `too-large-lazy`), so the interface can
say "over 1 GiB" instead of "over 64 MiB".

When the checkpoint count passes `MAX_CHECKPOINTS`, every second checkpoint is
dropped and the spacing doubles. A 75 MB run keeps about 72 checkpoints at
1 MiB (2.3 MiB of windows); a 1 GiB run keeps 256 at 4 MiB.

Measured in a release build, native:

| Run | Packed | Unpacked | Index pass | Spans | Windows | Notes |
|---|---|---|---|---|---|---|
| The Rubin raw | 94,940 B | 75,677,760 B | 0.38 s | 73 | 2.3 MiB | |
| Generated CSV-like text | 18.3 MB | 500 MB | 2.1 s | 239 | 7.8 MiB | 50 random 64 KiB reads: 0.2 s. Reading all 500 MB in 1 MiB reads: 0.97 s. Peak RSS 117 MB. |

`inflate_len`, which sizes a gzip member's deflate run in the listing, stays
at 64 MiB. Past that the template's fallback (everything before the trailer)
is right for a file of one member, which the test file is. Raising it would
make a hostile file cost the full 1 GiB walk once to size the member and again
to index it.

### One decoder

`codec.rs` says the traced path is the only path, so the bytes a reader sees
are the bytes the trace describes. The index pass, the span decodes and the
span traces all run `inflate::run`, changed in two ways:

- **Output goes to a sink**, a trait with `literal`, `copy(dist, len)`,
  `stored(bytes)` and `len()`. One sink grows a `Vec` (today's behaviour). The
  other keeps a sliding window and can capture an output range, record
  checkpoints and stop at a target. The index pass starts with the first and
  switches mode when the output passes 64 MiB.
- **It can start from a checkpoint**: read the block header and tables at
  `block` if there is one, then move to `in_bit` and carry on decoding codes.

The trace builder gets an "off" switch, so a decode that only wants bytes
records nothing.

### Reading bytes

The output is divided into spans, one per checkpoint, from that checkpoint to
the next. A read of bytes `[a, b)` finds the spans it covers by binary search,
and for each span either takes it from the cache or decodes it from its own
checkpoint and caches it. One decode per span: there is no decoding from an
earlier checkpoint and throwing output away. Spans are at most twice the
spacing plus 64 KiB, so a read costs at most one span decode per span it
touches (about 15 ms native for a 1 MiB span of text). The cache drops the
span read longest ago once it holds more than `SPAN_CACHE_BYTES`.

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
  rebuild the block's Huffman tables from them.
- If the checkpoint is inside a block, one `Opaque` step covers the codes
  between the end of the tables and the checkpoint. It produces no output in
  this span; those bytes belong to the span before. The steps still tile the
  span's own bits and bytes, so `check_tiles` holds for it.
- Span 0 of a gzip or zlib run starts with the wrapper header step, and the
  last span ends with the padding and the wrapper trailer step, as
  `inflate::zlib` and `inflate::gzip` push them today.
- Positions in a span trace are the same as in a whole trace: bits from the
  start of the run and bytes from the start of the output. Both fit the `u32`
  a `RawStep` keeps, since the run is at most 64 MiB (2^29 bits) and the
  output at most 1 GiB. So `map_out`, `map_in` and `decode_step` need no
  shifting. `Trace` gains the bit and byte its first step starts at (zero for
  a whole trace), and `check_tiles` counts from there.

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
  the next one starts. BFINAL and BTYPE are read again from the run when
  asked for, so a block start is 16 bytes.
- **Bounded like checkpoints.** A hostile run can start a block every 10 bits,
  which is tens of millions of blocks in a 64 MiB compressed run. Past
  `MAX_BLOCK_MARKS` the index keeps the start of every second block, then of
  every fourth, every eighth and so on, each time the list fills. A dropped start is found again by decoding from the kept start
  before it with the output counted and not kept, and the lists found this way
  are cached (16 of them). The total count is kept exactly, so the listing
  shows every block either way.
- **Where the second pass starts.** When the first pass's output passes
  64 MiB (see "The index pass: one walk"), the second pass starts from the
  last checkpoint and records again every block that
  starts at or after it. Each checkpoint keeps how many blocks start before
  it, so the record is cut back to that count first.
- **A block's rows.** Opening a block traces that block alone: the decoder
  starts at the block's header (as a span starting at a block boundary does)
  and stops at the next block's start, with the output counted and not kept.
  The trace has the block as its only entry, so `BlockView` and the rest of
  `traced.rs` read it as they read block `i` of a whole trace. `SPAN_TRACE_STEPS`
  sets two limits here: a block with more codes than that has its codes as one
  opaque step, and the cached block traces of one run hold at most that many
  steps in total.
- **One view in the evaluator.** `traced::Blocks` is either a whole trace or
  an index. The `blocks` row, each block's row, and which block holds a bit all
  come from the block starts, with no block traced (a dropped start is still
  found by decoding, as above). The rows inside a block come from
  `Blocks::trace_of(i)`, which returns the trace holding block `i`'s steps and
  the block's number in that trace: the whole trace and `i`, or the block's
  own trace and 0.
- **The inspector's code panel.** `Space::trace_at_bit` gives the trace of the
  block holding the bit, and the span's trace only for a bit outside every
  block (the wrapper, the padding). The listing lays a block's codes out from
  the same trace, so the panel can decode every code the listing shows. A
  span's trace could disagree with the listing: a span of 1 MiB of literals
  has more than `SPAN_TRACE_STEPS` steps, so its trace has one opaque step
  where the listing shows codes.

The gzipped Rubin raw, the test file above, has 10 blocks in its one member,
about 7.5 MB of output each.

## Where it plugs in

Nothing in the IR (`Ty`, `Expr`, templates) changes, and `Document` is
unchanged. The changes are in the decoded-space layer, plus one small change
to `ChunkStore` in `source.rs`.

**`codec::lazy` (new).** `Seekable`: the index, the span cache, the span trace
cache, `read(at, out)`, `map_out(byte)`, `map_in(bit)`, `trace_near(bit)`,
`crc32()`, `adler32()`. `decode_traced` keeps its signature; a new
`decode_or_index(codec, packed)` returns either the whole bytes and trace or a
`Seekable`, for the three deflate codecs, and calls `decode_traced` for the
rest.

**`Spaces` (`eval/space.rs`).** A third backing, `Backing::Lazy(Arc<Seekable>)`.
`Spaces::set_lazy_limits` (tests only) makes the limits small, as
`set_whole_cap` does for joined streams.
`len_bits` answers from the index; `buf` and `trace` answer `None`; a new
`lazy(space)` hands out the `Seekable`. `Evaluator::read_in` gets a third arm
beside the stitched one. Indexes are also kept in a small registry keyed by
where the run is (space, bit offset, bits, codec), so the member's `Decoded`
node, the stitched part over the same run and the tab all share one index. The
registry holds at most 64 MiB of compressed runs in total and drops the index
used longest ago to make room.

**`open_space_at`.** Calls `decode_or_index`; a `Seekable` becomes
`Backing::Lazy` instead of a refusal.

**Stitched streams (`eval/stitch.rs`).** `read_stitched`, for a packed part
whose claimed length is past 64 MiB or whose whole decode says `TooLarge`,
reads through the registry's `Seekable` for that part's run instead of
`unpacked_part`.

`join_whole`, for a stream past `whole_cap`, builds a joined document read a
piece at a time, when everything it reads from fits under `whole_cap` too
(64 MiB): each stored part's bytes and each packed part's compressed run are
read once and kept. A packed part under 64 MiB is decoded whole on first read and kept in a
cache bounded like `PartCache`; a larger one is read through its `Seekable`.
Past that input limit the joined stream is refused as a document as it is
today, and still reads a part at a time in the listing. A BAM larger than
64 MiB compressed is in that case; a single-member gzip of any output size up
to 1 GiB is not.

**`Space` (the document of its own).** Its document becomes
`Document<SpaceSource>`, where `SpaceSource` is an enum of `Whole(Arc<Vec<u8>>)`,
`Lazy(Arc<Seekable>)` and `Joined(Arc<Joined>)`, all implementing `Source`
and never reporting a missing chunk. `Space::source()` gives it, and
`Space::held()` gives the buffer of a space held whole. `Space::bytes()`
stays, for tests and callers that know the space is held whole, and panics
otherwise; no code outside tests calls it. `template_for` sniffs the first
36 KiB through `read`. `map_out` and `map_in` go to the `Seekable` or the
joined parts when there is no whole trace, and `Space::trace_at_bit(bit)`
gives the wasm crate's `decode_step` the trace that holds a bit: the whole
trace, or a span's. A joined document's codec is `Stored`, so `decode_step`
answers nothing for one, as before.

**`ChunkStore` (`source.rs`).** Gains an optional source behind it. A chunk it
does not hold is read from that source instead of reported missing, and
`has` answers true. The wasm crate's `Sheet::from_space` builds every space's
store this way, so a space is no longer copied into the store whole (today a
64 MiB space is held twice). This matters in the browser: `doc.ts`'s
`ensureRange` loads any chunk `has_chunk` says is absent from the *file*, so a
space's store must never answer false.

**Checks over unpacked bytes (`eval/check.rs`).** `run_check` reads the
whole output of a `Decoded` node. For a lazy one it uses the CRC-32 or
Adler-32 the index pass kept when the check is one of those (gzip's `crc32`
and zlib's `adler32` are), and otherwise refuses as too large, as it does
today. A check over one member of a lazy run is refused as unmapped.

**Other `spaces.buf` callers.** `jpegscan.rs`, `picklecells.rs`,
`pickletorch.rs` and `stitch.rs`'s whole-space shortcut read whole buffers of
JPEG coefficients, pickle arrays and parts. None of those are deflate runs
past 64 MiB except a PyTorch storage, which `pickletorch.rs` reads through
`buf`; each caller gets either a `read` through `read_in` or a refusal that
names the size, never "this stream is no longer open".

**The web interface.** The listing and the inspector used to offer Open
joined stream only for a joined stream under 64 MiB (`JOINED_WHOLE_CAP_BITS`
in `joinedpart.ts`, and the same test in the `tab_probe` example). Whether
one opens now depends on how much of the file its parts take up, which only
the core knows, so every joined stream is offered and a refusal is said in the
status bar when the button is pressed. `DECODED_REFUSED` has a string for
`too-large-lazy`.

## The browser

The index pass runs inside `open_space`, which the wasm crate runs with no
work slice (`open_in`), on the main thread. Measured native (see the table
above): the test file in 0.38 s, 500 MB of text-like data in 2.1 s, longer in
wasm.

The compressed run is still read into memory whole, and in the browser the
file's chunk cache holds 512 chunks of 64 KiB (`CHUNK_CAPACITY` in `doc.ts`),
so a run larger than 32 MiB compressed cannot be read in one go there even
under `CAP_BYTES`. Reading the compressed side a piece at a time as well is
separate work.

v1 accepts the blocking. The fix is cheap on the codec side because the walk can
restart from its last checkpoint: an index pass can stop at any checkpoint and
carry on later. What is left is the plumbing: keep the half-built index in
`Spaces`, charge the walk to the evaluator's work budget so it returns
`Interrupted` like any other slow reading, and have `doc.ts` show progress on
the tab. That is v2.

## The other codecs

| Codec | Restart points | Plan |
|---|---|---|
| zstd | A frame is independent; inside a frame, blocks share a window of up to 8 MiB at the default levels and up to 128 MiB at `--long`. A `.zst` file is usually one frame. | Several frames: a frame is a natural span, with no window to keep. One frame: stays whole. The seekable format (a skippable frame with a jump table) would give spans; `ruzstd` does not read it. |
| xz | Each xz block is independent and the index lists every block's two sizes. `xz -T` writes several blocks; plain `xz` writes one. | Several blocks: a block is a span, found from the index without a pass. One block: needs the LZMA dictionary (up to 64 MiB) per checkpoint, so it stays whole. |
| bzip2 | Blocks are independent and at most 900 kB of output, but start at any bit, found by the 48-bit block magic. | Restartable in principle. `bzip2-rs` decodes a whole stream only, so it needs a block decoder of our own or a way to hand the crate one block. |
| LZ4 frame | Independent blocks when the descriptor says so; linked blocks need 64 KiB of history. | Same shape as deflate with a 64 KiB window, and our own decoder. Later. |
| LZMA, lzip, RAR 5, LHA, Brotli, the rest | The dictionary or the context model is the state, and it is large or not exposed. | Stay whole. |

## Still to do

- Run the index pass in slices with progress on the tab (see "The browser").
- The other codecs, in the order of the table above.
- A test of a joined stream mixing stored and packed parts past the cap: the
  stored parts' path in `Joined` (`JoinedBytes::Held`) has no test of its own.
- A joined stream with a part past 1 GiB is refused as failed rather than as
  too large, since the part's own refusal is not carried through.

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
- A generated gzip of several hundred MB, timed with the release-mode bench in
  the session scratchpad rather than in `cargo test`, which runs in debug.

# Handover: what the scientific formats still leave unread

Written 2026-09-13 from a sweep of the sample collection and the module docs of
every scientific template. A worklist: bugs first, then the IR additions that
close gaps in several formats at once, then what is left per format.

All 13 scientific sample folders read with no errors under `check_tree`. So
almost everything below is coverage (bytes a template places but does not
read, or never places at all), not a template reading something wrong.

## How to measure

```bash
cargo build --release -p qubero-core --example check_tree --example spans_probe --example dump_tree
./target/release/examples/check_tree ../qubero-samples/<format>
./target/release/examples/spans_probe ../qubero-samples/<format>
./target/release/examples/dump_tree <file> <template> <depth>
```

`check_tree` walks only the first four and last four children of any long
list, so a problem in the middle of a list, or one that only shows when every
element is evaluated, does not show up there. `spans_probe` evaluates the whole
file the way the Listing does and prints `file, template, spans, bytes named`.
Treat "bytes named" as rough: it counts `bytes[]` leaves as named, it can
double count (the structured NPY sample names more bytes than the file has),
and a gap span can run over records placed by `Ty::At`. It is good for stark
cases only.

## Closed

| Was | Commit |
|---|---|
| S2: HDF5 extensible-array data blocks and secondary blocks past the index block, paged data blocks under them included | 508fa3b |
| S2: HDF5 paged fixed arrays | 508fa3b |
| S2: HDF5 implicit-index chunks | 508fa3b |
| BGZF, BAM, BAI and CSI: new templates. BGZF sniffs apart from gzip (which fixed false CRC mismatches on every `.bam`), the first block's header and records as fields, later records through a side reader, indexes with split virtual offsets. Matches bamnostic. | 1830e4a, 3b48b16, 8f47af1 |
| NIfTI-1, NIfTI-2 and Analyze 7.5: a new template, headers in either byte order, extensions, voxels shaped by `dim` with `dim[1]` innermost, whole-number scaling. Matches nibabel. `.nii.gz` opens through gzip. | 44d98c4, 12cf556, 8f63c5b |
| SEG-Y: a new template, EBCDIC 037 text, binary header, trace headers and samples for rev 0 to 2.1 in either byte order, and a new `ibm32` float type. Matches segyio on 13 files. | 1cadde4, 7254613, 11c297b |
| FITS tile-compressed images: the table named as a compressed image, and a side reader (`fits_tile.rs`) decoding RICE_1 (1, 2, 4 bytes), GZIP_1, GZIP_2, NOCOMPRESS and the fallback columns, un-quantizing with the standard's dither sequence. Every pixel of 8 images in 4 samples matches astropy. The chunk and page panels now share `steplist.ts`. | 89614b1, ffac1dd |
| HDF5 compound datatypes read by member name (versions 1 to 5, nested compounds), variable-length sequences read as their base type, and the version 2 B-tree walk counting from the HDF5 file's start so MATLAB 7.3 trees walk. Checked against h5py. | 84bb213, 35657aa, 6137b28 |
| GWF: every class checked against three files' own dictionaries, class names taken from the file, version 6 frames (14-byte structure header, fixed from a wrong 10), gzip vectors as spaces, differenced and zero-suppressed vectors in a side reader. GWOSC strain equals its HDF5 twin. | e69bdbd, 37adb5b, aeb0362 |
| HDF4 references across descriptor blocks: one `Ty::Gather` index over every block's table, searched by `Tagged`, so every group and ref in every sample opens (one vgroup member names ref 0, which nothing can have). Also: members kept as special elements found under the `0x4000` tag, version 4 vgroups read in the library's order (they were read with the vdata header layout), labels/units/formats per dimension, by-field vdata as columns. | 290dd24, 742748d |
| ROOT RNTuple: header and footer envelopes as frames (fields named, columns typed), each cluster group followed to its page list, every page placed with its checksum, and page values by column type where the header is stored. Matches uproot 5.7.6 on both samples. A tree walk names 25,113 of 25,318 bytes of `staff`, up from 1,296 (the hex view does not show it; see ROOT below). | 48f2d4b, a3fadfe, 3801d23 |
| miniSEED samples: a side reader (`mseed_steim.rs`) undoes Steim1/Steim2 differences, decodes the fixed-width encodings, CDSN and SRO, checks the reverse integration constant, and shows a samples panel. Exact match with obspy on all 22 records of 2.4 files; miniSEED 3 matches obspy's reading of libmseed's 2.x twins. | 54dca11, caef124 |
| MAT subsystem data: placed from the header offset, read as its own small MAT file, the `FileWrapper__` table's classes, objects and properties labelled by name, and each `MCOS` variable's object reference. Sparse arrays read as columns with a computed `row` per entry. A VAX level 4 file is recognised (it was not). Two BSD-3 samples from foreverallama/matio. | 007fbe9, 19c5cc1, 845c824 |
| Parquet page payloads: open by the chunk's codec (snappy and brotli new, plus gzip, zstd, LZ4_RAW, stored); dictionary pages and `DATA_PAGE_V2` values as fields; every page in the 16 samples read to its values by a side reader with a step panel, bar two brotli pages claiming 2 GB. Pinned against pyarrow. | c7cfeab, e54ecd9, d73b009 |
| GRIB complex packing (5.2, 5.3) as fields: the three group tables with their byte padding, and each group's run at `uint_expr(width)`. A side reader (`grib_values.rs`) undoes the differencing and matches ecCodes on all 195,480 GFS values. PNG-packed sections open as PNG. Two ecCodes-repacked samples. | 33f0f20, 7d3556b |
| S5. `Expr::StartOf`, `E::tagged_in_by`, and a tag index shared by every referrer to one list: an HDF5 variable-length string reads as its text, over its own bytes. Two generated samples, one behind a 512-byte user block. | 240ca28, fea1214 |
| FITS `TSCALn`/`TZEROn` and `BSCALE`/`BZERO`: the stored integer keeps its bytes and a zero-bit `worth = zero + scale * stored` hangs off it. Not read as the unsigned type: the convention is a bias, and `scaled.fits` shows physical 0 on disk as signed -32768. | ef3e54f |
| FITS columns past 32: a row is a list of cells, each working out its own `TFORMn` from `Idx`, so the cap is the standard's 999. Labels are `[2] flux` now, were `col3 flux`. Axes past 9 read; `NAXISn = 0` reads as no data. | cdb051f |
| FITS `CONTINUE` cards read as the pieces they hold; a bare `TFORM1 = 'I'` reads as one binary value (it went down the ASCII path and failed, which broke three of four astropy-written samples). | c2dcf36, ecd6b31 |
| HDF4: vdata rows with named columns, scientific datasets with scales, rasters and palettes, vgroups, special elements. Spans on `grtdfui83.hdf` 155 to 1,047 (bytes named cannot move: every descriptor already claimed its run). Four samples from the HDF Group's test files. | 65137af, 73719b7 |
| CDF values: VXR chain, VVR and CVVR values typed by the variable, attribute and pad values, byte order from the encoding, gzip and run-length (new `Codec::CdfRle`) unpacked for blocks and whole files, versions 2.5 to 2.7. `psp_fld_...cdf` names 70,002 of 70,003 bytes, up from 48,749. | 6f39711, 78faa34 |
| ROOT: a reader beside the template (`root_streamer.rs`, `root_tree.rs`) decodes `StreamerInfo` and every `TTree`: classes, branches, leaves, every basket's offset and entry range, and simple leaves' values, listed in the Logical tab as `ROOT contents`. Checked against uproot on all eight samples. The template still cannot place the baskets (see S4's correction). | 8ab3571 |
| NetCDF classic: a file with exactly one record variable writes its records unpadded, and the template stepped by the padded `vsize`. `recsize` is now the unpadded width in that case. Also fixed on the way: a record variable narrower than four bytes read values that belonged to later records. Four generated samples pin both cases. | 82c8c9d |
| miniSEED 3: a new template (`mseed3.rs`) recognised by `MS\x03`, records sized from their three lengths, extra headers as JSON, Steim frames shared with `mseed.rs`. Three libmseed samples. | 14e413d |
| S6, NPZ half: members already open as NPY through the ZIP entry's decoded space being sniffed; a test pins it and the stale doc is gone. Zarr ZipStore chunks remain (a reader, not an IR change). | d6b864a |
| S1. `Ty::Gather` and `Expr::Placer`: children placed at offsets read from records the template walks to, and a child asking its record again. | `2153c96` |
| FITS heap: every `P`/`Q` descriptor's array placed in the heap, sized by its count and typed by its letter. `comp.fits` names all 86,400 bytes, up from a heap of one gap. | `b5c4fd2` |
| `hdf5.rs` split: the four array chunk indexes and their tests moved to `hdf5_index.rs` (3,112 + 968 lines) | `1e1d5cd` |
| S2: HDF5 fractal-heap indirect blocks, for a heap grown past its largest direct block | `4308a8f` |
| S2: children of a version 2 B-tree node below the root (`HANDOVER-open-hazards.md` 4) | `4308a8f` |
| Kind totals counted an HDF5 object once per hard link to it, so `fractal-heap-deep.h5` covered 9.1 Mbit of a 4.9 Mbit file. What an `At` reaches now counts once. | `0d0ac8f` |
| S3. Field names taken from a sibling list. Now `Field::elem_name_from`: an expression worked out per element of a list, with `Idx` as that element's index, labelling it `[1] y` while the path stays `[1]`. | a47f1ed |
| NPY structured dtype field names: `[0] channel_0000` | ec4812d |
| MAT struct fields labelled with their names, struct arrays included: `[2] one` | e0d9fa3 |
| B1. `spans` never settled in goes on `parquet/delta_binary_packed.parquet`. Not slow and not a loop: a whole pass is 580 ms and 11,709 steps. The list walks in `walk.rs` charged a step for every element they stepped over, placed or not, and `spans` starts again from the top of its window each go, so going back over what the last go listed (7,854 steps of it here) used up a go of 5,000 before anything new was read. Only placing an element is charged now. The same fix settles 16 more samples `spans_probe` gave up on (ELF, PE, LE, Mach-O, DOS, firmware), nearly all of them full 4,000-row windows of code. | 83aba76 |

## Bugs

None open. B1 is in the table above.

## IR additions that close gaps in more than one format

### S1. A pointer list whose offsets come from a lookup

**Unblocks:** the FITS binary-table heap (variable-length array columns), and
one construct for Parquet's pages in place of the `At` per column chunk it uses
now.

**Correction.** The first version of this list said Parquet's pages were
unplaced. They have been placed since `912947b` (2026-09-05): `column_data` in
`parquet.rs` puts an `At` under every `ColumnChunk` for its pages, offset index,
column index and bloom filter, using `Expr::tagged_in`, and
`tests/parquet_real.rs` checks every sample. The module doc and a memory note
still said otherwise. What that arrangement does not give: a node for the region
between `PAR1` and the footer (pages sit at path depth 17, under the footer),
and anything FITS can use.

**Why FITS cannot use an `At` per descriptor.** `locate` asks the placed index
only for bits outside the root's extent, and a FITS root covers the whole file;
`child_at` finds an `At`'s contents only when the `At` is a direct field of the
struct being descended, and a descriptor is four levels inside `rows`;
`Anchor::Window` from inside a row resolves to the row's `Sized`, not the data
unit; and `placed.rs` stops indexing a list after 64 children that add nothing,
which 64 empty cells in a row would trigger.

**Built 2026-09-13** (`2153c96`, `3b1dc2f`, `b5c4fd2`; merged `1f646f0`), as
designed, with these differences found by building it: the walk lives in its
own `eval/gather.rs` with a `GatherState` on the list's `ListState`; only
reaching a record is charged against a go (the B1 lesson); a `Sized` round a
gather gives it a region, and without one it covers nothing where declared.
`Placed::Gathered` reaches the web app and the inspector says `where
descriptor rows[3].col1[0] points`. DESIGN.md has a section ("A list whose
offsets are scattered through the records that hold them").

The FITS heap reads (see Closed). Left from the design:

- Parquet still uses the `At` per column chunk. Moving it onto a gather gives
  the row-group region a node; the attempt stopped at making a struct's gap
  accounting see its zero-size gathers' children, and that unfinished,
  untested change is on branch `wip-parquet-gather-region` (`dc1c86c`).
- `memo.rs` `forget_after` assumes a field depends only on what is before it;
  a Parquet footer is after its pages, so editing a footer offset leaves
  stale placements (true before the gather too).
- A walk into an unpacked RNTuple envelope needs a step through a `Decoded`'s
  child (S4).

### S2. log2 and ceiling division in `Expr`

All five HDF5 gaps this unblocked are closed (see the Closed table).

The three chunk index gaps are checked by `chunk-indexes-large.h5`: every
chunk address the template reaches in it matched the byte offset h5py's
`get_chunk_info` gives, and `hdf5_real.rs` keeps the counts and the first and
last offsets. Placing them made a cost in `Expr::Sibling` show: a chunk asked
for its datatype by searching back through every index entry before it, so a
listing of a 100,000-chunk fixed array took over six minutes. Chunks now read a
copy of the datatype kept in the layout (`datatype_copy` in `hdf5.rs`), and the
same listing takes about 11 seconds, 4.5 of them the entry walk itself. The
version 1 b-tree's chunk entries still search back the old way, bounded by the
64 entries a node holds by default.

The heap tables and the B-tree children are closed (see the Closed table).
Both are checked by `fractal-heap-deep.h5`, whose group of 2,000 links puts
160 of them under a second heap table and indexes them with a tree of depth 2.
The two B-tree panel strings that said only the root of a version 2 tree is in
the Listing were reworded in `46d12a7`. They now show only for a node the
template places somewhere other than where `hdf5_tree.rs` read it, which a
MATLAB 7.3 file does (see HDF5 below).

The same file's links are all hard links to one dataset, which made
`kinds_real`'s `every_sample_adds_up` count that object header once per link,
9.1 Mbit covered of a 4.9 Mbit file. The kind walk now counts a thing an `At`
reaches once, by its start and length (`KindWalk::reached_by_address` in
`eval/kinds.rs`). Pointer lists and chains are not deduplicated, to keep a
large chunk index out of a set, so a graph built from those would still count
twice.

Every HDF5 gap here also applies to NetCDF-4, MATLAB 7.3 and `.h5ad`, which are
HDF5 files.

### Further shared gaps, not started

- **S4. Offsets into the file from inside unpacked data.** 7z's compressed
  header, written up in `formats/sevenzip.rs`.

  **Correction (2026-09-13, from building the ROOT reader).** For ROOT this
  is not the blocker. `Ty::At { anchor: Anchor::File }` already resolves into
  space 0 whatever space the field naming it was read in (`place_child` in
  `eval/mod.rs`), and `rntuple_record` relies on it to place both RNTuple
  envelopes from inside a compressed record; `tests/root_real.rs`'s `anchor`
  test asserts `placed.space == 0`. What blocks the TTree baskets is a level
  up: `fBasketSeek` is a member of a streamed `TBranch`, and a streamed
  object's layout is not a fact about the format but a schema written into
  another record of the same file (`StreamerInfo`), itself compressed and
  streamed. No `Ty` takes its shape from bytes. The IR need is a type whose
  inner shape is looked up at evaluation time from a table the file supplies,
  keyed by a class name and a version read from the prelude, with the
  class-tag back-reference map as evaluator state. Given that, a `Gather`
  with `Anchor::File` and `placer(fBasketBytes)` over
  `fBranches[*].fBasketSeek[*]` places the baskets with nothing else new.
  Whether 7z's case is the same shape or the original S4 is still to check.
- **S5. The Nth element of a list whose elements vary in size.** HDF5
  variable-length strings and every other global-heap object. `h5ad.rs` does
  the walk as a reader because a field cannot.

  **Built 2026-09-13** (`3b35878`..`f17a881`), as designed, with these
  differences found by building it:

  - `within` did not step through an `At` chosen by a `Switch`
    (`at_address` wraps one for the undefined address), so the lookup failed
    on every real file. `within_path` now steps through an `At` the file
    turned out to have, the same rule `descend` applies further down.
  - A map of tag to element index would not have paid: placing element *i*
    of a `Repeat` still walks 0..i-1 in each referrer's own copy of the list.
    The index keeps the list *path* that did the walking and resumes there,
    so every referrer to one stretch of bytes shares one walk (310 extra memo
    nodes for a second referrer without it, under 12 with). The origins panel
    for element 1999 therefore points at a node under element 0's
    `collection`: same bytes, different path.
  - A repeated label keeps its first element (`or_insert`); FITS repeats
    `COMMENT` and `HISTORY`.
  - h5py always appends within a collection, so indices ascend; the sample
    has gaps and a high start instead, and real out-of-order is a unit test.

  A VL string now reads as its text on an `object` row, the cursor on its
  bytes lands there, and `h5ad::vlen_string` is a three-line accessor. The
  index also speeds FITS (full walk of `comp.fits` 310 to 219 ms,
  `manyrows.fits` 172 to 125 ms). **Cost to watch:** every note now places
  its heap object and names itself from it, so a listing of a million-string
  column costs a million placements; if a real `.h5ad` is slow in the
  browser, have the listing not ask for a `named_by` field that is an `At`
  until the row is opened.

- **S6. A ZIP entry takes a template by its name.** NPZ members as NPY, the
  chunks of a Zarr ZipStore.

  **NPZ half closed (2026-09-13).** A ZIP entry's data is
  `T::decoded(.., decoded_text())`, a one-field text struct, so
  `says_only_bytes` holds and `template_for` sniffs the unpacked bytes, and
  `npy::MAGIC` is in the sniff table: both members of `two-arrays.npz` open
  as `npy` spaces. `tests/npy_real.rs` checks it now and the `npy.rs` doc no
  longer says it cannot be done. `Match` on the entry name over another
  template's `Named` types is possible mechanically but buys nothing here
  (`Match` is an exact compare and `Expr` has no suffix or concatenation).
  Zarr chunks are not a name-to-template problem: dtype, shape, order and
  compressor live in a sibling entry's `.zarray`/`zarr.json` in another
  decoded space, and the codec would come from a JSON value. Three IR
  additions for one format; do it as a reader beside the template, like
  `h5ad.rs`.

## Per format, most unread first

### ROOT

A reader beside the template (see Closed) now decodes the `StreamerInfo`
record and every `TTree` at every directory depth: class descriptions,
branches, leaves, every basket with its offset, size and entry range, and the
values of simple leaves. It shows in the Logical tab as `ROOT contents`. In
`uproot-Zmumu-lz4.root` the baskets it lists are 206,455 of 212,813 bytes.

What the *template* names is unchanged (about 2% of that file), because the
baskets can only be placed by the template once it can read a streamed
object, which is the corrected S4 above. Until then the hex view shows them
as a gap while the Logical tab lists them.

- Split `TBranchElement` branches (C++ objects) are reasoned about, not
  proven: no tree in the corpus uses one. Worth a sample.
- Not read: variable-length entries, multi-leaf branches, strings, 2-byte
  floats, 3-byte integers, `CS` compressed blocks.
- RNTuple reads from the anchor to every page (see Closed). Left:
  - **The hex view shows none of it.** `spans` knows a placed field only
    once its forward walk has resolved the field pointing at it, and the key
    list leading to the anchor sits after all the RNTuple data, so envelopes
    and pages show as one gap (the envelopes did before too). A `spans`
    change, not a template one. The same shape probably affects any format
    whose directory is at the end.
  - Page values read only when the header envelope is stored uncompressed: a
    page needs its column type from the header, and an expression cannot
    reach into a decoded run. ROOT nearly always compresses the header
    (`staff` does). A reader beside the template, like `root_tree.rs`, would
    also do split, zigzag and delta decoding with no IR change.
  - Large locators, payloads over one 16 MiB block, and payloads split over
    several keys are not placed. No xxhash-3, so no checksum is verified.
- The panel's classes group row says `StreamerInfo` where an `@0x…` would do,
  and an unsplit `TBranchElement` could name its class.

### Parquet

Pages, offset indexes, column indexes and bloom filters are placed from the
footer, each under the column chunk that points at it (see the correction in
S1). Page payloads open by codec and read to their values (see Closed).
Left:

- LZO has no decoder, and the Hadoop-framed LZ4 (codec 5) has no sample, so
  both keep their bytes.
- v1 data pages, the three DELTA encodings, BYTE_STREAM_SPLIT, BOOLEAN PLAIN
  and FIXED_LEN_BYTE_ARRAY read only in the side reader
  (`parquet_page.rs`), because a v1 page's levels depend on a schema walk
  and delta widths change every miniblock. A bit-packed hybrid group keeps its
  bytes in the template: Parquet packs from the low bit up and bits here are
  addressed from the high bit, so a field per value would name the wrong bits.
- No node covers the row-group region as a whole (branch
  `wip-parquet-gather-region`).
- `alloc-stdlib` 0.3.0 (via `brotli-decompressor`) declares BSD-3-Clause in
  its `Cargo.toml` but ships no licence file, so `THIRD-PARTY-NOTICES.md`
  names the licence without its text. Its sibling `alloc-no-stdlib`, same
  authors, does ship one; that text belongs in `tools/notices-extra.md` for
  it.
- `eval/explain.rs` now holds HDF5, SQLite, PDF and Parquet readers;
  `thrift_field` and `parquet_levels` would sit better in
  `formats/parquet_schema.rs`, and the chunk and page panels are near copies
  that could share one step-list component.

### NASA CDF

Values read now (see Closed): the VXR chain, each block's VVR or CVVR typed by
the variable's data type, attribute and pad values, byte order switched on
the CDR's encoding, gzip CVVRs and whole-file CCRs unpacked, version 2.5 to
2.7 read at half width. Five samples, all cross-checked with cdflib. Left:

- Huffman and adaptive Huffman compression (`d103a2x.cdf` in NASA's
  distribution) identify their codec and keep their bytes; a run-length
  compressed *block* has no signature to peek at and stays bytes.
- Sparse-record reconstruction, and multi-file variables (`example1.cdf`'s
  `.v0` to `.v3` sit in other files).
- VAX and VMS Alpha/Itanium encodings read at the right width as IEEE and are
  wrong, as `mat.rs` does for level 4 on a VAX; the doc says so.
- Records are sized from the block's room (`remaining / records / width`)
  rather than from `product(dim_sizes)`, because a dimension the variable does
  not vary along is not stored; the shape is in the descriptor for a reader
  to fold in.
- CDF_EPOCH (float64 ms since 0 AD) and CDF_TIME_TT2000 (int64 ns since J2000
  on TAI) are not declared as moments. `Counted.zero` is whole seconds,
  `moment_number` takes only integers, and TT2000 counts TAI so a linear count
  is up to 5.8 s out after 2017. A `time.rs` item, not a template one.
- The template needed `Ty::Chain` to take an `adjust` and to take its room
  from its anchor rather than the file, so a chain inside an unpacked run does
  not end where the compressed file does. `T::chain` defaults `adjust` to 0.

### HDF4

Vdata rows, scientific datasets (rank 1 to 4, byte order from the number
type, scales, max and min), raster images by interlace, palettes, vgroups
with their members, and special elements (linked blocks, compressed) all read
now, and references are found across every descriptor block (see Closed).
Seven samples, cross-checked with pyhdf. Left:

- Compressed rasters name their compression and stay bytes. `tdata.hdf`'s
  three datasets keep their values in linked blocks, which are placed but
  not joined into one run (the stitched-space gap).
- No HDF-EOS2 granule: hdfeos.org's zoo now needs an Earthdata login.
- The root's `tables` and `index` are hidden machinery rows; how the web
  listing shows them is unchecked. The index table is placed with
  `Anchor::SelfAligned(1)`, which says "at its own start" only by accident;
  an `Anchor::Own` would say it plainly.
- `dimensions` is the hidden 701 record elsewhere and the visible list of
  per-dimension strings in `Hdf4SdStrings`; one of them wants renaming.
- The type column reads `switch[][]` for a shaped array whose element type is
  decided at read time; `f32 be[10][10]` would need a list node to report its
  resolved element type, and would improve HDF5, FITS and NPY too.
- `hdf4.rs` is about 1,750 lines; the standalone records (number type,
  dimensions, palette, strings, vgroup, special element) would split out as
  `hdf4_records.rs`. Due.

### HDF5

Reads further than any other scientific format. Left:

- Filtered chunks are bytes in the template; `hdf5_chunk.rs` decodes deflate,
  shuffle, fletcher32 as a side reader. szip, nbit, scaleoffset and filters
  32000+ stop the walk.
- Virtual dataset mappings are bytes.
- Huge and tiny fractal-heap objects, free-space managers.
- 4-byte offsets are read wrong rather than refused.
- Checksums on the chunk index blocks and pages are placed as fields but not
  verified: the crate has no lookup3.
- **The B-trees panel and the HDF5 contents list still do not open for
  `.mat` files.** The walk now works behind a user block, but three places
  gate on the template name being `"hdf5"`: `contents` and `btree` in
  `crates/wasm/src/lib.rs`, `syncTabs` in `web/src/overviewpanel.ts` (the
  tab), and the `hdf5` adapter in `web/src/logicaloutline.ts`. Allowing
  `"mat"` everywhere would offer a B-trees tab on every level 5 MAT file,
  which has no trees, under an empty-state sentence about groups and chunked
  datasets. It wants a cheap "this MAT file is HDF5 inside" signal (the MAT
  template's root switch already picks a 7.3 arm) and the tab offered on that.
- Compound members go as deep as the file nests compounds; arrays,
  enumerations and sequence element types go two levels, then keep their
  bytes. True version 3 compound bytes and version 1 member dimensions are
  checked only by hand-built unit tests (h5py writes version 5).
- Every compound element walks its member records again when opened. Old
  `.h5ad` files keep `obs`/`var` as one compound row per cell; if one is slow,
  look here.
- Every dataset's `Data` struct gained a `datatype` field before `elements`,
  so hard-coded paths into datasets moved by one.
- `Expr::Sibling` searches back through every earlier element of every list
  around the field asking. Chunks under the array indexes read a copy of the
  datatype instead, but a version 1 b-tree's chunk entries (bounded by 64 per
  node) and the filtered-chunk explain panel in `eval/explain.rs` still pay
  it.

### FITS

Scaling, the column cap, axes and `CONTINUE` cards are closed (see Closed).
Seven samples. Left:

- Tile-compressed images decode (see Closed); PLIO_1 and HCOMPRESS_1 are
  named and not decoded. The gzip fallback column is tested with f32 only.
- **Evaluator bug:** a table's heap fails with "unknown field cards" once a
  later HDU's rows have been read (open `gzip2.fits`, read HDU 3's rows, and
  HDU 1's heap no longer resolves). Also true on main before the tile work.
  `fits_real` works round it with a fresh evaluator; a task was filed.
- `fits_tile.rs` is about 1,370 lines; the Rice decoder and the quantization
  code would each make a module. `explain.rs` grows by one decoder a time;
  a small trait (which ancestor, where the data is, how to decode) would let
  miniSEED, Parquet pages, FITS tiles and GWF vectors share the dispatch.
- The joined value of a `CONTINUE` string is not one node: each card reads as
  its piece, and nothing in the IR reads text out of several runs at once.
  A text-joining `Ty` is the missing piece.
- Every cell still asks the header for its `TFORMn`. Hoisting through a
  `columns` list was built and measured slower, because the memo keeps what a
  node *is* and never what it *reads as*: a `Computed` field re-runs its
  expression on every ask. **A value cache beside the shape cache in
  `eval/memo.rs`** is the fix, and it would help every format that hangs
  computed fields off records. Meanwhile nested `Switch` replaced
  `Expr::equals` for the scaling guard (`equals` is two comparisons, so two
  header walks per ask): `manyrows.fits` 3.7 s to 2.1 s, `comp.fits` 1.2 s
  to 0.28 s for a full deep walk.
- A scale or zero point written with an exponent (`2.0E+01`, cfitsio's
  default) is declined rather than misread; the cards show beside the
  column and the sum is left to the reader.
- `Expr::Idx` answers only for the nearest enclosing list, so a
  variable-length cell needed a `Descriptors` wrapper to keep its column
  index; an `Expr` naming a level out would remove it.
- `fits.rs` is about 900 lines before tests; the card reading (`card`,
  bodies, `tform_value`, `real_value`) would split from the data reading
  (rows, cells, heap, axes).

### GRIB

Complex packing (5.2, 5.3) reads as fields and PNG packing opens as a PNG
(see Closed). Left:

- **What a value is worth is computed and not shown.** `grib_values.rs`
  undoes group references, the smallest difference and spatial
  differencing, and matches ecCodes on every GFS value, but nothing reaches
  a panel: it needs an `Explain` variant and wasm and web wiring, the way
  Parquet's page reader was wired. `explain_packed` also looks only one
  level up from the cursor. Its step strings need a `ui-text` pass when they
  are wired.
- JPEG 2000 (5.40) names its codestream and stays bytes; there is no JPEG
  2000 template.
- Grid templates 3.0, 3.20, 3.30, 3.40 and product templates 4.0, 4.1, 4.8
  only; anything else is bytes.
- `grib.rs` is about 1,900 lines; edition 1 (about 300) would split out as
  `grib1.rs`.
- ecCodes has no wheel for Python 3.14; the cross-check used a uv Python 3.11
  environment.

### NPY / NPZ

- NPZ reads as a plain ZIP (S6).
- Structured dtype nested and shaped fields, explicit `offsets`.
- More than 4 dimensions read as one run.
- Header keys in a non-numpy order read as one run of text.

### MAT

Subsystem data, sparse arrays by column and VAX/Cray level 4 recognition are
closed (see Closed). Left, where the reverse-engineered MCOS documentation
(foreverallama/matio, tbeu/matio) runs out:

- Two unknown words in each class and object entry, regions 6 and 7, and the
  first of the three shared cells read as numbers.
- Only version 4 `FileWrapper__` tables have been seen.
- An object held in a property (a bare `uint32` column with the marker).
- A MATLAB `string`'s UTF-16 text inside its `uint64` value cell.
- The link is one way: a variable shows its object id but not what the
  object holds, since the variable is written before the table.
- scipy 1.18.1's `whosmat` raises `TypeError` on any file with an opaque
  variable; the cross-check used `loadmat`.
- An empty name reads as `ascii[]` in the type column and a filled one as
  `computed text`.
- VAX and Cray level 4 numbers are still read as IEEE and are wrong.

### miniSEED

Samples decode (see Closed), for 2.4 and 3, with a samples panel. Left:

- GEOSCOPE (12/13/14), US National Network, Graefenberg and IPG Strasbourg
  have no documented rule in SEED 2.4 Appendix D or libmseed, so they say
  "Not decoded" rather than guess. Steim3 and HGLP are bytes.
- The cursor on a 32 or 64-bit float sample gets the float bit-layout panel,
  because float types are handled before packings; the data array still
  shows the samples panel. A test records it.
- The SRO, 4096-byte CDSN, and little-endian Steim1/Steim2 reference files
  from libmseed were checked once and are not in the collection; copying them
  in would make those checks permanent.
- miniSEED 3's CRC-32C is placed and not verified.
- `ExplainDto` is one flat struct carrying every panel's fields, about 20
  more per panel; a tagged enum mirrored as a TypeScript union on `kind`
  would stop that. `explain_packed`'s chain of packing-name checks is the
  Rust half of the same problem.
- `chunkpanel.ts` puts its row of values in the 4em label column, so they
  stack one per line; the samples panel has an `.is-values` fix the chunk
  panel does not use yet.

### GWF

All 18 classes checked against file dictionaries, version 6 read, compressed
vectors unpacked (see Closed). Five samples. Left:

- **The vector panel uses HDF5's words.** `explain_gwf_vect` returns
  `Explain::Hdf5Chunk`, so a GWF vector's panel says "Inside this chunk" and
  "Filters, in the order they were undone". Give it its own variant, or make
  the chunk panel's headings neutral.
- Zero-suppressed vectors unpack only in the panel; a packing that carries a
  count expression would open them as a space like gzip ones.
- Unchecked against bytes: FrStatData and the static-data table-of-contents
  groups in version 6, zero suppression of 4- and 8-byte words, scheme 3
  (differences then gzip), complex vectors under zero suppression. Version 7
  stays bytes past its check words.
- `spans_probe` on the GWOSC file names 29 bytes fewer than before, around
  the last-block padding of zlib-decoded nodes (PNG and CDF share it).
- `gwf.rs` holds the v6 and v8 tables and bodies side by side; the class
  bodies would split out as `gwf_classes.rs`.

### NetCDF classic

Reads correctly, the one-record-variable case included (see Closed). Corpus:
seven small generated files. The three `sst-cdf*.nc` files were regenerated
on 2026-09-13: the generator's `surface()` had done its arithmetic on a `">f4"`
array, which drops the byte order, so the floats were little-endian and read
back near 1e-38; they now read as 270 K upwards in scipy and netCDF4 both.
NetCDF-4 is HDF5 and reads as `hdf5`.

### Zarr

A ZipStore is recognised and reads as a ZIP (S6). A directory store cannot be
opened at all, since the app opens one file.

## Corpus gaps

Thin enough that a clean sweep says little: NetCDF classic (3 files, 3 KB, one
dataset), HDF4 (2 files, 8 KB), GWF (1 file), CDF (2 files), FITS (3 files).
HDF5's are small synthetic files; nothing from a real instrument.

## Not built

BUFR, ADIOS2 BP, TDMS. Arrow IPC / Feather was being built on 2026-09-14.
DICOM is read by the bundled Kaitai description (`dicom.ksy`) rather than a
native template.

### BGZF, BAM, BAI and CSI (built 2026-09-14)

Every BGZF block as fields with its `BC` size and both checks; the BAM header,
references and whole records in the first block as fields; BAI and CSI with
every virtual offset split into block and in-block halves (see Closed). Nine
htslib and samtools samples, matched against bamnostic. Left:

- **Records past the first block need a stitched space.** A record can start
  in one block and end in another, and nothing joins several decoded members
  into one space, so later blocks read only through the side reader
  (`bam_records.rs`, `Evaluator::bam_block`). The addition that would fix it,
  as the agent specified: `Ty::Stitched { from: Arc<[Step]>, inner: Box<Ty> }`,
  a zero-width node like `Gather` whose `from` walks to Decoded nodes;
  `open_space_at` joins their outputs in walk order, inflating lazily under
  `CAP_BYTES`, with a part table of (start byte, member path) so a position
  maps back to (member k, offset j), which is exactly a BGZF virtual offset.
  PDB scattered streams, HDF4 linked blocks, Godot RSCC and FITS `CONTINUE`
  are the same shape.
- **No panel.** `bam_block` is a method, not an `Explain` variant, and it
  walks from the header on every call (up to 256 MB unpacked); a panel needs
  a variant, a `bampanel.ts` on `steplist.ts`, and a per-block cache.
- **A `.bam` is labelled "BGZF gzip blocks".** The label names the container,
  not the contents; a reader opening a BAM file wants to be told it is BAM.
  `templateSentence` is where Fable suggested saying so.
- A plain gzip file of several members that is not BGZF still reports a false
  CRC mismatch (its CRC compared with the last member's trailer): the gzip
  template needs a compressed run that ends where its decoder stopped.
- Blocks after the first read as invalid text: only block 0 knows it is BAM.
- `bam.rs` is about 980 lines; the BAI and CSI parts would split out as
  `bam_index.rs`.

### NIfTI and Analyze 7.5 (built 2026-09-14)

NIfTI-1, NIfTI-2 and Analyze 7.5 headers in either byte order, extensions,
and voxels shaped by `dim` (see Closed). Six nibabel samples. Left:

- Voxel scaling applies only when `scl_slope` and `scl_inter` are whole
  numbers, because expressions are integers; `functional.nii` (slope 0.0754)
  shows stored integers. A float-valued computed expression in the IR would
  fix it, and would also let FITS scale by a fractional `TSCALn`.
- `nifti.rs` decodes `vox_offset`'s IEEE float bits to a whole number with
  integer expressions; nothing in it is NIfTI's, so it belongs beside
  `template.rs` if another format stores an offset as a float.
- `NiftiTimeUnit` keeps the 3-bit field's own values 1 to 6 rather than
  `nifti1.h`'s masked constants 8 to 48.
- Two `NiftiEcode` descriptions (`WORKFLOW_FWDS`, `JIMDIMINFO`) came from
  memory rather than a source; check them.
- The `.img` halves of pairs are not in the collection: nothing in an `.img`
  identifies it.

### SEG-Y (built 2026-09-14)

Textual (EBCDIC 037 or ASCII), binary and extended textual headers, every
trace header and samples in formats 1 to 16 bar 4, rev 0 to 2.1, either byte
order (see Closed). IBM floats are a new editable type, `ibm32`. Six segyio
samples, zero mismatches against segyio on 13 files. Left:

- Coordinate, elevation and time scalars are not applied as display values.
- A trace's own extension count in Extension 1 is not followed.
- Seismic Unix (SU) files are not read: nothing identifies them.
- `source_type`, `source_measurement_unit` and `last_trace_flag` are plain
  numbers; their labels need drafting.
- `segy.rs` uses `°` and `²` in labels where `sac.rs` writes `nm/s2`.
- `segy.rs` is about 1,150 lines; a `segy/` folder with the enum tables and
  tests apart would help.
- The SEG rev 2.0 PDF could not be fetched (403); the work used the rev 1
  draft, rev 2.1 text and segyio's `segy.h`, which put the first-trace offset
  at 3521, not the 3301 an earlier brief said.

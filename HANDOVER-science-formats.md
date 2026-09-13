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
| S6, NPZ half: members already open as NPY through the ZIP entry's decoded space being sniffed; a test pins it and the stale doc is gone. Zarr ZipStore chunks remain (a reader, not an IR change). | see below |
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

- **S4. Offsets into the file from inside unpacked data.** Compressed RNTuple
  envelopes (ROOT's page lists), 7z's compressed header. Already written up in
  `formats/sevenzip.rs` and memory `check-ir-structural-gaps`.
- **S5. The Nth element of a list whose elements vary in size.** HDF5
  variable-length strings and every other global-heap object. `h5ad.rs` does
  the walk as a reader because a field cannot.

  **Design (Fable, 2026-09-13), not built.** Mostly there already:
  `Expr::Tagged` with `array: Some(within(["collection","objects"]))` and
  `Tag::Computed(field("object_index"))` finds the object today, since
  `descend` steps through an `At`. What is missing is (a) a child that
  *covers* the found object's bytes rather than a number read out of it, and
  (b) a cost fix: every referrer's `collection` is a distinct path to the
  same bytes, nothing dedups the search, so a column of N strings is O(N^2).

  Recommended: one `Expr::StartOf(expr)`, the byte offset at which the field
  the expression names begins (counted from the nearest `Origin`, so it pairs
  with `at_origin` like every HDF5 address), then in `vlen_reference` an
  `object` field: `T::at_origin(E::start_of(find(["data","payload"])),
  T::sized(find(["size"]), text of `length` bytes or bytes))` marked
  `field_aside` (required: without it `kinds_real` counts every string
  twice, as ELF's `name` shows) and `named_by("object")`. Split
  `heap_object.data` into `payload: bytes(size)` and `padding`. Add a
  builder `E::tagged_in_by(array: Expr, key, tag: Expr, field)`. Plus a key
  index in `tagged_path` for `array: Some(..)` searches: a map key -> element
  index per list keyed by the list node's `(space, offset, limit)`, dropped by
  range in `forget_after`, bounded; excludes `array: None` (GWF's
  nearest-earlier semantics). That index also closes the FITS "every cell
  asks the header for its `TFORMn` card again" note.

  Build order: (1) template-only `payload`/`padding` split and
  `tagged_in_by`, `h5ad::attribute` reads the field and `h5ad::vlen_string`
  goes; (2) the key index, with a test that two referrers walk one
  collection once; (3) `StartOf` (arms in `expr.rs` eval and `text_path`,
  `relate.rs`, `origin.rs`, `machinery.rs`, builder; `uniform()` false) and
  the `object` field; (4) a generated `vlen-strings.h5` (h5py: thousands of
  strings over two collections, VL attributes, a 512-byte user block
  variant, indices out of order) checked in `hdf5_real.rs`; (5) DESIGN.md
  lines on the global heap rewritten. Rejected: a `Ty::Pick` type (every
  `At`/`Chain` arm would need a twin; only HDF5 wants the bytes). Risks: the
  `StartOf` base convention is silent on plain files and off by 512 on
  MATLAB 7.3 if wrong; the placed index walks one stretch per string; a VL
  string inside a VL sequence must not read as a ring.

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

Nearly the whole file is unplaced. In `uproot-Zmumu-lz4.root` (213 KB) about
208 KB lies after the top directory record in bytes nothing reads: the TTree's
baskets, which are TKey records that the directory's key list does not list.
Their offsets are in `fBasketSeek` inside each streamed TBranch. RNTuple: the
anchor and both envelopes are placed; the schema, page lists and the pages
themselves are bytes.

- TTree: needs a StreamerInfo reader beside the template, as `h5ad.rs` is
  beside HDF5.
- RNTuple: the spec needs no streamers, so this is template work, except that a
  compressed envelope's page list names offsets in the file (S4).
  `rntviewer-testfile-uncomp-single-rntuple-v1-0-0-0.root` avoids that.

### Parquet

Pages, offset indexes, column indexes and bloom filters are placed from the
footer, each under the column chunk that points at it (see the correction in
S1). Page payloads keep their bytes: codecs (snappy, zstd, brotli, lz4, gzip)
and then encodings (RLE/bit-packed hybrid, dictionary, delta) are what is left.
No node covers the row-group region as a whole. Of the four at the top of this
list, Parquet is the least unread.

### NASA CDF

Descriptor chains read; values do not. Checked against a tree, not only the
module doc: variable index records (VXR), value records (VVR), compressed value
records (CVVR) and sparseness records all fall to the `T::bytes(E::Remaining)`
default in `cdf.rs`, as do attribute entry values and pad values. Needs a
switch on the encoding field in the descriptor record, the way GWF and ELF
switch on byte order. Whole-file compression is not unpacked. Version 2.x stops
after the global descriptor. The 66 unused-space records in
`psp_fld_l2_mag_rtn_1min_20200104_v02.cdf` are placed and really are free
space, so its bytes-named figure undercounts nothing there.

### HDF4

Every data descriptor is placed and named by tag and ref; three tags are opened
(library version, file identifier and description, vdata header). Scientific
datasets, raster images and vdata rows are bytes. Opening them means following
refs to the dimension record and number type. This is HDF-EOS2, which MODIS and
older NASA missions publish. Corpus: 2 files, 8 KB, no real granule.

### HDF5

Reads further than any other scientific format. Left:

- Filtered chunks are bytes in the template; `hdf5_chunk.rs` decodes deflate,
  shuffle, fletcher32 as a side reader. szip, nbit, scaleoffset and filters
  32000+ stop the walk.
- Variable-length strings (S5).
- Compound datatypes are one element of the right size.
- Virtual dataset mappings are bytes.
- Huge and tiny fractal-heap objects, free-space managers.
- 4-byte offsets are read wrong rather than refused.
- Checksums on the chunk index blocks and pages are placed as fields but not
  verified: the crate has no lookup3.
- `hdf5_tree.rs`'s version 2 walk ignores the superblock's base address, so
  behind a user block (every MATLAB 7.3 file) its nodes come back with no
  template path and the B-trees panel cannot open them in the Listing.
- `Expr::Sibling` searches back through every earlier element of every list
  around the field asking. Chunks under the array indexes now read a copy of
  the datatype instead, but a version 1 b-tree's chunk entries (bounded by 64
  per node) and the filtered-chunk explain panel in `eval/explain.rs` still
  pay it.
- `hdf5.rs` is about 3,700 lines. The chunk index code (implicit index, both
  arrays, `array_entry`, `entries`, `page`, `page_written`, `datatype_copy`) is
  about 500 lines with a clean edge and would split out as `hdf5_index.rs`.

### FITS

- `TSCALn`/`TZEROn` not applied, so unsigned 16-bit columns read as signed.
- Tile-compressed images read as a binary table of compressed tiles, and now
  the tiles' compressed bytes in the heap; nothing inflates a Rice or gzip
  tile into pixels.
- Every cell asks the header for its `TFORMn` again, so large tables are slow,
  and the heap walk visits every cell of every row before the heap has any
  children.
- Columns past 32, axes past 9, `CONTINUE` cards.
- Columns keep `Field::name_from`, one per column, rather than moving to
  `elem_name_from` (S3). A row is 32 fields and not a list because each
  column's type comes from a `TFORMn` card found by its keyword, and a list
  would need that keyword built from `Idx` (`TFORM` and a number, as text),
  which no expression can do. Worth revisiting only if that is added, and it
  would lift the 32-column cap too.

### GRIB

Values only for simple packing (5.0). Complex packing (5.2, 5.3; what GFS
output uses) and JPEG 2000 / PNG sections (5.40, 5.41) keep their bytes.
Grid templates 3.0, 3.20, 3.30, 3.40 and product templates 4.0, 4.1, 4.8
only; anything else is bytes.

### NPY / NPZ

- NPZ reads as a plain ZIP (S6).
- Structured dtype nested and shaped fields, explicit `offsets`.
- More than 4 dimensions read as one run.
- Header keys in a non-numpy order read as one run of text.

### MAT

- Subsystem data (objects, and so MATLAB `string` and `table`) is bytes.
- Sparse row indices and column starts read as numbers, not positions.

### miniSEED

- Steim1/2 read as differences; undoing them into samples is not done.
- miniSEED 3 (FDSN, 2023) is not recognised.
- Steim3 and HGLP encodings are bytes.

### GWF

- One sample. Eleven classes come from FrameL source and have never been
  checked against bytes.
- Compressed FrVect contents are not unpacked.
- Versions 6 and 7 past their structure headers are bytes.

### NetCDF classic

Reads correctly. A file with exactly one record variable writes its records
unpadded and this places them slightly wrong for an odd record width. Corpus:
3 tiny files of the same data.

### Zarr

A ZipStore is recognised and reads as a ZIP (S6). A directory store cannot be
opened at all, since the app opens one file.

## Corpus gaps

Thin enough that a clean sweep says little: NetCDF classic (3 files, 3 KB, one
dataset), HDF4 (2 files, 8 KB), GWF (1 file), CDF (2 files), FITS (3 files).
HDF5's are small synthetic files; nothing from a real instrument.

## Not built

DICOM, NIfTI, SEG-Y, BUFR, Arrow IPC / Feather, BAM / BGZF, ADIOS2 BP, TDMS.

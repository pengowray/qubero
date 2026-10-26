# Handover: reading files from the Vera C. Rubin Observatory

Written 2026-10-21. What Rubin publishes, what Qubero already reads of it, and
the three pieces of work left, each written so it can be handed to a session.

Samples are in `qubero2-extras/rubin`, with `sources.tsv` giving each file's
source URL, licence and SHA-256. They are outside the sample collection on
purpose: the two DP2 images have no licence, and the Avro files would fail the
`samples_real` sweep until an Avro template exists. Move the redistributable
ones into `qubero-samples` as each piece lands (see the licence column).

## What a person with "a file from Rubin" most likely has

1. **An alert** (Avro). Public, no proprietary period, redistributed by brokers.
2. **A science image** from the Butler or the Rubin Science Platform:
   `visit_image`, `difference_image`, `deep_coadd`, `template_coadd`. FITS in
   the LSST Science Pipelines' own layout (afw). DP1 and DP2 need data rights.
3. **A catalog** (Parquet): Object, Source, ForcedSource, DiaSource, Visit.
4. **A raw camera file** (FITS, one detector each). Mostly staff and
   commissioning users.
5. The tail: healsparse maps (FITS), HiPS tiles and MOC (FITS), the Butler
   registry (`gen3.sqlite3`), QuantumGraph files (`qgraph4` magic, LZMA JSON),
   the newer `lsst.images` FITS layout (no sample yet).

## Done

| What | Commit |
|---|---|
| A quoted FITS string whose closing quote is in column 80 failed with "field extends beyond its parent". Rubin's table headers have dozens (`TDOCn`). | 728d1ee7 |
| `HIERARCH` cards read as the long keyword, the value and the comment, and the card is labelled with the long keyword (`ARCHIVE_ID_PSF`, `LSST CALIB RUN BIAS`). The `key` field still says `HIERARCH`, so a lookup by keyword does not find them by their long name yet. | 728d1ee7 |
| Each HDU is labelled with its `EXTNAME`: a visit image reads `[1] IMAGE`, `[2] MASK`, `[3] VARIANCE`, `[4] ARCHIVE_INDEX`, `[11] SkyWcs`. | 728d1ee7 |

Already fine before this, checked on the DP2 visit image: `RICE_ONE` (CFITSIO's
alias for `RICE_1`), `SUBTRACTIVE_DITHER_2` with `ZDITHER0`, `Q` (64-bit)
heap descriptors, `GZIP_2` masks. Every Rubin FITS sample reads with no
errors in any node (`CHECK_TREE_ALL=1 check_tree ../qubero2-extras/rubin - 7`),
except the gzipped LATISS raw (see the unpack cap below).

## Piece 1: Avro, and Rubin alerts

**Done 2026-10-23**, on branch `nonhuman/busy-roentgen-b3d838`: a new `avro`
template (`avro.rs`, `avro_schema.rs`, `avro_alerts.rs`). Both Rubin alert
samples and six generated container files read with every value matching
fastavro 1.13.1 (1,952 values, `crates/core/tests/avro_real.rs`), and the
cutouts open as FITS.

**Two shapes on disk.**

- **Object container file.** `Obj\x01`, a metadata map (`avro.schema` as JSON,
  `avro.codec`), a 16-byte sync marker, then blocks: a count and a byte size
  (both zigzag varint longs), the block's data, the sync marker again.
- **Confluent wire format.** One byte `0x00`, a big-endian int32 schema ID,
  then one Avro record with no schema in the file. This is what is on the
  Kafka stream and in the alert archive (gzipped there, as
  `v2/alerts/{first 6 digits}/{id}.avro.gz`). Schema ID is
  major × 100 + minor, so 0x384 = 900 = schema 9.0, 1101 = 11.1.

**What reads.**

- The header's map, with `avro.schema` read as JSON and `avro.codec` as text.
  Each block's data is a `Ty::Schema` of kind `avro block`, built from the
  codec entry: null, deflate, snappy (with the CRC-32 after each block taken as
  a check over what it unpacks to), zstandard, bzip2, xz. A codec Qubero has no
  decoder for leaves the block as bytes with the codec named.
- Every block's sync marker is checked against the header's (a `Valid::Expr`
  comparing the two 16-byte runs eight bytes at a time with `PeekIn`, since a
  run of bytes is not a number in an expression).
- Records through the schema. Every named type (record, enum, fixed) is a
  `Ty::Schema` keyed by its full name, wherever it is used, so a type that
  names itself reads like any other. Namespaces resolve as the specification
  says, falling back to the bare name. Records carry the schema's `doc`
  strings as field docs; a long with a timestamp logical type gets
  `Time::unix_millis/micros/nanos`. Other logical types read as the type
  underneath, as the specification says a reader that does not know them
  should.
- Arrays and maps in blocks, negative counts with their byte size included,
  ending at the zero block. A union is a structure of `branch` (an enum of the
  branch types) and `value`, typed `null or string` and so on; null is an
  absent field.
- Every non-empty `bytes` value is a `Codec::Stored` run, so it opens as
  whatever it sniffs as. A Rubin cutout opens as FITS this way.
- A framed record's schema comes from the bundled alert schemas: all 93
  `.avsc` files of versions 3.0 to 11.1, byte for byte from `lsst/alert_packet`
  at `1362a8d7f1db4014cbd419390580a707141c683a`, in
  `crates/core/formats-avro/alert_packet` with its `LICENSE` and `COPYRIGHT`.
  `node tools/alert_schemas.mjs` fetches them and writes the table; change its
  `COMMIT` to update. A version is resolved the way `Schema.from_file` does it:
  the `.alert.avsc` file, then the file named after each type it uses and does
  not define. An ID with no bundled schema reads the header and leaves the
  record as bytes, saying which versions Qubero has. The notices entry is in
  `tools/notices-extra.md`, with the GPL-3.0 text.
- Recognition: container files by `Obj\x01`. A framed alert only when the file
  is named `.avro` and the ID after the zero byte is a bundled one
  (`BY_EXTENSION` in `recognise.rs`); the web passes the name already.
- `about.rs`, `web/src/filetype.ts` and `web/src/identity.ts` have Avro.

**Reference values.** `tools/avro_reference.py` (fastavro, cramjam and
backports.zstd in `~/.venvs/qubero-samples`) writes every value of every
record at its path, with logical types switched off. The collection's
`avro/` folder has the two Rubin alerts, six files from its
`tools/make_avro_samples.py` (every Avro type, one codec each, four blocks),
and `reference.tsv`. The two Ampel files in extras also read without errors;
they are not in the collection (not Rubin production data, licence unclear).

**Left.**

- An alert from the archive is `.avro.gz`. The gzip opens, but its contents
  are sniffed by their bytes alone, and a framed alert has no magic, so it
  opens as text. Recognising it needs the name inside the gzip header, or the
  outer name minus `.gz`, to reach `template_for` in `eval/mod.rs`.
- Array and map items are under their block: `prvDiaSources` item 3 is
  `value / [0] / items / [3]`. One flat list across blocks would need a gather
  over every block's items.
- A union is one level deeper than its value in the field tree. The linear
  views fold it (`contents` is `value`), the tree does not.
- A bytes value holding a decimal is a stored run like any other bytes, and
  the decimal is not worked out.
- No Butler-written Parquet catalog yet, so Piece 3 still waits.

## Piece 2: the afw layout inside FITS (images)

**Built 2026-10-23**, on branch `nonhuman/eager-engelbart-642ee1`. Checked
against astropy on `visit_image.fits`, `difference_image.fits`,
`exposure-version-2.fits` and `archiveImportTest.fits` by
`crates/core/tests/afw_real.rs`, whose reference values
`tools/afw_reference.py` writes to `qubero2-extras/rubin/afw_reference.tsv`.

**HDUs.** 0 is an empty primary. 1 to 3 are `IMAGE`, `MASK`, `VARIANCE`
(`EXTTYPE` and `EXTNAME`, `INHERIT = T`). Then `ARCHIVE_INDEX`, then one
`ARCHIVE_DATA` binary table per kind of persisted object. In DP2 the image and
variance are `RICE_ONE` with `SUBTRACTIVE_DITHER_2`, the mask `GZIP_2`; afw
adds its own `UZSCALE` card. All four afw samples have `EXTTYPE = 'MASK'` on
the mask HDU and `EXTTYPE = 'ARCHIVE_INDEX'` on the index HDU, including
`exposure-version-2.fits`, whose HDUs have no `EXTNAME`.

### What reads now

In the tree, with the FITS template:

| What | Code |
|---|---|
| **Long keywords.** A lookup for a keyword longer than eight bytes, or with a space in it, compares the text of each `HIERARCH` card's keyword: `card_at("ARCHIVE_ID_PSF", ...)` finds the card `HIERARCH ARCHIVE_ID_PSF = 417`. An unquoted `HIERARCH` value that starts with a digit, a sign or a decimal point is read with the same fields as a `BZERO` value (`text`, and `parts` with `int`, `frac` and `exp`), so `body.value.parts.int` is the integer 417. | `fits.rs` `card_at`, `fits_cards.rs` `hierarch_value` |
| **Masks.** The data of an HDU whose header says `EXTTYPE = 'MASK'` has the type `Mask planes[]`. Each pixel is a flags value whose bit names come from that header's `MP_<NAME> = <bit>` cards, including `HIERARCH` ones: 17 names in the DP2 visit image, 26 in the difference image, 9 in `exposure-version-2.fits`. A pixel of 32800 shows as `DETECTED\|SPIKE`. | `fits.rs` `mask_pixels`, `fits_afw.rs` `Planes` |
| **Compressed masks.** For a tile-compressed mask, the tile panel shows each decompressed pixel followed by the names of its set bits: `32800 (DETECTED\|SPIKE)`. To find the names, `fits_tile::Cards::parse` now reads `HIERARCH` cards, which it skipped before; it skipped 10 of the visit image's 17 plane names. | `fits_tile.rs` `Cards::mask_planes`, `Tile::text` |
| **The archive index.** The data of an HDU whose header says `EXTTYPE = 'ARCHIVE_INDEX'` has the type `Archive index`. Each row is an `Entry` labelled with the class in its `name` column, so the PSF's row in the visit image is labelled `[417] PiffPsf`. Each row also has a field `component`, which names the part of the image (PSF, SKYWCS, PHOTOCALIB and so on) that the row's object `id` is, according to the primary header's `ARCHIVE_ID_<PART> = <id>` cards. Older files write `PSF_ID = <id>` with "archive ID" in the comment, and those count too. The `component` field is absent for rows whose `id` no such card names, such as the 411 `TransformPoint2ToPoint2` rows. | `fits.rs` `archive_entry`, `fits_afw.rs` `Components` |
| **Archive HDU names.** An HDU with no `EXTNAME` is labelled with its `AR_NAME` card: HDU 5 of `exposure-version-2.fits` is labelled `[5] SkyWcs`. | `fits.rs` `hdu` |
| **Byte arrays in tables.** In a binary table that is not a compressed image, a heap array of bytes (a `PB` or `QB` column) is read by what it starts with. Bytes `0x80` then 2, 3, 4 or 5 are a Python pickle: the array is a stored run (`T::decoded` with `Codec::Stored`), and opening it in its own tab picks the pickle template, as a PNG inside a GRIB message does. In the visit image that is the 117,321-byte PiffPsf. Bytes that are all printable ASCII, tabs and line ends are UTF-8 text: the 411 AST transforms and the YAML summary statistics in HDU 8, and the two AST WCS objects in HDU 11. Any other bytes stay a `u8[]` array. Compressed images' tiles always stay `u8[]`. | `fits.rs` `blob` |

**Lookup cost.** Measured on the visit image's primary header (751 cards) in
a release build. A lookup for a keyword of eight bytes or fewer runs the
same search as before this work: the first such lookup takes 2.0 ms and each
later one 0.075 ms. A lookup for a long keyword searches a second index, of
the `HIERARCH` keywords. The first long-keyword lookup in a header takes
5.0 ms if no lookup has read the header yet, and about 3 ms if a short
lookup already has; each later one takes 0.03 ms.

**Sweeps.** `CHECK_TREE_ALL=1 check_tree <dir> - 7`, release build, before
and after this work: `qubero-samples/fits` reads all 14 files with no errors
both times (1.9 s, then 1.35 s); `qubero2-extras/rubin` has the same one
known error both times, the gzipped LATISS raw over the unpack cap (20.9 s,
then 9.3 s). The visit image alone went from 5.1 s to 1.4 s, because each
text byte array is now one node instead of one node per byte.

**Fixed on the way: scaled images cut a file short.** Before this work, a
`BSCALE` or `BZERO` card written in the standard fixed format, such as
`BSCALE  =                    1 / default scaling factor`, failed to read.
In that format the value field ends one space after the last digit, and the
check for digits after a decimal point read the byte after the end of the
field. The image HDU with that card then failed, and the list of HDUs ended
there without an error: `archiveImportTest.fits` read as 2 of its 6 HDUs, on
main as well. Any FITS file with a scaled image written this way was cut
short at that image.

**IR additions.** No new variants of `Ty` or `Expr`. `Expr::tagged_text` is
a new constructor that makes a tagged lookup with a fixed `Tag::Text` label;
before, the evaluator made a `Tag::Text` only from a `Tag::ComputedText`.
The mask planes and the components are two `Ty::Schema` builders in
`fits_afw.rs`, the same mechanism the ASDF template uses for its blocks. The
mask builder suits any format whose header names the bits of its data.

### Left

- **A link from an index row to the table rows it describes.** An index row
  says which rows hold its object: rows `row0` to `row0 + nrows - 1` of the
  table in HDU `(AR_HDU - 1) + cat.archive`, counting HDUs from 0. `AR_HDU`
  is a card in the primary header and `cat.archive` a column of the row.
  That HDU comes after the index, but an expression in an index row can
  still reach it: from row 0 of the visit image's index,
  `E::start_of(E::elem_field("hdus", E::lit(8), &["cards"]))` comes to
  371,520, where HDU 8's cards start, and a value inside those cards reads
  too (tried and reverted, 2026-10-23). Two ways to build the link:
  1. **As bytes, with what exists today.** A field of the index row that is
     an `At` with `Anchor::Space`, landing at
     `start_of(hdus[n].data) + row0 × NAXIS1`, where `n` is
     `(AR_HDU - 1) + cat.archive` and `NAXIS1` is the target HDU's own row
     width. `AR_HDU` comes from `hdus[0]`'s cards and `NAXIS1` from
     `hdus[n]`'s, both with `E::tagged_in_by` over an
     `E::elem_field("hdus", ..., &["cards"])`, comparing `keynum`. The field
     covers `nrows × NAXIS1` bytes. The cursor can follow it, but the target
     rows show as bytes, not as the target table's columns.
  2. **As typed rows.** Reading the target rows as columns needs a type
     whose expressions are evaluated in the target HDU, because that HDU's
     own cards describe its columns, and a field placed from the index row
     looks up cards in the index HDU. Neither of these exists: an `At` that
     evaluates its inner type in the frame of the HDU it lands in, or an
     overlay node that shows a node already read elsewhere in the tree. The
     origins panel already names, for each heap array, the descriptor that
     placed it, which is close to the second.
  Either way the link also needs `cat.persistable`, which orders the pieces
  of one object across tables (a value of -1 means the object was saved
  with no data), and has to handle one HDU that holds several classes with
  the same columns: HDU 8 of the visit image holds `TransformPoint2ToPoint2`,
  `PiffPsf` and `ExposureSummaryStats`.
- **Masks with `BZERO` or `BSCALE`.** Their pixels keep the reading every
  scaled image has, the stored integer and its scaled value, and their bits
  have no names. `archiveImportTest.fits` has one: an unsigned 16-bit mask,
  stored as signed integers with `BZERO = 32768`, so the top bit of the
  stored integer is the opposite of the top bit of the value. Naming the
  bits of the scaled value needs `Ty::Flags` over a computed number, and
  `Ty::Flags` can only name bits of the bytes it covers.
- **The relations panel says nothing about where the names came from.**
  For a mask pixel or a `component` field it should point at the header
  cards the names were read from. It does not, because `declared_schema`
  in `eval/schema.rs` stops at a `Switch` or `Match`, and both schema nodes
  are inside several (on the table kind, `EXTTYPE`, `BITPIX` and the scaling
  test). A fix: record the schema's kind, walk and key on the node when it
  is resolved, and have `declared_schema` check that record first.
- **A short keyword written as a `HIERARCH` card**, such as
  `HIERARCH PSF_ID = 1`, is not found by `card_at("PSF_ID")`, which
  searches only the eight-byte keywords. afw writes `PSF_ID` as an ordinary
  card, so nothing needs this yet. Falling back to the `HIERARCH` index on
  every failed lookup would build that index in every HDU, since most
  headers lack `BZERO`, `TZEROn` and other keywords the template looks up.
- **Two FITS files in one container can show each other's names.** Each
  schema build is cached under a key made of where the HDU's cards start and
  how long the file is (or the decompressed stream, for a FITS file inside
  a compressed one). Two FITS files with the same length and the same HDU
  layout, read inside one container, get the same key, so the second shows
  the first one's mask plane names or components.
- **Text detection is ASCII only.** A YAML byte array with non-ASCII UTF-8
  in it stays `u8[]`.
- **The tile panel** shows plane names only inside the pixel values. A list
  of all the mask's planes, or of the bits of the pixel under the cursor,
  needs a new field in the panel's data (`ExplainDto::Tile`).
- **Pickles show as text in the tree.** Inside the tree the pickle's stored
  run is read as `DecodedText`; only opening it in its own tab reads it as a
  pickle. Reading it as a pickle in the tree needs the FITS template to
  register the pickle template's `Op` type and its deducer.
- **Coadds** (`deep_coadd`, `template_coadd`): no sample yet. Their
  `COADD_INPUTS` component should get a `component` field like the others.

**Samples.** `visit_image.fits` and `difference_image.fits` (real DP2, no
licence: keep in extras), `exposure-version-2.fits`, `archiveImportTest.fits`,
`ticketdm26260.fits.fz` (afw test data, GPL-3.0-or-later from source headers).

## Piece 3: Parquet catalogs (smallest)

Written by the Butler's `ParquetFormatter`. File-level metadata keys:
`lsst::arrow::rowcount`, `table_meta_yaml` (astropy YAML), `pandas`,
`lsst::arrow::len::<col>`, `table::len::<col>`, `lsst::arrow::shape::<col>`,
`lsst::arrow::astropy_pandas_index`, `ARROW:schema`. Per field: `description`
and `unit`. Snappy, row groups near 1 GB. Schemas are Felis YAML in
`lsst/sdm_schemas`.

No sample has the `lsst::arrow` keys: `testSSObjects.parq` has pandas metadata
only, and the Fink file is a Spark flattening of the alert schema. Without a
Butler-written file there is little to check against, so this waits for one.

## Raw camera files

One detector per file: an empty primary, 16 amplifier HDUs `Segment10` to
`Segment17` then `Segment07` to `Segment00`, each `RICE_1` with 576 × 2048
pixels, then empty `REB_COND` and `CONFIG_COND` tables. These read already.
The obs_lsst test copies are gzipped behind a `.fits` name and open through
the gzip template.

**The 64 MiB unpack cap.** A real raw is 16 × 576 × 2048 × 4 bytes, about
75 MB, past `codec::CAP_BYTES`. So a gzipped raw does not open
(`3018092000065-det000.fits`). The cap is a deliberate guard against zip
bombs. Decided 2026-10-21: leave the cap, and make large decoded spaces lazy
later (unpack only the part being viewed), as a project of its own.

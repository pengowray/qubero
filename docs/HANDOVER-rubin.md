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

**HDUs.** 0 is an empty primary. 1 to 3 are `IMAGE`, `MASK`, `VARIANCE`
(`EXTTYPE` and `EXTNAME`, `INHERIT = T`). Then `ARCHIVE_INDEX`, then one
`ARCHIVE_DATA` binary table per kind of persisted object. In DP2 the image and
variance are `RICE_ONE` with `SUBTRACTIVE_DITHER_2`, the mask `GZIP_2`; afw
adds its own `UZSCALE` card.

**Mask plane names.** The `MASK` header has `MP_<NAME> = <bit>` cards: the DP2
visit image names 17 bits, BAD = 0, SAT = 1, ... UNMASKEDNAN = 16. Reading each
mask pixel as named bits from these cards is the clearest single thing to
show: the header says what every bit of the pixel means.

**The archive.**

- Primary header: `AR_HDU = 5` (1-based, so the index is HDU 4 counting from
  0), and per component two cards with the same archive ID, an old one like
  `HIERARCH PSF_ID` and a new one like `HIERARCH ARCHIVE_ID_PSF`. Components
  seen: SKYWCS, PSF, PHOTOCALIB, DETECTOR, FILTER, VALID_POLYGON, AP_CORR_MAP,
  SUMMARY_STATS, COADD_INPUTS, TRANSMISSION_CURVE.
- Index columns: `id`, `cat.archive`, `cat.persistable`, `row0`, `nrows`
  (all `1J`), `name`, `module` (`64A`). Data HDUs carry `AR_CATN` (their
  position) and `AR_NAME` cards; `EXTNAME` is the last class name.
- An object with ID `n` is every index row whose `id` is `n`: each points at
  the HDU `(AR_HDU − 1) + cat.archive`, rows `row0` to `row0 + nrows`.
  `cat.persistable` orders the pieces of one object across catalogs; −1 means
  saved empty. One HDU can hold several classes when their columns match.
- Blobs in `QB` columns: SkyWcs is AST text starting `1 SkyWcs 1  Begin
  FrameSet`; ExposureSummaryStats is YAML; PiffPsf is a Python pickle,
  protocol 4, which Qubero's pickle reader could open in place.

**Cheap version, no IR change.** Name the mask bits from `MP_` cards; show the
index as a table (it already is one); open each `QB` blob as what it is
(pickle, text). The `HIERARCH` keyword lookup needs the long name to be
findable first: today `tagged_bytes` compares the 8-byte `key`, which is the
word `HIERARCH` for all of them.

**Expensive version.** Link each index row to the rows it points at. Pointing
at the Nth variable-size element of the `hdus` list is the open S5 gap in
`check-ir-structural-gaps` (memory) and the science handover. Read those
before designing it.

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

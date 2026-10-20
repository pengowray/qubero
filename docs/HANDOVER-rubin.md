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

## Piece 1: Avro, and Rubin alerts (recommended first)

Avro is a general format Qubero lacks, and alerts are the Rubin files anyone
can get.

**Two shapes on disk.**

- **Object container file.** `Obj\x01`, a metadata map (`avro.schema` as JSON,
  `avro.codec`), a 16-byte sync marker, then blocks: a count and a byte size
  (both zigzag varint longs), the block's data, the sync marker again. Codecs:
  `null`, `deflate` (raw RFC 1951), `snappy` (each block is followed by a
  4-byte big-endian CRC-32 of the uncompressed data), `zstandard`, `bzip2`,
  `xz`. Rubin's pipeline writes `null`; old 2020 public samples were snappy.
- **Confluent wire format.** One byte `0x00`, a big-endian int32 schema ID,
  then one Avro record with no schema in the file. This is what is on the
  Kafka stream and in the alert archive (gzipped there, as
  `v2/alerts/{first 6 digits}/{id}.avro.gz`). Schema ID is
  major × 100 + minor, so 0x384 = 900 = schema 9.0, 1101 = 11.1.

**Record layout (11.1).** `diaSourceId` (long), `observation_reason` and
`target_name` (`[null, string]`), `diaSource`, then nullable `prvDiaSources`,
`prvDiaForcedSources`, `diaObject`, `ssSource`, `mpc_orbits`, then
`cutoutDifference`, `cutoutScience`, `cutoutTemplate`, each `["null","bytes"]`.

**Cutouts are plain FITS** (not gzipped), written by astropy `CCDData`:
primary float32 image in nJy with a local WCS, `UNCERT` (variance),
`MASKPLANE` (since 2026-10-02), `PSFIMAGE`. 30 to 102 pixels square. In the
real sample each cutout is union branch `02`, a varint length of 34560, then
`SIMPLE`.

**How it maps onto what exists.**

- The block framing is a template like any other. Zigzag varints exist
  (`varintbits.rs`, used by Thrift and Parquet).
- The records are a `Ty::Schema` built from the schema JSON, the way
  `asdf_tree.rs` builds one from the ASDF YAML tree and `arrow_schema.rs` from
  an Arrow schema. Avro types: null, boolean, int/long (zigzag varint), float,
  double (little-endian), bytes/string (long length then bytes), record,
  enum (varint index), array and map (blocks of count then items, a negative
  count followed by a byte size, ending at a zero count), union (varint branch
  index then the value), fixed. Named types can be referred to by name later
  in the schema.
- A cutout opens as FITS the way a GRIB PNG-packed section opens as a PNG:
  `T::decoded(E::Remaining, Codec::Stored, ...)` in
  `grib.rs::png_packed_data`.
- A framed alert has no magic worth sniffing (one zero byte), so it needs the
  `.avro` extension and the Confluent header to check out, like the
  extension-gated entries in `recognise.rs` (`table.iter().find(...)` near
  line 480).

**Decision needed: where a framed alert's schema comes from.** The file names
a schema by number only. Rubin's schemas are in `lsst/alert_packet`, which is
GPL-3.0; Qubero is MIT. Options: bundle them (a licence question for the
owner), fetch them when needed from GitHub or Rubin's schema registry, let the
user drop the `.avsc` beside the alert, or read only the 5-byte header and the
rest as bytes. Object container files carry their own schema and need none of
this.

**Reference values.** No venv has `fastavro` yet. Add it to
`~/.venvs/qubero-samples` and write `tools/avro_reference.py` in the shape of
`tools/asdf_reference.py`.

**Samples.** `7912941781254298.avro` (MIT, real, framed, schema 9.0, three
cutouts) and `fakeAlert.avro` (GPL-3.0, container file, schema 11.1, no
cutouts) can go in the collection. The two Ampel files are not Rubin
production data.

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

**Decision needed: the 64 MiB unpack cap.** A real raw is 16 × 576 × 2048 × 4
bytes, about 75 MB, past `codec::CAP_BYTES`. So a gzipped raw does not open
(`3018092000065-det000.fits`). The cap is a deliberate guard against zip
bombs. Either leave it (gunzip first), raise it, or make large decoded spaces
lazy, which is its own project.

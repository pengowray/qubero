# Handover: radio telescope files

Written 2026-11-25. Which radio telescope formats Qubero reads, how that was
checked, and what is left, in the order it was asked for: PSRFITS search
mode (1-bit and 2-bit), Breakthrough Listen filterbank files in HDF5, SIGPROC
filterbank, GUPPI raw voltages, LOFAR beamformed HDF5.

## Done

| What | Commit |
|---|---|
| A `TFORMn` repeat count of six or more digits (`393216X`, `458752B`) read as five digits and a type letter of `6`, so the `DATA` column of every PSRFITS search-mode file covered no bytes and 96% of the file was a gap in its rows. | dfa34552 |
| The `DATA` cell of a PSRFITS `SUBINT` table reads as a run of `NBITS`-wide samples (1, 2, 4 or 8) when `NBITS * NCHAN * NPOL * NSBLK` is exactly the bits its `TFORMn` counts, first channel in the top bits of a byte. The run is a table, one row per time sample and one column per channel, at `1/TBIN` rows a second, with `NBITS`, `NCHAN`, `NPOL`, `NSBLK`, `TBIN` and `CHAN_BW` as its facts. Any other `DATA` column (fold mode, or cards that do not add up) keeps its `TFORMn` reading. | 70c46e1c |
| `fits_contents` reads `FITSTYPE` and `OBS_MODE` from the primary header, and the toolbar names the file "PSRFITS search mode (filterbank samples) · FITS", or fold mode, or calibration. | 70c46e1c, 3ec159f3 |
| `NBITS`, `NCHAN`, `NPOL` and `NSBLK` read as numbers unless a quote or `*` comes before the comment, so a header that quotes an unused one still reads. | c7c694d5 |
| HDF5 filter 32008 (bitshuffle, with or without its LZ4 blocks) is undone in the chunk panel, as two steps: "bitshuffle LZ4" and "bitshuffle". | cb9beb61 |
| The chunk panel found an HDF5 dataset's datatype only among the messages before the layout message. The Breakthrough Listen files keep it two continuation blocks down, so their chunks unpacked with no element type and no values shown. | 525c7692 |
| `radio_real` checks all of this against astropy and h5py (see below). | c25d2a70 |

### Bit order inside a byte

The PSRFITS definition was not read for this: its page is on
www.atnf.csiro.au, which was not among the hosts this work could fetch from.
What was checked instead is the data. `tools/psrfits_bit_order.py` unpacks
the first 20 rows both ways and compares the correlation of neighbouring
channels inside a byte with the correlation across a byte boundary, which
only agree in the order the samples were packed. Top bits first: 0.1287 and
0.1544 for the Parkes 1-bit file, 0.0829 and 0.0827 for the HTRU 2-bit file.
Low bits first gives 0.0096 and 0.0002 across the boundary. astropy's own
bit-column unpacking (`_unwrapx`) is top bit first too, which is the FITS
standard's order for an `X` column.

### What Qubero did with each real file, before and after

`cover_probe`, bytes of the file the annotation spans name:

| File | Before | After |
|---|---|---|
| `PH0037_01711.sf`, Parkes 1-bit, 26.5 MB | 1,069,824 (4.0%) | 26,530,560 (100%) |
| `bpsr111206_072835_beam10.sf`, HTRU 2-bit, 1.06 GB | 40,125,632 (3.8%) | 1,060,390,080 (100%) |
| GJ699 `.gpuspec.0002.h5`, float32, 200 MB | 199,692,678 (99.99%) | the same; chunks now unpack, with values |
| GJ388 `.gpuspec.8.0001.h5`, uint8, 1.17 GB | 1,169,093,978 (99.999%) | the same; chunks now unpack, with values |

`cover_probe`'s own tree walk stops after 3,000,000 nodes, and one row of
1-bit samples is 393,216 of them, so its tree column (413,374 bytes after)
measures that limit, not the template. `check_tree` reads all four files with
no errors, before and after.

`spans_probe` over the whole 1-bit file after the change: 10,368 spans, no
gaps, and the spans add up to the file's 26,530,560 bytes. The bytes after
the last row (424 in the 1-bit file, 192 in the 2-bit file) are all zero and
are the padding to a 2880-byte block that every FITS data unit has. None of
the 13 Parkes files of that pointing, nor the HTRU file, has a part row
after its `NAXIS2` rows: in each, the bytes after the rows are exactly that
padding.

### How it was checked

`tools/radio_reference.py` writes one fact a line from astropy, h5py and
hdf5plugin; `crates/core/tests/radio_real.rs` reads each fact through the
template and fails on any difference.

- The sample collection: `psrfits/parkes-1bit-2rows.sf` and
  `psrfits/htru-2bit-1row.sf`, cut from the two observations (CC BY 4.0,
  CSIRO), and `hdf5/filterbank-bitshuffle.h5`, made from seeded random
  numbers. `tools/make_radio_samples.py` in the collection writes all three.
  552 facts in `crates/core/tests/fixtures/radio/reference.tsv`.
- Whole files, in `qubero2-extras/radio` (links to the copies in
  space-eels' `data/`, with `sources.tsv`): 4,170 facts in
  `radio_reference.tsv`. For each PSRFITS file: 17 card values, the row
  count, 96 float cells, 1,920 single samples spread over 12 rows and 36
  sums of runs of samples. For each HDF5 file: 14 chunks, each with its
  offset, stored size, unpacked size, the FNV-1a hash of all its unpacked
  bytes and its first four elements as the chunk panel shows them.

The 2-bit reference unpacks top bits first with numpy shifts, so it agrees
with the template by construction on bit order; the correlation check above
is what tests the order.

## Left

### Things to fix in what was done

- The toolbar label and the sample table have not been looked at in the
  app. `tsc` passes; nothing was opened in a browser.

- The table facts are labelled by path, `cards[12].body.value 6.4E-05`, and
  carry no unit. The unit is in the card's comment (`[s]`, `[MHz]`). WAV and
  AIFF facts have no units either, so this is a gap in the facts, not in the
  FITS template. A label read from the card's keyword and a unit read from a
  `[...]` at the front of its comment would fix both.
- The rate is `1/TBIN` truncated to a whole number of samples a second, the
  way `real_count_at` treats every rate. Both Parkes files have whole rates
  (8,000 and 15,625). A file with `TBIN = 4.096E-05` (GBT) would say 24,414
  where the true rate is 24,414.0625.
- The time column of the sample table counts from the start of the `DATA`
  cell, which is one row of `SUBINT`. The row's own start is `OFFS_SUB -
  TSUBINT/2`, in the row's cells.
- `DAT_SCL` and `DAT_OFFS` are shown as columns but not applied to the
  samples. For 1-bit and 2-bit data they are usually 1 and 0.
- No frequency-against-time picture. Nothing in the overview draws a 2-D
  image from a run of samples yet; the table is the view.
- Bitshuffle with zstd blocks (client data compression 3) stops the walk with
  a message. No sample uses it.
- The chunk panel calls `lz4_flex` directly, as it calls `miniz_oxide` for
  deflate. It is a panel step with sizes, not a traced codec: nothing shows
  the LZ4 sequences of a block, or the transposed bit rows.
- `radio_real` reads each whole file into memory, 1.2 GB for the largest.

### SIGPROC filterbank (`.fil`)

Not started, not even from a synthetic file. No sample: every `.fil` the
Breakthrough Listen open data API lists
(`seti.berkeley.edu/opendata/api/query-files`, 10,000 rows) is served from
`blpd0`, `blpd2`, `blpd5`, `blpd6` or `blpd1.ssl.berkeley.edu`, which were
not among the hosts this work could download from. The smallest with data is
5 MB (`guppi_58275_49663_030096_1934-638_B3_0001.0002.fil`). GJ699 has only
HDF5 files there.

The format: a run of keyword records between the strings `HEADER_START` and
`HEADER_END`, each string written as a 4-byte little-endian length and its
bytes. After a keyword comes its value, whose type is fixed by the keyword:
`int32` for `telescope_id`, `machine_id`, `data_type`, `nchans`, `nbits`,
`nifs`, `nbeams`, `ibeam`, `nsamples`, `barycentric`, `pulsarcentric`;
`float64` for `tstart`, `tsamp`, `fch1`, `foff`, `src_raj`, `src_dej`,
`az_start`, `za_start`, `refdm`, `period`; a length-prefixed string for
`source_name` and `rawdatafile`; one byte for `signed`. A keyword the reader
does not know has no length to skip by, so it has to stop there. The samples
follow the header with nothing between: `nchans * nifs` values a time
sample, `nbits` each, as many as the file holds. Below 8 bits the order
inside a byte has to be checked against a real file the way
`psrfits_bit_order.py` checks PSRFITS; sigproc's own `pack`/`unpack` are the
reference to read first.

A template would be a `T::Match` on the keyword text for the value, a list
of records until `HEADER_END`, and the samples as a run with a `TableShape`
like `search_samples` in `fits.rs` (`columns = nchans * nifs`, rate
`1/tsamp`). The sniff is the first 16 bytes: `0c 00 00 00` and
`HEADER_START`.

### GUPPI raw voltages (`.raw`)

Not started; a synthetic fixture was not written either. No sample under
150 MB that this work could fetch: the archive's "baseband data" files are on
`blpd0.ssl.berkeley.edu` too, and the smallest listed for
Voyager 1 is 66 MB (`blc2_2bit_guppi_57396_VOYAGER1_0002.0013.raw`), so a
real one exists if that host is allowed.

The format: blocks, each an ASCII header of 80-byte cards in FITS style
(`KEYWORD = value`) ending with an `END` card, then `BLOCSIZE` bytes of
samples. With `DIRECTIO = 1` the header is padded to a multiple of 512 bytes.
The samples are complex, `NBITS` each for the real and the imaginary part
(8 is usual), ordered channel, then time, then polarisation (from the
format's description, not checked against a file): `OBSNCHAN`
channels of `BLOCSIZE / (OBSNCHAN * NPOL * 2 * NBITS / 8)` time samples. The
cards can be read with `fits_cards::card`, which reads a card on its own;
the block is a structure of `Repeat(card, until END)`, padding, and a run
sized by `BLOCSIZE` found by keyword the way `fits.rs` finds `NAXIS2`.

### LOFAR beamformed HDF5

No sample, so nothing was tried, and what follows is from memory of the
LOFAR documentation, not checked here. A beamformed observation is
an HDF5 file of metadata (groups `SUB_ARRAY_POINTING_nnn/BEAM_nnn/STOKES_n`
with attributes) and, in the LOFAR long-term archive, the samples in a
separate `.raw` file that the HDF5 dataset names as external storage. So the
HDF5 template would read the metadata, and the samples need either the pair
of files opened together or the `.raw` read on its own with the shape from
the HDF5 attributes. A sample pair and the LOFAR ICD for beamformed data
(ICD 3) are what is needed first.

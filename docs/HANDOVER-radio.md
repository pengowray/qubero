# Handover: radio telescope files

Written 2026-11-25, and brought up to date after the SIGPROC and GUPPI work
the same day. Which radio telescope formats Qubero reads, how that was
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
| HDF5 filter 32008 (bitshuffle, with or without its LZ4 blocks) is undone in the chunk panel, as one step: "bitshuffle with lz4", or "bitshuffle" when the blocks are not compressed. It was two steps, the first called "bitshuffle LZ4", until 6ff6e25a. | cb9beb61, 6ff6e25a |
| The chunk panel found an HDF5 dataset's datatype only among the messages before the layout message. The Breakthrough Listen files keep it two continuation blocks down, so their chunks unpacked with no element type and no values shown. | 525c7692 |
| `radio_real` checks all of this against astropy and h5py (see below). | c25d2a70 |
| A table fact carries a label and a unit (`TableFact`, `FactUnit` in `template.rs`). The PSRFITS sample table shows `TBIN 0.000125 s` and `CHAN_BW -3 MHz`, by keyword, with the unit read from the `[...]` at the front of the card's comment; WAV, AIFF and AU sample rates are in Hz. The report shows the same. | 6ff6e25a |
| A table's rate is a real. `1/TBIN` for GBT's 40.96 us is 24,414.0625 rows a second, and a filterbank sampled every 15 s has a rate below one. | 6ff6e25a |
| SIGPROC filterbank (`sigproc.rs`) and GUPPI raw voltages (`guppi.rs`), each sniffed, each a table with facts, and both checked against numpy readers (see below). Toolbar: "SIGPROC filterbank file", "GUPPI raw voltages". | 99b18b1a, e42f4dd6 |
| A table wider than 4,096 columns arrives turned, with `rows` greyed out ("Too many channels to show samples as rows. The limit is 4,096 channels."). The 0002 `.fil` (180,224 columns) threw a RangeError from a spread of every column head. The chunk panel's filter rows are stacked, name above sizes, since `bitshuffle with lz4` and `fletcher32` ran under the sizes. A rate that is not whole says `0.0002926 s apart` rather than `1/3,417.969 s apart`. | f3d35f18 |
| The 200 MB GJ699 `.gpuspec.0002.h5` kept the page busy for minutes after it opened, and going to `2504` took a minute before the offset could even be typed. The listing opened the whole chunk index: 3,500 entries, under the 50,000-item limit, but each in a different 64 KiB chunk of the file, against a cache of 512. Every chunk that landed set off a walk of the listing that dropped another chunk it needed. A structure now arrives closed once the walk has read nodes from 128 chunks (`CHUNK_BUDGET` in `flatten.ts`). In headless Chromium: before, the page did not settle in 300 s; after, the row at `0x2500` is drawn 155 ms after Enter and the page answers again within 1 s, for `0xb001f87` (chunk 3,277) too, and for the 1.17 GB GJ388 file. | see git log |
| Every length of time goes through `web/src/timetext.ts`, which picks ns, µs, ms or s from the value: `TBIN 341.3 ns`, `341.3 ns apart`, `TBIN 64 µs` (the card's `6.4E-05 s` on hover). A whole rate is a time too: `125 µs apart`, not `1/8,000 s apart`. From a minute up, `4 min 12 s`. It replaced the table bar's spacing, the report's `secondsText`, the value plot's axis and hover, the WAV outline's `4:12` and the APNG frame delay. A table's time column takes the unit its last row calls for: `time (ms)` for a PSRFITS row of 64 µs samples. | see git log |
| Columns come in groups: SIGPROC `nifs` and PSRFITS `NPOL` (`ColumnGroups` in `template.rs`). With more than one, a column is `IF 2 channel 5` or `pol 2 channel 5`, and its hover is `Intermediate frequency (IF) 2 of 2, channel 5 of 64`. GUPPI raw headers stay `pol 1 real`, with `Polarisation 1, real part` on hover (`TableShape::titles`). | see git log |
| A table inside lists the template names says which one it is: `samples, block 2 of 128, channel 3 of 64` for GUPPI, `samples, HDU 3 of 3, row 1 of 2` for PSRFITS, in the bar, the tab (`block 2, channel 3`), the hover and the export's file name (`web/src/tableplace.ts`). GUPPI channels and FITS rows were counted as items, since the word was lost on the size window around each one. | see git log |
| The toolbar's file type is written shorter rather than cut (`web/src/labelfit.ts`): up to its middle dot, then without a bracketed remark at its end, then up to its first comma, then the template's own name. `PSRFITS search mode`, `HDF5`. The shortest form sets the label's minimum width. | see git log |

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

`cover_probe` and `spans_probe`, bytes of the file the annotation spans name:

| File | Before | After |
|---|---|---|
| `PH0037_01711.sf`, Parkes 1-bit, 26.5 MB | 1,069,824 (4.0%) | 26,530,560 (100%) |
| `bpsr111206_072835_beam10.sf`, HTRU 2-bit, 1.06 GB | 40,125,632 (3.8%) | 1,060,390,080 (100%) |
| GJ699 `.gpuspec.0002.h5`, float32, 200 MB | 199,692,678 (99.99%) | the same; chunks now unpack, with values |
| GJ388 `.gpuspec.8.0001.h5`, uint8, 1.17 GB | 1,169,093,978 (99.999%) | the same; chunks now unpack, with values |
| `guppi_58326_..._0001.0002.fil`, BL Parkes, 32-bit, 2.2 MB | 0 (no template) | 2,163,083 (100%) |
| `guppi_58275_..._0001.0001.stokesI.8b.fil`, BL Parkes, 8-bit, 7.2 MB | 0 (no template) | 7,209,305 (100%) |
| `guppi_58275_..._0001.0000.fil`, BL Parkes, header only, 395 bytes | 0 (no template) | 395 (100%) |
| `blc2_2bit_guppi_57396_VOYAGER1_0002.0013.raw`, BL GBT, 2-bit, 66 MB | 0 (no template) | 66,138,624 (100%) |

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

SIGPROC and GUPPI, added later, are checked the same way. No third party
reader was used: blimpy, your and sigpyproc are not installed, and PyPI was
not among the hosts that work could download from. `radio_reference.py`
reads both from the published layouts with numpy, written apart from the
templates. For one filterbank there is also a second reference: the HDF5
copy Breakthrough Listen published of the header-only `0000.fil`, whose
attributes h5py reads. All of its 17 values agree, once `src_raj` and
`src_dej` are converted: the `.fil` holds them as `hhmmss.s` and
`ddmmss.s`, the HDF5 copy as hours and degrees.

- Whole files: 843 new facts (header records, where the samples start,
  values, sums, block placement, cards, 2-bit codes) in the four files
  above. With the earlier 4,170, 5,013 facts; none differ.
- Collection: `sigproc/synthetic-32bit-2if.fil` (a part time sample at the
  end), `guppi/synthetic-8bit-directio.raw` (DIRECTIO = 1, the second block
  cut 100 bytes short) and `guppi/synthetic-2bit.raw`, from seeded random
  numbers. 505 new facts; with the earlier 552, 1,057; none differ.

What was checked about the GUPPI layout against the Voyager 1 file, rather
than taken from memory: the samples are channel by channel (the share of
outer 2-bit levels changes smoothly within each 1/64 of a block and jumps
between them), and the first two values of each byte behave as one pair and
the last two as another. `NPKT * PKTSIZE = BLOCSIZE`, so a channel has
`BLOCSIZE * 8 / (OBSNCHAN * NPOL * NBITS)` time samples: `NPOL = 4` already
counts real and imaginary. The earlier version of this note had an extra
factor of 2.

## Left

### Things to fix in what was done


- The `DATA` column's `TUNITn` (`Jy` in both Parkes files) is not shown on
  the sample table, on purpose: the samples are the digitiser's levels until
  `DAT_SCL` and `DAT_OFFS` are applied. Once they are applied, the unit
  belongs in `TableShape::units`.
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

Read; see Done. Left:

- 1, 2 and 4-bit samples stay bytes. No file of those widths was at hand
  (Breakthrough Listen writes 32 and 8), and the order inside a byte has
  to be checked against one, the way `psrfits_bit_order.py` checks PSRFITS.
  sigproc's own `pack`/`unpack` are the reference to read first.
- `npuls` and any keyword not in the lists in `sigproc.rs` ends the header
  there, and the rest of the file reads as `Unread`.
- `FREQUENCY_START` ... `fchannel` ... `FREQUENCY_END` is read as records,
  and the channel frequencies it lists are not used for column names.
- A file of 45,056 channels and 4 IFs is a table of 180,224 columns, named
  `IF 1 channel 1` to `IF 4 channel 45,056`. PSRFITS `NPOL` names them the
  same way, `pol 2 channel 5`. The PSRFITS `POL_TYPE` card (`AABBCRCI`) is
  not read, so a column says `pol 2` and not `BB`. That a SIGPROC time
  sample is every channel of IF 1 and then every channel of IF 2 is from
  memory of sigproc's writer; no reference or real multi-IF file checked it.
- A table's time column changes unit with its length, and a copy or an
  export of the table carries the same unit in its heading: `time (ms)`.
- A table both too wide to draw across (over 4,096 columns) and too long to
  turn (over 1,000 rows) is drawn across, and will be slow or fail.
- `telescope_id` and `machine_id` are numbers, not names.

### GUPPI raw voltages (`.raw`)

Read; see Done. Left:

- The order of 2-bit values inside a byte (top bits first) is taken from
  blimpy's `unpack_2to8`, from memory, and nothing in the data checked it:
  reversing it only swaps which polarisation is which and real with
  imaginary. A file with a known tone in one polarisation would settle it.
- 4-bit samples stay bytes.
- With `DIRECTIO = 1` the data is padded to a multiple of 512 bytes too,
  from memory of rawspec. The synthetic file has a `BLOCSIZE` of 512, so the
  question does not come up in it; no real direct-I/O file was at hand.
- The table is one channel of one block. Nothing joins a channel across
  blocks, and `OVERLAP` samples, repeated at the start of a block, are read
  as samples like any other.
- The codes are shown as codes, 0 to 3. blimpy maps them to +40, +12, -12
  and -40.

### LOFAR beamformed HDF5

No sample on the hosts this work could download from: the Breakthrough
Listen API lists no LOFAR target, and a CSIRO data portal search for LOFAR
returns only Parkes collections. So nothing was tried, and what follows is from memory of the
LOFAR documentation, not checked here. A beamformed observation is
an HDF5 file of metadata (groups `SUB_ARRAY_POINTING_nnn/BEAM_nnn/STOKES_n`
with attributes) and, in the LOFAR long-term archive, the samples in a
separate `.raw` file that the HDF5 dataset names as external storage. So the
HDF5 template would read the metadata, and the samples need either the pair
of files opened together or the `.raw` read on its own with the shape from
the HDF5 attributes. A sample pair and the LOFAR ICD for beamformed data
(ICD 3) are what is needed first.

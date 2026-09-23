# Report view

A reading of one file from start to end: what it is, what it holds, how it is
put together, and what is unusual about it, with figures that explain rather
than decorate. It sits one level of abstraction above the listing. Every claim
in it links down to the bytes it rests on.

This document is the design. It is based on eight reports on real sample
files, each written individually on 2026-09-23 for this design rather than
generated, with the goal of finding what a program can generate without an
LLM. The reports, their notes, the brief the agents
followed, and the shared page kit are in `qubero2-extras/reports/`, outside
this repository, like the 2026-08-29 view mockups:

| Report | File | Size | Template | What it leads with |
|---|---|---|---|---|
| `midi-three-tracks.html` | `midi/format1-smpte-three-tracks.mid` | 182 bytes | `midi` | Five notes in 80 ms, timed in milliseconds, so the tempo event changes nothing |
| `ttf-unknown.html` | `font/qubero-fixture.ttf` | 1,004 bytes | none | The bytes alone find all 10 tables and verify all 10 checksums |
| `jpeg-testorig.html` | `jpeg/libjpeg-turbo-testorig-baseline.jpg` | 5,770 bytes | `jpeg` | 89% is the scan, and every table is a stock table |
| `sqlite-overflow-freelist.html` | `sqlite/page512-overflow-freelist.db` | 37,888 bytes | `sqlite` | Every row spills to overflow pages, and 27 free pages still hold most of the deleted rows |
| `wav-pipistrelle.html` | `wav/xc1060673-kuhls-pipistrelle-data-size-short.wav` | 301,024 bytes | `wav` | A recorder's 980-byte metadata block sits where the samples should start |
| `elf-busybox-x86_64.html` | `elf/busybox-x86_64` | 1,131,168 bytes | `elf` | 402 commands in one static, stripped program, laid out twice |
| `docx-word16-table.html` | `docx/word16-table.docx` | 13,648 bytes | `zip` | 102 bytes of text in 13,648 bytes, and the page redrawn from its XML |
| `png-adam7-16bit.html` | `png/pngsuite-basi6a16-rgba-16bit-interlaced.png` | 4,180 bytes | `png` | 1,024 pixels, and every step from 4,107 compressed bytes to them |

Each report has a `.notes.md` file beside it. For every block of the report,
the notes say what data the block needed, whether Qubero produces that data
today and where, how general the block is, and what a program would need to
generate it. The rest of this document is drawn from those notes. A count such
as "5 of 8" says how many of the eight reports used a block.

## A separate view, not a longer listing

The listing reads a file in file order, one row per field, and that is its
design (see "The listing" in DESIGN.md). The reports do not read in file order.
All eight put the content of the file before its header, and five of the eight
explain the header last, after the parts it describes. All eight open with what
the file holds and the finding that matters most, not with the file's first
field. A listing that reordered itself this way would stop being a listing.

So the report is its own main view, beside Hex, Listing, and Text. It borrows
from the listing rather than replacing it:

- The **outline** (`ListingReport.outline()`) gives the parts the report talks about, and their colors.
- **Record tables** in a report are the listing at the grain of a record: one row per record, its bytes inline, and one sentence for what it says.
- **Byte strips** and hex dumps are the ones the listing and the hex view already draw.
- Every **byte reference** in the report (`0x1c`, "the 4 bytes at 0x2c") moves the cursor there, the way `hexlinks.ts` links do, so one click goes from a claim to its bytes in the hex view or the listing.

Cards like the JPEG ones in `jpegcards.ts` sit on the boundary. They are report
blocks that happen to be drawn in the listing, and the same code can draw them
in both.

## The order a report reads in

Each report chose its own order, and the orders agree more than they differ:

1. **What the file is, and the finding that matters most.** The name, the one
   fact a reader needs first, and the facts table. For the WAV file that is the
   metadata block read as sound. For the SQLite file it is the 27 free pages
   full of deleted rows.
2. **The content.** The picture, the sound, the notes, the glyphs, the rows,
   the commands, the document's page. A reader cannot make sense of a header
   field until they know what it describes.
3. **Where the bytes go.** A ledger of every byte, with each part's share, and
   a map of the whole file.
4. **The structures that produce the content**, largest or most informative
   first: the JPEG scan before its Huffman tables, the SQLite B-tree before
   the database header, the ELF code before the ELF header. Where the content
   is compressed, this is the chain from the compressed bits to the content,
   one stage at a time.
5. **What the file claims against what the bytes show**, where the two differ.
6. **The header and directory**, last.

The TTF report is the exception, and it shows why: for a file with no
template, the directory is the finding, so the evidence for it comes second.

A program can choose most of this order without judgment:

- Step 2 needs a *content role* on the template (see
  [Blocks](#blocks-that-recur)). With none, the step is left out.
- Step 4 is the reverse of the dependency order that `origin.rs` already
  computes. A part whose fields place or count another part (a header, a
  directory, a table of offsets) goes after the part it places. Parts with no
  dependency between them go by size.
- Step 5 lists what `check.rs`, `valid.rs`, and the extent audit (below) find,
  most severe first.

What does need judgment is the lede sentence, and choosing which of several
findings leads. The fallback without an LLM is the facts table with no
sentence above it, and the findings sorted by severity and then by how many
bytes they concern.

## Blocks that recur

Each row is a block that more than one report used.

| Block | Used by | Data it needs | What Qubero has | What is missing |
|---|---|---|---|---|
| Facts table | 8 of 8 | Counts, totals, and identity facts, each with its comparison | Every raw value in the tree. `TableShape.facts` is a precedent for "fields that describe this" | Summary facts on the template, and aggregate expressions to compute them |
| Byte ledger: every byte, grouped, with shares | 8 of 8 | Every span, grouped, with gaps split into zero and nonzero, and padding explained | `kinds.rs` totals by kind and type. Spans in `listing.rs`. `PadTo` padding carries a derived doc | A grouping rule (below), and liveness: whether a unit is reachable |
| The content itself | 8 of 8 | A content role: image, samples at a rate, events in time, outlines, rows, code, a document | Images through `contentcard.ts`. Rows and rates through `TableShape` and the table view | Roles for audio, time-ordered events, vector outlines, and documents. Pixels placed in the image grid |
| One element traced in full | 8 of 8 | One representative record, block, or code, with every bit named | Bits for every field. Deflate traces in `codec/inflate.rs` | A rule for choosing the element. Traces for other codecs |
| Glosses on format terms | 8 of 8 | Per-term text, sometimes with a small figure | `Field.doc` for prose about one field | A place for a format's vocabulary: SMPTE, MCU, freelist, segment |
| Claimed against actual | 8 of 8 | Stored sizes, counts, summaries, and second copies, recomputed or compared | `Valid::Eq(Expr)` and checksums in `check.rs`. `consumed_by` marks every length and count | Aggregates in `Expr`. An extent audit over every length. Equality across records, such as a ZIP local header against its central directory entry |
| Logical against physical | 7 of 8 | For each logical unit, the extents it occupies. For each address, the file offset | `Payload.extents` for SQLite. Decoded spaces in `space.rs`. `placed.rs` | Typed references, and address spaces declared by a table |
| Known values named | 7 of 8 | A registry of standard constants: tables, magic numbers, program lists, epochs, encoders | Enums, `explain.rs` for expected magic, DiE rules for what made a file | A registry keyed by field, value, or byte pattern |
| Record table with one sentence per record | 7 of 8 | A summary line per struct or switch case | `named_by`, `name_from`, and `ComputedText` for single fields | Summary text for a whole record |
| From compressed bits to content, stage by stage | 3 of 8, every report whose content is compressed and decodable | For each stage: its size, and which input produced each output byte | Deflate traces (`codec/inflate.rs`, `traced.rs`), decoded spaces and `Space::map_out`. The DOCX and PNG reports matched Qubero's trace code for code | Traces for other codecs. The PNG unfilter with parameters from the header |
| Wheel-zoomable map with semantic redraw | 5 of 8 | Spans at each depth for a byte range, byte classes, and optionally a memory row | Spans, `overview.rs` byte classes, the hex view | A view that picks the depth from the zoom level |
| Readings that disagree | 6 of 8, in the notes | The strings view, byte classes, and template types, joined over the same bytes | Each reading exists on its own | The join |
| Guesses with a confidence | 1 of 8 as a method, 5 as single inferences | Evidence analyses with a fixed confidence rule | Signature rules and `file(1)` for identification only | The analyses (below) |

### Facts, findings, and the lede

Every report opens with a small table of facts. Each fact is a number with its
comparison: "32 of 182 bytes (18%)", "27 of 74 pages (36%)", "5,770 bytes, 1.36
bits per pixel". A template can declare these as a list of labelled
expressions, the way `TableShape.facts` already lists the fields above a table.
Most need aggregates the IR does not have: a count of note events, the largest
advance width, the number of pages a row spills over.

Findings come from what the core already checks (`check.rs`, `valid.rs`,
`problem.rs`) plus the extent audit. The first finding, by severity, goes in the
lede position.

### Where the bytes go

Every report accounts for every byte, and five of the eight notes files name the
ledger or the map among the three blocks most worth generalizing. It needs one grouping rule and
no judgment:

- Group the bytes of each part by the **variant of the nearest ancestor that is
  an element of a list**: the switch case or enum name that decided the element's
  type. MIDI events group as note, pitch bend, and meta text. SQLite pages group
  as leaf, interior, overflow, and free.
- Count **machinery** separately: every field `consumed_by` another (lengths,
  counts, chunk IDs, pointers).
- Split every **gap** into zero and nonzero bytes. Name padding with its reason
  where `PadTo` gives one.
- Where the template can say whether a unit is reached from the root (see
  [References](#references-are-values-that-point)), split **reachable** from
  **unreachable**. The SQLite report's most important number, 27 free pages
  holding deleted rows, comes from this split.

The ledger is drawn twice: as a table with bytes and share of the file, and as
a stacked bar or map. Hovering a row lights its bytes in the map.

### The whole-file map

Four reports draw the whole file as a map that the reader zooms with the
wheel, and a fifth does the same for the JPEG scan. The map does not magnify:
it redraws. Zoomed out, it shows parts and byte classes. Further in, it shows
fields. Past about 14 pixels a byte, it shows the bytes in hex. The ELF map adds
a second row for memory addresses. The SQLite and ELF maps have preset buttons
for the ranges the report talks about.

Everything the map draws exists: spans at each depth come from `listing.rs`,
and byte classes from `overview.rs`. The new part is a view that chooses the
depth from the zoom level.

### Content

The content comes first or second in every report, and every one has a figure
of it: a piano roll, a picture and its channel planes, a spectrogram, glyph
outlines, the rows of a table, a grid of commands, the document's page redrawn
from its XML. The
content is what a reader opened the file for, and the structure is easier to
follow once they have seen it.

Qubero already shows pictures the browser can decode (`contentcard.ts`) and
tables with a rate (`TableShape`). What is missing is a declared *content role*
per template, so the report knows which figure to draw:

- **Image**: a browser-decodable file, or a decoded raster with width, height,
  and channels.
- **Samples**: a run of numbers at a rate (`TableShape.rate`), drawn as a
  waveform with a min and max pyramid and a spectrogram, both zoomable down to
  single samples.
- **Events in time**: records with a time, either absolute or a delta that
  accumulates, and a time unit expression. MIDI's tick length is
  `1 / (fps × ticks_per_frame)` seconds. `Field.time` today declares an instant,
  a date; this is a duration on an axis.
- **Outlines**: points and contours, for fonts and vector formats.
- **Rows**: the table view's rows.
- **Code**: instructions, with functions found by `elf_disasm.rs`.
- **Document**: text with its structure, read from a format inside the file,
  such as the XML of a DOCX part. Qubero reads each part as one string today.

### Claimed against actual

All eight reports compare a value the file states with the value its bytes
support, or one copy of a value with another. The DOCX report checks that the
local header and the central directory agree on all 11 entries, which today
Qubero cannot do because the directory's offsets point at nothing. Some of these the IR can already say and the templates do not: the WAV
template could declare `Valid::Eq(SpaceSize - 8)` on the RIFF size today. Others
need aggregates: the TTF report recomputes 16 summary fields (the font's
bounding box, the largest advance width, the glyph count) and all of them are a
maximum, a union, or a count over an array.

One check applies to every template without any new declaration, the **extent
audit**. For every field marked `consumed_by`, compare where the length it
states ends with where the next thing starts, or with the end of its parent or
of the file. The WAV report found its main finding this way. The same audit
finds trailing data, truncation, and the GUANO undercount that `stretch_to`
already handles.

The report shows a mismatch as two windows at the same scale: where the size
field says the part ends, and where the bytes say it ends.

### From compressed bits to content

The three reports whose content is compressed and decodable (JPEG, DOCX, and
PNG) each show the chain from the compressed bits to the content, one stage at a
time. The PNG report draws the stages to scale: 4,107 bytes of zlib, 8,252
inflated bytes, seven passes of filtered scanlines, then 1,024 pixels of 8 bytes
each. The DOCX report zooms from the XML text down through the deflate step that
wrote each byte to the single bits that step read, and it traces one paragraph
from its bits to its words. This is the direction in the memory on showing
every transformation step, and for deflate it works today: the DOCX and PNG
agents checked Qubero's own trace against an independent decode, and all 8,088
and 4,696 codes matched.

Two figures carry it. One draws each stage as a bar to scale, with each part's
share. The other joins each code in the compressed stream to the bytes it
produces: a literal is a narrow band that widens at the top, and a copy is a
band that widens at the bottom, with an arc to the bytes it repeats. Both come
from `Trace` and `Space::map_out`. What is missing is traces for codecs other
than deflate, and codecs whose settings come from other fields (see
[Codecs](#known-values-and-parameterized-codecs)).

### Logical against physical

Seven reports draw the same bytes in two arrangements. The ELF report draws the
file's layout above its layout in memory, with the two segments joining them.
The SQLite report draws each row across the pages that hold it. The JPEG report
colors the picture by how many bits each MCU cost. The MIDI report lights a
note's bytes when the reader hovers the note. The TTF report joins each
directory record to the range it describes, and the DOCX report does the same
for the central directory with arcs. The PNG report puts each pixel back in the
grid from the pass that stored it.

The data for most of this exists: paths and extents give the hover links, and
`Payload.extents` gives the SQLite lanes. What the IR cannot say is covered in
[References](#references-are-values-that-point) and
[Address spaces](#address-spaces-declared-by-a-table).

### Known values

Naming a value as a known one turns a table of numbers into a finding. The JPEG
report names both quantization tables as the IJG tables at quality 75 and the
Huffman tables as the examples in Annex K of the JPEG standard, and that is the
evidence for which encoder wrote the file. The MIDI report names General MIDI
programs. The TTF report names the `head` magic number and finds the date's
epoch. The ELF report finds musl. The WAV report identifies a Pettersson D500X
block.

This is a registry of known values: a byte pattern, a value, or a whole table,
with a name and a source. Whole files belong in it too. The JPEG sample turned
out to be byte for byte the `testorig.jpg` of IJG libjpeg 6b (1998), which that
release's own tests expect `jpegtran` to write, and a table of hashes of known
test files gives that kind of provenance with no guessing. It follows the rule set for the `file(1)` and Detect
It Easy databases: interpret, never convert. The same registry holds CRC
polynomials, fixed Huffman codes, default palettes, and color profiles.

### Record tables

A record table is the listing at the grain of a record: time or offset, the
record's bytes inline with their roles styled, and one sentence for what the
record says ("Note-on: C5 (72), velocity 96"). The sentence is a per-struct or
per-case summary expression. ImHex has this as `[[format]]`, and 151 of the 310
`.hexpat` files in ImHex-Patterns use it or one of its variants, 1,021 times in all.
Qubero's converter notes each one and shows the raw value
(`hexpat/lower.rs`), so adding a summary expression to the IR also recovers
what those patterns already say.

The MIDI table also shows what the IR knows and the listing does not show: a
status byte that running status leaves out is drawn dashed, as implied. The IR
already defines that field as "this, or the previous element's", so any field
defined that way can be drawn the same way.

### One element traced in full

Every report picks one element and shows all of its bits: one JPEG block from
its Huffman codes to its pixels, one TTF glyph with every flag bit, the MIDI
`division` field, row 1's cell in the SQLite file, one ELF function prologue,
the first 2 ms of the WAV read from two starting points. The choice was
judgment each time. A rule that works without it: the first element whose
variant is the most common one in the ledger, with a control to step through
the others.

### Glosses

Each report has 10 or more glosses, and most are for a format's concepts,
not for one field: SMPTE time, running status, MCU, chroma subsampling,
freelist, overflow page, segment, position-independent executable. `Field.doc`
holds prose about one field. The template needs a glossary as well: terms with
a title, text, and an optional figure, which field docs and remarks can refer
to. Programmer vocabulary gets no gloss (see the memory on programmer
vocabulary).

### Readings that disagree

Five agents found places where two of Qubero's own readings contradict each
other over the same bytes:

- The byte-class map finds text and zero runs inside what the WAV template reads as `int16` samples.
- The strings view reports 8,727 strings inside ELF `.text`, 11 inside the bat call, and 70 of 71 in the JPEG file that are not text.
- The strings view pairs UTF-16 in the TTF file one byte off.

Joining the strings view and the byte classes against the template's spans
needs no new reading. A string that falls inside a numeric run, or text inside
samples, is either a false hit to suppress or a sign that the template is
reading the wrong bytes. The WAV case is the second kind.

### Guesses with a confidence

The TTF report works out a whole directory-based format from the bytes alone,
with a confidence for each step on a four-level scale: verified (a checksum or
an arithmetic identity holds), strong (several independent facts agree),
plausible, and weak. The evidence analyses are small and general:

- **Tag stride:** four printable bytes repeating at a fixed stride.
- **Search header:** the `(count × size, log2, remainder)` numbers that binary-search tables carry.
- **Offset and length tiling:** number pairs that, read as offset and length, cover the file with no overlap.
- **Checksum search:** known sums and CRCs over each candidate range, retried with one word zeroed.
- **Epoch search:** a number that reads as a plausible date from a common epoch.

The same analyses help a file that has a template: run them against the
template's reading, and a disagreement is a finding. The single inferences in
other reports (the encoder, the libc, the order rows were inserted) follow the
same rule: state the evidence, and say it is an inference.

## What the IR needs

The notes propose many additions. They group into eight, roughly ranked by how many
blocks each one unlocks.

### References are values that point

A page number, a file offset, a memory address, and a table ID are all values
that point at something. The IR reads them as plain integers, so no view can
follow them and no template can say what a thing is from what points at it.
This one gap causes the largest misreadings in the reports:

- SQLite typed a page by its first byte, so four free pages that still start with the leaf type byte read as live table pages, and 12 rows appeared where the database has 8. The freelist half is fixed: the template reads the freelist from the header before the pages, and a page it names is free whatever its first byte says. Overflow pages are still typed by elimination, because a page cannot ask whether a cell on a later page points at it. That needs a reference type and a list typed by what reaches its elements; `sqlite.rs` says so at the top.
- ELF has 2,265 jump-table entries, 402 command table entries, and the entry point, all as numbers, because no address maps to a file offset.
- JPEG components name their quantization and Huffman tables by ID, and the tables sit in separate segments.
- The ZIP central directory's `local_header_offset` points at nothing, so Qubero cannot say that the directory and the local headers describe the same 11 entries, or notice if they disagree.

An addition that says "this value refers to an element of that list, found by
this key" gives the report its logical-against-physical block, the ledger its
reachable split, and every view a link to follow. It is the direction in the
memories on exposing the IR of connections and on fq-style paths.

### Aggregates and summary facts

`count`, `sum`, `min`, `max`, and bounding-box union over a path. They give
the facts table its numbers and `Valid::Eq` its targets (the TTF summary
fields), and they are what the check-IR-gaps memory calls S5-adjacent: reading
over many elements at once. A template-level list of summary facts, each a
label and an expression, uses them.

### Prose in three tiers

- `Field.doc`, which exists: what one field is.
- **Remarks**: a condition and a sentence attached to a field, shown wherever the field is. "The tempo does not affect timing when `division.in_frames` is 1." "A note-on with velocity 0 means note-off."
- **A template glossary**: the format's vocabulary.

### Summary text per record

An expression per struct or switch case that renders the record as one line.
It gives record tables their sentences, gives the listing better row labels,
and recovers the `[[format]]` attributes the ImHex converter drops.

### Content roles

Declared on the template, as `TableShape` is: image, samples, events in time
(with a time unit and a delta or absolute time field), outlines, rows, code.

### Address spaces declared by a table

The ELF program headers map file ranges to memory ranges. The notes propose
`Space::Mapped { rows, file_offset, address, file_len, mem_len, filter }`,
which names the table and the four fields in each row. It gives an
address-to-offset conversion, fields typed as addresses, and the memory row of
the map. `space.rs` today opens spaces for decoded streams only.

### Pixels in the image grid

Deinterlacing a PNG is a permutation, not a codec: pixel (x, y) of an Adam7
image is read from a computed offset in the pass that stored it, so the output
is not in input order and cannot be a `Trace`. The PNG notes propose
`Ty::Raster { width, height, pixel, order }`, with the order `Rows` or `Adam7`,
which reads each pixel at a computed offset the way `Ty::Gather` reads records.
It gives every pixel a path, which the content figure, the pixel walkthrough,
and hover all need.

### Known values and parameterized codecs

The known-values registry is described earlier. For codecs, `Packing` already
carries parameters as expressions (`Lzma1`, `Rar5`), so a JPEG baseline codec
fits the same shape, except that its parameters are whole tables from earlier
segments rather than numbers. Its trace needs steps for DC differences, AC
coefficients, and end of block, and a position map across byte stuffing, where
each `ff 00` in the file is one `ff` in the bit stream. The ELF file's bzip2
streams have no `BZh` signature, and `codec/bzip2.rs` requires one. The PNG
unfilter, `Codec::PngUnfilter`, takes a stride fixed when the template is built,
so the `png` template cannot use it: a PNG's stride comes from its header, and
an interlaced image has a different stride in each pass. The PNG notes propose
`Packing::PngScanlines { width, height, bits_per_pixel, interlace }`, which
unfilters pass by pass and writes one trace block per scanline.

## Interaction

The prototypes settled on the same few rules:

- **Visible on arrival.** Every figure shows its point without a click. This is the rule the listing follows too (see the memory on the listing showing real bytes).
- **Hover for detail.** Exact values, the bytes behind a mark, and the field's path.
- **Wheel to zoom, with a redraw at each level.** Maps, strips, spectrograms, and time axes, down to single bytes, bits, samples, or ticks. Drag pans, and double-click resets. Presets jump to the ranges the text mentions.
- **One click at most** to pin a popup or pick an element.
- **Cross-highlighting.** Hovering a note, a row, a ledger line, or a map cell lights the same thing in every other figure. It needs only paths and extents.
- **Byte references everywhere.** A popup with the bytes on hover. In the app, a click moves the cursor there.
- **Glosses** for format terms only, with a dotted underline and a popup that can hold a small figure.
- **Every figure has a caption that states its point.** The point comes first, in bold, and then how to read the figure.

The kit in `qubero2-extras/reports/kit/` implements the glosses, byte popups,
hex dumps, wheel zoom, tooltips, and the light and dark themes, in about 500
lines of JavaScript and CSS. It is a sketch for the real components, not code to port.

## Order of work

1. **Blocks from data the core already has.** The facts table without
   aggregates, the byte ledger with the grouping rule, the whole-file map with
   semantic zoom, record tables from listing rows, glosses from `Field.doc`,
   findings from `check.rs` and `valid.rs`, the extent audit, and the chain
   from compressed bits to content for deflate streams. This is a report for
   every file with a template, in the order described earlier, with no IR
   change.
2. **Readings that disagree.** Join the strings view and byte classes against
   template spans. It also fixes the strings view's false hits.
3. **Prose and summaries in the IR.** Remarks, the glossary, record summary
   text, and summary facts with aggregates.
4. **Content roles and their figures.** Samples, events in time, outlines.
5. **References and mapped address spaces.** The logical-against-physical
   block, liveness in the ledger, and links from every pointer.
6. **Evidence analyses** for files with no template.
7. **Codec traces and pixel placement**: JPEG baseline, headerless bzip2, PNG
   scanlines with parameters, and `Ty::Raster`.

## Bugs the reports found

The agents checked every number two ways, and these are the disagreements that
are Qubero's. Each is described in the notes file named.

| Where | What | Notes |
|---|---|---|
| `sqlite` template | Free pages that keep the leaf type byte read as live leaves: 9 leaves and 12 rows, where the database has 5 and 8. All 63 other pages are `bytes[]` because overflow pages are typed only when the freelist is empty | `sqlite-overflow-freelist.notes.md` |
| `wav` template | Reads samples from `0x2c` in a D500X recording, so a 980-byte metadata block plays as sound and the last 490 samples become a gap. The RIFF size that overshoots the file by 8 is capped silently. Fixed on 2026-09-24: the block is read as `d500x_metadata` before the samples, the `data` chunk is read as the block plus its size, and a RIFF size past the end of the file is marked invalid | `wav-pipistrelle.notes.md` |
| x86 disassembler | 338 instructions print the 64-bit register where the instruction works on 32 bits (95 `bswap`, 242 `bt`, 1 `movd`). Fixed on 2026-09-24 in the copy of `yaxpeax-x86` in `crates/vendor` | `elf-busybox-x86_64.notes.md` |
| Strings view | UTF-16 read one byte off where big-endian text starts at an odd offset, and the Mac Roman text before it is dropped | `ttf-unknown.notes.md` |
| Strings view | 8,727 hits inside ELF `.text` that are instruction bytes, 11 inside the bat call, and 70 of 71 in the JPEG file | ELF, WAV, and JPEG notes |
| `ksy:ttf` | Unicode range bits named from the wrong end (upstream `.ksy`). `glyf` fails on `.max`. No signature, so it is never sniffed. `ksy_bundled.rs` still lists `ttf` as having no sample | `ttf-unknown.notes.md` |
| `codec/bzip2.rs` | Requires `BZh`, so BusyBox's headerless streams cannot be opened | `elf-busybox-x86_64.notes.md` |
| `midi` template | Running-status events show `status : computed = Int(0)`, which reads as a value the file holds | `midi-three-tracks.notes.md` |
| `png` template | IDAT data and gAMA are plain bytes, so none of the chain past the chunks is reachable. The `p8png` and `p64png` templates already show the decoded shape for non-interlaced images | `png-adam7-16bit.notes.md` |
| `zip` template and `relate.rs` | Central directory offsets are plain numbers. `relations()` shows ZIP64 placeholder arithmetic for an archive with no ZIP64. `origins()` on an entry's data does not name `data_size` as its length | `docx-word16-table.notes.md` |
| Strings view | Does not look inside decoded spaces, so it finds none of the DOCX document's text, and it reads a name length and an extra length as one number | `docx-word16-table.notes.md` |
| Sample collection | `sources.tsv` has no row for `jpeg/libjpeg-turbo-testorig-baseline.jpg` (it is IJG libjpeg 6b's `testorig.jpg`) or for the four `pngsuite-*` PNG files | JPEG and PNG notes |

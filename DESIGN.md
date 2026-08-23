# Qubero design notes

A web hex editor that behaves a bit like a spreadsheet: opens files of any size,
edits bits and bytes nondestructively, and overlays typed, computed structure on
the raw data. Rust core compiled to wasm; TypeScript glue; works on mobile.

## Fixed decisions

### The original file is never loaded into memory
wasm32 linear memory tops out at 4 GiB and phones fall over long before that. The
core reads the original through `Source` (`crates/core/src/source.rs`), currently a
`ChunkStore` of 64 KiB chunks with an LRU cap fed by JS from `Blob.slice()`. A read
that touches an unloaded chunk returns zeros plus a list of missing chunks; the host
fetches them and re-renders. A future backend can use a Worker with OPFS
`FileSystemSyncAccessHandle` for synchronous reads behind the same trait.

### Offsets are bits, everywhere in the core
The piece table (`piece.rs`) stores `(source, bit_off, bit_len)`. Bit 0 of the
document is the MSB of byte 0. Byte-aligned operations take a fast path in
`bits::copy_bits`. Deleting a single bit in the middle of a 4 GiB file is one piece
split, not a rewrite. Retrofitting bit granularity onto a byte rope later would have
been a rewrite, so it is there from day one.

Pieces are a `Vec` with cached prefix offsets (O(log n) lookup, O(n) edit). Swap for
a red-black piece tree when edit counts make that matter; the API does not change.

### Undo is a snapshot of the piece list
The add buffer is append-only and shared by all snapshots, so a snapshot is just the
(small) piece vector. `amend_*` variants fold a write into the previous step, used
when one user action (typing the second hex digit) is two writes.

### Virtual scrolling is custom
Browsers cap element heights around 33M px; a 4 GiB file at 16 bytes per row is
billions of px. So the hex view (`web/src/hexview.ts`) keeps a `topRow`, renders only
the rows that fit, and owns its scrollbar (row <-> offset). This is the documented
exception to "use a library for UI primitives": no virtual-list library survives this.

### Workspace
- `crates/core`: pure Rust, no wasm deps, `cargo test` natively. All logic lives here.
- `crates/wasm`: wasm-bindgen surface only. Offsets cross as `f64` (exact to 2^53).
- `web`: Vite + TS. `npm run wasm` rebuilds the package into `web/src/pkg`.
- `?synthetic=5G` opens a deterministic fake file for large-file testing.

### Templates are expressions, not a static layout
`crates/core/src/template.rs` is the IR: ints/floats of any bit width and endianness,
LEB128, magic, bytes/utf8 with computed length, struct, array with computed count,
repeat-until (end of container, or an element whose field matches bytes), `Sized`
(parse inside an N-byte window) and `Switch` (choose a type by an earlier field).
Expressions are integer arithmetic over earlier fields; a short text or byte field
used in an expression is its bytes as a big-endian number, so a switch can key on
`"IHDR"`.

`eval.rs` evaluates lazily by path with memoised offsets and sizes. Results are a
strict tri-state: value, pending (unloaded chunks, which the host fetches before
re-asking), or error. Zero-filled reads never reach the parser. Invalidation is
coarse (whole memo on any edit), so on a large templated file every keystroke
re-walks the root repeat from offset 0: O(file) per edit. A dependency tracker that
invalidates only the fields that read the edited bytes is the upgrade when that bites.

Built-in templates live in `crates/core/src/formats/` (PNG, wasm, MP4, ID3, WAV,
W4V, MIDI, SQLite), one file per format plus `wasm_opcodes.rs` for the instruction
table. WAV carries the metadata chunks bat recorders write: GUANO (`guan`) as
UTF-8 lines,
and `wamd` as a stream of tagged items whose tag numbers were read out of files
rather than a specification. W4V is the same RIFF container with a format tag of
0x5741, its `data` chunk a run of 392-byte blocks: a predictor, a scale, five
undocumented bytes, and 512 six-bit codes packed MSB first. That layout follows
the reverse-engineered decoder in the batchi project and covers only the six-bit
flavour; wider ones would need the code width read from a sibling chunk, which a
field cannot do.

SQLite reads down to the rows. The header and the page grid are ordinary: the
fields are flat in the root struct so the page size is in scope where the pages
are sized, a page size of 1 (which means 65536, since the field is two bytes) is
a `Switch` because there is no conditional expression, and the run of pages ends
at the end of the file rather than trusting the header's page count, which
legacy files leave stale. A page whose first byte is not a b-tree type reads as
bytes, which is what a freelist, overflow or pointer-map page is.

The cells are what the format cost the IR, and they are in the list below:
`PointerList` places them, `SqliteVarint` measures them, and `Expr::Elem` with
`Expr::Idx` types their columns. Two things a database does are still out of
reach. A payload too big for its page spills onto an overflow page, and how much
stays behind is a formula with a comparison in it, which expressions cannot say;
such a cell reads as an error rather than as the wrong bytes, and a comparison
or a min/max in `Expr` is what would fix it. And page numbers, in an interior
cell or at the end of a spilled one, are read as numbers and not followed: a
b-tree that pointed at its own pages would stop being a tree, and the template
would describe a graph rather than a file.

A text format for templates,
and importers for C structs and bitfields, ASN.1, protobuf, Zig packed structs,
Python pickle and C# StructLayout, are next. Further target formats: zip, rkyv, glTF.
Text encodings live in `text.rs`: UTF-8, ASCII, Latin-1, CP437, UTF-16 either
way, plus two that the bytes settle. Hand-rolled rather than pulled in, against
the usual preference for libraries: `encoding_rs` carries the whole WHATWG set
into a wasm bundle and still lacks CP437, which DOS-era formats are full of.
The CP437 table is generated from Python's codec rather than typed out. JIS is
still to come.

Later additions to the IR, each paying for itself in a format:
* `Enum` names the values of an integer field without changing it, so a switch
  keyed on that field still sees the number. PNG colour types, wasm section ids
  and opcodes, H.264 NAL unit types. Values with no name show the number and are
  flagged. Editing takes the name or any number: a file is allowed to hold a
  value its format never defined.
* `Fixed` is fixed-point, so MP4's 16.16 track width reads 64 rather than
  4194304. Typing a value rounds to the nearest representable step.
* `StrLen` says how a text field ends: a fixed run of bytes, a run whose tail is
  padding (the value stops at the first pad byte, and writing pads back out to
  the field's size, so a name can be shortened without moving the file), or a run
  that ends at a terminator which belongs to the field (a C string, whose length
  cannot change without shifting everything after it; with `or_end`, a field with
  no terminator in it reads to the end of its container and is read-only, which
  is what a last GUANO line without a newline needs). A padded field whose tail
  is not all padding is not editable, since writing what is shown would drop what
  is not. MP4's `hdlr` name and `compressor_name`, PNG's `tEXt` keyword.
* `Encoding` sits beside `StrLen` on a text field. Two of its cases are vague on
  purpose: `Bom` lets a byte-order mark decide (and falls back when there is
  none), `Unknown` reads as UTF-8 when the bytes are valid UTF-8 and Latin-1
  otherwise. Either way the node says what it settled on, so a guess is never
  passed off as fact. A format that names its own encoding in a byte, as ID3
  does, needs nothing new: `Switch` picks the text type from that field.
  Scanning, padding and terminators step in whole code units, so UTF-16 text
  does not stop at the first zero byte of "H"; a mark belongs to the field but
  not to the value, which is why `NodeInfo` carries where the value starts as
  well as how long it is. Text is written back in the encoding it was read in,
  mark included, and a character the encoding cannot hold is refused.
* `Expr::SizeOf` measures an earlier field, which is how a field that runs to the
  end of its container knows where the variable-length one before it stopped.
  `Expr::Remaining` measures the other way, from a field to the end of its
  container: an MP4 box of size 0, an ID3 frame body whichever way its version
  counts, the tail of a RIFF chunk. There is no modulo, so RIFF's pad byte is
  `size - (size / 2) * 2`.
* `Named` looks a type up in `Template::types`, which is what lets an MP4 box
  contain more boxes. Resolution has a 64-hop limit, so an alias that resolves
  to itself errors instead of spinning.
* `PointerList` places its children at offsets held in an earlier array,
  rather than one after another: a b-tree page keeps its cells that way. The
  offsets count from the nearest enclosing `Sized` window, which is the page,
  with an `adjust` for the one page whose offsets count from somewhere else
  (SQLite's page 1 starts 100 bytes into the file but counts from 0). The
  children can be in any order, need not fill the space, and one whose offset
  or contents make no sense is an error on its own rather than one that takes
  the page with it. What no child covers is a gap, which is what free space
  inside a page is. This is the first type whose children are not in file
  order, so `locate` looks at all of them instead of stopping at the first that
  starts too late, and a gap inside one ends where the next child begins rather
  than at the end of the parent.
* `Expr::Elem` reads one element of an earlier array and `Expr::Idx` is the
  index of the element the expression sits in. Together they say "my type is
  the one this list gives for my position", which is how a database row's
  columns are typed: a header of serial types, one per column, read just before
  the columns themselves. The serial types from 12 up are lengths rather than
  names, even for a blob and odd for text; there is no remainder operator, so
  the parity is `s - (s / 2) * 2`, as with RIFF's pad byte.
* `SqliteVarint` is seven bits per byte, most significant group first, up to
  nine bytes, where a ninth byte contributes all eight of its bits. `Vlq` stops
  at four and never does that, so it could not stand in. The value is 64-bit
  two's complement, so a negative row id reads as one, and writing keeps the
  field's size by padding at the front with empty groups.
* `Vlq` is MIDI's variable-length quantity: seven bits per byte, most
  significant group first. LEB128 packs the same seven bits the other way
  round, so it could not stand in. Writing one keeps the field's size by
  padding at the front with 0x80, a group of zero bits that says "more
  follows": redundant, legal, and what stops a delta time moving the track.

A wasm function body reads as a list of instructions: the opcode is an `Enum`
over the byte and its immediate is a `Switch` on that byte. The 0xFD (SIMD) and
0xFE (thread) prefixes read their sub-opcode but not its immediates, so a body
using them desyncs from that point; the enclosing `Sized` window keeps it inside
one function.

### Editing a typed field writes only that field
`crates/core/src/encode.rs` is the inverse of the readers in `eval.rs`: text in,
MSB-first packed bits out, exactly the field's current size. `Evaluator::prepare_write`
resolves the field, encodes, and hands back the bits and offset; it reports `Pending`
like a read, because working out where a field starts can touch unloaded chunks. The
host applies the write and invalidates the memo.

Keeping the size fixed is the point: a shorter or longer value would shift the rest of
the file. So LEB128 pads out with redundant continuation bytes (legal, and what wasm
tools do), and text and byte fields must be typed at their exact length. Growing a
field is a structural edit and waits for the redundant-editing work below.

The type table (`web/src/typetable.ts`) edits values in place with this. It rebuilds
its rows from scratch on every document change, so the open input is re-created each
render with its text and caret restored; a committed edit can restructure the tree
(change a count, flip a switch) and the edited row may simply not exist afterwards.
The inspector's per-type lenses are the same idea in TS, but they cannot serve the
table: they are byte-aligned and `DataView`-sized, while a template field can be three
bits at an odd offset. Core owns encoding; the inspector's lenses stay where they are
until the redundant-editing work gives both sides one model.

### One position, three views
The hex cursor is a bit position, and it is what the views agree on.
`Evaluator::locate` walks the template down to the deepest field covering a bit,
so moving the cursor selects that field in the table and marks its bits in the
hex view; picking a field in the table or in the inspector's trail moves the
cursor to its first bit. `web/src/main.ts` owns that loop and holds a `picking`
flag so it does not chase its own tail.

The inspector reads from the cursor's bit, not its byte, so its integer and float
rows show what an unaligned read would give. Its first mode ("Field") shows what
the template says is there instead: the trail of enclosing structures, the value
(editable, sub-byte fields included), and the type, offset and size.

The panel reads a long value from the document rather than from the node, whose
`Value::Bytes` carries only a 16-byte preview. That is why the core's text and
byte edit limit is 4 KiB (`encode::EDIT_LIMIT_BYTES`) while the type table keeps
its own 16-byte one: the table's Value column shows a preview, and writing back
a preview would replace the part it elided. A text field is decoded strictly, so
invalid bytes are shown as hex and not editable there; a lossy decode would let
one replacement character be written back over three valid-length bad bytes.

`locate` walks a repeat to find an element, so on a large templated file it costs
what displaying it costs. The memo makes the next call cheap until the next edit,
which is the same coarse-invalidation limit described above.

### The field column
The hex view's right-hand column shows either the bytes as text or, with a
template, what each byte is: every field tinted where it sits, and its name and
value on a chip beside the row it starts on. Clicking a chip selects the field
in all three views. The point is to read a file's structure without clicking
through it field by field, so a template being selected now shows the field
column by default; the text column leads when no template is set, and each of
those two states remembers what the user last chose for it.

`Evaluator::spans` feeds it: one call per screenful rather than one per field.
It walks `locate` forwards from the first bit on screen, and does two things
that are not one field each. Slack inside a structure comes back as a gap,
since `locate` answers with the enclosing composite when no child covers a bit,
and reporting that composite would mislabel everything before it. And a long
run of plain numbers comes back as the run: W4V's 512 six-bit codes would
otherwise be 512 entries saying `[0]`, `[1]`, `[2]`, which is less information
than one saying `codes  512 values`. Text repeats are not collapsed, because
GUANO lines are each worth reading.

Colour comes from the field's path, not from its position on screen, so
scrolling never repaints the file in different colours. Six hues, and the name
is always on the chip, so colour is never the only signal. Selection and the
cursor are painted after the tints and keep the upper hand where they overlap;
the tints themselves are deliberately weak against the background (about 1.6:1)
because they sit under hex digits that have to stay readable.

How many chips fit is worked out from the text before any are drawn, from the
column width measured on the previous frame, and what is left over is counted
on a `+N` chip rather than quietly cut off. The same applies to the 600-field
limit on one screenful, which says so on the last row.

Both side panels fold to a title bar that still names them: the bottom one to a
strip, the right one to a narrow vertical tab. The bar is also where those two
panels got names, which they did not have before.

### Sub-byte fields in the hex view
A field of three bits that straddles two bytes is normal in these formats, so the
byte-granular highlight was not enough. A fully covered byte keeps the block
highlight; a partly covered one gets a bar under exactly the bits in the field
(bit 0 leftmost, matching the reading order the core uses). The text column
cannot show part of a byte, so it is tinted more faintly there instead of lying.

Binary mode (`web/src/hexview.ts`) renders each byte as its eight bits, the
cursor addresses one of them, `0`/`1` overwrites or inserts a bit, and Delete and
Backspace remove one. Bytes are split into per-bit spans only where the cursor or
a partial highlight needs it. Selecting binary drops the row width to 8 bytes,
since eight digits per byte is wide.

### Saving
`save.rs` turns the piece list into runs. The host composes a `Blob` from lazy slices
of the original plus add-buffer bytes; only bit-unaligned stretches are read through
the core. Written to a new file via `showSaveFilePicker` where available, otherwise a
download. Note: a bit-level insert or delete shifts everything after it, so the rest
of the file is rewritten on save. That is inherent; the UI should show progress.

### Opening a file
Dropping a file anywhere on the page opens it, and an overlay says so while the
drag is over the window: `Drop to open`, and what letting go costs, which is
that the open file closes. Not "replaces", which during a drag reads as
overwriting that file on disk, and nothing here ever writes to the original.
The overlay appears only when the drag carries files, so dragging text across
the page is left alone, and it counts dragenter against dragleave, since
crossing into a child element counts as leaving its parent.

Edits live only in the tab: `Save as` writes a copy, so nothing on disk holds
them. Opening a second file therefore asks before discarding them, whether it
arrives by drop or by the Open button.

### What a type permits
An enum knows the values it names, a magic field knows the bytes it wanted, and
a `Flags` field knows what each of its bits means. None of that is in the value,
so `Evaluator::explain` returns it separately and the At cursor panel shows it
under the editor: one section, three renderings, absent for a type whose value
already says everything. For the two editable kinds it is also the input, since
clicking a named value or a named bit beats retyping a number.

`Ty::Flags` names bits from the least significant up, which is how every format
that has them numbers them. A bit with no name is still listed: a set bit the
format does not describe is the kind of thing worth noticing, not hiding.
Writing a flags field writes the number underneath, because typing a name would
mean deciding whether it replaced the other bits or joined them.

### PE
The DOS header, the PE header, the optional header and the section table.
Three things in it are worth knowing, because each is a thing the IR had to be
able to say. The stub between the DOS header and the real one has a length of
`pe_header_offset - 64`, and since a length expression can only name a field
beside it or above it, never one inside a sibling, the stub belongs to the DOS
header rather than sitting next to it. The optional header comes in a 32-bit
and a 64-bit shape told apart by its own first two bytes, so `Switch` picks
between them from a field inside the struct being switched. And a data
directory entry is an address and a length with nothing saying which directory
it is: position decides, so `Switch` on `Expr::idx()` gives entry three the
type name `exception`.

Sniffing it needs more than a magic number. A DOS executable and a Windows one
both start `MZ`; only a `PE  ` at the offset held at 0x3c separates them, so
`sniff` is given 1 KiB rather than 64 bytes. A file whose header sits past that
is left unclaimed, since claiming it would put this template on every DOS
program ever written.

Not yet described: the section contents, so imports, exports, resources and
relocations are all gaps. The characteristics fields are numbers, and want the
named-bit type the IR does not have.

### What made a file
A second database answers a different question. Where `file(1)` says what
format a file is, the Detect It Easy signature rules say what tool produced it:
which packer, which compiler, which protector. That is what someone opens a DOS
executable to find out.

A rule is a small JavaScript program. `diescript.rs` reads the ones that say
everything in their own text, which is a test on a byte pattern and some
assignments, and counts the rest by reason. Of the 596 DOS rules shipped, 494
are read; every one of them recognises bytes built to its own pattern. The
pattern language is implemented from its definition in the engine's `xbinary.h`
rather than from the shapes these rules happen to use, because the same engine
is a shared submodule across the author's other detectors.

Two anchors. A `.COM` is loaded flat, so its rules test from the first byte; an
`MZ` executable's test from the instruction the loader jumps to, worked out from
the header. An entry point outside the file, or a packer's negative code
segment, means the rule is skipped rather than tested somewhere wrong.

Which bundle to fetch comes from the file: `msdos.sig` for anything starting
`MZ`, `com.sig` for a small file nothing else could name, since a `.COM` has no
header to declare itself and the format's own 64 KiB limit is the only
constraint there is.

Nothing is converted. `tools/die.mjs` fetches a pinned commit and concatenates
the rule files byte for byte, author credits included, with a marker line before
each. Updating is bumping the hash and running it again, which is the whole
point: a database that had been rewritten into another format would need its
changes merged by hand every time upstream moved.

### Naming a file
Every file is identified using the rule database of the `file` command, through
the `pure-magic` crate, and the rule's own sentence goes in the toolbar beside
the file's name. A format with a template gets one too: `PNG image data, 8 x 4,
8-bit/color RGBA, non-interlaced` says things the template does not. Only a file
without a template is kept waiting for it, so a templated file says nothing
until it knows rather than flashing a progress message over a file it can
already lay out.

The rules and the engine that runs them come to 1.7 MB of wasm, more than five
times the editor itself, so they are their own module (`crates/magic`, built to
`web/src/pkg-magic`) rather than part of the editor's. `doc.ts` imports it on
two occasions: when `formats::sniff` has come up empty and a file needs naming
now, and speculatively once a file is open and the browser is idle, since the
next file dropped may be one no template covers. The speculative fetch stops on
a connection that would rather not have it, meaning Save Data or anything the
browser rates below 4G; those sessions fetch on demand instead, and the toolbar
says `Identifying file type...` while they wait. Identifying against a database
already in memory takes tens of milliseconds, under the 300 ms that message
waits for, so a prefetched session never shows it.

Rules count offsets from the start of a file, so `identify` hands them the
first 64 KiB and nothing else. Rules that search rather than test a fixed
offset see only that far, and the handful that measure from the end of a file
cannot be answered at all. The database's own last-resort answers (`data`,
`ASCII text`) are dropped: they say less than the hex view already shows.

The rule that named the format also says where the format's signature is, and
that much is expressible: `magicrule.rs` reads the rule file and turns a
level-0 test of fixed bytes at a fixed offset into a one-field template. The
Fields table then has a row, the hex view tints those bytes, and the select
says `Template: zip (signature only)` so nobody mistakes it for a real one.
Everything after the signature is left undescribed rather than covered by a
catch-all, because the rule says nothing about it and the annotation column
already shows what no field covers.

Of the 349 rule files shipped, 2,980 level-0 rules pin fixed bytes at a fixed
place and 2,590 come back exactly right. A file matching two rules gets no
template at all: a template that contradicts the name is worse than none.

The parser is in core, not in the magic module, because the two wasm modules
have separate heaps and a `Template` built in one cannot be handed to the
other. The rule text crosses the boundary instead: the magic module names the
rule file, `doc.ts` fetches it from `web/public/magdir` (generated by
`tools/magdir.mjs` from the same crate the compiled database came from), and
core turns it into a template.

Licences travel with the code, so `tools/notices.mjs` regenerates
`THIRD-PARTY-NOTICES.md` from `cargo metadata`; run it after changing
dependencies. It lists only the licence arm actually taken, since reproducing
the GPL under a crate taken under BSD terms would claim otherwise.

## Roadmap (not yet built)

### Resilient redundant editing
Two fields derived from the same bytes, or two bytes ranges that must agree
(seconds vs minutes, a length field and the array it describes). Model as pairs of
invertible expressions (bidirectional lenses): each typed field has `decode(bytes)`
and `encode(value) -> bytes`; `eval.rs` and `encode.rs` are those two halves already,
and the inspector rows have the same shape. Editing either side writes through its `encode`; dependent fields re-evaluate. When an edit
would make a constraint unsatisfiable, say so rather than silently picking a side.

### Known gaps
Bit order is MSB-first everywhere: bit 0 of a byte is its top bit, and a field
narrower than a byte is packed big-endian, with `endian` on such a field silently
ignored. Formats that pack the other way (DEFLATE, Zig packed structs) need an
LSB-first order on the field type, which is a real addition to the IR rather than
a display option.

MP4: sample tables past `stsd` are bytes. An SPS or PPS is exp-golomb coded,
which the IR cannot describe, so a NAL unit stops at its header bits. An H.264 Annex B stream (start codes rather
than lengths) needs a "scan until these bytes" primitive that does not exist.

W4V covers the six-bit flavour only, and `.wac` is not read at all.

MIDI running status is the sharpest gap, because it is the normal case rather
than a corner one: a message may leave its status byte out and mean "the same
as the last one", and every one of the three MIDI files Windows ships hits that
within three events, so the rest of each track reads as bytes. Following it
needs three small additions that no format has needed yet, and which would
serve any format that carries state between elements: an expression that reads
the next bytes without consuming them (so a field can exist only when the byte
says so), one that reads a field of the previous element of a repeat, and a
zero-width field whose value is an expression. The last event's status would
then be `Or(status, Prev(effective_status))`, since a real status byte is never
zero and an absent one is a field of no bits. Element `n` would depend on
`n - 1`, so it also needs the evaluator to work forwards rather than recurse.

A field the panel will not let you edit says why only when you try to: the box
is simply disabled until then, because the reasons live in the write path.

A chip in the field column says `body` where the useful name is in a sibling:
a RIFF chunk is identified by its `id` field, not by the name of the field
holding its contents. Nothing generic can know which sibling that is, and
guessing at the first primitive child works for RIFF and fails on PNG, where
the length comes before the type. The fix is for a struct to be able to declare
which of its fields names it, which is a small addition to the IR and is worth
doing when the template text format lands.

`Value::Magic` carries only whether the bytes matched, so a mismatch reads
`does not match` without saying what was expected. The expected bytes are in
the template and could be carried with the value.

Save shows no progress while rewriting bit-shifted stretches. The type table has no
keyboard navigation between rows, so a value cell is reached by clicking or tabbing.
Text fields in the type table are displayed through `from_utf8_lossy`, so invalid
bytes show as U+FFFD there; the field panel decodes strictly instead and refuses
to edit what it cannot decode.

### Later
A magic field that does not match could offer to write the bytes the format
wanted. Deliberately not built: `encode::editable` refuses to write a magic
field at all, and one narrow exception to that is a decision worth taking on
purpose rather than in passing.

Search (bytes, text, regex) streaming over chunks. Selection ranges, copy/paste.
Bit-level cursor mode in the UI. Column/width presets. Worker-side core so the main
thread never blocks on reads.

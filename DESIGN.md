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
W4V, MIDI, SQLite, PE, MS-DOS), one file per format plus `wasm_opcodes.rs` for the
instruction table. WAV carries the metadata chunks bat recorders write: GUANO (`guan`) as
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

### One position, four views
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

### The listing
The hex view answers "what is at this address" and the field tree answers "how
is this file put together". Neither reads a file straight through, which is what
a listing is for: one row per field, in file order, with its offset, its own
bytes, and what those bytes say. It takes the main pane rather than sitting
beside the hex rows, since it carries its own bytes column and two of those
would say the same thing twice.

Its scroll position is a bit offset rather than a row number, because nothing
can know how many rows a file has without walking all of it, and `spans` is
windowed by bit range, which is the same shape. Both directions scroll by
counting fields rather than rows on screen, so a notch down and a notch up land
back where they started: how many headings a screenful carries depends on where
it starts, and that is not the same going the other way. Scrolling back has to
ask for a window that reaches the current top row rather than one that merely
starts before it, since a fixed number of fields from further back returns the
first of them and the wanted ones are the last.

Headings come from the trail of enclosing structures. Entering several at once
is one heading rather than one per level, because five rows reading `sections[9]`,
`body`, `entries[0]`, `body`, `code` push the fields off the screen to say what
one row can say.

The bytes are shown as text as well, but only where they read as text: three
bytes or more, and mostly printable. A one-byte count of 65 beside an `A`
invites reading a number as a letter.

### The overview
A sidebar down the left of both the hex grid and the listing, describing the
file before either of them is read: its size, what the identification made of
it, a map of the whole file, the top-level regions, and a sentence for
anything that stands out. The aim is that what a reader would find by paging
through the whole file (the last half being zeros, a compressed middle) is on
screen before they page anywhere. It belongs to neither view because "what is
in this file" is the same question from both, and it starts folded away
because filling it in reads the whole file.

The map divides the file into equal buckets, each a power of two of bytes so a
cell stands for a round number, and classifies each bucket's bytes: all zero,
one repeated byte, mostly printable, ordinary data, or high entropy. The scan
behind it lives in `core/src/overview.rs` and runs the way a search does, a
bounded window per step answering with the chunks it needs, because it reads
the whole file and the file may not be here yet. The classes cross to the UI
as one digit per bucket; runs, percentages and sentences are worked out there.
Because the map answers from the bytes alone, it is the one part of the app
that says something about a file no template covers.

An edit throws the scan away and the next step starts it over; one pass over
the file is what the feature costs, so it only runs while the sidebar is
unfolded. The stepping loop does as many steps as a frame allows rather than
one per timeout, because a background tab's chained timeouts are throttled to
a crawl.

A cell of either map is a stretch of the file rather than a place in it, so
picking one selects those bytes as well as moving the cursor to their front.
The panel at the cursor then reads the selection as a number, which is most of
what picking a few bytes out of a map is for.

A bucket is judged as a whole, so a cell says nothing about what is inside it:
the first cell of a model file reads as high entropy although it is the header
and its strings, because the weights that follow fill most of it. Picking a
cell scans that block on its own, at a resolution the block's own size allows,
and reports the block's entropy against the most a block that long could
reach, how many byte values appear, and the commonest of them. That pair is
the honest form: 7.9 out of 8 means dense, 7.9 out of 7.9 means only that
there are few bytes here.

The block view can also measure only the bytes no field describes, which is
where the byte classes still have something to say about a file a template
already covers. The stretches come from `spans`, whose gaps are exact, and
that is affordable over one block where it would not be over a whole file;
picking one measures it on its own rather than quoting the block's numbers for
it.

The region list is the template's top-level children, with runs of three or
more plain fields folded into one row so a header's bookkeeping does not
outnumber the parts with any size to them; structures keep their rows however
small. Hovering a region shows where it sits on the map; picking one moves the
cursor the way picking a row does. Next, per the roadmap: carrying each
format's own units upward (pages, tensors, tables, functions) so the regions
read in the document's terms rather than the template's, and coverage at
whole-file resolution so the map itself can show what no field describes.

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

### Selecting bytes, and which caret says what
A selection is an anchor and a focus, both absolute bits, so it composes with
everything else here that counts in bits. Dragging, shift-clicking and the
shifted movement keys all move the focus; a plain click clears it. Both ends
move as a drag crosses its anchor, so the byte pressed on and the byte under
the pointer are both always in, which is what a hex editor does and not what a
text editor does. Drawing is clipped per visible byte, so selecting a whole
file costs what one row costs.

The selection is deliberately not the field highlight. That one is
accent-coloured and comes from the template; this one is a neutral wash and
comes from the reader, and a byte that is both keeps the field's underline
over the wash. The pane the cursor is in draws it strongly and the other draws
it weakly, so both say what is selected and only one claims the focus.

Shift+Home and shift+End used to mean the start and end of the file. They now
extend within the row, because shift means extend everywhere else, and the
file ends moved to Ctrl+Home and Ctrl+End.

The panel at the cursor reads whatever is selected as one number: unsigned,
signed as two's complement over its own width, and hex, with the bytes also
reversed where the selection is whole bytes lying together and a format could
have stored it the other way round. It is a `bigint`, so a sixteen-byte
selection reads exactly rather than through a float. Bits are taken in file
order, MSB first inside each byte, which makes a selection that does not fill
whole bytes read as a number too: four bits of a header are four bits.

The reading takes a list of runs rather than one, because a value a format
does not keep in one piece is several runs of bits, and putting those back
together is exactly what a reader needs a number for. Nothing in the interface
makes a selection of more than one run yet: the hex grid has one anchor and
one focus. The reading is where that gap will close.

Each reading keeps to one line and is cut short rather than wrapped, since a
sixteen-byte selection is thirty-nine digits and five readings of it would
otherwise fill the panel. Under the pointer, or under the keyboard, two buttons
appear over the tail of the number: Copy takes the whole of it, which is the
answer to a number too long to read, and Edit opens it for typing. They sit
over the number rather than beside it so that it does not get shorter when the
pointer arrives.

Every reading is two-way. Typing a number into one writes it back over the
selected bits: signed accepts a negative and stores two's complement, hex takes
digits with or without `0x`, and the reversed readings write the bytes the
other way round. A value that does not fit says what the range is instead of
being clamped or truncated. Writing is one undo step even where the selection
is several runs, because the runs are one value.

Past 1,024 bytes the number rows are simply absent, leaving the length. Nobody
selects a thousand bytes meaning to read them as one integer, so there is
nothing to explain, and the same limit is what stops selecting half a file from
locking the page up.

Insert mode draws a two-pixel bar on the leading edge of the cursor cell
instead of the overwrite block, in both panes and in binary mode. Which of the
two modes a keystroke is about to do is worth seeing without reading the
status bar, and a block over a byte is the usual way of saying it will be
replaced.

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

### Search
A search is a series of bounded steps rather than one scan. The file may be
larger than memory and its bytes arrive a chunk at a time, so a call that ran
to the end would either block for minutes or read bytes that are not there. One
step reads a window and answers with a match, with the chunks it needs, or with
where to carry on; the browser drives that loop on a frame budget, so a scan
over gigabytes still repaints and still takes a click to stop.

Windows overlap, or a match lying across a join would vanish. A literal
overlaps by one byte less than itself. A pattern has no length to go on and
overlaps by four kilobytes, which is the one limit here: a regex match longer
than that and lying across a join is missed, and that is written down rather
than hidden.

Offsets are bytes. Everything else in the core is bits and this is the
deliberate exception: a needle that could start at any bit would match noise in
most files.

`^` and `$` are refused with a reason and a way round. A window is not a line
and not the file, so they would match wherever the search happened to stop
reading, and quietly finding the wrong thing is worse than saying no.

Case folding is ASCII. Matching E-acute to e-acute means knowing the encoding,
and a hex editor does not know what encoding a stretch of a file is in.

Replacing every match is one thing the user did, so `Document::begin_batch`
folds the edits into one undo step. A batch that changes nothing leaves no step
behind, since the snapshot is taken at the first edit rather than when the
batch opens.

regex-automata is the core's only dependency, and it costs 537 KB of the
module. The Unicode tables are left out, which is most of a regex engine's
weight and no loss over bytes. The signature database is already a module of
its own fetched on first use, and doing the same for the regex engine is the
obvious next move: the main module would hand it windows rather than owning the
search.

### A field that says "the same as the last one"
MIDI running status is what this is for. A message may leave its status byte
out and mean the same status as the message before it, which most files written
by a sequencer do, so it is the normal case and not a corner one: the three
files Windows ships have 24,420 events between them that leave it out.

Three additions carry it, and none of them is about MIDI. `Expr::Peek(bits)`
reads where a field starts without taking the bits, so a field can exist only
when the byte says it does: `Peek(8) / 128` is 1 for a status byte and 0 for a
data byte, and the switch already there does the rest. `Expr::Prev(name)` is
field `name` of the element before this one in the nearest enclosing list, and
zero outside one. `Expr::Or(a, b)` is the first of the two that is not zero.
`Ty::Computed(expr)` is a field of no bits whose value is worked out, so
`Or(status, Prev(effective_status))` is a field and not a special case. It is
not editable, because there is nothing in the file to write.

Element `n` asks element `n - 1`, so a computed value is kept on the resolved
node. Without that a track of ten thousand events is ten thousand frames deep,
which is a stack overflow rather than a slow answer. The elements of a list are
already resolved in order, so every one of those lookups is a memo hit.

The spec says a system message cancels running status; this carries it through
one instead, which is what lenient sequencers accept. It can only misread a
file that is already invalid, where the alternative was to stop reading valid
ones.

### A structure that says which field names it
A RIFF chunk is identified by its `id`, a PNG chunk by its `type`, a wasm
section by its `id`. Nothing generic can work that out: guessing at the first
primitive child works for RIFF and fails on PNG, where the length comes before
the type. So a structure declares it. `named_by` is the field whose value names
the structure, and `contents` is the field that is merely what it holds.

A node is then labelled `[9] code` rather than `[9]`: the index says which of
thirteen, and the name says which one it is, and both are worth having when two
of the thirteen are custom sections. `contents` drops a step from the trail the
linear views build their headings from, since `sections[9] code, body` says
nothing `sections[9] code` did not.

The label is worked out where a node is read rather than where it is resolved,
because it means reading a sibling and resolving has to stay cheap. A naming
field that has not streamed in yet leaves the node with the name it had.

### A structure that reads on one row
`StructDef::inline` says that a structure is one thing rather than several. A
wasm instruction is an opcode and its immediate, and an `op` row followed by an
`imm` row says less than one row saying `local.get 0`.

Only the linear views honour it. `Evaluator::spans` trims a located path up to
the outermost inline ancestor and joins the fields' values into `Span::line`;
`locate` is untouched, so the cursor still lands on the bit it is on, and the
field tree still opens the structure up. That split is the point: the flag is
about reading, not about what the bytes are, and the two places that need the
bytes exactly are the two that ignore it.

Values on a shared row are written shorter than the same values on rows of their
own, because a row that holds several of them has less room for each: a named
number gives its name and drops the number behind it. A field of no bits
contributes nothing rather than an empty string, which is what the switch for an
opcode with no immediate selects.

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
is left unclaimed, since reading a Windows program as a DOS one would describe
the stub that exists to say it needs Windows.

Not yet described: the section contents, so imports, exports, resources and
relocations are all gaps. The characteristics fields are numbers, and want the
named-bit type the IR does not have.

### MS-DOS executables
The same fourteen words, and then a different file. `dos.rs` owns them and
`pe.rs` extends them, so the header a DOS program shares with every Windows one
is written once.

What follows those words is where the format earns a template. The relocation
table is at the offset the header gives rather than where it falls, so a gap of
`relocation_table - 28` precedes it; a file with no relocations points nowhere,
and `Switch` on the count is how the IR says that the field means nothing then.
The header ends at `header_paragraphs * 16`, which is what `SizeOf` on the
relocation area is for: what ends the header is the count in it, not the sum of
what has been read. The program is whole pages less the unused tail of the last
one, with a count of zero bytes meaning a full page, and since there is no
conditional expression the full page is its own `Switch` case rather than a
subtraction that would have to know it is zero. What is left is the overlay:
bytes DOS never loaded, which is where a self-extracting archive keeps its
payload. Across 162 DOS executables in hand, every one of these adds up with no
field in error.

The fields are flat in the root struct, as SQLite's are and for the same
reason: `pages` and `header_paragraphs` have to be in scope where the program
after them is sized.

Which `MZ` files it claims turns on `relocation_table` at 0x18. A DOS program's
relocations start before 0x40, which is where the pointer to a later header
would have to be, so such a file is a DOS program and nothing else. A file that
leaves room for one is claimed only once the bytes it points at have been seen
and are none of `PE`, `NE`, `LE` or `LX`: a Windows 3.x program is left to the
rule database, which can name it, rather than to a template that would describe
its stub.

### What made a file
A second database answers a different question. Where `file(1)` says what
format a file is, the Detect It Easy signature rules say what tool produced it:
which packer, which compiler, which protector. That is what someone opens a DOS
executable to find out.

A rule is a small JavaScript program. `diescript.rs` reads the ones that say
everything in their own text, which is a test on a byte pattern and some
assignments, and counts the rest by reason. Of the 1,435 rules shipped, 683
are read; every one of them recognises bytes built to its own pattern. The DOS
rules read far better than the Windows ones (490 of 596 against 193 of 839),
because a PE rule usually asks about imports or .NET metadata rather than about
a run of bytes.

A branch's test is a tree, not one comparison: rules join tests with `&&` and
`||`, negate them, and break them over lines. What they may not do is nest.
A rule whose branch body holds another `if` is refining its answer rather than
offering an alternative, and reading the inner test as a branch of its own would
have it match every file the inner test happens to be true of. Such a rule is
skipped. The
pattern language is implemented from its definition in the engine's `xbinary.h`
rather than from the shapes these rules happen to use, because the same engine
is a shared submodule across the author's other detectors.

Two anchors. A `.COM` is loaded flat, so its rules test from the first byte; an
`MZ` executable's test from the instruction the loader jumps to, worked out from
the header. An entry point outside the file, or a packer's negative code
segment, means the rule is skipped rather than tested somewhere wrong.

Three anchors, then. A `.COM` is loaded flat, so its rules test from the first
byte. An `MZ` executable's test from the instruction the loader jumps to, worked
out from the header. A Windows executable's do too, except that its header gives
the entry point as an address in memory, so the section table has to turn it
back into a place in the file; an address in the part of a section that only
exists once loaded has no bytes behind it and the rule is skipped.

Which bundle to fetch comes from the file. Anything starting `MZ` gets `pe.sig`
or `msdos.sig` depending on whether a PE header sits where the DOS header
points, the same question the built-in sniffer asks. A small file nothing else
could name gets `com.sig`, since a `.COM` has no header to declare itself and
the format's own 64 KiB limit is the only constraint there is.

Nothing is converted. `tools/die.mjs` fetches a pinned commit and concatenates
the rule files byte for byte, author credits included, with a marker line before
each. Updating is bumping the hash and running it again, which is the whole
point: a database that had been rewritten into another format would need its
changes merged by hand every time upstream moved.

### Which BASIC runtime a DOS program wants
One answer is the editor's own, because no rule can give it. Microsoft's 1980s
BASIC compilers produced two kinds of program: one linked against
`BCOM<version>.LIB`, carrying its runtime inside it, and one linked against
`BRUN<version>.EXE`, which stays a separate file and has to be on the disk or
the program prints `Cannot find BRUN20G.EXE` and quits. Which of the two a file
is, and which runtime it names, is the first thing anybody opening one wants to
know, and it is the reason such a program will not run today.

A signature rule cannot say it. The entry point of one of these is a far call
into the loader stub the linker put at the end of the program, and the segment
it calls is different in every program, so an entry-point pattern has nothing
fixed to match. What is fixed is the stub itself: it ends the load module and
its last string is the file name it asks DOS to load. `dosbasic.rs` reads it
from the last 8 KiB of the load module, and from the load module rather than
the file, so a self-extracting archive carrying `BRUN20G.EXE` as its payload is
not mistaken for a program that needs one. Over 188 DOS executables in hand it
answers for 8 and stays silent on the rest, including the runtime itself.

The name is reported as the file writes it, and nothing is made of it: mapping
`BRUN20G` to a product version would be a guess about a naming scheme nobody
documented. A detection of the editor's own is marked `source: "qubero"`, so
the dialog credits it to reading the file rather than to a database that never
made it.

Rules test the first 64 KiB, but a DOS executable is read to 1 MiB, since the
stub is at the end of the program and so is the entry point of anything but a
small one.

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

### A stream that says nowhere how long it is
JPEG is a list of segments, and all but one kind of them carry a length. The
one that does not is the whole point of the format: after the marker that
starts a scan, the compressed bits run until the next marker, and nothing
anywhere says how far that is.

Reading it needs one addition, and it is a distance rather than a type.
`Expr::ToMarker { lead, unless }` is how far it is from here to the next
`lead` byte that is not followed by one of `unless`, so the scan is
`T::bytes(E::to_marker(0xff, ESCAPES))` and everything else about it is
ordinary. A distance composes where a type would not: it can be subtracted
from, windowed, switched on.

`unless` is there because the terminator and the escape are the same byte. An
0xff that is data is written with a zero after it, and the eight restart
markers belong inside a scan rather than ending one, so only the byte after
the 0xff tells a marker from the stream it sits in.

Two answers had to be decided rather than discovered. A container with no such
byte in it measures to its end, because a file cut off mid-scan is exactly the
file worth looking at and refusing to place the bytes would hide what went
wrong. And a lone `lead` as the last byte of a container is not a marker,
because nothing has said what it is.

The other addition is smaller. `Expr::SumOf(name)` adds up the numbers of an
earlier array, which `ProductOf` already did the other way. A Huffman segment
writes how many codes there are of each of sixteen lengths and then that many
symbols, and never writes the total.

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
which the IR cannot describe, so a NAL unit stops at its header bits. An H.264 Annex B stream is closer than it was: `Expr::ToMarker` measures to
the next occurrence of one byte that is not followed by one of a set, which
is what a JPEG scan needs. A start code is three bytes rather than one, and
a multi-byte lead is the addition still missing.

W4V covers the six-bit flavour only, and `.wac` is not read at all.

A field the panel will not let you edit says why only when you try to: the box
is simply disabled until then, because the reasons live in the write path.

`Value::Magic` carries only whether the bytes matched, so a mismatch reads
`does not match` without saying what was expected. The expected bytes are in
the template and could be carried with the value.

A signature is written as C would write a string (`text::c_string`), which puts
two bases in one line: Matroska reads `"\032E\xdf\xa3"`, and `\032` is the same
byte the gutter calls `0x1a`. C is the reason, since `\x` there swallows every
hex digit that follows and `\x1aE` would be one number. Rust and Python stop
after two digits and have no such problem. Which rules to write for should be a
setting, with C the default because a string that is wrong in C is wrong
silently. The same setting is where a plain `1a 45 df a3` belongs, for readers
who want no escapes at all.

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

Show the IR of the connections between fields, rather than only computing with
it. Where one field's meaning depends on others, the relationship should be
readable: the `Expr` tree behind a length or a count written out the way the
quantised-weights panel writes `d * scale * stored - dmin * min` with the
numbers substituted. The pieces exist (`Expr` trees, `Evaluator::origins` and
its "Depends on" section), and the rule that follows from it is that the core
should describe a relationship in a small struct the UI renders, rather than the
UI inferring one from a field's name. `ggml_quant::Offset` is the first of
those.

Search (bytes, text, regex) streaming over chunks. Selection ranges, copy/paste.
Bit-level cursor mode in the UI. Column/width presets. Worker-side core so the main
thread never blocks on reads.

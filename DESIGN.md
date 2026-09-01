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
LEB128, EBML VINT, magic, bytes/utf8 with computed length, struct, array with computed count,
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

Built-in templates live in `crates/core/src/formats/` (including PNG, wasm, MP4,
Matroska, raw DV, ISO 9660, COFF, OMF, ID3, WAV, W4V, MIDI, SQLite, PE, MS-DOS,
PDF, HDF5, and the complete current Assimp importer set), one file per substantial
format plus shared family modules such as `assimp.rs` and `wasm_opcodes.rs` for the
instruction table. WAV carries the metadata chunks bat recorders write: GUANO (`guan`) as
UTF-8 lines,
and `wamd` as a stream of tagged items whose tag numbers were read out of files
rather than a specification. W4V is the same RIFF container with a format tag of
0x5741, its `data` chunk a run of 392-byte blocks: a predictor, a scale, five
undocumented bytes, and 512 six-bit codes packed MSB first. That layout follows
the reverse-engineered decoder in the batchi project and covers only the six-bit
flavour; wider ones would need the code width read from a sibling chunk, which a
field cannot do.

The Assimp family module is deliberately a breadth layer over the importer
registry rather than fifty claims of equal parsing depth. GLB, 3DS, B3D, IFF-based
LWO/LXO, ZIP packages, Assbin, FBX, IQM, MD2/MD3, DirectX X and USDC expose their
container or header fields. JSON models use the JSON reader, XML honours byte-order
marks, and line-oriented interchange formats expose their source records. Mixed or
proprietary formats whose representation cannot be selected honestly from a shared
header keep their payload as bytes. Automatic detection is narrower still: only a
signature or a cross-checked size is allowed to claim a dropped file.

SQLite reads down to the rows. The header and the page grid are ordinary: the
fields are flat in the root struct so the page size is in scope where the pages
are sized, a page size of 1 (which means 65536, since the field is two bytes) is
a `Switch` because there is no conditional expression, and the run of pages ends
at the end of the file rather than trusting the header's page count, which
legacy files leave stale. Only a b-tree page has a type byte, so the byte is
peeked rather than read: a page that is not one keeps all of its bytes, and the
byte a page number happens to start with is not shown as a type nobody defined.
What that page is, the page never says. The header can still settle it in the
usual case: with an empty freelist and no auto-vacuum there are no freelist
pages and no pointer maps, so what is left is the continuation of a payload and
reads as the next page in that chain and the bytes it carries. With either of
those in play the honest answer is the bytes.

The cells are what the format cost the IR, and they are in the list below:
`PointerList` places them, `SqliteVarint` measures them, and `Expr::Elem` with
`Expr::Idx` types their columns. A payload too big for its page spills onto an overflow
page, and how much stays behind is SQLite's own formula, with a modulo and two
comparisons in it. Neither is an operator here and neither needs to be: a
modulo is the quotient multiplied back out and taken away, and "P fits in X" is
"P divided by X plus one is nothing", which a `Switch` asks in one case. So the
cell reads the bytes that stayed and the number of the page the rest went to.
It stops there. The record itself is not parsed across the break, because the
header saying what the columns are can be cut in half by it, and the bytes it
continues into are on another page entirely.

One thing a database does is still out of reach: page numbers, in an interior
cell or at the end of a spilled one, are read as numbers and not followed: a
b-tree that pointed at its own pages would stop being a tree, and the template
would describe a graph rather than a file.

HDF5 is the first format read here that is a graph rather than a run of bytes.
Nothing in it follows anything: the superblock holds the address of the root
group's object header, that header holds a message naming a b-tree and a local
heap, the tree's leaves name symbol table nodes, and each entry in one names
the address of another object header. Every step is `At`, so the field tree the
app shows is the file's own group hierarchy, walked one click at a time because
evaluation is lazy by path. SQLite deliberately stops short of this and says
so; HDF5 leaves no choice, since an address is all a header holds. The cycle a
pair of hard links could make is caught by the ring check `At` already had.

Two things had to give. A field placed elsewhere is no longer bounded by the
structure it was declared in: an object header message is sixteen bytes long
and the heap it names is half a kilobyte further on, so an `At` counted from
the start of the file is bounded by the file. And a group's b-tree is placed
*under* its local heap rather than beside it, because a name in that tree is a
byte offset into the heap's data segment, and an expression sees the fields of
the structures it sits inside.

A dataset's bytes read as its elements. What an element is lives in the
datatype message and where the bytes are lives in the layout message, so the
one asks the other with `Expr::Sibling`, exactly as a WAVE `data` chunk asks
`fmt ` for its sample width. The width is then kept in a field of no bytes,
since a list only gets a stride when its element size is an expression that
cannot vary per element, and a column of ten million strings measured one at a
time is not a column anyone can scroll.

An attribute's value reads as elements as well, and that is what `Expr::Within`
was added for: the datatype describing an attribute's value is written *inside*
the attribute, and `Expr::Ref` names a field beside this one and stops there.
`Within` names a field and then a path down into it. Global heap collections
are placed too, so a variable-length string is one step from the note that
points at it; which object in a collection is the one is left to the reader,
since the objects have no fixed size and there is no expression for "the
element whose index is this".

What is read: superblock versions 0 and 1, object headers of version 1 and 2,
the messages a dataset is made of (dataspace, datatype, layout, filters,
attributes, links, symbol tables and continuations), version 1 b-trees for both
groups and chunks, local heaps, symbol table nodes and global heap collections.
A group with more links than fit as messages keeps them in a fractal heap, and
that is read too: the header, the table of blocks whose rows double in size,
and the links written one after another inside those blocks, which is where the
names are. Where the links stop inside a block is written nowhere, so a run of
zeros or a stretch too short to hold one ends the run, and a block whose free
space still holds an older link reads it again. The version 2 b-tree indexing
those links is read to its root node with its records left as bytes: a record
is a hash and an offset into that heap rather than anything in the file, and
the links are already in hand.

What is not: a heap grown past the largest direct block size, whose later rows
hold indirect blocks and which needs a base two logarithm to tell apart; huge
and tiny heap objects; free-space managers; and data layout messages of version
4 and later.

A filtered chunk is the one thing in such a file whose contents are not in the
file: what is written there is the output of a pipeline of filters, so the
template leaves it as bytes and `hdf5_chunk.rs` says what it holds, the way
`pdf_objstm` and `ggml_quant` already do for packed contents. It undoes deflate
and shuffle, takes a checksum off, and stops at a filter it does not know,
naming it. Every step is reported with what went in and what came out, since a
chunk that arrived at 129 bytes and left at 400 has said something about
itself. The panel then reads the first elements with the datatype beside the
chunk, so what a reader sees is the numbers rather than the compression.

A text format for templates,
and importers for C structs and bitfields, ASN.1, protobuf, Zig packed structs,
Python pickle and C# StructLayout, are next. Further target formats: rkyv and
virtual-disk containers (VDI, VHD and VHDX).
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
`Open visible sections` expands one visible level at a time with bounded previews,
so an overview does not accidentally draw a million-element list. Expanded
composites also show undefined gaps with the same offset and length as Listing.
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

A field that reads its contents somewhere else used to break that agreement.
`At` costs no bytes where it is declared, so a structure ends where its last
real field ends and what the pointer named sits outside it: the cursor could
not land there. For a WAD that never showed, since the lumps cover the stretch
the directory sits in. For HDF5 it is the whole file: the root structure ends
ninety-six bytes in and everything else is reached by address, so the hex view
had nothing to say about a hundred megabytes the field tree could describe in
full. `crates/core/src/eval/placed.rs` indexes every stretch an `At` placed and
the field that placed it; `locate` asks that index for a bit outside the root
and carries on from there, and a bit covered by nothing is a gap rather than an
error. The walk is pruned by the template (a type holding no `At` is skipped
whole), judged by what a list's first element turns out to be rather than by
what its type could be, deduplicated by stretch, and resumable: it keeps its own
stack, does a bounded number of nodes per go and charges them against the same
allowance every other walk uses, so a five-gigabyte file is indexed across
several questions instead of freezing the first one. Until the walk reaches a
stretch, the bytes there read as a gap, which is what they read as before any of
this existed.

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
one row can say. Top-level headings are larger and carry more space than nested
ones, while field rows keep a fixed scroll unit. On touch, vertical movement
advances those virtual rows and horizontal movement remains the native column
scroll.

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

Below that, the block view lists the stretches of the block no field
describes, which is where the byte classes still have something to say about a
file a template already covers. The stretches come from `spans`, whose gaps
are exact, and that is affordable over one block where it would not be over a
whole file; picking one measures it on its own rather than quoting the block's
numbers for it.

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

Colour describes the field's data family, not its arbitrary path, so
numbers, text, markers, named categories, structures and opaque bytes keep the
same small colour vocabulary in every view. The hex grid colours digits and a
hairline rather than filling the byte: neutral fill remains selection, accent
fill plus underline remains the active field, and an outline remains the
cursor. A short vertical hairline marks field boundaries.

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

Some formats name a few values and then stop naming and start counting. A
SQLite serial type names ten of them, and from twelve up every even number is a
blob and every odd one is text, with how far up the number is saying how long
the value is. Listing those one at a time is not possible and reading them as
unknown is not true, so an enum carries runs beside its cases: a first value,
how far apart the values of the run are, and a name written with `{n}` where the
count goes. Two runs at a step of two is how one range holds both. The number
underneath is untouched, so a switch still sees it, and the panel says what the
value is called rather than that nobody named it. Nothing reverses: writing by
name still only finds the values named one at a time, because "text, 2 bytes"
is a description of the bytes elsewhere rather than a value to choose.

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
both start `MZ`; only a `PE\0\0` at the offset held at 0x3c separates them, so
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

### A format written inside another one
The EXIF block of a JPEG is a whole TIFF file, header and directory and all,
written into a segment partway through. Reading it twice, once as TIFF and
once as EXIF, would be two descriptions of one format that could disagree.

What stopped the one description serving both was that `Ty::At` counted from
the start of the file, and an embedded TIFF counts from its own first byte.
So `At` carries an anchor now, the same three choices `PointerList` already
had. `Anchor::Window` is the nearest `Sized` around the field and falls back
to the start of the file when there is none, so the TIFF sits in a window of
its own and the one layout is a file when it is a file and a copy inside
something else when it is not.

Borrowing the description needed one more thing. Named types live on the
`Template`, so a bare `Ty` handed to a borrower cannot carry the names it
refers to, and a directory whose entries point at directories is nothing but
names. A `Part` is the type and its vocabulary together: `tiff_part()` hands
over both, `Template::with_part` takes them in, and there is no way to place
the description and leave the names behind. Names are prefixed by the format
they came from, `tiff.Ifd.le`, because the table they land in is shared. A
scoped lookup would have been the tidier answer and is the wrong one: three
places in the evaluator resolve a name, two of them without a path to scope
by, and forgetting to scope a fourth would fail quietly, where forgetting to
register fails loudly.

An offset that names a place can name a place already open, and that is a
ring. Every other way of placing a child moves forward, so a type referring to
itself is bounded by whatever contains it and must end; an `At` is the one that
is not, and a ring is not slow but endless, since the cursor asking what covers
a byte would go round it forever. Placing an `At` child now asks the line of
ancestors whether any of them is the same type in the same place, which is what
makes two nodes one node, and refuses the step that would close the loop. Only
ancestors: two entries pointing at the same string are two entries pointing at
the same string. A count of the pointers above comes with it, so a chain that
is not a ring but is a million long is stopped as well. What a type is called
is settled before comparing: the pointer says `tiff.Ifd.le` and the node
already open there remembers the structure that name stands for, and comparing
the two as written would have the guard quietly never fire on the one shape it
was built for.

That guard is what makes the three pointing tags safe to follow. `exif ifd`,
`gps ifd` and `sub ifd` hold the place another directory begins rather than a
value, and what the first two point at is read against another set of tag
names: tag 1 is `subfile type` in a TIFF directory and `latitude ref` in a GPS
one. Splitting the tag spaces also bounds the recursion, since nothing in the
camera's names points back.

The same anchor settled the other half of a TIFF entry. Four bytes hold the
value when it fits and where to find it when it does not, and nothing in the
entry says which: it is the size of the type times the count. That is a field
of no bits, and the switch is on whether the answer is over four. A value that
does not fit is read at its offset from the window, so a camera's make and
model and exposure are strings and fractions rather than three numbers that
happen to be small.

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

### A file that writes its numbers as digits
PDF is the first format here that is mostly text. The header is a line, the
cross-reference table is lines, the trailer is a dictionary typed out in full,
and the offset that finds the table is written as decimal digits under the
word `startxref`. Reading it needed two things the IR could not say, and both
of them are about words and digits rather than about PDF.

The first is `Ty::TextInt`. A text field used in an expression is its bytes as
a number, which is what lets a switch key on `IHDR`; on `408` it gives
0x343038, a number three million past the end of a four hundred byte file. A
pointer list handed that places its children nowhere. The parse has to happen
where the field is read, so the type reads like text, measures like text, and
values like an integer. It takes the same `StrLen` as text does, so a run of
digits can be a fixed width, or scanned with the white space around it stepped
over, and leading zeros are read rather than complained about since padding a
number to ten columns is how the format keeps its table lined up.

The second is `Expr::Find { needle, last }`, which is `ToMarker` for a word
instead of a byte. Where the pointer at the end of a PDF is cannot be worked
out by counting: the offset under `startxref` is as wide as it is, and the
end marker after it is followed by whichever of three line endings the writer
preferred, so the word itself has to be looked for. `last` is there because a
file that has been saved twice keeps both its tables and means the second, and
because the same search run forwards is what ends an object: `endobj` closes
one, and nothing says how long the body was.

What the two together buy is the shape the format is actually in. A field of
no bits reads the offset under the word, another reads the table where that
offset points, and the objects are a pointer list over its entries, exactly as
a WAD's lumps are a pointer list over its directory. Every byte of the tail is
named by a field that costs nothing where it is declared, so the objects can
still be the list that covers the whole body.

The numbers on the line between `xref` and the entries are read one at a time
rather than as a line, and that is what `SizeOf` is for: a decimal is as wide
as it was written, so where the count starts is only known once the number
before it has been read. It is also why the fields are flat rather than nested
in a structure of their own, since a name reaches fields beside it and fields
around it but not fields inside something else.

One thing the table says had to be looked ahead for. An entry's last column
says whether the object is there, and it is written after the two numbers whose
meaning it settles: a free entry's ten digits are the number of the next free
object, not an offset, and reading them as one is how a reader ends up parsing
whatever sits twelve bytes into the file. The entry is picked by a peek at that
column, and the free case gives its offset as a computed zero, which the list
already reads as pointing at nothing.

The peek is the one place a run of entries starting late is felt. The spec
fixes an entry at twenty bytes and the line above it at whatever the writer
liked, and a scanned field takes one byte of that line ending, so the entries
can begin a byte or two after the arithmetic says. They step over what is left
and read the same; a fixed look-ahead does not. So it looks at all three
places the letter could be, by subtracting it from each and multiplying: a
product is zero when any of its parts is, and nothing else that far into an
entry is a letter.

What is left out is the part of PDF that stopped being text. A cross-reference
stream keeps the same table compressed inside an object, which is a template
that needs the object read and inflated first, and the `/Prev` chain that an
incrementally saved file leaves behind is a walk backwards through every
revision. Neither is here, so a file written that way reads its header and its
end marker and says the table did not. Nor is a table written as several runs
of object numbers: the entries of the first run are read, and the heading of
the second is whatever the trailer after them turns out to hold.

### A size written somewhere else
A ZIP entry is measured by the `compressed_size` in its local header, and for
thirty years that was the whole story. It is not any more. An entry too big for
32 bits writes 0xFFFFFFFF there and its real size in an extra field tagged 1,
and the advice a writer is now given, in OME-NGFF's RFC-9 among others, is to do
that for every entry whatever its size, so that a store written a chunk at a
time never has to decide. Read as a number, the placeholder measures the entry
as four gigabytes and takes every record after it with it.

The extra fields were a run of bytes here, so the size was not reachable even
though it was sitting fifty bytes back. They are now what they are: tagged
records, each an id, a length, and a body, read as a list. Which is worth
having on its own, since an archive keeps its timestamps, its Unix ownership
and its alignment padding there and none of it was named before.

Reaching into that list is the one addition. `Expr::Tagged { array, key, tag,
field }` is the value at `field` in the first element of `array` whose `key`
holds `tag`, and zero when nothing does, so `Or` can name what to do without
one. `Elem` reaches an element by where it is, which is no use here: the writer
may put the record anywhere in the list, put its own records around it, and
leave it out entirely. So the length of an entry's data is the header's size
when the header wrote one, then the tagged record, then the scan for a data
descriptor that a streamed entry already needed. A streamed ZIP64 entry asks
all three, and correctly: its extra field is there and holds zeros, because the
writer did not know the sizes when it wrote either of them.

The same tag means something else in the central directory. There, only the
fields whose 32-bit counterparts hold the placeholder are written, in a fixed
order, so an entry that sits past four gigabytes in a small archive carries an
offset and no sizes at all. The record is read by asking, field by field, what
the header around it said, which an expression can do because a name inside a
record reaches outward through the structures it sits in.

One thing is decided rather than read. Nothing in a data descriptor says
whether its two sizes are four bytes each or eight; the answer is whether the
entry was a ZIP64 one, and the descriptor is a record of its own with no way
back to the header it belongs to except the search `Expr::Sibling` already does
for a WAVE `data` chunk asking what `fmt ` declared. It looks back for the
nearest local header's `compressed_size` and takes a placeholder there as the
answer. A writer that streams a ZIP64 entry while leaving those sizes at zero
is read as the narrower one, and the eight bytes left over read as a record
nobody defined, which the walk passes over as bytes to the next `PK`. Wrong
shape, right place.

### A header that says how much of itself there is
Five formats a Linux system leaves lying about went in together, and none of
them needed anything new. That is the point of adding them: the IR is tested by
what it cannot say, and this time it could say all of it. A flattened device
tree is a stream of tokens and three blocks at offsets the header gives, a GRUB
environment block is text padded with `#` to exactly 1024 bytes, `wtmp` is one
struct repeated with nothing in front of it, an initramfs is a cpio archive
whose numbers are written as hexadecimal digits, and a systemd journal is an
arena of tagged objects.

What they have in common is worth naming, because two of them do the same
thing and so does the ZIP64 work above it. A journal header is as long as
`header_size` says, and which fields are in it depends on which systemd wrote
it: `n_data` arrived in 187, the chain depths in 246, the tail entry array in
252. A ZIP64 record in a central directory holds only the fields whose 32-bit
counterparts were left as placeholders. Both are read the same way: the whole
run is a `Sized` window, and every field that might not be there is a `Switch`
on whether there is still room for it, which is `Expr::Remaining` compared
against the field's own width. A file from an older writer stops where it
stopped, and what follows it is not read as the fields it does not have. Since
both wrote that out, it is `Ty::if_room(ty)`, and `Ty::present_if(when, ty)`
where the format has something else to say about whether the field is there.
How much room is the type's own size, so nothing declares its width twice.

The other shared answer is about padding. Three of these formats align
something to a boundary: a device tree pads a node's name to four bytes, a cpio
archive pads its name and its file data to four, and a journal pads every
object to eight. There is no remainder operator, so each wrote out the same
subtraction: `n` less the whole fours in it is the overhang, four less that is
the padding, and the same subtraction again takes it back off in the case where
the run already ended on a boundary and the answer came to four rather than
none. Three copies of an arithmetic whose only interesting case is the one it
keeps getting wrong is an argument, so it is `Expr::PadTo { n, align }` now.
Four older templates were writing it too, in a form that only worked for two:
IFF, AIFF and the two Corel containers pad an odd chunk to an even boundary,
and an AppleDouble attribute name pads to four.

`Ty::TextInt` grew a base. PDF needed decimal digits read as a number; cpio
writes every number in its header as eight hexadecimal digits, which is the
same field with a different radix, and the length of a file being text is what
the padding arithmetic then has to work from.

Two things are deliberately not read. A compressed stream inside any of them
stays a run of bytes named by its magic: the second half of an initramfs, and a
journal payload whose object flags say XZ, LZ4 or ZSTD. Showing what is in
those means decompressing them, which is the same open question the gzip
template already stands in front of.

### A file with no front
A stream of CCSDS Space Packets has no magic number, no start-of-packet
marker, and no checksum. A packet is six octets of header and then its data
field, and the header's last two octets hold a count one fewer than the length
of that field. The only way to find the second packet is to have read the
first.

That makes the template trivial and the recognition the whole problem. The
answer is the same shape as the one a Zarr store in a ZIP needed: read the
lengths and see whether they chain. Eight packets in a row, each beginning
with the three zero bits that say this is version 1 of the standard, is the
evidence; a file of zeros chains perfectly well as seven-octet packets of
nothing, so a stream in which no header says anything at all is refused. It
sits at the end of the careful tests, after the table of signatures, because
a file that says what it is should get to say so first.

A real capture is a recording that was stopped, and the one this was written
against ends eight bytes into a packet that says it is 231 long. Reading what
is there is the answer, so the data field is `Min(claimed, Remaining)`. Three
templates had already written that clamp out as a comparison multiplied by
each side and added back together, which is `Expr::Min` and `Expr::Max` now:
an MS-DOS header that counts pages a library never wrote wants the same thing.

What the packets hold is not here and cannot be. The standard says the data
field may begin with a secondary header, that its format is registered with
the mission rather than with CCSDS, and that a packet whose APID is all ones
is an idle packet sent to keep a downlink busy. A capture is mostly those: of
the 242,725 packets in the one this was tested against, 229,231 are idle.

### Four ways to write a package
A Debian package, an RPM, a Windows cabinet and a macOS installer hold the same
thing and agree on nothing about how to write it. They went in together, and
between them they cost the IR one constructor.

A `.deb` is an `ar` archive, which is the oldest archive format Unix still
uses: a magic line and then members, each with sixty bytes of header written as
text. A static library is the same file with object files in it, so one
template reads both and the only thing that makes a package a package is the
name of the first member, `debian-binary`. Every number in the header is
digits, left aligned in a field padded with spaces, and the mode is octal
because a Unix mode is read in no other base, which is `Ty::octal` and the
third radix `TextInt` has grown. The awkward part is that GNU `ar` leaves the
timestamp, owner and mode of its long-name table blank. An empty field is not a
number and reading it as one would report an error where the archive did
nothing wrong, so each of those fields is a `Switch` on `Expr::Peek` of its own
first byte: a space at the front of a left-aligned field means there is nothing
in it, and the field reads as the spaces it holds.

An RPM is a lead nobody reads any more, two headers of the same shape, and the
payload. A header is a run of sixteen-byte index entries and a store of bytes
those entries point into, so the values are in no order and the space between
them is alignment padding: a run of four-byte numbers starts on a four-byte
boundary. Reading the store in order would mean guessing at that padding, so
the store is a `PointerList` and every value is at the offset its own entry
gives, which leaves what no entry claims as a gap. The list sits in a
structure inside the store's `Sized` window rather than being the window: an
anchor of `Anchor::Window` looks outside the field asking, so a list that is
its own window would count from the file. The two headers keep separate tag
vocabularies, because the numbers overlap and mean different things: 1000 in
the signature header is how many bytes follow it, and 1000 in the header is the
package's name. One asymmetry has a test of its own, since a file read a byte
late is unreadable from there on: the signature section is padded to a multiple
of eight bytes and the header after it is not.

A cabinet is Microsoft's, and it is the only one of the four that lays its own
files out rather than wrapping an archive someone else wrote. Folders say where
a run of compressed blocks starts, files say where in a folder's decompressed
stream they begin, and the blocks are that stream cut into pieces of at most 32
KiB. So a file's bytes are in no one place, and the offset in an entry counts in
a stream this template does not produce. The header is the interesting part:
three of its fields are written only when a flag says so, and everything below
measures its reserved space against them. A field that is not there is not a
field holding zero, but the format says the reserve is empty when the flag is
clear, so those three are a `Switch` whose other case is `Ty::computed(0)`: no
bytes, and a number the folders and the blocks can still be sized against.
`Ty::present_if` would have left them as no bytes at all, which is right for the
two strings beside them and wrong for a size.

A macOS `.pkg` is a xar, and it is the one that gets away. Its table of contents
is XML and it is compressed, so which files are in the package, what they are
called and where in the heap each one sits are all behind an inflate. What is
readable is the header, and it earns its place twice over: it measures the table
both ways round, which is what makes the heap findable without inflating
anything, and it says how long it is itself, so a longer header from a newer
writer moves the table rather than breaking it.

They stop in the same place. A `.deb` carries two tar archives, an RPM carries a
cpio archive, a cabinet carries MSZIP, LZX or Quantum, and a xar carries a
deflated table. Naming what wrote a compressed run from its first bytes is as
far as this goes, which is where the initramfs stopped as well.

### A file that is a dump of another file
A hex dump is the oldest view of a file there is, and a capture of one is the
file written out in digits. People send them in bug reports, paste them into
issues, print them in manuals, and take them off machines that have no other
way to get a file out. What arrives is text that describes a binary, and the
binary is what the reader wanted. `crates/core/src/hexdump/` reads it back.

Nothing in a dump says how to read it. `xxd` writes an eight-digit hex address,
a colon, eight groups of two bytes and a text column; `od` writes six digits,
no colon, sixteen groups of one and a text column inside angle brackets;
`certutil` indents, splits the digits in half and writes no text column;
PowerShell writes sixteen digits, a label naming the file and a column heading;
and every one of them writes something else if it is asked to. A reader that
knows one of them knows one of them.

So the layout is read off the lines. The one thing a dump cannot lie about is
arithmetic: the address of a line plus the bytes on it is the address of the
next. Every hypothesis about which token is the address and what base it is
written in is checked against that sum, and how many bytes are on a line is
taken from the difference of two addresses rather than from counting digits.
That last point is what makes it work at all, because counting digits is what
goes wrong: a line reading `3031 3233 3435 3637  01234567` has a text column
that is itself eight hex digits, and its addresses say the line holds eight
bytes rather than twelve.

Two things are settled the same way, by being checked rather than assumed. An
octal address is not a hex one because subtracting two of them gives sixteen
one way and thirty-two the other. And a dump with no address column at all
survives nothing, which is how `certutil -encodehex` is told apart from a dump
whose bytes happen to climb evenly: `00 01 .. 0f` over `10 11 .. 1f` gives a
first token that steps by eight in octal, and the hypothesis that believes it
has left half a line of digits sitting where the characters should be. A
hypothesis that does that on more lines than it explains is the wrong one.

The names (`xxd`, `od -Ax -tx1z`, `Format-Hex`) are attached afterwards, to a
layout already settled. They are a label on the answer and never a route to it,
so a tool nobody here has heard of reads the same as one that ships with every
system.

**The two columns check each other.** Most dumps write the bytes twice, once as
digits and once as characters, and that is redundancy sitting unused in nearly
every dump ever pasted anywhere. Read both and a mistyped digit, a line wrapped
by a mail client, or a group written backwards stops being invisible. The
answer per byte has three cases rather than two, because a full stop stands for
so many bytes that most of the column can only ever confirm: agreed, not
checkable, or in conflict. Which encoding the characters are in is decided by
which one the digits contradict least, so `Format-Hex` is read as Latin-1 and a
DOS tool's column as CP437, and when the column holds nothing but printable
ASCII, which every encoding agrees on, the layout says it assumed.

That check is what reads `xxd -e`. It writes each group as a little-endian
number, so the digits run backwards inside the group, and nothing on the line
says so except the text column, which still reads in file order. Both orders
are tried and the one the characters agree with wins. With no text column the
digits are taken at their word and the layout says that too.

**What is missing is not filled in.** A dump of part of a file is ordinary, and
so is a terminal transcript holding two runs of `xxd` over different stretches
with a shell prompt and the output of `ls` between them. The result says which
addresses it covers and leaves the rest as a hole. A run of identical lines
collapsed to a `*` is the same problem from the other end: the length of the
run is written nowhere, and is the difference between the address before the
marker and the address after it.

**The file says things about itself.** `Format-Hex` writes the path it dumped,
`od` and `certutil` write the length, and a transcript keeps the command with
its arguments on the line above. That is metadata a hex parser throws away and
a reader wants, since it is how a dump of a stretch in the middle of a file
knows that is what it is. Only the few shapes that actually carry something are
read; anything looser reads a sentence out of a shell prompt.

`hexdump/write.rs` is the other direction, and it is the plain text view: a
file, or a stretch of one, written out as a dump in a layout. It is here
because the dump is sometimes the deliverable, and because it is how the reader
is tested. Seven of the captures in the sample collection come back character
for character after being read and written again, squeezed runs and ragged last
lines included, and anything the reader failed to notice about a layout turns
up as a column in the wrong place.

The samples are in `qubero-samples/hexdump/`: one small file made for the
purpose and a dozen captures of it, from `xxd` with five sets of options
(including one that keeps the ANSI colour and one that reverses its groups),
`od` with three address bases, `certutil` two ways, `Format-Hex` in UTF-8 and
UTF-16, and a bash transcript. A dump reproduces the file it dumps, so nothing
that is not redistributable can be dumped into that collection.

A hex viewer that draws on a screen instead of writing to a pipe does not need
a stand-in at all, and that turned out to be a second rule rather than a second
encoding. The hardware has a glyph for all 256 values, so XTree Gold with its
mask off shows a smiling face for 0x01 and a musical note for 0x0D, and the
character column then says something about every byte rather than about the
printable ninety-five. `hexdump/glyphs.rs` holds both rules and the column is
read as whichever it contradicts least: CP437 the encoding has nothing to say
about 0x01, and CP437 the screen says it is U+263A.

Such a capture then arrives one of two ways, and the file does not say which. A
DOS screen is CP437, so it can be taken as the bytes it was drawn in or as the
Unicode a modern clipboard turns those into: the rule under XTree's header is a
run of 0xC4 in one and U+2500 in the other. Both are read and the one whose two
columns agree better is kept, which is the same rule as everywhere else here
rather than a new one.

**There are two ways through, and the dump picks.** Almost everything anyone
opens is a machine's output, unedited: the same layout on every line, every
line the same length, every address one line's worth past the one above.
`hexdump/strict.rs` checks exactly that, once, and keeps the answer as a
handful of runs. Where line `n` starts is `at + n * stride`, which line an
address is on is a division, and the line is read when it is asked for and not
before, so a dump of a gigabyte costs an index and a screenful. Everything else
falls to the path that reads a line at a time and keeps every one, which is
what a shell prompt between two dumps, a column heading, a line wrapped by a
mail client, a screen of box drawing, or a `*` standing for a run of identical
lines needs.

That is the same division a browser makes between a parser for well-formed
markup and a parser for what people write, and for the same reason: the strict
one is fast because it is allowed to refuse. What it refuses is written down:
a byte-order mark, an escape sequence, any byte outside ASCII, a `*`, an
address that does not follow the one above it, lines of differing length in the
middle, and more than a few lines of heading or footing around the outside.

It is a verifier and not a second grammar. Deciding how to read a line is
`parse_row`, the one the slow path uses, so there is one place where the format
is understood; the fast path only decides *where* the lines are. What keeps
them honest is `read_irregular`, which reads a dump the slow way whether or not
it deserves it, and a test that reads every capture in the collection both ways
and compares the layout, the stretches covered, the notes, the column conflicts
and the bytes. Seven of the nineteen take the fast path, which is the seven
that are nothing but a tool's output.

Two things are settled from a bounded sample of rows rather than from all of
them, on both paths so that both answer alike: which way round a group reads,
and how the characters were written. A dump is still read in one go, up to
`hexdump::LIMIT`, because the text arrives as bytes; reading it through a
`Source` instead is what the run index was built for. A dump laid out regularly needs
nothing of the sort, since the line holding an address is arithmetic on the
line length, and that is the upgrade when a dump arrives too big to hold. Nor
is the recovered binary a document yet: it is a `Dump` that answers reads by
address, which is the shape a `Source` needs. Where it goes is decided: a
document of its own beside the text, rather than a view the text file swaps
into. The two cannot share a cursor, since an offset in the dump and an
address in what it describes are different numbers, and a document with its
own four views is what makes the recovered bytes an ordinary file with a
template, a listing and an overview. What keeps the pair connected is
provenance: every byte knows the digits that wrote it, so a byte in one can
still point at a place in the other.

Editing through it is further off still, and it is the first real client of
the redundant-editing work below: changing a byte in the binary has to
rewrite a pair of digits and a character, and the two have to keep agreeing.

XTree Gold is in the collection now, four ways: masked and not, as CP437 bytes
and as Unicode. Its hex view is a screen rather than a stream, which is a
different thing to read. Nineteen lines fit, so a capture covers the first 0x130
bytes and the rest of the file is simply not there; the header across the top
names the file and the mode rather than being anything a hex parser would look
at; and one of the four is damaged on purpose, because unmasked the glyphs for
0x0A and 0x0D went through a clipboard as a real line ending and split a line of
the dump in two.

Getting it needs DOSBox-X, and the way through is worth writing down since none
of it is obvious. The program files are inside ZIPs on the floppy images, so
7-Zip unpacks them and no installer runs. Keystrokes have to be injected as
scancodes rather than as virtual keys, which is what SDL reads; DOSBox-X's own
`AUTOTYPE` does the same job for anything typed before the program starts.
Ctrl+F5 is "Copy all text on the DOS screen", which is the text screenshot, and
`COPY CLIP$ FILE.TXT` inside the guest writes what the clipboard holds back
through the guest's own code page, which is how the same screen is captured
both ways. The steps are in the sample folder's own README.

### The file as the text it is
The hex grid answers "what is at this address" and the listing answers "how is
this file put together". Neither reads a file the way it was written to be
read, and plenty of files were: a log, a manifest, a terminal captured to disk,
a hex dump somebody pasted. `crates/core/src/textview.rs` is the model and
`web/src/textview.ts` is the third main view.

It scrolls by byte offset rather than by line number, for the reason the
listing already has: nothing can say how many lines a file has without reading
all of it. So the scrollbar stands for a place in the file, exactly as the hex
grid's does, and the gutter carries each line's offset rather than a line
number that would only be right if the file had been read from the top. Going
backwards is a search for endings in a window before the position, which is the
only way back through lines that are not a fixed length.

Four things a text file does not write down are each answered rather than
assumed, and all four turned up in the dump reader first. Which encoding: a
byte-order mark settles it, and where there is none the bytes decide and the
view says it was a guess, with a chooser beside it because nothing in a capture
of a DOS screen says it is CP437. Which line ending: per line, since a file may
use all of them, and a line whose ending is not the one the rest of the screen
used is marked. Where a line stops when it never does: a minified file is one
line of two gigabytes, so a line is cut at 4 KiB and says it was cut. And where
the escape sequences are: a capture of a coloured terminal is full of them, and
they are neither dropped nor shown as gibberish. Each line says which stretches
of it are escapes; the view dims them and shows control characters as their
Unicode pictures, so nothing moves the text around and nothing is hidden.

A document is now a `Source`, so what the view reads is what the document says
rather than what the file on disk says. That is what makes the view editable:
typing writes through the piece table and the next window read comes back
changed.

The caret is the cursor rather than a second position, which is the same rule
the other three views follow. It sits between bytes and is drawn as a line
before the character it is in front of, while the character the cursor is
inside keeps its highlight; the two say different things and after an insert
they are next to each other. Typing inserts at the cursor, and every other view
follows it as it already did.

What is typed goes back through `text::encode_settled` in whichever encoding
the file is being read in, so a box-drawing character typed into a CP437
capture is one byte and an accent typed into a file read as ASCII is refused.
The refusal names the character and the encoding, because the encoding may have
been a guess and this is where a wrong one is found out. Backspace takes a
character rather than a byte, so an accent in UTF-8 goes in one press and both
of its bytes go with it, and a character above the basic plane in UTF-16 takes
four. Enter writes whichever ending the rest of the file uses, since a file
that has settled on one should not be given the other. One keystroke is one
undo step.

There is one selection, the way there is one cursor. It lives in the hex view,
which is where it always lived; the text view renders what that says and writes
back through `selectRange`, so the two cannot drift apart. Shift with a
movement key extends one, a drag pulls one out, typing or backspace replaces
it, and Ctrl+C copies what it says rather than the hex pairs the grid copies,
which is the difference between the two views in one keystroke.

Two details that only showed up once it was built. `selectRange` grew a third
argument for where the cursor goes, because a selection dragged out in the text
has its caret at the end being dragged and putting it back at the front
collapsed the selection on the next keypress. And every text edit is wrapped in
a batch: a replacement that changes length is a delete and an insert
underneath, so without one a keystroke over a selection took two presses of
undo to put back, leaving the file in a state nobody typed.

Editing a document opened out of a dump changes those bytes and not the digits
in the dump that spell them. Making the two agree is the redundant-editing work
below, and this is the first real client of it.

### What a selection says
The panel beside the cursor reads a selection as a number. It now reads it as
text as well, which is the question a hex editor's reader asks about a stretch
of bytes at least as often.

Six encodings and six rows would be five rows of the same sentence: most runs
are printable and every encoding here agrees on that range. So `text::readings`
gathers the encodings that agree and the panel shows one row per distinct
reading, with the encodings that produced it as the row's label. That the four
of them agree is worth knowing on its own: bytes that read the same whatever
you assume are bytes nobody can misread. Which encoding is named first is
whichever the text view is reading the file in.

An encoding the bytes do not fit is named rather than shown, since what it
produces is a row of replacement characters that says nothing, and leaving it
out silently would read as the panel having forgotten it. A reading's control
characters get the pictures the text view gives them, because a selected line
feed drawn as a line feed is a row that looks empty. One byte is enough to
read: that is a character, unlike the byte-reversed number rows, which want two
before reversing means anything. A selection made over the bits is not
characters at all and says nothing here, which is the same rule those rows
already follow.

### A file that describes another file, opened as one
`hexdump/source.rs` makes a dump's bytes readable like any other file, and the
app offers them. A text file that turns out to be a dump says so in a row above
the views, with what was found and the two things that would mislead someone
opening it blind: gaps nobody wrote down, which read as zeros, and bytes the
hex column and the character column disagree about. Opening it makes a document
of its own with all four views, named after the file the dump named where it
named one, which is what `Format-Hex`'s label and XTree's header line carry.

Nothing new was needed to hold it: opening bytes lifted out of a document as a
tab of their own is what a zip entry already does. What is new is that the
bytes were never in the file, only written out in digits.

### Bits from the bottom of the byte
Bit order used to be MSB-first everywhere. Bit 0 of a byte was its top bit, a
field narrower than a byte was packed big-endian whatever it said, and `endian`
on such a field was read and thrown away. DEFLATE and a Zig packed struct fill
a byte from the other end, and neither could be described.

The addition is not a new field on the type. It is the meaning of the one that
was already there and being ignored. `endian` says which end of the field the
low bits come from, and for a field of whole bytes on a byte boundary that is
byte order and nothing else, which is all it used to mean. A field narrower
than a byte, or one starting partway through a byte, has no bytes to order, and
the same question there is which end of the *byte* its bits are taken from. So
`Big` is the MSB-first packing this IR always had, and `Little` is LSB-first:
the field sits at the bottom of the byte and the fields declared after it stack
upwards. The two answers agree at whole-byte widths already, since taking a
number's bits from the bottom of each byte in turn is little-endian, so nothing
that was being read correctly changed.

What that costs is one thing, and it is the thing worth writing down. A field's
offset is a count of the bits laid down before it, and for an MSB-first field
that count is also an address. LSB-first stacks the other way, so the count has
to be turned around inside the byte to say where the bits are: two fields of
three and five bits are at bit 5 and bit 0 of the byte they share, in that
order, which is `0bBBBBBAAA` written out. `Resolved` therefore keeps both
numbers — `offset`, where the bits are, and `cursor`, where the count reached —
and the walk that places the next field adds its size to the second. Everything
else in the app reads the first and so is unchanged: the value, the write, the
gutter, the listing, all of it takes those bits as it takes any other field's.

Two consequences follow and both are deliberate. A structure that packs bits
from the bottom has fields that are not in the order they sit in, so `child_at`
treats one as scattered, the same as a structure holding a field that points
elsewhere: the search for what covers a bit cannot stop at the first field
starting past it. And a field that would straddle a byte boundary is refused
rather than placed. Twelve bits from the bottom of one byte are all of that
byte and the low nibble of the next, and a bit address numbered from the top of
each byte has no single range meaning those bits; a field that cannot be given
an honest place says so instead of being given a dishonest one. That is the one
shape this does not cover, and DEFLATE's header does not need it.

`Expr::Peek` answers the same way for the same reason, since a peek of three
bits is a field of three bits that takes no space. `Ty::u8()` moved from
`Little` to `Big`, which changes nothing for a byte on a byte boundary and
keeps the MSB-first packing for the byte-wide fields that sit partway through
one — JPEG XR has them. The type table writes ` lsb` after a sub-byte field's
width and nothing after an MSB-first one, since only the order that is not the
default is worth the space.

### A stream that ends at something longer than a byte
`Expr::ToMarker` measured to the next occurrence of one byte not followed by
one of a set, which is exactly a JPEG scan: 0xff and a byte that is not zero,
where zero is how an 0xff that is data gets written and so the escape and the
terminator are the same byte. An H.264 Annex B start code is `00 00 01`, and
one byte was not enough to say that.

`lead` is now a sequence. A single-byte lead is a sequence of one and behaves
as it did, which is what keeps the JPEG scan working; `E::to_marker` still
takes a byte and `E::to_marker_seq` takes the sequence.

The second half of it is what `unless` means when it is empty. A start code is
followed by the NAL header, not by an escape, so there is nothing to tell the
marker apart from — and with nothing to tell it apart from, a lead at the very
end of the container is a marker rather than a lead nobody confirmed. So an
empty `unless` also drops the requirement that a byte follow the lead, and the
blocks the search reads in overlap by one byte less: a marker and its
successor have to be whole in one block, and where there is no successor only
the marker does.

## Roadmap (not yet built)

### Resilient redundant editing
Two fields derived from the same bytes, or two bytes ranges that must agree
(seconds vs minutes, a length field and the array it describes). Model as pairs of
invertible expressions (bidirectional lenses): each typed field has `decode(bytes)`
and `encode(value) -> bytes`; `eval.rs` and `encode.rs` are those two halves already,
and the inspector rows have the same shape. Editing either side writes through its `encode`; dependent fields re-evaluate. When an edit
would make a constraint unsatisfiable, say so rather than silently picking a side.

### Known gaps
An LSB-first field that straddles a byte boundary is refused rather than
placed, since the bit address space has no single range that means its bits.
See "Bits from the bottom of the byte" for why, and for what is built.

MP4: sample tables past `stsd` are bytes. An SPS or PPS is exp-golomb coded,
which the IR cannot describe, so a NAL unit stops at its header bits. What is
left for an H.264 Annex B stream is a template for one; the measure it needs is
built, and see "A stream that ends at something longer than a byte".

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

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

Built-in templates live in `formats.rs` (PNG, wasm). A text format for templates,
and importers for C structs and bitfields, ASN.1, protobuf, Zig packed structs,
Python pickle and C# StructLayout, are next. Further target formats: zip, rkyv, glTF.
Text encodings still to add: CP437, JIS.

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

### Saving
`save.rs` turns the piece list into runs. The host composes a `Blob` from lazy slices
of the original plus add-buffer bytes; only bit-unaligned stretches are read through
the core. Written to a new file via `showSaveFilePicker` where available, otherwise a
download. Note: a bit-level insert or delete shifts everything after it, so the rest
of the file is rewritten on save. That is inherent; the UI should show progress.

## Roadmap (not yet built)

### Resilient redundant editing
Two fields derived from the same bytes, or two bytes ranges that must agree
(seconds vs minutes, a length field and the array it describes). Model as pairs of
invertible expressions (bidirectional lenses): each typed field has `decode(bytes)`
and `encode(value) -> bytes`; `eval.rs` and `encode.rs` are those two halves already,
and the inspector rows have the same shape. Editing either side writes through its `encode`; dependent fields re-evaluate. When an edit
would make a constraint unsatisfiable, say so rather than silently picking a side.

### Known gaps
Save shows no progress while rewriting bit-shifted stretches. The type table has no
keyboard navigation between rows, so a value cell is reached by clicking or tabbing.
Text fields are displayed through `from_utf8_lossy`, so invalid bytes show as U+FFFD;
committing that back would write the replacement character, and only the exact-length
check stands in the way.

### Later
Search (bytes, text, regex) streaming over chunks. Selection ranges, copy/paste.
Bit-level cursor mode in the UI. Column/width presets. Worker-side core so the main
thread never blocks on reads.

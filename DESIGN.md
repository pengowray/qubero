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

## Roadmap (not yet built)

### Templates are expressions, not a static layout
A field's offset, length, and type can all be computed: an array of u32 whose length
is read from another field; a LEB128 whose byte length depends on its content; a
chunk whose position is "after the previous one". So the template IR must be an
expression graph evaluated lazily against the document, with a dependency tracker so
a byte edit invalidates only the fields that read it. Think spreadsheet cells whose
formulas can say `bytes(offset, len)`, `u32le(at)`, `leb128(at)`, `sizeof(field)`.

Target formats to drive this: zip, wasm (LEB128), rkyv, png, glTF. Descriptions to
import: C structs and bitfields, ASN.1, protobuf, Zig packed structs, Python pickle,
C# StructLayout. Text encodings: UTF-8, ASCII, CP437, JIS. Primitives: f16/f32/f64,
integers of arbitrary bit width, magic numbers, alignment/padding.

### Resilient redundant editing
Two fields derived from the same bytes, or two bytes ranges that must agree
(seconds vs minutes, a length field and the array it describes). Model as pairs of
invertible expressions (bidirectional lenses): each typed field has `decode(bytes)`
and `encode(value) -> bytes`; the inspector rows already have this shape. Editing
either side writes through its `encode`; dependent fields re-evaluate. When an edit
would make a constraint unsatisfiable, say so rather than silently picking a side.

### Nested type table
Below the hex view: a tree/table of the template's fields with live values, expandable
arrays and structs, click to select the bytes in the hex view, edit in place.

### Saving
Write out by streaming pieces: original chunks pass through untouched, add-buffer
pieces are interleaved. Use the File System Access API where available (write to a
new file, never in place), with a download fallback.

### Later
Search (bytes, text, regex) streaming over chunks. Selection ranges, copy/paste.
Bit-level cursor mode in the UI. Column/width presets. Worker-side core so the main
thread never blocks on reads.

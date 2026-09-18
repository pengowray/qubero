# TODO

Notes from Pengo, kept here so they are not lost between sessions. Newest
section first. A ticked item is done; the rest are open.

## 2026-09-17

Text (done this day):

- [x] File type dialog, Source row: drop "which read the file".
- [x] File type dialog, other answers: "names a different format" → "disagrees with the answer above".
- [x] File type dialog, other answers: "disagrees with the answer above" → "differs from the template" / "differs from the file(1) rule", since four rows sat above it.
- [x] Core read failure: "runs past the end of its container" → "field extends beyond its parent" (and the file / stream variants).
- [x] ROOT (`uproot-Zmumu-lz4.root`) hex view: "codes not named one at a time" repeated per step → "compressed block", "unnamed codes", etc.
- [x] Diagram fold buttons: "Show fewer cases" / "Show fewer fields" → "Show less".
- [x] Diagram strips: truncated size or note lines show the full text on hover. (Was showing the box's count tooltip instead.)
- [x] Diagram count tooltip: "This file has 1 patterns" plural.

Open:

- [x] Field docs: `Ty::doc` on a non-struct type dropped the text silently (dbf.rs); now a debug assert, dbf uses `field_doc`, and the doc shows in the inspector under the type line and on hover of the listing name.
- [x] Padding fields sized by `PadTo` now carry a derived doc ("Alignment padding: pads text to a 4-byte boundary. 0 bytes here because text already ends on one."); fixed-size ones like ELF ident padding get a hand-written `field_doc`. Others (mat, tiff `Remaining` padding, wav `pad()`) still say nothing.
- [ ] Alignment: find formats where it matters; a toggle for whether alignment padding is shown, or a way to tag sections that use it (sometimes it is computed).
- [ ] Diagram, boxes and arrows (`busybox-mips`): lines leave right and enter left without crossing, but nobody can follow the tracks. Needs a different mechanism (hover highlighting a route, endpoint labels, or bundled stubs).
- [ ] Diagram, strips: synonyms. When there are several ways to say the same thing, sometimes use the longer form, sometimes the shorter. PNG: `Chunk[], until type 'IEND', ..., chunks last` vs `Chunk[], ..., chunk type 'IEND'`. "chunks" could cover the whole block containing `[chunk] [chunk] ... [chunk IEND]`; `chunks` and `Chunk[]` are redundant here. Boxes inside boxes, but not deeper than that.
- [ ] Diagram, polymorphism: switch cases. PNG covers two chunk types so it is fine, but 10s or 100s (an opcode field with 1000 cases) needs a break-out shown separately. Work out heuristics across a few formats; how much common structure to repeat per subtype has no simple answer. The case list is redundant (`'IHDR' → IHDR`); just list them or leave it to the break-out. FITS image files have huge switches that are cut off with no way to see the rest. MOD files "switch on signature" lists '1CHN' to '99CN' as separate items each saying "computed", which means nothing.
- [ ] Diagram: stylistic simplification when a structure is simple. MIDI-style illustration: `[Header Chunk id:'MThd' size body][Event Chunks: Event Chunk id:'MTrk' size body | ...]`. Could be heuristic or baked in; must not change the IR. Possibly a second, provably equivalent IR just for display.
- [ ] Diagram: large numbers of fields; the boxes-and-arrows view could get mixed in when there are many.
- [ ] Diagram: a way to see actual data without switching to hex view; maybe example data; optionally laid out as hex.
- [ ] Diagram: always allow scroll-zoom out at least as far as "Fit" goes.
- [ ] Diagram loading: show the spinning Qubero logo while the diagram is first built.
- [ ] `chunk-indexes-large.h5`: why is Strips almost empty when Boxes and Arrows is huge?
- [ ] `H-CAL_FAC_V03-729273600-5094000.gwf` (LIGO/Virgo frame): wild; diagrams will be a challenge. `(nFrame < 4294967295) * nFrame × 4 bytes` is surely necessary but far too verbose for display.
- [ ] `channels-structured-v2.npy` strips view: what's with all the datetime64s? Similar in `proto4-unframed-payload.pickle`.
- [ ] Hex view: jump to top of field / previous field / next field when fields are big (e.g. `system_area 32,768 bytes · continued`).
- [ ] Addresses: option for hex (current default) or decimal; needs its own formatting options, e.g. `4000:A002` style.
- [x] Use red for incorrect items (design: DESIGN-wrong-values.md; all of it landed 2026-09-18: magic, enum, flags, floats, declared ranges, table view cells and column counts, WAV/AU/AIFF float samples declared finite, and checksums over a run of at most 64 KiB whose bytes are already here, which are taken without the reader asking. A bigger sum, or one over a stream, is still the inspector's to run): a magic number that does not match the loaded template (visible right after switching template, without clicking), bad checksums, undefined enum values, numbers out of range, unexpected NaN/Inf/-Inf, unusual NaN payloads.
- [x] `proto4-sklearn-pipeline.pickle` hex view: at 0xe5 the SHORT_BINUNICODE opcode is a chip but the length+string after it (`\x05numpy`) gets no chip, so it is not selectable. Many similar cases.
- [ ] `proto4-sklearn-pipeline.pickle` hex view: 1-byte opcodes are presented as an editable text string ("SHORT_BINUNICODE"), which makes them feel like strings. Wrong affordance. Design something better, do not patch it.
- [ ] Saving a diff of edits. Binary diffs are not very standard. Not for a generated zip; there we would let them download a zip with the original files in a folder plus a diff file, or a revert file, or a python script to undo/redo, which becomes a general mechanism.

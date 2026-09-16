# Handover: the value table in the hex view

Written 2026-09-05. Adds to DESIGN.md "The field column".

## What is wrong

A run of values (the samples of a WAV, W4V's 512 six-bit codes, a deflate
block's symbols, a tensor table) is one chip on the row it starts on, `body
72,000 values`, and the rows it covers show nothing on the right but the text
column. The reader can see the bytes and cannot see the numbers they are. Chips
per element were tried and taken out again: a button per value is too many
elements and too much text, and they fold into one chip on purpose.

## The model

A folded run's chip stays where it is. Beside every row the run covers, the
field column also draws a **value table**: one cell per element whose bits fall
on that row, in the order of the bytes, as plain text, laid out so the reader's
eye can go from a byte to its value and back.

One of two layouts, chosen per run per screenful from measured text, the way
`chipfit.ts` chooses how many chips fit:

- **Aligned.** The table is a grid of `bpr × 8` bit columns, sized so a byte of
  it has the pitch of a hex cell when the column is wide enough (so the cells
  sit under the same column numbers as the bytes) and shrinks proportionally
  when it is not. A cell spans the bits of its element, clamped to the row. An
  element that straddles two rows has its text on the row it starts on and a
  **continuation cell** on the next row: same tint, no text. Used when the
  widest text in the run's cells on screen fits the narrowest cell it would get.
  A 24-bit sample at 16 per row is the acceptance case: three-byte cells,
  the sixth of every row split across the row edge, `-394928` readable in each.
- **Uniform.** Equal-width cells, the width of the widest text on screen plus
  padding, wrapped into as many lines as the row needs. The order is the order
  of the bytes. Used when text does not fit its bit width: nibbles, six-bit
  codes, Huffman symbols of varying length, and any element the core marks as
  not stored in one contiguous run of bits. In condensed mode the lines are
  capped like chips (three) and the rest counted on a `+N` cell.

Both report their height to the `RowHeights` ledger before layout, as
`planRowChips` reports `extraHeight`. The two scroll rules hold: no element is
detached while the view scrolls (cells are pooled and refilled in place, keyed
so a cursor move does not rewrite them), and a row's height never depends on
whether it is the top row.

Numbers (`uint`, `int`, `float`) are right-aligned in their cell; everything
else left. Cells take the run's field family tint (`fieldClass`), lighter than
a chip, with a hairline between cells. The cell holding the cursor gets the
active-field accent the hex cell gets. Clicking a cell selects the element
(`onPick([...runPath, index])`) in all three views.

Values arrive asynchronously, like spans. The last table stays on screen until
the next one is ready (the pattern of commit 9e6ec99 for chips).

The table is on whenever the field column is (all four `fields`/`both` modes,
the default with a template). No new toggle.

## Contract between core and web

Core (`crates/core/src/eval/`):

```rust
pub struct Cell {
    /// Index of the element in its run: the last path step.
    pub index: u64,
    pub offset_bits: u64,
    pub size_bits: u64,
    /// What the listing would say on a shared row (`brief`), or the symbol's
    /// name for a traced block (`literal 'a'`, `match 3 back 12`, `end of
    /// block`), so the two views agree.
    pub text: String,
    /// "uint" | "int" | "float" | "bytes" | "str" | "enum" | "flags" |
    /// "composite" | "symbol"
    pub kind: &'static str,
    /// False when the element's bits are not one contiguous run (a Q5 weight's
    /// fifth bit lives elsewhere). The web then uses the uniform layout.
    pub contiguous: bool,
}

/// The elements of the folded run at `path` whose bits overlap
/// `from..to`, in file order, at most `max`. `path` is the run a `Span` with
/// `count > 0` names (its `path`), or a traced block.
pub fn run_cells(&mut self, doc, path, from_bit, to_bit, max) -> R<Vec<Cell>>
```

- Fixed-stride arrays: index from `(from − run.offset) / stride`, then
  `children`. Check first that `node(doc, [..run, i])` on a large array is O(1)
  through `stride` (eval/mod.rs ~1065) and not a walk; if it walks, add the
  fast path.
- Variable-length elements (MIDI events, varints): walk children from the last
  element known to start before `from`; stop at `max`. Keep it cheap enough for
  one call per frame on a screenful: cache the last (index, offset) reached.
- Traced blocks (`Ty::Traced { part: Block(i) }`): the steps in
  `BlockView.symbols` whose `in_bits` overlap the window; `kind: "symbol"`,
  text as `traced.rs` names the step.
- Struct runs (a record per element): **one cell per record**, text the
  record's one-line reading when it has one (`one_line`) or its name otherwise.
  Leaves are not cells this round.
- Pending bytes come back as `EvalError::Pending` like `children` does.

Wasm (`crates/wasm/src/lib.rs`): `run_cells(space, path, from_bit, to_bit,
max) -> String`, same envelope as `template_children`. `doc.ts`:
`runCells(path, fromBit, toBit, max): TemplateReply<Cell[]>`.

Web (`web/src/`): a new `valuetable.ts` (planning, pure, tested under
`node --test`) and `valuecells.ts` (DOM, pooled elements), used from
`hexview.ts`'s `drawNotes`. The plan for a row says which layout, the cells'
grid columns or widths, the lines it adds, and a key string so the DOM is
written only when the text changes.

## Strings

Written here; agents use them verbatim.

| Where | Text |
|---|---|
| Cell tooltip | `{run}[{index}] · {type} · {text}` e.g. `body[5] · i24 le · -394928` |
| Symbol cell tooltip | `symbol {index} · {text} · {n} bits` |
| Continuation cell tooltip | `{run}[{index}] · continued from the row above` |
| Continuation cell aria-label | `continued from the row above` |
| `+N` cell (uniform, condensed) | text `+{N}`, tooltip `{N} more values on this row` |
| Struct record cell (no one-line reading) | the record's name as the listing shows it |

No new words for the reader beyond these. Nothing in a cell is reformatted by
the web: text comes from the core's `brief`.

## Headings: space above

Unrelated, same round. Every heading line in the hex view gets space above it
(level 0 more than level 1), except the heading for the part that starts at
offset 0 of the file. The space is inside the fixed heights (`--hv-heading`,
`--hv-subheading`) as top padding, since `rowheights.ts` reads those back;
`fillHeadings` marks the offset-0 heading with a class and `rowheights.ts`
knows its smaller height. Keyed on file position, never on screen position (the
existing `.hv-row:first-child … :first-child` rule keys on the top of the
screen and stays as it is: it only hides a border).

## Verification

Headless Playwright (global install) against `web-samples` on its own port:
`bat.wav` (16-bit samples, aligned), `bat.w4v` (six-bit codes, uniform),
`tune.mid` (variable-length events), `notes.zip` (deflate symbols), `hello.exe`
(mixed), and `wave-extensible-pcm-5.1-48000.wav` if it is in the sample
collection (24-bit, the straddling case). Screenshots of each for review.
Then `web/tools/touchscroll.mjs` on `bat.wav` and `notes.sqlite`, expecting 0
jumpy steps. Core changes need `npm run wasm:core` (~35 s) before the web sees
them.

## Work packages

- **A, core and wasm**: `run_cells`, the wasm export, `doc.ts` binding, unit
  tests in the core over a fixed array, a variable-length array, a traced
  block and a struct run.
- **B, the view**: `valuetable.ts`, `valuecells.ts`, `hexview.ts` wiring,
  `rowheights.ts`, `style.css`, the heading space, DESIGN.md "The field
  column". Prototype the aligned layout on `templateChildren` for the WAV case
  until A lands, then switch to `runCells`.

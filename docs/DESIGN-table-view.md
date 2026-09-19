# Table view

A list of records, or a run of samples, opened in a tab of its own as a table.
This is the first view that is an abstraction away from the bytes: a row is a
record or a sample, not a stretch of the file. The link back to the bytes is
kept, but as an option the reader turns on, not as the frame the table sits in.

## Two lenses

The same table can be read two ways, and the reader picks:

- **Meaning.** Rows are what the format says they are: samples of each
  channel, records of a dBase table. Columns are the record's fields or the
  channels. A time column appears when the format gives a rate. This is the
  default.
- **Data.** The same rows with the byte address (and size) of each row as
  extra columns, so every row can be traced back to where it is stored. A
  checkbox in the tab's bar turns this on; it is remembered per session.

Clicking a row always puts the file tab's cursor on that row's bytes and marks
them, whichever lens is on. That is an action, not a column. Shift-click and
shift-arrow extend the selection to a range of rows; the mark then covers the
whole range. The selected rows can be copied as tab-separated text with a
heading line, from a `Copy {n} rows` button in the bar or Ctrl+C, so a run of
samples or a set of records pastes into a spreadsheet as it is on screen.

Columns are as wide as what has been seen in them: the heading to start with,
widened by the longest value in the rows read so far, never narrowed, capped at
40 characters. A table of one channel is a narrow table, not one number adrift
in a tab-wide column. Values whose kind is a number sit against the right edge
so their digits line up; the side is decided by the first value seen in the
column and kept. The header shares the rows' monospace face so the character
arithmetic holds for it too.

A cell whose value is wrong (a NaN sample under a finite constraint, an
undefined enum in a record) is marked the way DESIGN-wrong-values.md says:
glyph and hover text on the cell, a count in the column header. The count
follows the heading after a dot rather than in brackets, because the unit is
already in brackets: `left (dB) · 2 invalid`. It says `so far` until every
row has been read, which for a run of samples is never. The heading may widen
its column to hold the count, since the whole point of the count is that it can
be read without scrolling the table.

## Where it comes from: IR first, heuristic second

`web/src/records.ts` says the registry stays until two formats declare their
columns beside their fields. dBase (`values` named per element from `fields`)
and NPY structured dtypes (`elem_name_from`) are those two, and WAV's channel
count and sample rate cannot be found by any heuristic. So:

1. **IR.** A `Field` can carry a `TableShape`. It says how many elements make
   one row, what the columns are called, the units they are measured in, what
   one row is called, the rate rows come at, and which fields describe the
   table (the facts shown above it). The template that knows the format writes
   it. See `crates/core/src/template.rs`.
2. **Heuristic.** A list node with no shape still gets the table when its
   elements are uniform records: the first few elements are composites with
   the same field names in the same order, with at least two leaf columns. One
   nested list level is flattened, so dBase's `{deleted, values[]}` becomes
   `deleted | NAME | AGE | ...` with the element names the file gave. A shape
   with no `columns` on a list of records (dBase's, which names the row and
   the facts) gets its columns the same way, from the first record. Nodes
   `records.ts` already claims (SQLite, GGUF) are not offered twice; their
   plans feed the tab. Scalar runs without a shape are not offered: the list
   pane already shows those.

## IR

```rust
pub struct TableShape {
    /// Elements per row. None: one element is one row.
    pub columns: Option<Expr>,
    /// Column names, used when there are exactly this many columns.
    pub names: Vec<Arc<str>>,
    /// Units per column, parallel to `names`, as UCUM codes ("s", "Hz", "dB",
    /// "" for none). Shown in the header as `name (unit)`. A crate like `uom`
    /// could validate these later; for now they are strings the view shows.
    pub units: Vec<Arc<str>>,
    /// What to call a column when `names` does not fit: "channel" gives
    /// "channel 1", "channel 2", ...
    pub column_word: Option<Arc<str>>,
    /// What one row is: "sample", "record".
    pub row_word: Option<Arc<str>>,
    /// Rows per second, when rows are spaced in time. Gives the time column.
    pub rate: Option<Expr>,
    /// Fields that describe the table, shown above it with links to where
    /// they are stored: sample rate, channels, bits per sample.
    pub facts: Vec<Expr>,
}
```

`Field.table: Option<Arc<TableShape>>`, set with `Ty::field_table(field, shape)`
on the structure the same way `field_elem_named_from` is. `NodeInfo.table`
is true when the field has one. `template_text` renders it as a field extra
(`table columns <expr>, rate <expr>`), and `machinery::ty_refs` includes the
shape's expressions so the relations and graph views show the samples
depending on `fmt.channels` and `fmt.sample_rate`.

WAV wraps the `data` case in a `Samples` structure with one field `samples`, so
the shape sits beside the array it describes rather than on `Chunk.body`, which
every chunk shares. AIFF and AU do the same where they read samples.

## Core and wasm

`table_shape(space, path)` returns the shape evaluated at the node:

```json
{"status":"ok","node":{"columns":2,"names":["left","right"],"units":["",""],
 "column_word":"channel","row_word":"sample","rate":44100,
 "facts":[{"label":"fmt.sample_rate","path":[3,0,2,2],"value":"44100"}]}}
```

`node` is null when the field has no shape. Facts are origins collected by
`from_expr`, so they carry the same label, path and value the At-cursor panel
shows. A shape whose `columns` or `rate` cannot be evaluated answers with that
part null; a pending read answers pending like every other call.

## Web

- `tableplan.ts`: `tablePlan(doc, node): TablePlan | null`. Decides whether a
  node is a table (shape, records.ts plan, or heuristic), and builds a lazy
  plan: `rows` (count), `columns` (name, unit, kind), `meta` (facts), `time`
  (rate or null), `row(i)` returning `RecordCell[]` plus offset and size, or
  null while its bytes are pending. Rows are fetched with `templateChildren`
  in windows; a shape with `columns = k` makes row `i` from elements
  `i*k .. i*k+k`. Pure enough to test under `node --test` with `TemplateNode`
  fixtures.
- `tableview.ts`: the tab page. Which way round the table is drawn, the rows as
  they are drawn, the selection and the copy. Row click picks the row, or the
  cell where the cells have addresses of their own; the file tab's cursor
  moving into the table selects and scrolls to the row. Keyboard: arrows, page,
  home, end. The parts that stand on their own are modules around it:
  - `tablebar.ts`: the bar. The title, the count, the facts (links), the
    row-meaning line, the rows-or-columns choice, the address checkbox, and
    the two buttons the view owns.
  - `tablecolumns.ts`: how wide each column is, which side its values sit,
    what its heading says with the count of wrong values in it, and the header
    row itself, the right way up and turned.
  - `tablescroll.ts`: which rows the scroll position is asking for and where
    to draw them, with the scroll position mapped to row index by ratio when
    the row count exceeds what a canvas can be tall enough for (a ten-minute
    stereo WAV is 26 million rows).
  - `tableaddress.ts`: where a row, a cell or a turned table's drawn row is
    stored, said in words.
  - `tabletext.ts`, `tableexport.ts`, `tableexportpanel.ts`: the table as
    text, and saving it as a file.
- `tabs.ts`: a tab has a `kind`: the document, or a table of a path in a
  document. `forSpace` skips table tabs; `close` does not ask about unsaved
  edits for a table tab; the edited mark is only on document tabs.
  `forTable(doc, path)` brings an open table to the front.
- Buttons, all reading "View samples as table" with the row word: the
  At-cursor panel (beside "Open unpacked", for the field at the cursor or the
  nearest enclosing table), the listing heading (beside "Show all in pane"),
  and a run chip in the hex view on double-click, the same gesture that opens
  an unpacked stream.

## Which way round (2026-09-19)

A table means one thing: a row is a record, a column is a field. The plan
answers in records, and every copy and export is made from that by
`tabletext.ts`. Which way round the view DRAWS it is a separate state, `turned`,
with a choice in the bar, `{Rows} in: rows / columns`.

- **Default.** `turnsByDefault`: the table can be turned (at most `TURN_MAX`,
  1,000, records, since a turned row needs every record read), its columns are
  only places in a list, and there are more than 20 of them and at least four
  to every row. Two TDMS channels of 500 values arrive as two columns.
- **Kept.** `qubero.table.turned`, written only when the reader flips a table
  that `turnsByDefault` picks out, and read only for those. Flipping a table of
  records lasts for that tab: it should not turn every dBase file from then on.
- **What turns.** The rows that scroll and are selected are fields. The facts
  about a record (number, name, time, address, size) are columns the right way
  up and lines of sticky heading turned, written by the same `turnedLines` a
  copy uses. A click goes to the clicked cell's own bytes (`TableRow.spans`),
  or to the whole record where the plan has no spans.
- **What does not.** Export writes a record to a row unless the reader picks
  `columns, as on screen`; JSON is always one object per record. Copy
  is of the rows on screen and so follows the view.
- **Headings.** Columns that are all `[n]` are headed `n` (`plainIndexes`);
  one named column among them keeps the brackets on all. Rows the format names
  (TDMS channel paths) get a `name` column.
- **Header.** Inside the scroller, `position: sticky`, in a sheet that is
  `max-content` wide, so it scrolls across with the rows; the row number (or,
  turned, the field label) is sticky at the near edge.
- **Not done: columns are not virtual.** Every column of every drawn row is an
  element, about 30 microseconds each: 2 rows of 500 is nothing, 50 rows of 500
  is most of a second a repaint, and 4,000 columns that tall would not scroll.
  A table both wide and tall needs a column window like the row one; the
  tracks are fixed `ch` widths, so the arithmetic is prefix sums.

## Where the cells are (2026-09-19)

Most tables have a row that is a run of the file, and `Stored at` and `Size`
say where it starts and how long it is. The tables the core works out -- a
pandas frame, a torch tensor, a checkpoint's summary -- do not. A frame's row
is one value out of each of several blocks, and at protocol 2 two of them can
be in two different address spaces. So `FrameCell.at` says where each cell is
or why it has none (`counted`, a `RangeIndex` label worked out from a start and
a step; `nowhere`, a place this reading cannot find), and `RecordCell` carries
that through to the view. `tableaddress.ts` holds the words, because the
columns on screen, a copy and an exported file all have to say the same thing.

- **A row.** `rowRun` over its cells: one run of one space gives an address and
  a length; cells in several places give the first cell's address, `per cell`
  for the size, and the reason on hover; a row with no placed cell at all says
  `computed` or `unknown` instead of an address of nought.
- **A cell.** Its own address is a line on its hover, under its value and under
  whatever is wrong with it, and only while the address columns are on. An
  address in an unpacked stream gets the line that says what the `+` counts
  from, since a hover cannot carry a hover.
- **A click.** The cell that was clicked is what the file tab is sent, when the
  cell has bytes in the tab's own space. A cell elsewhere, or with no bytes,
  leaves the cursor where it was rather than moving it somewhere unasked.
- **Turned.** `Stored at` and `Size` are about the record, so turned they are
  two of the heading lines, a cell to a record, exactly as `turnedLines`
  writes them. What the drawn row is then -- the same field of every record --
  has an address of its own, on the row's heading: a frame keeps a column's
  values in one block, so the run the unturned table could only call `per cell`
  reads straight down the page. `columnPlaces` collects them, from the cells'
  own addresses or, for a table read from a list's elements, from
  `TableRow.spans`. Hover and click follow the drawn cell either way up.
- **Copy and export.** `recordParts` writes the same two cells the columns
  draw, so a pasted row does not claim bytes its table never held. An export
  has them only when the address columns are on, like every other column: the
  file is what was on screen.

## Strings

| Where | String |
|---|---|
| Buttons | `View {rows} as table` ({rows} = plural row word: samples, records, values) |
| Tab title | `{field} as table · {file}` |
| Tab tooltip | `Table of {rows} from {field} of {file}` |
| Count | `{n} {rows}` |
| Row meaning, with rate and columns | `Each row is one {row} of each {column word}, 1/{rate} s apart.` |
| Row meaning, with rate only | `Each row is one {row}, 1/{rate} s apart.` |
| Index column | `#` |
| Time column | `time (s)` |
| Unnamed column | `{column word} {n}` |
| Address columns (data lens) | `Stored at`, `Size` |
| Address checkbox | `Show byte addresses` |
| Size, cells in several places | `per cell`; hover, on both columns `This row's cells are in different places. Hover a cell for its address.` |
| Address, no bytes | `computed`, `unknown`; hover `No bytes: computed from the index's start and step.`, `No bytes: unknown location.` |
| A cell's address, on its hover | `{address} · {size}`, then `Offset within the unpacked stream` for an address in one |
| Row name column | `name` |
| Rows-or-columns choice | `{Rows} in:` `rows` / `columns` (radio pair); hover `One {row} per row, or one {row} per column. (Display only)` |
| `columns` greyed, too many records | hover `Too many {rows} to show as columns. The limit is 1,000.` |
| Column meaning, turned | `Each column is one {row} of each {column word}, 1/{rate} s apart.` |
| Copy button, nothing selected (disabled) | `Copy selected rows`; hover `Select rows to copy them as tab-separated text` |
| Copy button, rows selected | `Copy selected row`, `Copy {n} selected rows`; hover `Copy the selected rows to the clipboard as tab-separated text, with their headings (Ctrl+C)` |
| Copy notice | `Copied {n} rows as tab-separated text.` |
| Export button | `Export...`; while saving `Stop export` |
| Export form | `Export`: `Whole table`, `Selected rows only ({n})`; `Format`: `CSV`, `TSV`, `JSON`; `{Rows} in` (turned only): `rows`, `columns, as on screen`; `Save file` |
| Export size | `Writes {n} rows of {m} columns, under one heading row.` / `..., as on screen.` / `Writes {n} objects, one per {row}.` |
| Export notices | `Exporting row {n} of {total}...`, `Exported {n} rows as {format}.`, `Export stopped.`, `Couldn't export: {message}` |
| Copy refused, too many | `Selection too large to copy: {n} rows, limit 100,000.` |
| Copy refused, still reading | `Rows are still loading. Try again in a moment.` |
| Copy failed | `Couldn't copy to the clipboard.` |
| Waiting cell | `Reading…` (REPORT.paneWaiting) |

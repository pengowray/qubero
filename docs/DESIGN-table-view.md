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
them, whichever lens is on. That is an action, not a column.

A cell whose value is wrong (a NaN sample under a finite constraint, an
undefined enum in a record) is marked the way DESIGN-wrong-values.md says:
glyph and hover text on the cell, a count in the column header.

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
   `deleted | NAME | AGE | ...` with the element names the file gave. Nodes
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
- `tableview.ts`: the tab page. A bar with the title, the count, the facts
  (links), the row-meaning line, and the address checkbox. A sticky header
  row. Virtual rows of a fixed height, with scroll position mapped to row
  index by ratio when the row count exceeds what a canvas can be tall enough
  for (a ten-minute stereo WAV is 26 million rows). Row click picks the row;
  the file tab's cursor moving into the table selects and scrolls to the row.
  Keyboard: arrows, page, home, end.
- `tabs.ts`: a tab has a `kind`: the document, or a table of a path in a
  document. `forSpace` skips table tabs; `close` does not ask about unsaved
  edits for a table tab; the edited mark is only on document tabs.
  `forTable(doc, path)` brings an open table to the front.
- Buttons, all reading "View samples as table" with the row word: the
  At-cursor panel (beside "Open unpacked", for the field at the cursor or the
  nearest enclosing table), the listing heading (beside "Show all in pane"),
  and a run chip in the hex view on double-click, the same gesture that opens
  an unpacked stream.

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
| Waiting cell | `Reading…` (REPORT.paneWaiting) |

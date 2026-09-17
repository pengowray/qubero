# Wrong values

A field whose value is not what its format allows is marked wherever the value
is shown, without the reader clicking anything. The mark says what is wrong in
words next to the value, in red where the format itself rules the value out
and quietly where Qubero merely has no name for it.

Status: design, no code. Written 2026-09-17 from TODO.md's "Use red for
incorrect items" line.

## What exists already

The core already knows most of this and the web throws it away.

- `Value::Magic { ok, bytes, expected }` (eval/read.rs) and
  `magic_reading` print `PK\3\4 does not match \x89PNG` into the value
  column. That is the only trace of a mismatch in any view.
- `Value::Enum { name: None }` prints `{num} (unknown)` from the wasm node,
  but the listing's own `brief` (eval/listing.rs:80) prints the bare number
  with no marker, so the two views disagree about the same field.
- `Value::Flags { unnamed }` prints `+3 unnamed`.
- `NodeDto.ok` (wasm/src/lib.rs `shown`) is false for both of the above and
  is read by no file under `web/src`. `TemplateNode.ok` exists in doc.ts and
  is unused. `SpanDto` (hex chips) and `CellDto` (value table) carry no
  status at all, so those two views cannot say anything is wrong today.
- Checksums: `Check`s in thirteen format files, run by `run_check` only when
  the inspector shows the field. The inspector runs one on its own under
  `AUTO_CHECK_BYTES` (1 MiB) and offers `Check the CRC-32` above that. The
  verdict lives in the Integrity section and nowhere else. ZIP placeholder
  sums have their own states (`ARCHIVE_SUMS`). `.insp-check-result.ok`
  colours with `--good`, which no stylesheet defines, so Valid is not green.
- Kaitai `valid:` is parsed in full (`ValidSpec`: Eq, Min, Max, Range,
  AnyOf, InEnum, Expr) and one case survives: `valid: [bytes]` on a bytes
  field becomes `Ty::magic`, as `contents:` does. The rest are dropped with a
  conversion note ("templates check checksums, not a field's value"). ImHex
  `std::assert` becomes a note on the field. Neither reaches the reader as a
  check.
- Floats print through `f64::Display` in core (`NaN`, `inf`, `-inf`) and the
  web's float lens prints `Infinity` and `-Infinity`, so the value column and
  the At-cursor lens spell the same double two ways. The type panel's bit
  layout (d0c677e, typepanel.ts) already names quiet and signalling NaN,
  infinities, subnormals and the x87 pseudo forms, with the fraction as the
  payload. That knowledge does not reach the node.
- `--warn` is the one alarm token, used in eighteen places: `.insp-check-result
  .bad`, `input.invalid`, the toolbar message, `.dlg-disagrees` in the file
  type dialog, the converter panels' gap counts, and `.is-warn` notes on the
  graph and diagram views. There is no `--good`, `--err` or `--warn-soft`
  (`--warn-soft` is referenced once and falls back to blue). Chip fill is
  `--field-color`, one colour per value kind, and style.css:347 already
  states the rule this design follows: "overlapping UI state is carried by
  background, underline and outline instead."

## Two tiers, and a third state that is neither

Who says the value is wrong decides how loudly it is marked.

**Invalid.** The format rules the value out. A magic number that is not the
one the template requires; a checksum that does not equal the sum of the bytes
it covers; a value outside a range or set the format specification states
(Kaitai `valid`, ImHex `std::assert`, or a template's own `Field::valid`).
Marked in red, with the reason. A file with one of these is damaged, truncated,
not the format the template thinks it is, or written by a tool that got the
specification wrong. All of those are things the reader came to find.

**Undefined.** Qubero's tables have no name for the value, and the format may
or may not allow it. An enum value with no case; flag bits nobody named; a
NaN or infinity in a float field whose format did not say values are finite;
a NaN whose payload is not the canonical quiet NaN. DESIGN.md already commits
to "a file is allowed to hold a value its format never defined", so these are
not red. They get a glyph and the words, in the muted colour, so a reader
scanning for damage is not pulled to a wasm opcode Qubero has not catalogued.

**Not checked.** A check that could not be made is neither. A checksum over a
run too large to sum on its own, or over bytes not loaded, or over an unpacked
stream whose decoder is not present; a `Check::when` guard that did not hold;
a ZIP placeholder sum of 0 waiting for `sumjob`; a magic field whose bytes
have not arrived. These keep their existing `Not checked · why` wording in the
inspector and carry no mark anywhere else. The rule is DESIGN.md's checksum
rule: a fact the core cannot establish is nothing, not a guess.

A template can promote a case from undefined to invalid by declaring the
constraint: WAV float samples declared finite make a NaN sample invalid; a
Kaitai `valid: {in-enum: true}` makes an unnamed enum value invalid.

## The six TODO cases

| Case | Tier | Where it is decided | Runs on its own? |
|---|---|---|---|
| Magic does not match the loaded template | invalid | `Value::Magic.ok`, already | yes, already |
| Bad checksum | invalid | `run_check`, already | only under the eager cap (below) |
| Undefined enum value | undefined | `Value::Enum.name == None`, already | yes, already |
| Unnamed flag bits set | undefined | `Value::Flags.unnamed`, already | yes, already |
| Number out of range | invalid | new `Field::valid` | yes, one expression |
| NaN, +Inf, -Inf | undefined, or invalid under `Field::valid` | new, in `shown`'s float arm | yes |
| Unusual NaN payload | undefined | new, same arm | yes |

## IR

### A constraint on a field

```rust
/// What a field's value has to be, when the format specification says.
pub enum Valid {
    /// Equal to this.
    Eq(Expr),
    /// At least this.
    Min(Expr),
    /// At most this.
    Max(Expr),
    /// Between these, inclusive.
    Range { min: Expr, max: Expr },
    /// One of these.
    AnyOf(Vec<Expr>),
    /// Named in the field's enum. Promotes an unnamed value to invalid.
    InEnum,
    /// This expression is non-zero. `Expr::less_than` and `equals` exist,
    /// so any comparison is writable; `This` is the field's own value.
    Expr(Expr),
    /// A float that is not NaN and not infinite.
    Finite,
}
```

`Field.valid: Option<Valid>`, set with `Ty::field_valid(name, Valid::..)` the
way `field_doc` and `field_table` are. It mirrors `ksy::spec::ValidSpec` case
for case so the Kaitai converter lowers `valid:` instead of noting it, and the
ImHex converter lowers a `std::assert(cond, msg)` whose condition names only
the field it sits after. Asserts the converters cannot express stay notes.

The check is evaluated where `Field::checks` expressions are: at the end of
the structure the field sits in, so a bound may name a later field. Reading a
value never fails on a constraint. The field is read as declared, the value
is what it is, and the verdict is a second fact about it. This is the
existing rule for magic and for checks, and it keeps switches keyed on the
field working.

The expression walk gets one new arm, `Expr::This`, in its own cold function
per the evaluator-frame rule (see memory: new byte-reading Expr arms go in a
cold fn returning `R<T>` or the depth tests overflow).

### Carrying the verdict

`NodeDto.ok` goes. In its place, on `NodeInfo` and `NodeDto`, mirroring
`doc`:

```rust
/// What is wrong with this value, when something is.
pub struct Problem {
    pub tier: Tier,          // Invalid | Undefined
    /// One line, state first: "Does not match: expected \x89PNG".
    pub text: String,
}
pub problem: Option<Problem>,
/// How many descendants carry a problem, by tier. Zero on a leaf.
pub problems_within: (u32, u32),
```

The same `problem` goes on `SpanDto` and `CellDto`, since the hex chips and
the value table are built from those and not from nodes. The text is built in
core, as `magic_reading` is, so the listing, the chip tooltip, the inspector
and the table say the same words about the same bytes. The float text is
built there too, and `showFloat` in lenses.ts is changed to print what core
prints, so a double is spelled one way.
The value column stops carrying the magic reason: `magic_reading` prints the
bytes only, and the reason moves to `problem.text`. The `(unknown)` suffix on
an unnamed enum value and the `+{n} unnamed` suffix on flags stay in the
value text, since that is how the rest of the app says it and the words
already carry the fact. The listing's `brief` is made to print them too.

`problems_within` is what an ancestor shows. It is counted over the children
the core has already read, never by reading more: the listing walks a
structure's fields when it draws them, and a collapsed run of 40,000 records
has had a handful read, so its count is a count so far and the row says so.
The count becomes exact when the run is opened or the table view reads it
through. No view triggers a walk of the file to complete a count.

### Which checks run on their own

Everything except checksums is one expression or one comparison against a
table and runs during `node`. Checksums read bytes, so they run on their own
only when the covered run is at most `EAGER_CHECK_BYTES` (64 KiB, the
`gapcheck` cap and the same reasoning: past this, nobody looked) and every
byte of it is already loaded. That is smaller than the inspector's own
`AUTO_CHECK_BYTES` (1 MiB) on purpose: the inspector sums one field the
reader is looking at, and this sums every field the listing draws. A check
that fails the test is Not checked and unmarked until the reader opens the
inspector, where the 1 MiB rule and the `Check the CRC-32` button stay as
they are.

An eager check is cached on the node like the value is and invalidated with
it, so scrolling the listing does not re-sum a PNG's chunks.

## Where it shows

The key fact is *this file has something wrong in it, here*. The reader must
get it from the toolbar without scrolling, find the field from any view, and
read the reason without opening a panel.

**Toolbar, file type answer.** When the root's magic field is invalid, the
template answer is not "Qubero read the file's structure" any more, because
it did not. `identity.ts` gets the root problem and the answer's evidence
line becomes `Template {label} was applied, but the signature does not match`
with `disagrees: true`, so the file type dialog lists it with the
`.dlg-disagrees` class it already uses for a file(1) rule that names another
format. This is what makes a template switch visible at once:
picking PNG for a ZIP turns the toolbar answer red before anything is clicked.
The menu-pick path in main.ts does not currently update the toolbar answer at
all; it has to, for this to work.

**Listing.** An invalid leaf gets a glyph before the value, the value text in
`--warn`, and the reason in the muted colour after it on the same row,
truncated to the column with the full text on hover. An undefined leaf gets
the glyph only, with the reason on hover: the value already says `(unknown)`
or `+2 unnamed`, and repeating it in a second phrase would make the quiet
tier as long as the loud one. A structure or run row with `problems_within` above zero
shows the count at the end of its row: `· 3 invalid`, `· 12 undefined`, both
when both. The count is a button that opens the row and scrolls to the first.

**Hex view chips.** The chip keeps its kind fill: a red chip would hide what
kind of field it is, and colour is already spent on kind. An invalid chip gets
a 2px `--warn` underline and the glyph at its left edge; an undefined chip gets
the glyph only. The chip tooltip appends the problem text as its own line.
A structure chip gets the count in its tooltip and no mark on the chip.

**Inspector.** A new row directly under the value, before Date & time and
Integrity, reading the problem text with the glyph. For a magic field the
existing expected/actual bytes stay in the type table. For a checksum, this
row and the Integrity result slot say the same thing; the row is the summary
and Integrity stays the place with the ranges. For a float, the bit layout
row names the NaN kind: `quiet NaN, payload 0x1`, `signalling NaN`,
`+infinity`.

**Table view.** A cell with a problem gets the glyph and, for invalid, the
`--warn` text colour. The column header shows a count when any cell in a
loaded row has one: `left (2 invalid)`. Hover on the cell gives the text.
Sorting or filtering by problem is a later addition; the header count is what
says there is something to look for. DESIGN-table-view.md gets a line
pointing here.

**Diagram, strips and boxes.** Nothing. The diagram is drawn from the IR
(eval/diagram.rs) and shows the format, not this file, so it has no value to
mark. A file that fails its signature is reported by the toolbar instead.

**Treemap.** Structure mode gives a box with `problems_within` or its own
problem a `--warn` outline. The hover line adds the count.

**Relations and graph view.** Nothing. They show dependence, not values.

**Overview panel and Contents.** A Contents heading gets the same trailing
count as a listing structure row. The overview panel's two facts at the top
become three when the document's root count is non-zero: `2 invalid values, 5 undefined · Show
first`, which is the single place a reader can start from without knowing
where to look.

### The glyph

One glyph for both tiers, so a reader learns one shape: a small solid circle
`●` at 0.6em, coloured `--warn` for invalid and `--muted` for undefined. Not a
triangle or an exclamation mark, which say "warning" and "error" and would
put a third vocabulary beside the words. The words carry the tier; the colour
repeats it; the glyph only says "look here" and is what remains in
monochrome. A screen reader gets the text row, not the glyph.

### Colour rules

- `--warn` for invalid only. It already means hard failure in
  `.insp-check-result.bad` and edit refusals, and nothing else may take it.
- Undefined uses no colour beyond `--muted` text. If a reader cannot tell the
  two apart at a glance, that is the intent: undefined is not a finding.
- Never fill. Fill is kind. Underline, text colour, outline and glyph are the
  channels for this.
- Every mark has its words within one hover. Nothing is colour alone.

## Editing and reading

A marked field is as editable as an unmarked one. Writing the expected magic
or the computed sum over a wrong one is an ordinary undoable edit, and the
inspector offers `Update to: …` for a checksum the way `ARCHIVE_SUMS` does for
a placeholder. A template never refuses to read because of a constraint; red
describes the file, not Qubero's willingness to open it. The one thing a
constraint may change in reading is nothing: a switch keyed on an invalid
value still takes the case for the number that is there.

## What this does not do

- Detect things the template does not declare: an ELF section table past the
  end of the file is a read failure, already reported as one, not a wrong
  value.
- Validate strings (encoding failures are already shown as hex and refused
  for edit).
- Sort or filter a listing by problem. The counts and `Show first` are enough
  for a first cut.
- Sum anything large without being asked.

## Strings

State first, then cause, matching `Not checked · why`.

| Where | String | Note |
|---|---|---|
| Problem text, magic | `Does not match: expected {bytes}` | value column shows the actual bytes only |
| Problem text, checksum | `Mismatch: computed {sum}` | value column shows the stored sum |
| Problem text, range | `Out of range: must be {min} to {max}` | also `at least {min}`, `at most {max}` |
| Problem text, set | `Not allowed: must be one of {a}, {b}, {c}` | up to four listed, then `and {n} more` |
| Problem text, equality | `Must be {value}` | |
| Problem text, expression | `Fails the check: {msg}` | ImHex's assert message when given, else the expression in the template's own text; the expression always on hover; uncertain |
| Problem text, enum, undefined | `Undefined in {enum}` | on hover and in the inspector; the value keeps `{num} (unknown)`; agreed 2026-09-17 |
| Problem text, enum, invalid (InEnum) | `Not allowed: undefined in {enum}` | |
| Problem text, flags | `{n} unnamed bits set` | replaces `+{n} unnamed` in the value |
| Problem text, float | `Not a number (quiet NaN)` / `Not a number (signalling NaN, payload 0x{p})` / `Infinity` / `Negative infinity` | |
| Problem text, float, invalid (Finite) | `Not allowed: not a number` etc. | |
| Structure row count | `· {n} invalid` / `· {n} undefined` / `· {n} invalid, {m} undefined` | with ` so far` while pending |
| Overview line | `{n} invalid values, {m} undefined · Show first` | either half dropped when zero |
| Toolbar evidence | `Template {label} was applied, but the signature does not match` | |
| Table header | `{column} ({n} invalid)` | |
| Inspector, checksum action | `Update to: {sum}` | exists in `ARCHIVE_SUMS` |

On `Undefined in {enum}`: the value text keeps `{num} (unknown)`, which is
what the app says for an unnamed enum value elsewhere and is fine on a row.
The problem text is for the hover and the inspector row, where naming the
enum is the context that says whose table has no entry. `Not a named value
of {enum}` and `No name for {num} in {enum}` were considered; the short form
was chosen, and the word matching the tier is a feature: the count row says
`2 undefined` and each of those rows says `Undefined in …` on hover.
`{enum}` is the enum type's own name, which is the field name for most
built-in templates and the declared type name for ImHex and Kaitai ones.

Sign-off still wanted on the expression case.

## Order of work

1. Core: `Problem` on `NodeInfo`, built for magic, enum, flags, float. Drop
   the reason from the value text. `problems_within` in the child walk. Tests
   on `template_text` snapshots and a `table_shape`-style wasm test.
2. Web: read `problem` in listing, chips, inspector. The glyph and CSS.
   Overview line. This alone closes the magic and enum cases.
3. Toolbar: root problem into `identity.ts`; menu picks update the answer.
4. `Field::valid`, `Expr::This`, WAV and one Kaitai format as the two
   templates that pay for it; the converters lower `valid` and simple asserts.
5. Eager checksums under the cap, cached on the node.
6. Table view cells and header counts, once the table tab exists.

# Kaitai Struct in Qubero

Four pieces of work, planned 2026-09-13. Read this before touching any of
them. The Kaitai checkout is at `D:\github\kaitai_struct` (recursive clone,
version 0.11 compiler, 185 formats, 339 test formats with 60 binaries and 320
expected-value specs). Measurements below come from
`tools/ksy_stats.py` run over that checkout; rerun it rather than trust
the numbers if the checkout moves.

1. **Converter**: `.ksy` text in, a `Template` and a conversion report out.
   Rust, in the core, because the IR is Rust-only and never crosses the wasm
   boundary as data.
2. **Bundled formats**: the permissively licensed `.ksy` files for formats
   Qubero does not parse yet, compiled in and offered like builtins.
3. **Live converter panel**: paste or drop a `.ksy`, see the IR it became as
   text, see what could not be expressed, apply it to the open file.
4. **Structure diagram view**: the format as boxes and arrows, drawn from our
   IR. This is the Kaitai *diagram* (the Graphviz output on formats.kaitai.io),
   not `ksv`, whose tree-and-hex pair Listing and Hex already are.

## Fixed decisions

### The IR stays the IR
The converter targets `Template` as it is, plus the handful of additions
listed under "IR additions" that pay for themselves in the corpus. A `.ksy`
is never executed, interpreted or kept around at run time: once converted, a
Kaitai format is indistinguishable from a hand-written one, and every view
works on it unchanged. Nothing from the Kaitai compiler (GPL-3.0) is ported;
it is read for semantics only. The formats are per-file licensed (141 CC0,
23 MIT, 7 Unlicense, 4 Apache-2.0, 4 BSD; 2 GPL-3.0, 1 LGPL-2.1, 1
GPL-2.0, 1 GFDL-1.3) and only the permissive ones are bundled, each with its
licence line in `THIRD-PARTY-NOTICES.md`.

### A construct the IR cannot express is named, never approximated
The report is as much the deliverable as the template. For every field the
report says what it became; for anything inexpressible it gives the ksy path,
the source text and the reason, and the field is left as bytes (or the
instance dropped). Silent approximation is the defect: a `repeat-until` whose
predicate was quietly turned into "until end" reads a file wrongly and says
nothing. Where the mapping is exact but indirect (a string compared as its
bytes read big-endian), the report says that too, as a note rather than a
gap.

### Strict tier only
The converter is the fast, strict parser (see the "fast strict, slow lenient"
rule). A `.ksy` that the compiler would reject is rejected here with the
same path in the message. Unknown keys are errors except those starting
with `-`, which the compiler ignores and we read where useful
(`-webide-representation`, `-orig-id`).

### Only a leading magic can claim a dropped file
DESIGN.md's rule for detection holds: a bundled Kaitai format is offered by
sniffing only when its first `seq` field is a `contents` at offset 0 (117 of
185 formats have `contents` somewhere; the count at offset 0 is what the
bundling agent must measure). Everything else is opened by choosing it.

### The diagram is laid out in layers, not by force
fCoSe (the Graph view) is force-directed and wrong for a structure diagram.
The diagram uses `@dagrejs/dagre` (MIT, ~100 KB minified, no ports) for node
placement, `rankdir=LR`, and draws its own boxes and edges: boxes are HTML
tables reusing the listing's `.rp-*` classes and field-kind colours, edges are
an SVG overlay in the `hexlinks.ts` style with the role word on them. Edge
endpoints are the y of the field row, so "edges line up" comes from the row
grid, not from port support. If orthogonal routing proves inadequate, `elkjs`
(EPL-2.0, ~1.4 MB bundled, has ports) is the upgrade; not the start.
Reusable from `graphview.ts`: `readPalette`, `family()`, the lazy `import()`
chunk pattern, and the seven-place view checklist in `main.ts`. Not gated
behind a `?` flag.

## Where things go

```
crates/core/src/ksy/
  mod.rs        pub fn convert(text: &str, imports: &dyn Imports) -> Result<Converted, KsyError>
                Converted { template: Template, report: Report }
  yaml.rs       saphyr document -> a tree that keeps the ksy path of every node
  spec.rs       ClassSpec / AttrSpec / InstanceSpec / EnumSpec / ParamSpec model,
                one fromYaml per Kaitai `format/*.scala`, legal-key checks included
  expr/         lexer.rs, parser.rs (Pratt), ast.rs  -- the ksy expression language
  lower.rs      spec -> Template, Report
  report.rs     Report { fields: Vec<Became>, gaps: Vec<Gap>, notes: Vec<Note> }
  imports.rs    Imports trait: bundled resolver + user-supplied map
  bundled.rs    include_str! table of the shipped .ksy files, generated
crates/core/formats-ksy/<category>/<id>.ksy   verbatim copies, per-file licence in NOTICES
crates/core/src/template_text.rs   render(&Template) -> String, the IR as text (read-only)
crates/core/src/eval/diagram.rs    Diagram DTO walk over a Template
crates/wasm/src/lib.rs             set_ksy_template(text, imports_json), ksy_report(),
                                   template_text(), template_diagram()
web/src/ksypanel.ts                live converter panel
web/src/diagramview.ts             the view; strings in strings.ts under KSY and DIAGRAM
```

Dependencies added to `crates/core`: `saphyr` (YAML 1.2, MIT/Apache-2.0, pure
Rust, `encoding_rs` feature off). No serde: the ksy is walked as a generic
tree with path tracking, as the compiler does, so error messages carry
`/types/chunk/seq/2/size`.

## The mapping

Counts are files-using / occurrences over the 185 formats.

| ksy | IR | Notes |
|---|---|---|
| `meta/endian` | default for every `u2..u8`, `f4/f8` without suffix | `switch-on` endian (5 files) is a gap: nothing in the IR picks endianness by expression |
| `meta/bit-endian` | `Endian` on `bN` fields | |
| `meta/imports` (32) | `Part` with the import's id as prefix | resolver: bundled `common/` first, then user-supplied |
| `meta/encoding` | default `Encoding` for `str` | |
| `seq` | `StructDef.fields` in order | |
| `types` (174) | `Template::types`, nested names joined `outer.inner` | `Named` for every user-type reference |
| `id` | field name | `-orig-id` (52) kept in the report only |
| `doc`, `doc-ref` (162 files, 1277 fields) | new `doc` slot, see IR additions | |
| `type: u1..u8, s1..s8, ±le/be` | `UInt`/`Int {bits, endian}` | |
| `type: f4/f8` | `F32`/`F64` | |
| `type: bN[le/be]` (53) | `UInt {bits: N, endian}` | `b1` shown as boolean via `Enum {0: false, 1: true}`? No: leave as 1-bit uint; report nothing |
| no `type`, `size: e` (169/970) | `Bytes(e)` | |
| no `type`, `size-eos` (40) | `Bytes(Remaining)` | |
| `type: str, size` | `Str {Fixed}` inside `Sized(e)` | `pad-right` (11) -> `StrLen::Padded` |
| `type: strz` | `Str {Terminated {end: 0}}` | `terminator` (14): single byte -> `Terminated`; multi-byte -> gap. `include: true` (3), `consume: false` (0), `eos-error: false` (2) -> gap |
| `contents` (117) | `Magic(bytes)` | string, byte list or mixed list all lowered to bytes |
| `type: user_type` | `Named` | with `size`: `Sized(e, Named)` |
| `type: user_type(args)` (26/56, 49 with refs) | monomorphised: a struct copy whose first fields are zero-width `Computed(arg)` named after the params, marked machinery | `Ref` climbs into enclosing structs, so an arg naming the caller's sibling resolves. The converter must refuse when a param name shadows a field visible from inside |
| `type: {switch-on, cases}` (88/399) | `Switch` on an int, `Match` on a `str` field | case keys: enum labels resolve to their ints, byte strings to big-endian ints (<= 16 bytes), `_` -> default. Mixed `true`/`false` keys are ints |
| `enum` (108) | `Enum {def}` | |
| `if` (103/454) | new `Ty::When`, see IR additions | |
| `repeat: expr` (98/362) | `Array {count}` | |
| `repeat: eos` (82) | `Repeat {Until::End}` | |
| `repeat: until` (31/38) | `_.f == lit` (10) -> `Until::FieldValue`; `_io.eof` (4) -> `Until::End`; the other 23 -> new `Until::Cond`, see IR additions | |
| `instances: {value}` (113/691) | zero-width `Computed(e)` field appended after `seq` | integer-valued only; a float or string result is a gap (6 files use float arithmetic) |
| `instances: {pos}` (84/266) | `At {anchor, at, inner}` appended after `seq` | `io` absent or `_io` -> `Anchor::Window`; `_root._io` (72 of 106 `io:`) -> `Anchor::File`; `_parent._io` (4) and `field._io` (30) -> gap unless the field is the nearest `Sized`, which is the common case worth checking |
| `process: zlib` | `Decoded {Packing::Fixed(Codec::Zlib)}` | `xor`, `rol`, `ror`, custom (5 files total) -> gap |
| `valid` (26 files) | `eq` on bytes -> `Magic`; the rest -> report note, no check | `Check` is checksum-shaped; a value constraint type is not worth adding for 26 files yet |
| `params` | see `type: user_type(args)` | |
| `-webide-representation` on a type (121 files) | `StructDef::line` parts; a lone `{field}` -> `named_by` | `:dec`/`:hex` suffixes map to the word/quiet flags where they fit, else dropped with a note |
| `to-string` (1) | ignored, note | |
| `enums` | `EnumDef` | labels used in expressions become their ints |

### Expressions

Integer-valued `Expr` only. The corpus inside expressions: comparison 128
files / 1160, arithmetic 148 / 962, `_root.` 61 / 318, `_parent.` 33 / 161,
bit ops 50 / 297, `and`/`or`/`not` 52 / 237, ternary 64 / 234, enum labels
79 / 1060, string literals 27 / 219 (39 of them in `==` comparisons), modulo
22 / 44, shifts 36 / 113, subscript `[]` 35 / 165, `.to_i` 11 / 76, `_index`
19 / 48, `.size` 15 / 36, `_io.pos` 20 / 34, `.as<>` 8 / 27, `_io.size` 21 /
27, `_io.eof` 10 / 15, `.length` 6 / 11, `.first/.last` 5 / 8, `.to_s` 3 / 8,
`sizeof<>` 3 / 5, `.substring` 1, f-string 1, float literals in 6 files.

| ksy | IR |
|---|---|
| `name` | `Ref(name)`: earlier sibling, then enclosing structs. The converter checks the name resolves at lowering time and that nothing between shadows it |
| `_parent.x`, `_parent._parent.x` | `Ref(x)` after checking that the climb lands where Kaitai's would |
| `_root.x` | `Ref(x)` when `x` is a root field; `_root.a.b` -> `Within` |
| `a.b` (field then path) | `Within(path)` |
| `arr[i]` | `Elem {array, index, field}` |
| `_index` | `Idx` |
| `_` in `repeat-until` | the current element; see `Until::Cond` |
| `+ - * / << >> & \| min max` | the existing variants; `%` -> new `Mod` |
| `== != < <= > >=` | `Less` and the new comparisons |
| `and or not` | new boolean variants |
| `c ? a : b` | new `Cond` |
| `x.to_i` on an int or bool | `x` |
| `x.to_i` on a str | gap unless every use is `to_i`, in which case the field lowers to `TextInt` |
| `.size`/`.length` on bytes/str | `SizeOf(name)` |
| `.size` on an array | new `LenOf(name)` |
| `.as<t>` | dropped (a cast) |
| `sizeof<t>` | the static size when the type has one, else gap |
| `_io.size` | new `WindowSize`; `_io.pos` -> new `Pos`; `_io.eof` -> `Remaining == 0` |
| `"abcd"` compared with a fixed-size str/bytes field | the bytes as a big-endian literal, with a report note |
| float, string concatenation, `.to_s`, `.substring`, `.reverse`, f-strings | gap |
| enum labels `e::l` | the int |
| byte array literal `[0x00, 0x01]` in a comparison | big-endian int (<= 16 bytes) |

### IR additions

Each with the count that justifies it. Land these first, on main, with
`eval`, `relate.rs` rendering and tests, before the converter is merged.

* `doc: Option<Arc<str>>` on `Field`, `StructDef` and each `EnumDef` value.
  1277 fields in 162 files carry prose; the inspector gets a place to show
  it. `doc-ref` appended as a URL line.
* `Expr::Mod`. 44 uses; the `q - (q / x) * x` idiom exists but renders
  unreadably and the relations panel would show the idiom, not the intent.
* `Expr::Eq`, `Ne`, `Le`, `Ge`, `Gt` beside `Less`; `Expr::And`, `Or`,
  `Not` as booleans (0/1). Note `Expr::Or` today is value-or ("right side
  when the left is zero"); the boolean must be a new name, e.g. `Either`, or
  the existing one renamed first. 1160 comparisons and 237 boolean ops make
  emulation through `Less`/`Min`/`Max` a readability disaster in `relate.rs`.
* `Expr::Cond { when, then, otherwise }`. 234 ternaries; the `Less`+`Or`
  emulation fails when `then` is zero.
* `Ty::When { cond: Expr, inner }`: a field that is absent (zero size, no
  node) when the condition is false. 454 uses in 103 files. `Switch` with an
  empty default is the current idiom and shows a switch where the format
  says "optional".
* `Until::Cond(Expr)`: stop after the element for which the expression is
  true, evaluated inside that element (`_` is the element, `_.f` is `Ref(f)`
  there, `_index` is `Idx`, `_io.eof` is `Remaining == 0` of the list's
  container). 23 uses no existing `Until` can say.
* `Expr::Pos` (bits from the start of the nearest window to here),
  `Expr::WindowSize` (size of the nearest window), `Expr::LenOf(name)`
  (element count of an earlier list). 34 + 27 + 36 uses.

Not added, deliberately: endianness by expression (5 files), value
constraints (26 files, `valid`), floats and strings in expressions, `io:`
naming an arbitrary field's stream, parametrised types as a first-class
construct (monomorphising covers the corpus).

## Test oracle

`D:\github\kaitai_struct\tests\formats\*.ksy` (339), `tests\src\*.bin` (60,
present) and `tests\spec\ks\*.kst` (320). A `.kst` names the ksy id, the
input file and a list of `{actual: <ks expr>, expected: <ks expr>}` asserts.
`crates/core/tests/ksy_oracle.rs` (skipped when `KAITAI_STRUCT` is unset)
converts every ksy, evaluates on the named bin, and checks each assert whose
`actual` is a plain field path and whose `expected` is a literal. Everything
else is a skip, counted. The report the test prints is the pass rate as it
is: converted / gap / error, and per assert pass / fail / skip. Do not curate
the list to make it green; the numbers are the point.

Also `tests\formats_err\*.ksy` (156): each must fail to convert. A ksy that
converts from that directory is a bug.

## Sniffing and bundling

The bundling agent produces the list from `HANDOVER-kaitai-gaplist.md`
(formats absent from Qubero, permissive licence, worth having), copies each
verbatim into `crates/core/formats-ksy/`, adds them to `bundled.rs`, the
sniff table (leading `contents` only) and `THIRD-PARTY-NOTICES.md`, and
reports each one's conversion result. A bundled format whose conversion has
gaps ships with the gaps visible in the panel, or does not ship: the
coordinator decides per format from the report.

## The panel

Two columns: ksy text on the left (paste, drop, or pick a bundled one), the
IR as text on the right, with the report between them: a list of gaps (path,
source, reason) that click to the line. "Apply to file" sets the template on
the open document. The IR text comes from `template_text.rs`, which is also
what the diagram uses for size and position labels; it is read-only for now
(a parser for it is DESIGN.md's "text format for templates" and a separate
step).

## The diagram

Core `eval/diagram.rs` walks a `Template` (not a file) and emits:

```
Diagram { types: Vec<TypeBox>, edges: Vec<DiagramEdge>, omitted: u32 }
TypeBox { name, rows: Vec<Row>, kind: Seq | Instances | Switch, parent: Option<name> }
Row { name, type_text, size_text, pos_text, kind: field kind, port: row index }
DiagramEdge { from: (type, row), to: (type, row | whole box), role: Role, label }
```

Roles come from `origin.rs::Role` (Length, Width, Count, Type, Position,
Value, Name, Points) plus `Case` for switch fan-out. Expression text is what
`relate.rs` writes for `written`. The Kaitai emitter's shape is the
reference: one table per type with pos / size / type / id columns, a side
table per switch listing case -> type, bold edges field -> type, grey edges
from the field an expression reads to the cell that reads it.

Web `diagramview.ts`: dagre for placement, LR. A box past `ROW_CAP` rows
collapses to a summary row that links to its own section further right
("defined elsewhere"); a `Named` self-reference is a back edge with its
label, never an expansion. Clicking a row highlights the same field in
Listing and Hex where the open file has an instance of it (a core query for
"first path whose type is X" is needed; second pass). Hovering an edge shows
the written expression.

## Phases and agents

Opus for implementation, Fable for design and strings. Every worktree agent
starts with `git merge main`, sets `QUBERO_SAMPLES` and
`KAITAI_STRUCT=D:/github/kaitai_struct`, and is merged only on cargo's own
exit code from a debug `cargo test`.

Wave 1 (parallel, separate worktrees):
* A. IR additions (above) with eval, relate, encode where relevant, tests.
  Merge first; B and C rebase on it.
* B. `template_text.rs` and the wasm `template_text` entry. Snapshot tests
  over every builtin.
* C. `eval/diagram.rs`, wasm `template_diagram`, `diagramview.ts`, the view
  button and strings.

Wave 2:
* D. The converter (`ksy/`), the oracle test, `set_ksy_template`. Depends on A.
* E. Bundling and sniffing. Depends on D and the gap list.
* F. The panel. Depends on B and D.

Strings for the panel and the diagram are written by the coordinator with
`ui-text` and `info-design` loaded and listed at the end of the turn.

## Status, 2026-09-14

Landed on main, in this order: the ksy reader, expression parser and spec
model; the IR as text (`template_text.rs`, ten snapshots); the Diagram view;
the IR additions; the lowering (`ksy/lower.rs`, 2,440 lines, wants splitting
into `lower/{resolve,fields,exprs,instances,repr}.rs`); the converter panel
(`web/src/ksypanel.ts`); one expression writer shared by the IR text, the
relations panel and the diagram; and the bundled formats: 112 files, 105 offered in the chooser, 20 of
them sniffable, 6 excluded by licence and 6 by conversion gaps on their
main path (see `crates/core/formats-ksy/README.md` for each decision;
`node tools/ksy_bundle.mjs` regenerates it).

Numbers, uncurated, from `cargo test -p qubero-core --test ksy_oracle -- --nocapture`
with `KAITAI_STRUCT` set:

| corpus | clean | with gaps | refused |
|---|---|---|---|
| tests/formats (339) | 200 | 139 | 0 |
| formats (185) | 111 | 74 | 0 |

`.kst` asserts: 709 pass, 589 fail, 120 skipped. The failures follow the gap
reasons below, not converter bugs found so far; each is a field the template
cannot compute or place.

Top gap reasons over formats/, by occurrence: byte order chosen while the
file is read (51); `io:` naming another field's stream (40); not placed
because an earlier field's size is unknown (35); value is text (34); bitwise
or (26); value is a float (25); `.to_i` on text (19); instance with no `pos`
(10). The next IR additions, if the corpus is the guide: `Expr::BitOr`,
`BitXor`, `BitNot` (31 uses, trivial); an anchor naming an earlier field's
window for `io:` (40); endianness by expression (51, five files, harder).

Decisions taken during the build that the plan did not settle:
`Expr::Or` is written `or else` everywhere (the boolean is `Either`, written
`or`); `Pos` and `WindowSize` are bytes; `|` and `^` are gaps rather than
lowered to the value-or; Kaitai's byte-alignment before a non-bit read is
made explicit as machinery padding fields; `io: _parent._io` is a gap
(Window is this field's window, and nothing proves they coincide);
`Role::Condition` labels a `When` guard in the diagram and the relations
panel; the diagram caps at 300 boxes (`BOX_CAP`), which HDF5 hits and WAV
sits one under; `&` binds tighter than a comparison in every written
expression, the Kaitai and Python reading.

Open: the diagram can pick only the root type's rows (needs a core "first
path whose type is X" query); an instance overlay on the diagram (counts per
type, the cursor's type lit); `str` + `.to_i` lowering to `TextInt` by
whole-program use analysis; a parser for the IR text.

### Diagram, later on 2026-09-14

Boxes are drawn once per structure (keyed on the type's name plus its
`template_text` rendering, so ELF's four class/endian variants stay apart and
ID3's thirteen TextFrame cases collapse to one; WAV went from 299 boxes to
69). Arrows are routed by ELK (`elkjs`, EPL-2.0, in a web worker; layered,
orthogonal, one port per row) with hop-over arcs at every crossing and a
nudge pass for collinear runs between layers; dagre is gone. A census of the
open file (`eval/census.rs`, `Editor::diagram_census`) puts a `×n` badge on
every box and row, mutes what the file lacks, drives the "Only what this file
has" toggle, and gives every row a first path for double-click. A second
mode, Strips (`web/src/strips.ts` plans it, `diagramview.ts` draws it), shows
each type as its fields in file order the way a format spec's syntax figure
does: repeats as first box, dots, last box; optional fields with a dashed
outline and their condition underneath; composites expanded in the strip
below by dashed funnels, each type once. `diagramview.ts` is ~1,300 lines
holding both renderers and wants splitting into `diagramarrows.ts` and
`diagramstrips.ts`. No sample in the collection exercises `Ty::When`, so the
dashed optional outline is proven by unit test and a forced screenshot only.

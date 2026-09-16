# ImHex patterns in Qubero

Planned 2026-09-17. Read `HANDOVER-kaitai.md` first: this is the same shape
of work for ImHex's pattern language (`.hexpat`), and every fixed decision
there holds here unless this file says otherwise. Checkouts, both shallow
clones made on 2026-09-17:

* `~/github/ImHex-Patterns` (GPL-2.0 as a repository, 310 patterns, 54k
  lines, 46 include files under `includes/std`, `includes/type`,
  `includes/hex`; 202 pattern-plus-sample pairs under
  `tests/patterns/test_data`, 39 MB).
* `~/github/PatternLanguage` (LGPL-2.1, the reference implementation;
  91 language tests under `tests/include/test_patterns/*.hpp`). Read for
  semantics only. Port nothing from either checkout's C++.

Counts below come from `hexpat_stats.py` (kept beside this file in
`tools/` once the work starts; the first run was from the session
scratchpad). Rerun it rather than trust the numbers if the corpus moves.

## Decisions (made with the user, 2026-09-17)

**Import on demand; bundle only a vetted subset.** The corpus has one
repository licence, GPL-2.0, and no per-file licences apart from a handful
of headers. Patterns are not compiled in the way the `.ksy` set is. The app
takes a `.hexpat` by drop, paste, or URL, and a library browser lists the
upstream patterns and fetches the one picked from
`raw.githubusercontent.com/WerWolv/ImHex-Patterns/master/patterns/<path>`
at the moment of use. The vetted subset that may be bundled, each with its
licence line in `THIRD-PARTY-NOTICES.md`: `gltf.hexpat` (MIT text in the
header) and the four MPL-2.0 files (`bink_container.hexpat`, `vhd.hexpat`,
`fs/mbr.hexpat`, and any other carrying the MPL header), kept verbatim as
separate files. `pbz.hexpat` and `BroEngine/*` (GPL-2, EUPL-1.2) are not
bundled. The include files (`std/*.pat`, `type/*.pat`) are GPL-2.0 too and
are not bundled either: the converter knows what the commonly used `type::`
names mean (below) and treats `std::` calls by name.

**Converter to the IR, declarative subset.** A `.hexpat` becomes a
`Template` plus a report, exactly as a `.ksy` does. Nothing is executed at
run time. The imperative half of the language (`fn`, statement `while`,
`for`, local variables, `in`/`out`, `try`, `std::` calls that compute) is
reported as gaps, field by field; display-only calls (`std::format`,
`std::print`, `[[format]]`, `[[comment]]`) are notes. 153 of the 310
patterns use none of the imperative half. Never approximate: a `[[format]]`
whose function would have printed a decoded time is a note saying the raw
value is shown, not a guess at the function's output.

**Strict tier only.** A pattern the reference implementation would reject
is rejected with the same line and column. Unknown attributes are errors;
unknown `#pragma`s are errors except the documented ones listed below.

**Detection by `#pragma magic` only.** `#pragma magic [ 4D 5A ] @ 0x00`
(91 files) is the one thing that may claim a dropped file; `type::Magic`
fields and `std::assert` checks are read but do not claim. Same rule as
Kaitai's contents-at-offset-0.

## Where things go

```
crates/core/src/hexpat/
  mod.rs        pub fn convert(text: &str, includes: &dyn Includes) -> Result<Converted, HexpatError>
                Converted { template: Template, report: Report }   (reuse ksy::Report as is; move it to a shared module if the ksy one is not already generic)
  lexer.rs      tokens with line/column; string, char, numeric literals (0x, 0b, 0o, ', digits with ' separators, float literals kept as text)
  ast.rs        the declarative AST: pragmas, imports, namespaces, using, struct/union/bitfield/enum, fields, arrays, placements, attributes, if/else, match, templates; plus an opaque Statement node for anything imperative so the parser never fails on it
  parser.rs     recursive descent + Pratt expressions; one error type with position
  expr.rs       the expression language (shared writer with ksy where the operators coincide; see the ksy follow-up on one expression writer)
  lower.rs      AST -> Template, Report
  types.rs      the `type::` and `std::` names the converter understands, by name, with what each becomes
  includes.rs   Includes trait: user-supplied map of `std/mem.pat`-style paths to text (never bundled), plus the built-in table above
crates/core/formats-hexpat/<id>.hexpat   the vetted subset only, verbatim, licence line per file
crates/wasm/src/lib.rs                   set_hexpat_template(text, includes_json), hexpat_report(), preview_hexpat_template()
web/src/hexpatpanel.ts                   converter panel; strings in strings.ts under HEXPAT
web/src/hexpatlibrary.ts                 the upstream list (patterns/**/*.hexpat, refreshed by tools/hexpat_index.mjs into web/public/hexpat-index.json) and the fetch
tools/hexpat_stats.py                    the census
tools/hexpat_bundle.mjs                  regenerates the bundled table and README for the vetted subset
```

Reuse from the ksy work: `Report`/`Became`/`Gap`/`Note`, `template_text::render`,
the `ExtraTemplate` kind mechanism in `main.ts`, the panel layout of
`ksypanel.ts` (copy its structure, share its CSS), the settings-dialog entry
that opens the panel, and the drop handling for a text file with the
matching extension.

## The mapping

Counts are files-using / occurrences over 310 patterns.

| hexpat | IR | Notes |
|---|---|---|
| `#pragma endian little/big` (108) | default `Endian` for every unsuffixed multi-byte type | absent means little, as the reference does |
| `#pragma magic [ .. ] @ N` (91) | sniff signature | the only claim on a dropped file |
| `#pragma description/author/version/once/array_limit/pattern_limit/loop_limit/eval_depth/source(s)/repo/history/filename/debug` | description and author kept for the panel and the report; the rest read and ignored with a note | |
| `#include`/`import` (215) | resolved through `Includes`; `std.*`/`type.*`/`hex.*` handled by name (below); anything else missing -> error naming the path | |
| `namespace a { .. }` (19) | names joined `a::b` in `Template::types` | |
| `using X = T;` (32) | `Named` alias, or the aliased base type inlined | `using X = type::Magic<"..">` is common |
| `struct S { .. }` | `StructDef` in `types`, `Named` at every use | |
| `struct S : Base` (4) | Base's fields copied first | converter-side |
| `struct S<auto N, T>` (20/51) | monomorphised per instantiation, as the ksy `params` lowering does | value params become zero-width `Computed` machinery fields; type params substitute |
| `union U { .. }` (5/7) | new `StructDef::overlap` (below) | |
| `bitfield B { a : 3; padding : 2; signed s : 4; bool f : 1; E e : 2; }` (68/184) | `SizedBits` over `UInt/Int {bits}` and `Enum` of those; `padding : n` -> `UInt {bits: n}` named `padding`, machinery | bit order: default is LSB-first within each byte read in the pragma endian; `[[left_to_right]]`/`[[right_to_left]]`/`bitfield_order` (12 files) -> the `Endian` on each bit field. Nested bitfields and bitfield arrays as they come; `[[bitfield_order]]` with an explicit size is a gap unless it matches |
| `enum E : u16 { A, B = 5, C = 0x10 ... 0x1F }` (118/319; ranges 9/24) | `EnumDef` with `cases`; ranges -> `EnumSpan {from, step: 1}` | implicit values continue from the previous one, as C does |
| `u8..u128`, `s8..s128` | `UInt`/`Int {bits, endian}` | `u128` must be checked against the evaluator's `i128` arithmetic; if 128-bit unsigned overflows, gap |
| `float`, `double`, `float16` (31/176) | `F32`, `F64`, `F16` | |
| `bool` (34/167) | `Enum {0: false, 1: true}` over `UInt {8}` | |
| `char x[N]` (121/523) | `Str {Padded {size: N, pad: 0}, Ascii}` | the reference stops at the first NUL when displaying |
| `char x[]` (22/65) | `Str {Terminated {end: 0}}` | |
| `char16 x[N]` (16/47) | `Str` with UTF-16 in the pragma endian | |
| `str` field (15/38) | gap | only meaningful with functions |
| `le`/`be` prefix (20/72) | overrides the field's `Endian` | |
| `T x[N]` | `Array {count}` | `N` any integer expression |
| `T x[]` (23/75) | `Repeat {Until::End}` | until the enclosing window or file ends |
| `T x[while(c)]` (55/106) | new `Until::While(c)` (below) | `while(!std::mem::eof())` (17) -> `Until::End`; `while($ < e)` (18) -> `Until::While(SpacePos < e)` |
| `padding[N]` (57/245) | `Bytes(N)` named `padding`, machinery | |
| `T x @ addr;` at top level (139/381) | root struct field `At {anchor: File, at: addr, inner}` | `@ $` (11) and `@ addressof(f)` (15) as expressions |
| `T x @ addr;` inside a struct | `At {anchor: File, ..}` | hexpat addresses are absolute unless `[[pointer_base]]` says otherwise |
| `T *p : u32;` (7/19) | inline struct `{offset: u32, target: At {File, offset, T}}` | the exact shape the pointer-display work of 2026-09-17 renders; `[[pointer_base("fn")]]` (2) is a gap |
| `if (c) { .. } else { .. }` (110/519; else 70/341) | one `When {cond, inner}` per block, `inner` an inline struct (`StructDef::inline`) of the block's fields; the else block `When {Not(cond), ..}`; `else if` chains likewise | one `When` per block, not per field: the condition then appears once in the relations panel and the diagram |
| `u32 x = <expr>;` local inside a struct (56/225) | zero-width `Computed(expr)` machinery field, the same lowering as a ksy `instances: value` | only when the right side is a pure expression over fields in scope. Reassignment later, `$` arithmetic with side effects, or a call to a `fn` on the right side is a gap |
| `match (a, b) { (1, _): T x; (2 ... 5, _): ..; (_, _): .. }` (23/66) | `Switch` when every arm is a literal or `_` on one scrutinee; ranges -> repeated cases when short, else gap; two scrutinees -> gap | |
| `[[name("x")]]` (21/130) | `Field::name` becomes the display name, the source name kept in the report | |
| `[[comment("..")]]` (19/137) | `Field::doc` | |
| `[[inline]]` (29/79) | `StructDef::inline` on a copy of the type | |
| `[[hidden]]` (24/55) | note: the field is read and shown | display-only; a hidden flag on `Field` is not worth adding for 24 files |
| `[[format("fn")]]`, `[[format_read]]`, `[[transform]]` (68 / 6 / 3) | note: raw value shown | the functions are not run |
| `[[sealed]]` (36/57) | note | the reference shows the struct as one value; our listing already joins short structs on one line |
| `[[color]]`, `[[single_color]]`, `[[static]]`, `[[export]]`, `[[fixed_size]]`, `[[hex::visualize]]`, `[[highlight_hidden]]`, `[[no_unique_address]]` | ignored with a note | `no_unique_address` (6/9) is a union of one field; lower it as `overlap` when it appears beside a sibling at the same offset |
| `[[hex::spec_name("..")]]` (1/17) | `Field::doc` line | |
| `type::Magic<"ABCD">` (26/32) | `Magic(bytes)` | |
| `type::GUID` (9/21) | the struct the include declares (`u32 u16 u16 u8[8]`), named `GUID`, with a `line` that prints it in the usual form | |
| `type::time32_t`, `time64_t`, `FILETIME`, `DOSTime`, `DOSDate` (9/17) | `UInt` with `Field::time` set to the matching `Time` variant | check `Time` has each; add the missing ones |
| `type::Size`, `Size16/32/64`, `Size8` (14/75) | `UInt`; note that the reference formats it as a size | |
| `type::uLEB128`, `LEB128`, `sLEB128` (4/19) | `Leb128`, `Zigzag` or signed LEB as the IR has them | |
| `type::Hex`, `RGBA8`, `Nibbles`, `escape_bytes` (5/10) | `UInt` (hex flag), the RGBA struct, two 4-bit fields, `Bytes` | |
| `std::mem::eof()` | `Remaining == 0` | |
| `std::mem::size()` (23/45) | new `Expr::SpaceSize` (below) | |
| `std::mem::read_unsigned/read_signed(addr, n[, endian])` (30/49) | `PeekAt` for `$` and `$ + k`, new `Expr::PeekIn` for everything else (below) | `read_string` -> gap |
| `std::mem::create_section` and friends (7) | gap | |
| `std::core::member_count`, `array_index` (18/39) | `LenOf`, `Idx` where the argument is a field in scope | |
| `std::assert`, `assert_warn`, `warning`, `error` (41/115) | note, with the condition rendered | a value constraint type is still not worth adding (same call as the ksy `valid`) |
| `std::format`, `std::print` (77/254) | note | |
| every other `std::` call, `fn`, statement `while`/`for`, reassigned locals, `in`/`out`, `try`, `$ = ..` (8/22) | gap on the field that uses it; the field is left as `Bytes` of its static size when it has one, else the struct ends there with a gap | |

### Expressions

Integer-valued `Expr` only, as for ksy.

| hexpat | IR |
|---|---|
| `name` | `Ref`: earlier sibling, then enclosing structs; checked at lowering |
| `parent.x`, `parent.parent.x` (38/157, 7/12) | `Ref(x)` after checking the climb lands where the reference's would |
| `this.x` (3/5) | `Ref(x)` in the current struct |
| `a.b` | `Within` |
| `arr[i]` (23/137) | `Elem` |
| `$` read (60/196) | new `Expr::SpacePos` (below) |
| `sizeof(f)` (36/82) | `SizeOf(f)`; `sizeof(T)` the static size or gap |
| `addressof(f)` (29/78) | `StartOf(f)` as an absolute address |
| `E::Label` (51/660) | the int |
| `+ - * / % << >> &` | existing |
| `\|` bitwise (19/197), `^` (2/16), `~` (3/4) | new `BitOr`, `BitXor`, `BitNot` (below) |
| `== != < <= > >= && \|\| !` | existing `Eq..Ge`, `Both`, `Either`, `Not` |
| `c ? a : b` (16/18) | `Cond` |
| `"abcd" == f` (26/74) | the bytes as a big-endian literal when `f` is a fixed `Str`/`Bytes` of <= 16 bytes, note; else gap |
| float literal, float arithmetic (20/28) | gap |
| string functions, `std::string::*` | gap |

### IR additions

Each with the count that justifies it. Land these first, on main, with
`eval`, `relate.rs`/`template_text.rs` rendering and tests, before the
converter is merged.

**All four landed 2026-09-17**, with `Expr::PeekIn` beside them for the
absolute read (see the paragraph after this list), evaluation, rendering,
unit tests over a `MemSource`, and a `tests/snapshots/template_text/notation.txt`
snapshot of what the notation reads like.

* `Expr::BitOr`, `BitXor`, `BitNot`. 19 + 2 + 3 files here, and on the
  Kaitai follow-up list already. `Expr::Or` stays the value-or; the new
  names say what they are. `BitNot` is over the whole 128-bit number, so a
  pattern meaning `~x` within a word lowers with the mask it wrote.
* `Until::While(Expr)`: checked *before* each element, in the list's own
  scope (`Ref` names the list's siblings, `Idx` the element about to be
  read, `Pos`/`SpacePos` where it would start). 55 files. `Until::Cond` is
  the after-the-element check and stays. Written `repeat(while ...)` where
  `Until::Cond` is `repeat(until ...)`.
* `Expr::SpacePos` and `Expr::SpaceSize`: position and size measured from
  the start of the whole space (the file at the top level, the unpacked
  bytes inside a compressed run), regardless of any `Sized` window
  between. `Pos`/`WindowSize` stay window-relative; hexpat's `$` and
  `std::mem::size()` are space-relative (60 + 23 files), and the Kaitai
  `_root._io` cases want the same. Also needed so `addressof`
  round-trips with `@`.
* `StructDef::overlap: bool`: the fields start at the same offset and the
  struct is as long as the longest. 5 files with `union` and 6 with
  `[[no_unique_address]]`. Read: each field at offset 0 of the struct;
  size: the maximum. Only the *first* field is counted towards any total,
  the others being second readings of the same bytes the way `Field::aside`
  marks one by hand, so a union of four readings of sixteen bytes is sixteen
  bytes of file and not sixty-four. The cursor lands on the first field, as
  it does for any overlapping stretch. Constructor `Ty::union_structure`;
  written `(overlap)` in the IR text; the inspector's Position row says
  "same start as every field of colour".
* No `Time` additions: `Time::unix`, `Time::filetime`, `Time::dos` and
  `Time::dos_halves` (template.rs, after line 1724) already cover
  `time32_t`, `time64_t`, `FILETIME`, `DOSTime` and `DOSDate`.

Not added, and why: a hidden flag (24 files; display-only, so a note);
float expressions (20 files, all inside display functions); sections; `$`
assignment (8 files, imperative by nature).

**Decided 2026-09-17: an anchored peek, `Expr::PeekIn {at, bits, endian}`,
landed with the four additions above.** `std::mem::read_unsigned(addr, n)`
reads at an absolute address and `PeekAt {skip, ..}` is relative to the
current position, so `skip = addr - SpacePos` was the other candidate. It is
unsound: a negative skip on `PeekAt` already means something else, counting
back from the end of the container, so every address behind the field asking
would be read from the wrong end of the file, silently. `at` is bits from the
start of the space, the same stretch `SpacePos` counts from and
`Anchor::Space` anchors to, and it is held to the space rather than to the
container, since a record naming an address is usually inside a window that
does not hold it.

The corpus says how much each form is worth. Of about 130 `read_unsigned` and
`read_signed` calls, 77 pass a bare `$` and another two dozen pass `$ + k` for
a constant `k`: those lower to `PeekAt {skip: k * 8}` and want nothing new.
The rest are what `PeekIn` is for: eight backward reads (`$ - 4` three times,
`$ - 1`, `$ - 32`, `$ - wordsize()`, `current_address - 20`,
`std::mem::size() - 256*3 - 1`) and about fifteen absolute ones, literal
(`0x3c`, `0`, `4096`, `0x8`) or read from a field (`EOCD64.CDOffset`,
`offset`, `start_offset`). A machinery `At {anchor: Space, ..}` field beside
the reader, referenced by name, would cover the absolute ones without new IR,
and it changes the field list the reader sees for what is only a read; it also
cannot help a `[while(...)]` condition, which has no field to hang machinery
on, and `Set set[while (std::mem::read_unsigned($ - 1, 1) != Type::EndSet)]`
in the corpus is exactly that. So: lower `$` and `$ + k` as `PeekAt`, and
everything else as `PeekIn`.

## To confirm against the reference tests before lowering

Three facts in the table read plausibly and may be wrong. Check each
against `~/github/PatternLanguage/tests/include/test_patterns/` and one
corpus file with a known layout, then write "confirmed against
test_pattern_X" into the row: the default bit order inside a `bitfield`
(`test_pattern_bitfields.hpp`); whether `char x[N]` shows all N bytes or
stops at the first NUL (`test_pattern_strings.hpp`, `test_pattern_arrays.hpp`);
and which names a `[while(c)]` condition can see and what `$` is inside it
(`test_pattern_arrays.hpp`, `test_pattern_dollar.hpp`).

## Panel and library

The panel is the ksy panel with a second file type: drop or paste a
`.hexpat`, see the IR as text and the report, apply it to the open file.
Includes the pattern needs and does not have are listed as missing with a
paste box each, as ksy imports are. A "From the ImHex library" button opens
the list from `web/public/hexpat-index.json` (path, description from
`#pragma description`, author, magic if any), filtered by name; picking one
fetches it and its `import`s that are not built-in from the raw GitHub URL,
shows the licence notice (GPL-2.0 for the repository; the file is fetched
into this session and not stored by Qubero), and runs the converter. No
network call happens without a click. The index is regenerated by
`tools/hexpat_index.mjs` from the checkout and committed; it is metadata,
not the patterns.

The template chooser lists an applied pattern as an extra template
`hexpat:<name>`, the same way `ksy:<id>` appears.

## Tests and the oracle

* Language: port the *meaning* of each `tests/include/test_patterns/*.hpp`
  case that is within the declarative subset into a Rust test with our own
  bytes; the reference's expected values are the oracle. Cases outside the
  subset are tests that the converter reports the right gap.
* Syntax: `cargo run --example hexpat_parse` over `IMHEX_PATTERNS` reports
  how many of the 310 patterns and 46 includes parse, imperative bodies as
  opaque statements. The parser is done at 310/310 and 46/46, not before.
* Corpus: `cargo run --example hexpat_gaps` over `~/github/ImHex-Patterns/patterns`
  (gated by `IMHEX_PATTERNS=<path>`) prints, per pattern, clean / gaps
  with counts, and the totals go in this file's status section. Then for
  each of the 202 pairs in `tests/patterns/test_data`, evaluate the
  converted template over the sample and report whether it reads to the
  end without an error; that pass rate is reported as it is.
* Browser: `web/test/hexpat.browser.mjs` in the style of `ksy.browser.mjs`:
  paste a small pattern, apply, see the fields; open the library list, pick
  one (served from a local fixture, not the network), see the notice.

## Status

2026-09-17: planned. Census run once from the scratchpad.

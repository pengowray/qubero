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
| `bitfield B { a : 3; padding : 2; signed s : 4; bool f : 1; E e : 2; }` (68/184) | `SizedBits` over `UInt/Int {bits}` and `Enum` of those; `padding : n` -> `UInt {bits: n}` named `padding`, machinery | bit order: confirmed, fields pack from bit 0 of each byte upwards under the pragma endian's `little` (the default) and from bit 7 downwards under `big`, running across byte boundaries either way. `[[left_to_right]]` and `[[right_to_left]]` no longer exist: the reference throws E0008 on both. `[[bitfield_order(direction, size)]]` takes exactly two arguments, `direction` 0 = most-to-least significant and 1 = least-to-most, `size` > 0, and it reverses the layout when `direction` disagrees with the endian. Nested bitfields and bitfield arrays as they come |
| `enum E : u16 { A, B = 5, C = 0x10 ... 0x1F }` (118/319; ranges 9/24) | `EnumDef` with `cases`; ranges -> `EnumSpan {from, step: 1}` | implicit values continue from the previous one, as C does |
| `u8..u128`, `s8..s128` | `UInt`/`Int {bits, endian}` | `u128` must be checked against the evaluator's `i128` arithmetic; if 128-bit unsigned overflows, gap |
| `float`, `double` (31/176) | `F32`, `F64` | `float16` is not a built-in type: `includes/type/float16.pat` declares `using float16 = u16 [[format("type::impl::format_float16")]]`, so it arrives as a `type::` name (2 files). Lower it to `F16` by name, not by keyword |
| `bool` (34/167) | `Enum {0: false, 1: true}` over `UInt {8}` | |
| `char x[N]` (121/523) | `Str {Padded {size: N, pad: 0}, Ascii}` | confirmed: the field owns all N bytes and its value is all N bytes, embedded NULs included. Only the *display* trims, and it trims trailing NULs, not everything after the first one. `Padded` is right; a `Terminated` would be wrong |
| `char x[]` (22/65) | `Str {Terminated {end: 0}}` | |
| `char16 x[N]` (16/47) | `Str` with UTF-16 in the pragma endian | |
| `str` field (15/38) | gap | only meaningful with functions |
| `le`/`be` prefix (20/72) | overrides the field's `Endian` | |
| `T x[N]` | `Array {count}` | `N` any integer expression |
| `T x[]` (23/75) | `Repeat {Until::End}` | until the enclosing window or file ends |
| `T x[while(c)]` (55/106) | new `Until::While(c)` (below) | confirmed: the condition is checked *before* each element, and `$` in it is the absolute position the element would start at. It sees the list's own siblings and everything further out (`this` is the enclosing struct, `parent` climbs), but never the element about to be read. `while(!std::mem::eof())` (17) -> `Until::End`; `while($ < e)` (18) -> `Until::While(SpacePos < e)` |
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

## Confirmed against the reference tests, 2026-09-17

All three checked, and the rows above rewritten. Two of the three were
wrong in the first draft.

**Bit order inside a `bitfield`: bits pack from the low bit up under
`little`, from the high bit down under `big`, and the default is `little`.**
Confirmed against `TestPatternBitfields` in `test_pattern_bitfields.hpp` and
`Evaluator::readBits` (`lib/source/pl/core/evaluator.cpp:114`). The test reads
`be TestBitfield testBitfield @ 0x25` over bytes `49 44 41 54 78`: `a : 2`
is `0b01` = 1, `b : 3` is `0b001` = 1, the nested `c.nestedA : 4` takes the
last three bits of `0x49` and the top bit of `0x44` and is 2. That is
most-significant-bit first, packing across the byte boundary, which is what
`readBits` does for `big`; for `little` it takes `bitOffset` from the low end
instead. Nothing in the language changes the *order fields are declared in*,
only which end of the byte bit 0 sits at.

Two corrections came with it. `[[left_to_right]]` and `[[right_to_left]]` are
**gone**: `ASTNodeBitfield::createPatterns` throws E0008, "Attribute ... is no
longer supported", on either one, so a pattern using them is rejected, not
reordered. And `[[bitfield_order]]` is not a bare marker: it takes exactly two
arguments, a direction (0 most-to-least significant, 1 least-to-most) and a
fixed size in bits that must be greater than zero, and it reverses the layout
only when the direction disagrees with the bitfield's endian
(`TestPatternReversedBitfields` writes `[[bitfield_order(1, 16)]]` under
`#pragma endian big` and expects the reversed layout).

**`char x[N]` is all N bytes, not up to the first NUL.** The plan's note was
wrong. `PatternString::getValue(size)` (`lib/include/pl/patterns/pattern_string.hpp`)
reads exactly `size` bytes into a string of that length and returns it,
embedded NULs and all, and `sizeof` on the field is N. The only trimming is in
`formatDisplayValue`, which does `find_last_not_of('\x00')` and so drops
*trailing* NULs before printing. `test_pattern_strings.hpp` covers `str`
literals only and has no `char[N]` case; the char-array path is
`ast_node_array_variable_decl.cpp:253`, and `TestPatternAttributes`
(`test_pattern_attributes.hpp`) has the one `char s[5]`. `Str {Padded {size: N,
pad: 0}}` is therefore the right lowering, and a `Terminated` would read the
wrong length.

**A `[while(c)]` condition is checked before each element, and `$` inside it
is the absolute position that element would start at.** Confirmed against
`TestPatternArrays` in `test_pattern_arrays.hpp` and `TestPatternDollar` in
`test_pattern_dollar.hpp`. In the first, `u8 second[while(!end_of_signature())]`
with `fn end_of_signature() { return $ >= 8; }` starts at offset 4 and the test
asserts `sizeof(sign.second) == 4`: the check runs at 4, 5, 6, 7 and fails at
8, so it is a before-the-element check over the position the element would
occupy, not an after-the-element one. In the second,
`u8 array[while($ == addressof(this) || $[$-1] != 0x36)]` reads `$` as that
same position, `addressof(this)` as the enclosing struct's start, and `$[i]`
as a read at an absolute address, and the same `ReadArray` placed at four
offsets gives four different lengths. The condition's scope is the ordinary
one: the list's earlier siblings, `this` for the enclosing struct, `parent` to
climb out, and functions; the element being read does not exist yet and cannot
be named. `Until::While(Expr)` with `SpacePos` for `$` matches this exactly.
`Until::Cond`, the after-the-element check, does not.

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
  opaque statements. The bar was 310/310 and 46/46; it stands at **309/310
  and 46/46**, and the one that is left is an upstream typo, not a gap here:
  `patterns/ffx/jp/txt/single2.hexpat:4` writes `String2jp` where
  `patterns/ffx/utils.hexpat:382` declares `String2Jp`, and the sibling
  `double2.hexpat` in the same directory spells it correctly. The reference
  rejects an undeclared type in type position, so it rejects that file too.
  Nothing upstream caught it because `tests/patterns/CMakeLists.txt` globs
  `patterns/*.hexpat` and never descends into subdirectories. Treat 309/310 as
  the ceiling until upstream fixes the spelling.
* Corpus: `cargo run --example hexpat_gaps` over `~/github/ImHex-Patterns/patterns`
  (gated by `IMHEX_PATTERNS=<path>`) prints, per pattern, clean / gaps
  with counts, and the totals go in this file's status section. Then for
  each of the 202 pairs in `tests/patterns/test_data`, evaluate the
  converted template over the sample and report whether it reads to the
  end without an error; that pass rate is reported as it is.
* Browser: `web/test/hexpat.browser.mjs` in the style of `ksy.browser.mjs`:
  paste a small pattern, apply, see the fields; open the library list, pick
  one (served from a local fixture, not the network), see the notice.

## What the parser turned up that the plan did not say

Found while building `crates/core/src/hexpat/`, and worth knowing before
lowering starts.

* **The parser has to resolve includes itself.** It is type aware: a name in
  type position must already be declared, and whether a `<...>` argument is
  read as a type or as an expression depends on the declared parameter's kind.
  So `parse` takes a `Resolver` (`path -> name + text`, the search order of
  `resolvers.cpp`) and parses `#include` and `import` as it goes. Without
  `includes/`, about a third of the corpus does not parse at all. The
  `Includes` trait the plan puts in `includes.rs` should adapt to this rather
  than replace it.
* **`#include` and `import` are not the same thing.** `import` is its own
  translation unit: it is parsed with a fresh type table and only its
  declared types come back. `#include` splices tokens, so the included file is
  parsed with the includer's table *and hands the whole table back* --
  `patterns/GoldBox/GB_CHR.hexpat` includes `GB_ENUM.cs` and then
  `GB_STRUCT.cs`, and the second only parses because it can see the first's
  `COLORNAME`. Each file is spliced once, and the two once-guards check each
  other, as the reference's do.
* **Eight host-registered types live under `builtin::`.** ImHex's decode plugin
  declares `builtin::hex::dec::{Json, Bson, Cbor, Bjdata, Msgpack, Ubjson}`
  with one value parameter each, `EncodedString` with two and `Instruction`
  with four. `includes/hex/type/*.pat` wrap them, so those four include files
  do not parse without the table. It is in `parser.rs` as `BUILTIN_TYPES`.
* **Precedence is not C's.** Bitwise `| ^ &` bind *tighter* than the
  comparisons, so `a & 1 == 0` is `(a & 1) == 0` in a `.hexpat` and
  `a & (1 == 0)` in C. Any expression the report prints, and any condition
  lowering hands to `Expr`, has to keep that shape.
* **`#pragma` takes a key and the rest of its line.** The lexer reads the first
  word after the directive as the key and everything to the end of the line as
  the value, unprocessed, which is how `#pragma magic [ 4D 5A ] @ 0x00` keeps
  its brackets. Pragma names are not checked when parsing; the reference
  rejects an unknown one when the pattern runs, so that check belongs to
  lowering.
* **A top-level `T x;` reads nothing.** Only a declaration with an `@` reads
  the file; a bare one is a global variable, the same as `T x = e;` without
  the value. The parser marks it `FieldKind::Local`, so lowering never has to
  guess, and never emits a read at offset zero for it.
* **Five places where the parser is looser than the reference**, each because
  it only ever rejects and none of them fires on the corpus. Tighten them if
  a pattern ever depends on it, but not before. (1) An `in`/`out` variable's
  type is not checked to resolve to an integer, float, `bool`, `char`, `str`
  or enum. (2) A `namespace auto` alias is treated as a plain namespace;
  nothing in the corpus imports with an alias except `import * from`, which
  does not parse the file at all. (3) An attribute is accepted after an
  assignment, a call or a `return`, where the reference says "Cannot use
  attribute here." (4) `$ += e` is accepted at the top level, where the
  reference has only `$ = e`. (5) An imported type does not overwrite one the
  importer already declared under the same name; the reference's last import
  wins.

## Status

2026-09-17: planned. Census run once from the scratchpad.

2026-09-17: the IR additions are on main (416ce67):
`Expr::BitOr`/`BitXor`/`BitNot`, `Until::While`, `Expr::SpacePos`/`SpaceSize`,
`StructDef::overlap` with `Ty::union_structure`, and `Expr::PeekIn` for the
absolute read. Evaluation, `template_text` rendering, the relations panel,
unit tests over a `MemSource` and a notation snapshot. The converter is not
started.

2026-09-17: lexer, AST, expression tree and parser landed in
`crates/core/src/hexpat/`, with `crates/core/examples/hexpat_parse.rs` as the
syntax oracle. 309/310 patterns and 46/46 includes parse; the three facts above
are confirmed. 57 unit tests, one per case of the reference's language tests
plus the strictness checks. No lowering yet: the imperative half parses into
`ast::Statement`, which keeps a coarse kind, the byte range and the source text
of each construct, ready to become a gap.

2026-09-17: the lowering landed, in `crates/core/src/hexpat/{lower,types,includes}.rs`
with `pub fn convert(text, &dyn Includes)` in `hexpat/mod.rs`, and the
`Report`/`Became`/`Gap`/`Note` shape moved out of `ksy/` into
`crates/core/src/report.rs` so both converters share it. 41 tests in
`crates/core/tests/hexpat_lower.rs`, one per mapping row and one per imperative
construct, each reading bytes written by hand for it, plus three
`template_text::render` snapshots in
`crates/core/tests/snapshots/template_text/hexpat_*.txt`.

`cargo run -p qubero-core --example hexpat_gaps -- ~/github/ImHex-Patterns`, as
it stands:

```text
81 clean / 226 with gaps of 1744 total, 3 refused, of 310 patterns
samples: 36 read to the end, 109 stopped at a gap, 40 errored, 5 without a converted pattern, of 231 files under test_data
pass rate: 145/185 read without an error (78%)
```

The three verdicts are defined in the example's own header, so the number means
the same thing next time: **reads to the end** is every node resolved and a
report with no gaps, **stopped at a gap** is every node resolved and a report
with gaps, and **error** is a node that did not resolve, with the path that
failed. The three refusals are the parser's, not the lowering's:
`ffx/jp/txt/single2.hexpat` is the upstream spelling mistake described above,
and `ffx/all/mon.bin.hexpat` and `notepadwindowstate.hexpat` both redeclare a
name an include already declared, which is looseness (5) in the parser's list,
the other way round.

The commonest gap reasons, which is what says what to do next:

```text
    293  left unread: the member before it was not placed
    203  an assignment
     54  a local variable of a function at the top level
     50  Offset is not a field in scope here
     42  an `if` statement outside a structure at the top level
     41  [[transform]] replaces the value with what a function returns
     35  `$` reached through a path
     28  a `break`
     24  addressof(this)
     23  an in/out variable
```

`left unread` is not a gap of its own: it is the marker on every structure that
ended early because the member before it could not be placed, so the count of
things that could not be said is nearer 1,450. Taking the rest in order, the
imperative half is most of it and none of it is an IR question. The three that
*are* IR questions, and what each would cost:

* **A name declared inside an `if` block, read after the block** (lua40, lua50,
  lua51, gmd, tiff, wav, and about ninety gaps between them once the `left
  unread` each one causes is counted). `Expr::Ref` is resolved by
  `Evaluator::find_field`, which searches the fields before this one in this
  structure and then climbs out, and never descends into an earlier sibling's
  inline structure. A hexpat `if` block is not a scope, so the pattern can name
  what it declared inside one. Either the IR's name search steps into an inline
  `When`'s structure, or `if` blocks stop being structures. The first is a
  change to one function and to what `Within` means; the second gives up the
  one-condition-per-block the mapping chose on purpose.
* **`addressof(this)`** (24). The enclosing structure's own start. Nothing in
  `Expr` names it: `Pos` and `SpacePos` are where the *field* is, and `StartOf`
  wants a field to be the start of. An `Expr::HereStart` counted the way
  `SpacePos` counts would cover it, and `bson.hexpat`'s
  `[while($ < addressof(this) + listLength - 1)]` is the shape asking.
* **A low-bit-first bitfield field that crosses a byte** (about forty, across
  `3ds`, `lnk`, `lz4`, `ape`, `ne`, `id3`, `BroEngine/dds`). `decode::lsb_offset`
  refuses one and says why: a twelve-bit field packed from the bottom is the
  whole of one byte and the low nibble of the next, which is not one range in
  an address space numbered from the top of each byte. This is the same gap
  Kaitai's `bit-endian: le` hits, so whatever is done for one does both. Note
  what does *not* hit it: under `big` every width works, since the IR's own
  packing is most significant bit first, and under `little` a field that is a
  whole number of bytes on a byte boundary is an ordinary little-endian number.

### What the plan got wrong

Four things, each found by running it:

* **`char x[N]` is `StrLen::Fixed`, not `StrLen::Padded`.** The reference's
  reading is right in the plan, all N bytes with the embedded NULs, and the IR
  type named for it is the wrong one: `StrLen::Padded` ends the *value* at the
  first pad byte, so `char s[6]` holding `ab\0cd` would read as `ab` and
  `s == "ab"` would come out true where the reference says false. `Fixed` is
  the exact one. The one difference left is the display, where the reference
  drops trailing NULs before printing and the IR prints what the field holds.
* **An unknown `#pragma` is not an error, and neither is an unknown
  attribute.** `Preprocessor::process` (`preprocessor.cpp:623`) runs the
  handler for a pragma it has one for and passes over every other, and
  `Attributable::hasAttribute` looks its named attributes up by name and never
  asks what the others were. So `#pragma organization` in `fs/refs.hexpat`,
  `#pragma authors` in `gguf.hexpat`, `#pragma little` in `msf.hexpat` and
  `[[attribute("hidden")]]` in `job.hexpat` all run upstream and do nothing.
  Both are notes here. Refusing them cost six patterns for no reason.
* **`[[transform]]` is a gap, not a note.** A `[[format]]` changes what is
  printed and a `[[transform]]` changes what the field *is*: a `cpio` header's
  `filesize` is a `u32` with a byte-swapping transform on it, and the field
  after it is that many bytes long. Showing the raw number would be a note;
  letting a length read it is reading the file wrongly and saying nothing,
  which is the one thing this converter exists not to do. 41 gaps.
* **An enum range is written out as one case per value, not as an
  `EnumSpan`.** `EnumSpan` has a start and a step and no end, and
  `EnumSpan::count` answers for every value from `from` upwards, so
  `A = 0x10 ... 0x1F` as a span would name 0x20 as A as well. The values are
  written out up to 4,096 of them and anything longer is a gap.

Two smaller ones. `[[name("x")]]` cannot become `Field::name`: the IR has one
name per field and every expression, path and edit goes through it, so the
declared name stays the name and the display name goes in the field's prose. And
a top-level `T x = e;` is not only a gap, because later placements name it, so
it is kept as a zero-width `Computed` machinery field of the root, the same as a
local inside a structure.

One the plan did not decide, now decided: `[[no_unique_address]]` is an `At` at
the field's own position, `Ty::at_in_window(Expr::Pos, ..)`, which reads where
the field stands and takes no room, so the field after it starts in the same
place. A union of the run was the other candidate and is wrong: `StructDef::overlap`
makes the structure as long as its longest field, and `no_unique_address` means
the field contributes nothing at all. `bmp.hexpat` is why it matters -- its
`data` field is the whole file read at offset zero -- and with the union
lowering every placement after it ran past the end.

### Two evaluator bugs the corpus turned up

Both were panics rather than errors, caught per sample by `hexpat_gaps` and
counted as errors; neither was in the converter. Both are evaluation errors
naming the field now, each with a unit test in `eval/tests.rs`, and the corpus
run is free of panics.

* `eval/read.rs`, `self.str_span(doc, r, size)?.expect("text field")`, on
  `fbx.hexpat` and `wad.hexpat` over their own samples. `str_span` measures by
  the field's own type, and an enum on a text type reaches the text arm with
  the field's type still the enum. Now `typeCode is not a text field, so it
  cannot be read as text`.
* `eval/size.rs`, `attempt to divide by zero`, counting a run to the end by
  dividing the room by a fixed element size that came to nought: `mp4.hexpat`,
  `qoi.hexpat`, and `blend.hexpat` and `tar.hexpat` before their first real
  error. `stride` now keeps a fixed width of nought to arrays, as it already
  did for a computed one, so the run is walked and the walk refuses the element
  with `data repeats an element of zero size`. Tar reads to a gap now, which is
  the one sample the pass rate gained.

The error messages are built in cold, out-of-line helpers on purpose. The walk
and the value arms sit in frames the depth backstop is measured against, and a
`format!` in either put `a_run_that_holds_a_run_is_refused_at_the_same_depth`
over the stack budget one field short of the count.

### The panel, the library and the bundled subset, 2026-09-17

All landed. What is worth knowing beyond the plan:

* **The panel is shared with the `.ksy` one.** `web/src/convertpanel.ts` holds
  the layout, the debounced conversion, the report, the caret jump and the
  bottom bar; `ksypanel.ts` and `hexpatpanel.ts` each supply the entries of the
  core they call, how a report path finds its line, and what they offer beside
  the text box. Both keep the `kp` class and its stylesheet, and each adds
  `kp-ksy` or `kp-hexpat` so a rule or a test can tell them apart.
* **`set_hexpat_template` takes a third argument, `name`.** A `.ksy` names
  itself in `meta/id`; a pattern has no such key, so the template's name comes
  from the file it was dropped or fetched as, and a bare paste is `pattern`.
  The same goes for `preview_hexpat_template`.
* **A failed conversion carries `missing`.** Parsing stops at the first include
  it cannot find, so the wasm layer also scans the text for every `#include`
  and `import` and reports every path neither the caller nor the built-in table
  answers for. The panel gives each a box. An include's own includes appear as
  they are supplied, which is one round each; nothing pretends otherwise.
* **`hex/type/json` joined the built-in declarations.** `gltf.hexpat` imports
  it, and without it the bundled copy did not parse. Six type names and their
  arity, written out fresh; `types::known` already answers for the whole `hex::`
  namespace with a gap, so the bodies are never lowered.
* **`#pragma magic @ -0x0200` is read as no magic at all.** `parse_magic` takes
  a `u64` address, so a magic measured back from the end of the file fails to
  parse and is noted as "a magic that starts with a byte it does not care
  about", which is the wrong reason. It costs `vhd.hexpat` its signature and
  nothing else. Worth fixing when negative offsets are worth having.
* **Nothing sniffs by a bundled pattern.** The four carry their `#pragma magic`
  in the table, and the template search matches a typed-in byte query against
  it, but no dropped file is claimed by one: the built-in probes and the Kaitai
  collection already answer that, and `mbr`'s two bytes at 0x1FE are not
  evidence they lack.
* **The four bundled patterns all have gaps**, 16 between them, which is the
  language rather than the files: `crates/core/formats-hexpat/README.md` says
  which gaps each has and why they are off the path through the file.

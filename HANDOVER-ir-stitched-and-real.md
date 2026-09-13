# Handover: two IR additions, designed and not built

Written 2026-09-14 from a design pass (Fable) that read the evaluator at
`7a3d53a`. Both are gaps named in `HANDOVER-science-formats.md`: S7 is the
"one space stitched from several runs" gap (BAM records across BGZF blocks,
PDB scattered streams, HDF4 linked blocks, Godot RSCC), and S8 is "computed
values that are not integers" (NIfTI and FITS fractional scaling, GRIB simple
packing's worth, NIfTI's float `vox_offset`). The designs came from reading
the code, not running it: where they turn out wrong, follow the code and say
so.

---

# S7. One space stitched from several runs

## What the evaluator fixes before any shape is chosen

- A space is one buffer. `Spaces` keeps `bufs: Vec<Arc<Vec<u8>>>` beside
  `traces`, `open_space_at` fills one by `decode_traced` over a `Decoded`
  node's whole run, and `read_in` (`eval/read.rs`) bounds-checks against
  `src.len()`. Nothing evicts; a space lives until `invalidate_from` drops
  them all.
- The inner child's `limit` is fixed at placement: `place_child`
  (`eval/mod.rs`) hands it `Place { offset: 0, limit: spaces.len_bits(space),
  space }`, and `Remaining`, `Until::End`, `count_repeat` and `window_of` all
  read that limit. So a stitched space's total length must be known before its
  contents are placed, or `bytes(Remaining)` and `Repeat` break.
- A `Trace` cannot span a gigabyte: `RawStep.in_start`/`out_start` are `u32`
  (`codec.rs`). `TraceBuilder::absorb` stitches traces, but only under 512 MiB
  of input.
- `read_in` is `&self`. Inflating a part on demand inside it needs interior
  mutability or a `&mut` pre-step.
- `Gather`'s walk (`eval/gather.rs`) already has resumable frames over
  `Step`s, charged once per record reached; but its `Each` refuses lists of
  plain elements (`holds_no_fields`), and a stitched walk lands on runs, which
  are plain.
- `locate` never enters a non-zero space; `placed::record` skips `space != 0`;
  `memo.forget_decoded` drops nodes by `space != 0` or under a `Decoded`;
  `prepare_write` refuses `space != 0`; `space_at` resolves the nearest
  `Decoded` ancestor. Each of these has a `matches!(.., Ty::Decoded ..)` a new
  node has to join.

## Candidates

**A. `Ty::Stitched { from, part_len, len, inner }`.** A zero-width node like
`Gather`; `from` walks to the runs in order; the parts are joined into one
space read lazily. Stored runs (PDB, HDF4) and packed runs (BGZF, RSCC) are one
construct with two part sources. Recommended.

**B. A multi-member codec over one contiguous run.** Rejected: `CAP_BYTES` (64
MiB) against gigabyte BAMs, nothing lazy, and it covers none of PDB, HDF4 or
RSCC, whose runs are scattered or interleaved with headers.

**C. A list that opens a space (`Repeat { joined: true }`).** Rejected: the
list is in file space and its children are the members; the contents have
nowhere to hang, and `Idx`, `locate` and the listing would all have two
meanings for one node.

## The IR change

```rust
// template.rs
/// One stream kept in several runs, read as what it holds. Zero bits where
/// it is declared, like a `Gather`; its one child is read in a space of its
/// own made of the parts joined in walk order.
Stitched {
    /// The walk to the runs, in the order their bytes go. Lands on a `Decoded`
    /// field, whose output is the part, or on any other field, whose bytes are.
    from: Arc<[Step]>,
    /// How many bytes a part comes to, worked out one past the last field of
    /// the structure the run is a field of (the frame `Gather.offset` uses).
    /// None: a stored run is as long as it is, and a packed run is opened to
    /// find out.
    part_len: Option<Expr>,
    /// The stream's total, where the format states it; the last part is cut
    /// there. Worked out where the node is declared.
    len: Option<Expr>,
    inner: Box<Ty>,
}
pub fn stitched(from: Vec<Step>, part_len: Option<Expr>, len: Option<Expr>, inner: Ty) -> Ty
```

```rust
// eval/space.rs
enum Backing {
    Whole { bytes: Arc<Vec<u8>>, trace: Trace },          // today's Decoded
    Stitched(Box<Stitch>),
}
struct Stitch {
    parts: Vec<Part>,             // by start; complete before the child is placed
    len_bytes: u64,
    cache: RefCell<PartCache>,    // inflated packed parts, LRU, total <= STITCH_CACHE_BYTES
}
struct Part { start: u64, len: u64, path: Vec<usize>, source: PartSource }
enum PartSource {
    Stored { space: u32, at_bits: u64 },
    Packed { space: u32, at_bits: u64, size_bits: u64, codec: Codec },
}
pub struct PartHit { pub index: usize, pub path: Vec<usize>, pub in_part: u64, pub run_offset_bits: u64 }
```

## How it evaluates

*Opening* (`eval/stitch.rs`, new; `open_stitched_at(path) -> Opened`, kept in
`Spaces::opened` like a `Decoded`). The walk reuses `gather_step`'s frame
machinery, factored out with a landing rule: a landing is any node, stepped
through an `At`, not only a struct with fields. Per run reached: resolve it; a
`Decoded` gives `Packed` with the codec from `codec_at` (resolved now, so
`read_in` never needs it) and `part_len` evaluated at `record_frame(nearest
struct ancestor)`; with `part_len: None` a packed part is inflated now to
measure it (into the cache). Anything else gives `Stored` of the node's own
size. Reached parts are charged (`spend`) and the walk resumes across goes;
`Pending` propagates as it does in `gather_record`. `len` cuts the sum. A part
that does not start on a byte refuses the whole (`Unaligned`); no parts is a
space of nothing.

*Reads.* `read_in` on a stitched backing: binary-search the part, copy across
as many parts as `[at, at+n)` covers. `Stored` parts read from the file or
their source space; `Packed` parts come from the already-open member space
when `spaces.get(path)` is `Space(id)`, else from the `RefCell` LRU, else
`decode_traced` now (per-part `CAP_BYTES`, total cache cap; evict least
recently read). A part that will not inflate, or comes to a length other than
`part_len` claimed, fails the read with the part named ("block 9000 would not
unpack"); a `Repeat` stops with `repeat_trouble`.

*Placement and the rest.* `place_child` with a `Stitched` parent: idx 0 →
`Place { offset: 0, limit: len_bits, space }`. `child_count`: 1 opened, 0
refused. `size_of`: zero where declared. `space_at`, `memo.forget_decoded`,
`placed::places`, `kinds`, `graph`, `diagram`, `template_text::inline`,
`shape::placed` (a new `Placed::Stitched`) each get the arm `Decoded` has.
`locate` and `spans` need nothing: the cursor on block 3's bytes still lands
on block 3's run.

*Mapping back.* No stitched trace. `Evaluator::part_of(space, byte) ->
Option<PartHit>` answers from the part table; `map_out(space, byte)` on a
stitched backing delegates to the part's own trace and returns the step with
`run_offset_bits`, so the web's `markFromStep` marks the right block. For
BGZF, `run_offset_bits/8 << 16 | in_part` is the virtual offset. `open_space`
(a tab): only when `len <= CAP_BYTES`; otherwise `Refused(TooLarge)` with the
listing still reading lazily.

*Edits.* Stage 1: `prepare_write` refuses `space != 0` as now. Stage 4: allow
a write when every bit of the field lies inside one `Stored` part in space 0,
translating `offset_bits` through the part table; a field across two parts is
refused naming both.

*Laziness.* One small part-table entry per part (a 1 GB BAM is about 16k).
Nothing inflated until read; the cache is bounded. A BGZF stream has no total,
so `len_bits` is one pass over every block's header and trailer (no
inflation); an open-ended limit would break `Remaining` and `Until::End`.

## Files touched

`template.rs`; `eval/space.rs`; `eval/stitch.rs` (new); `eval/gather.rs`
(factor the frame walk); `eval/mod.rs` (`place_child`, `space_at`, `node`,
`open_space`/`unpack`, `map_out`, `part_of`); `eval/read.rs`; `eval/size.rs`;
`eval/memo.rs`; `eval/placed.rs`; `eval/shape.rs`; `eval/origin.rs`;
`eval/relate.rs`; `eval/kinds.rs`; `eval/graph.rs`; `eval/diagram.rs`;
`template_text.rs`; `encode.rs` (stage 4); `explain.rs`
(`Explain::StitchedPart`); `crates/wasm/src/lib.rs`
(`MapStepDto.run_offset_bits`, `part_of`); `web/src/main.ts`, `inspector.ts`.

## Build order, with tests

1. **Stored parts.** Node, backing enum, walk, read dispatch, the `Decoded`
   arms, refusal of edits. PDB scattered streams. `eval/stitch.rs`:
   `a_stream_kept_in_scattered_pages_reads_as_one`,
   `a_field_cut_across_two_parts_reads_whole`,
   `a_declared_length_cuts_the_last_part`, `a_part_of_no_bytes_adds_nothing`,
   `the_cursor_on_a_page_lands_on_the_page_and_not_on_the_stream`,
   `nothing_inside_a_stitched_space_is_editable`,
   `an_edit_to_the_file_closes_a_stitched_space`. `tests/pdb_real.rs`:
   `a_scattered_stream_reads_as_its_body`.
2. **Packed parts.** Cache, `part_len`, `codec_at` at walk time, failure per
   part. BGZF/BAM. `blocks_of_one_stream_read_as_the_stream`,
   `a_member_is_measured_from_its_trailer_and_not_inflated`,
   `a_part_that_will_not_unpack_ends_the_stream_there_and_says_which`,
   `a_part_that_comes_to_another_length_than_claimed_is_refused`,
   `parts_inflate_when_read_and_go_when_the_cache_is_full`,
   `a_walk_that_runs_out_carries_on_from_the_part_it_stood_on`.
   `tests/bam_real.rs`: `every_record_of_range_bam_is_a_field`,
   `a_record_cut_across_seventeen_blocks_reads_whole`,
   `the_template_and_the_side_reader_agree_on_every_record` (`bam_records.rs`
   becomes the oracle), `mpileup_header_reads_past_its_first_block`.
3. **Mapping.** `part_of`, `map_out` delegation, origin row, panel.
   `a_byte_of_a_stitched_space_names_its_part_and_offset`,
   `a_records_virtual_offset_matches_the_bai`.
4. **HDF4, RSCC, stored-part edits, tabs under the cap.**
   `tests/hdf4_real.rs::tdata_values_kept_in_linked_blocks_match_pyhdf`,
   `tests/godot_real.rs::a_resource_written_across_blocks_opens_as_the_resource`,
   `a_field_inside_one_stored_part_writes_to_that_part`,
   `a_field_across_two_stored_parts_is_refused_naming_both`.

## Risks

- Factor `Gather`'s frame walk for the `Each`-on-plain-elements difference; do
  not copy it.
- `part_len` claims: a gzip `original_size` is modulo 2^32 (fine for 64 KB
  BGZF members, wrong for a multi-gigabyte plain member); the inflate-time
  length check catches it.
- Settling a BGZF length on a remote file streams every chunk once.
- `bam_real.rs` paths like `[0, i, "compressed", 0]` and the `Idx == 0` payload
  sniff move under `stream`.
- `kinds` must exclude a stitched space's contents as it excludes decoded
  contents.
- Check HDF4's `linkinfo_t` in `hfile.h` for a `first_length` field before
  building the HDF4 sketch: the unit test builds bytes to match the template,
  so a missing field would not be caught.
- Multi-member plain gzip's false CRC is a different fix. FITS `CONTINUE` fits
  the shape once `continue_body` splits the `&` out of each piece, then
  `Stitched { inner: text }` is the text-joining type the FITS notes ask for.

## Format sketches

**BAM** (`formats/bam.rs`). `payload()` and its `Idx == 0` sniff,
`whole_or_cut` and `if_room` go; every block's contents become
`T::bytes(E::Remaining)`.

```rust
pub fn bgzf() -> Template {
    Template::new("bgzf", T::structure("BGZF", vec![
        ("blocks", T::repeat(block(), Until::End)),
        ("stream", T::stitched(
            vec![Step::field("blocks"), Step::each(), Step::field("compressed")],
            Some(E::field("original_size")),
            None,
            stream_contents(),
        )),
    ]))
}

fn stream_contents() -> T {
    let magic = |m: &[u8]| u32::from_be_bytes(m.try_into().expect("four bytes")) as i128;
    let by_magic = T::switch(E::peek(32, Big),
        vec![(magic(BAM_MAGIC), bam_stream()), (magic(CSI_MAGIC), csi_stream())], super::decoded_text());
    T::switch(E::lit(3).less_than(E::Remaining), vec![(1, by_magic)], super::decoded_text())
}

fn bam_stream() -> T {
    T::structure("Bam", vec![
        ("magic", T::magic(BAM_MAGIC)),
        ("l_text", T::u32(Little)),
        ("text", T::text(StrLen::Fixed(E::field("l_text")), Encoding::Utf8)),
        ("n_ref", T::u32(Little)),
        ("references", T::array(reference(), E::field("n_ref"))),
        ("records", T::repeat(record(), Until::End)),
    ])
}
```

**PDB** (`formats/pdb.rs`). Keep `runs_together`: a one-run stream stays in the
file, editable. `blocks` becomes records of `number` and a computed `at`.

```rust
("contents", T::switch(empty(), vec![(1, nothing())],
    T::switch(runs_together("run", count()),
        vec![(1, T::at(E::elem_field("blocks", E::lit(0), &["at"]), T::sized(size(), body())))],
        T::structure("PdbScatteredStream", vec![
            ("pages", T::pointer_list_sized("blocks", &["at"], Anchor::File, E::lit(0), T::bytes(E::field("block_size")))),
            ("stream", T::stitched(vec![Step::field("pages"), Step::each()], None, Some(size()), body())),
        ])))),
```

**HDF4 linked blocks** (`formats/hdf4.rs`). `special_element()` takes the
`inner` the values read as; the link tables are a `Chain` whose `next` is a
computed offset (`extend_chain_to` reads `next` through
`node().value.as_int()` and skips the all-ones test for zero-bit fields).

```rust
fn linked_blocks(inner: T) -> T {
    let at_ref = |r: E| look(key(20, r), &["offset"]);
    let len_ref = |r: E| look(key(20, r), &["length"]);
    let table = T::structure("Hdf4LinkTable", vec![
        ("next_ref", u16be()),
        ("block_refs", T::array(u16be(), E::field("number_blocks"))),
        ("next_at", T::computed(at_ref(E::field("next_ref")))),
        ("blocks", T::array(
            T::at(at_ref(E::elem("block_refs", E::idx())), T::bytes(len_ref(E::elem("block_refs", E::idx())))),
            E::field("number_blocks"))),
    ]).machinery(&["next_at"]);
    T::structure("Hdf4LinkedBlocks", vec![
        ("length", T::i32(Big)),
        ("block_length", T::i32(Big)),
        ("number_blocks", T::i32(Big)),
        ("link_ref", u16be()),
        ("tables", T::chain(at_ref(E::field("link_ref")), &["next_at"], Anchor::File, table)),
        ("values", T::stitched(
            vec![Step::field("tables"), Step::each(), Step::field("blocks"), Step::each()],
            None, Some(E::field("length")), inner)),
    ])
}
```

**Godot RSCC**: `("resource", T::stitched(vec![Step::field("blocks"),
Step::each()], Some(E::field("block_size")), Some(E::field("uncompressed_size")),
unpacked()))` after `blocks`, with every block's contents
`T::bytes(E::Remaining)`.

---

# S8. Computed values that are not integers

## What the evaluator fixes

`eval_expr_at` returns `i128`; a leaf reads a node's `Value` through
`as_int()`, which is `None` for `Value::Float`, so `E::field("scl_slope")` on an
`F32` fails "is not a number". `Ty::Computed` caches `Resolved.computed:
Option<i128>` and answers `Value::Int`. The formula writer
(`template_text::write_expr`) and the substituter (`relate.rs::leaf_value`) are
one writer over the leaves. Sizes, counts, addresses, switch keys, `When`,
`UIntExpr` widths and `Packing` settings all call `eval_expr`.
`encode::editable` already excludes `Computed`. `Value::Float`, `Unset::Float`
and `narrow_f32` exist. The Kaitai lowerer rejects float literals
(`ksy/lower.rs`).

## Candidates

**A. A separate `FExpr`** and `Ty::ComputedFloat(FExpr)`. Statically typed, but
a second evaluator, second spelling, second origin walker, second substituter
and second diagram arms for operators that already exist.

**B. A `Value`-typed evaluator.** One language, but every caller of
`eval_expr` unwraps, integer and real division fork silently, the memo cache
widens, and the integer path pays a match per operation. Ruled out.

**C. One enum, two evaluators, the mode chosen at the node.** Recommended.
`Ty::ComputedReal(Expr)` runs `eval_real_at`; everything else runs
`eval_expr_at` as today. Four new leaves and one bridge. A real in a size
compiles and fails at evaluation with a sentence, which is how sizes and counts
stay integers.

## The IR change

```rust
// template.rs
pub enum Expr {
    // ...
    /// A number with a fraction, as the template writes it: `0.5`. It has no
    /// whole-number reading, so it fails in a size, a count or an address.
    Real(f64),
    /// The real number a run of text spells, `2.0E+01` and Fortran's `1.0D-3`
    /// included: what a FITS card holds. Nothing found reads as 0, the way
    /// `Tagged` answers, so `Or` can say what to do without it; text that is
    /// there and is not a number fails.
    RealText(Box<Expr>),
    /// Two, or ten, to an integer power, which is as often negative as not.
    Pow2(Box<Expr>),
    Pow10(Box<Expr>),
    /// The whole part of a real, towards nought: the one way a real enters a
    /// size, a count or an address. NIfTI's `vox_offset` is a float that
    /// places bytes.
    Trunc(Box<Expr>),
}
pub enum Ty {
    // ...
    /// A field of no bits whose value is a real number worked out from others.
    ComputedReal(Expr),
}
// Builders: E::real(0.5), E::real_text(e), E::pow2(e), E::pow10(e), E::trunc(e), T::computed_real(e).
```

```rust
// eval/mod.rs
enum Computed { Int(i128), Real(f64) }     // Resolved.computed: Option<Computed>
```

## Evaluation

`eval_real_at(doc, at, e, here) -> R<f64>` in `eval/expr.rs`:

- `Real(v)` → `v`; `Lit(v)` → `v as f64`.
- `Add`, `Sub`, `Mul`, `Div`, `Min`, `Max`, `Or`, `Cond` → recurse in real mode
  (`Cond`'s test and `Or`'s zero test in their own modes; division by 0.0
  fails as integer division does).
- `Pow2(e)`, `Pow10(e)` → integer exponent, `2f64.powi`, `10f64.powi`.
- `RealText(p)` → `text_at`, trimmed, `D`/`d` → `E`, `str::parse::<f64>`;
  empty → 0.0.
- Leaves that name a field (`Ref`, `Within`, `Elem`, `ElemWithin`, `Tagged`,
  `Placer`, `Sibling`, `Prev`) → the node's `Value`: `Float(f)` → `f`;
  anything `as_int()` reads → `as f64`; else fail. Text is explicit through
  `RealText`, because `as_int` already gives a short `Str` a second reading.
- Anything else → `eval_expr_at` converted.

The integer evaluator gains five arms: `Real` and `RealText` fail ("a number
with a fraction has no place here; wrap it in trunc(...)"); `Pow2(e)` is `1 <<
e` for `0 <= e < 127` and fails otherwise; `Pow10(e)` likewise to 38;
`Trunc(e)` evaluates `e` in real mode and truncates, failing on NaN, infinity or
a value outside `i128`. `primitive_value` answers `ComputedReal` with
`Value::Float`, cached in `Computed::Real`. Guards stay integers: `Switch`,
`When` and every size read int mode.

*The relations panel.* `relation()` takes the mode from the type: for
`ComputedReal`, `result` is `eval_real_at` formatted with `{}` (shortest
round-trip), and `leaf_value` formats a real leaf the same way.
`template_text` spells `Real(v)` with a point always (`1.0`), `RealText(p)` as
`real(p)`, `Pow2`/`Pow10` as `pow2(e)`/`pow10(e)`, `Trunc` as `trunc(e)`;
`inline` writes `computed real {expr}`; `display_name` says `computed real`.
`origin::from_expr` recurses into the four wrappers. `graph::value_kind` →
`"float"`; `diagram::fixed_bits` `Some(0)`; `shape::sizing` `Nothing`;
`listing::plain` includes it; `kinds` as `Computed`.

*Edits.* A `ComputedReal` is not editable; the stored integer beside it stays
editable through the `Scaled { stored, worth }.payload(&["stored"])` pattern.

## Files touched

`template.rs`; `eval/expr.rs`; `eval/read.rs`; `eval/mod.rs` (`Computed`);
`template_text.rs`; `eval/relate.rs`; `eval/origin.rs`; `eval/graph.rs`;
`eval/diagram.rs`; `eval/shape.rs`; `eval/listing.rs`; `eval/kinds.rs`;
`formats/nifti.rs`; `formats/fits.rs`; `formats/grib.rs`.

## Build order, with tests

1. **IR and evaluators.** `eval/tests.rs`:
   `a_real_computed_field_reads_as_the_number_it_works_out`,
   `a_real_has_no_place_in_a_size_or_a_count`,
   `the_whole_part_of_a_float_places_bytes` (352.0, 352.9, -1.5 → "negative
   offset", NaN fails), `digits_read_as_the_real_they_spell` (`2.0E+01`,
   `1.0D-3`, `.5`, `1.`, `32768`, `abc` fails, nothing found is 0),
   `two_to_a_negative_power_is_a_real_and_not_a_shift`,
   `an_integer_computed_field_reads_as_it_did`. `template_text.rs`:
   `real_expressions_read_as_written`.
2. **Panels.** `a_real_relation_is_written_with_its_values_in_place`,
   `a_real_computed_field_is_not_editable`,
   `a_real_field_names_the_fields_it_reads`.
3. **NIfTI.** `data_offset =
   T::computed(E::trunc(E::field("vox_offset")).at_least(E::lit(0)))`;
   `scalable` reads the floats.
   `tests/nifti_real.rs::functional_voxels_are_worth_what_scl_slope_says`.
4. **FITS.**
   `tests/fits_real.rs::a_scale_written_with_an_exponent_scales_the_column`.
5. **GRIB simple packing.**
   `tests/grib_real.rs::a_simply_packed_value_agrees_with_the_panel`.

## Risks

- Mode-dependent leaves: `E::field(f32)` works in a `ComputedReal` and fails
  in a `Computed`. The failure message must say which mode it was read in.
- `{}` on f64 prints `0.30000000000000004` for `0.1 + 0.2`; oracle checks
  compare doubles, not strings.
- `powi` beyond 10^22 is inexact.
- The relations panel drops a relation when `substituted == result`; a real
  relation with one leaf can trip it.
- A FITS cell still walks the header per ask (the value-cache note stands).
- Kaitai `FloatNum` stays rejected.

## Format sketches

**NIfTI**:

```rust
fn scalable(stored: T, width: i128) -> T {
    let slope = E::within(&["header", "scl_slope"]);
    let inter = E::within(&["header", "scl_inter"]);
    let worth = E::field("stored").mul(slope).add(inter);
    let with = T::inline_structure("Scaled", vec![("stored", stored.clone()), ("worth", T::computed_real(worth))])
        .payload(&["stored"]);
    T::switch(E::within(&["header", "scaled"]), vec![(1, shaped(with, width))], shaped(stored, width))
}
```

**FITS**:

```rust
let real = |prefix: &str| E::real_text(numbered_at(prefix, n.clone(), &["body", "value", "text"]));
let scale = real("TSCAL").or(E::real(1.0));
let with = T::structure("Scaled column", vec![
    ("scale", T::computed_real(scale)),
    ("zero", T::computed_real(real("TZERO"))),
    ("values", T::array(worth_of(ty, E::field("scale"), E::field("zero")), r.clone().at_most(E::Remaining.div(E::lit(width))))),
]).machinery(&["scale", "zero"]).payload(&["values"]);
```

**GRIB simple packing**:

```rust
let worth = reference_value().add(E::field("stored").mul(E::pow2(binary_scale()))).div(E::pow10(decimal_scale()));
T::inline_structure("Packed", vec![("stored", T::uint_expr(width, Big)), ("worth", T::computed_real(worth))]).payload(&["stored"])
```

Complex packing and spatial differencing stay in `grib_values.rs`.

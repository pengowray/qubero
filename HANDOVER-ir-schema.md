# Handover: a type built from a schema the file supplies (S9)

Written 2026-09-14 from a design pass (Fable) that read the evaluator at
`8abe6f6` and ran `dump_tree` over `uproot-Zmumu-lz4.root`.

**Built 2026-09-14, stages 1 to 4** (`9322815`..`b8205b9`). Where the code
differs from this design: the builder takes `&mut dyn Descriptions` (a generic
`build<S>` cannot sit behind `Arc<dyn>`) and reads descriptions lazily, since
StreamerInfo's own classes are described by the bootstrap set; the cache is
keyed by kind and key and does not keep failures; an edit to description
bytes a build read invalidates the whole evaluator; a class with no
description becomes bytes with a note; member-wise streaming is handled in
the builder; `Step::Deep` finds baskets at any depth (split branches and
`TBranchElement`); `placed::places` answers false for a schema node;
`KeyPart::Bytes` was dropped. Stage 5 (FFS) is not started; what it needs is
in `HANDOVER-science-formats.md` under ROOT.

The design as first written follows. It is the corrected S4 in
`HANDOVER-science-formats.md`: ROOT TTree
baskets cannot be placed by the template because a streamed object's layout
is data in the same file. Where the design is wrong, follow the code and say
so.

## What the evaluator fixes

- `effective` (`eval/mod.rs`) unwraps `Named`, `Sized`, `Switch`, `When`,
  `Match` in a loop and stores the concrete type on `Resolved.ty`. A new arm
  that hands back a built `Ty` is the same move as `Switch`.
  `Template::types` is static and is consulted only from that loop,
  `settled_name`, `unit_of` and `holds_no_fields`.
- `find_field` sees only fields declared *before* the asker (`take(idx)`).
  `streamer_info` is field 17 of `TFile` and `directory` is 16, so nothing
  reached through the directory can name the schema record. Both are
  zero-size `At`s; reorder them.
- No path and no `Step` crosses a `Decoded`: `child_index` answers `None` for
  it and `through_at` steps through `At` only. The four codec parts put their
  `Decoded` at different names and depths (zlib `stream.compressed`, lz4
  `stream.block`, zstd/xz `stream.decoded[0]` through an `at_in_window`). The
  crossing must be "the stream under here", not a name. The global path rule
  must not change: `E::size_of("compressed")` in every codec template would
  change meaning.
- `Step::Each` passes over lists whose elements have no fields
  (`holds_no_fields`, `eval/gather.rs`). `fBasketSeek[*]` are plain `i64`s, so
  a gather "over `fBranches[*].fBasketSeek[*]`" would place one child per
  branch. Records with fields have to exist per basket.
- The class-tag back-reference needs no evaluator state. In `read_any`, a name
  spelled at buffer position *p* is remembered under `(p − 4) + fKeylen + 2`,
  so a later tag *t* names the C string at buffer offset
  `(t & 0x7fffffff) + 2 − fKeylen`. That is an `At` into the same space,
  anchored `Origin` (a `Window` anchor would land on the byte-count `Sized`).
  Object back-references (`fLeafCount`) are 4 bytes and nothing else; placing
  baskets never needs them resolved.
- `Ty::Match` compares text exactly; `text_of` on a switch-chosen `At` fails,
  and only `within_path` steps through a resolved `At`, so a key read from
  such a field is `E::within(&["class_name"])`, not `E::field`.
- `Expr::Deduced`/`Template::deduced_by` is the precedent for a format
  registering Rust that runs beside the IR and whose answers are kept beside
  the memo. `TagIndex` is the precedent for caching by a list's stretch
  `(space, offset, limit)` rather than by path.

## Candidates

**A. `Ty::Schema` with a builder registered per kind.** The template says
where the descriptions are (a `Step` walk) and what keys this node is
(expressions); a Rust `SchemaBuilder` the format registers turns the
description *nodes* into a `Ty`, cached per key. Recommended.

**B. A generic "fields described by records" type** (name, width expression,
type code through an enum map, optional offset). Covers about 80% of FFS and
none of the hard part of ROOT: base classes merged in with their own headers,
counted pointers whose count is a member name written in the record (no `Expr`
names a field by computed text), collection classes whose streamer differs
from their `StreamerInfo`, and subformats reached by name. Rejected as the
primary; A's builders emit exactly what B would.

**C. Dynamic entries in `Template::types` filled by a `Deducer`-like pass.**
Same builder, but the key is one string and the descriptions are found by the
pass rather than named in the template, so the relations panel cannot point at
record X, element Y. Folded into A.

## The IR change

```rust
// template.rs
/// One part of what a schema is looked up by, read in the node's own frame.
pub enum KeyPart { Int(Expr), Text(Expr), Bytes(Expr), TextLit(Arc<str>) }
pub enum Ty {
    // ...
    /// A type whose shape is looked up at evaluation time from descriptions the
    /// file supplies. `table` walks to the description records (a gather's
    /// walk); `key` says which one; `kind` names the builder that turns it into
    /// a type. Zero bits where declared only if what it builds is.
    Schema { kind: Arc<str>, table: Arc<[Step]>, key: Arc<[KeyPart]> },
}
pub enum Step {
    // ...
    /// Into the stream under the node here: depth-first through structures and
    /// `At`s (never into lists) to the first `Decoded` or `Stitched`, landing on
    /// it. A `Field` step taken from a stream goes to its contents first.
    Stream,
}
pub trait SchemaBuilder: Debug + Send + Sync {
    /// Build the type for `key` from the description records `table` landed on,
    /// reading them as nodes. `Err` is a sentence the node fails with.
    fn build<S: Source>(&self, ev: &mut Evaluator, doc: &Document<S>, landings: &[Vec<usize>], key: &[KeyValue]) -> Result<Built, String>;
}
pub struct Built { pub ty: Ty, pub from: Option<Vec<usize>>, pub members_from: Vec<Option<Vec<usize>>> }
impl Template { pub fn with_schema(self, kind: &str, builder: Arc<dyn SchemaBuilder>) -> Template }
impl Ty { pub fn schema(kind: &str, table: Vec<Step>, key: Vec<KeyPart>) -> Ty }
```

## How it evaluates

*The arm.* `effective` meets `Ty::Schema`: evaluate each `KeyPart` in the
frame `(offset, limit)`; look up `memo.schemas[(kind, key)]`; on a miss, walk
`table` with `walk_next`/`Landing::Records` on a throwaway `Walk`, `spend`
once, call the builder, store `Arc<Built>` keyed by the kind, the key and the
table's stretch; continue the loop with `ty = built.ty.clone()`. Nested
objects are `Ty::Schema` nodes the builder emitted, so recursion happens per
node at evaluation, not at build.

*Memo and laziness.* `Resolved.ty` holds the built struct, as it holds a
switch's case; `Ty::Struct(Arc<StructDef>)` clones cheaply per element.
`forget_decoded` drops every schema whose table stretch is not space 0;
`forget_after(bit)` keeps one whose stretch ended before `bit`, as `tags`
does. `walk.rs` may drop description nodes it moved past; the cache survives
that and the builder re-resolves on a miss.

*Cycles.* A `building: FxHashSet<(kind, key)>` on the evaluator; re-entry on
the same key fails with "TBranch's description is being read with TBranch's
description". A class holding its own class (`TBranch.fBranches`) recurses at
evaluation only, bounded by `DEEPEST_PATH`, the 88-level expression depth
guard, and byte-count `Sized` windows that shrink. A kind whose descriptions
are typed by itself (ROOT's `TStreamerInfo`) is answered by the builder from a
bootstrap table without touching the file table.

*locate and gaps.* The built struct is an ordinary node; children placed from
it by an `Anchor::File` gather go into `placed` like any gathered child.

*Relations and origins.* `relations()` gets an arm: the declared `Ty::Schema`
writes its key (`fClassName.text, version` → `"TTree", 19` → `TTree v19`)
under `Role::Type`. `origin.rs::wrapper_origins` adds `Origin { role: Type,
path: built.from }` labelled by `walk_label`
(`streamer_info.object.members.elements[7]`), and a member whose parent was
schema-built adds one at `members_from[idx]`.

*Rendering.* `template_text` writes `schema(root, from
streamer_info.object…elements[], key fClassName.text, version)`;
`display_name` of the declared type is `schema`; the resolved node shows the
built struct's name. `decode::fixed_bits` → `None`; `placed::places` → `true`;
`diagram`, `graph`, `kinds`, `check.rs` walkers get a no-child arm.

*Editing.* Nothing new: members are editable by the existing rules.

## ROOT sketch

Records become stitched over their blocks (fixes the 16 MiB split too),
objects are typed by the file, and a `TreeRecord` gathers the baskets.

```rust
const NEW_CLASS: i128 = 0xFFFF_FFFF;
const STREAMER: &[Step] = &[Step::field("streamer_info"), Step::field("object"), Step::field("members"), Step::field("elements"), Step::each()];

/// A counted object: its byte count and version, then members typed by the file.
fn streamed(class: E) -> T {
    T::structure("Object", vec![
        ("count_raw", T::u32(Big)),
        ("byte_count", T::computed(E::field("count_raw").and(E::lit(0x3fff_ffff)))),
        ("version", T::u16(Big)),
        ("members", T::sized(E::field("byte_count").sub(E::lit(2)),
            T::schema("root", STREAMER.to_vec(), vec![KeyPart::Text(class), KeyPart::Int(E::field("version").and(E::lit(0x3fff)))]))),
    ]).machinery(&["count_raw"])
}

/// A pointer: null, a reference back, or a class (spelled out the first time,
/// then the place it was spelled at) and the object.
fn object_ref() -> T {
    let first = || E::field("first");
    let counted = || first().bit(30).both(first().not_equal(E::lit(NEW_CLASS)));
    let tag = || E::cond(counted(), E::field("tag"), first());
    T::structure_named("ObjectRef", "class_name", "object", vec![
        ("first", T::u32(Big)),
        ("tag", T::when(counted(), T::u32(Big))),
        ("class_name", T::when(tag().bit(31), T::switch(tag().equal_to(E::lit(NEW_CLASS)), vec![(1, T::cstr())],
            T::at_origin(tag().and(E::lit(0x7fff_ffff)).add(E::lit(2)).sub(E::field("fKeylen")), T::cstr())))),
        ("object", T::when(tag().bit(31), streamed(E::within(&["class_name"])))),
    ]).machinery(&["first", "tag"])
}

fn tree_record() -> T {   // chosen in `by_class` for "TTree"
    let to_baskets = vec![Step::field("object"), Step::field("members"), Step::field("fBranches"), Step::field("members"),
        Step::field("elements"), Step::each(), Step::field("object"), Step::field("members"), Step::field("baskets"), Step::each()];
    with_key("TreeRecord", "fName", "object", vec![("body", body()), ("object", /* stitched object */),
        ("baskets", T::gather(to_baskets, E::field("seek"), Anchor::File, E::lit(0),
            T::sized(E::placer(E::field("bytes")), T::Named("Basket".into()))).skipping_zero())])
}
```

The `root` builder: bootstrap classes (`TObject`, `TNamed`, `TString`,
`TArray*`, `TObjArray`, `TList`, `TStreamerInfo` and the element subclasses,
ported from `root_streamer::bootstrap` and `read_body`) come from Rust;
anything else from the landings, matching `fName` and `fClassVersion` with
`Schema::find`'s fallback. Elements: basic codes become fixed-width ints and
floats; +20 a fixed array; +40 `("marker", u8)` then an array counted by
`E::field(count_name)` (or `E::within([base, name])`, since bases nest as
sub-structs); 61/62/63/68 `streamed(E::TextLit(type_name))`; 64/69/70
`object_ref()`; 65 `tstring()`; unknown codes `bytes(Remaining)`. For
`TBranch` it appends a zero-size `baskets: array(inline_structure("BasketRef",
[seek, bytes, first_entry]), fWriteBasket)`. `Basket` is a key with
`TBasket`'s five extra numbers, `body()`, and the values as bytes until stage 4
types them by leaf.

## BP5: stays a side reader

The format table is in `mmd.0`, Qubero opens one file, and nothing can reach it
from `md.0`: a document boundary, not a type-system gap. The FFS builder is
still worth writing to prove `Ty::Schema` is not ROOT-shaped: every field an
`At` at its recorded offset inside the record's `Sized` region, nested
subformats built eagerly with a depth cap, run in `adios_real.rs` over an
evaluator on `mmd.0` and applied to `md.0`'s first record in a throwaway
template.

## Files touched

`template.rs`; `eval/mod.rs` (`effective`, `building`); `eval/memo.rs`
(`schemas`, both forgets); `eval/gather.rs` (`Step::Stream`, `Field` from a
stream); `eval/expr.rs` (`KeyPart` evaluation); `eval/relate.rs`;
`eval/origin.rs`; `template_text.rs`; `decode.rs`;
`eval/{placed,shape,kinds,graph,diagram,check}.rs` arms; `formats/root.rs`;
new `formats/root_schema.rs`; `formats/adios.rs` plus new
`formats/ffs_schema.rs`; `root_tree.rs` stays as the oracle.

## Build order, with tests

1. **IR and evaluator**, with a synthetic kind in `eval/tests.rs`:
   `a_type_is_built_from_the_records_the_file_describes_it_with`,
   `two_nodes_of_one_key_share_one_built_type`,
   `a_schema_that_describes_itself_is_refused_with_its_name`,
   `an_edit_to_a_description_rebuilds_what_it_typed`,
   `a_walk_steps_into_the_stream_under_a_field`. `template_text.rs`:
   `a_schema_reads_as_written`.
2. **ROOT objects and the bootstrap.** `tests/root_real.rs`:
   `the_streamer_info_record_reads_as_the_classes_the_side_reader_lists`
   (oracle `root_tree::contents().classes`),
   `a_second_object_of_a_class_reads_its_name_from_where_the_first_spelled_it`.
3. **File-typed objects and baskets.**
   `every_basket_the_side_reader_lists_is_placed_by_the_template`,
   `the_same_tree_places_the_same_baskets_through_lz4_lzma_and_zstd`,
   `a_baskets_type_names_the_streamer_element_it_came_from`,
   `zmumu_names_nine_tenths_of_its_bytes` (from about 2%).
4. **Basket values by leaf.** `a_baskets_values_match_the_side_reader`
   (oracle `root_tree::read_basket`).
5. **FFS.** `ffs_schema.rs`: `a_record_reads_by_the_format_its_id_names`;
   `adios_real.rs`: `the_first_bp5_step_reads_its_variables_with_mmd0s_format`.

## Risks

- Members window is `byte_count − 2`. The memberwise bit and old uncounted
  records stay bytes.
- `walk_step` for `Step::Field` from a `Stitched` landing goes to child 0;
  `Step::Stream` under `body` must stop at the block's stream, not descend
  `Traced` blocks.
- `no_ring`/`settled_name` cannot name a `Schema` before it is built, so the
  ring guard may not fire for an `At` whose inner is one.
- Sub-branches need a second gather level (Zmumu is flat); the hex view shows
  baskets only once the forward walk reaches the tree key, as for RNTuple.
- A NanoAOD's thousand branches resolve a thousand `ObjectRef`s per listing.
- The builder reads nodes, not bytes, so bootstrap layouts must match
  `read_body` exactly; the stage 2 oracle test catches drift.

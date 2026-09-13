//! RNTuple, the columnar format ROOT 6.34 made final, as it sits inside a ROOT
//! file.
//!
//! Unlike a `TTree`, an RNTuple does not need the class descriptions in
//! `StreamerInfo` to be read. Its layout is written down in ROOT's
//! `BinaryFormatSpecification.md`, and every structure below is from version
//! 1.0.0.0 of that document, with the two things 1.1.0.0 added. The only part
//! of it that is a ROOT object at all is the anchor, which [`super::anchor`]
//! reads; everything the anchor points at is little-endian and was never
//! streamed.
//!
//! What the anchor points at is envelopes. An envelope is a sixteen-bit type
//! and a forty-eight-bit length packed into one number, a payload, and an
//! xxhash-3 of the two. The header envelope describes the fields and the
//! columns under them, the footer lists the cluster groups, and each cluster
//! group links to a page list envelope, which says where every page of every
//! column of every cluster is.
//!
//! Inside an envelope, lists and records are written as frames. A frame opens
//! with a signed 64-bit size that counts the whole frame, negative for a list,
//! and a list then says how many items it holds. A reader is told to skip by
//! the size rather than by adding up what it read, so that a newer writer can
//! put more in a record than an older reader knows about. So every frame here
//! is a window of the size it declares, and whatever is left in it after the
//! fields this knows reads as a gap inside the frame rather than being taken for
//! the start of the next one.
//!
//! An envelope, like a page, may be compressed with the same nine-byte blocks a
//! ROOT record uses, and it is compressed exactly when the length it takes in
//! the file is short of the length it comes to. Nothing else says so.
//!
//! Every page is placed where its locator says, whatever space the locator was
//! read in, so a page list unpacked out of a compressed envelope still puts its
//! pages in the file. In `ntpl001_staff_rntuple_v1-0-1-0.root` the pages are
//! 23,623 of 25,318 bytes. A page that reads as a list of plain values does,
//! and a split, delta or bit-packed page keeps its bytes under a name saying how
//! they are laid out.
//!
//! ## What a page cannot find out
//!
//! What a page holds is set by its column's type, and the type is in the
//! header, which is a different envelope from the page list. Reaching a field
//! of another envelope takes a path down into it, and a path cannot go into a
//! compressed run. So when the header was compressed, which in a compressed
//! ntuple it almost always is, a page cannot learn its type: it is placed, its
//! block is opened when it opens like one, and what came out keeps its bytes.
//! The staff sample is that case, and every one of its pages opens and none is
//! read as values; an ntuple written uncompressed reads all the way down. See
//! [`header_stored`] for the IR change that would close it.
//!
//! Not read: a locator of any kind but the standard one (the large locator is
//! the only other kind defined, and places nothing here), an envelope or page
//! larger than one sixteen-mebibyte block, whose blocks keep their bytes, and a
//! payload ROOT split across several keys because it was larger than the
//! anchor's maximum key size. No checksum is checked, since the IR has no
//! xxhash-3.

use crate::template::{Endian::*, Expr as E, Template, Ty as T, Until};

/// Whether an envelope or a page holds more than one block. ROOT compresses in
/// blocks of at most sixteen mebibytes unpacked, so anything larger is several
/// blocks whose unpacked bytes only make sense joined up, and the template has
/// no way to join two decoded runs into one. So a block that is the whole of
/// its envelope is read as the envelope, and a block that is only part of one
/// keeps its bytes. `len` is what the whole envelope comes to.
fn whole_envelope(len: E) -> T {
    T::switch(
        E::field("uncompressed_size").equal_to(len),
        vec![(1, T::Named("RNTupleEnvelope".into()))],
        T::bytes(E::Remaining),
    )
}

/// `nbytes` bytes of envelope here, coming to `len` once unpacked.
pub(super) fn placed_envelope(nbytes: E, len: E) -> T {
    T::sized(
        nbytes.clone(),
        T::switch(
            nbytes.less_than(len.clone()),
            vec![(1, T::repeat(super::compressed(whole_envelope(len)), Until::End))],
            T::Named("RNTupleEnvelope".into()),
        ),
    )
}

/// A string: a 32-bit length and that many bytes of UTF-8.
fn string() -> T {
    T::structure_named("RNTupleString", "", "text", vec![("len", T::u32(Little)), ("text", T::utf8(E::field("len")))])
}

fn i64le() -> T {
    T::Int { bits: 64, endian: Little }
}

/// How many bytes a frame takes, from the size it opens with. A record frame
/// writes that size as it is and a list frame writes it negated, and the size
/// has to be known before the frame's own fields are read so that it can be
/// the window they are read in.
fn frame_len() -> E {
    let raw = E::peek(64, Little);
    E::cond(raw.clone().bit(63), E::lit(1i128 << 64).sub(raw.clone()), raw)
}

/// A record frame: its size, and `fields`. Anything of it past them is a gap
/// inside the frame, which in a file ROOT writes today is nothing, and in a
/// file a later version writes is the fields it added.
fn record_frame(name: &str, named_by: &str, fields: Vec<(&str, T)>) -> T {
    let mut all = vec![("size", i64le())];
    all.extend(fields);
    T::sized(frame_len(), T::structure_named(name, named_by, "", all).machinery(&["size"]))
}

/// A list frame: its size, how many items it holds, and the items.
fn list_frame(item: T) -> T {
    T::sized(
        frame_len(),
        T::structure_named(
            "ListFrame",
            "",
            "items",
            vec![("size", i64le()), ("count", T::u32(Little)), ("items", T::array(item, E::field("count")))],
        )
        .machinery(&["size"]),
    )
}

/// The feature flags a header or a footer opens with. Every bit is a feature a
/// reader has to support to read the file at all, and the top bit says another
/// word of them follows, which no writer has needed yet.
fn feature_flags() -> Vec<(&'static str, T)> {
    let word = || T::flags("RNTupleFeatures", T::u64(Little), &[(0, "nested deferred columns"), (63, "more follow")]);
    vec![
        ("feature_flags", word()),
        (
            "more_feature_flags",
            T::when(
                E::field("feature_flags").bit(63),
                T::repeat(
                    T::structure("RNTupleFeatureWord", vec![("flags", word())]),
                    Until::Cond(E::field("flags").bit(63).equal_to(E::lit(0))),
                ),
            ),
        ),
    ]
}

const ENVELOPE_TYPE: &[(i128, &str)] = &[(1, "header"), (2, "footer"), (3, "page list")];

/// One envelope, unpacked: what kind it is, how long it is, what it holds and
/// the checksum over all of that.
///
/// The length counts the eight bytes it is packed into and the eight of the
/// checksum, so the payload is sixteen bytes shorter than it says.
fn envelope() -> T {
    T::structure(
        "RNTupleEnvelope",
        vec![
            ("type_id", T::enumeration("RNTupleEnvelopeType", T::u16(Little), ENVELOPE_TYPE)),
            ("length", T::UInt { bits: 48, endian: Little }),
            (
                "payload",
                T::sized(
                    E::field("length").sub(E::lit(16)),
                    T::switch(
                        E::field("type_id"),
                        vec![
                            (1, T::Named("RNTupleHeader".into())),
                            (2, T::Named("RNTupleFooter".into())),
                            (3, T::Named("RNTuplePageList".into())),
                        ],
                        T::bytes(E::Remaining),
                    ),
                ),
            ),
            // xxhash-3 of everything above it, which nothing here can check.
            ("checksum", T::u64(Little)),
        ],
    )
}

/// The header: what the ntuple is called, what wrote it, and the schema.
fn header() -> T {
    let mut fields = feature_flags();
    fields.extend([
        ("name", string()),
        ("description", string()),
        ("writer", string()),
    ]);
    fields.extend(schema(true));
    T::structure("RNTupleHeader", fields)
}

/// The four lists a schema is made of, which the header holds and the footer's
/// schema extension holds again for whatever was added while writing.
///
/// A column names its field by number, and in the header that number is where
/// the field is in the list beside it, so a column can be labelled with its
/// field's name. In an extension it is not: the numbers carry on from the
/// header's, and a column added late may belong to a field of the header, so
/// `named` is only set where the number can be looked up in the list beside it.
fn schema(named: bool) -> Vec<(&'static str, T)> {
    vec![
        ("fields", list_frame(T::Named("RNTupleField".into()))),
        ("columns", list_frame(column(named))),
        ("alias_columns", list_frame(alias_column())),
        ("extra_type_info", list_frame(type_info())),
    ]
}

const FIELD_ROLE: &[(i128, &str)] = &[(0, "plain"), (1, "collection"), (2, "record"), (3, "variant"), (4, "streamer")];

/// One field of the schema. Its ID is where it is in the list, and a top-level
/// field names itself as its own parent.
fn field() -> T {
    let flag = |bit| E::field("flags").bit(bit);
    record_frame(
        "RNTupleField",
        "name",
        vec![
            ("field_version", T::u32(Little)),
            ("type_version", T::u32(Little)),
            ("parent_field_id", T::u32(Little)),
            ("structural_role", T::enumeration("RNTupleFieldRole", T::u16(Little), FIELD_ROLE)),
            (
                "flags",
                T::flags(
                    "RNTupleFieldFlags",
                    T::u16(Little),
                    &[(0, "repetitive"), (1, "projected"), (2, "type checksum"), (3, "SoA")],
                ),
            ),
            ("name", string()),
            ("type_name", string()),
            ("type_alias", string()),
            ("description", string()),
            // How many copies of the one field under it make a fixed-size
            // array.
            ("array_size", T::when(flag(0), T::u64(Little))),
            // The field a projected field is a view of.
            ("source_field_id", T::when(flag(1), T::u32(Little))),
            ("type_checksum", T::when(flag(2), T::u32(Little))),
        ],
    )
}

/// Column types by the number the format gives them. The `Split` ones have
/// their bytes rearranged, all the first bytes of the elements and then all the
/// second, and the signed ones among them zigzag each value first and the index
/// ones store differences; see [`values`] for which of these read as values.
pub(super) const COLUMN_TYPE: &[(i128, &str)] = &[
    (0x00, "Bit"),
    (0x01, "Byte"),
    (0x02, "Char"),
    (0x03, "Int8"),
    (0x04, "UInt8"),
    (0x05, "Int16"),
    (0x06, "UInt16"),
    (0x07, "Int32"),
    (0x08, "UInt32"),
    (0x09, "Int64"),
    (0x0a, "UInt64"),
    (0x0b, "Real16"),
    (0x0c, "Real32"),
    (0x0d, "Real64"),
    (0x0e, "Index32"),
    (0x0f, "Index64"),
    (0x10, "Switch"),
    (0x11, "SplitInt16"),
    (0x12, "SplitUInt16"),
    (0x13, "SplitInt32"),
    (0x14, "SplitUInt32"),
    (0x15, "SplitInt64"),
    (0x16, "SplitUInt64"),
    (0x17, "SplitReal16"),
    (0x18, "SplitReal32"),
    (0x19, "SplitReal64"),
    (0x1a, "SplitIndex32"),
    (0x1b, "SplitIndex64"),
    (0x1c, "Real32Trunc"),
    (0x1d, "Real32Quant"),
];

/// One column: how its elements are written, and which field they belong to.
/// Its ID is where it is in the list, and a field with several columns lists
/// them in order: a `std::string` is an index column and then a `Char` one.
///
/// A deferred column was added after some entries had already been written,
/// and says which element it starts at; a column with a range says the least
/// and greatest value it can hold, which is what a quantised real is packed
/// against.
fn column(named: bool) -> T {
    let flag = |bit| E::field("flags").bit(bit);
    let mut fields = vec![
        ("type", T::enumeration_hex("RNTupleColumnType", T::u16(Little), COLUMN_TYPE)),
        ("bits_on_storage", T::u16(Little)),
        ("field_id", T::u32(Little)),
        ("flags", T::flags("RNTupleColumnFlags", T::u16(Little), &[(0, "deferred"), (1, "value range")])),
        ("representation_index", T::u16(Little)),
        ("first_element_index", T::when(flag(0), i64le())),
        ("min_value", T::when(flag(1), T::F64(Little))),
        ("max_value", T::when(flag(1), T::F64(Little))),
    ];
    if named {
        // The name of the field this column belongs to, read from the list of
        // fields before it: a column is a number in the file and a name is
        // what says which one it is.
        fields.push((
            "field",
            T::computed_text(E::elem_within(&["fields", "items"], E::field("field_id"), &["name", "text"])),
        ));
    }
    record_frame("RNTupleColumn", if named { "field" } else { "" }, fields)
}

/// A column with no pages of its own, which reads another column's elements as
/// a projected field.
fn alias_column() -> T {
    record_frame(
        "RNTupleAliasColumn",
        "",
        vec![("physical_column_id", T::u32(Little)), ("field_id", T::u32(Little))],
    )
}

/// Extra information about a type. The one kind there is so far is the streamer
/// information for the fields ROOT streamed as whole objects, written as a
/// streamed `TList` in what is left of the frame.
fn type_info() -> T {
    record_frame(
        "RNTupleTypeInfo",
        "type_name",
        vec![
            ("content_id", T::enumeration("RNTupleTypeInfoKind", T::u32(Little), &[(0, "streamer info")])),
            ("type_version", T::u32(Little)),
            ("type_name", string()),
            ("content", T::bytes(E::Remaining)),
        ],
    )
}

/// The footer: which header it belongs to, what was added to the schema while
/// writing, and where the clusters are.
///
/// Version 1.1.0.0 of the format added a list of linked attribute sets after
/// the cluster groups, and a footer written before that ends where the cluster
/// groups do, which is how the two are told apart.
fn footer() -> T {
    let mut fields = feature_flags();
    fields.extend([
        // The xxhash-3 of the header envelope, so that a footer can be matched
        // to the header it was written with.
        ("header_checksum", T::u64(Little)),
        ("schema_extension", schema_extension()),
        ("cluster_groups", list_frame(cluster_group())),
        ("attribute_sets", T::when(E::lit(0).less_than(E::Remaining), list_frame(attribute_set()))),
    ]);
    T::structure("RNTupleFooter", fields)
}

/// Fields and columns added after the header was written. Often empty, and
/// then the frame is its eight bytes of size and nothing else.
fn schema_extension() -> T {
    let present = |ty| T::when(E::lit(0).less_than(E::Remaining), ty);
    record_frame(
        "RNTupleSchemaExtension",
        "",
        schema(false).into_iter().map(|(name, ty)| (name, present(ty))).collect(),
    )
}

/// A locator: where something is, in the form every file ROOT writes today uses,
/// a size and an offset into the file.
///
/// A negative size says the locator is some other kind, and then the size is
/// packed: its low sixteen bits are how long the locator itself is and its top
/// byte, negated, which kind. The one other kind defined is a large locator,
/// sixty-four bits of size and then the offset, which is read here as its
/// bytes; nothing behind a locator of any kind but the standard one is followed.
fn locator() -> T {
    let packed = E::lit(0).sub(E::field("size"));
    let other = T::structure(
        "RNTupleOtherLocator",
        vec![
            ("size", T::i32(Little)),
            ("kind", T::enumeration("RNTupleLocatorKind", T::computed(packed.clone().shr(E::lit(24))), &[(1, "large")])),
            ("payload", T::bytes(packed.and(E::lit(0xffff)).sub(E::lit(4)).at_least(E::lit(0)))),
        ],
    );
    let standard = T::structure("RNTupleLocator", vec![("size", T::i32(Little)), ("offset", T::u64(Little))]);
    T::switch(E::peek(32, Little).bit(31), vec![(1, other)], standard)
}

/// Whether the locator in the field `name` is the standard kind, which is the
/// only kind anything is placed from.
fn standard(name: &str) -> E {
    E::lit(-1).less_than(E::within(&[name, "size"]))
}

/// `inner`, at the offset the standard locator in the field `name` gives, and
/// nothing when the locator is of another kind. The offset counts from the
/// start of the file wherever the locator was read, which for a locator inside
/// a compressed envelope is somewhere that is not the file at all.
fn located(name: &str, inner: T) -> T {
    T::when(standard(name), T::at(E::within(&[name, "offset"]), inner))
}

/// One cluster group: the entries it covers, how many clusters it has, and a
/// link to the page list envelope that says where their pages are. A link is
/// the length the envelope comes to unpacked and a locator for where it is.
fn cluster_group() -> T {
    record_frame(
        "RNTupleClusterGroup",
        "",
        vec![
            ("min_entry", T::u64(Little)),
            ("entry_span", T::u64(Little)),
            ("cluster_count", T::u32(Little)),
            ("page_list_length", T::u64(Little)),
            ("page_list_locator", locator()),
            (
                "page_list",
                located(
                    "page_list_locator",
                    placed_envelope(E::within(&["page_list_locator", "size"]), E::field("page_list_length")),
                ),
            ),
        ],
    )
}

/// A page list: for each cluster in the group, which entries it holds, and
/// then for each cluster again, for each column, where that column's pages are.
///
/// The two lists are kept apart rather than one record per cluster, and the
/// second is a list of lists of lists: clusters, columns in each cluster in the
/// order the header numbers them, pages in each column.
fn page_list() -> T {
    T::structure(
        "RNTuplePageList",
        vec![
            ("header_checksum", T::u64(Little)),
            ("cluster_summaries", list_frame(cluster_summary())),
            ("clusters", list_frame(list_frame(T::Named("RNTupleColumnPages".into())))),
        ],
    )
}

/// Which entries a cluster holds. The count and the flags share one 64-bit
/// number, the count in the low fifty-six bits; the one flag defined is for
/// sharded clusters, which the format has reserved and nothing yet writes.
fn cluster_summary() -> T {
    record_frame(
        "RNTupleClusterSummary",
        "",
        vec![
            ("first_entry", T::u64(Little)),
            ("entry_count", T::UInt { bits: 56, endian: Little }),
            ("flags", T::flags("RNTupleClusterFlags", T::u8(), &[(0, "sharded")])),
        ],
    )
}

/// One column's pages in one cluster: a list frame whose items are the pages,
/// with two more numbers inside the frame after them.
///
/// The element offset is how many elements of the column come before this
/// cluster, so an entry can be found without reading every cluster before it.
/// A negative one says the column is suppressed here, because another
/// representation of the same field is the one this cluster wrote, and then
/// there are no pages and no compression settings. The settings are the
/// algorithm times a hundred and the level, the same packing a ROOT file's
/// header uses: 505 is zstd at level 5.
///
/// Nothing in the page list says what the column's elements are; the header
/// does, and the column is known only by where it is in this list. So the three
/// fields of no bits before the pages read that from the header: the column's
/// number, its type, and the name of the field it belongs to. They are there
/// only when the header can be read from here at all, which is when it was
/// stored as it stands. See [`typed`] for why a compressed header cannot be.
fn column_pages() -> T {
    let header_column = |field: &str| E::elem_within(&["header", "payload", "columns", "items"], E::field("column_id"), &[field]);
    T::sized(
        frame_len(),
        T::structure_named(
            "RNTupleColumnPages",
            "field",
            "",
            vec![
                ("size", i64le()),
                ("page_count", T::u32(Little)),
                ("column_id", T::computed(E::idx())),
                (
                    "column_type",
                    T::when(typed(), T::enumeration_hex("RNTupleColumnType", T::computed(column_field("type")), COLUMN_TYPE)),
                ),
                ("field", T::when(in_header(), T::computed_text(header_column("field")))),
                ("pages", T::array(page_entry(), E::field("page_count"))),
                ("element_offset", i64le()),
                ("compression_settings", T::when(E::lit(-1).less_than(E::field("element_offset")), T::u32(Little))),
            ],
        )
        .machinery(&["size", "column_id"]),
    )
}

/// Whether the anchor's header envelope is in the file as it stands, and so
/// can be read from somewhere else in the ntuple.
///
/// A page needs its column's type to be read, and the type is in the header,
/// in another envelope. An expression reaches a field of another envelope by a
/// path down into it, and a path goes through structures, lists and `At`s but
/// not into a compressed run: what came out of a `Decoded` is not a field of
/// it that a name can reach. So when the header was compressed, which in a
/// compressed ntuple it nearly always is, a page cannot find out what it holds
/// and reads as its bytes, opened when they are a compressed block. What would
/// change that is a path stepping into a decoded run the way it steps through
/// an `At`, and a way to name the run whatever codec wrote it.
fn header_stored() -> E {
    E::lit(0).less_than(E::field("seek_header")).both(E::field("nbytes_header").equal_to(E::field("len_header")))
}

/// How many columns the header describes, and how many the footer's schema
/// extension adds after them. The extension's numbers carry on from the
/// header's, so a column past the header's count is in the extension.
fn header_columns() -> E {
    E::within(&["header", "payload", "columns", "count"])
}

fn extension_columns() -> E {
    E::cond(
        E::within(&["schema_extension", "size"]).greater_than(E::lit(8)),
        E::within(&["schema_extension", "columns", "count"]),
        E::lit(0),
    )
}

/// Whether this column's description can be read from here: the header is
/// stored, and the column is one it or the extension describes.
fn typed() -> E {
    header_stored().both(E::field("column_id").less_than(header_columns().add(extension_columns())))
}

/// The same, for a column the header itself describes, which is the only kind
/// whose field can be named by the number it gives. See [`schema`].
fn in_header() -> E {
    header_stored().both(E::field("column_id").less_than(header_columns()))
}

/// The field `name` of this column's description, from the header or from the
/// footer's schema extension. Only to be asked where [`typed`] holds.
fn column_field(name: &str) -> E {
    E::cond(
        E::field("column_id").less_than(header_columns()),
        E::elem_within(&["header", "payload", "columns", "items"], E::field("column_id"), &[name]),
        E::elem_within(&["schema_extension", "columns", "items"], E::field("column_id").sub(header_columns()), &[name]),
    )
}

/// One page in a column's list: how many elements it holds, and where it is.
///
/// The count's sign is a flag. Negative says eight bytes of xxhash-3 follow
/// the page in the file, which the locator's size does not count.
fn page_entry() -> T {
    let count = E::field("element_count");
    T::structure(
        "RNTuplePageEntry",
        vec![
            ("element_count", T::i32(Little)),
            ("elements", T::computed(E::cond(count.clone().less_than(E::lit(0)), E::lit(0).sub(count.clone()), count))),
            ("locator", locator()),
            ("page", located("locator", T::Named("RNTuplePage".into()))),
        ],
    )
    .counted_as("page")
}

/// A page, where its locator puts it: the bytes the locator counts, and the
/// checksum after them when the page list said there is one.
///
/// A page says nowhere whether it was compressed. What says so is its size
/// against the size its elements come to, which is the element count times the
/// column's bits on storage, and where the column's type cannot be read (see
/// [`typed`]) the page's own first bytes are asked instead: a ROOT block opens
/// with one of the algorithm's two letters and a compressed size that fits in
/// the page. A page written as it stands could open that way by chance, and
/// then it reads as a block that does not open.
fn page() -> T {
    let unpacked = E::field("elements").mul(column_field("bits_on_storage")).add(E::lit(7)).div(E::lit(8));
    let packed = E::cond(typed(), E::Remaining.less_than(unpacked), looks_like_block());
    T::structure(
        "RNTuplePage",
        vec![
            (
                "data",
                T::sized(
                    E::within(&["locator", "size"]),
                    T::switch(
                        packed,
                        vec![(1, T::repeat(super::compressed(T::Named("RNTupleValues".into())), Until::End))],
                        T::Named("RNTupleValues".into()),
                    ),
                ),
            ),
            ("checksum", T::when(E::field("element_count").less_than(E::lit(0)), T::u64(Little))),
        ],
    )
}

/// Whether the bytes here open the way a ROOT compressed block does: one of the
/// algorithms' two letters, and a compressed size that the nine-byte header and
/// it leave room for.
fn looks_like_block() -> E {
    let tag = E::peek(16, Big);
    let named = super::ALGORITHM.iter().fold(E::lit(0), |any, (code, _)| any.either(tag.clone().equal_to(E::lit(*code))));
    E::lit(9)
        .less_or_equal(E::Remaining)
        .both(named)
        .both(E::peek_at(E::lit(24), 24, Little).add(E::lit(9)).less_or_equal(E::Remaining))
}

/// What a page's bytes hold, once they are out of any block, by the type of
/// its column.
///
/// The fixed-width types are their values one after another, little-endian,
/// and read as them. `Index32` and `Index64` are too: each is where the next
/// entry's elements start in the column under it, counted from the start of
/// the cluster. The rest keep their bytes, named for how they are laid out,
/// because reading them needs arithmetic over the whole page that no field
/// does: a `Split` column has all the first bytes of its elements and then all
/// the second, a signed one zigzags each value before that, a split index
/// column stores each value less the one before, and `Bit`, `Real32Trunc` and
/// `Real32Quant` pack each element into fewer bits than a byte-wide field.
///
/// How many values fit is worked out from the room there is rather than from
/// the page's element count. A page of more than sixteen mebibytes is several
/// blocks, and each is read on its own.
fn values() -> T {
    let each = |ty: T, bytes: i128| T::array(ty, E::Remaining.div(E::lit(bytes)));
    let laid = |name: &str| T::structure_named(name, "", "bytes", vec![("bytes", T::bytes(E::Remaining))]);
    let mut cases = vec![
        (0x00, laid("RNTupleBitPacked")),
        (0x01, T::bytes(E::Remaining)),
        (0x02, T::utf8(E::Remaining)),
        (0x03, each(T::Int { bits: 8, endian: Little }, 1)),
        (0x04, each(T::u8(), 1)),
        (0x05, each(T::Int { bits: 16, endian: Little }, 2)),
        (0x06, each(T::u16(Little), 2)),
        (0x07, each(T::i32(Little), 4)),
        (0x08, each(T::u32(Little), 4)),
        (0x09, each(i64le(), 8)),
        (0x0a, each(T::u64(Little), 8)),
        (0x0b, each(T::F16(Little), 2)),
        (0x0c, each(T::F32(Little), 4)),
        (0x0d, each(T::F64(Little), 8)),
        (0x0e, each(T::u32(Little), 4)),
        (0x0f, each(T::u64(Little), 8)),
        // Where the entry's value is in the column of the variant member it
        // holds, and which member that is.
        (0x10, each(T::structure("RNTupleSwitch", vec![("index", T::u64(Little)), ("tag", T::u32(Little))]), 12)),
        (0x1a, laid("RNTupleDeltaSplit")),
        (0x1b, laid("RNTupleDeltaSplit")),
        (0x1c, laid("RNTupleBitPacked")),
        (0x1d, laid("RNTupleBitPacked")),
    ];
    // Split, and zigzagged first where the type is signed.
    cases.extend([0x12, 0x14, 0x16, 0x17, 0x18, 0x19].map(|c| (c, laid("RNTupleSplit"))));
    cases.extend([0x11, 0x13, 0x15].map(|c| (c, laid("RNTupleZigzagSplit"))));
    T::switch(E::cond(typed(), E::field("column_type"), E::lit(-1)), cases, T::bytes(E::Remaining))
}

/// A linked attribute set: another RNTuple, in the same file, holding metadata
/// about ranges of this one's entries. The locator is where that one's anchor
/// is.
fn attribute_set() -> T {
    record_frame(
        "RNTupleAttributeSet",
        "name",
        vec![
            ("schema_version_major", T::u16(Little)),
            ("schema_version_minor", T::u16(Little)),
            ("anchor_length", T::u32(Little)),
            ("anchor_locator", locator()),
            ("name", string()),
        ],
    )
}

/// The types an RNTuple is read with, added to the ROOT template's table.
pub(super) fn with_types(t: Template) -> Template {
    t.with_type("RNTupleEnvelope", envelope())
        .with_type("RNTupleHeader", header())
        .with_type("RNTupleFooter", footer())
        .with_type("RNTuplePageList", page_list())
        .with_type("RNTupleField", field())
        .with_type("RNTupleColumnPages", column_pages())
        .with_type("RNTuplePage", page())
        .with_type("RNTupleValues", values())
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::root;
    use super::super::tests::{anchor_path, down, rntuple_file};
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// An envelope of kind `kind` around `payload`: the packed type and length,
    /// the payload, and eight bytes standing for the checksum.
    fn envelope(kind: u16, payload: &[u8]) -> Vec<u8> {
        let len = payload.len() as u64 + 16;
        let mut b = ((len << 16) | u64::from(kind)).to_le_bytes().to_vec();
        b.extend_from_slice(payload);
        b.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        b
    }

    fn string(s: &str) -> Vec<u8> {
        let mut b = (s.len() as u32).to_le_bytes().to_vec();
        b.extend_from_slice(s.as_bytes());
        b
    }

    /// A record frame around `fields`, and `spare` bytes after them that a
    /// later version of the format might have put there.
    fn record(fields: &[u8], spare: usize) -> Vec<u8> {
        let mut b = ((8 + fields.len() + spare) as i64).to_le_bytes().to_vec();
        b.extend_from_slice(fields);
        b.extend(std::iter::repeat_n(0xab, spare));
        b
    }

    fn list(items: &[Vec<u8>]) -> Vec<u8> {
        let body: usize = items.iter().map(Vec::len).sum();
        let mut b = (-((12 + body) as i64)).to_le_bytes().to_vec();
        b.extend_from_slice(&(items.len() as u32).to_le_bytes());
        for item in items {
            b.extend_from_slice(item);
        }
        b
    }

    fn field(name: &str, type_name: &str, parent: u32, flags: u16, optional: &[u8], spare: usize) -> Vec<u8> {
        let mut f = Vec::new();
        for v in [0u32, 0, parent] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&flags.to_le_bytes());
        for s in [name, type_name, "", ""] {
            f.extend(string(s));
        }
        f.extend_from_slice(optional);
        record(&f, spare)
    }

    fn column(ty: u16, bits: u16, field: u32) -> Vec<u8> {
        let mut c = ty.to_le_bytes().to_vec();
        c.extend_from_slice(&bits.to_le_bytes());
        c.extend_from_slice(&field.to_le_bytes());
        c.extend_from_slice(&[0; 4]);
        record(&c, 0)
    }

    /// A header with five fields and five columns: a double; a string, which
    /// is an index column and a character column; an array of three floats,
    /// whose one column belongs to the field under it; and an integer stored
    /// split and zigzagged. The string's field record has four bytes more in
    /// it than this version knows about.
    fn header() -> Vec<u8> {
        let mut p = 0u64.to_le_bytes().to_vec();
        for s in ["events", "a test", "hand"] {
            p.extend(string(s));
        }
        p.extend(list(&[
            field("x", "double", 0, 0, &[], 0),
            field("name", "std::string", 1, 0, &[], 4),
            field("hits", "std::array<float,3>", 2, 1, &3u64.to_le_bytes(), 0),
            field("_0", "float", 2, 0, &[], 0),
            field("n", "std::int32_t", 4, 0, &[], 0),
        ]));
        p.extend(list(&[
            column(0x0d, 64, 0),
            column(0x0f, 64, 1),
            column(0x02, 8, 1),
            column(0x0c, 32, 3),
            column(0x13, 32, 4),
        ]));
        p.extend(list(&[]));
        p.extend(list(&[]));
        envelope(1, &p)
    }

    /// One page to write: its bytes as they unpack, how many elements they
    /// are, whether a checksum follows it, and whether it is written as a
    /// compressed block.
    pub(super) struct Page {
        pub data: Vec<u8>,
        pub elements: i32,
        pub checksum: bool,
        pub packed: bool,
    }

    /// A whole file: the pages of one cluster, a page list saying where they
    /// are, the header, and a footer with one cluster group linking the page
    /// list. `columns` holds each column's pages in the header's order, and
    /// `None` for a column this cluster suppresses. Answers with the file and
    /// where each page landed.
    fn ntuple(columns: &[Option<Vec<Page>>], header_packed: bool) -> (Vec<u8>, Vec<Vec<(u64, usize)>>) {
        let mut placed = Vec::new();
        let bytes = rntuple_file(|at| {
            let mut b = Vec::new();
            let mut entries = Vec::new();
            for pages in columns {
                let mut items = Vec::new();
                let mut here = Vec::new();
                for page in pages.iter().flatten() {
                    let stored = if page.packed { zl_block(&page.data) } else { page.data.clone() };
                    let offset = at + b.len() as u64;
                    here.push((offset, stored.len()));
                    let count = if page.checksum { -page.elements } else { page.elements };
                    let mut item = count.to_le_bytes().to_vec();
                    item.extend_from_slice(&(stored.len() as i32).to_le_bytes());
                    item.extend_from_slice(&offset.to_le_bytes());
                    items.push(item);
                    b.extend(stored);
                    if page.checksum {
                        b.extend_from_slice(&0xc0ff_ee00_c0ff_ee00u64.to_le_bytes());
                    }
                }
                // A column's page list is a list frame with its element offset
                // and compression settings inside it, after the pages.
                let mut trailer = match pages {
                    Some(_) => 0i64.to_le_bytes().to_vec(),
                    None => i64::MIN.to_le_bytes().to_vec(),
                };
                if pages.is_some() {
                    trailer.extend_from_slice(&101u32.to_le_bytes());
                }
                let mut frame = list(&items);
                let size = -(frame.len() as i64 + trailer.len() as i64);
                frame[..8].copy_from_slice(&size.to_le_bytes());
                frame.extend(trailer);
                entries.push(frame);
                placed.push(here);
            }
            let mut pl = 0x1122_3344_5566_7788u64.to_le_bytes().to_vec();
            pl.extend(list(&[record(&[0u64.to_le_bytes(), 5u64.to_le_bytes()].concat(), 0)]));
            pl.extend(list(&[list(&entries)]));
            let page_list = envelope(3, &pl);
            let page_list_at = at + b.len() as u64;
            let page_list_len = page_list.len();
            b.extend(page_list);

            let h = header();
            let (h_stored, h_len) = match header_packed {
                true => (zl_block(&h), h.len()),
                false => (h.clone(), h.len()),
            };
            let header_at = at + b.len() as u64;
            let h_n = h_stored.len();
            b.extend(h_stored);

            let mut group = [0u64.to_le_bytes(), 5u64.to_le_bytes()].concat();
            group.extend_from_slice(&1u32.to_le_bytes());
            group.extend_from_slice(&(page_list_len as u64).to_le_bytes());
            group.extend_from_slice(&(page_list_len as i32).to_le_bytes());
            group.extend_from_slice(&page_list_at.to_le_bytes());
            let mut fp = 0u64.to_le_bytes().to_vec();
            fp.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
            fp.extend(record(&[list(&[]), list(&[]), list(&[]), list(&[])].concat(), 0));
            fp.extend(list(&[record(&group, 0)]));
            let f = envelope(2, &fp);
            let footer_at = at + b.len() as u64;
            let f_n = f.len() as u64;
            b.extend(f);
            (b, [header_at, h_n as u64, h_len as u64, footer_at, f_n, f_n])
        });
        (bytes, placed)
    }

    fn doubles(v: &[f64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn page(data: Vec<u8>, elements: i32, checksum: bool) -> Page {
        Page { data, elements, checksum, packed: false }
    }

    /// Five columns' pages in the clear: two pages of doubles, the first with
    /// a checksum after it; a string's offsets and characters; three floats;
    /// and five integers split and zigzagged, 1, -1, 2, -2, 3.
    fn plain_pages() -> Vec<Option<Vec<Page>>> {
        let split: Vec<u8> = {
            let zz = [2u32, 1, 4, 3, 6];
            (0..4).flat_map(|b| zz.iter().map(move |v| (v >> (8 * b)) as u8).collect::<Vec<_>>()).collect()
        };
        vec![
            Some(vec![page(doubles(&[1.5, -2.0, 3.25]), 3, true), page(doubles(&[4.0, 5.5]), 2, false)]),
            Some(vec![page([3u64, 3, 8, 8, 8].iter().flat_map(|x| x.to_le_bytes()).collect(), 5, false)]),
            Some(vec![page(b"abcdefgh".to_vec(), 8, false)]),
            Some(vec![page([0.5f32, 1.0, 2.0].iter().flat_map(|x| x.to_le_bytes()).collect(), 3, true)]),
            Some(vec![page(split, 5, false)]),
        ]
    }

    /// From the anchor to the one cluster group, to its page list's payload,
    /// and to the pages of column `column` in the one cluster.
    fn group() -> Vec<usize> {
        down(&anchor_path(), &[16, 0, 2, 4, 2, 0])
    }

    fn column_pages(column: usize) -> Vec<usize> {
        down(&group(), &[6, 0, 2, 2, 2, 0, 2, column])
    }

    /// The page `k` of a column, where its locator put it.
    fn placed_page(column: usize, k: usize) -> Vec<usize> {
        down(&column_pages(column), &[5, k, 3, 0])
    }

    /// A footer with nothing added to the schema and no cluster groups.
    pub(in crate::formats::root) fn empty_footer() -> Vec<u8> {
        let mut p = 0u64.to_le_bytes().to_vec();
        p.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        p.extend(record(&[list(&[]), list(&[]), list(&[]), list(&[])].concat(), 0));
        p.extend(list(&[]));
        envelope(2, &p)
    }

    /// `data` as one ROOT `ZL` block.
    fn zl_block(data: &[u8]) -> Vec<u8> {
        let stream = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        assert!(stream.len() + 9 < data.len(), "a block that did not come out shorter would have been stored");
        let mut b = b"ZL\x08".to_vec();
        b.extend_from_slice(&(stream.len() as u32).to_le_bytes()[..3]);
        b.extend_from_slice(&(data.len() as u32).to_le_bytes()[..3]);
        b.extend(stream);
        b
    }

    /// A file whose header and footer are stored as they stand.
    fn stored() -> Vec<u8> {
        rntuple_file(|at| {
            let (h, f) = (header(), empty_footer());
            let (hn, fn_) = (h.len() as u64, f.len() as u64);
            ([h, f].concat(), [at, hn, hn, at + hn, fn_, fn_])
        })
    }

    /// The header envelope's payload.
    fn payload() -> Vec<usize> {
        down(&anchor_path(), &[15, 0, 2])
    }

    #[test]
    fn a_header_says_what_wrote_it_and_names_each_field_by_its_own_name() {
        let d = Document::new(MemSource(stored()));
        let mut ev = Evaluator::new(root());
        let p = payload();
        assert_eq!(ev.node(&d, &down(&anchor_path(), &[15, 0, 0])).unwrap().value.as_int(), Some(1));
        assert_eq!(ev.node(&d, &down(&p, &[2, 1])).unwrap().value, Value::Str("events".into()));
        assert_eq!(ev.node(&d, &down(&p, &[4, 1])).unwrap().value, Value::Str("hand".into()));
        // The fields list: its size, its count, and the fields, each labelled
        // with the name written inside it.
        let fields = down(&p, &[5, 2]);
        assert_eq!(ev.node(&d, &fields).unwrap().child_count, 5);
        assert_eq!(ev.node(&d, &down(&fields, &[0])).unwrap().name, "[0] x");
        assert_eq!(ev.node(&d, &down(&fields, &[1])).unwrap().name, "[1] name");
        // A repetitive field says how many copies make its array, and a field
        // that is not one has no such number at all.
        let hits = down(&fields, &[2]);
        assert_eq!(ev.node(&d, &down(&hits, &[5])).unwrap().value.as_int(), Some(1));
        assert_eq!(ev.node(&d, &down(&hits, &[10])).unwrap().value, Value::UInt(3));
        assert!(ev.node(&d, &down(&fields, &[0, 10])).unwrap().absent);
    }

    #[test]
    fn a_frame_is_as_long_as_its_size_says_and_what_is_past_its_fields_is_left() {
        let d = Document::new(MemSource(stored()));
        let mut ev = Evaluator::new(root());
        let fields = down(&payload(), &[5, 2]);
        // The string's record is four bytes longer than its fields, and the
        // field after it still starts where the size says, not where the
        // fields ran out.
        let name = ev.node(&d, &down(&fields, &[1])).unwrap();
        let hits = ev.node(&d, &down(&fields, &[2])).unwrap();
        assert_eq!(name.offset_bits + name.size_bits, hits.offset_bits);
        let last = ev.node(&d, &down(&fields, &[1, 9])).unwrap();
        assert_eq!(name.offset_bits + name.size_bits - (last.offset_bits + last.size_bits), 4 * 8);
        assert_eq!(ev.node(&d, &down(&fields, &[2, 6, 1])).unwrap().value, Value::Str("hits".into()));
    }

    #[test]
    fn a_column_is_labelled_with_the_field_it_belongs_to() {
        let d = Document::new(MemSource(stored()));
        let mut ev = Evaluator::new(root());
        let columns = down(&payload(), &[6, 2]);
        assert_eq!(ev.node(&d, &columns).unwrap().child_count, 5);
        assert_eq!(
            ev.node(&d, &down(&columns, &[0, 1])).unwrap().value,
            Value::Enum { raw: 0x0d, name: Some("Real64".into()), hex: true }
        );
        // Both of a string's columns belong to it, and the array's element
        // column belongs to the field under the array.
        assert_eq!(ev.node(&d, &down(&columns, &[1])).unwrap().name, "[1] name");
        assert_eq!(ev.node(&d, &down(&columns, &[2])).unwrap().name, "[2] name");
        assert_eq!(ev.node(&d, &down(&columns, &[3])).unwrap().name, "[3] _0");
    }

    #[test]
    fn a_compressed_header_reads_the_same_from_inside_its_block() {
        let bytes = rntuple_file(|at| {
            let h = header();
            let block = zl_block(&h);
            let f = empty_footer();
            let (bn, fn_) = (block.len() as u64, f.len() as u64);
            ([block, f].concat(), [at, bn, h.len() as u64, at + bn, fn_, fn_])
        });
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        // The block, the zlib stream in it, the deflate run in that, and the
        // envelope that came out.
        let opened = down(&anchor_path(), &[15, 0, 0, 4, 6, 0]);
        let envelope = ev.node(&d, &opened).unwrap();
        assert_eq!(envelope.type_name, "RNTupleEnvelope");
        assert_ne!(envelope.space, 0);
        let fields = down(&opened, &[2, 5, 2]);
        assert_eq!(ev.node(&d, &down(&fields, &[3])).unwrap().name, "[3] _0");
        assert_eq!(ev.node(&d, &down(&opened, &[2, 6, 2, 2])).unwrap().name, "[2] name");
    }

    #[test]
    fn the_footer_says_which_header_it_belongs_to() {
        let d = Document::new(MemSource(stored()));
        let mut ev = Evaluator::new(root());
        let footer = down(&anchor_path(), &[16, 0, 2]);
        assert_eq!(ev.node(&d, &down(&footer, &[2])).unwrap().value, Value::UInt(0x1122_3344_5566_7788));
        // Nothing was added to the schema, so each of its lists is there and
        // empty, and there are no cluster groups.
        assert_eq!(ev.node(&d, &down(&footer, &[3, 2, 2])).unwrap().child_count, 0);
        assert_eq!(ev.node(&d, &down(&footer, &[4, 2])).unwrap().child_count, 0);
        // A footer written before attribute sets ends where its cluster groups
        // do, so it has none.
        assert!(ev.node(&d, &down(&footer, &[5])).unwrap().absent);
    }

    #[test]
    fn a_cluster_group_links_its_page_list_where_the_locator_says() {
        let (bytes, _) = ntuple(&plain_pages(), false);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        let g = group();
        assert_eq!(ev.node(&d, &down(&g, &[3])).unwrap().value, Value::UInt(1));
        let offset = ev.node(&d, &down(&g, &[5, 1])).unwrap().value.as_int().unwrap();
        let size = ev.node(&d, &down(&g, &[5, 0])).unwrap().value.as_int().unwrap();
        let list = ev.node(&d, &down(&g, &[6, 0])).unwrap();
        assert_eq!((list.offset_bits / 8, list.size_bits / 8), (offset as u64, size as u64));
        assert_eq!(list.space, 0);
        assert_eq!(
            ev.node(&d, &down(&g, &[6, 0, 0])).unwrap().value,
            Value::Enum { raw: 3, name: Some("page list".into()), hex: false }
        );
        // One cluster, five entries, and a list of pages for each of the five
        // columns the header has.
        let payload = down(&g, &[6, 0, 2]);
        assert_eq!(ev.node(&d, &down(&payload, &[1, 2, 0, 2])).unwrap().value, Value::UInt(5));
        assert_eq!(ev.node(&d, &down(&payload, &[2, 2, 0, 2])).unwrap().child_count, 5);
    }

    #[test]
    fn every_page_is_placed_in_the_file_with_its_checksum_after_it() {
        let (bytes, placed) = ntuple(&plain_pages(), false);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        for (column, pages) in placed.iter().enumerate() {
            let list = column_pages(column);
            assert_eq!(ev.node(&d, &down(&list, &[5])).unwrap().child_count as usize, pages.len());
            for (k, (offset, size)) in pages.iter().enumerate() {
                let page = placed_page(column, k);
                let data = ev.node(&d, &down(&page, &[0])).unwrap();
                assert_eq!((data.offset_bits / 8, data.size_bits / 8), (*offset, *size as u64), "column {column} page {k}");
                assert_eq!(data.space, 0);
            }
        }
        // The first page of doubles says it has a checksum with a negative
        // count, and the eight bytes after it are that checksum; the second
        // says nothing of the kind and has none.
        let entry = down(&column_pages(0), &[5, 0]);
        assert_eq!(ev.node(&d, &down(&entry, &[0])).unwrap().value, Value::Int(-3));
        assert_eq!(ev.node(&d, &down(&entry, &[1])).unwrap().value, Value::Int(3));
        let sum = ev.node(&d, &down(&placed_page(0, 0), &[1])).unwrap();
        assert_eq!(sum.offset_bits / 8, placed[0][0].0 + 24);
        assert_eq!(sum.value, Value::UInt(0xc0ff_ee00_c0ff_ee00));
        assert!(ev.node(&d, &down(&placed_page(0, 1), &[1])).unwrap().absent);
        // After the pages, inside the same frame: the element offset and the
        // compression settings.
        assert_eq!(ev.node(&d, &down(&column_pages(0), &[6])).unwrap().value, Value::Int(0));
        assert_eq!(ev.node(&d, &down(&column_pages(0), &[7])).unwrap().value, Value::UInt(101));
    }

    #[test]
    fn a_suppressed_column_has_no_pages_and_no_compression_settings() {
        let mut columns = plain_pages();
        columns[4] = None;
        let (bytes, _) = ntuple(&columns, false);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        let list = column_pages(4);
        assert_eq!(ev.node(&d, &down(&list, &[5])).unwrap().child_count, 0);
        assert_eq!(ev.node(&d, &down(&list, &[6])).unwrap().value, Value::Int(i64::MIN.into()));
        assert!(ev.node(&d, &down(&list, &[7])).unwrap().absent);
        // The column before it is untouched by it.
        assert_eq!(ev.node(&d, &down(&column_pages(3), &[7])).unwrap().value, Value::UInt(101));
    }

    /// The values under a page's `data`, as numbers.
    fn numbers(ev: &mut Evaluator, d: &Document<MemSource>, at: &[usize]) -> Vec<Value> {
        let n = ev.node(d, at).unwrap().child_count as usize;
        (0..n).map(|i| ev.node(d, &down(at, &[i])).unwrap().value).collect()
    }

    #[test]
    fn a_page_reads_as_the_values_its_column_type_says() {
        let (bytes, _) = ntuple(&plain_pages(), false);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        // The column list says which column it is, what type, and whose.
        assert_eq!(ev.node(&d, &column_pages(0)).unwrap().name, "[0] x");
        assert_eq!(ev.node(&d, &column_pages(3)).unwrap().name, "[3] _0");
        assert_eq!(
            ev.node(&d, &down(&column_pages(4), &[3])).unwrap().value,
            Value::Enum { raw: 0x13, name: Some("SplitInt32".into()), hex: true }
        );
        let data = |c, k| down(&placed_page(c, k), &[0]);
        let floats = |v: &[f64]| v.iter().map(|x| Value::Float(*x)).collect::<Vec<_>>();
        assert_eq!(numbers(&mut ev, &d, &data(0, 0)), floats(&[1.5, -2.0, 3.25]));
        assert_eq!(numbers(&mut ev, &d, &data(0, 1)), floats(&[4.0, 5.5]));
        assert_eq!(numbers(&mut ev, &d, &data(1, 0)), [3u128, 3, 8, 8, 8].map(Value::UInt).to_vec());
        assert_eq!(ev.node(&d, &data(2, 0)).unwrap().value, Value::Str("abcdefgh".into()));
        assert_eq!(numbers(&mut ev, &d, &data(3, 0)), floats(&[0.5, 1.0, 2.0]));
        // A split column keeps its bytes, and says how they are laid out.
        let split = ev.node(&d, &data(4, 0)).unwrap();
        assert_eq!(split.type_name, "RNTupleZigzagSplit");
        assert_eq!(ev.node(&d, &down(&data(4, 0), &[0])).unwrap().size_bits, 20 * 8);
    }

    #[test]
    fn a_compressed_page_opens_and_holds_the_same_values() {
        let mut columns = plain_pages();
        let many = vec![7.0; 40];
        columns[0].as_mut().unwrap()[1] = Page { data: doubles(&many), elements: 40, checksum: true, packed: true };
        let (bytes, placed) = ntuple(&columns, false);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        let data = down(&placed_page(0, 1), &[0]);
        let node = ev.node(&d, &data).unwrap();
        assert_eq!((node.offset_bits / 8, node.size_bits / 8), (placed[0][1].0, placed[0][1].1 as u64));
        // The block, the zlib stream in it, the deflate run, and the values
        // that came out.
        assert_eq!(
            ev.node(&d, &down(&data, &[0, 0])).unwrap().value,
            Value::Enum { raw: 0x5a4c, name: Some("zlib".into()), hex: true }
        );
        let values = down(&data, &[0, 4, 6, 0]);
        assert_ne!(ev.node(&d, &values).unwrap().space, 0);
        assert_eq!(numbers(&mut ev, &d, &values), vec![Value::Float(7.0); 40]);
        // The stored page beside it is still read as it stands.
        assert_eq!(ev.node(&d, &down(&placed_page(0, 0), &[0])).unwrap().child_count, 3);
    }

    #[test]
    fn with_the_header_compressed_a_page_is_opened_by_its_bytes_and_left_untyped() {
        let mut columns = plain_pages();
        columns[0].as_mut().unwrap()[1] = Page { data: doubles(&[7.0; 40]), elements: 40, checksum: false, packed: true };
        let (bytes, placed) = ntuple(&columns, true);
        let d = Document::new(MemSource(bytes));
        let mut ev = Evaluator::new(root());
        // Nothing here can reach the column's description, so there is no type
        // and no field name, and the column is known by its number.
        assert!(ev.node(&d, &down(&column_pages(0), &[3])).unwrap().absent);
        assert!(ev.node(&d, &down(&column_pages(0), &[4])).unwrap().absent);
        assert_eq!(ev.node(&d, &column_pages(0)).unwrap().name, "[0]");
        // The compressed page still opens, because it opens like a block, and
        // what comes out is its bytes.
        let data = down(&placed_page(0, 1), &[0]);
        assert_eq!(ev.node(&d, &data).unwrap().size_bits / 8, placed[0][1].1 as u64);
        let opened = ev.node(&d, &down(&data, &[0, 4, 6, 0])).unwrap();
        assert_eq!(opened.value, Value::Bytes { len: 320, preview: 7.0f64.to_le_bytes().repeat(2) });
        // The stored ones are bytes as they stand.
        assert_eq!(ev.node(&d, &down(&placed_page(2, 0), &[0])).unwrap().value.clone(), Value::Bytes {
            len: 8,
            preview: b"abcdefgh".to_vec()
        });
    }
}

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
/// ones store differences; see [`super::rntuple`]'s page contents for which of
/// these read as values.
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
        ],
    )
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
        .with_type("RNTuplePageList", T::bytes(E::Remaining))
        .with_type("RNTupleField", field())
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

    /// A header with four fields and four columns: a double; a string, which
    /// is an index column and a character column; and an array of three
    /// floats, whose one column belongs to the field under it. The string's
    /// field record has four bytes more in it than this version knows about.
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
        ]));
        p.extend(list(&[column(0x0d, 64, 0), column(0x0f, 64, 1), column(0x02, 8, 1), column(0x0c, 32, 3)]));
        p.extend(list(&[]));
        p.extend(list(&[]));
        envelope(1, &p)
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
        assert_eq!(ev.node(&d, &fields).unwrap().child_count, 4);
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
        assert_eq!(ev.node(&d, &columns).unwrap().child_count, 4);
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
}

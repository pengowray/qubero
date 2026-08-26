//! HDF5: a file of objects reached by address, which is what an `.h5ad`
//! single-cell dataset, a Keras model and a decade of scientific data are
//! written in.
//!
//! Nothing here is laid out one thing after another. The superblock names the
//! address of the root group's object header; that header holds a message
//! naming a b-tree and a local heap; the b-tree's leaves name symbol table
//! nodes; each entry in one names the address of another object header, and
//! the name of the object is at an offset into the heap's data segment. Every
//! one of those steps is [`Ty::At`](crate::template::Ty::At), a field that
//! costs no bytes where it is declared and reads its contents somewhere else,
//! and the whole tree falls out of it: the file's group hierarchy is the
//! template's field tree.
//!
//! That is further than SQLite is followed here, where a page number is read
//! as a number and left alone so the template describes a file rather than a
//! graph. HDF5 gives no choice: an address is the only thing a header holds,
//! and a reader that stops at one shows a superblock and nothing else. What
//! makes it affordable is that evaluation is lazy by path, so a group is
//! walked when someone opens it and not before. What it costs is that a file
//! whose links form a cycle (two groups holding hard links to each other, which
//! HDF5 allows) can be opened forever, one level per click. No file written by
//! a tool does that, and nothing here spins on its own.
//!
//! The names are what the arrangement is for, and they are the reason the
//! b-tree is placed under the local heap rather than beside it: an expression
//! sees the fields of the structures it sits inside, so the heap's data
//! segment address is in scope for every entry in every node of the tree
//! below it, which is where a name is a heap offset and nothing else.
//!
//! ## What is read
//!
//! Superblock versions 0 and 1, the object header of version 1 and its
//! messages, version 1 b-trees (both the group ones and the chunk ones),
//! local heaps, and symbol table nodes. The messages read as their own fields
//! are the ones a dataset is made of: dataspace, datatype, both fill values,
//! link info, link, data layout, group info, the filter pipeline, attributes,
//! comments, modification times, the symbol table, and the continuation that
//! puts the rest of an object's messages elsewhere in the file. A message of
//! any other type keeps its bytes.
//!
//! The object header of version 2 (`OHDR`) is read as far as its messages,
//! which are the same messages in a shorter wrapper.
//!
//! ## What is not
//!
//! Only 8-byte offsets and lengths. The superblock says what size it uses and
//! every writer in practice says 8; a file that says 4 is read wrong from the
//! root group entry onwards rather than refused, which the size fields
//! themselves make plain.
//!
//! The base address is not added to the addresses read: it is zero in every
//! file that does not sit inside another one.
//!
//! A signature may also sit at 512, 1024 or any later power of two, with a
//! user block in front of it. Only a file that starts with one is claimed.
//!
//! Version 2 and 3 superblocks are read to their root object header address,
//! and stop there. A file written that way keeps its links in a fractal heap
//! indexed by a version 2 b-tree, which is a different machine from the one
//! here and the next thing to build.
//!
//! ## The elements
//!
//! A dataset's bytes are placed where its layout message says: one run for a
//! contiguous dataset, one per chunk for a chunked one, in the header itself
//! for a compact one. They read as the dataset's elements, and what an element
//! is comes from the datatype message beside the layout one: integers of any
//! width and either sign, floats, fixed-width strings, and the note a
//! variable-length element leaves. A datatype this does not take apart, a
//! compound row among them, is one element of the right size and its own
//! bytes.
//!
//! Nothing in a layout message says what its elements are, and nothing in a
//! datatype message says where they are, so the two have to see each other.
//! They are separate messages in one list, which is what `Expr::Sibling`
//! reaches across: the same thing a WAVE `data` chunk does to learn its sample
//! width from the `fmt ` chunk before it. The width is then written into a
//! field of no bytes, because a list is walked element by element unless its
//! elements are all the same size, and "the same size" has to name a field
//! rather than ask a message two levels away.
//!
//! A chunk that went through a filter is the exception: what is written there
//! is the pipeline's output, usually deflated, so it keeps its bytes. Undoing
//! that belongs where `pdf_objstm` and `ggml_quant` do the same job, not in a
//! field.
//!
//! A variable-length element is a length, the address of a global heap
//! collection and an index into it. All three are shown, and the collection is
//! placed, so the bytes are one step from the note that names them. Which
//! object in the collection is the one is left to the reader: the objects have
//! no fixed size, so the fifteenth is wherever the fourteen before it ended,
//! and there is no expression here for "the element whose index is this".
//!
//! An attribute's value reads as elements too, by the datatype written inside
//! the attribute rather than beside it. That is the one thing the IR could not
//! say: `Expr::Ref` names a field beside this one and stops there, so
//! `Expr::Within` was added to name a field and then a path down into it.
//! `shape` on an `.h5ad` group is two numbers because of it.

use crate::template::{
    Encoding,
    Endian::{Big, Little},
    Expr as E, StrLen, Template, Ty as T, Until,
};

/// The address a file writes for "there is nothing here": all ones, in
/// however many bytes an address takes.
const UNDEFINED: i128 = u64::MAX as i128;

pub fn hdf5() -> Template {
    Template::new("hdf5", root())
        .with_type("ObjectHeader", object_header())
        .with_type("Message", message())
        .with_type("MessageV2", message_v2())
        .with_type("Node", node())
        .with_type("LocalHeap", local_heap())
        .with_type("Datatype", datatype())
        .with_type("Dataspace", dataspace())
        .with_type("GlobalHeap", global_heap())
}

fn root() -> T {
    T::structure(
        "HDF5",
        vec![
            ("signature", T::magic(b"\x89HDF\r\n\x1a\n")),
            ("superblock_version", T::u8()),
            (
                "superblock",
                T::switch(
                    E::field("superblock_version"),
                    vec![(0, superblock_v0(false)), (1, superblock_v0(true)), (2, superblock_v2()), (3, superblock_v2())],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// The original superblock, and version 1, which adds the two fields for the
/// b-tree that indexes chunked storage and is otherwise the same.
fn superblock_v0(indexed_k: bool) -> T {
    let mut fields = vec![
        ("free_space_version", T::u8()),
        ("root_group_entry_version", T::u8()),
        ("reserved", T::u8()),
        ("shared_message_version", T::u8()),
        // Everything below reads addresses as eight bytes, which is what
        // these two say in every file a tool has written.
        ("offset_size", T::u8()),
        ("length_size", T::u8()),
        ("reserved_2", T::u8()),
        ("group_leaf_k", T::u16(Little)),
        ("group_internal_k", T::u16(Little)),
        ("file_consistency_flags", T::u32(Little)),
    ];
    if indexed_k {
        fields.push(("indexed_storage_internal_k", T::u16(Little)));
        fields.push(("reserved_3", T::u16(Little)));
    }
    fields.extend(vec![
        ("base_address", addr()),
        ("free_space_address", addr()),
        ("end_of_file_address", addr()),
        ("driver_info_address", addr()),
        // The root group, and with it every object in the file.
        ("root_group", symbol_table_entry(false)),
    ]);
    T::structure("Superblock", fields)
}

/// Versions 2 and 3, which drop the caches and the node sizes and add a
/// checksum. Where the objects are is the same question and a different
/// answer: the root group's header is a version 2 one and its links are in a
/// fractal heap, which nothing here reads yet.
fn superblock_v2() -> T {
    T::structure(
        "Superblock",
        vec![
            ("offset_size", T::u8()),
            ("length_size", T::u8()),
            ("file_consistency_flags", T::u8()),
            ("base_address", addr()),
            ("superblock_extension_address", addr()),
            ("end_of_file_address", addr()),
            ("root_group_object_header_address", addr()),
            ("checksum", T::u32(Little)),
            ("root_group", at_address("root_group_object_header_address", T::Named("ObjectHeader".into()))),
        ],
    )
}

/// An address, which is as wide as the superblock's offset size says and is
/// read here as the eight bytes it always is.
fn addr() -> T {
    T::u64(Little)
}

/// A length, which the superblock sizes separately from an address and which
/// no writer sizes differently.
fn length() -> T {
    T::u64(Little)
}

/// What `field` points at, or nothing when it holds the undefined address.
fn at_address(field: &str, inner: T) -> T {
    T::switch(
        E::field(field),
        vec![(UNDEFINED, T::bytes(E::lit(0)))],
        T::at(E::field(field), inner),
    )
}

/// One bit of a flags field as a number, since there is no bitwise operator:
/// shift it down and take away everything above it.
fn bit(field: &str, k: u32) -> E {
    let below = E::field(field).div(E::lit(1i128 << k));
    below.clone().sub(E::field(field).div(E::lit(1i128 << (k + 1))).mul(E::lit(2)))
}

/// A field that is there when a flag is set and takes no bytes when it is not.
fn when(flag: E, ty: T) -> T {
    T::switch(flag, vec![(1, ty)], T::bytes(E::lit(0)))
}

/// Rounded up to the next multiple of eight, which is how a version 1 object
/// header aligns everything inside it.
fn pad8(e: E) -> E {
    e.add(E::lit(7)).div(E::lit(8)).mul(E::lit(8))
}

/// The sixteen bytes a symbol table entry keeps for a soft link: an offset
/// into the group's heap, and, where that heap is in scope, the path it holds.
fn link_cache(named: bool) -> T {
    let mut fields = vec![("link_value_offset", T::u32(Little))];
    if named {
        fields.push((
            "link_value",
            T::at(
                E::field("data_segment_address").add(E::field("link_value_offset")),
                T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Utf8),
            ),
        ));
    }
    fields.push(("unused", T::bytes(E::lit(12))));
    T::structure("LinkCache", fields)
}

/// An entry in a symbol table: a name, an address, and a cache of what is
/// there. `named` says whether a local heap is in scope, which it is for
/// every entry but the root group's: that one is in the superblock, where the
/// heap holding its name has not been reached and its name is the empty one.
fn symbol_table_entry(named: bool) -> T {
    let mut fields = vec![
        ("link_name_offset", length()),
        ("object_header_address", addr()),
        (
            "cache_type",
            T::enumeration(
                "CacheType",
                T::u32(Little),
                &[(0, "nothing cached"), (1, "group"), (2, "symbolic link")],
            ),
        ),
        ("reserved", T::u32(Little)),
        (
            "scratch",
            T::switch(
                E::field("cache_type"),
                vec![
                    // A copy of what the object header's symbol table message
                    // says, kept here so a reader need not open the header to
                    // walk past the group. Read as the two numbers it is: the
                    // header below places the tree, and placing it twice would
                    // show every group in the file twice.
                    (
                        1,
                        T::inline_structure(
                            "GroupCache",
                            vec![("cached_btree_address", addr()), ("cached_heap_address", addr())],
                        ),
                    ),
                    (2, link_cache(named)),
                ],
                T::bytes(E::lit(16)),
            ),
        ),
    ];
    if named {
        // The name is not here: it is a byte offset into the data segment of
        // the local heap this node hangs under, which is in scope because the
        // tree is placed inside the heap.
        fields.push((
            "name",
            T::at(
                E::field("data_segment_address").add(E::field("link_name_offset")),
                T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Utf8),
            ),
        ));
    }
    fields.push(("object", at_address("object_header_address", T::Named("ObjectHeader".into()))));
    if named {
        T::structure_named("Link", "name", "object", fields)
    } else {
        T::structure("Link", fields)
    }
}

/// An object header, in either of the two shapes a file writes one: the
/// version 1 one, which opens with its version byte, and the version 2 one,
/// which opens with `OHDR`.
fn object_header() -> T {
    T::switch(
        E::peek(8, Little),
        vec![(1, object_header_v1()), (b'O' as i128, object_header_v2())],
        T::structure("UnknownObjectHeader", vec![("version", T::u8())]),
    )
}

fn object_header_v1() -> T {
    T::structure(
        "ObjectHeader",
        vec![
            ("version", T::u8()),
            ("reserved", T::u8()),
            ("message_count", T::u16(Little)),
            ("reference_count", T::u32(Little)),
            // The messages only: the sixteen bytes of prefix are not counted.
            ("header_size", T::u32(Little)),
            ("padding", T::bytes(E::lit(4))),
            ("messages", T::sized(E::field("header_size"), T::repeat(T::Named("Message".into()), Until::End))),
        ],
    )
}

/// The version 2 header. Its size field is one, two, four or eight bytes wide
/// depending on the low two bits of the flags, so the messages are sized by
/// whichever of the four was written.
fn object_header_v2() -> T {
    let size_field = |bits: u32| T::UInt { bits, endian: Little };
    T::structure(
        "ObjectHeader",
        vec![
            ("signature", T::magic(b"OHDR")),
            ("version", T::u8()),
            (
                "header_flags",
                T::flags(
                    "HeaderFlags",
                    T::u8(),
                    &[
                        (2, "attribute creation order tracked"),
                        (3, "attribute creation order indexed"),
                        (4, "non-default attribute storage"),
                        (5, "times stored"),
                    ],
                ),
            ),
            (
                "times",
                when(
                    bit("header_flags", 5),
                    T::inline_structure(
                        "Times",
                        vec![
                            ("access_time", T::u32(Little)),
                            ("modification_time", T::u32(Little)),
                            ("change_time", T::u32(Little)),
                            ("birth_time", T::u32(Little)),
                        ],
                    ),
                ),
            ),
            (
                "attribute_storage",
                when(
                    bit("header_flags", 4),
                    T::inline_structure(
                        "AttributeStorage",
                        vec![("max_compact", T::u16(Little)), ("min_dense", T::u16(Little))],
                    ),
                ),
            ),
            (
                "chunk_size",
                T::switch(
                    E::field("header_flags").sub(E::field("header_flags").div(E::lit(4)).mul(E::lit(4))),
                    vec![(0, size_field(8)), (1, size_field(16)), (2, size_field(32)), (3, size_field(64))],
                    T::bytes(E::lit(0)),
                ),
            ),
            ("messages", T::sized(E::field("chunk_size"), T::repeat(T::Named("MessageV2".into()), Until::End))),
            ("checksum", T::u32(Little)),
        ],
    )
}

/// The messages of an object header, which are what says whether an object is
/// a group or a dataset, what shape it has and where its data is.
const MESSAGE_TYPE: &[(i128, &str)] = &[
    (0x00, "nil"),
    (0x01, "dataspace"),
    (0x02, "link info"),
    (0x03, "datatype"),
    (0x04, "fill value (old)"),
    (0x05, "fill value"),
    (0x06, "link"),
    (0x07, "external data files"),
    (0x08, "data layout"),
    (0x09, "bogus"),
    (0x0a, "group info"),
    (0x0b, "filter pipeline"),
    (0x0c, "attribute"),
    (0x0d, "object comment"),
    (0x0e, "modification time (old)"),
    (0x0f, "shared message table"),
    (0x10, "object header continuation"),
    (0x11, "symbol table"),
    (0x12, "modification time"),
    (0x13, "btree k values"),
    (0x14, "driver info"),
    (0x15, "attribute info"),
    (0x16, "reference count"),
];

fn message_flags() -> T {
    T::flags(
        "MessageFlags",
        T::u8(),
        &[
            (0, "constant"),
            (1, "shared"),
            (2, "do not share"),
            (3, "fail on unknown while writing"),
            (4, "mark on unknown"),
            (5, "was not understood"),
            (6, "shareable"),
            (7, "fail on unknown"),
        ],
    )
}

fn message() -> T {
    T::structure_named(
        "Message",
        "type",
        "body",
        vec![
            ("type", T::enumeration_hex("MessageType", T::u16(Little), MESSAGE_TYPE)),
            // Always a multiple of eight, padding included.
            ("size", T::u16(Little)),
            ("flags", message_flags()),
            ("reserved", T::bytes(E::lit(3))),
            ("body", T::sized(E::field("size"), message_body())),
        ],
    )
}

/// The same messages in the version 2 wrapper: a one-byte type, no reserved
/// bytes, no padding, and a creation order when the header asked for one.
fn message_v2() -> T {
    T::structure_named(
        "Message",
        "type",
        "body",
        vec![
            ("type", T::enumeration_hex("MessageType", T::u8(), MESSAGE_TYPE)),
            ("size", T::u16(Little)),
            ("flags", message_flags()),
            // Present when the header said it tracks creation order, which is
            // its own flag and not this message's.
            ("creation_order", when(bit("header_flags", 2), T::u16(Little))),
            ("body", T::sized(E::field("size"), message_body())),
        ],
    )
}

fn message_body() -> T {
    T::switch(
        E::field("type"),
        vec![
            (0x00, T::bytes(E::Remaining)),
            (0x01, T::Named("Dataspace".into())),
            (0x02, link_info()),
            (0x03, T::Named("Datatype".into())),
            (0x04, fill_value_old()),
            (0x05, fill_value()),
            (0x06, link()),
            (0x08, data_layout()),
            (0x0a, group_info()),
            (0x0b, filter_pipeline()),
            (0x0c, attribute()),
            (0x0d, T::structure("Comment", vec![("comment", T::cstr())])),
            (0x10, continuation()),
            (0x11, symbol_table()),
            (0x12, modification_time()),
        ],
        T::bytes(E::Remaining),
    )
}

/// How many dimensions a dataset has and how long each of them is.
fn dataspace() -> T {
    T::structure(
        "Dataspace",
        vec![
            ("version", T::u8()),
            ("dimensionality", T::u8()),
            ("flags", T::flags("DataspaceFlags", T::u8(), &[(0, "max dimensions"), (1, "permutation")])),
            // Version 1 pads to eight bytes here; version 2 spends the first
            // of those bytes saying whether the space is scalar, simple or
            // null, and has no maximum-dimension list.
            (
                "kind",
                T::switch(
                    E::field("version"),
                    vec![(
                        1,
                        T::inline_structure(
                            "Reserved",
                            vec![("reserved", T::u8()), ("reserved_2", T::u32(Little))],
                        ),
                    )],
                    T::enumeration("SpaceType", T::u8(), &[(0, "scalar"), (1, "simple"), (2, "null")]),
                ),
            ),
            ("dimensions", T::array(length(), E::field("dimensionality")).counted_as("dimensions")),
            ("max_dimensions", when(bit("flags", 0), T::array(length(), E::field("dimensionality")))),
            ("permutation", when(bit("flags", 1), T::array(length(), E::field("dimensionality")))),
        ],
    )
}

/// What one element of a dataset is. The class says which of a dozen kinds,
/// and the properties after the size differ for every one of them; the three
/// whose properties are worth naming are named, and the rest keep their bytes.
fn datatype() -> T {
    T::structure(
        "Datatype",
        vec![
            // The high nibble is the version and the low one is the class,
            // which is why they are read as four bits each rather than as a
            // byte and a shift.
            ("version", T::UInt { bits: 4, endian: Little }),
            (
                "class",
                T::enumeration(
                    "DatatypeClass",
                    T::UInt { bits: 4, endian: Little },
                    &[
                        (0, "fixed-point"),
                        (1, "floating-point"),
                        (2, "time"),
                        (3, "string"),
                        (4, "bit field"),
                        (5, "opaque"),
                        (6, "compound"),
                        (7, "reference"),
                        (8, "enumerated"),
                        (9, "variable-length"),
                        (10, "array"),
                    ],
                ),
            ),
            // Three bytes whose meaning is the class's own. The first of them
            // carries what a reader of the data needs, so it is read as flags
            // rather than kept whole: whether the bytes are big-endian, and,
            // for an integer, whether they are signed.
            (
                "bit_field",
                T::switch(
                    E::field("class"),
                    vec![
                        (0, T::flags("IntegerBits", T::u8(), &[(0, "big-endian"), (3, "signed")])),
                        (1, T::flags("FloatBits", T::u8(), &[(0, "big-endian")])),
                        (
                            3,
                            T::flags("StringBits", T::u8(), &[(0, "null-terminated"), (1, "null-padded"), (4, "utf-8")]),
                        ),
                    ],
                    T::u8(),
                ),
            ),
            ("bit_field_rest", T::u16(Little)),
            ("size", T::u32(Little).counted_as("bytes per element")),
            (
                "properties",
                T::switch(
                    E::field("class"),
                    vec![
                        (
                            0,
                            T::inline_structure(
                                "FixedPoint",
                                vec![("bit_offset", T::u16(Little)), ("bit_precision", T::u16(Little))],
                            ),
                        ),
                        (
                            1,
                            T::inline_structure(
                                "FloatingPoint",
                                vec![
                                    ("bit_offset", T::u16(Little)),
                                    ("bit_precision", T::u16(Little)),
                                    ("exponent_location", T::u8()),
                                    ("exponent_size", T::u8()),
                                    ("mantissa_location", T::u8()),
                                    ("mantissa_size", T::u8()),
                                    ("exponent_bias", T::u32(Little)),
                                ],
                            ),
                        ),
                        // A string says everything it has to say in the class
                        // bits: padding in the low nibble, character set in
                        // the one above it.
                        (3, T::bytes(E::lit(0))),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// How wide one element is, from the datatype message beside this one. Zero
/// when there is none in reach, which reads as one byte rather than dividing
/// by nothing.
/// Where the datatype describing a run of elements is written. A dataset's is
/// a message of its own beside the layout message that places the bytes; an
/// attribute writes one inside itself, before its data. The elements are read
/// the same way either way, so the only difference is how the three things a
/// reader needs are reached.
#[derive(Clone, Copy)]
enum Described {
    /// By a datatype message among the same object's messages.
    Beside,
    /// By the datatype written inside the attribute holding the elements.
    Inside,
}

impl Described {
    fn part(self, name: &str) -> E {
        match self {
            Described::Beside => E::sibling(&["body", name]),
            Described::Inside => E::within(&["datatype", name]),
        }
    }
}

fn element_size(by: Described) -> E {
    by.part("size").or(E::lit(1))
}

/// A field of no bytes holding the width of one element, so that everything
/// below can name it rather than ask the datatype message again.
///
/// This is not tidiness. A list is walked element by element unless its
/// elements are all the same size, and "the same size" has to be an expression
/// that cannot vary from one element to the next: naming a field is such an
/// expression and asking a message two levels away is not. Without this a
/// column of ten million strings would be measured by reading all ten million.
fn element_size_field(by: Described) -> (&'static str, T) {
    ("element_size", T::computed(element_size(by)))
}

/// What one element of a dataset is, read from the datatype message that sits
/// before the layout one among the object's messages. A datatype this does not
/// take apart leaves its elements as their own bytes, which is still one row
/// per element and still the right size.
fn element_type(by: Described) -> T {
    // Class, byte order, width and sign in one number, so a single switch can
    // ask about all four: there is no bitwise operator here, and four nested
    // switches would say the same thing at four times the length.
    let bits = by.part("bit_field");
    let big_endian = bits.clone().sub(bits.clone().div(E::lit(2)).mul(E::lit(2)));
    let signed = bits.clone().div(E::lit(8)).sub(bits.div(E::lit(16)).mul(E::lit(2)));
    let key = by
        .part("class")
        .mul(E::lit(1000))
        .add(big_endian.mul(E::lit(500)))
        .add(by.part("size").mul(E::lit(2)))
        .add(signed);
    let numeric = vec![
        (2, T::u8()),
        (3, T::Int { bits: 8, endian: Little }),
        (4, T::u16(Little)),
        (5, T::Int { bits: 16, endian: Little }),
        (8, T::u32(Little)),
        (9, T::i32(Little)),
        (16, T::u64(Little)),
        (17, T::Int { bits: 64, endian: Little }),
        (502, T::UInt { bits: 8, endian: Big }),
        (503, T::Int { bits: 8, endian: Big }),
        (504, T::u16(Big)),
        (505, T::Int { bits: 16, endian: Big }),
        (508, T::u32(Big)),
        (509, T::i32(Big)),
        (516, T::u64(Big)),
        (517, T::Int { bits: 64, endian: Big }),
        (1004, T::F16(Little)),
        (1008, T::F32(Little)),
        (1016, T::F64(Little)),
        (1504, T::F16(Big)),
        (1508, T::F32(Big)),
        (1516, T::F64(Big)),
    ];
    let width = E::field("element_size");
    // The switch is outside the arrays rather than inside one: an array of a
    // type chosen per element has no stride, and a stride is what lets the
    // cursor land in the middle of thirteen million numbers without reading
    // the ones before it.
    let count = E::field("run_bytes").div(width.clone());
    let of = |t: T| T::array(t, count.clone()).counted_as("elements");
    let numeric: Vec<(i128, T)> = numeric.into_iter().map(|(k, t)| (k, of(t))).collect();
    let opaque = of(T::sized(width.clone(), T::bytes(width.clone())));
    T::switch(
        by.part("class"),
        vec![
            (0, T::switch(key.clone(), numeric.clone(), opaque.clone())),
            (1, T::switch(key, numeric, opaque.clone())),
            // A string of a fixed width, which is what a column of names is.
            // Sized as well as padded, so the run has a stride.
            (
                3,
                of(T::sized(
                    width.clone(),
                    T::text(StrLen::Padded { size: width.clone(), pad: 0 }, Encoding::Utf8),
                )),
            ),
            // A variable-length element is not the thing but a note saying
            // where the thing is: how long it is, which global heap
            // collection holds it, and which object in that collection it is.
            // The collection is reached from here and from nowhere else, and
            // an object inside one is found by walking it rather than by
            // arithmetic, so this stops at the note. Sixteen bytes is the
            // shape a file with eight-byte addresses writes; anything else is
            // left as its bytes.
            (
                9,
                T::switch(
                    width.clone(),
                    vec![(16, of(T::sized(width.clone(), vlen_reference())))],
                    opaque.clone(),
                ),
            ),
        ],
        opaque,
    )
}

/// The note a variable-length element leaves in place of its contents: how
/// long it is, which global heap collection holds it, and which object in that
/// collection it is. The collection is placed, so the bytes are one step away;
/// which object is which is a matter of reading the indices, since the objects
/// are of no fixed size and the fifteenth is wherever the fourteen before it
/// ended.
fn vlen_reference() -> T {
    T::structure(
        "GlobalHeapId",
        vec![
            ("length", T::u32(Little).counted_as("bytes")),
            ("collection_address", addr()),
            ("object_index", T::u32(Little)),
            ("collection", at_address("collection_address", T::Named("GlobalHeap".into()))),
        ],
    )
}

/// A global heap collection: everything that had no fixed size, written
/// together in one block that several datasets share.
fn global_heap() -> T {
    T::structure(
        "GlobalHeap",
        vec![
            ("signature", T::magic(b"GCOL")),
            ("version", T::u8()),
            ("reserved", T::bytes(E::lit(3))),
            ("collection_size", length().counted_as("bytes")),
            (
                "objects",
                T::sized(
                    E::field("collection_size").sub(E::lit(16)),
                    T::repeat(global_heap_object(), Until::End),
                )
                .counted_as("objects"),
            ),
        ],
    )
}

/// One object in a collection. Index zero is not an object but the free space
/// after the last one, and it says how much there is in the same field the
/// others use for their length.
fn global_heap_object() -> T {
    // A collection is a round number of bytes and its objects are not, so the
    // last few bytes can be too few to hold even the sixteen a header takes.
    // That tail is padding rather than an object read past the end of the
    // collection, and this is where the difference is decided: what is left,
    // before anything is read.
    T::switch(
        E::Remaining.less_than(E::lit(16)),
        vec![(1, T::structure("Padding", vec![("padding", T::bytes(E::Remaining))]))],
        heap_object(),
    )
}

fn heap_object() -> T {
    T::structure_named(
        "HeapObject",
        "object_index",
        "data",
        vec![
            ("object_index", T::u16(Little)),
            ("reference_count", T::u16(Little)),
            ("reserved", T::u32(Little)),
            ("size", length().counted_as("bytes")),
            // Rounded up to eight bytes, and the padding belongs to nobody.
            //
            // Object zero is the free space rather than an object, and its
            // size counts the sixteen bytes of header this one has already
            // read, so it is measured to the end of the collection instead:
            // that is what free space is, and it saves subtracting a header
            // from a length that may be shorter than one.
            (
                "data",
                T::switch(E::field("object_index"), vec![(0, T::bytes(E::Remaining))], T::bytes(pad8(E::field("size")))),
            ),
        ],
    )
}

/// A run of `bytes` bytes read as the dataset's elements. The two fields of no
/// bytes in front of them are how far the elements can see: an expression
/// reads the fields of the structures it sits in, and both of these are
/// answers from somewhere else in the object header.
fn elements(by: Described, bytes: E) -> T {
    T::structure(
        "Data",
        vec![element_size_field(by), ("run_bytes", T::computed(bytes)), ("elements", element_type(by))],
    )
}

fn fill_value_old() -> T {
    T::structure(
        "FillValue",
        vec![("size", T::u32(Little)), ("value", T::bytes(E::field("size")))],
    )
}

fn fill_value() -> T {
    T::structure(
        "FillValue",
        vec![
            ("version", T::u8()),
            (
                "body",
                T::switch(
                    E::field("version"),
                    vec![(
                        3,
                        T::inline_structure(
                            "Flagged",
                            vec![
                                ("flags", T::u8()),
                                // Bit 5 says a value was written; without it
                                // the message ends here.
                                (
                                    "value",
                                    when(
                                        bit("flags", 5),
                                        T::inline_structure(
                                            "Value",
                                            vec![("size", T::u32(Little)), ("value", T::bytes(E::field("size")))],
                                        ),
                                    ),
                                ),
                            ],
                        ),
                    )],
                    T::inline_structure(
                        "Described",
                        vec![
                            ("space_allocation_time", T::u8()),
                            ("fill_write_time", T::u8()),
                            ("fill_defined", T::u8()),
                            (
                                "value",
                                when(
                                    E::field("fill_defined"),
                                    T::inline_structure(
                                        "Value",
                                        vec![("size", T::u32(Little)), ("value", T::bytes(E::field("size")))],
                                    ),
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        ],
    )
}

/// A link in a new-style group, which keeps its links as messages rather than
/// in a symbol table.
fn link() -> T {
    T::structure_named(
        "Link",
        "name",
        "target",
        vec![
            ("version", T::u8()),
            (
                "flags",
                T::flags(
                    "LinkFlags",
                    T::u8(),
                    &[(2, "creation order"), (3, "link type"), (4, "character set")],
                ),
            ),
            ("link_type", when(bit("flags", 3), T::enumeration("LinkType", T::u8(), &[(0, "hard"), (1, "soft"), (64, "external")]))),
            ("creation_order", when(bit("flags", 2), T::u64(Little))),
            ("character_set", when(bit("flags", 4), T::enumeration("Charset", T::u8(), &[(0, "ascii"), (1, "utf-8")]))),
            // The low two bits of the flags say how wide the name's length is.
            (
                "name_length",
                T::switch(
                    E::field("flags").sub(E::field("flags").div(E::lit(4)).mul(E::lit(4))),
                    vec![
                        (0, T::u8()),
                        (1, T::u16(Little)),
                        (2, T::u32(Little)),
                        (3, T::u64(Little)),
                    ],
                    T::u8(),
                ),
            ),
            ("name", T::utf8(E::field("name_length"))),
            (
                "target",
                T::switch(
                    E::field("link_type"),
                    vec![
                        (
                            1,
                            T::inline_structure(
                                "SoftLink",
                                vec![("value_length", T::u16(Little)), ("value", T::utf8(E::field("value_length")))],
                            ),
                        ),
                        (
                            64,
                            T::inline_structure(
                                "ExternalLink",
                                vec![("value_length", T::u16(Little)), ("value", T::bytes(E::field("value_length")))],
                            ),
                        ),
                    ],
                    // A hard link, which is an address, and the object at it.
                    T::structure(
                        "HardLink",
                        vec![
                            ("object_header_address", addr()),
                            ("object", at_address("object_header_address", T::Named("ObjectHeader".into()))),
                        ],
                    ),
                ),
            ),
        ],
    )
}

fn link_info() -> T {
    T::structure(
        "LinkInfo",
        vec![
            ("version", T::u8()),
            ("flags", T::flags("LinkInfoFlags", T::u8(), &[(0, "creation order tracked"), (1, "creation order indexed")])),
            ("max_creation_index", when(bit("flags", 0), T::u64(Little))),
            // Where a new-style group keeps its links: a fractal heap, which
            // nothing here reads, and the b-trees that index it.
            ("fractal_heap_address", addr()),
            ("name_index_btree_address", addr()),
            ("creation_order_index_address", when(bit("flags", 1), addr())),
        ],
    )
}

fn group_info() -> T {
    T::structure(
        "GroupInfo",
        vec![
            ("version", T::u8()),
            ("flags", T::flags("GroupInfoFlags", T::u8(), &[(0, "link phase change"), (1, "estimated entry info")])),
            (
                "phase_change",
                when(
                    bit("flags", 0),
                    T::inline_structure(
                        "PhaseChange",
                        vec![("max_compact", T::u16(Little)), ("min_dense", T::u16(Little))],
                    ),
                ),
            ),
            (
                "estimates",
                when(
                    bit("flags", 1),
                    T::inline_structure(
                        "Estimates",
                        vec![("entry_count", T::u16(Little)), ("name_length", T::u16(Little))],
                    ),
                ),
            ),
        ],
    )
}

/// Where a dataset's elements are: in the header itself, in one run, or in
/// chunks placed by a b-tree of their own.
fn data_layout() -> T {
    T::structure(
        "DataLayout",
        vec![
            ("version", T::u8()),
            (
                "body",
                // Version 4 and whatever follows it write the dimensions in
                // a width the message itself declares and name one of half a
                // dozen ways of indexing the chunks. That is a machine of its
                // own; until it is here, such a message keeps its bytes.
                T::switch(
                    E::field("version"),
                    vec![(1, layout_v1()), (2, layout_v1()), (3, layout_v3())],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// Versions 1 and 2, which write the dimensions before the class and always in
/// four bytes.
fn layout_v1() -> T {
    T::structure(
        "Layout",
        vec![
            ("dimensionality", T::u8()),
            ("layout_class", layout_class()),
            ("reserved", T::bytes(E::lit(5))),
            ("address", addr()),
            ("dimensions", T::array(T::u32(Little), E::field("dimensionality"))),
            ("element_size", when(E::field("layout_class").div(E::lit(2)), T::u32(Little))),
            (
                "storage",
                T::switch(
                    E::field("layout_class"),
                    vec![
                        (
                            0,
                            T::structure(
                                "Compact",
                                vec![("size", T::u32(Little)), ("data", elements(Described::Beside, E::field("size")))],
                            ),
                        ),
                        (2, at_address("address", T::Named("Node".into()))),
                    ],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

fn layout_class() -> T {
    T::enumeration("LayoutClass", T::u8(), &[(0, "compact"), (1, "contiguous"), (2, "chunked")])
}

fn layout_v3() -> T {
    T::structure(
        "Layout",
        vec![
            ("layout_class", layout_class()),
            (
                "storage",
                T::switch(
                    E::field("layout_class"),
                    vec![
                        (
                            0,
                            T::structure(
                                "Compact",
                                vec![("size", T::u16(Little)), ("data", elements(Described::Beside, E::field("size")))],
                            ),
                        ),
                        (
                            1,
                            T::structure(
                                "Contiguous",
                                vec![
                                    ("address", addr()),
                                    ("size", length()),
                                    ("data", at_address("address", elements(Described::Beside, E::field("size")))),
                                ],
                            ),
                        ),
                        (
                            2,
                            T::structure(
                                "Chunked",
                                vec![
                                    // One more than the dataset has: the last
                                    // dimension of a chunk key is the size of
                                    // an element. The b-tree below reads this
                                    // to know how wide its keys are.
                                    ("dimensionality", T::u8()),
                                    ("address", addr()),
                                    (
                                        "chunk_dimensions",
                                        T::array(T::u32(Little), E::field("dimensionality").sub(E::lit(1))),
                                    ),
                                    ("element_size", T::u32(Little)),
                                    ("chunks", at_address("address", T::Named("Node".into()))),
                                ],
                            ),
                        ),
                    ],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// What was done to a chunk's bytes before they were written, which has to be
/// undone in the other order before they are elements again.
fn filter_pipeline() -> T {
    T::structure(
        "FilterPipeline",
        vec![
            ("version", T::u8()),
            ("filter_count", T::u8()),
            // Version 1 pads the header out and pads every filter to eight
            // bytes; version 2 does neither.
            ("reserved", T::switch(E::field("version"), vec![(1, T::bytes(E::lit(6)))], T::bytes(E::lit(0)))),
            ("filters", T::array(filter(), E::field("filter_count"))),
        ],
    )
}

fn filter() -> T {
    T::structure_named(
        "Filter",
        "filter_id",
        "",
        vec![
            (
                "filter_id",
                T::enumeration(
                    "FilterId",
                    T::u16(Little),
                    &[
                        (1, "deflate"),
                        (2, "shuffle"),
                        (3, "fletcher32"),
                        (4, "szip"),
                        (5, "nbit"),
                        (6, "scaleoffset"),
                        (32000, "lzf"),
                        (32001, "blosc"),
                        (32004, "lz4"),
                        (32008, "bitshuffle"),
                        (32015, "zstd"),
                    ],
                ),
            ),
            // Version 2 leaves the length out for a filter the library knows,
            // which is every id below 256.
            (
                "name_length",
                T::switch(
                    E::field("version"),
                    vec![(1, T::u16(Little))],
                    T::switch(
                        E::field("filter_id").less_than(E::lit(256)),
                        vec![(1, T::computed(E::lit(0)))],
                        T::u16(Little),
                    ),
                ),
            ),
            ("flags", T::flags("FilterFlags", T::u16(Little), &[(0, "optional")])),
            ("client_data_count", T::u16(Little)),
            (
                "name",
                T::switch(
                    E::field("version"),
                    vec![(1, T::utf8(pad8(E::field("name_length"))))],
                    T::utf8(E::field("name_length")),
                ),
            ),
            ("client_data", T::array(T::u32(Little), E::field("client_data_count"))),
            // Version 1 pads an odd number of client data values out to eight
            // bytes; the padding is not counted anywhere, so it is here.
            (
                "padding",
                T::switch(
                    E::field("version"),
                    vec![(
                        1,
                        T::bytes(
                            E::field("client_data_count")
                                .sub(E::field("client_data_count").div(E::lit(2)).mul(E::lit(2)))
                                .mul(E::lit(4)),
                        ),
                    )],
                    T::bytes(E::lit(0)),
                ),
            ),
        ],
    )
}

/// An attribute: a name, what one of its elements is, what shape they are in,
/// and the elements themselves. This is where an `.h5ad` file keeps
/// `encoding-type`, and where anything written beside a dataset ends up.
fn attribute() -> T {
    T::structure_named(
        "Attribute",
        "name",
        "data",
        vec![
            ("version", T::u8()),
            // Version 1 has nothing to say here; the later ones keep flags for
            // datatypes and dataspaces shared with another object.
            ("flags", T::u8()),
            ("name_size", T::u16(Little)),
            ("datatype_size", T::u16(Little)),
            ("dataspace_size", T::u16(Little)),
            // Version 3 says what the name's bytes mean; versions 1 and 2 do not.
            (
                "name_charset",
                T::switch(
                    E::field("version"),
                    vec![(3, T::enumeration("Charset", T::u8(), &[(0, "ascii"), (1, "utf-8")]))],
                    T::bytes(E::lit(0)),
                ),
            ),
            // Version 1 rounds all three of the parts below up to eight bytes.
            (
                "name",
                T::switch(
                    E::field("version"),
                    vec![(1, T::text(StrLen::Padded { size: pad8(E::field("name_size")), pad: 0 }, Encoding::Utf8))],
                    T::text(StrLen::Padded { size: E::field("name_size"), pad: 0 }, Encoding::Utf8),
                ),
            ),
            (
                "datatype",
                T::switch(
                    E::field("version"),
                    vec![(1, T::sized(pad8(E::field("datatype_size")), T::Named("Datatype".into())))],
                    T::sized(E::field("datatype_size"), T::Named("Datatype".into())),
                ),
            ),
            (
                "dataspace",
                T::switch(
                    E::field("version"),
                    vec![(1, T::sized(pad8(E::field("dataspace_size")), T::Named("Dataspace".into())))],
                    T::sized(E::field("dataspace_size"), T::Named("Dataspace".into())),
                ),
            ),
            // Whatever is left is the value, read as elements by the datatype
            // written just above it. `shape` is two numbers, `encoding-type`
            // is a word, and both are what a reader of an `.h5ad` came for.
            ("data", elements(Described::Inside, E::Remaining)),
        ],
    )
}

/// The rest of an object's messages, written somewhere else in the file
/// because they did not fit where the first ones are.
fn continuation() -> T {
    T::structure(
        "Continuation",
        vec![
            ("offset", addr()),
            ("length", length()),
            // A version 1 header continues into plain messages; a version 2
            // one writes a block of its own, signed and checksummed, holding
            // the shorter messages that header uses. Which of the two is in
            // hand is the version of the header this message sits in.
            (
                "messages",
                T::switch(
                    E::field("version"),
                    vec![(
                        2,
                        at_address(
                            "offset",
                            T::sized(
                                E::field("length"),
                                T::structure(
                                    "ContinuationBlock",
                                    vec![
                                        ("signature", T::magic(b"OCHK")),
                                        (
                                            "messages",
                                            T::sized(
                                                E::field("length").sub(E::lit(8)),
                                                T::repeat(T::Named("MessageV2".into()), Until::End),
                                            ),
                                        ),
                                        ("checksum", T::u32(Little)),
                                    ],
                                ),
                            ),
                        ),
                    )],
                    at_address(
                        "offset",
                        T::sized(E::field("length"), T::repeat(T::Named("Message".into()), Until::End)),
                    ),
                ),
            ),
        ],
    )
}

/// What makes an object a group: a b-tree of its links, and the heap holding
/// their names.
fn symbol_table() -> T {
    T::structure(
        "SymbolTable",
        vec![
            ("btree_address", addr()),
            ("heap_address", addr()),
            // The tree is placed inside the heap rather than beside it, so
            // that every name offset in it has the heap's data segment in
            // scope. See the note at the top of this file.
            ("heap", at_address("heap_address", T::Named("LocalHeap".into()))),
        ],
    )
}

fn modification_time() -> T {
    T::structure(
        "ModificationTime",
        vec![
            ("version", T::u8()),
            ("reserved", T::bytes(E::lit(3))),
            ("seconds", T::u32(Little).counted_as("seconds since 1970")),
        ],
    )
}

/// The heap a group's link names are written into, and, placed under it, the
/// tree whose keys are offsets into it.
fn local_heap() -> T {
    T::structure(
        "LocalHeap",
        vec![
            ("signature", T::magic(b"HEAP")),
            ("version", T::u8()),
            ("reserved", T::bytes(E::lit(3))),
            ("data_segment_size", length()),
            ("free_list_offset", length()),
            ("data_segment_address", addr()),
            ("data", at_address("data_segment_address", T::bytes(E::field("data_segment_size")))),
            ("tree", at_address("btree_address", T::Named("Node".into()))),
        ],
    )
}

/// What an address inside a tree points at: another tree, a symbol table node,
/// or something neither, which is said rather than guessed at.
fn node() -> T {
    T::switch(
        E::peek(32, crate::template::Endian::Big),
        vec![
            (u32::from_be_bytes(*b"TREE") as i128, btree()),
            (u32::from_be_bytes(*b"SNOD") as i128, symbol_table_node()),
        ],
        T::structure("UnknownNode", vec![("signature", T::utf8(E::lit(4)))]),
    )
}

/// A version 1 b-tree, which indexes either the links of a group or the chunks
/// of a dataset. Which of the two it is settles what a key looks like, and a
/// node at level zero is the one whose children are the things themselves.
fn btree() -> T {
    T::structure(
        "BTree",
        vec![
            ("signature", T::magic(b"TREE")),
            ("node_type", T::enumeration("NodeType", T::u8(), &[(0, "group"), (1, "chunk")])),
            ("node_level", T::u8()),
            ("entries_used", T::u16(Little)),
            ("left_sibling", addr()),
            ("right_sibling", addr()),
            (
                "entries",
                T::array(
                    T::switch(E::field("node_type"), vec![(1, chunk_entry())], group_entry()),
                    E::field("entries_used"),
                )
                .counted_as("entries"),
            ),
            // A tree of n children has n+1 keys, and the last one closes the
            // range rather than opening a child.
            (
                "last_key",
                T::switch(E::field("node_type"), vec![(1, chunk_key())], T::structure("Key", vec![("name_offset", length())])),
            ),
        ],
    )
}

/// A key and a child of a group's tree: the name the child's range starts at,
/// and the node holding the links themselves.
fn group_entry() -> T {
    T::structure_named(
        "Entry",
        "key_name",
        "child",
        vec![
            ("name_offset", length()),
            (
                "key_name",
                T::at(
                    E::field("data_segment_address").add(E::field("name_offset")),
                    T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Utf8),
                ),
            ),
            ("child_address", addr()),
            ("child", at_address("child_address", T::Named("Node".into()))),
        ],
    )
}

/// A key and a child of a chunk tree. The key says where the chunk sits in the
/// dataset and how long it is once written; the child is the chunk itself,
/// which is the bytes a filter pipeline left behind.
fn chunk_entry() -> T {
    T::structure(
        "Entry",
        vec![
            ("chunk_size", T::u32(Little).counted_as("bytes")),
            ("filter_mask", T::u32(Little)),
            // As many offsets as the layout message said dimensions, which is
            // one more than the dataset has.
            ("offsets", T::array(T::u64(Little), E::field("dimensionality"))),
            ("child_address", addr()),
            (
                "child",
                T::switch(
                    E::field("node_level"),
                    // At the bottom of the tree the child is the chunk itself.
                    // Its bytes are elements only when nothing was done to
                    // them on the way out: a filter pipeline among the
                    // messages before this one means what is here is that
                    // pipeline's output, and undoing it is not something a
                    // field can do.
                    vec![(
                        0,
                        at_address(
                            "child_address",
                            T::switch(
                                E::sibling(&["body", "filter_count"]),
                                vec![(0, elements(Described::Beside, E::field("chunk_size")))],
                                // Marked so the panel can find the reader that
                                // undoes the pipeline; the bytes themselves
                                // stay whole, because they are what is in the
                                // file and the elements are not.
                                T::structure("FilteredChunk", vec![("bytes", T::bytes(E::field("chunk_size")))])
                                    .packed_as(super::hdf5_chunk::PACKING),
                            ),
                        ),
                    )],
                    at_address("child_address", T::Named("Node".into())),
                ),
            ),
        ],
    )
}

fn chunk_key() -> T {
    T::structure(
        "Key",
        vec![
            ("chunk_size", T::u32(Little)),
            ("filter_mask", T::u32(Little)),
            ("offsets", T::array(T::u64(Little), E::field("dimensionality"))),
        ],
    )
}

/// A leaf of a group's tree: the links themselves, in name order.
fn symbol_table_node() -> T {
    T::structure(
        "SymbolTableNode",
        vec![
            ("signature", T::magic(b"SNOD")),
            ("version", T::u8()),
            ("reserved", T::u8()),
            ("symbol_count", T::u16(Little)),
            ("symbols", T::array(symbol_table_entry(true), E::field("symbol_count")).counted_as("links")),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::eval::{Evaluator, Value};
    use crate::source::MemSource;

    /// Where things go in the file built below.
    const ROOT_HEADER: u64 = 96;
    const BTREE: u64 = 136;
    const HEAP: u64 = 184;
    const HEAP_DATA: u64 = 216;
    const SNOD: u64 = 232;
    const ALPHA_HEADER: u64 = 280;
    const DATA: u64 = 376;

    /// The path from the file down to the one link the group holds. Every step
    /// of it but the last two is a pointer being followed: the root entry's
    /// object header, its symbol table message, the heap that message names,
    /// the tree placed under the heap, the entry's child node, and the entry
    /// in it.
    const LINK: &[usize] = &[
        2,  // superblock
        14, // root group entry
        5,  // the object header it points at
        0, //
        6,  // its messages
        0,  // the symbol table message
        4,  // its body
        2,  // the heap it names
        0,  //
        7,  // the tree placed under the heap
        0,  //
        6,  // its entries
        0,  //
        3,  // the child node
        0,  //
        4,  // its links
        0,  //
    ];

    fn put(v: &mut Vec<u8>, at: u64, bytes: &[u8]) {
        let at = at as usize;
        if v.len() < at + bytes.len() {
            v.resize(at + bytes.len(), 0);
        }
        v[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn addr_bytes(v: u64) -> [u8; 8] {
        v.to_le_bytes()
    }

    /// The smallest file with a group in it: a superblock, the root group's
    /// object header, the tree and heap that header names, a symbol table node
    /// with one link in it, and the object that link points at.
    fn one_link_file() -> Vec<u8> {
        let mut f = Vec::new();
        put(&mut f, 0, b"\x89HDF\r\n\x1a\n");
        // Version 0, and eight bytes for both an address and a length.
        put(&mut f, 8, &[0, 0, 0, 0, 0, 8, 8, 0]);
        put(&mut f, 16, &4u16.to_le_bytes());
        put(&mut f, 18, &16u16.to_le_bytes());
        put(&mut f, 20, &0u32.to_le_bytes());
        put(&mut f, 24, &addr_bytes(0));
        put(&mut f, 32, &addr_bytes(u64::MAX));
        put(&mut f, 40, &addr_bytes(304));
        put(&mut f, 48, &addr_bytes(u64::MAX));
        // The root group entry: a group, with the tree and heap cached in it.
        put(&mut f, 56, &addr_bytes(0));
        put(&mut f, 64, &addr_bytes(ROOT_HEADER));
        put(&mut f, 72, &1u32.to_le_bytes());
        put(&mut f, 80, &addr_bytes(BTREE));
        put(&mut f, 88, &addr_bytes(HEAP));
        // The root group's object header: one message, the symbol table.
        put(&mut f, ROOT_HEADER, &[1, 0]);
        put(&mut f, ROOT_HEADER + 2, &1u16.to_le_bytes());
        put(&mut f, ROOT_HEADER + 4, &1u32.to_le_bytes());
        put(&mut f, ROOT_HEADER + 8, &24u32.to_le_bytes());
        put(&mut f, ROOT_HEADER + 16, &0x11u16.to_le_bytes());
        put(&mut f, ROOT_HEADER + 18, &16u16.to_le_bytes());
        put(&mut f, ROOT_HEADER + 24, &addr_bytes(BTREE));
        put(&mut f, ROOT_HEADER + 32, &addr_bytes(HEAP));
        // The tree: one entry, whose child is the node with the link in it.
        put(&mut f, BTREE, b"TREE");
        put(&mut f, BTREE + 4, &[0, 0]);
        put(&mut f, BTREE + 6, &1u16.to_le_bytes());
        put(&mut f, BTREE + 8, &addr_bytes(u64::MAX));
        put(&mut f, BTREE + 16, &addr_bytes(u64::MAX));
        put(&mut f, BTREE + 24, &addr_bytes(0));
        put(&mut f, BTREE + 32, &addr_bytes(SNOD));
        put(&mut f, BTREE + 40, &addr_bytes(14));
        // The heap, and the names in its data segment.
        put(&mut f, HEAP, b"HEAP");
        put(&mut f, HEAP + 8, &16u64.to_le_bytes());
        put(&mut f, HEAP + 16, &addr_bytes(0));
        put(&mut f, HEAP + 24, &addr_bytes(HEAP_DATA));
        put(&mut f, HEAP_DATA + 8, b"alpha\0");
        // The node, and its one link.
        put(&mut f, SNOD, b"SNOD");
        put(&mut f, SNOD + 4, &[1, 0]);
        put(&mut f, SNOD + 6, &1u16.to_le_bytes());
        put(&mut f, SNOD + 8, &8u64.to_le_bytes());
        put(&mut f, SNOD + 16, &addr_bytes(ALPHA_HEADER));
        // What the link points at: a dataset of two signed 32-bit numbers,
        // written in one run. Three messages say so, and the one that says
        // where the numbers are says nothing about what they are: the layout
        // message reads the datatype message beside it, which is the whole
        // point of the test below.
        put(&mut f, ALPHA_HEADER, &[1, 0]);
        put(&mut f, ALPHA_HEADER + 2, &3u16.to_le_bytes());
        put(&mut f, ALPHA_HEADER + 4, &1u32.to_le_bytes());
        put(&mut f, ALPHA_HEADER + 8, &80u32.to_le_bytes());
        // A dataspace of one dimension, two long.
        let m = ALPHA_HEADER + 16;
        put(&mut f, m, &1u16.to_le_bytes());
        put(&mut f, m + 2, &16u16.to_le_bytes());
        put(&mut f, m + 8, &[1, 1, 0, 0]);
        put(&mut f, m + 16, &2u64.to_le_bytes());
        // Fixed-point, signed, four bytes.
        let m = m + 24;
        put(&mut f, m, &3u16.to_le_bytes());
        put(&mut f, m + 2, &16u16.to_le_bytes());
        put(&mut f, m + 8, &[0x10, 0x08, 0, 0]);
        put(&mut f, m + 12, &4u32.to_le_bytes());
        put(&mut f, m + 16, &0u16.to_le_bytes());
        put(&mut f, m + 18, &32u16.to_le_bytes());
        // Laid out in one run, at the end of the file.
        let m = m + 24;
        put(&mut f, m, &8u16.to_le_bytes());
        put(&mut f, m + 2, &24u16.to_le_bytes());
        put(&mut f, m + 8, &[3, 1]);
        put(&mut f, m + 10, &addr_bytes(DATA));
        put(&mut f, m + 18, &8u64.to_le_bytes());
        put(&mut f, DATA, &(-7i32).to_le_bytes());
        put(&mut f, DATA + 4, &1000i32.to_le_bytes());
        f.resize(DATA as usize + 8, 0);
        f
    }

    fn read(f: &[u8], path: &[usize]) -> (String, Value) {
        let doc = Document::new(MemSource(f.to_vec()));
        let mut ev = Evaluator::new(hdf5());
        let node = ev.node(&doc, path).expect("node");
        (node.name, node.value)
    }

    #[test]
    fn a_link_is_named_by_the_heap_the_tree_hangs_under() {
        let f = one_link_file();
        let mut name = LINK.to_vec();
        name.extend_from_slice(&[5, 0]);
        let (_, value) = read(&f, &name);
        assert!(matches!(&value, Value::Str(s) if s == "alpha"), "{value:?}");
    }

    #[test]
    fn a_link_reaches_the_object_header_it_names() {
        let f = one_link_file();
        let mut object = LINK.to_vec();
        object.extend_from_slice(&[6, 0]);
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let node = ev.node(&doc, &object).expect("object header");
        assert_eq!(node.offset_bits / 8, ALPHA_HEADER);
    }

    /// The cursor reaches what an address placed. Everything in one of these
    /// files but the superblock is placed that way, so a hex view that could
    /// only land in what the root structure covers would say nothing about the
    /// whole file.
    #[test]
    fn the_cursor_lands_in_what_an_address_placed() {
        let f = one_link_file();
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());

        // The first of the two numbers the dataset holds.
        let at = ev.locate(&doc, DATA * 8).expect("locate");
        let node = ev.node(&doc, &at).expect("node");
        assert_eq!(node.value.as_int(), Some(-7));
        // The object header the root group's entry points at, which is 184
        // bytes past where the structure holding that entry ends.
        let at = ev.locate(&doc, ALPHA_HEADER * 8).expect("locate");
        assert_eq!(ev.node(&doc, &at).expect("node").name, "version");
        // A name in the heap's data segment, which is placed inside another
        // placed stretch: the narrower one is the answer.
        let at = ev.locate(&doc, (HEAP_DATA + 8) * 8).expect("locate");
        assert!(matches!(ev.node(&doc, &at).expect("node").value, Value::Str(ref s) if s == "alpha"));

        // The spans a view asks for run over the placed stretches as well as
        // over what the root covers.
        let spans = ev.spans(&doc, DATA * 8, DATA * 8 + 64, 8).expect("spans");
        assert!(spans.first().is_some_and(|s| s.offset_bits == DATA * 8 && !s.gap), "the dataset is not a span");
        let spans = ev.spans(&doc, BTREE * 8, BTREE * 8 + 64, 8).expect("spans");
        assert!(spans.first().is_some_and(|s| s.offset_bits == BTREE * 8), "the tree is not a span");

        // A byte nothing covers is a gap rather than an error: bytes tacked
        // on after everything the file places are named by nobody, and saying
        // so is the answer.
        let mut padded = one_link_file();
        padded.resize(400, 0);
        let doc = Document::new(MemSource(padded));
        let mut ev = Evaluator::new(hdf5());
        assert!(ev.locate(&doc, 396 * 8).expect("locate").is_empty());
        let tail = ev.spans(&doc, 396 * 8, 400 * 8, 8).expect("spans");
        assert!(tail.first().is_some_and(|s| s.gap), "the tail is not a gap");
    }

    /// The layout message says where a dataset's bytes are and never what they
    /// are: that is in the datatype message beside it, which is why the run is
    /// read as signed 32-bit numbers rather than as bytes.
    #[test]
    fn a_dataset_reads_as_the_elements_its_datatype_declares() {
        let f = one_link_file();
        // The layout message of the object the link points at, down to the
        // first of the numbers it places.
        let mut first = LINK.to_vec();
        first.extend_from_slice(&[6, 0, 6, 2, 4, 1, 1, 2, 0, 2, 0]);
        let (_, value) = read(&f, &first);
        assert_eq!(value.as_int(), Some(-7));
        let mut second = first.clone();
        second.pop();
        second.push(1);
        assert_eq!(read(&f, &second).1.as_int(), Some(1000));
    }

    /// A pointer that says "nothing here" is not followed. Every optional part
    /// of the format writes all ones for it, and reading address 2^64-1 would
    /// be an error on a field the file deliberately left empty.
    #[test]
    fn the_undefined_address_points_at_nothing() {
        let mut f = one_link_file();
        put(&mut f, SNOD + 16, &addr_bytes(u64::MAX));
        let mut object = LINK.to_vec();
        object.push(6);
        let (_, value) = read(&f, &object);
        assert!(matches!(value, Value::Bytes { len: 0, .. }), "{value:?}");
    }

    /// `sniff` over a file that is exactly these bytes.
    fn sniffed(head: &[u8]) -> Option<&'static str> {
        crate::formats::sniff(head, head.len() as u64)
    }

    /// The signature is the only thing that identifies the container, and
    /// nothing in it says what the file holds.
    #[test]
    fn the_signature_claims_the_file() {
        assert_eq!(sniffed(b"\x89HDF\r\n\x1a\n\0\0"), Some("hdf5"));
    }
}

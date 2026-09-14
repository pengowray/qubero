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
//! Superblock versions 0 to 3, the object header of version 1 and its
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
//! ## Where a chunk is
//!
//! A data layout message of version 1, 2 or 3 has one answer for a chunked
//! dataset: a version 1 b-tree, keyed by where the chunk sits in the dataset.
//! Versions 4 and 5, which arrived with HDF5 1.10 and are what a file written
//! to the latest library version gets, name one of five others instead, and
//! which one follows from the dataset's shape rather than being asked for.
//!
//! All five are read. A single chunk index is the address of the one chunk
//! and nothing else. An implicit index is no index: the chunks are written one
//! after another in the order they are counted. A fixed array is one entry per
//! chunk, in that same order, for a dataset whose dimensions cannot grow; an
//! extensible array is the same for one dimension that can, with the first
//! handful of entries in the index block itself; and more than one dimension
//! that can grow is a version 2 b-tree, which was already read here for a
//! group's links. The entries of both arrays are followed to the chunks, and
//! an unfiltered chunk reads as elements, since the chunk dimensions
//! multiplied together are how many bytes one chunk comes to. The four that
//! are not a b-tree live in [`hdf5_index`](super::hdf5_index), since their
//! shape is arithmetic over the message rather than structure in the file.
//!
//! That last multiplication is why a version 4 message cannot be read as a
//! version 3 one with the numbers moved about. Version 3 writes the chunk
//! dimensions in four bytes each and the size of an element beside them;
//! version 4 writes them in as many bytes as the largest of them needs, one
//! to eight, and the last dimension is the element size rather than a
//! dimension of the chunk.
//!
//! Three of the five lay their chunks out by counting, and none of the three
//! writes the counting down. An implicit index's run is as many chunks as the
//! largest extent the dataspace allows rounds up to, a dimension at a time. A
//! fixed array with more entries than a page holds keeps them in pages of a
//! power of two, behind a bitmap of which pages were ever written. An
//! extensible array keeps a few entries in its index block, then the
//! addresses of data blocks, then the addresses of secondary blocks that hold
//! more data block addresses, the blocks doubling as the array grows and
//! paged once they are past a page; how many addresses of each kind the index
//! block has is a base-2 logarithm of numbers in the header away. All of it is
//! worked out here the way the library works it out, every block and page of
//! both arrays is placed with its checksum, and every chunk they name is
//! reached.
//!
//! ## What is not
//!
//! Only 8-byte offsets and lengths. The superblock says what size it uses and
//! every writer in practice says 8; a file that says 4 is read wrong from the
//! root group entry onwards rather than refused, which the size fields
//! themselves make plain.
//!
//! Every address counts from the base address the superblock names, which is
//! where this copy of the file begins: nought for a file that is only HDF5,
//! and the size of the user block for one written into the middle of
//! something else. A signature may sit at 512, 1024 or any later power of two
//! with a user block in front of it, and a MATLAB 7.3 file is exactly that,
//! 128 bytes of MAT header inside a 512-byte block. Both are read, by counting
//! from an [`Anchor::Origin`](crate::template::Anchor::Origin) rather than
//! from the front of the file.
//!
//! A group with more links than fit as messages keeps them in a fractal heap:
//! a header, a table of blocks whose rows double in size, and the links
//! written one after another inside those blocks. That is read, and the names
//! come out, however far the heap has grown. Once a row's blocks would be
//! bigger than the largest direct block, that row holds tables of its own
//! instead, and those are followed too. Which rows are which, and how many
//! rows a table below the root has, are written nowhere: they follow from the
//! block sizes in the header by a base-two logarithm, and are worked out that
//! way. Where the links stop inside a block is not written anywhere either, so
//! a run of zeros or a stretch too short to hold a link ends the run; a block
//! whose free space holds an older link reads that link again, wrongly, and
//! the name says so.
//!
//! The version 2 b-tree that indexes those links by name is read all the way
//! down, and its records are left as bytes: a record is a hash and a heap id,
//! which is an offset into the heap rather than an address in the file, and the
//! links themselves are already read from the blocks. A node below the root is
//! reached through the pointer above it, which is also what says how many
//! records the node holds. How wide that count is, and the total beside it,
//! is worked out a level at a time from the node size and the record size, the
//! way the library works it out when it opens the tree.
//!
//! What is still not read: huge and tiny heap objects, and the free-space
//! managers.
//!
//! The chunk indexes' checksums are placed and not checked. A page of array
//! entries that was set aside and never written keeps its bytes: nothing wrote
//! entries there, and read as entries they would be addresses of whatever the
//! file held in that room before. An implicit index's run includes the chunks
//! a dataset has room to grow into, since they were written when it was made,
//! so it can show more chunks than the dataset's current extent covers.
//!
//! A virtual dataset is read as far as the global heap collection holding its
//! mapping. What is in that object, the selections in this dataset and the
//! names of the files and datasets they come from, keeps its bytes.
//!
//! ## The elements
//!
//! A dataset's bytes are placed where its layout message says: one run for a
//! contiguous dataset, one per chunk for a chunked one, in the header itself
//! for a compact one. They read as the dataset's elements, and what an element
//! is comes from the datatype message beside the layout one: integers of any
//! width and either sign, floats, fixed-width strings, the note a
//! variable-length element leaves, compounds, enumerations and arrays. A
//! datatype this does not take apart (a time, a bit field, an opaque blob, a
//! reference) is one element of the right size and its own bytes.
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
//! collection and an index into it. All three are shown, the collection is
//! placed, and so is the object itself: the collection is searched for the
//! object carrying that index, and `Expr::StartOf` turns where that object was
//! found into an address to place a field at. The objects have no fixed size,
//! so the fifteenth is wherever the fourteen before it ended and the search is
//! a walk; what one walk learned is kept by the stretch of bytes it covered,
//! so a column of two thousand strings is one walk and not two thousand. The
//! bytes are counted in the collection and not in the column, since several
//! notes can point at one object.
//!
//! The note is the same for a string and for a sequence of anything else, and
//! the low four bits of the datatype's class bits say which. A string's object
//! reads as its text. A sequence's reads as `length` elements of the datatype
//! inside the variable-length one, which is a datatype message of its own, so
//! a sequence of 32-bit integers is a run of `i32` over the heap object's
//! bytes and a sequence of compounds is a run of records. An empty sequence
//! points at nothing, and HDF5 2.0 writes it with address nought rather than
//! the undefined address, so both mean nothing here.
//!
//! An attribute's value reads as elements too, by the datatype written inside
//! the attribute rather than beside it. That is the one thing the IR could not
//! say: `Expr::Ref` names a field beside this one and stops there, so
//! `Expr::Within` was added to name a field and then a path down into it.
//! `shape` on an `.h5ad` group is two numbers because of it.
//!
//! ## Compounds
//!
//! A compound element is a row of named members, each at an offset inside the
//! element and each of a type of its own, and none of that is in the element:
//! the datatype message lists the members, a name, an offset and a whole
//! datatype message again for each. So every member of the message is read,
//! in all three shapes its versions give it (a name padded to eight bytes or
//! not, an offset in four bytes or in as few as the compound's size needs, the
//! dimensions only version 1 has), and each element is a `Record` whose
//! `values` are a [`Ty::Gather`](crate::template::Ty::Gather) over those member
//! records: one value per record, at the offset the record gives, read by the
//! record's datatype through `Expr::Placer` and labelled with its name,
//! `[2] label`. The records need not be in the order their bytes are, and
//! room no member covers is a gap. HDF5 2.0 writes version 5 for a file bound
//! to the latest library, and lays a compound out in it as version 3 does.
//!
//! Every element walks the member records again, so the list the walk starts
//! from has to be a field in scope. An attribute has its datatype there
//! already; a dataset places its datatype message a second time beside its
//! run, as `datatype`, and a chunked layout keeps one above its chunks. Only
//! classes whose elements ask their datatype something (a compound, an
//! enumeration, an array, a variable-length sequence) get the copy, since it
//! is a search back through the messages to find.
//!
//! A member may be a compound itself. That member places its own datatype and
//! walks its own members, and it is followed as deep as the file nests it:
//! each level is inside the bytes of the one above, so the nesting cannot
//! loop. An array member reads as its elements, and so does an enumeration as
//! the integers it is based on; those two, and what a sequence holds, are
//! followed two levels down (an array of arrays, an enumeration inside an
//! array) before what is further in keeps its bytes, because each level is
//! written out again in the template for every class that could be inside it.

use super::hdf5_index::{chunked_v4, extensible_array, fixed_array};
use crate::template::{
    Anchor, Encoding,
    Endian::{Big, Little},
    Expr as E, Part, Step, StrLen, Template, Time, Ty as T, Until,
};

/// The address a file writes for "there is nothing here": all ones, in
/// however many bytes an address takes.
const UNDEFINED: i128 = u64::MAX as i128;

pub fn hdf5() -> Template {
    let part = hdf5_part();
    Template::new("hdf5", part.root.clone()).with_part(&part)
}

/// The whole of an HDF5 file and the names it refers to, for a format that
/// carries one inside it. Wrap the root in [`T::origin`] where it does not
/// start at the front of the file and every address in it lands in the right
/// place; on its own it needs no wrapper, since an origin anchor with no
/// origin around it counts from nought.
pub fn hdf5_part() -> Part {
    Part::new(with_user_block())
        .with_type("ObjectHeader", object_header())
        .with_type("Message", message())
        .with_type("MessageV2", message_v2())
        .with_type("Node", node())
        .with_type("LocalHeap", local_heap())
        .with_type("Datatype", datatype())
        .with_type("Dataspace", dataspace())
        .with_type("GlobalHeap", global_heap())
        .with_type("FractalHeap", fractal_heap())
        .with_type("HeapDirectBlock", heap_direct_block())
        .with_type("HeapIndirectBlock", heap_indirect_block())
        .with_type("BTree2", btree2())
        .with_type("BTree2Node", btree2_node())
        .with_type("FixedArray", fixed_array())
        .with_type("ExtensibleArray", extensible_array())
        .with_type(ELEMENTS_BESIDE, element_type(Described::Beside))
        .with_type(ELEMENTS_INSIDE, element_type(Described::Inside))
        .with_type(BASE_VALUE, value_of(&Reach::top(Described::Inside).base(), 1))
        .with_type(BASE_SEQUENCE, base_sequence(&Reach::top(Described::Inside).base(), 1))
        .with_type(MEMBER_BASE_VALUE, value_of(&Reach::member().base(), 1))
        .with_type(MEMBER_BASE_SEQUENCE, base_sequence(&Reach::member().base(), 1))
        .with_type("Member", member())
        .with_type("EnumName", enum_name())
        .with_type(MEMBER_VALUE, member_value())
        .with_type(MEMBER_ELEMENT, value_of(&Reach::member(), 0))
}

/// The eight bytes every HDF5 superblock opens with.
pub const SIGNATURE: &[u8] = b"\x89HDF\r\n\x1a\n";

/// The same, as one big-endian word, so that where it is can be asked before
/// anything is placed.
const SIGNATURE_WORD: i128 = 0x8948_4446_0d0a_1a0a;

/// A file that opens with the signature, and one that keeps a user block in
/// front of it. The second is the same layout begun further in, and every
/// address inside it counts from there rather than from the front of the
/// file: that is what the origin says.
///
/// A file with no block is left exactly as it was. An origin around it would
/// change nothing, since an origin anchor with none around it counts from
/// nought, and not wrapping it keeps the tree the shape every reading of an
/// ordinary file has had.
fn with_user_block() -> T {
    T::switch(
        E::peek(64, Big),
        vec![(SIGNATURE_WORD, root())],
        T::structure(
            "HDF5",
            vec![
                ("user_block", T::bytes(E::to_bytes(SIGNATURE))),
                ("file", T::origin(root())),
            ],
        ),
    )
}

fn root() -> T {
    T::structure(
        "HDF5",
        vec![
            ("signature", T::magic(SIGNATURE)),
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
/// answer: the root group's header is a version 2 one, and a group with many
/// links keeps them in a fractal heap.
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
///
/// All ones is the undefined address, the file's way of writing "nothing
/// here", and it is named so a chip says that rather than spelling out
/// 18446744073709551615. Every other value shows as the number it is.
pub(super) fn addr() -> T {
    T::enumeration("Address", T::u64(Little), &[(UNDEFINED, "undefined")])
}

/// A length, which the superblock sizes separately from an address and which
/// no writer sizes differently.
pub(super) fn length() -> T {
    T::u64(Little)
}

/// What `field` points at, or nothing when it holds the undefined address.
///
/// Counted from where this copy of the file begins rather than from the front
/// of whatever holds it. On its own the two are the same place; behind a user
/// block they are not, and a MATLAB 7.3 file is this format written 512 bytes
/// in. Not from the nearest window either: a message body is sized, and an
/// address written inside one still counts from the superblock. See
/// [`Anchor::Origin`](crate::template::Anchor::Origin).
pub(super) fn at_address(field: &str, inner: T) -> T {
    T::switch(
        E::field(field),
        vec![(UNDEFINED, T::bytes(E::lit(0)))],
        T::at_origin(E::field(field), inner),
    )
}

/// One bit of a flags field as a number, since there is no bitwise operator:
/// shift it down and take away everything above it.
pub(super) fn bit(field: &str, k: u32) -> E {
    let below = E::field(field).div(E::lit(1i128 << k));
    below.clone().sub(E::field(field).div(E::lit(1i128 << (k + 1))).mul(E::lit(2)))
}

/// A field that is there when a flag is set and takes no bytes when it is not.
pub(super) fn when(flag: E, ty: T) -> T {
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
            T::at_origin(
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
            T::at_origin(
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
                    )
                    .field_times(
                        &["access_time", "modification_time", "change_time", "birth_time"],
                        Time::unix(),
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
            ("version", T::UInt { bits: 4, endian: Big }),
            (
                "class",
                T::enumeration(
                    "DatatypeClass",
                    T::UInt { bits: 4, endian: Big },
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
                        (11, "complex"),
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
                        (2, T::inline_structure("Time", vec![("bit_precision", T::u16(Little))])),
                        // A string says everything it has to say in the class
                        // bits: padding in the low nibble, character set in
                        // the one above it.
                        (3, T::bytes(E::lit(0))),
                        (
                            4,
                            T::inline_structure(
                                "BitField",
                                vec![("bit_offset", T::u16(Little)), ("bit_precision", T::u16(Little))],
                            ),
                        ),
                        // The tag is as long as the low byte of the class bits
                        // says, nul-padded to a multiple of eight.
                        (
                            5,
                            T::structure(
                                "Opaque",
                                vec![("tag", T::text(StrLen::Padded { size: E::field("bit_field"), pad: 0 }, Encoding::Ascii))],
                            ),
                        ),
                        (6, compound_type()),
                        // Which kind of reference is in the class bits, and
                        // nothing follows them.
                        (7, T::bytes(E::lit(0))),
                        (8, enumeration_type()),
                        (9, T::structure("VariableLength", vec![("base", T::Named("Datatype".into()))])),
                        (10, array_type()),
                        (11, T::structure("Complex", vec![("base", T::Named("Datatype".into()))])),
                    ],
                    // Every class the format has is above. What is left runs to
                    // the end of the message, which is right for a datatype
                    // message of its own and wrong for one inside another: a
                    // member of an unknown class takes the members after it
                    // with it.
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}

/// How many members a compound or an enumeration has, which is the low sixteen
/// bits of the class bits.
fn member_count() -> E {
    E::field("bit_field").add(E::field("bit_field_rest").and(E::lit(255)).mul(E::lit(256)))
}

/// A name inside a datatype message: nul-terminated, and in versions 1 and 2
/// padded with more nuls to a multiple of eight bytes, the terminator counted.
/// Version 3 and later stopped padding.
fn padded_name(fields: &mut Vec<(&'static str, T)>, version: u8) {
    fields.push(("name", T::text(StrLen::Terminated { end: 0, or_end: false }, Encoding::Utf8)));
    if version < 3 {
        fields.push(("name_padding", T::bytes(E::size_of("name").pad_to(8))));
    }
}

/// The members of a compound datatype: a name, where in the element the
/// member's bytes start, and the member's own datatype, which is a whole
/// datatype message again and may be a compound itself.
///
/// The three versions write a member three ways. Version 1 pads the name to
/// eight bytes, writes the offset in four, and then a dimensionality and four
/// dimension sizes, which is how a member was made an array before there was
/// an array class. Version 2 drops those, since an array member is an array
/// datatype now. Version 3 stops padding the name and writes the offset in as
/// few bytes as the compound's size needs: one for a compound under 256 bytes.
/// A file written under HDF5 2.0's latest bound says version 5, and writes a
/// compound the way version 3 does.
fn compound_type() -> T {
    T::structure(
        "Compound",
        vec![(
            "members",
            T::array(T::Named("Member".into()), member_count()),
        )],
    )
}

/// One member, in whichever of the three shapes the compound's version gives
/// it. A named type so that the list of them says what it is a list of.
fn member() -> T {
    T::switch(E::field("version"), vec![(1, member_v(1)), (2, member_v(2))], member_v(3))
}

fn member_v(version: u8) -> T {
    let mut fields = Vec::new();
    padded_name(&mut fields, version);
    if version < 3 {
        fields.push(("offset", T::u32(Little)));
    } else {
        // The fewest whole bytes that can write the compound's size, which is
        // what the library sizes the offset by.
        let width = E::field("size").log2().div(E::lit(8)).add(E::lit(1));
        fields.push(("offset", T::uint_expr(width.mul(E::lit(8)), Little)));
    }
    if version == 1 {
        fields.extend(vec![
            ("dimensionality", T::u8()),
            ("reserved", T::bytes(E::lit(3))),
            ("permutation", T::u32(Little)),
            ("reserved_2", T::u32(Little)),
            // Always four, however many are used.
            ("dimensions", T::array(T::u32(Little), E::lit(4))),
            (
                "element_count",
                T::when(E::lit(0).less_than(E::field("dimensionality")), T::computed(used_dimensions())),
            ),
        ]);
    }
    fields.push(("datatype", T::Named("Datatype".into())));
    T::structure_named("Member", "name", "", fields)
}

/// The first `dimensionality` of a version 1 member's four dimension sizes,
/// multiplied together.
fn used_dimensions() -> E {
    (0..4).fold(E::lit(1), |product, k| {
        let used = E::lit(k).less_than(E::field("dimensionality"));
        product.mul(E::cond(used, E::elem("dimensions", E::lit(k)), E::lit(1)))
    })
}

/// An enumeration: the integer type its values are written in, then every
/// name, then every value in the same order.
fn enumeration_type() -> T {
    T::structure(
        "Enumeration",
        vec![
            ("base", T::Named("Datatype".into())),
            (
                "names",
                T::array(T::Named("EnumName".into()), member_count()),
            ),
            (
                "values",
                run_of(&Reach::local(&["base"]), member_count(), E::within(&["base", "size"]), MAX_NESTING),
            ),
        ],
    )
    .field_elem_named_from("values", E::elem_field("names", E::idx(), &["name"]))
}

/// One name of an enumeration, padded the way a compound member's name is.
fn enum_name() -> T {
    let name = |version: u8| {
        let mut fields = Vec::new();
        padded_name(&mut fields, version);
        T::structure_named("EnumName", "name", "", fields)
    };
    T::switch(E::field("version"), vec![(1, name(1)), (2, name(2))], name(3))
}

/// An array: dimensions, and the datatype of one element of it. Versions 1 and
/// 2 keep three reserved bytes and a permutation the library never used; 3
/// dropped both.
fn array_type() -> T {
    let before_3 = || E::field("version").less_than(E::lit(3));
    T::structure(
        "Array",
        vec![
            ("dimensionality", T::u8()),
            ("reserved", T::when(before_3(), T::bytes(E::lit(3)))),
            ("dimensions", T::array(T::u32(Little), E::field("dimensionality"))),
            ("permutation", T::when(before_3(), T::array(T::u32(Little), E::field("dimensionality")))),
            ("element_count", T::computed(E::product_of("dimensions"))),
            ("base", T::Named("Datatype".into())),
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
pub(super) enum Described {
    /// By a datatype message among the same object's messages.
    Beside,
    /// By a field called `datatype` in a structure around the elements: the
    /// datatype an attribute writes before its value, or the second reading of
    /// the message a chunked layout keeps above its chunks (see
    /// `hdf5_index::datatype_copy`).
    Inside,
}

impl Described {
    pub(super) fn part(self, name: &str) -> E {
        match self {
            Described::Beside => E::sibling(&["body", name]),
            Described::Inside => E::within(&["datatype", name]),
        }
    }

    /// Where the datatype message beside this one starts, as an address of
    /// this format, so that it can be placed again somewhere an expression
    /// can name it.
    ///
    /// Found by a field only a datatype has, two bytes in, rather than by the
    /// message's `body`: every message has a body, and the one just before a
    /// layout message is as likely to be a fill value.
    pub(super) fn beside_address() -> E {
        E::start_of(E::sibling(&["body", "bit_field_rest"])).sub(E::lit(2))
    }
}

/// Whether a run's elements are of a class whose elements ask their datatype
/// something once per element: a compound's members, an enumeration's or an
/// array's base type, a sequence's element type. Those go through a copy of
/// the datatype in scope, and a run of any other class places no copy.
fn needs_datatype(by: Described) -> E {
    let class = || by.part("class");
    let sequence = by.part("bit_field").and(E::lit(15)).equal_to(E::lit(0));
    E::cond(
        class().equal_to(E::lit(9)),
        sequence,
        class().equal_to(E::lit(6)).either(class().equal_to(E::lit(8))).either(class().equal_to(E::lit(10))),
    )
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

/// How deep an array, an enumeration or a variable-length sequence is followed
/// into the type of what it holds, before what is further in keeps its bytes.
///
/// A bound on the template rather than on the file. Each of those three is
/// written out again for the type inside it when the template is built, so
/// every level multiplies what the template holds by three; two levels reach
/// an array of arrays, a sequence of arrays and an enumeration inside either.
/// A compound is not counted here: a member is a named type, followed only
/// when it is opened, and each nested compound is inside the bytes of the one
/// around it, so the file bounds how deep that goes.
const MAX_NESTING: u32 = 2;

/// What a gathered member of a compound is read as: one value of its type,
/// or, for a version 1 member with dimensions of its own, an array of them.
const MEMBER_VALUE: &str = "MemberValue";
/// One element of that: the value itself, or one of the array's.
const MEMBER_ELEMENT: &str = "MemberElement";

/// How an expression reaches the datatype describing the bytes it is about.
///
/// At the top of a run of elements that is [`Described`]: a message beside
/// the layout, or a `datatype` field in a structure around the elements. Below
/// that, what an element holds is described by a datatype inside that one: a
/// member's, an array's base, a sequence's base. So a reach is a path from a
/// field called `datatype` down to the datatype wanted, asked either where the
/// value is or, for the member of a compound, of the member record that placed
/// the value (see [`Expr::Placer`](crate::template::Expr::Placer)).
#[derive(Clone)]
struct Reach {
    path: Vec<&'static str>,
    /// Asked of the member record that placed this value.
    placer: bool,
    /// The three numbers asked once per run, `class`, `bit_field` and `size`,
    /// come from the message beside the layout rather than from a `datatype`
    /// in scope. Only the top of a dataset's run is read this way, and only
    /// those three: anything asked once per element goes through the copy of
    /// the message the run keeps, because a search back through the messages
    /// from inside a column of a million elements passes every element before
    /// the one asking.
    beside: bool,
}

impl Reach {
    fn top(by: Described) -> Reach {
        Reach { path: vec!["datatype"], placer: false, beside: matches!(by, Described::Beside) }
    }

    /// A datatype reached down a path from a field in scope.
    fn local(path: &[&'static str]) -> Reach {
        Reach { path: path.to_vec(), placer: false, beside: false }
    }

    /// The datatype of the member whose record placed this value.
    fn member() -> Reach {
        Reach { path: vec!["datatype"], placer: true, beside: false }
    }

    /// A number inside the datatype, at `rest` below it.
    fn at(&self, rest: &[&'static str]) -> E {
        if self.beside && rest.len() == 1 {
            return Described::Beside.part(rest[0]);
        }
        let mut path = self.path.clone();
        path.extend_from_slice(rest);
        let e = E::within(&path);
        if self.placer { E::placer(e) } else { e }
    }

    /// The same datatype, asked from inside one element rather than once for
    /// the run.
    fn per_element(&self) -> Reach {
        Reach { beside: false, ..self.clone() }
    }

    /// The datatype of what this one holds: an array's element, a sequence's
    /// element, the integer an enumeration is written in.
    fn base(&self) -> Reach {
        let mut path = self.path.clone();
        path.extend_from_slice(&["properties", "base"]);
        Reach { path, placer: self.placer, beside: false }
    }

    /// Where the datatype starts, as an address, so it can be placed again.
    fn address(&self) -> E {
        let e = E::start_of(E::within(&self.path));
        if self.placer { E::placer(e) } else { e }
    }

    /// Whether the datatype is the `datatype` field in scope, which a record
    /// can walk its members from without placing anything.
    fn in_scope(&self) -> bool {
        !self.placer && self.path == ["datatype"]
    }
}

/// Which of the numeric types an integer or float datatype is, as one number:
/// class, byte order, width and sign together, so a single switch asks about
/// all four instead of four nested switches saying the same thing at four
/// times the length. Keyed as `class * 1000 + big_endian * 500 + size * 2 +
/// signed`.
fn numeric(reach: &Reach) -> (E, Vec<(i128, T)>) {
    let bits = reach.at(&["bit_field"]);
    let key = reach
        .at(&["class"])
        .mul(E::lit(1000))
        .add(bits.clone().bit(0).mul(E::lit(500)))
        .add(reach.at(&["size"]).mul(E::lit(2)))
        .add(bits.bit(3));
    let types = vec![
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
    (key, types)
}

/// What one element of a dataset is, read from the datatype message that sits
/// before the layout one among the object's messages. A datatype this does not
/// take apart leaves its elements as their own bytes, which is still one row
/// per element and still the right size.
///
/// A named type, one for each of the two places a datatype can be, because a
/// run of elements is read in half a dozen places and every class of element
/// is a few hundred lines of template once written out.
fn element_type(by: Described) -> T {
    let width = E::field("element_size");
    run_of(&Reach::top(by), E::field("run_bytes").div(width.clone()), width, 0)
}

const ELEMENTS_BESIDE: &str = "ElementsBeside";
const ELEMENTS_INSIDE: &str = "ElementsInside";

/// `count` elements of the datatype `reach` reaches, each `width` bytes.
///
/// The switch is outside the arrays rather than inside one: an array of a type
/// chosen per element has no stride, and a stride is what lets the cursor land
/// in the middle of thirteen million numbers without reading the ones before
/// it. Every element here is one width, so a type that is not a number is
/// sized to it, which gives the array its stride back.
fn run_of(reach: &Reach, count: E, width: E, depth: u32) -> T {
    let of = |t: T| T::array(t, count.clone()).counted_as("elements");
    let (key, numeric) = numeric(reach);
    let numeric: Vec<(i128, T)> = numeric.into_iter().map(|(k, t)| (k, of(t))).collect();
    let opaque = of(T::sized(width.clone(), T::bytes(width.clone())));
    let one = reach.per_element();
    let mut cases = vec![
        (0, T::switch(key.clone(), numeric.clone(), opaque.clone())),
        (1, T::switch(key, numeric, opaque.clone())),
        // A string of a fixed width, which is what a column of names is.
        // Sized as well as padded, so the run has a stride.
        (3, of(T::sized(width.clone(), T::text(StrLen::Padded { size: width.clone(), pad: 0 }, Encoding::Utf8)))),
        // A variable-length element is not the thing but a note saying where
        // the thing is: how long it is, which global heap collection holds it,
        // and which object in that collection it is. The note carries the
        // object too, found by searching the collection for that index.
        // Sixteen bytes is the shape a file with eight-byte addresses writes;
        // anything else is left as its bytes.
        //
        // Class nine covers two things and the low four bits of the class bit
        // field say which: a string, or a sequence of some other type. Asked
        // here rather than inside the note, so that a column of a million
        // strings asks it once: reaching a datatype message two levels away is
        // what `element_size_field` exists to avoid doing per element.
        (9, {
            let string = reach.at(&["bit_field"]).and(E::lit(15));
            let note = |kind: Vlen| {
                T::switch(width.clone(), vec![(16, of(T::sized(width.clone(), vlen_reference(kind))))], opaque.clone())
            };
            let sequence = if depth < MAX_NESTING { Vlen::Sequence(one.base(), depth + 1) } else { Vlen::Bytes };
            T::switch(string, vec![(1, note(Vlen::Text))], note(sequence))
        }),
    ];
    if depth < MAX_NESTING {
        // A compound: every element is a record of its members, each at its
        // own offset inside the element.
        cases.push((6, of(T::sized(width.clone(), record(&one)))));
        // An enumeration is written as the integer it is based on.
        cases.push((8, of(T::sized(width.clone(), base_value(&one, depth)))));
        cases.push((
            10,
            of(T::sized(
                width,
                T::array(base_value(&one, depth), one.at(&["properties", "element_count"])),
            )),
        ));
    }
    T::switch(reach.at(&["class"]), cases, opaque)
}

/// One value of the datatype inside the one `reach` reaches: an array's
/// element, the integer an enumeration is written in.
///
/// One level down from a run's own datatype, and from a member's, the path is
/// the same wherever the run or the member is, so those two are named types
/// written out once rather than once per class of element that holds one.
fn base_value(reach: &Reach, depth: u32) -> T {
    match depth {
        0 => T::Named(if reach.placer { MEMBER_BASE_VALUE } else { BASE_VALUE }.into()),
        _ => value_of(&reach.base(), depth + 1),
    }
}

/// The elements of a variable-length sequence, `length` of them, of the
/// datatype `base` reaches.
fn base_sequence(base: &Reach, depth: u32) -> T {
    run_of(base, E::field("length"), base.at(&["size"]), depth)
}

/// The names those named types go by. What a reader sees of them is the type
/// column of an array, `ArrayElement[]`, and of a sequence's object,
/// `at → SequenceElements`: an enumeration resolves to the integer it is
/// written in before anything shows its type. `Member` in front is the same
/// thing reached from inside a compound's member.
const BASE_VALUE: &str = "ArrayElement";
const BASE_SEQUENCE: &str = "SequenceElements";
const MEMBER_BASE_VALUE: &str = "MemberArrayElement";
const MEMBER_BASE_SEQUENCE: &str = "MemberSequenceElements";

/// One value of the datatype `reach` reaches, where the value is on its own
/// rather than one of a run: a member of a compound, an element of an array
/// member. The same classes [`run_of`] reads, one at a time.
fn value_of(reach: &Reach, depth: u32) -> T {
    let size = reach.at(&["size"]);
    let opaque = T::bytes(size.clone());
    let (key, numeric) = numeric(reach);
    let mut cases = vec![
        (0, T::switch(key.clone(), numeric.clone(), opaque.clone())),
        (1, T::switch(key, numeric, opaque.clone())),
        (3, T::sized(size.clone(), T::text(StrLen::Padded { size: size.clone(), pad: 0 }, Encoding::Utf8))),
    ];
    if depth < MAX_NESTING {
        // Sized, because a record counts its members' offsets from the start
        // of the nearest sized window, and that has to be this value rather
        // than the element around it.
        cases.push((6, T::sized(size.clone(), record(reach))));
        cases.push((8, base_value(reach, depth)));
        cases.push((
            9,
            T::switch(
                size.clone(),
                vec![(
                    16,
                    T::sized(
                        size.clone(),
                        T::switch(
                            reach.at(&["bit_field"]).and(E::lit(15)),
                            vec![(1, vlen_reference(Vlen::Text))],
                            vlen_reference(Vlen::Sequence(reach.base(), depth + 1)),
                        ),
                    ),
                )],
                opaque.clone(),
            ),
        ));
        cases.push((
            10,
            T::array(base_value(reach, depth), reach.at(&["properties", "element_count"])),
        ));
    }
    T::switch(reach.at(&["class"]), cases, opaque)
}

/// One element of a compound: its members, each read by its own datatype at
/// the offset the datatype message gives it, and labelled with its name.
///
/// The members are a [`Ty::Gather`](crate::template::Ty::Gather) over the
/// member records in the datatype message. Nothing in an element says where a
/// member starts or what it is; the records do, and they are not in order: a
/// member may be listed before one that sits in front of it, and there may be
/// room between two that no member covers. Each value asks its record what it
/// is with `placer`, and the record's name is the label, `[2] label`.
///
/// Where the datatype is already a field in scope, the walk starts from it.
/// Anywhere else, a member that is a compound or an array of them, it is
/// placed again here first, as a second reading of the message's bytes, since
/// a walk has to start from a field.
///
/// A version 1 member may be an array of its type without being an array
/// datatype, by the dimensions in its record, so every member's value looks.
fn record(reach: &Reach) -> T {
    let own = !reach.in_scope();
    let mut fields = Vec::new();
    if own {
        fields.push(("datatype", T::at_origin(reach.address(), T::Named("Datatype".into()))));
    }
    fields.push((
        "values",
        T::gather(
            vec![Step::field("datatype"), Step::field("properties"), Step::field("members"), Step::each()],
            E::field("offset"),
            Anchor::Window,
            E::lit(0),
            T::Named(MEMBER_VALUE.into()),
        ),
    ));
    let record = T::structure("Record", fields)
        .field_elem_named_from("values", E::placer(E::field("name")))
        .counted_as("record");
    if own { record.field_aside("datatype").machinery(&["datatype"]) } else { record }
}

/// A member's value: one of its type, or, for a member of a version 1
/// compound whose record gives it dimensions, as many as those multiply to.
///
/// The compound's version is asked first and the record's dimensionality only
/// when it is 1, since a record of any later version has no such field and
/// the search for one would climb out of the record looking.
fn member_value() -> T {
    let element = || T::Named(MEMBER_ELEMENT.into());
    let dimensioned = E::within(&["datatype", "version"])
        .equal_to(E::lit(1))
        .both(E::lit(0).less_than(E::placer(E::field("dimensionality"))));
    T::switch(
        dimensioned,
        vec![(0, element())],
        T::array(element(), E::placer(E::field("element_count"))),
    )
}

/// What a variable-length element's object is read as.
enum Vlen {
    /// The text of a string.
    Text,
    /// `length` elements of the datatype reached, at this depth.
    Sequence(Reach, u32),
    /// Bytes, past [`MAX_NESTING`].
    Bytes,
}

/// The note a variable-length element leaves in place of its contents: how
/// long it is, which global heap collection holds it, which object in that
/// collection it is, and the object itself.
///
/// Which object is which is a matter of reading the indices, since the objects
/// are of no fixed size and the fifteenth is wherever the fourteen before it
/// ended. So `object` searches the collection for the index written here and
/// is then placed at wherever that object's bytes begin, which is what
/// [`E::start_of`] is for.
///
/// The bytes are counted in the collection and not here. This field is a
/// second reading of them: several notes can point at one object, and an
/// object nothing points at is still part of the heap. See [`Field::aside`].
///
/// A string's `length` counts bytes and a sequence's counts elements, and the
/// object is read as the one or the other. Which it is, is the low four bits
/// of the datatype's class bit field, asked where the column is typed rather
/// than once per element. What a sequence's elements are is the datatype
/// inside the variable-length one, and that is asked per element, through the
/// copy of the datatype message the run keeps.
fn vlen_reference(kind: Vlen) -> T {
    let found = |field: &[&str]| {
        E::tagged_in_by(E::within(&["collection", "objects"]), &["object_index"], E::field("object_index"), field)
    };
    let contents = match kind {
        Vlen::Text => T::text(StrLen::Fixed(E::Remaining), Encoding::Utf8),
        Vlen::Sequence(base, 1) => T::Named(if base.placer { MEMBER_BASE_SEQUENCE } else { BASE_SEQUENCE }.into()),
        Vlen::Sequence(base, depth) => base_sequence(&base, depth),
        Vlen::Bytes => T::bytes(E::Remaining),
    };
    T::structure_named(
        "GlobalHeapId",
        "object",
        "",
        vec![
            ("length", T::u32(Little).counted_as("bytes")),
            ("collection_address", addr()),
            ("object_index", T::u32(Little)),
            // An element of no length points at nothing. Some writers say so
            // with the undefined address, the same as every other address in
            // this format; HDF5 2.0 writes a sequence of no elements with
            // address nought, which is the superblock and never a collection.
            (
                "collection",
                T::switch(
                    E::field("collection_address"),
                    vec![(0, T::bytes(E::lit(0))), (UNDEFINED, T::bytes(E::lit(0)))],
                    T::at_origin(E::field("collection_address"), T::Named("GlobalHeap".into())),
                ),
            ),
            (
                "object",
                T::switch(
                    E::field("collection_address"),
                    vec![(0, T::bytes(E::lit(0))), (UNDEFINED, T::bytes(E::lit(0)))],
                    T::at_origin(E::start_of(found(&["payload"])), T::sized(found(&["size"]), contents)),
                ),
            ),
        ],
    )
    .field_aside("object")
}

/// The heap a group keeps its links in once there are too many of them to
/// write as messages. "Fractal" is the doubling: each row of blocks is twice
/// the size of the row before it, so a heap that grows keeps its bookkeeping
/// the same shape rather than rewriting itself.
fn fractal_heap() -> T {
    T::structure(
        "FractalHeap",
        vec![
            ("signature", T::magic(b"FRHP")),
            ("version", T::u8()),
            ("heap_id_length", T::u16(Little).counted_as("bytes")),
            ("io_filter_length", T::u16(Little).counted_as("bytes")),
            (
                "flags",
                T::flags("HeapFlags", T::u8(), &[(0, "huge ids are wrapped"), (1, "direct blocks are checksummed")]),
            ),
            ("max_managed_object_size", T::u32(Little).counted_as("bytes")),
            ("next_huge_object_id", length()),
            ("huge_object_btree_address", addr()),
            ("managed_free_space", length().counted_as("bytes")),
            ("free_space_manager_address", addr()),
            ("managed_space", length().counted_as("bytes")),
            ("allocated_managed_space", length().counted_as("bytes")),
            ("direct_block_iterator_offset", length()),
            ("managed_object_count", length().counted_as("objects")),
            ("huge_object_size", length().counted_as("bytes")),
            ("huge_object_count", length().counted_as("objects")),
            ("tiny_object_size", length().counted_as("bytes")),
            ("tiny_object_count", length().counted_as("objects")),
            // How many blocks a row holds, and how big the first row's blocks
            // are. Every row after it doubles.
            ("table_width", T::u16(Little).counted_as("blocks per row")),
            ("starting_block_size", length().counted_as("bytes")),
            ("max_direct_block_size", length().counted_as("bytes")),
            ("max_heap_size", T::u16(Little).counted_as("bits")),
            ("starting_rows", T::u16(Little).counted_as("rows")),
            ("root_block_address", addr()),
            ("current_rows", T::u16(Little).counted_as("rows")),
            // A heap whose blocks went through filters says here what it did
            // to the root one.
            (
                "filtered_root",
                T::switch(
                    E::field("io_filter_length"),
                    vec![(0, T::bytes(E::lit(0)))],
                    T::inline_structure(
                        "FilteredRoot",
                        vec![
                            ("size", length().counted_as("bytes")),
                            ("filter_mask", T::u32(Little)),
                            ("filter_info", T::bytes(E::field("io_filter_length"))),
                        ],
                    ),
                ),
            ),
            ("checksum", T::u32(Little)),
            // How many rows of any table hold direct blocks: the first two
            // rows are the starting size, and each row after that doubles,
            // until a row's blocks would be bigger than a direct block may be.
            // A table row past these holds indirect blocks instead.
            (
                "direct_rows",
                T::computed(
                    E::field("max_direct_block_size")
                        .log2()
                        .sub(E::field("starting_block_size").log2())
                        .add(E::lit(2)),
                )
                .counted_as("rows"),
            ),
            // The root table's rows, under the name an entry in a table gives
            // the rows of the table it points at. See `heap_child`.
            ("block_rows", T::computed(E::field("current_rows")).counted_as("rows")),
            // No rows means the root block holds the objects themselves;
            // otherwise it is the table that says where those blocks are.
            (
                "root_block",
                T::switch(
                    E::field("current_rows"),
                    vec![(
                        0,
                        at_address(
                            "root_block_address",
                            T::sized(E::field("starting_block_size"), T::Named("HeapDirectBlock".into())),
                        ),
                    )],
                    at_address("root_block_address", T::Named("HeapIndirectBlock".into())),
                ),
            ),
        ],
    )
}

/// A block holding the objects themselves. What is in one is not said
/// anywhere in the block: a heap object has no header, and where each one ends
/// is known only from the id that names it. For a group's heap the objects are
/// links, written exactly as a link message is, so they are read one after
/// another until the free space at the end of the block, which is zeros where
/// a link's version would be.
fn heap_direct_block() -> T {
    T::structure(
        "HeapDirectBlock",
        vec![
            ("signature", T::magic(b"FHDB")),
            ("version", T::u8()),
            ("heap_header_address", addr()),
            // As many bytes as the heap said its offsets take, rounded up.
            ("block_offset", T::bytes(E::field("max_heap_size").add(E::lit(7)).div(E::lit(8)))),
            ("checksum", when(bit("flags", 1), T::u32(Little))),
            ("links", T::repeat(heap_link(), Until::End).counted_as("links")),
        ],
    )
}

/// One object in a fractal heap's block, which for a group's heap is a link.
///
/// Where the links stop is the hard part: nothing in the block says how many
/// there are, and the space after them belongs to a free-space manager this
/// does not read. Two things end the run honestly. A link's version is never
/// zero, so a run of zeros is free space and is shown as that. And a stretch
/// too short to hold the shortest possible link is free space too, whatever is
/// in it. What is left over is a block whose free space holds something that
/// is neither: bytes a link was written into and later moved out of. Those are
/// read as links, wrongly, and a reader can tell by the names.
fn heap_link() -> T {
    let free = |what: &str| T::structure(what, vec![("free", T::bytes(E::Remaining))]);
    // The shortest a link can be: a version, flags with nothing optional set,
    // a one-byte length, a name of no characters, and an address.
    const SHORTEST_LINK: i128 = 11;
    T::switch(
        E::peek(8, Little),
        vec![(0, free("HeapFreeSpace"))],
        T::switch(E::Remaining.less_than(E::lit(SHORTEST_LINK)), vec![(1, free("HeapTail"))], link()),
    )
}

/// The table of where a heap's blocks are: `table_width` of them per row, each
/// row twice the size of the one before.
///
/// The rows up to the largest direct block size hold direct blocks, and every
/// row past them holds indirect blocks: another table like this one, standing
/// for as much of the heap as a block of that row's size would. That is how a
/// heap grows past its largest direct block without its blocks getting any
/// bigger, and how a group of a few thousand links ends up with a table inside
/// a table.
///
/// How many rows a table has is written in the heap's header for the root one
/// and nowhere at all for the rest. A table in a row whose blocks are of size
/// `s` has as many rows as it takes for its own blocks to add up to `s`, which
/// is `log2(s)` less `log2` of the first row's total, plus one. The entry
/// pointing at the table works that out, and the table reads it from there
/// under the same name the header gives the root's: `block_rows`.
fn heap_indirect_block() -> T {
    T::structure(
        "HeapIndirectBlock",
        vec![
            ("signature", T::magic(b"FHIB")),
            ("version", T::u8()),
            ("heap_header_address", addr()),
            ("block_offset", T::bytes(E::field("max_heap_size").add(E::lit(7)).div(E::lit(8)))),
            (
                "children",
                T::array(heap_child(), E::field("table_width").mul(E::field("block_rows"))).counted_as("blocks"),
            ),
            ("checksum", T::u32(Little)),
        ],
    )
}

/// One entry of that table: where a block is, and the block, which is a direct
/// block in the rows up to `direct_rows` and another table in the rows past
/// them.
///
/// A heap whose blocks went through filters writes two more fields beside each
/// direct block's address: how big the block came out, and which filters were
/// skipped. A filtered block is the filters' output, so it keeps its bytes.
/// Nothing in the collection has a filtered heap, since a group's heap is
/// never filtered, so that reading has been checked against the specification
/// and not against a file.
fn heap_child() -> T {
    let direct = || E::field("row").less_than(E::field("direct_rows"));
    let filtered = || E::lit(0).less_than(E::field("io_filter_length"));
    T::structure(
        "HeapBlock",
        vec![
            ("address", addr()),
            ("row", T::computed(E::Idx.div(E::field("table_width")))),
            ("block_size", row_block_size()),
            (
                "filtered_block",
                when(
                    direct().mul(filtered()),
                    T::inline_structure(
                        "FilteredBlock",
                        vec![("size", length().counted_as("bytes")), ("filter_mask", T::u32(Little))],
                    ),
                ),
            ),
            // How many rows the table this entry points at has, for an entry
            // in the indirect rows. Nought for a direct block, which is not a
            // table.
            (
                "block_rows",
                T::switch(
                    direct(),
                    vec![(1, T::computed(E::lit(0)))],
                    T::computed(
                        E::field("block_size")
                            .log2()
                            .sub(E::field("starting_block_size").mul(E::field("table_width")).log2())
                            .add(E::lit(1)),
                    ),
                )
                .counted_as("rows"),
            ),
            (
                "block",
                T::switch(
                    direct(),
                    vec![(
                        1,
                        T::switch(
                            filtered(),
                            vec![(1, at_address("address", T::bytes(E::within(&["filtered_block", "size"]))))],
                            at_address("address", T::sized(E::field("block_size"), T::Named("HeapDirectBlock".into()))),
                        ),
                    )],
                    at_address("address", T::Named("HeapIndirectBlock".into())),
                ),
            ),
        ],
    )
}

/// How big the blocks of one row are, in a field of no bytes: the starting
/// size for the first two rows, and double the row before for every row after.
///
/// Sizing them matters because a direct block says nothing about how long it
/// is. Without this the links inside one would be read on past the block's end
/// and into whatever the bytes after it happen to be. And an indirect block's
/// size is what says how many rows it has.
fn row_block_size() -> T {
    let starting = E::field("starting_block_size");
    T::switch(
        E::field("row"),
        vec![(0, T::computed(starting.clone()))],
        T::computed(starting.shl(E::field("row").sub(E::lit(1)))),
    )
    .counted_as("bytes")
}

/// What a version 2 b-tree's records are, by the type byte its header writes.
/// Section IV.A.2.h of the HDF5 File Format Specification, version 3.0.
///
/// Shared with [`super::hdf5_tree`], which walks one of these trees and has to
/// say in words which of the twelve it found. Two copies of a table like this
/// drift, and the way they drift is that a reader is told a tree holds link
/// names while the Listing at the same bytes calls it something else.
pub(crate) const BTREE2_TYPE: &[(i128, &str)] = &[
    (0, "testing"),
    (1, "huge objects, indirectly accessed"),
    (2, "huge objects, filtered and indirectly accessed"),
    (3, "huge objects, directly accessed"),
    (4, "huge objects, filtered and directly accessed"),
    (5, "link names"),
    (6, "link creation order"),
    (7, "shared object header messages"),
    (8, "attribute names"),
    (9, "attribute creation order"),
    (10, "chunks, unfiltered"),
    (11, "chunks, filtered"),
];

/// A version 2 b-tree: what a group written by a newer library uses to find a
/// link by the hash of its name, and what a chunked dataset written by one
/// uses to find a chunk.
///
/// The records are left as their bytes. What a record means is settled by the
/// tree's type, and the one that matters here holds a hash and a heap id,
/// which is a length and an offset into the heap above rather than an address
/// in the file. The links themselves are read from the heap's blocks, so
/// nothing is lost by leaving the index alone.
///
/// Every node is placed, the root and everything under it. A node says nothing
/// about itself beyond its signature: how many records it holds, and how far
/// above the leaves it sits, are written in the pointer that names it, or in
/// the header for the root. So the header writes those two for the root under
/// the same names a child pointer writes them for its child, `child_records`
/// and `child_level`, and a node reads whichever is nearest above it. One node
/// type serves every level that way, which is what lets the tree be as deep as
/// the file made it.
///
/// What a pointer holds is not written anywhere either. It is an address, the
/// child's record count, and, above the level just over the leaves, how many
/// records the child and everything under it hold; the two counts are as many
/// bytes as the largest number each could ever be needs, and those largest
/// numbers come from the node size, the record size, and every level below.
/// `levels` works that out, one level at a time from the leaves up, the way the
/// library does when it opens the tree. [`super::hdf5_tree`] does the same
/// arithmetic in Rust to draw the tree, and the two are checked against each
/// other on real files.
fn btree2() -> T {
    T::structure(
        "BTree2",
        vec![
            ("signature", T::magic(b"BTHD")),
            ("version", T::u8()),
            ("type", T::enumeration("BTree2Type", T::u8(), BTREE2_TYPE)),
            ("node_size", T::u32(Little).counted_as("bytes")),
            ("record_size", T::u16(Little).counted_as("bytes")),
            ("depth", T::u16(Little)),
            ("split_percent", T::u8()),
            ("merge_percent", T::u8()),
            ("root_node_address", addr()),
            ("root_record_count", T::u16(Little).counted_as("records")),
            ("record_count", length().counted_as("records")),
            ("checksum", T::u32(Little)),
            // How wide the record count in every child pointer is: as many
            // bytes as the most records a leaf can hold needs. A leaf holds
            // the most of any node, having no pointers to make room for, so
            // the one width does for the whole tree.
            (
                "child_records_size",
                T::computed(
                    E::field("node_size")
                        .sub(E::lit(BTREE2_NODE_PREFIX))
                        .div(E::field("record_size"))
                        .log2()
                        .div(E::lit(8))
                        .add(E::lit(1)),
                )
                .counted_as("bytes"),
            ),
            (
                "levels",
                T::array(btree2_level(), E::field("depth").at_most(E::lit(BTREE2_MAX_LEVELS)).add(E::lit(1))),
            ),
            // The root node's level and record count, under the names a child
            // pointer gives its child's.
            ("child_level", T::computed(E::field("depth"))),
            ("child_records", T::computed(E::field("root_record_count")).counted_as("records")),
            (
                "root_node",
                at_address("root_node_address", T::sized(E::field("node_size"), T::Named("BTree2Node".into()))),
            ),
        ],
    )
}

/// What a node spends on being a node: its signature, version and type byte in
/// front, and its checksum at the end. The library calls it the metadata
/// prefix.
const BTREE2_NODE_PREFIX: i128 = 10;

/// The deepest tree whose levels are worked out. Each level is worked out from
/// the one below it, so a depth field that is not a depth would otherwise ask
/// for sixty thousand of them, each waiting on the last; a tree this deep with
/// the library's own node sizes would index more records than a file can hold.
/// The same bound [`super::hdf5_tree`] walks to.
const BTREE2_MAX_LEVELS: i128 = 24;

/// One level of a version 2 b-tree, as the library works it out: how wide a
/// pointer written by a node at this level is, how many records such a node can
/// hold, and how many records can be under one in all. Element 0 is the leaves.
///
/// Every number is computed and none is read. A level's pointers hold a total
/// as wide as the level below it can need, so each element asks the one before
/// it, and the leaves, which point at nothing, start the chain off.
fn btree2_level() -> T {
    let leaves = || E::Idx.less_than(E::lit(1));
    T::structure(
        "BTree2Level",
        vec![
            // An address, the child's record count, and the total under the
            // child, which is as wide as the level below needed. Nothing at
            // all at the leaves.
            (
                "pointer_size",
                T::switch(
                    leaves(),
                    vec![(1, T::computed(E::lit(0)))],
                    T::computed(E::lit(8).add(E::field("child_records_size")).add(E::prev("total_records_size"))),
                )
                .counted_as("bytes"),
            ),
            // A node has one more pointer than it has records, so the spare
            // one is taken off with the prefix before dividing.
            (
                "max_records",
                T::computed(
                    E::field("node_size")
                        .sub(E::lit(BTREE2_NODE_PREFIX))
                        .sub(E::field("pointer_size"))
                        .div(E::field("record_size").add(E::field("pointer_size"))),
                )
                .counted_as("records"),
            ),
            // A node's own records and the whole of each of its children.
            // Held to what a length can say, since a node size read from
            // bytes that are not a tree header can make the product as large
            // as it likes.
            (
                "max_total_records",
                T::computed(
                    E::field("max_records")
                        .add(E::field("max_records").add(E::lit(1)).mul(E::prev("max_total_records")))
                        .at_most(E::lit(u64::MAX as i128)),
                )
                .counted_as("records"),
            ),
            // How wide the total in a pointer to a node of this level is. A
            // pointer to a leaf writes no total, since the leaf's own count is
            // all there is under it.
            (
                "total_records_size",
                T::switch(
                    leaves(),
                    vec![(1, T::computed(E::lit(0)))],
                    T::computed(E::field("max_total_records").log2().div(E::lit(8)).add(E::lit(1))),
                )
                .counted_as("bytes"),
            ),
        ],
    )
}

/// A node of such a tree, of whichever of the two kinds the signature says.
fn btree2_node() -> T {
    T::switch(
        E::peek(32, Big),
        vec![
            (u32::from_be_bytes(*b"BTLF") as i128, btree2_leaf()),
            (u32::from_be_bytes(*b"BTIN") as i128, btree2_internal()),
        ],
        T::structure("UnknownBTree2Node", vec![("signature", T::utf8(E::lit(4)))]),
    )
}

fn btree2_leaf() -> T {
    T::structure(
        "BTree2Leaf",
        vec![
            ("signature", T::magic(b"BTLF")),
            ("version", T::u8()),
            ("type", T::u8()),
            (
                "records",
                T::array(T::bytes(E::field("record_size")), E::field("child_records")).counted_as("records"),
            ),
            ("checksum", T::u32(Little)),
        ],
    )
}

/// A node above the leaves: records of its own, read the same way a leaf's
/// are, and one more pointer than it has records, since a record sits between
/// every two children.
fn btree2_internal() -> T {
    T::structure(
        "BTree2Internal",
        vec![
            ("signature", T::magic(b"BTIN")),
            ("version", T::u8()),
            ("type", T::u8()),
            ("level", T::computed(E::field("child_level"))),
            (
                "records",
                T::array(T::bytes(E::field("record_size")), E::field("child_records")).counted_as("records"),
            ),
            (
                "children",
                T::array(btree2_child(), E::field("child_records").add(E::lit(1))).counted_as("children"),
            ),
            ("checksum", T::u32(Little)),
        ],
    )
}

/// One child pointer, and the node it points at.
///
/// The total is left out of a pointer to a leaf rather than read as a field of
/// no bytes, which is what the library writes: the leaf's own count is all
/// there is under it.
fn btree2_child() -> T {
    T::structure(
        "BTree2Child",
        vec![
            ("address", addr()),
            (
                "child_records",
                T::uint_expr(E::field("child_records_size").mul(E::lit(8)), Little).counted_as("records"),
            ),
            (
                "total_records",
                when(
                    E::lit(1).less_than(E::field("level")),
                    T::uint_expr(
                        E::elem_field("levels", E::field("level").sub(E::lit(1)), &["total_records_size"]).mul(E::lit(8)),
                        Little,
                    )
                    .counted_as("records"),
                ),
            ),
            ("child_level", T::computed(E::field("level").sub(E::lit(1)))),
            ("node", at_address("address", T::sized(E::field("node_size"), T::Named("BTree2Node".into())))),
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
        "payload",
        vec![
            ("object_index", T::u16(Little)),
            ("reference_count", T::u16(Little)),
            ("reserved", T::u32(Little)),
            ("size", length().counted_as("bytes")),
            // The object's own bytes, and then the padding that rounds the
            // collection on to the next multiple of eight. They were one field
            // covering both, which reads the same and says less: what a
            // variable-length element points at is the object, and a field
            // that runs on past its end cannot be that. The padding belongs to
            // nobody and is written down as its own row for the same reason.
            //
            // Object zero is the free space rather than an object, and its
            // size counts the sixteen bytes of header this one has already
            // read, so it is measured to the end of the collection instead:
            // that is what free space is, and it saves subtracting a header
            // from a length that may be shorter than one.
            (
                "payload",
                T::switch(E::field("object_index"), vec![(0, T::bytes(E::Remaining))], T::bytes(E::field("size"))),
            ),
            (
                "padding",
                T::switch(
                    E::field("object_index"),
                    vec![(0, T::bytes(E::lit(0)))],
                    T::bytes(pad8(E::field("size")).sub(E::field("size"))),
                ),
            ),
        ],
    )
}

/// A run of `bytes` bytes read as the dataset's elements. The two fields of no
/// bytes in front of them are how far the elements can see: an expression
/// reads the fields of the structures it sits in, and both of these are
/// answers from somewhere else in the object header.
///
/// A dataset whose elements are compounds, enumerations, arrays or sequences
/// also places its datatype message a third time, as `datatype`: each element
/// asks it what its members or its base type are, and a search back through
/// the messages from inside the run would pass every element before the one
/// asking. An attribute has its own `datatype` in scope already, and a chunked
/// layout keeps one above its chunks.
pub(super) fn elements(by: Described, bytes: E) -> T {
    let mut fields = vec![element_size_field(by), ("run_bytes", T::computed(bytes))];
    let beside = matches!(by, Described::Beside);
    if beside {
        fields.push((
            "datatype",
            T::when(needs_datatype(by), T::at_origin(Described::beside_address(), T::Named("Datatype".into()))),
        ));
    }
    let named = match by {
        Described::Beside => ELEMENTS_BESIDE,
        Described::Inside => ELEMENTS_INSIDE,
    };
    fields.push(("elements", T::Named(named.into())));
    let data = T::structure("Data", fields);
    if beside { data.field_aside("datatype").machinery(&["datatype"]) } else { data }
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
            // Where a group with more than a handful of links keeps them: a
            // fractal heap holding the links themselves, and a version 2
            // b-tree indexing them by the hash of their names.
            ("fractal_heap_address", addr()),
            ("name_index_btree_address", addr()),
            ("creation_order_index_address", when(bit("flags", 1), addr())),
            ("heap", at_address("fractal_heap_address", T::Named("FractalHeap".into()))),
            ("name_index", at_address("name_index_btree_address", T::Named("BTree2".into()))),
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
                // Version 5 is version 4 byte for byte. What it changes is one
                // width in the chunk index rather than anything in the message:
                // the size of a filtered chunk, which version 4 wrote in as
                // few bytes as an unfiltered chunk needs plus one, and version
                // 5 writes in as many as an address takes, so that a filter
                // which makes a chunk bigger cannot overflow the field. The
                // entry works that width out from its own size and so reads
                // either. A version past that keeps its bytes.
                T::switch(
                    E::field("version"),
                    vec![(1, layout_v1()), (2, layout_v1()), (3, layout_v3()), (4, layout_v4()), (5, layout_v4())],
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
    // A virtual dataset is a version 4 class and cannot appear in the earlier
    // messages, which is why naming it here costs those nothing.
    T::enumeration("LayoutClass", T::u8(), &[(0, "compact"), (1, "contiguous"), (2, "chunked"), (3, "virtual")])
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
                        (0, compact_storage()),
                        (1, contiguous_storage()),
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

/// Compact and contiguous, which versions 3 and 4 write the same way: the
/// elements in the message itself, or one run of them at an address.
fn compact_storage() -> T {
    T::structure(
        "Compact",
        vec![("size", T::u16(Little)), ("data", elements(Described::Beside, E::field("size")))],
    )
}

fn contiguous_storage() -> T {
    T::structure(
        "Contiguous",
        vec![
            ("address", addr()),
            ("size", length()),
            ("data", at_address("address", elements(Described::Beside, E::field("size")))),
        ],
    )
}

/// Versions 4 and 5, which a file written to the latest library version gets.
/// Compact and contiguous are what version 3 made them, a virtual dataset is only
/// here, and a chunked one names which of five indexes places its chunks
/// rather than always being a version 1 b-tree.
fn layout_v4() -> T {
    T::structure(
        "Layout",
        vec![
            ("layout_class", layout_class()),
            (
                "storage",
                T::switch(
                    E::field("layout_class"),
                    vec![(0, compact_storage()), (1, contiguous_storage()), (2, chunked_v4()), (3, virtual_storage())],
                    T::bytes(E::Remaining),
                ),
            ),
        ],
    )
}
/// A virtual dataset, whose elements are in other datasets and other files.
/// What maps this dataset's selections onto theirs is one object in a global
/// heap collection, reached the same way a variable-length element is.
fn virtual_storage() -> T {
    T::structure(
        "Virtual",
        vec![
            ("collection_address", addr()),
            ("object_index", T::u32(Little)),
            ("collection", at_address("collection_address", T::Named("GlobalHeap".into()))),
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
    .field_time("seconds", Time::unix())
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
                T::at_origin(
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
pub(crate) mod tests {
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
    pub(crate) const ALPHA_HEADER: u64 = 280;
    pub(crate) const DATA: u64 = 376;

    /// The path from the file down to the one link the group holds. Every step
    /// of it but the last two is a pointer being followed: the root entry's
    /// object header, its symbol table message, the heap that message names,
    /// the tree placed under the heap, the entry's child node, and the entry
    /// in it.
    pub(crate) const LINK: &[usize] = &[
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

    pub(crate) fn put(v: &mut Vec<u8>, at: u64, bytes: &[u8]) {
        let at = at as usize;
        if v.len() < at + bytes.len() {
            v.resize(at + bytes.len(), 0);
        }
        v[at..at + bytes.len()].copy_from_slice(bytes);
    }

    pub(crate) fn addr_bytes(v: u64) -> [u8; 8] {
        v.to_le_bytes()
    }

    /// The smallest file with a group in it: a superblock, the root group's
    /// object header, the tree and heap that header names, a symbol table node
    /// with one link in it, and the object that link points at.
    pub(crate) fn one_link_file() -> Vec<u8> {
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

    pub(crate) fn read(f: &[u8], path: &[usize]) -> (String, Value) {
        let doc = Document::new(MemSource(f.to_vec()));
        let mut ev = Evaluator::new(hdf5());
        let node = ev.node(&doc, path).expect("node");
        (node.name, node.value)
    }

    /// A cursor nowhere near a tree still gets one: the root group's, which
    /// every file with a version 0 superblock has. Without this a reader who
    /// opened the tab before moving the cursor would be told the file has no
    /// B-tree, which would be a lie about a file with one at 0x88.
    #[test]
    fn a_cursor_outside_every_tree_gets_the_root_group_s() {
        let f = one_link_file();
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let tree = super::super::hdf5_tree::tree(&mut ev, &doc, &[0], 64).expect("walk").expect("a tree");
        assert_eq!(tree.job, super::super::hdf5_tree::Job::Group);
        assert_eq!(tree.nodes.len(), 2, "{tree:?}");
        assert_eq!(tree.nodes[0].kind, super::super::hdf5_tree::Kind::Index);
        assert_eq!(tree.nodes[0].address, BTREE);
        assert_eq!(tree.nodes[0].entries, 1);
        assert_eq!(tree.nodes[0].depth, 0);
        // The row a chunk tree does not have: the links are one hop past the
        // bottom index node, in a node of their own.
        assert_eq!(tree.nodes[1].kind, super::super::hdf5_tree::Kind::LinkTable);
        assert_eq!(tree.nodes[1].address, SNOD);
        assert_eq!(tree.nodes[1].depth, 1);
        assert_eq!(tree.nodes[1].entries, 1);
    }

    /// A key range is read at the bottom, from the names a link table holds,
    /// and every node above takes the span of its children. The index node
    /// over one link table covers exactly that link.
    #[test]
    fn a_key_range_comes_from_the_names_at_the_bottom() {
        let f = one_link_file();
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let tree = super::super::hdf5_tree::tree(&mut ev, &doc, &[0], 64).expect("walk").expect("a tree");
        assert_eq!(tree.nodes[1].first_key, "alpha");
        assert_eq!(tree.nodes[1].last_key, "alpha");
        assert_eq!(tree.nodes[0].first_key, "alpha");
        assert_eq!(tree.nodes[0].last_key, "alpha");
    }

    /// Every node says where its entries start and how wide one is, so a view
    /// can divide the box it draws and send a reader to one entry's bytes.
    ///
    /// Checked against the file built above, whose bytes are placed here by
    /// hand: the tree's one entry is the eight-byte key at `BTREE + 24` and
    /// the child address at `BTREE + 32`, and the symbol table's one entry is
    /// the forty bytes from `SNOD + 8`. The addresses are what say the stride
    /// lands: the child address is the last eight bytes of an entry, so
    /// reading it back out of the entry the numbers place is reading the
    /// number this file was built with.
    #[test]
    fn a_node_says_where_its_entries_start_and_how_wide_one_is() {
        let f = one_link_file();
        let doc = Document::new(MemSource(f.clone()));
        let mut ev = Evaluator::new(hdf5());
        let tree = super::super::hdf5_tree::tree(&mut ev, &doc, &[0], 64).expect("walk").expect("a tree");
        let at = |v: &[u8], b: u64| u64::from_le_bytes(v[b as usize..b as usize + 8].try_into().unwrap());

        // A key and a child address: sixteen bytes, after the twenty-four the
        // node spends on its signature, its level, its count and its two
        // siblings.
        let node = &tree.nodes[0];
        assert_eq!((node.first_entry_bits, node.entry_bits), (24 * 8, 16 * 8), "{node:?}");
        let entry = node.address * 8 + node.first_entry_bits;
        assert_eq!(at(&f, entry / 8), 0, "the key of the one entry is the heap offset the name is at");
        assert_eq!(at(&f, (entry + node.entry_bits) / 8 - 8), SNOD, "the entry does not end at its child address");
        // The key that closes the range is not an entry: it sits one stride
        // past the last one, and is the offset of the end of the heap.
        assert_eq!(at(&f, (entry + node.entry_bits) / 8), 14);

        // A symbol table entry: a name offset, an object header address, a
        // cache type, four reserved bytes and sixteen of scratch.
        let links = &tree.nodes[1];
        assert_eq!((links.first_entry_bits, links.entry_bits), (8 * 8, 40 * 8), "{links:?}");
        let entry = links.address * 8 + links.first_entry_bits;
        assert_eq!(at(&f, entry / 8), 8, "the link's name is at offset 8 in the heap");
        assert_eq!(at(&f, entry / 8 + 8), ALPHA_HEADER, "the entry does not name the object it points at");

        // The last entry of either ends inside the node rather than past it.
        for node in &tree.nodes {
            let end = node.first_entry_bits + node.entries * node.entry_bits;
            assert!(end <= node.size_bits, "{node:?}: its entries run past the node");
        }
    }

    /// From inside a node, the walk climbs to the root of that node's own
    /// tree. The climb has to stop where one tree ends: a group's tree sits,
    /// in the template, under the tree of the group above it, so a climb that
    /// went to the topmost `TREE` would answer with the root group's tree
    /// wherever the reader stood.
    #[test]
    fn a_cursor_inside_a_node_gets_that_node_s_own_tree() {
        let f = one_link_file();
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        // The link itself, which sits inside the symbol table node.
        let tree = super::super::hdf5_tree::tree(&mut ev, &doc, LINK, 64).expect("walk").expect("a tree");
        assert_eq!(tree.nodes[0].address, BTREE);
        assert_eq!(tree.nodes.len(), 2);
    }

    /// The cap is a cap on nodes drawn, and what it costs is counted rather
    /// than quietly dropped.
    #[test]
    fn a_cap_says_how_many_children_it_left_out() {
        let f = one_link_file();
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        let tree = super::super::hdf5_tree::tree(&mut ev, &doc, &[0], 1).expect("walk").expect("a tree");
        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.omitted, 1);
        assert!(tree.nodes[0].truncated);
        // A node whose children were not all reached keeps no range: it would
        // be right at one end and short at the other.
        assert_eq!(tree.nodes[0].first_key, "");
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
        // first of the numbers it places. The run's fourth field: the third is
        // the copy of the datatype a run of compounds keeps, which a run of
        // integers leaves out.
        let mut first = LINK.to_vec();
        first.extend_from_slice(&[6, 0, 6, 2, 4, 1, 1, 2, 0, 3, 0]);
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


    // A file with version 2 structures in it, built by hand. The addresses are
    // spread out so that nothing in it touches anything else: what these tests
    // are about is a walk that reads its own way from one node to the next,
    // and two structures that happened to abut would hide a length read wrong.
    const V2_ROOT_HEADER: u64 = 96;
    const V2_BTHD: u64 = 256;
    const V2_BTIN: u64 = 1024;
    const V2_LEAF_A: u64 = 2048;
    const V2_LEAF_B: u64 = 3072;
    const V2_NODE_SIZE: u64 = 512;
    const V2_RECORD: u64 = 24;
    const V2_END: u64 = 4096;

    /// The path from the file down to the version 2 tree's header: the root
    /// group's object header, its link info message, and the tree that message
    /// names.
    const V2_TREE: &[usize] = &[
        2, // superblock
        8, // the root group's object header
        0, //
        6, // its messages
        0, // the link info message
        4, // its body
        7, // the name index the body names
        0, //
    ];

    /// A file with a version 2 superblock, a version 2 root group object
    /// header holding a link info message, and the version 2 b-tree that
    /// message names: a `BTIN` root over two `BTLF` leaves.
    ///
    /// The record type is 10, unfiltered chunks, because that is the one of
    /// the two read types whose records a test can assert about: a link name
    /// record holds a hash and a heap id and nothing that could be checked
    /// against a name. The records are 24 bytes, which is an address and two
    /// dimension offsets, so this is a rank 2 dataset's chunk index.
    fn v2_file() -> Vec<u8> {
        let mut f = Vec::new();
        put(&mut f, 0, b"\x89HDF\r\n\x1a\n");
        // A version 2 superblock: eight bytes for both an address and a
        // length, and the root group's header named outright rather than
        // through a symbol table entry.
        put(&mut f, 8, &[2, 8, 8, 0]);
        put(&mut f, 12, &addr_bytes(0));
        put(&mut f, 20, &addr_bytes(u64::MAX));
        put(&mut f, 28, &addr_bytes(V2_END));
        put(&mut f, 36, &addr_bytes(V2_ROOT_HEADER));
        // The root group's object header, version 2: no message count, flags
        // saying its messages are sized in one byte, and one message in them.
        put(&mut f, V2_ROOT_HEADER, b"OHDR");
        put(&mut f, V2_ROOT_HEADER + 4, &[2, 0, 22]);
        // A link info message: a type, a size, flags, and then the body, which
        // names a fractal heap it has none of and the tree that indexes it.
        put(&mut f, V2_ROOT_HEADER + 7, &[0x02]);
        put(&mut f, V2_ROOT_HEADER + 8, &18u16.to_le_bytes());
        put(&mut f, V2_ROOT_HEADER + 10, &[0, 0, 0]);
        put(&mut f, V2_ROOT_HEADER + 13, &addr_bytes(u64::MAX));
        put(&mut f, V2_ROOT_HEADER + 21, &addr_bytes(V2_BTHD));
        // The header: node size 512, records of 24 bytes, one level of
        // internal nodes over the leaves.
        put(&mut f, V2_BTHD, b"BTHD");
        put(&mut f, V2_BTHD + 4, &[0, 10]);
        put(&mut f, V2_BTHD + 6, &(V2_NODE_SIZE as u32).to_le_bytes());
        put(&mut f, V2_BTHD + 10, &(V2_RECORD as u16).to_le_bytes());
        put(&mut f, V2_BTHD + 12, &1u16.to_le_bytes());
        put(&mut f, V2_BTHD + 14, &[100, 40]);
        put(&mut f, V2_BTHD + 16, &addr_bytes(V2_BTIN));
        put(&mut f, V2_BTHD + 24, &1u16.to_le_bytes());
        put(&mut f, V2_BTHD + 26, &5u64.to_le_bytes());
        // The root: one record of its own and two children. A pointer at this
        // level is an address and one byte of record count, and no running
        // total, because the level below it is the leaves.
        put(&mut f, V2_BTIN, b"BTIN");
        put(&mut f, V2_BTIN + 4, &[0, 10]);
        put(&mut f, V2_BTIN + 6, &chunk_record(400));
        put(&mut f, V2_CHILD, &addr_bytes(V2_LEAF_A));
        put(&mut f, V2_CHILD + 8, &[2]);
        put(&mut f, V2_CHILD + 9, &addr_bytes(V2_LEAF_B));
        put(&mut f, V2_CHILD + 17, &[2]);
        // Two leaves of two records each, in order, which is what makes the
        // root's range run from the first of the first to the last of the last.
        put(&mut f, V2_LEAF_A, b"BTLF");
        put(&mut f, V2_LEAF_A + 4, &[0, 10]);
        put(&mut f, V2_LEAF_A + 6, &chunk_record(0));
        put(&mut f, V2_LEAF_A + 6 + V2_RECORD, &chunk_record(200));
        put(&mut f, V2_LEAF_B, b"BTLF");
        put(&mut f, V2_LEAF_B + 4, &[0, 10]);
        put(&mut f, V2_LEAF_B + 6, &chunk_record(600));
        put(&mut f, V2_LEAF_B + 6 + V2_RECORD, &chunk_record(800));
        f.resize(V2_END as usize, 0);
        f
    }

    /// Where the root's first child pointer starts: past the signature, the
    /// version, the type and the one record it holds.
    const V2_CHILD: u64 = V2_BTIN + 6 + V2_RECORD;

    /// One unfiltered chunk record: where the chunk is, and where in the
    /// dataset it starts. Two dimensions, the second of them always 50, so a
    /// test can tell the two numbers of a key apart.
    fn chunk_record(row: u64) -> [u8; 24] {
        let mut out = [0u8; 24];
        out[0..8].copy_from_slice(&addr_bytes(V2_END));
        out[8..16].copy_from_slice(&row.to_le_bytes());
        out[16..24].copy_from_slice(&50u64.to_le_bytes());
        out
    }

    fn v2_tree(f: Vec<u8>, at: &[usize], limit: usize) -> super::super::hdf5_tree::Tree {
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        super::super::hdf5_tree::tree(&mut ev, &doc, at, limit).expect("walk").expect("a tree")
    }

    /// The shape of a version 2 tree is not written in the file. The widths of
    /// the two counts in a child pointer come out of the node size, the record
    /// size and the depth, and every child address after the first is read at
    /// a place those widths decide. Walked, the three nodes come back where
    /// the file put them.
    #[test]
    fn a_version_2_tree_is_walked_from_its_header() {
        use super::super::hdf5_tree::{Job, Kind, Records};
        let tree = v2_tree(v2_file(), &[0], 64);
        assert_eq!(tree.version, 2);
        assert_eq!(tree.job, Job::Chunk);
        assert_eq!(tree.records, Records::Read);
        assert_eq!(tree.record_type, 10);
        // The header's own count, and not the sum of the nodes': the root's own
        // record is not repeated in a leaf, so the leaves add up to four.
        assert_eq!(tree.records_total, 5);
        assert_eq!(tree.nodes.iter().map(|n| n.entries).sum::<u64>(), 5);
        assert_eq!(tree.nodes.len(), 3, "{tree:?}");
        assert_eq!(tree.nodes[0].address, V2_BTIN);
        assert_eq!(tree.nodes[0].sign, "BTIN");
        assert_eq!(tree.nodes[0].kind, Kind::Index);
        // The level counts up from the leaves, the way a version 1 node's own
        // `node_level` does, so level 0 is the bottom row in both versions.
        assert_eq!(tree.nodes[0].level, 1);
        assert_eq!(tree.nodes[0].entries, 1);
        assert_eq!(tree.nodes[1].address, V2_LEAF_A);
        assert_eq!(tree.nodes[1].sign, "BTLF");
        assert_eq!(tree.nodes[1].kind, Kind::Leaf);
        assert_eq!(tree.nodes[1].level, 0);
        assert_eq!(tree.nodes[1].depth, 1);
        assert_eq!(tree.nodes[1].entries, 2);
        assert_eq!(tree.nodes[2].address, V2_LEAF_B);
        // A node is measured to what it wrote and not to the 512 bytes it was
        // given, or every node of a tree would be drawn the same size and none
        // of them would say how full it is.
        assert_eq!(tree.nodes[1].size_bits, (10 + 2 * 24) * 8);
        assert_eq!(tree.nodes[0].size_bits, (10 + 24 + 2 * 9) * 8);
    }

    /// A chunk tree's keys are the offsets in its own leaves' records, carried
    /// up by the same pass the version 1 walk uses. A version 2 record holds
    /// one number per dimension and no more, unlike a version 1 key, which
    /// ends with an offset inside an element that is always zero.
    #[test]
    fn a_version_2_chunk_range_comes_from_the_records_at_the_bottom() {
        let tree = v2_tree(v2_file(), &[0], 64);
        assert_eq!(tree.coords, 2);
        assert!(!tree.coords_pad);
        assert_eq!(tree.nodes[1].first_key, "0, 50");
        assert_eq!(tree.nodes[1].last_key, "200, 50");
        assert_eq!(tree.nodes[2].first_key, "600, 50");
        assert_eq!(tree.nodes[2].last_key, "800, 50");
        // The root's own record sits between its children, so the children's
        // span is the root's span.
        assert_eq!(tree.nodes[0].first_key, "0, 50");
        assert_eq!(tree.nodes[0].last_key, "800, 50");
    }

    /// A record type this does not read keeps its shape and is said to be
    /// unread. The shape of a version 2 tree depends on the record size and
    /// not on what a record means, so there is a true picture to be had; what
    /// would not be true is a range, and there is none.
    #[test]
    fn an_unread_record_type_keeps_its_shape_and_says_so() {
        use super::super::hdf5_tree::Records;
        // Type 11: filtered chunks, whose offsets sit behind a field as wide
        // as the layout message decided, which is not in the tree at all.
        let mut f = v2_file();
        put(&mut f, V2_BTHD + 5, &[11]);
        let tree = v2_tree(f, &[0], 64);
        assert_eq!(tree.records, Records::Unread);
        assert_eq!(tree.record_type_name, "chunks, filtered");
        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(tree.coords, 0);
        assert!(tree.nodes.iter().all(|n| n.first_key.is_empty()), "{tree:?}");
        // A type no version of the specification names says that instead, and
        // is not quietly drawn as one of the types that is named.
        let mut f = v2_file();
        put(&mut f, V2_BTHD + 5, &[200]);
        let tree = v2_tree(f, &[0], 64);
        assert_eq!(tree.records, Records::Unknown);
        assert_eq!(tree.record_type_name, "");
        assert_eq!(tree.nodes.len(), 3);
    }

    /// A group tree's records hold the hash of a link's name and an id into a
    /// fractal heap, and no name. So there is no range to show and none is
    /// shown: a hash where a name belongs is a label whose value is not the
    /// thing it names. Nothing is marked short either, because nothing was
    /// missed.
    #[test]
    fn a_version_2_group_tree_shows_no_key_range_at_all() {
        use super::super::hdf5_tree::{Job, Kind};
        let mut f = v2_file();
        put(&mut f, V2_BTHD + 5, &[5]);
        let tree = v2_tree(f, &[0], 64);
        assert_eq!(tree.job, Job::Group);
        assert_eq!(tree.coords, 0);
        assert!(tree.nodes.iter().all(|n| n.first_key.is_empty() && n.last_key.is_empty()), "{tree:?}");
        assert!(tree.nodes.iter().all(|n| !n.truncated), "{tree:?}");
        // And it has no row of link tables, which is the row that tells a
        // version 1 group tree apart from everything else: these links are in
        // the heap, which the tree does not point at.
        assert!(tree.nodes.iter().all(|n| n.kind != Kind::LinkTable));
    }

    /// A pointer at a node that is not the node the level says should be there
    /// is not followed. The signature is checked against the level rather than
    /// taken as whatever it happens to be, because a `BTLF` a level above the
    /// leaves means the depth and the nodes disagree and nothing below them
    /// can be trusted.
    #[test]
    fn a_child_that_is_not_the_node_it_was_promised_to_be_is_refused() {
        let mut f = v2_file();
        put(&mut f, V2_LEAF_B, b"JUNK");
        let tree = v2_tree(f, &[0], 64);
        assert_eq!(tree.nodes.len(), 2, "{tree:?}");
        // The root says it did not read all of its children, so it keeps no
        // range: its own would be right at one end and short at the other.
        assert!(tree.nodes[0].truncated);
        assert_eq!(tree.nodes[0].first_key, "");
    }

    /// An address past the end of the file refuses rather than reading. The
    /// order matters: a document reads past its own end as bytes still on
    /// their way, so a walk that read first and checked after would hand the
    /// view a "still reading" it could never finish waiting for.
    #[test]
    fn a_pointer_outside_the_file_refuses_rather_than_waiting() {
        let mut f = v2_file();
        put(&mut f, V2_CHILD, &addr_bytes(1 << 40));
        let tree = v2_tree(f, &[0], 64);
        assert_eq!(tree.nodes.len(), 2, "{tree:?}");
        assert!(tree.nodes[0].truncated);
    }

    /// A child pointing back at its own parent is a ring, and following one is
    /// not slow but endless. The walk stops at an address it has already been
    /// to and says the node above it is short.
    #[test]
    fn a_ring_of_child_pointers_stops() {
        let mut f = v2_file();
        put(&mut f, V2_CHILD, &addr_bytes(V2_BTIN));
        put(&mut f, V2_CHILD + 9, &addr_bytes(V2_BTIN));
        let tree = v2_tree(f, &[0], 64);
        assert_eq!(tree.nodes.len(), 1, "{tree:?}");
        assert!(tree.nodes[0].truncated);
    }

    /// A record count larger than a node of that size can hold was read out of
    /// the wrong bytes, and going on with it would read the records and the
    /// child pointers out of the wrong bytes too.
    #[test]
    fn a_record_count_too_big_for_its_node_is_refused() {
        let mut f = v2_file();
        put(&mut f, V2_CHILD + 8, &[250]);
        let tree = v2_tree(f, &[0], 64);
        assert_eq!(tree.nodes.len(), 2, "{tree:?}");
        assert!(tree.nodes[0].truncated);
    }

    /// The cap counts the same way it does for a version 1 tree: nodes drawn,
    /// with what it left out said rather than dropped.
    #[test]
    fn a_version_2_cap_says_how_many_children_it_left_out() {
        let tree = v2_tree(v2_file(), &[0], 1);
        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.omitted, 2);
        assert!(tree.nodes[0].truncated);
        assert_eq!(tree.nodes[0].first_key, "");
    }

    /// A version 2 object header writes no message count, so a search for one
    /// that asked only for that field found no object header at all in a file
    /// a recent library wrote, and the tab was empty wherever the cursor
    /// stood. From inside the tree's own header the walk answers with that
    /// tree.
    ///
    /// Every node carries the path that takes a reader to it: the root's is a
    /// field of the header, and each node below is a field of the pointer
    /// above it.
    #[test]
    fn a_version_2_object_header_is_found_by_its_flags() {
        let f = v2_file();
        let doc = Document::new(MemSource(f));
        let mut ev = Evaluator::new(hdf5());
        // The cursor on the header's own `node_size`, which is inside the
        // tree's header and inside the object header above it.
        let mut at = V2_TREE.to_vec();
        at.push(3);
        let tree = super::super::hdf5_tree::tree(&mut ev, &doc, &at, 64).expect("walk").expect("a tree");
        assert_eq!(tree.nodes[0].address, V2_BTIN);
        let root = named(&mut ev, &doc, V2_TREE, &["root_node"]);
        assert_eq!(tree.nodes[0].path, root);
        for node in &tree.nodes {
            assert!(!node.path.is_empty(), "{tree:?}");
            assert_eq!(ev.node(&doc, &node.path).expect("node").offset_bits / 8, node.address);
        }
    }

    /// The path under `from` that the names lead to, stepping into what each
    /// named field points at, since a node reached by address is the field's
    /// only child. By name rather than by index, so a field added to a
    /// structure does not quietly move the test onto its neighbour.
    fn named(ev: &mut Evaluator, doc: &Document<MemSource>, from: &[usize], names: &[&str]) -> Vec<usize> {
        let mut at = from.to_vec();
        for name in names {
            if let Ok(i) = name.parse::<usize>() {
                at.push(i);
                continue;
            }
            at = ev.child_named(doc, &at, name).expect("reads").unwrap_or_else(|| panic!("no {name} under {at:?}"));
            if matches!(ev.node(doc, &at).expect("reads").type_name.as_str(), t if t.starts_with("at ")) {
                at.push(0);
            }
        }
        at
    }

    fn int_at(ev: &mut Evaluator, doc: &Document<MemSource>, at: &[usize]) -> i128 {
        match ev.node(doc, at).expect("reads").value {
            Value::Int(v) => v,
            Value::UInt(v) => i128::try_from(v).expect("fits"),
            Value::Enum { raw, .. } => raw,
            other => panic!("{at:?} holds {other:?}"),
        }
    }

    /// The children of a version 2 node are fields, and each one is the node
    /// its pointer names, holding as many records as the pointer says. A
    /// pointer's two counts are not written at any width the file states, so
    /// the second child landing on its leaf is what says the width of the
    /// first pointer was worked out right.
    #[test]
    fn a_version_2_node_below_the_root_is_placed_by_the_pointer_above_it() {
        let doc = Document::new(MemSource(v2_file()));
        let mut ev = Evaluator::new(hdf5());
        // Node size 512, records of 24 bytes: a leaf holds 20 records, so a
        // count takes one byte, and a pointer from level 1 is an address and
        // that byte.
        let level = named(&mut ev, &doc, V2_TREE, &["levels", "1", "pointer_size"]);
        assert_eq!(int_at(&mut ev, &doc, &level), 9);
        let root = named(&mut ev, &doc, V2_TREE, &["root_node"]);
        let children = named(&mut ev, &doc, &root, &["children"]);
        assert_eq!(ev.node(&doc, &children).expect("children").child_count, 2);
        for (i, leaf) in [(0usize, V2_LEAF_A), (1, V2_LEAF_B)] {
            let i = i.to_string();
            let count = named(&mut ev, &doc, &children, &[&i, "child_records"]);
            assert_eq!(int_at(&mut ev, &doc, &count), 2);
            // No total in a pointer to a leaf, and not a field of nought bytes
            // read as nought records either.
            let total = named(&mut ev, &doc, &children, &[&i, "total_records"]);
            assert_eq!(ev.node(&doc, &total).expect("total").size_bits, 0);
            let node = named(&mut ev, &doc, &children, &[&i, "node"]);
            let info = ev.node(&doc, &node).expect("leaf");
            assert_eq!(info.offset_bits / 8, leaf);
            assert_eq!(info.type_name, "BTree2Leaf");
            let records = named(&mut ev, &doc, &node, &["records"]);
            assert_eq!(ev.node(&doc, &records).expect("records").child_count, 2);
        }
    }

    /// Where the tree above sits under one more node, at level 2.
    const V2_TOP: u64 = 512;

    /// The same tree two levels deep: a `BTIN` at level 2 holding no records of
    /// its own and pointing at the level 1 node above the leaves.
    fn v2_deep_file() -> Vec<u8> {
        let mut f = v2_file();
        put(&mut f, V2_BTHD + 12, &2u16.to_le_bytes());
        put(&mut f, V2_BTHD + 16, &addr_bytes(V2_TOP));
        put(&mut f, V2_BTHD + 24, &0u16.to_le_bytes());
        // A pointer from level 2 is an address, the one-byte count, and a total
        // as wide as the most records a level 1 node and its leaves can hold,
        // which is 14 of its own and 20 in each of its 15 leaves: 314, which
        // takes two bytes.
        put(&mut f, V2_TOP, b"BTIN");
        put(&mut f, V2_TOP + 4, &[0, 10]);
        put(&mut f, V2_TOP + 6, &addr_bytes(V2_BTIN));
        put(&mut f, V2_TOP + 14, &[1]);
        put(&mut f, V2_TOP + 15, &5u16.to_le_bytes());
        f
    }

    /// A pointer above level 1 carries a total beside its count, as wide as
    /// the levels under it can need, and the node under it is read at the
    /// level one below its own. Walked from the bytes, every node lands where
    /// the template placed it.
    #[test]
    fn a_version_2_pointer_two_levels_up_reads_the_total_beside_its_count() {
        let doc = Document::new(MemSource(v2_deep_file()));
        let mut ev = Evaluator::new(hdf5());
        for (i, (pointer, total)) in [(0, 0), (9, 2), (11, 2)].into_iter().enumerate() {
            let i = i.to_string();
            let at = named(&mut ev, &doc, V2_TREE, &["levels", &i, "pointer_size"]);
            assert_eq!(int_at(&mut ev, &doc, &at), pointer, "level {i}");
            let at = named(&mut ev, &doc, V2_TREE, &["levels", &i, "total_records_size"]);
            assert_eq!(int_at(&mut ev, &doc, &at), total, "level {i}");
        }
        let top = named(&mut ev, &doc, V2_TREE, &["root_node"]);
        assert_eq!(ev.node(&doc, &top).expect("top").offset_bits / 8, V2_TOP);
        let level = named(&mut ev, &doc, &top, &["level"]);
        assert_eq!(int_at(&mut ev, &doc, &level), 2);
        let total = named(&mut ev, &doc, &top, &["children", "0", "total_records"]);
        assert_eq!(int_at(&mut ev, &doc, &total), 5);
        let middle = named(&mut ev, &doc, &top, &["children", "0", "node"]);
        assert_eq!(ev.node(&doc, &middle).expect("middle").offset_bits / 8, V2_BTIN);
        let level = named(&mut ev, &doc, &middle, &["level"]);
        assert_eq!(int_at(&mut ev, &doc, &level), 1);
        let leaf = named(&mut ev, &doc, &middle, &["children", "1", "node"]);
        assert_eq!(ev.node(&doc, &leaf).expect("leaf").offset_bits / 8, V2_LEAF_B);

        let tree = super::super::hdf5_tree::tree(&mut ev, &doc, V2_TREE, 64).expect("walk").expect("a tree");
        assert_eq!(tree.nodes.len(), 4, "{tree:?}");
        for node in &tree.nodes {
            assert!(!node.path.is_empty(), "{tree:?}");
            assert_eq!(ev.node(&doc, &node.path).expect("node").offset_bits / 8, node.address);
        }
        assert_eq!(tree.nodes[3].path, leaf);
    }

    // A group whose links are in a fractal heap grown past its direct rows,
    // built by hand and made as small as the arithmetic allows: two blocks to
    // a row, 64 bytes to the smallest block and 128 to the largest direct one.
    // So rows 0 to 2 hold direct blocks and row 3, of 256-byte blocks, holds
    // tables, each of two rows.
    const HEAP_HEADER_V2: u64 = 256;
    const HEAP_ROOT_TABLE: u64 = 512;
    const HEAP_ROOT_BLOCK: u64 = 640;
    const HEAP_CHILD_TABLE: u64 = 768;
    const HEAP_CHILD_BLOCK: u64 = 896;
    const HEAP_END: u64 = 1024;

    /// The path to the heap: the root group's object header, its link info
    /// message, and the heap that message names.
    const HEAP_PATH: &[usize] = &[2, 8, 0, 6, 0, 4];

    /// One link in a heap block: version 1, no optional fields, a name, and an
    /// address that says there is nothing there to follow.
    fn heap_link_bytes(name: &str) -> Vec<u8> {
        let mut out = vec![1, 0, name.len() as u8];
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&addr_bytes(u64::MAX));
        out
    }

    fn heap_file() -> Vec<u8> {
        let mut f = v2_file();
        f.truncate(V2_BTHD as usize);
        // The link info message names the heap, and no tree.
        put(&mut f, V2_ROOT_HEADER + 13, &addr_bytes(HEAP_HEADER_V2));
        put(&mut f, V2_ROOT_HEADER + 21, &addr_bytes(u64::MAX));
        put(&mut f, 28, &addr_bytes(HEAP_END));
        let h = HEAP_HEADER_V2;
        put(&mut f, h, b"FRHP");
        put(&mut f, h + 5, &7u16.to_le_bytes());
        put(&mut f, h + 7, &0u16.to_le_bytes());
        put(&mut f, h + 9, &[0]);
        put(&mut f, h + 22, &addr_bytes(u64::MAX));
        put(&mut f, h + 38, &addr_bytes(u64::MAX));
        put(&mut f, h + 110, &2u16.to_le_bytes());
        put(&mut f, h + 112, &64u64.to_le_bytes());
        put(&mut f, h + 120, &128u64.to_le_bytes());
        put(&mut f, h + 128, &32u16.to_le_bytes());
        put(&mut f, h + 130, &1u16.to_le_bytes());
        put(&mut f, h + 132, &addr_bytes(HEAP_ROOT_TABLE));
        put(&mut f, h + 140, &4u16.to_le_bytes());
        // The root table: four rows of two, every entry empty but the first
        // direct block and the first table in row 3.
        let entries = HEAP_ROOT_TABLE + 17;
        put(&mut f, HEAP_ROOT_TABLE, b"FHIB");
        for i in 0..8 {
            put(&mut f, entries + 8 * i, &addr_bytes(u64::MAX));
        }
        put(&mut f, entries, &addr_bytes(HEAP_ROOT_BLOCK));
        put(&mut f, entries + 8 * 6, &addr_bytes(HEAP_CHILD_TABLE));
        put(&mut f, HEAP_ROOT_BLOCK, b"FHDB");
        put(&mut f, HEAP_ROOT_BLOCK + 17, &heap_link_bytes("alpha"));
        // The table in row 3, of two rows, whose first block holds the second
        // link.
        let entries = HEAP_CHILD_TABLE + 17;
        put(&mut f, HEAP_CHILD_TABLE, b"FHIB");
        for i in 0..4 {
            put(&mut f, entries + 8 * i, &addr_bytes(u64::MAX));
        }
        put(&mut f, entries, &addr_bytes(HEAP_CHILD_BLOCK));
        put(&mut f, HEAP_CHILD_BLOCK, b"FHDB");
        put(&mut f, HEAP_CHILD_BLOCK + 17, &heap_link_bytes("beta"));
        f.resize(HEAP_END as usize, 0);
        f
    }

    /// A heap row past the largest direct block size holds a table rather than
    /// a block, and that table has as many rows as its size says, which is
    /// written nowhere. The links in the blocks under it come out the same as
    /// the links in the root table's own blocks.
    #[test]
    fn a_heap_row_past_the_direct_rows_holds_another_table() {
        let doc = Document::new(MemSource(heap_file()));
        let mut ev = Evaluator::new(hdf5());
        let heap = named(&mut ev, &doc, HEAP_PATH, &["heap"]);
        let direct = named(&mut ev, &doc, &heap, &["direct_rows"]);
        assert_eq!(int_at(&mut ev, &doc, &direct), 3);
        let root = named(&mut ev, &doc, &heap, &["root_block"]);
        assert_eq!(ev.node(&doc, &root).expect("root").offset_bits / 8, HEAP_ROOT_TABLE);
        let children = named(&mut ev, &doc, &root, &["children"]);
        assert_eq!(ev.node(&doc, &children).expect("children").child_count, 8);
        let name = named(&mut ev, &doc, &children, &["0", "block", "links", "0", "name"]);
        assert_eq!(ev.node(&doc, &name).expect("name").value, Value::Str("alpha".into()));

        let size = named(&mut ev, &doc, &children, &["6", "block_size"]);
        assert_eq!(int_at(&mut ev, &doc, &size), 256);
        let rows = named(&mut ev, &doc, &children, &["6", "block_rows"]);
        assert_eq!(int_at(&mut ev, &doc, &rows), 2);
        let table = named(&mut ev, &doc, &children, &["6", "block"]);
        let info = ev.node(&doc, &table).expect("table");
        assert_eq!((info.offset_bits / 8, info.type_name.as_str()), (HEAP_CHILD_TABLE, "HeapIndirectBlock"));
        // Four entries and then the checksum, which is what a table read with
        // the root's four rows would have run straight past.
        assert_eq!(info.size_bits / 8, 17 + 4 * 8 + 4);
        let name = named(&mut ev, &doc, &table, &["children", "0", "block", "links", "0", "name"]);
        assert_eq!(ev.node(&doc, &name).expect("name").value, Value::Str("beta".into()));
    }
    /// The eight bytes every datatype message opens with: version and class in
    /// one byte, the three bytes of class bits, the size.
    fn datatype_head(version: u8, class: u8, bits: [u8; 3], size: u32) -> Vec<u8> {
        let mut v = vec![version << 4 | class];
        v.extend_from_slice(&bits);
        v.extend_from_slice(&size.to_le_bytes());
        v
    }

    /// An integer datatype of this many bytes, little-endian.
    fn integer(bytes: u32, signed: bool) -> Vec<u8> {
        let mut v = datatype_head(1, 0, [if signed { 8 } else { 0 }, 0, 0], bytes);
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&(bytes as u16 * 8).to_le_bytes());
        v
    }

    /// A name padded the way versions 1 and 2 pad one.
    fn name8(name: &str) -> Vec<u8> {
        let mut v = name.as_bytes().to_vec();
        v.push(0);
        v.resize(v.len().div_ceil(8) * 8, 0);
        v
    }

    /// A datatype and then elements of it, read the way an attribute reads
    /// its value: the datatype is a field in scope, and the rest is the run.
    fn typed(datatype: &[u8], data: &[u8]) -> (Evaluator, Document<MemSource>) {
        let part = hdf5_part();
        let root = T::structure(
            "Typed",
            vec![("datatype", T::Named("Datatype".into())), ("data", elements(Described::Inside, E::Remaining))],
        );
        let mut bytes = datatype.to_vec();
        bytes.extend_from_slice(data);
        (Evaluator::new(Template::new("typed", root).with_part(&part)), Document::new(MemSource(bytes)))
    }

    /// A member of a version 1 compound can be an array without being an array
    /// datatype: its record says how many dimensions and how long each is, and
    /// the type is the element's. No library since 1.4 writes one, so this is
    /// built by hand. The second member is two 32-bit integers.
    #[test]
    fn a_version_1_member_with_dimensions_reads_as_that_many_values() {
        let mut dt = datatype_head(1, 6, [2, 0, 0], 12);
        for (name, offset, dims) in [("a", 0u32, [0u32; 4]), ("b", 4, [2, 0, 0, 0])] {
            dt.extend(name8(name));
            dt.extend_from_slice(&offset.to_le_bytes());
            dt.push(u8::from(dims[0] > 0));
            dt.extend_from_slice(&[0; 3]);
            dt.extend_from_slice(&0u32.to_le_bytes());
            dt.extend_from_slice(&0u32.to_le_bytes());
            for d in dims {
                dt.extend_from_slice(&d.to_le_bytes());
            }
            dt.extend(integer(4, true));
        }
        let data: Vec<u8> = [7i32, 8, 9].iter().flat_map(|v| v.to_le_bytes()).collect();
        let (mut ev, doc) = typed(&dt, &data);
        let values = [1, 2, 0, 0];
        let member = |ev: &mut Evaluator, path: &[usize]| {
            let mut p = values.to_vec();
            p.extend_from_slice(path);
            ev.node(&doc, &p).expect("reads")
        };
        let a = member(&mut ev, &[0]);
        assert_eq!((a.name.as_str(), a.value.as_int()), ("[0] a", Some(7)));
        let b = member(&mut ev, &[1]);
        assert_eq!((b.name.as_str(), b.child_count, b.offset_bits), ("[1] b", 2, (dt.len() as u64 + 4) * 8));
        assert_eq!(member(&mut ev, &[1, 0]).value.as_int(), Some(8));
        assert_eq!(member(&mut ev, &[1, 1]).value.as_int(), Some(9));
    }

    /// Version 3 writes a member's offset in as few bytes as the compound's
    /// size needs, so a compound of 300 bytes writes two, and a name with no
    /// padding after it. Read one byte wide, the second member's offset would
    /// be 43 and its datatype would start a byte early.
    #[test]
    fn a_version_3_member_offset_is_as_wide_as_the_compound_s_size_needs() {
        let mut dt = datatype_head(3, 6, [2, 0, 0], 300);
        for (name, offset) in [("x", 0u16), ("y", 299)] {
            dt.extend_from_slice(name.as_bytes());
            dt.push(0);
            dt.extend_from_slice(&offset.to_le_bytes());
            dt.extend(integer(1, false));
        }
        let mut data = vec![0u8; 300];
        data[0] = 5;
        data[299] = 6;
        let (mut ev, doc) = typed(&dt, &data);
        let x = ev.node(&doc, &[1, 2, 0, 0, 0]).expect("reads");
        let y = ev.node(&doc, &[1, 2, 0, 0, 1]).expect("reads");
        assert_eq!((x.name.as_str(), x.value.as_int()), ("[0] x", Some(5)));
        assert_eq!((y.name.as_str(), y.value.as_int()), ("[1] y", Some(6)));
        assert_eq!(y.offset_bits, (dt.len() as u64 + 299) * 8);
    }

    /// An enumeration's elements are the integers it is based on, and its
    /// values in the datatype message are labelled with the names written
    /// before them. What h5py stores a boolean as.
    #[test]
    fn an_enumeration_reads_as_its_base_integers_and_names_its_values() {
        let mut dt = datatype_head(1, 8, [2, 0, 0], 1);
        dt.extend(integer(1, true));
        dt.extend(name8("FALSE"));
        dt.extend(name8("TRUE"));
        dt.extend_from_slice(&[0, 1]);
        let (mut ev, doc) = typed(&dt, &[1, 0, 1]);
        let run = [1, 2];
        assert_eq!(ev.node(&doc, &run).expect("reads").child_count, 3);
        assert_eq!(ev.node(&doc, &[1, 2, 0]).expect("reads").value.as_int(), Some(1));
        assert_eq!(ev.node(&doc, &[1, 2, 1]).expect("reads").value.as_int(), Some(0));
        let values = named(&mut ev, &doc, &[], &["datatype", "properties", "values"]);
        let mut second = values.clone();
        second.push(1);
        let second = ev.node(&doc, &second).expect("reads");
        assert_eq!((second.name.as_str(), second.value.as_int()), ("[1] TRUE", Some(1)));
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

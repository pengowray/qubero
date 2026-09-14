//! The HDF4 records, each as the fields its own tag lays out: the version,
//! the number type and one value of it, the three dimension records, the
//! palette, the header of a special element and the linked blocks it names,
//! a dataset's label, unit and format strings, the vdata header, and the
//! vgroup.
//!
//! Apart from [`hdf4`](super::hdf4), which keeps the descriptors, the blocks,
//! the index, and the readings that join one record to another, such as a
//! raster image read through its dimension record or a table's rows through
//! their header. Where a record here needs another thing in the file, it asks
//! through the lookups `hdf4.rs` lends it.

use super::hdf4::{counted_name, found, i16be, key, look, own_ref, place_of_member, record_at, tag, tag_of, u16be, u32be};
use crate::template::{Anchor, Encoding, Endian::Big, Expr as E, Step, StrLen, Ty as T};

/// The number types a vdata field can have, which are the same numbers a
/// scientific dataset writes in its `number type` record.
const NUMBER_TYPE: &[(i128, &str)] = &[
    (3, "uchar"),
    (4, "char"),
    (5, "float32"),
    (6, "float64"),
    (20, "int8"),
    (21, "uint8"),
    (22, "int16"),
    (23, "uint16"),
    (24, "int32"),
    (25, "uint32"),
    (26, "int64"),
    (27, "uint64"),
];

/// The fourth byte of a number type record, for a whole number: which way
/// round the bytes of one are written. Big-endian is what HDF4 calls its own,
/// and a file written on an Intel machine by a library told not to convert
/// says 4 here and means every number in it reads the other way.
const INT_CLASS: &[(i128, &str)] = &[(1, "big-endian"), (2, "VAX"), (4, "little-endian")];

/// The same byte for a floating point number: whose floating point it is. The
/// two that are still written are IEEE either way round; the rest are machines
/// whose last file is thirty years old.
const FLOAT_CLASS: &[(i128, &str)] =
    &[(1, "IEEE big-endian"), (2, "VAX"), (3, "Cray"), (4, "IEEE little-endian"), (5, "Convex"), (6, "VP")];

/// The same byte again for a character: which character set. One byte either
/// way round is the same byte, so this says nothing about order.
const CHAR_CLASS: &[(i128, &str)] = &[(0, "bytes"), (1, "ASCII"), (5, "EBCDIC")];

/// The three ways the samples of a raster image with more than one component
/// per pixel are laid out.
const PIXEL_INTERLACE: &[(i128, &str)] =
    &[(0, "by pixel"), (1, "by line"), (2, "by component")];

/// Which class byte means a little-endian machine wrote the numbers. The
/// same 4 for whole numbers (Intel byte order) and for floating point (the
/// PC's IEEE), which is why one number answers for both.
const LITTLE_ENDIAN_CLASS: i128 = 4;

/// The version record: which release of the library wrote the file, and a line
/// of text saying so in words.
pub(super) fn version() -> T {
    T::structure(
        "Hdf4Version",
        vec![
            ("major", u32be()),
            ("minor", u32be()),
            ("release", u32be()),
            // Padded out with nuls to whatever room the writer left for it.
            ("description", T::text(StrLen::Padded { size: E::Remaining, pad: 0 }, Encoding::Ascii)),
        ],
    )
}

/// The number type record: four bytes saying how to read every value of the
/// thing that names it.
///
/// `type` is the same number a vdata header writes for a column, and it
/// carries the width and the sign on its own. `width` says the same thing in
/// bits, and `class` says which machine's way round the bytes are, which for
/// anything still written means big-endian or little. What `class` means at
/// all depends on the type, since the numbers were handed out three times
/// over: 1 is IEEE for a float, Motorola byte order for a whole number and
/// ASCII for a character.
pub(super) fn number_type() -> T {
    T::structure(
        "Hdf4NumberType",
        vec![
            ("version", T::u8()),
            ("type", T::enumeration("Hdf4NumberTypeName", T::u8(), NUMBER_TYPE)),
            ("width", T::u8()),
            (
                "class",
                T::switch(
                    E::field("type"),
                    vec![
                        (3, T::enumeration("Hdf4CharClass", T::u8(), CHAR_CLASS)),
                        (4, T::enumeration("Hdf4CharClass", T::u8(), CHAR_CLASS)),
                        (5, T::enumeration("Hdf4FloatClass", T::u8(), FLOAT_CLASS)),
                        (6, T::enumeration("Hdf4FloatClass", T::u8(), FLOAT_CLASS)),
                    ],
                    T::enumeration("Hdf4IntClass", T::u8(), INT_CLASS),
                ),
            ),
        ],
    )
}

/// One value of the number type a number type record describes, read where
/// that record's `type` and `class` can be reached by expression.
///
/// Twelve types and two byte orders is twenty-four cases, which is what it
/// costs to say that a byte order read out of the file decides how a number
/// is read: the type of a field is settled when the template is built, so the
/// choice has to be a switch over the types rather than an order handed to
/// one. Anything the number type record does not name is left as bytes as
/// wide as it says it is, so an unknown type does not push everything after
/// it out of line.
pub(super) fn number_value(number_type: E, class: E, width: E) -> T {
    let of = |e: crate::template::Endian| {
        T::switch(
            number_type.clone(),
            vec![
                (3, T::u8()),
                (4, T::Int { bits: 8, endian: e }),
                (5, T::F32(e)),
                (6, T::F64(e)),
                (20, T::Int { bits: 8, endian: e }),
                (21, T::u8()),
                (22, T::Int { bits: 16, endian: e }),
                (23, T::u16(e)),
                (24, T::i32(e)),
                (25, T::u32(e)),
                (26, T::Int { bits: 64, endian: e }),
                (27, T::u64(e)),
            ],
            T::bytes(width.clone().div(E::lit(8)).at_least(E::lit(1))),
        )
    };
    T::switch(
        class.equals(E::lit(LITTLE_ENDIAN_CLASS)),
        vec![(1, of(crate::template::Endian::Little))],
        of(Big),
    )
}

/// The image dimension record, which every raster image and every palette
/// that was written with one has beside it.
///
/// `xdim` is how many pixels are across and `ydim` how many rows there are,
/// and the image is written a row at a time. `nelements` is how many samples
/// make a pixel: one for a greyscale image or an index into a palette, three
/// for RGB. A palette's own dimension record says 256 by 1 with three
/// elements, which is the same thing said about a table of colours.
///
/// The compression tag is zero for an image stored as it is. Where it is not,
/// the bytes are compressed and the tag says how.
pub(super) fn image_dimensions() -> T {
    T::structure(
        "Hdf4ImageDimensions",
        vec![
            ("xdim", T::i32(Big)),
            ("ydim", T::i32(Big)),
            ("number_type_tag", tag()),
            ("number_type_ref", u16be()),
            ("nelements", i16be()),
            ("interlace", T::enumeration("Hdf4PixelInterlace", i16be(), PIXEL_INTERLACE)),
            ("compression_tag", tag()),
            ("compression_ref", u16be()),
        ],
    )
}

/// The dimension record of an eight-bit image, which is all the older tags
/// say: how wide, how tall, and nothing about the number type, because there
/// was only ever one.
pub(super) fn eight_bit_dimensions() -> T {
    T::structure("Hdf4EightBitDimensions", vec![("xdim", i16be()), ("ydim", i16be())])
}

/// The dimension record of a scientific dataset: how many dimensions it has,
/// how long each of them is, what its values are, and what the scale along
/// each dimension is.
///
/// The first dimension is the slowest to change, the way C counts an array.
/// Every scale has a number type of its own because a dataset of bytes may
/// still be measured in seconds.
pub(super) fn sd_dimensions() -> T {
    let rank = || E::field("rank").at_least(E::lit(0));
    T::structure(
        "Hdf4SdDimensions",
        vec![
            ("rank", i16be()),
            ("dims", T::array(T::i32(Big), rank())),
            ("number_type_tag", tag()),
            ("number_type_ref", u16be()),
            (
                "scales",
                T::array(
                    T::structure("Hdf4SdScaleType", vec![("tag", tag()), ("ref", u16be())]),
                    rank(),
                ),
            ),
        ],
    )
}

/// Where each of a vgroup's members is, worked out from the tag and the
/// reference number that name it.
///
/// A vgroup writes its tags and its reference numbers as two runs, so these
/// rows put the two back together and add where each pair points. They take
/// no bytes of their own.
fn members(tag_from: E, ref_of: E, count: E) -> T {
    T::array(
        T::structure_named(
            "Hdf4VgroupMember",
            "tag",
            "",
            vec![
                ("tag", tag_of(T::computed(tag_from))),
                ("ref", T::computed(ref_of)),
                ("offset", T::computed(place_of_member(&["offset"]))),
                ("length", T::computed(place_of_member(&["length"]))),
            ],
        ),
        count,
    )
}

/// A palette: 256 colours, each a red, a green and a blue byte.
///
/// The count is the length rather than a fixed 256, because the older
/// eight-bit palette tag and the newer lookup table are both written this way
/// and a writer is free to keep fewer.
pub(super) fn palette() -> T {
    T::structure(
        "Hdf4Palette",
        vec![(
            "colours",
            T::array(
                T::structure("Hdf4Colour", vec![("red", T::u8()), ("green", T::u8()), ("blue", T::u8())]),
                E::Remaining.div(E::lit(3)),
            ),
        )],
    )
}

/// The header of a special element: the same object kept somewhere other than
/// in one run of bytes.
///
/// Every tag has a twin with 0x4000 added, and a descriptor carrying one of
/// those points at this rather than at the object. The first number says which
/// of the seven ways it is kept, and the two worth reading say where the rest
/// is: linked blocks name the first of a chain of them, and a compressed
/// element names the run of compressed bytes and says what compressed it.
///
/// Values in linked blocks are joined back into the one run they are and read
/// as `inner`. The descriptor of a special element has nothing to say what
/// that run holds, so it passes bytes; a scientific dataset, which knows its
/// shape and its number type, passes those. A compressed element's bytes stay
/// bytes under the tag that holds them.
pub(super) fn special_element(inner: T) -> T {
    let compressed = T::structure(
        "Hdf4CompressedElement",
        vec![
            ("version", i16be()),
            ("length", T::i32(Big)),
            ("data_ref", u16be()),
            ("model_type", T::enumeration("Hdf4CompressionModel", u16be(), &[(0, "stored as read")])),
            (
                "compression",
                T::enumeration(
                    "Hdf4Compression",
                    u16be(),
                    &[
                        (0, "none"),
                        (1, "run-length"),
                        (2, "n-bit"),
                        (3, "skipping Huffman"),
                        (4, "deflate"),
                        (5, "szip"),
                    ],
                ),
            ),
            ("settings", T::bytes(E::Remaining)),
        ],
    );
    T::structure(
        "Hdf4SpecialElement",
        vec![
            (
                "kind",
                T::enumeration(
                    "Hdf4SpecialKind",
                    i16be(),
                    &[
                        (1, "linked blocks"),
                        (2, "in another file"),
                        (3, "compressed"),
                        (4, "variable-length linked blocks"),
                        (5, "chunked"),
                        (6, "buffered"),
                        (7, "compressed raster"),
                    ],
                ),
            ),
            ("kept", T::switch(E::field("kind"), vec![(1, linked_blocks(inner)), (3, compressed)], T::bytes(E::Remaining))),
        ],
    )
}

/// An object kept in linked blocks: one run of bytes cut into blocks wherever
/// the writer found room, and joined back here into the run it is.
///
/// What the library does when something grows past where it was first written,
/// which for a scientific dataset is any dataset with an unlimited dimension.
/// The header says how long the whole run is, how long a block is, and how
/// many blocks a link table lists, and names the first link table. A link
/// table is a descriptor tagged 20 like the blocks it lists: the reference
/// number of the next table, or zero, and then one reference number per block
/// in the order the run's bytes go. A slot not yet used is zero.
///
/// The first block is the object as it was before it grew, so it is as long as
/// its own descriptor says rather than a block's length. `HLIstaccess` reads it
/// that way too: `linkinfo_t` has a `first_length`, and the file does not, so
/// every block's length is read from its descriptor and the run is cut at
/// `length`. Blocks written past the length, and the unused tail of the last,
/// fall outside it.
///
/// The tables are a chain, and the step from one to the next is a reference
/// number rather than an offset, so it goes through the index: `next_at` is
/// where the descriptor named by `next_ref` is, and nought, which ends the
/// chain, where `next_ref` is zero and names nothing. The tables and the blocks
/// are the bytes of their own descriptors, so they are read here a second time
/// and counted there.
///
/// A slot left at zero before one in use would stand for bytes never written,
/// and joining has nothing to put in their place: that slot adds nothing and
/// every byte after it reads early. No sample leaves one.
fn linked_blocks(inner: T) -> T {
    let at_ref = |r: E| look(key(20, r), &["offset"]);
    let len_ref = |r: E| look(key(20, r), &["length"]);
    let block_ref = || E::elem("block_refs", E::idx());
    // An unused slot is placed where it stands and takes nothing, rather than
    // at an offset of nought, which is the signature.
    let block = T::switch(
        block_ref().equals(E::lit(0)),
        vec![(1, T::bytes(E::lit(0)))],
        T::at(at_ref(block_ref()), T::bytes(len_ref(block_ref()))),
    );
    let blocks = || E::field("number_blocks").at_least(E::lit(0));
    let table = T::structure(
        "Hdf4LinkTable",
        vec![
            ("next_ref", u16be()),
            ("block_refs", T::array(u16be(), blocks())),
            ("next_at", T::computed(at_ref(E::field("next_ref")))),
            ("blocks", T::array(block, blocks())),
        ],
    )
    .machinery(&["next_at"]);
    T::structure(
        "Hdf4LinkedBlocks",
        vec![
            ("length", T::i32(Big)),
            ("block_length", T::i32(Big)),
            ("number_blocks", T::i32(Big)),
            ("link_ref", u16be()),
            ("tables", T::chain(at_ref(E::field("link_ref")), &["next_at"], Anchor::File, table)),
            (
                "values",
                T::stitched(
                    vec![Step::field("tables"), Step::each(), Step::field("blocks"), Step::each()],
                    None,
                    Some(E::field("length").at_least(E::lit(0))),
                    inner,
                ),
            ),
        ],
    )
    .machinery(&["tables"])
    .field_aside("tables")
}

/// The label, unit and format records of a scientific dataset: one string for
/// the dataset and then one for each of its dimensions, run together and each
/// ended with a nul. An empty string is a nul on its own, so a dataset with
/// labelled dimensions and no label of its own starts with one.
///
/// How many dimensions there are is in the dimension record with the same
/// reference number, which is how the library writes these beside it, so the
/// first string is read as the dataset's and the rest as the dimensions', one
/// each in the order the dimension record lists them. Without that record the
/// strings are still read, as a list whose first entry is the dataset's.
pub(super) fn sd_strings() -> T {
    let dims = || key(701, own_ref());
    let text = || T::text(StrLen::Terminated { end: 0, or_end: true }, Encoding::Ascii);
    let rank = E::within(&["dimension_record", "rank"]).at_least(E::lit(0));
    let known = T::structure(
        "Hdf4SdStrings",
        vec![
            ("dimension_record", record_at(dims(), sd_dimensions())),
            ("dataset", text()),
            ("dimensions", T::array(text(), rank)),
        ],
    )
    .machinery(&["dimension_record"])
    .field_aside("dimension_record");
    let unknown =
        T::structure("Hdf4SdStringList", vec![("strings", T::repeat(text(), crate::template::Until::End))]);
    T::switch(found(dims()), vec![(1, known)], unknown)
}

/// One column of a table: what its values are, how wide one is, where in a
/// record it starts, and how many of them there are per record.
///
/// The four are written as four arrays rather than as one array of four, so
/// this type is what a reader assembles rather than what the file holds.
fn vdata_fields(count: E) -> Vec<(&'static str, T)> {
    let n = || count.clone();
    vec![
        ("field_types", T::array(T::enumeration("Hdf4NumberTypeName", i16be(), NUMBER_TYPE), n())),
        ("field_sizes", T::array(u16be(), n())),
        ("field_offsets", T::array(u16be(), n())),
        ("field_orders", T::array(u16be(), n())),
        ("field_names", T::array(counted_name(), n())),
    ]
}

/// The vdata header: the shape of a table, without any of its rows.
///
/// `ivsize` is one record, `nvertices` is how many of them there are, and the
/// rows themselves are in a separate descriptor tagged `vdata storage` with
/// the same ref.
///
/// The version and the `more` field are written twice over, once here and
/// again at the very end. That is not a mistake in the reading: a library old
/// enough not to know about the attributes in between still finds the pair it
/// expects by counting back from the end of the record. Version 4 is where
/// those attributes came in, and a version 3 header stops before the flags.
pub(super) fn vdata_header() -> T {
    let n = || E::field("nfields").at_least(E::lit(0));
    let mut fields = vec![
        ("interlace", T::enumeration("Hdf4Interlace", i16be(), &[(0, "by record"), (1, "by field")])),
        ("nvertices", u32be()),
        ("ivsize", u16be()),
        ("nfields", i16be()),
    ];
    fields.extend(vdata_fields(n()));
    fields.extend(vec![
        ("name", counted_name()),
        ("class", counted_name()),
        // Where a record too large for this one is continued. Zero in every
        // file a current library writes.
        ("extension_tag", u16be()),
        ("extension_ref", u16be()),
        ("version", i16be()),
        ("more", i16be()),
    ]);
    fields.extend(version_four_attributes());
    fields.extend(vec![
        // The pair again, and the nul the writer ends the record with.
        ("version_again", T::if_room(i16be())),
        ("more_again", T::if_room(i16be())),
        ("terminator", T::bytes(E::Remaining)),
    ]);
    T::structure_named("Hdf4VdataHeader", "name", "", fields)
}

/// The attributes a vdata header grew in version 4: a flag word, a count, and
/// one record per attribute saying which field it belongs to and where its
/// value is kept. A vgroup's are written differently; see [`vgroup`].
///
/// A field index of -1 is an attribute of the whole thing rather than of one
/// of its columns. The value itself is a vdata of one record, named by the tag
/// and ref here, and `offset` and `length` say where that vdata's header is,
/// the same way a group member says where its member is.
fn version_four_attributes() -> Vec<(&'static str, T)> {
    vec![
        ("flags", T::present_if(E::lit(3).less_than(E::field("version")), u32be())),
        ("nattrs", T::present_if(E::field("flags").bit(0), T::i32(Big))),
        (
            "attributes",
            T::array(
                T::structure(
                    "Hdf4VdataAttribute",
                    vec![
                        ("field_index", T::i32(Big)),
                        ("tag", tag()),
                        ("ref", u16be()),
                        ("offset", T::computed(place_of_member(&["offset"]))),
                        ("length", T::computed(place_of_member(&["length"]))),
                    ],
                ),
                E::field("nattrs").at_least(E::lit(0)),
            ),
        ),
    ]
}

/// A vgroup: a name, a class, and the tags and reference numbers of whatever
/// belongs to it, which may be other vgroups. This is the only tag that makes
/// a file a tree rather than a list, and it is what the HDF4 interfaces call a
/// group, a scientific dataset's container and an image's container alike.
///
/// The tags and the reference numbers are written as two runs rather than as
/// pairs, so `members` reads them back together and is where each one points.
///
/// A vgroup is not laid out the way a vdata header is, though the two share
/// their version numbers. The version and the `more` field are written once,
/// last, and the library finds them by counting back five bytes from the end
/// of the record. What version 4 added comes straight after the extension
/// pair: a flag word, and where the flag says so a count and a tag and a
/// reference number for each attribute, with no field index, since a vgroup
/// has no columns. A vgroup with no flags set is written the old way, so the
/// flag word is there exactly when more than those five bytes are left.
pub(super) fn vgroup() -> T {
    let n = || E::field("nvelt").at_least(E::lit(0));
    let attribute = T::structure(
        "Hdf4VgroupAttribute",
        vec![
            ("tag", tag()),
            ("ref", u16be()),
            ("offset", T::computed(place_of_member(&["offset"]))),
            ("length", T::computed(place_of_member(&["length"]))),
        ],
    );
    let fields = vec![
        ("nvelt", i16be()),
        ("tags", T::array(tag(), n())),
        ("refs", T::array(u16be(), n())),
        ("members", members(E::elem("tags", E::idx()), E::elem("refs", E::idx()), n())),
        ("name", counted_name()),
        ("class", counted_name()),
        ("extension_tag", u16be()),
        ("extension_ref", u16be()),
        ("flags", T::present_if(E::lit(5).less_than(E::Remaining), u32be())),
        ("nattrs", T::present_if(E::field("flags").bit(0), T::i32(Big))),
        ("attributes", T::array(attribute, E::field("nattrs").at_least(E::lit(0)))),
        ("version", i16be()),
        ("more", i16be()),
        // The nul the writer ends the record with.
        ("terminator", T::bytes(E::Remaining)),
    ];
    // The two runs are where the bytes are; `members` is the reading of them a
    // person wants, so it is the one the listing leads with.
    T::structure_named("Hdf4Vgroup", "name", "", fields).machinery(&["tags", "refs"])
}
